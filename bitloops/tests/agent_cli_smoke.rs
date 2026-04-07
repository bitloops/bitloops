mod agent_smoke_support;
mod test_command_support;

use agent_smoke_support::{
    checkpoint_id_for_head, claude_stop, claude_user_prompt_submit, committed_content,
    committed_summary, init_and_enable_cursor, init_claude, init_repo, run_git_expect_success,
    write_claude_transcript, write_cursor_transcript,
};
use bitloops::cli::versioncheck::DISABLE_VERSION_CHECK_ENV;
use bitloops::config::{resolve_sqlite_db_path_for_repo, resolve_store_backend_config_for_repo};
use bitloops::host::checkpoints::session::create_session_backend_or_local;
use bitloops::host::checkpoints::session::phase::SessionPhase;
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

#[test]
fn claude_cli_smoke_condenses_checkpoint_on_commit() {
    let dir = tempfile::tempdir().expect("tempdir");
    init_repo(dir.path());
    init_claude(dir.path());

    let sid = "claude-smoke-1";
    let transcript_path = dir.path().join("claude-smoke.jsonl");
    claude_user_prompt_submit(
        dir.path(),
        sid,
        transcript_path.to_string_lossy().as_ref(),
        "Create auth module",
    );
    fs::create_dir_all(dir.path().join("src")).expect("create src dir");
    fs::write(
        dir.path().join("src/auth.rs"),
        "package auth\n\nfunc Auth() bool { return true }\n",
    )
    .expect("write auth module");
    write_claude_transcript(
        &transcript_path,
        "Create auth module",
        "Implemented src/auth.rs",
    );
    claude_stop(dir.path(), sid, transcript_path.to_string_lossy().as_ref());

    run_git_expect_success(dir.path(), &["add", "src/auth.rs"], "stage auth module");
    run_git_expect_success(
        dir.path(),
        &["commit", "-m", "feat: add auth module"],
        "commit auth module",
    );

    let checkpoint_id =
        checkpoint_id_for_head(dir.path()).expect("HEAD commit should map to a checkpoint");
    let summary = committed_summary(dir.path(), &checkpoint_id);
    assert!(
        summary["files_touched"]
            .as_array()
            .is_some_and(|files| files.iter().any(|f| f.as_str() == Some("src/auth.rs"))),
        "checkpoint should include src/auth.rs in files_touched"
    );
    let content = committed_content(dir.path(), &checkpoint_id, 0);
    assert!(
        content["prompts"]
            .as_str()
            .is_some_and(|prompts| prompts.contains("Create auth module")),
        "checkpoint prompts should include the Claude prompt"
    );
}

#[test]
fn cursor_cli_smoke_maps_basic_workflow_commit() {
    let dir = tempfile::tempdir().expect("tempdir");
    init_repo(dir.path());
    init_and_enable_cursor(dir.path());

    let sid = "cursor-smoke-1";
    let transcript_path = dir.path().join("cursor-smoke.jsonl");
    agent_smoke_support::cursor_before_submit_prompt(
        dir.path(),
        sid,
        transcript_path.to_string_lossy().as_ref(),
        "Create cursor_hello.rs",
    );
    fs::write(
        dir.path().join("cursor_hello.rs"),
        "package main\n\nfunc CursorHello() {}\n",
    )
    .expect("write cursor file");
    write_cursor_transcript(
        &transcript_path,
        "Create cursor_hello.rs",
        "Created cursor_hello.rs",
    );
    agent_smoke_support::cursor_stop(dir.path(), sid, transcript_path.to_string_lossy().as_ref());

    run_git_expect_success(
        dir.path(),
        &["add", "cursor_hello.rs"],
        "stage cursor_hello.rs",
    );
    run_git_expect_success(
        dir.path(),
        &["commit", "-m", "add cursor hello file"],
        "commit cursor_hello.rs",
    );

    assert!(
        checkpoint_id_for_head(dir.path()).is_some(),
        "Cursor workflow commit should map HEAD to a checkpoint"
    );
}

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

