use bitloops::cli::versioncheck::DISABLE_VERSION_CHECK_ENV;
use bitloops::config::resolve_sqlite_db_path_for_repo;
use bitloops::config::resolve_store_backend_config_for_repo;
use bitloops::host::checkpoints::session::create_session_backend_or_local;
use bitloops::host::checkpoints::strategy::manual_commit::{
    read_commit_checkpoint_mappings, read_committed, read_session_content,
};
use bitloops::host::devql::watch::DISABLE_WATCHER_AUTOSTART_ENV;
use rusqlite::Connection;
use std::env;
use std::ffi::{OsStr, OsString};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::{Mutex, MutexGuard, OnceLock};

fn bitloops_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_bitloops"))
}

struct HomeEnvPaths {
    xdg_config: PathBuf,
    xdg_data: PathBuf,
    xdg_cache: PathBuf,
    xdg_state: PathBuf,
}

fn home_env_paths(home: &Path) -> HomeEnvPaths {
    let paths = HomeEnvPaths {
        xdg_config: home.join("xdg-config"),
        xdg_data: home.join("xdg-data"),
        xdg_cache: home.join("xdg-cache"),
        xdg_state: home.join("xdg-state"),
    };
    for dir in [
        home,
        &paths.xdg_config,
        &paths.xdg_data,
        &paths.xdg_cache,
        &paths.xdg_state,
    ] {
        fs::create_dir_all(dir).expect("create isolated Bitloops app dir");
    }
    paths
}

fn apply_home_env(cmd: &mut Command, home: &Path) {
    let paths = home_env_paths(home);
    cmd.env("HOME", home)
        .env("USERPROFILE", home)
        .env("XDG_CONFIG_HOME", paths.xdg_config)
        .env("XDG_DATA_HOME", paths.xdg_data)
        .env("XDG_CACHE_HOME", paths.xdg_cache)
        .env("XDG_STATE_HOME", paths.xdg_state)
        .env(DISABLE_WATCHER_AUTOSTART_ENV, "1")
        .env(DISABLE_VERSION_CHECK_ENV, "1")
        .env_remove("BITLOOPS_DEVQL_PG_DSN")
        .env_remove("BITLOOPS_DEVQL_CH_URL")
        .env_remove("BITLOOPS_DEVQL_CH_DATABASE")
        .env_remove("BITLOOPS_DEVQL_CH_USER")
        .env_remove("BITLOOPS_DEVQL_CH_PASSWORD");
}

fn env_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

struct HomeEnvGuard {
    _lock_guard: MutexGuard<'static, ()>,
    previous_env: Vec<(String, Option<OsString>)>,
}

impl Drop for HomeEnvGuard {
    fn drop(&mut self) {
        restore_env_vars(&self.previous_env);
    }
}

fn apply_env_vars(vars: &[(&str, Option<&OsStr>)]) -> Vec<(String, Option<OsString>)> {
    let mut previous = Vec::with_capacity(vars.len());
    for (key, value) in vars {
        previous.push(((*key).to_string(), env::var_os(key)));
        // SAFETY: integration tests serialise process env mutation through env_lock().
        unsafe {
            match value {
                Some(v) => env::set_var(key, v),
                None => env::remove_var(key),
            }
        }
    }
    previous
}

fn restore_env_vars(previous: &[(String, Option<OsString>)]) {
    for (key, value) in previous.iter().rev() {
        // SAFETY: integration tests serialise process env mutation through env_lock().
        unsafe {
            match value {
                Some(v) => env::set_var(key, v),
                None => env::remove_var(key),
            }
        }
    }
}

fn enter_home_env(home: &Path) -> HomeEnvGuard {
    let lock_guard = env_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let paths = home_env_paths(home);
    let previous_env = apply_env_vars(&[
        ("HOME", Some(home.as_os_str())),
        ("USERPROFILE", Some(home.as_os_str())),
        ("XDG_CONFIG_HOME", Some(paths.xdg_config.as_os_str())),
        ("XDG_DATA_HOME", Some(paths.xdg_data.as_os_str())),
        ("XDG_CACHE_HOME", Some(paths.xdg_cache.as_os_str())),
        ("XDG_STATE_HOME", Some(paths.xdg_state.as_os_str())),
        ("BITLOOPS_DEVQL_PG_DSN", None),
        ("BITLOOPS_DEVQL_CH_URL", None),
        ("BITLOOPS_DEVQL_CH_DATABASE", None),
        ("BITLOOPS_DEVQL_CH_USER", None),
        ("BITLOOPS_DEVQL_CH_PASSWORD", None),
    ]);
    HomeEnvGuard {
        _lock_guard: lock_guard,
        previous_env,
    }
}

fn with_home_env<T>(home: &Path, f: impl FnOnce() -> T) -> T {
    let _guard = enter_home_env(home);
    f()
}

fn run_cmd_with_home(repo: &Path, home: &Path, args: &[&str], stdin: Option<&str>) -> Output {
    let mut cmd = Command::new(bitloops_bin());
    cmd.args(args).current_dir(repo);
    apply_home_env(&mut cmd, home);

    if let Some(input) = stdin {
        cmd.stdin(Stdio::piped());
        let mut child = cmd.spawn().expect("failed to spawn bitloops command");
        child
            .stdin
            .as_mut()
            .expect("stdin should be piped")
            .write_all(input.as_bytes())
            .expect("failed to write stdin");
        child.wait_with_output().expect("failed to wait for output")
    } else {
        cmd.output().expect("failed to execute bitloops command")
    }
}

fn assert_success(output: &Output, context: &str) {
    assert!(
        output.status.success(),
        "{context} failed\nstatus: {:?}\nstdout:\n{}\nstderr:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn run_git_output(repo: &Path, home: &Path, args: &[&str]) -> Output {
    let mut search_paths = vec![
        bitloops_bin()
            .parent()
            .expect("bitloops test binary should have a parent directory")
            .to_path_buf(),
    ];
    if let Some(existing_path) = env::var_os("PATH") {
        search_paths.extend(env::split_paths(&existing_path));
    }
    let path = env::join_paths(search_paths).expect("failed to construct PATH");

    let mut cmd = Command::new("git");
    cmd.args(args).current_dir(repo).env("PATH", path);
    apply_home_env(&mut cmd, home);
    cmd.output().expect("failed to run git")
}

fn run_git(repo: &Path, home: &Path, args: &[&str]) -> String {
    let out = run_git_output(repo, home, args);
    assert!(
        out.status.success(),
        "git {:?} failed\nstdout:\n{}\nstderr:\n{}",
        args,
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn run_git_expect_success(repo: &Path, home: &Path, args: &[&str], context: &str) -> Output {
    let out = run_git_output(repo, home, args);
    assert!(
        out.status.success(),
        "{context}\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    out
}

fn run_git_without_hooks_expect_success(
    repo: &Path,
    home: &Path,
    args: &[&str],
    context: &str,
) -> Output {
    let no_hooks_dir = home.join("git-hooks-disabled");
    fs::create_dir_all(&no_hooks_dir).expect("create empty hooks directory");

    let mut prefixed_args = Vec::with_capacity(args.len() + 2);
    prefixed_args.push("-c");
    let hooks_path_override = format!("core.hooksPath={}", no_hooks_dir.display());
    prefixed_args.push(hooks_path_override.as_str());
    prefixed_args.extend_from_slice(args);

    run_git_expect_success(repo, home, &prefixed_args, context)
}

fn write_repo_config(repo: &Path) {
    let config_path = repo.join(bitloops::config::BITLOOPS_CONFIG_RELATIVE_PATH);
    fs::write(
        &config_path,
        r#"[stores.relational]
sqlite_path = "stores/relational/relational.db"

[stores.events]
duckdb_path = "stores/event/events.duckdb"

[stores.blob]
local_path = "stores/blob"
"#,
    )
    .expect("write repo config");
    bitloops::config::settings::write_repo_daemon_binding(
        &repo.join(bitloops::config::REPO_POLICY_LOCAL_FILE_NAME),
        &config_path,
    )
    .expect("write repo daemon binding");
}

fn init_repo(repo: &Path, home: &Path) {
    run_git(repo, home, &["init"]);
    run_git(repo, home, &["config", "user.email", "t@t.com"]);
    run_git(repo, home, &["config", "user.name", "Test"]);
    run_git(repo, home, &["config", "commit.gpgsign", "false"]);
    run_git(repo, home, &["config", "tag.gpgsign", "false"]);
    write_repo_config(repo);
    fs::write(repo.join("README.md"), "init\n").unwrap();
    run_git(repo, home, &["add", "."]);
    run_git(repo, home, &["commit", "-m", "initial"]);
}

fn write_transcript(path: &Path, prompt: &str, response: &str, file_path: &str) {
    write_transcript_turn(path, prompt, response, file_path, false);
}

fn write_transcript_turn(path: &Path, prompt: &str, response: &str, file_path: &str, append: bool) {
    let payload = format!(
        r#"{{"type":"user.message","data":{{"content":{prompt_json}}}}}
{{"type":"tool.execution_complete","data":{{"model":"gpt-5","toolTelemetry":{{"properties":{{"filePaths":{file_paths_json}}}}}}}}}
{{"type":"assistant.message","data":{{"content":{response_json},"outputTokens":42}}}}
"#,
        prompt_json = serde_json::to_string(prompt).unwrap(),
        response_json = serde_json::to_string(response).unwrap(),
        file_paths_json = serde_json::to_string(&format!("[\"{file_path}\"]")).unwrap(),
    );

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    if append {
        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .unwrap();
        file.write_all(payload.as_bytes()).unwrap();
    } else {
        fs::write(path, payload).unwrap();
    }
}

fn append_transcript_shutdown(path: &Path) {
    let payload = r#"{"type":"session.shutdown","data":{"modelMetrics":[{"requests":{"count":1},"usage":{"inputTokens":100,"outputTokens":42,"cacheReadTokens":0,"cacheWriteTokens":0}}]}}
"#;
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .unwrap();
    file.write_all(payload.as_bytes()).unwrap();
}

fn checkpoint_sqlite_path(repo_root: &Path, _home: &Path) -> PathBuf {
    let cfg = resolve_store_backend_config_for_repo(repo_root).expect("resolve backend config");
    if let Some(path) = cfg.relational.sqlite_path.as_deref() {
        resolve_sqlite_db_path_for_repo(repo_root, Some(path)).expect("resolve sqlite path")
    } else {
        bitloops::utils::paths::default_relational_db_path(repo_root)
    }
}

fn runtime_sqlite_path(repo_root: &Path) -> PathBuf {
    bitloops::config::resolve_repo_runtime_db_path_for_repo(repo_root)
        .expect("resolve runtime sqlite path")
}

fn ensure_relational_store_file(repo_root: &Path, home: &Path) {
    let sqlite =
        bitloops::storage::SqliteConnectionPool::connect(checkpoint_sqlite_path(repo_root, home))
            .expect("create relational sqlite file");
    sqlite
        .initialise_checkpoint_schema()
        .expect("initialise checkpoint schema");
}

fn init(repo: &Path, home: &Path) {
    let _guard = enter_home_env(home);
    ensure_relational_store_file(repo, home);
    let policy_path = repo.join(bitloops::config::REPO_POLICY_LOCAL_FILE_NAME);
    bitloops::config::settings::write_project_bootstrap_settings(
        &policy_path,
        bitloops::config::settings::DEFAULT_STRATEGY,
        &[String::from("copilot")],
    )
    .expect("write project bootstrap settings");
    bitloops::adapters::agents::claude_code::git_hooks::install_git_hooks(repo, false)
        .expect("install git hooks");
    bitloops::adapters::agents::AgentAdapterRegistry::builtin()
        .install_agent_hooks(
            repo,
            "copilot",
            false,
            false,
            bitloops::adapters::agents::AgentHookInstallOptions::default(),
        )
        .expect("install Copilot hooks");
}

fn temporary_checkpoint_count(repo_root: &Path, home: &Path, session_id: &str) -> i64 {
    with_home_env(home, || {
        let conn = Connection::open(runtime_sqlite_path(repo_root)).expect("open runtime sqlite");
        conn.query_row(
            "SELECT COUNT(*) FROM temporary_checkpoints WHERE session_id = ?1",
            [session_id],
            |row| row.get(0),
        )
        .expect("count temporary checkpoints")
    })
}

#[test]
fn copilot_agent_stop_without_transcript_path_uses_session_fallback() {
    let dir = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    init_repo(dir.path(), home.path());

    init(dir.path(), home.path());
    run_git_expect_success(
        dir.path(),
        home.path(),
        &["add", ".github/hooks/bitloops.json"],
        "stage copilot hook file",
    );
    run_git_without_hooks_expect_success(
        dir.path(),
        home.path(),
        &["commit", "-m", "seed copilot infra"],
        "commit copilot infra files",
    );

    let sid = "copilot-fallback-1";
    let transcript_path = home
        .path()
        .join(".copilot")
        .join("session-state")
        .join(sid)
        .join("events.jsonl");

    assert_success(
        &run_cmd_with_home(
            dir.path(),
            home.path(),
            &["hooks", "copilot", "session-start"],
            Some(&format!(r#"{{"sessionId":"{sid}"}}"#)),
        ),
        "hooks copilot session-start",
    );
    assert_success(
        &run_cmd_with_home(
            dir.path(),
            home.path(),
            &["hooks", "copilot", "user-prompt-submitted"],
            Some(&format!(
                r#"{{"sessionId":"{sid}","prompt":"Create fallback.txt"}}"#
            )),
        ),
        "hooks copilot user-prompt-submitted",
    );

    fs::write(dir.path().join("fallback.txt"), "fallback\n").unwrap();
    write_transcript(
        &transcript_path,
        "Create fallback.txt",
        "Created fallback.txt",
        "fallback.txt",
    );

    assert_success(
        &run_cmd_with_home(
            dir.path(),
            home.path(),
            &["hooks", "copilot", "agent-stop"],
            Some(&format!(r#"{{"sessionId":"{sid}","transcriptPath":""}}"#)),
        ),
        "hooks copilot agent-stop",
    );
    assert_eq!(temporary_checkpoint_count(dir.path(), home.path(), sid), 1);

    assert_success(
        &run_cmd_with_home(
            dir.path(),
            home.path(),
            &["hooks", "copilot", "session-end"],
            Some(&format!(r#"{{"sessionId":"{sid}"}}"#)),
        ),
        "hooks copilot session-end",
    );

    run_git_expect_success(
        dir.path(),
        home.path(),
        &["add", "fallback.txt"],
        "stage fallback",
    );
    run_git_expect_success(
        dir.path(),
        home.path(),
        &["commit", "-m", "add fallback file"],
        "commit fallback",
    );

    let head_sha = run_git(dir.path(), home.path(), &["rev-parse", "HEAD"]);
    let mappings = with_home_env(home.path(), || {
        read_commit_checkpoint_mappings(dir.path()).expect("read mappings")
    });
    assert!(
        mappings.contains_key(&head_sha),
        "Copilot fallback transcript path should still produce a commit mapping"
    );
}

#[test]
fn copilot_session_start_initial_prompt_bootstraps_first_prompt() {
    let dir = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    init_repo(dir.path(), home.path());

    init(dir.path(), home.path());
    run_git_expect_success(
        dir.path(),
        home.path(),
        &["add", ".github/hooks/bitloops.json"],
        "stage copilot hook file",
    );
    run_git_without_hooks_expect_success(
        dir.path(),
        home.path(),
        &["commit", "-m", "seed copilot infra"],
        "commit copilot infra files",
    );

    let sid = "copilot-bootstrap-1";
    assert_success(
        &run_cmd_with_home(
            dir.path(),
            home.path(),
            &["hooks", "copilot", "session-start"],
            Some(&format!(
                r#"{{"sessionId":"{sid}","initialPrompt":"Bootstrap prompt"}}"#
            )),
        ),
        "hooks copilot session-start",
    );

    let backend = with_home_env(home.path(), || create_session_backend_or_local(dir.path()));
    let state = backend
        .load_session(sid)
        .expect("load session")
        .expect("session should exist");
    assert_eq!(state.first_prompt, "Bootstrap prompt");
}

#[test]
fn copilot_multi_turn_session_condenses_both_prompts() {
    let dir = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    init_repo(dir.path(), home.path());

    init(dir.path(), home.path());
    run_git_expect_success(
        dir.path(),
        home.path(),
        &["add", ".github/hooks/bitloops.json"],
        "stage copilot hook file",
    );
    run_git_without_hooks_expect_success(
        dir.path(),
        home.path(),
        &["commit", "-m", "seed copilot infra"],
        "commit copilot infra files",
    );

    let sid = "copilot-multi-turn-1";
    let transcript_path = home
        .path()
        .join(".copilot")
        .join("session-state")
        .join(sid)
        .join("events.jsonl");

    assert_success(
        &run_cmd_with_home(
            dir.path(),
            home.path(),
            &["hooks", "copilot", "session-start"],
            Some(&format!(r#"{{"sessionId":"{sid}"}}"#)),
        ),
        "hooks copilot session-start",
    );

    assert_success(
        &run_cmd_with_home(
            dir.path(),
            home.path(),
            &["hooks", "copilot", "user-prompt-submitted"],
            Some(&format!(
                r#"{{"sessionId":"{sid}","prompt":"Create first.txt"}}"#
            )),
        ),
        "hooks copilot user-prompt-submitted first",
    );
    fs::write(dir.path().join("first.txt"), "first\n").unwrap();
    write_transcript_turn(
        &transcript_path,
        "Create first.txt",
        "Created first.txt",
        "first.txt",
        false,
    );
    assert_success(
        &run_cmd_with_home(
            dir.path(),
            home.path(),
            &["hooks", "copilot", "agent-stop"],
            Some(&format!(
                r#"{{"sessionId":"{sid}","transcriptPath":"{}"}}"#,
                transcript_path.to_string_lossy()
            )),
        ),
        "hooks copilot agent-stop first",
    );

    assert_success(
        &run_cmd_with_home(
            dir.path(),
            home.path(),
            &["hooks", "copilot", "user-prompt-submitted"],
            Some(&format!(
                r#"{{"sessionId":"{sid}","prompt":"Create second.txt"}}"#
            )),
        ),
        "hooks copilot user-prompt-submitted second",
    );
    fs::write(dir.path().join("second.txt"), "second\n").unwrap();
    write_transcript_turn(
        &transcript_path,
        "Create second.txt",
        "Created second.txt",
        "second.txt",
        true,
    );
    append_transcript_shutdown(&transcript_path);
    assert_success(
        &run_cmd_with_home(
            dir.path(),
            home.path(),
            &["hooks", "copilot", "agent-stop"],
            Some(&format!(
                r#"{{"sessionId":"{sid}","transcriptPath":"{}"}}"#,
                transcript_path.to_string_lossy()
            )),
        ),
        "hooks copilot agent-stop second",
    );

    assert_success(
        &run_cmd_with_home(
            dir.path(),
            home.path(),
            &["hooks", "copilot", "session-end"],
            Some(&format!(r#"{{"sessionId":"{sid}"}}"#)),
        ),
        "hooks copilot session-end",
    );

    run_git_expect_success(
        dir.path(),
        home.path(),
        &["add", "first.txt", "second.txt"],
        "stage multi-turn files",
    );
    run_git_expect_success(
        dir.path(),
        home.path(),
        &["commit", "-m", "add multi-turn files"],
        "commit multi-turn files",
    );

    let head_sha = run_git(dir.path(), home.path(), &["rev-parse", "HEAD"]);
    let mappings = with_home_env(home.path(), || {
        read_commit_checkpoint_mappings(dir.path()).expect("read mappings")
    });
    let checkpoint_id = mappings
        .get(&head_sha)
        .cloned()
        .expect("checkpoint mapping should exist");
    let session = with_home_env(home.path(), || {
        read_session_content(dir.path(), &checkpoint_id, 0).expect("read session")
    });
    assert!(session.prompts.contains("Create first.txt"));
    assert!(session.prompts.contains("Create second.txt"));

    let summary = with_home_env(home.path(), || {
        read_committed(dir.path(), &checkpoint_id)
            .expect("read committed checkpoint")
            .expect("committed checkpoint should exist")
    });
    assert!(summary.files_touched.contains(&"first.txt".to_string()));
    assert!(summary.files_touched.contains(&"second.txt".to_string()));
}