fn assert_home_success(output: &Output, context: &str) {
    assert!(
        output.status.success(),
        "{context} failed\nstatus: {:?}\nstdout:\n{}\nstderr:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn run_git_output_with_home(repo: &Path, home: &Path, args: &[&str]) -> Output {
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

fn run_git_with_home(repo: &Path, home: &Path, args: &[&str]) -> String {
    let out = run_git_output_with_home(repo, home, args);
    assert!(
        out.status.success(),
        "git {:?} failed\nstdout:\n{}\nstderr:\n{}",
        args,
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn run_git_expect_success_with_home(repo: &Path, home: &Path, args: &[&str], context: &str) {
    let out = run_git_output_with_home(repo, home, args);
    assert!(
        out.status.success(),
        "{context}\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

fn run_git_without_hooks_expect_success_with_home(
    repo: &Path,
    home: &Path,
    args: &[&str],
    context: &str,
) {
    let no_hooks_dir = home.join("git-hooks-disabled");
    fs::create_dir_all(&no_hooks_dir).expect("create empty hooks directory");

    let mut prefixed_args = Vec::with_capacity(args.len() + 2);
    prefixed_args.push("-c");
    let hooks_path_override = format!("core.hooksPath={}", no_hooks_dir.display());
    prefixed_args.push(hooks_path_override.as_str());
    prefixed_args.extend_from_slice(args);

    run_git_expect_success_with_home(repo, home, &prefixed_args, context);
}

fn write_repo_config(repo: &Path) {
    fs::write(
        repo.join(bitloops::config::BITLOOPS_CONFIG_RELATIVE_PATH),
        r#"[stores.relational]
sqlite_path = "stores/relational/relational.db"

[stores.events]
duckdb_path = "stores/event/events.duckdb"

[stores.blob]
local_path = "stores/blob"
"#,
    )
    .expect("write repo config");
}

fn init_repo_with_home(repo: &Path, home: &Path) {
    run_git_with_home(repo, home, &["init"]);
    run_git_with_home(repo, home, &["config", "user.email", "t@t.com"]);
    run_git_with_home(repo, home, &["config", "user.name", "Test"]);
    run_git_with_home(repo, home, &["config", "commit.gpgsign", "false"]);
    run_git_with_home(repo, home, &["config", "tag.gpgsign", "false"]);
    write_repo_config(repo);
    fs::write(repo.join("README.md"), "init\n").expect("write README");
    run_git_with_home(repo, home, &["add", "."]);
    run_git_with_home(repo, home, &["commit", "-m", "initial"]);
}

fn write_copilot_transcript(path: &Path, prompt: &str, response: &str, file_path: &str) {
    let payload = format!(
        r#"{{"type":"user.message","data":{{"content":{prompt_json}}}}}
{{"type":"tool.execution_complete","data":{{"model":"gpt-5","toolTelemetry":{{"properties":{{"filePaths":{file_paths_json}}}}}}}}}
{{"type":"assistant.message","data":{{"content":{response_json},"outputTokens":42}}}}
"#,
        prompt_json = serde_json::to_string(prompt).expect("serialize Copilot prompt"),
        response_json = serde_json::to_string(response).expect("serialize Copilot response"),
        file_paths_json = serde_json::to_string(&format!("[\"{file_path}\"]"))
            .expect("serialize Copilot file paths"),
    );
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("create Copilot transcript parent");
    }
    fs::write(path, payload).expect("write Copilot transcript");
}

fn runtime_sqlite_path(repo_root: &Path) -> PathBuf {
    bitloops::config::resolve_repo_runtime_db_path_for_repo(repo_root)
        .expect("resolve runtime sqlite path")
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

fn ensure_relational_store_file(repo_root: &Path, _home: &Path) {
    let cfg = resolve_store_backend_config_for_repo(repo_root).expect("resolve backend config");
    let sqlite_path = if let Some(path) = cfg.relational.sqlite_path.as_deref() {
        resolve_sqlite_db_path_for_repo(repo_root, Some(path)).expect("resolve sqlite path")
    } else {
        bitloops::utils::paths::default_relational_db_path(repo_root)
    };
    let sqlite = bitloops::storage::SqliteConnectionPool::connect(sqlite_path)
        .expect("create relational sqlite file");
    sqlite
        .initialise_checkpoint_schema()
        .expect("initialise checkpoint schema");
}

fn init_copilot(repo: &Path, home: &Path) {
    let _guard = enter_home_env(home);
    ensure_relational_store_file(repo, home);
    let policy_path = repo.join(bitloops::config::REPO_POLICY_LOCAL_FILE_NAME);
    bitloops::config::settings::write_project_bootstrap_settings(
        &policy_path,
        bitloops::config::settings::DEFAULT_STRATEGY,
        &[String::from("copilot")],
    )
    .expect("write Copilot project bootstrap settings");
    bitloops::adapters::agents::claude_code::git_hooks::install_git_hooks(repo, false)
        .expect("install git hooks");
    bitloops::adapters::agents::AgentAdapterRegistry::builtin()
        .install_agent_hooks(repo, "copilot", false, false)
        .expect("install Copilot hooks");
}

#[test]
fn copilot_cli_smoke_maps_basic_workflow_commit() {
    let dir = tempfile::tempdir().expect("tempdir");
    let home = tempfile::tempdir().expect("home tempdir");
    init_repo_with_home(dir.path(), home.path());
    init_copilot(dir.path(), home.path());

    run_git_expect_success_with_home(
        dir.path(),
        home.path(),
        &["add", ".github/hooks/bitloops.json"],
        "stage Copilot hook file",
    );
    run_git_without_hooks_expect_success_with_home(
        dir.path(),
        home.path(),
        &["commit", "-m", "seed copilot bitloops infra files"],
        "commit Copilot infra files",
    );

    let sid = "copilot-smoke-1";
    let transcript_path = home
        .path()
        .join(".copilot")
        .join("session-state")
        .join(sid)
        .join("events.jsonl");

    let out = run_cmd_with_home(
        dir.path(),
        home.path(),
        &["hooks", "copilot", "session-start"],
        Some(r#"{"sessionId":"copilot-smoke-1"}"#),
    );
    assert_home_success(&out, "hooks copilot session-start");

    let out = run_cmd_with_home(
        dir.path(),
        home.path(),
        &["hooks", "copilot", "user-prompt-submitted"],
        Some(r#"{"sessionId":"copilot-smoke-1","prompt":"Create copilot_hello.txt"}"#),
    );
    assert_home_success(&out, "hooks copilot user-prompt-submitted");

    fs::write(dir.path().join("copilot_hello.txt"), "hello from copilot\n")
        .expect("write Copilot output");
    write_copilot_transcript(
        &transcript_path,
        "Create copilot_hello.txt",
        "Created copilot_hello.txt",
        "copilot_hello.txt",
    );

    let stop_input = format!(
        r#"{{"sessionId":"{sid}","transcriptPath":"{}"}}"#,
        transcript_path.to_string_lossy()
    );
    let out = run_cmd_with_home(
        dir.path(),
        home.path(),
        &["hooks", "copilot", "agent-stop"],
        Some(&stop_input),
    );
    assert_home_success(&out, "hooks copilot agent-stop");

    assert_eq!(
        temporary_checkpoint_count(dir.path(), home.path(), sid),
        1,
        "Copilot turn end should write one temporary checkpoint before commit"
    );

    let out = run_cmd_with_home(
        dir.path(),
        home.path(),
        &["hooks", "copilot", "session-end"],
        Some(r#"{"sessionId":"copilot-smoke-1"}"#),
    );
    assert_home_success(&out, "hooks copilot session-end");

    run_git_expect_success_with_home(
        dir.path(),
        home.path(),
        &["add", "copilot_hello.txt"],
        "stage Copilot output",
    );
    run_git_expect_success_with_home(
        dir.path(),
        home.path(),
        &["commit", "-m", "add copilot hello file"],
        "commit Copilot output",
    );

    let head_sha = run_git_with_home(dir.path(), home.path(), &["rev-parse", "HEAD"]);
    let checkpoint_id = with_home_env(home.path(), || {
        read_commit_checkpoint_mappings(dir.path())
            .expect("read commit-checkpoint mappings")
            .get(&head_sha)
            .cloned()
    })
    .expect("Copilot workflow should map HEAD to a committed checkpoint");

    let summary = with_home_env(home.path(), || {
        read_committed(dir.path(), &checkpoint_id)
            .expect("read committed checkpoint")
            .expect("committed checkpoint should exist")
    });
    assert!(
        summary
            .files_touched
            .contains(&"copilot_hello.txt".to_string()),
        "committed checkpoint should include Copilot-created file"
    );

    let session = with_home_env(home.path(), || {
        read_session_content(dir.path(), &checkpoint_id, 0).expect("read session content")
    });
    assert_eq!(session.metadata["agent"], "copilot");

    let backend = with_home_env(home.path(), || create_session_backend_or_local(dir.path()));
    let state = backend
        .load_session("copilot-smoke-1")
        .expect("load Copilot session")
        .expect("Copilot session should exist");
    assert_eq!(state.phase, SessionPhase::Ended);
}
