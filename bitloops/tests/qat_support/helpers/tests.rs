use super::*;
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::{Duration as StdDuration, Instant};

use crate::qat_support::world::QatRunConfig;
use bitloops::cli::versioncheck::DISABLE_VERSION_CHECK_ENV;
use bitloops::daemon::CapabilityEventRunStatus;
use bitloops::host::devql::watch::DISABLE_WATCHER_AUTOSTART_ENV;
use bitloops::host::interactions::types::{InteractionSession, InteractionTurn};
use bitloops::host::runtime_store::{DaemonSqliteRuntimeStore, RepoWatcherRegistrationState};

#[cfg(unix)]
fn spawn_detached_long_lived_process() -> u32 {
    let output = Command::new("sh")
        .args(["-c", "sleep 60 >/dev/null 2>&1 & echo $!"])
        .output()
        .expect("spawn detached long-lived process");
    assert!(
        output.status.success(),
        "failed to spawn detached long-lived process: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .expect("detached pid stdout should be utf8")
        .trim()
        .parse()
        .expect("detached pid should parse")
}

#[cfg(windows)]
fn spawn_detached_long_lived_process() -> u32 {
    let output = Command::new("powershell")
        .args([
            "-NoProfile",
            "-Command",
            "$p = Start-Process -FilePath ping -ArgumentList '-n 60 127.0.0.1' -WindowStyle Hidden -PassThru; $p.Id",
        ])
        .output()
        .expect("spawn detached long-lived process");
    assert!(
        output.status.success(),
        "failed to spawn detached long-lived process: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .expect("detached pid stdout should be utf8")
        .trim()
        .parse()
        .expect("detached pid should parse")
}

#[cfg(unix)]
fn pid_is_running(pid: u32) -> bool {
    Command::new("kill")
        .args(["-0", &pid.to_string()])
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

#[cfg(windows)]
fn pid_is_running(pid: u32) -> bool {
    Command::new("cmd")
        .args([
            "/C",
            &format!("tasklist /FI \"PID eq {pid}\" | findstr {pid}"),
        ])
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

fn wait_for_pid_exit(pid: u32) {
    let deadline = Instant::now() + StdDuration::from_secs(5);
    loop {
        if !pid_is_running(pid) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "expected long-lived process {pid} to exit after teardown",
        );
        std::thread::sleep(StdDuration::from_millis(25));
    }
}

#[test]
fn sanitize_name_normalizes_user_input() {
    assert_eq!(
        sanitize_name("BDD Foundation: Stores"),
        "bdd-foundation-stores"
    );
    assert_eq!(sanitize_name(" Already__Slugged "), "already-slugged");
}

#[test]
fn git_date_for_relative_day_uses_stable_noon_timestamp() {
    let today = git_date_for_relative_day(0).expect("today git date");
    let yesterday = git_date_for_relative_day(1).expect("yesterday git date");

    assert!(today.ends_with('Z') || today.contains("+00:00"));
    assert!(yesterday.ends_with('Z') || yesterday.contains("+00:00"));
    assert_ne!(today[..10].to_string(), yesterday[..10].to_string());
    assert!(today.contains("12:00:00"));
    assert!(yesterday.contains("12:00:00"));
}

#[test]
fn offline_vite_scaffold_writes_expected_files() {
    let dir = tempfile::tempdir().expect("tempdir");
    create_offline_vite_react_ts_scaffold(dir.path()).expect("create scaffold");

    assert!(dir.path().join("my-app").join("package.json").exists());
    assert!(dir.path().join("my-app").join("index.html").exists());
    assert!(
        dir.path()
            .join("my-app")
            .join("src")
            .join("App.tsx")
            .exists()
    );
    assert!(
        dir.path()
            .join("my-app")
            .join("src")
            .join("main.tsx")
            .exists()
    );
}

#[test]
fn shell_single_quote_escapes_single_quotes() {
    assert_eq!(shell_single_quote("plain"), "'plain'");
    assert_eq!(shell_single_quote("it's ok"), "'it'\"'\"'s ok'");
}

#[test]
fn parse_timeout_seconds_uses_default_for_invalid_values() {
    assert_eq!(
        parse_timeout_seconds(None, 120).as_secs(),
        120,
        "missing value should use default"
    );
    assert_eq!(
        parse_timeout_seconds(Some(""), 120).as_secs(),
        120,
        "empty value should use default"
    );
    assert_eq!(
        parse_timeout_seconds(Some("0"), 120).as_secs(),
        120,
        "zero should use default"
    );
    assert_eq!(
        parse_timeout_seconds(Some("abc"), 120).as_secs(),
        120,
        "non-numeric should use default"
    );
}

#[test]
fn parse_timeout_seconds_accepts_positive_seconds() {
    assert_eq!(
        parse_timeout_seconds(Some("5"), 120).as_secs(),
        5,
        "positive value should be used"
    );
}

#[test]
fn parse_claude_auth_logged_in_reads_boolean_field() {
    let logged_in = r#"{"loggedIn":true,"authMethod":"oauth"}"#;
    let logged_out = r#"{"loggedIn":false,"authMethod":"none"}"#;

    assert_eq!(parse_claude_auth_logged_in(logged_in), Some(true));
    assert_eq!(parse_claude_auth_logged_in(logged_out), Some(false));
}

#[test]
fn text_has_claude_auth_failure_detects_auth_prompts() {
    assert!(text_has_claude_auth_failure(
        "Not logged in · Please run /login"
    ));
    assert!(text_has_claude_auth_failure("Authentication required"));
    assert!(!text_has_claude_auth_failure("all good"));
}

#[test]
fn text_has_missing_production_artefacts_error_detects_relational_materialization_failures() {
    assert!(text_has_missing_production_artefacts_error(
        "Error: no production artefacts found for commit abc123; materialize production artefacts first"
    ));
    assert!(!text_has_missing_production_artefacts_error("all good"));
}

#[test]
fn error_chain_contains_not_found_detects_missing_binary_errors() {
    let err = anyhow::Error::from(std::io::Error::new(
        std::io::ErrorKind::NotFound,
        "No such file or directory",
    ))
    .context("executing bitloops daemon stop");

    assert!(error_chain_contains_not_found(&err));
}

#[test]
fn error_chain_contains_not_found_ignores_other_io_errors() {
    let err = anyhow::Error::from(std::io::Error::new(
        std::io::ErrorKind::PermissionDenied,
        "Permission denied",
    ))
    .context("executing bitloops daemon stop");

    assert!(!error_chain_contains_not_found(&err));
}

#[test]
fn build_init_bitloops_args_defaults_to_sync_false_when_unspecified() {
    let args = build_init_bitloops_args("claude-code", false, None);
    assert_eq!(
        args,
        vec![
            "init",
            "--agent",
            "claude-code",
            "--sync=false",
            "--ingest=false",
        ]
    );
}

#[test]
fn build_init_bitloops_args_supports_sync_false_choice() {
    let args = build_init_bitloops_args("claude-code", false, Some(false));
    assert_eq!(
        args,
        vec![
            "init",
            "--agent",
            "claude-code",
            "--sync=false",
            "--ingest=false",
        ]
    );
}

#[test]
fn build_init_bitloops_args_supports_sync_true_choice_and_force() {
    let args = build_init_bitloops_args("codex", true, Some(true));
    assert_eq!(
        args,
        vec![
            "init",
            "--agent",
            "codex",
            "--sync=true",
            "--ingest=false",
            "--force",
        ]
    );
}

#[test]
fn build_init_bitloops_args_with_backfill_enables_ingest_and_sets_window() {
    let args = build_init_bitloops_args_with_options(
        &["claude-code"],
        false,
        Some(false),
        Some(false),
        Some(2),
    );
    assert_eq!(
        args,
        vec![
            "init",
            "--agent",
            "claude-code",
            "--sync=false",
            "--ingest=true",
            "--backfill=2",
        ]
    );
}

#[test]
fn build_init_bitloops_args_supports_repeated_agent_flags() {
    let args =
        build_init_bitloops_args_with_options(&["claude-code", "codex"], false, None, None, None);
    assert_eq!(
        args,
        vec![
            "init",
            "--agent",
            "claude-code",
            "--agent",
            "codex",
            "--sync=false",
            "--ingest=false",
        ]
    );
}

#[test]
fn parse_ingest_summary_field_reads_key_value_pairs() {
    let stdout = "DevQL ingest complete: commits_processed=0, checkpoint_companions_processed=0, events_inserted=0, artefacts_upserted=0";
    assert_eq!(
        parse_ingest_summary_field(stdout, "commits_processed"),
        Some(0)
    );
    assert_eq!(
        parse_ingest_summary_field(stdout, "artefacts_upserted"),
        Some(0)
    );
}

#[test]
fn build_devql_task_enqueue_args_builds_sync_command_with_flags() {
    let args = build_devql_task_enqueue_args(DevqlTaskEnqueueKind::Sync, &["--repair", "--status"]);
    assert_eq!(
        args,
        vec![
            "devql".to_string(),
            "tasks".to_string(),
            "enqueue".to_string(),
            "--kind".to_string(),
            "sync".to_string(),
            "--repair".to_string(),
            "--status".to_string(),
        ]
    );
}

#[test]
fn build_devql_task_enqueue_args_builds_ingest_command_with_status() {
    let args = build_devql_task_enqueue_args(DevqlTaskEnqueueKind::Ingest, &["--status"]);
    assert_eq!(
        args,
        vec![
            "devql".to_string(),
            "tasks".to_string(),
            "enqueue".to_string(),
            "--kind".to_string(),
            "ingest".to_string(),
            "--status".to_string(),
        ]
    );
}

#[test]
fn build_devql_task_enqueue_args_supports_paths_and_require_daemon_flags() {
    let args = build_devql_task_enqueue_args(
        DevqlTaskEnqueueKind::Sync,
        &[
            "--paths",
            "src/main.rs,src/lib.rs",
            "--require-daemon",
            "--status",
        ],
    );
    assert_eq!(
        args,
        vec![
            "devql".to_string(),
            "tasks".to_string(),
            "enqueue".to_string(),
            "--kind".to_string(),
            "sync".to_string(),
            "--paths".to_string(),
            "src/main.rs,src/lib.rs".to_string(),
            "--require-daemon".to_string(),
            "--status".to_string(),
        ]
    );
}

#[test]
fn parse_task_id_from_submission_extracts_queue_id() {
    let stdout = "task queued: task=task-123 repo=bitloops kind=sync";
    assert_eq!(
        parse_task_id_from_submission(stdout),
        Some("task-123".to_string())
    );
}

#[test]
fn update_last_task_id_from_output_preserves_existing_task_for_read_only_task_output() {
    let mut world = QatWorld {
        last_task_id: Some("ingest-task-123".to_string()),
        ..Default::default()
    };

    update_last_task_id_from_output(
        &mut world,
        "task sync-task-999: kind=sync status=running repo=bitloops",
        TaskIdCaptureMode::PreserveExisting,
    );

    assert_eq!(world.last_task_id.as_deref(), Some("ingest-task-123"));
}

#[test]
fn update_last_task_id_from_output_captures_new_submission_ids() {
    let mut world = QatWorld {
        last_task_id: Some("sync-task-old".to_string()),
        ..Default::default()
    };

    update_last_task_id_from_output(
        &mut world,
        "task queued: task=ingest-task-456 repo=bitloops kind=ingest",
        TaskIdCaptureMode::CaptureSubmission,
    );

    assert_eq!(world.last_task_id.as_deref(), Some("ingest-task-456"));
}

#[test]
fn update_last_task_id_from_output_read_only_mode_can_seed_empty_world_once() {
    let mut world = QatWorld::default();

    update_last_task_id_from_output(
        &mut world,
        "task ingest-task-456: kind=ingest status=queued repo=bitloops",
        TaskIdCaptureMode::PreserveExisting,
    );

    assert_eq!(world.last_task_id.as_deref(), Some("ingest-task-456"));
}

#[test]
fn parse_task_briefs_reads_task_summary_lines() {
    let stdout = "task task-123: kind=sync status=completed repo=bitloops\n\
task task-456: kind=ingest status=queued repo=bitloops";
    assert_eq!(
        parse_task_briefs(stdout),
        vec![
            DevqlTaskBriefRecord {
                task_id: "task-123".to_string(),
                kind: "sync".to_string(),
                status: "completed".to_string(),
                repo: "bitloops".to_string(),
            },
            DevqlTaskBriefRecord {
                task_id: "task-456".to_string(),
                kind: "ingest".to_string(),
                status: "queued".to_string(),
                repo: "bitloops".to_string(),
            },
        ]
    );
}

#[test]
fn parse_task_queue_status_reads_status_fields_and_current_repo_tasks() {
    let stdout = "DevQL task queue\n\
state: paused\n\
queued: 1\n\
running: 0\n\
failed: 0\n\
completed_recent: 2\n\
pause_reason: qat-cancel\n\
last_action: pause\n\
kind=sync queued=1 running=0 failed=0 completed_recent=2\n\
current_repo_tasks:\n\
task task-123: kind=sync status=queued repo=bitloops\n";
    assert_eq!(
        parse_task_queue_status(stdout).expect("parse queue status"),
        DevqlTaskQueueStatusSnapshot {
            state: "paused".to_string(),
            queued: 1,
            running: 0,
            failed: 0,
            completed_recent: 2,
            pause_reason: Some("qat-cancel".to_string()),
            last_action: Some("pause".to_string()),
            current_repo_tasks: vec![DevqlTaskBriefRecord {
                task_id: "task-123".to_string(),
                kind: "sync".to_string(),
                status: "queued".to_string(),
                repo: "bitloops".to_string(),
            }],
        }
    );
}

#[test]
fn devql_task_queue_status_is_idle_requires_zero_queued_and_running_tasks() {
    let busy = DevqlTaskQueueStatusSnapshot {
        state: "running".to_string(),
        queued: 1,
        running: 0,
        failed: 0,
        completed_recent: 0,
        pause_reason: None,
        last_action: Some("enqueue".to_string()),
        current_repo_tasks: vec![],
    };
    let idle = DevqlTaskQueueStatusSnapshot {
        queued: 0,
        running: 0,
        ..busy.clone()
    };

    assert!(!devql_task_queue_status_is_idle(&busy));
    assert!(devql_task_queue_status_is_idle(&idle));
}

#[test]
fn assert_last_task_id_matches_kind_accepts_matching_ingest_id() {
    let world = QatWorld {
        last_task_id: Some("ingest-task-123".to_string()),
        ..Default::default()
    };

    assert!(assert_last_task_id_matches_kind(&world, "ingest").is_ok());
}

#[test]
fn assert_last_task_id_matches_kind_rejects_mismatched_kind() {
    let world = QatWorld {
        last_task_id: Some("sync-task-999".to_string()),
        ..Default::default()
    };

    let err = assert_last_task_id_matches_kind(&world, "ingest")
        .expect_err("mismatched kind should fail");
    assert!(
        err.to_string()
            .contains("expected tracked DevQL task kind `ingest`")
    );
}

#[test]
fn render_guide_aligned_semantic_clones_config_uses_auto_summary_fake_profile_and_two_workers() {
    let temp = tempfile::tempdir().expect("tempdir");
    let repo_dir = temp.path().join("repo");
    fs::create_dir_all(&repo_dir).expect("create repo dir");
    let bin_dir = temp.path().join("bin");
    fs::create_dir_all(&bin_dir).expect("create bin dir");
    let suite_root = temp.path().join("suite");
    fs::create_dir_all(&suite_root).expect("create suite root");

    let world = QatWorld {
        run_dir: Some(temp.path().join("run")),
        repo_dir: Some(repo_dir),
        run_config: Some(Arc::new(QatRunConfig {
            binary_path: bin_dir.join("bitloops"),
            suite_root,
        })),
        ..Default::default()
    };

    let config =
        render_guide_aligned_semantic_clones_config(&world, "sh", &["fake-runtime.sh".to_string()]);

    assert!(config.contains("summary_mode = \"auto\""));
    assert!(config.contains("embedding_mode = \"deterministic\""));
    assert!(config.contains("enrichment_workers = 2"));
    assert!(config.contains("[semantic_clones.inference]"));
    assert!(config.contains("code_embeddings = \"fake\""));
    assert!(config.contains("summary_embeddings = \"fake\""));
    assert!(config.contains("[inference.profiles.fake]"));
    assert!(config.contains("driver = \"bitloops_embeddings_ipc\""));
    assert!(config.contains("model = \"qat-test-model\""));
}

#[test]
fn build_git_command_prepends_qat_binary_dir_to_path() {
    let temp = tempfile::tempdir().expect("tempdir");
    let repo_dir = temp.path().join("repo");
    fs::create_dir_all(&repo_dir).expect("create repo dir");
    let bin_dir = temp.path().join("bin");
    fs::create_dir_all(&bin_dir).expect("create bin dir");
    let suite_root = temp.path().join("suite");
    fs::create_dir_all(&suite_root).expect("create suite root");

    let world = QatWorld {
        run_dir: Some(temp.path().join("run")),
        repo_dir: Some(repo_dir.clone()),
        run_config: Some(Arc::new(QatRunConfig {
            binary_path: bin_dir.join("bitloops"),
            suite_root,
        })),
        ..Default::default()
    };

    let command = build_git_command(&world, &["status"], &[]);
    let path_value = command
        .get_envs()
        .find_map(|(key, value)| {
            if key == OsStr::new("PATH") {
                value.map(|v| v.to_os_string())
            } else {
                None
            }
        })
        .expect("build_git_command should set PATH");

    let mut paths = std::env::split_paths(&path_value);
    let first = paths.next().expect("PATH should have at least one entry");
    assert_eq!(first, bin_dir);
}

#[test]
fn build_git_command_excludes_bitloops_stores_from_git_add_all() {
    let temp = tempfile::tempdir().expect("tempdir");
    let repo_dir = temp.path().join("repo");
    fs::create_dir_all(&repo_dir).expect("create repo dir");
    let bin_dir = temp.path().join("bin");
    fs::create_dir_all(&bin_dir).expect("create bin dir");
    let suite_root = temp.path().join("suite");
    fs::create_dir_all(&suite_root).expect("create suite root");

    let world = QatWorld {
        run_dir: Some(temp.path().join("run")),
        repo_dir: Some(repo_dir),
        run_config: Some(Arc::new(QatRunConfig {
            binary_path: bin_dir.join("bitloops"),
            suite_root,
        })),
        ..Default::default()
    };

    let args = build_git_command(&world, &["add", "-A"], &[])
        .get_args()
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect::<Vec<_>>();

    assert_eq!(
        args,
        vec!["add", "-A", "--", ".", ":(exclude).bitloops/stores"]
    );
}

#[test]
fn build_commit_without_hooks_command_disables_post_commit_refresh() {
    let temp = tempfile::tempdir().expect("tempdir");
    let repo_dir = temp.path().join("repo");
    fs::create_dir_all(&repo_dir).expect("create repo dir");
    let bin_dir = temp.path().join("bin");
    fs::create_dir_all(&bin_dir).expect("create bin dir");
    let suite_root = temp.path().join("suite");
    fs::create_dir_all(&suite_root).expect("create suite root");

    let world = QatWorld {
        run_dir: Some(temp.path().join("run")),
        repo_dir: Some(repo_dir),
        run_config: Some(Arc::new(QatRunConfig {
            binary_path: bin_dir.join("bitloops"),
            suite_root,
        })),
        ..Default::default()
    };

    let command = build_commit_without_hooks_command(&world, true);
    let args = command
        .get_args()
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    let disable_refresh = command
        .get_envs()
        .find_map(|(key, value)| {
            if key == OsStr::new("BITLOOPS_DISABLE_POST_COMMIT_DEVQL_REFRESH") {
                value.map(|v| v.to_string_lossy().into_owned())
            } else {
                None
            }
        })
        .expect("commit without hooks should disable post-commit refresh");

    assert_eq!(
        args,
        vec!["commit", "--allow-empty", "-m", "QAT change (no hooks)"]
    );
    assert_eq!(disable_refresh, "1");
}

#[test]
fn build_init_commit_without_post_commit_refresh_command_disables_post_commit_refresh() {
    let temp = tempfile::tempdir().expect("tempdir");
    let repo_dir = temp.path().join("repo");
    fs::create_dir_all(&repo_dir).expect("create repo dir");
    let bin_dir = temp.path().join("bin");
    fs::create_dir_all(&bin_dir).expect("create bin dir");
    let suite_root = temp.path().join("suite");
    fs::create_dir_all(&suite_root).expect("create suite root");

    let world = QatWorld {
        run_dir: Some(temp.path().join("run")),
        repo_dir: Some(repo_dir),
        run_config: Some(Arc::new(QatRunConfig {
            binary_path: bin_dir.join("bitloops"),
            suite_root,
        })),
        ..Default::default()
    };

    let command = build_init_commit_without_post_commit_refresh_command(&world);
    let args = command
        .get_args()
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    let disable_refresh = command
        .get_envs()
        .find_map(|(key, value)| {
            if key == OsStr::new("BITLOOPS_DISABLE_POST_COMMIT_DEVQL_REFRESH") {
                value.map(|v| v.to_string_lossy().into_owned())
            } else {
                None
            }
        })
        .expect("init commit without post-commit refresh should disable post-commit refresh");

    assert_eq!(args, vec!["commit", "-m", "chore: initial commit"]);
    assert_eq!(disable_refresh, "1");
}

#[test]
fn expected_commit_path_pairs_pairs_paths_with_matching_sha_prefix() {
    let pairs = expected_commit_path_pairs(
        &[
            "sha-1".to_string(),
            "sha-2".to_string(),
            "sha-merge".to_string(),
        ],
        &["src/one.rs".to_string(), "src/two.rs".to_string()],
    )
    .expect("build expected commit/path pairs");

    assert_eq!(
        pairs,
        vec![
            ("sha-1".to_string(), "src/one.rs".to_string()),
            ("sha-2".to_string(), "src/two.rs".to_string()),
        ]
    );
}

#[test]
fn expected_commit_path_pairs_rejects_more_paths_than_shas() {
    let err = expected_commit_path_pairs(
        &["sha-1".to_string()],
        &["src/one.rs".to_string(), "src/two.rs".to_string()],
    )
    .expect_err("more paths than SHAs should fail");

    assert!(
        err.to_string()
            .contains("expected path count 2 exceeds expected SHA count 1"),
        "unexpected error: {err:#}"
    );
}

#[test]
fn build_bitloops_command_applies_watcher_hardening_env_by_default() {
    let temp = tempfile::tempdir().expect("tempdir");
    let repo_dir = temp.path().join("repo");
    fs::create_dir_all(&repo_dir).expect("create repo dir");
    let bin_dir = temp.path().join("bin");
    fs::create_dir_all(&bin_dir).expect("create bin dir");
    let suite_root = temp.path().join("suite");
    fs::create_dir_all(&suite_root).expect("create suite root");

    let world = QatWorld {
        run_dir: Some(temp.path().join("run")),
        repo_dir: Some(repo_dir),
        run_config: Some(Arc::new(QatRunConfig {
            binary_path: bin_dir.join("bitloops"),
            suite_root,
        })),
        ..Default::default()
    };

    let command =
        build_bitloops_command(&world, &["daemon", "start"]).expect("build bitloops command");

    assert_eq!(
        command_env_value(&command, DISABLE_WATCHER_AUTOSTART_ENV),
        Some("1".into())
    );
    assert_eq!(
        command_env_value(&command, DISABLE_VERSION_CHECK_ENV),
        Some("1".into())
    );
}

#[test]
fn build_bitloops_command_skips_watcher_hardening_when_world_allows_autostart() {
    let temp = tempfile::tempdir().expect("tempdir");
    let repo_dir = temp.path().join("repo");
    fs::create_dir_all(&repo_dir).expect("create repo dir");
    let bin_dir = temp.path().join("bin");
    fs::create_dir_all(&bin_dir).expect("create bin dir");
    let suite_root = temp.path().join("suite");
    fs::create_dir_all(&suite_root).expect("create suite root");

    let world = QatWorld {
        run_dir: Some(temp.path().join("run")),
        repo_dir: Some(repo_dir),
        run_config: Some(Arc::new(QatRunConfig {
            binary_path: bin_dir.join("bitloops"),
            suite_root,
        })),
        watcher_autostart_enabled: true,
        ..Default::default()
    };

    let command =
        build_bitloops_command(&world, &["daemon", "start"]).expect("build bitloops command");

    assert_eq!(
        command_env_value(&command, DISABLE_WATCHER_AUTOSTART_ENV),
        None
    );
    assert_eq!(
        command_env_value(&command, DISABLE_VERSION_CHECK_ENV),
        Some("1".into())
    );
}

#[test]
fn stop_daemon_for_scenario_terminates_registered_watcher_processes() {
    let temp = tempfile::tempdir().expect("tempdir");
    let run_dir = temp.path().join("run");
    let repo_dir = temp.path().join("repo");
    let terminal_log_path = run_dir.join("terminal.log");
    fs::create_dir_all(&run_dir).expect("create run dir");
    fs::create_dir_all(&repo_dir).expect("create repo dir");
    fs::create_dir_all(repo_dir.join(".git")).expect("create git dir");
    fs::write(repo_dir.join(".bitloops.local.toml"), "").expect("write local policy marker");

    let mut world = QatWorld {
        run_dir: Some(run_dir),
        repo_dir: Some(repo_dir.clone()),
        terminal_log_path: Some(terminal_log_path),
        watcher_autostart_enabled: true,
        ..Default::default()
    };

    let watcher_pid = spawn_detached_long_lived_process();
    with_scenario_app_env(&world, || {
        let config_root = bitloops::config::default_daemon_config_path()
            .expect("default daemon config path")
            .parent()
            .expect("daemon config parent")
            .to_path_buf();
        let runtime_store = RepoSqliteRuntimeStore::open_for_roots(&config_root, world.repo_dir())
            .expect("open runtime store");
        runtime_store
            .save_watcher_registration(
                watcher_pid,
                "qat-test-restart-token",
                world.repo_dir(),
                RepoWatcherRegistrationState::Ready,
            )
            .expect("seed watcher registration");
    });
    fs::remove_dir_all(repo_dir.join(".git")).expect("remove git dir before teardown");
    fs::remove_file(repo_dir.join(".bitloops.local.toml"))
        .expect("remove local policy marker before teardown");

    stop_daemon_for_scenario(&mut world).expect("stop scenario");

    wait_for_pid_exit(watcher_pid);
    let registration = with_scenario_app_env(&world, || {
        let config_root = bitloops::config::default_daemon_config_path()
            .expect("default daemon config path")
            .parent()
            .expect("daemon config parent")
            .to_path_buf();
        let runtime_store = RepoSqliteRuntimeStore::open_for_roots(&config_root, world.repo_dir())
            .expect("open runtime store");
        runtime_store
            .load_watcher_registration()
            .expect("load watcher registration")
    });
    assert!(
        registration.is_none(),
        "watcher teardown should clear the scenario registration"
    );
}

#[test]
fn daemon_runtime_store_candidate_paths_cover_isolated_state_dirs() {
    let run_dir = Path::new("/tmp/qat-run");
    let paths = daemon_runtime_store_candidate_paths(run_dir);

    assert_eq!(
        paths,
        vec![
            PathBuf::from("/tmp/qat-run/home/xdg-state/bitloops/daemon/runtime.sqlite"),
            PathBuf::from("/tmp/qat-run/home/.local/state/bitloops/daemon/runtime.sqlite"),
            PathBuf::from("/tmp/qat-run/home/xdg/bitloops/stores/runtime/runtime.sqlite"),
            PathBuf::from(
                "/tmp/qat-run/home/Library/Application Support/bitloops/stores/runtime/runtime.sqlite"
            ),
            PathBuf::from(
                "/tmp/qat-run/home/Library/Application Support/bitloops/daemon/runtime.sqlite"
            ),
        ]
    );
}

#[test]
fn load_latest_test_harness_capability_event_run_reads_macos_current_state_store() {
    let temp = tempfile::tempdir().expect("tempdir");
    let run_dir = temp.path().join("run");
    let repo_dir = temp.path().join("repo");
    let bin_dir = temp.path().join("bin");
    let suite_root = temp.path().join("suite");
    fs::create_dir_all(&run_dir).expect("create run dir");
    fs::create_dir_all(&repo_dir).expect("create repo dir");
    fs::create_dir_all(&bin_dir).expect("create bin dir");
    fs::create_dir_all(&suite_root).expect("create suite root");

    let runtime_path = run_dir
        .join("home")
        .join("Library")
        .join("Application Support")
        .join("bitloops")
        .join("stores")
        .join("runtime")
        .join("runtime.sqlite");
    let runtime_parent = runtime_path.parent().expect("runtime parent");
    fs::create_dir_all(runtime_parent).expect("create runtime parent");
    let store = DaemonSqliteRuntimeStore::open_at(runtime_path).expect("open runtime store");
    store
        .with_connection(|conn| {
            conn.execute(
                "INSERT INTO capability_workplane_cursor_runs (run_id, repo_id, repo_root, mailbox_name, capability_id, from_generation_seq, to_generation_seq, reconcile_mode, status, attempts, submitted_at_unix, started_at_unix, updated_at_unix, completed_at_unix, error) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
                rusqlite::params![
                    "run-1",
                    "repo-1",
                    "/tmp/repo",
                    "test_harness.current_state",
                    "test_harness",
                    1_i64,
                    2_i64,
                    "full_reconcile",
                    "completed",
                    1_i64,
                    100_i64,
                    101_i64,
                    102_i64,
                    103_i64,
                    Option::<String>::None,
                ],
            )?;
            Ok(())
        })
        .expect("insert current-state run");

    let world = QatWorld {
        run_dir: Some(run_dir),
        repo_dir: Some(repo_dir),
        run_config: Some(Arc::new(QatRunConfig {
            binary_path: bin_dir.join("bitloops"),
            suite_root,
        })),
        ..Default::default()
    };

    let (_, run) = load_latest_test_harness_capability_event_run(&world, "repo-1")
        .expect("load latest run")
        .expect("completed run should be found");

    assert_eq!(run.run_id, "run-1");
    assert_eq!(run.status, CapabilityEventRunStatus::Completed);
    assert_eq!(run.capability_id, "test_harness");
    assert_eq!(run.consumer_id, "test_harness.current_state");
    assert_eq!(run.event_kind, "current_state_consumer");
}

#[test]
fn load_latest_test_harness_capability_event_run_reads_xdg_config_runtime_store() {
    let temp = tempfile::tempdir().expect("tempdir");
    let run_dir = temp.path().join("run");
    let repo_dir = temp.path().join("repo");
    let bin_dir = temp.path().join("bin");
    let suite_root = temp.path().join("suite");
    fs::create_dir_all(&run_dir).expect("create run dir");
    fs::create_dir_all(&repo_dir).expect("create repo dir");
    fs::create_dir_all(&bin_dir).expect("create bin dir");
    fs::create_dir_all(&suite_root).expect("create suite root");

    let runtime_path = run_dir
        .join("home")
        .join("xdg")
        .join("bitloops")
        .join("stores")
        .join("runtime")
        .join("runtime.sqlite");
    let runtime_parent = runtime_path.parent().expect("runtime parent");
    fs::create_dir_all(runtime_parent).expect("create runtime parent");
    let store = DaemonSqliteRuntimeStore::open_at(runtime_path).expect("open runtime store");
    store
        .with_connection(|conn| {
            conn.execute(
                "INSERT INTO capability_workplane_cursor_runs (run_id, repo_id, repo_root, mailbox_name, capability_id, from_generation_seq, to_generation_seq, reconcile_mode, status, attempts, submitted_at_unix, started_at_unix, updated_at_unix, completed_at_unix, error) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
                rusqlite::params![
                    "run-xdg",
                    "repo-1",
                    "/tmp/repo",
                    "test_harness.current_state",
                    "test_harness",
                    1_i64,
                    2_i64,
                    "full_reconcile",
                    "completed",
                    1_i64,
                    100_i64,
                    101_i64,
                    102_i64,
                    103_i64,
                    Option::<String>::None,
                ],
            )?;
            Ok(())
        })
        .expect("insert current-state run");

    let world = QatWorld {
        run_dir: Some(run_dir),
        repo_dir: Some(repo_dir),
        run_config: Some(Arc::new(QatRunConfig {
            binary_path: bin_dir.join("bitloops"),
            suite_root,
        })),
        ..Default::default()
    };

    let (_, run) = load_latest_test_harness_capability_event_run(&world, "repo-1")
        .expect("load latest run")
        .expect("completed run should be found");

    assert_eq!(run.run_id, "run-xdg");
    assert_eq!(run.status, CapabilityEventRunStatus::Completed);
    assert_eq!(run.capability_id, "test_harness");
    assert_eq!(run.consumer_id, "test_harness.current_state");
}

#[test]
fn load_latest_test_harness_capability_event_run_prefers_newer_legacy_run() {
    let temp = tempfile::tempdir().expect("tempdir");
    let run_dir = temp.path().join("run");
    let repo_dir = temp.path().join("repo");
    let bin_dir = temp.path().join("bin");
    let suite_root = temp.path().join("suite");
    fs::create_dir_all(&run_dir).expect("create run dir");
    fs::create_dir_all(&repo_dir).expect("create repo dir");
    fs::create_dir_all(&bin_dir).expect("create bin dir");
    fs::create_dir_all(&suite_root).expect("create suite root");

    let runtime_path = run_dir
        .join("home")
        .join("Library")
        .join("Application Support")
        .join("bitloops")
        .join("stores")
        .join("runtime")
        .join("runtime.sqlite");
    let runtime_parent = runtime_path.parent().expect("runtime parent");
    fs::create_dir_all(runtime_parent).expect("create runtime parent");
    let store = DaemonSqliteRuntimeStore::open_at(runtime_path).expect("open runtime store");
    store
        .with_connection(|conn| {
            conn.execute(
                "INSERT INTO capability_workplane_cursor_runs (run_id, repo_id, repo_root, mailbox_name, capability_id, from_generation_seq, to_generation_seq, reconcile_mode, status, attempts, submitted_at_unix, started_at_unix, updated_at_unix, completed_at_unix, error) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
                rusqlite::params![
                    "run-current-state",
                    "repo-1",
                    "/tmp/repo",
                    "test_harness.current_state",
                    "test_harness",
                    1_i64,
                    2_i64,
                    "full_reconcile",
                    "completed",
                    1_i64,
                    100_i64,
                    101_i64,
                    102_i64,
                    103_i64,
                    Option::<String>::None,
                ],
            )?;
            conn.execute(
                "INSERT INTO pack_reconcile_runs (run_id, repo_id, repo_root, capability_id, consumer_id, from_generation_seq, to_generation_seq, reconcile_mode, status, attempts, submitted_at_unix, started_at_unix, updated_at_unix, completed_at_unix, error) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
                rusqlite::params![
                    "run-legacy",
                    "repo-1",
                    "/tmp/repo",
                    "test_harness",
                    "test_harness.current_state",
                    3_i64,
                    4_i64,
                    "merged_delta",
                    "completed",
                    1_i64,
                    200_i64,
                    201_i64,
                    202_i64,
                    203_i64,
                    Option::<String>::None,
                ],
            )?;
            Ok(())
        })
        .expect("insert mixed-schema runs");

    let world = QatWorld {
        run_dir: Some(run_dir),
        repo_dir: Some(repo_dir),
        run_config: Some(Arc::new(QatRunConfig {
            binary_path: bin_dir.join("bitloops"),
            suite_root,
        })),
        ..Default::default()
    };

    let (_, run) = load_latest_test_harness_capability_event_run(&world, "repo-1")
        .expect("load latest run")
        .expect("completed run should be found");

    assert_eq!(run.run_id, "run-legacy");
    assert_eq!(run.status, CapabilityEventRunStatus::Completed);
    assert_eq!(run.capability_id, "test_harness");
    assert_eq!(run.consumer_id, "test_harness.current_state");
}

#[test]
fn load_latest_test_harness_capability_event_run_filters_to_requested_repo() {
    let temp = tempfile::tempdir().expect("tempdir");
    let run_dir = temp.path().join("run");
    let repo_dir = temp.path().join("repo");
    let bin_dir = temp.path().join("bin");
    let suite_root = temp.path().join("suite");
    fs::create_dir_all(&run_dir).expect("create run dir");
    fs::create_dir_all(&repo_dir).expect("create repo dir");
    fs::create_dir_all(&bin_dir).expect("create bin dir");
    fs::create_dir_all(&suite_root).expect("create suite root");

    let runtime_path = run_dir
        .join("home")
        .join("Library")
        .join("Application Support")
        .join("bitloops")
        .join("stores")
        .join("runtime")
        .join("runtime.sqlite");
    let runtime_parent = runtime_path.parent().expect("runtime parent");
    fs::create_dir_all(runtime_parent).expect("create runtime parent");
    let store = DaemonSqliteRuntimeStore::open_at(runtime_path).expect("open runtime store");
    store
        .with_connection(|conn| {
            conn.execute(
                "INSERT INTO capability_workplane_cursor_runs (run_id, repo_id, repo_root, mailbox_name, capability_id, from_generation_seq, to_generation_seq, reconcile_mode, status, attempts, submitted_at_unix, started_at_unix, updated_at_unix, completed_at_unix, error) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
                rusqlite::params![
                    "run-other-current",
                    "repo-other",
                    "/tmp/other",
                    "test_harness.current_state",
                    "test_harness",
                    9_i64,
                    10_i64,
                    "full_reconcile",
                    "completed",
                    1_i64,
                    500_i64,
                    501_i64,
                    502_i64,
                    503_i64,
                    Option::<String>::None,
                ],
            )?;
            conn.execute(
                "INSERT INTO capability_workplane_cursor_runs (run_id, repo_id, repo_root, mailbox_name, capability_id, from_generation_seq, to_generation_seq, reconcile_mode, status, attempts, submitted_at_unix, started_at_unix, updated_at_unix, completed_at_unix, error) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
                rusqlite::params![
                    "run-requested",
                    "repo-1",
                    "/tmp/repo",
                    "test_harness.current_state",
                    "test_harness",
                    1_i64,
                    2_i64,
                    "full_reconcile",
                    "completed",
                    1_i64,
                    100_i64,
                    101_i64,
                    102_i64,
                    103_i64,
                    Option::<String>::None,
                ],
            )?;
            conn.execute(
                "INSERT INTO pack_reconcile_runs (run_id, repo_id, repo_root, capability_id, consumer_id, from_generation_seq, to_generation_seq, reconcile_mode, status, attempts, submitted_at_unix, started_at_unix, updated_at_unix, completed_at_unix, error) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
                rusqlite::params![
                    "run-other-legacy",
                    "repo-other",
                    "/tmp/other",
                    "test_harness",
                    "test_harness.current_state",
                    11_i64,
                    12_i64,
                    "merged_delta",
                    "completed",
                    1_i64,
                    600_i64,
                    601_i64,
                    602_i64,
                    603_i64,
                    Option::<String>::None,
                ],
            )?;
            Ok(())
        })
        .expect("insert mixed-repo runs");

    let world = QatWorld {
        run_dir: Some(run_dir),
        repo_dir: Some(repo_dir),
        run_config: Some(Arc::new(QatRunConfig {
            binary_path: bin_dir.join("bitloops"),
            suite_root,
        })),
        ..Default::default()
    };

    let (_, run) = load_latest_test_harness_capability_event_run(&world, "repo-1")
        .expect("load latest run")
        .expect("completed run should be found");

    assert_eq!(run.run_id, "run-requested");
    assert_eq!(run.repo_id, "repo-1");
}

#[test]
fn load_latest_test_harness_capability_event_run_prefers_higher_generation_when_timestamps_tie() {
    let temp = tempfile::tempdir().expect("tempdir");
    let run_dir = temp.path().join("run");
    let repo_dir = temp.path().join("repo");
    let bin_dir = temp.path().join("bin");
    let suite_root = temp.path().join("suite");
    fs::create_dir_all(&run_dir).expect("create run dir");
    fs::create_dir_all(&repo_dir).expect("create repo dir");
    fs::create_dir_all(&bin_dir).expect("create bin dir");
    fs::create_dir_all(&suite_root).expect("create suite root");

    let runtime_path = run_dir
        .join("home")
        .join("Library")
        .join("Application Support")
        .join("bitloops")
        .join("stores")
        .join("runtime")
        .join("runtime.sqlite");
    let runtime_parent = runtime_path.parent().expect("runtime parent");
    fs::create_dir_all(runtime_parent).expect("create runtime parent");
    let store = DaemonSqliteRuntimeStore::open_at(runtime_path).expect("open runtime store");
    store
        .with_connection(|conn| {
            conn.execute(
                "INSERT INTO capability_workplane_cursor_runs (run_id, repo_id, repo_root, mailbox_name, capability_id, from_generation_seq, to_generation_seq, reconcile_mode, status, attempts, submitted_at_unix, started_at_unix, updated_at_unix, completed_at_unix, error) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
                rusqlite::params![
                    "run-older-generation",
                    "repo-1",
                    "/tmp/repo",
                    "test_harness.current_state",
                    "test_harness",
                    0_i64,
                    1_i64,
                    "full_reconcile",
                    "completed",
                    1_i64,
                    100_i64,
                    101_i64,
                    102_i64,
                    103_i64,
                    Option::<String>::None,
                ],
            )?;
            conn.execute(
                "INSERT INTO capability_workplane_cursor_runs (run_id, repo_id, repo_root, mailbox_name, capability_id, from_generation_seq, to_generation_seq, reconcile_mode, status, attempts, submitted_at_unix, started_at_unix, updated_at_unix, completed_at_unix, error) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
                rusqlite::params![
                    "run-newer-generation",
                    "repo-1",
                    "/tmp/repo",
                    "test_harness.current_state",
                    "test_harness",
                    1_i64,
                    2_i64,
                    "full_reconcile",
                    "completed",
                    1_i64,
                    100_i64,
                    101_i64,
                    102_i64,
                    103_i64,
                    Option::<String>::None,
                ],
            )?;
            Ok(())
        })
        .expect("insert same-timestamp runs");

    let world = QatWorld {
        run_dir: Some(run_dir),
        repo_dir: Some(repo_dir),
        run_config: Some(Arc::new(QatRunConfig {
            binary_path: bin_dir.join("bitloops"),
            suite_root,
        })),
        ..Default::default()
    };

    let (_, run) = load_latest_test_harness_capability_event_run(&world, "repo-1")
        .expect("load latest run")
        .expect("completed run should be found");

    assert_eq!(run.run_id, "run-newer-generation");
    assert_eq!(run.to_generation_seq, 2);
}

#[test]
fn load_latest_test_harness_generation_state_reads_cursor_progress_from_xdg_config_store() {
    let temp = tempfile::tempdir().expect("tempdir");
    let run_dir = temp.path().join("run");
    let repo_dir = temp.path().join("repo");
    let bin_dir = temp.path().join("bin");
    let suite_root = temp.path().join("suite");
    fs::create_dir_all(&run_dir).expect("create run dir");
    fs::create_dir_all(&repo_dir).expect("create repo dir");
    fs::create_dir_all(&bin_dir).expect("create bin dir");
    fs::create_dir_all(&suite_root).expect("create suite root");

    let runtime_path = run_dir
        .join("home")
        .join("xdg")
        .join("bitloops")
        .join("stores")
        .join("runtime")
        .join("runtime.sqlite");
    let runtime_parent = runtime_path.parent().expect("runtime parent");
    fs::create_dir_all(runtime_parent).expect("create runtime parent");
    let store =
        DaemonSqliteRuntimeStore::open_at(runtime_path.clone()).expect("open runtime store");
    store
        .with_connection(|conn| {
            conn.execute(
                "INSERT INTO capability_workplane_cursor_generations (repo_id, generation_seq, source_task_id, sync_mode, active_branch, head_commit_sha, requires_full_reconcile, created_at_unix) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                rusqlite::params![
                    "repo-1",
                    1_i64,
                    "sync-task-1",
                    "full_reconcile",
                    "main",
                    "abc123",
                    0_i64,
                    100_i64,
                ],
            )?;
            conn.execute(
                "INSERT INTO capability_workplane_cursor_generations (repo_id, generation_seq, source_task_id, sync_mode, active_branch, head_commit_sha, requires_full_reconcile, created_at_unix) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                rusqlite::params![
                    "repo-1",
                    2_i64,
                    "sync-task-2",
                    "full_reconcile",
                    "main",
                    "def456",
                    0_i64,
                    200_i64,
                ],
            )?;
            conn.execute(
                "INSERT INTO capability_workplane_cursor_mailboxes (repo_id, capability_id, mailbox_name, last_applied_generation_seq, last_error, updated_at_unix) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                rusqlite::params![
                    "repo-1",
                    "test_harness",
                    "test_harness.current_state",
                    2_i64,
                    Option::<String>::None,
                    210_i64,
                ],
            )?;
            Ok(())
        })
        .expect("insert generation state");

    let world = QatWorld {
        run_dir: Some(run_dir),
        repo_dir: Some(repo_dir),
        run_config: Some(Arc::new(QatRunConfig {
            binary_path: bin_dir.join("bitloops"),
            suite_root,
        })),
        ..Default::default()
    };

    let (path, state) = load_latest_test_harness_generation_state(&world, "repo-1")
        .expect("load generation state")
        .expect("generation state should be found");

    assert_eq!(path, runtime_path);
    assert_eq!(state.latest_generation_seq, 2);
    assert_eq!(state.last_applied_generation_seq, Some(2));
    assert_eq!(state.last_error, None);
}

#[test]
fn test_harness_generation_state_reaches_target_only_after_cursor_covers_it() {
    let pending = TestHarnessGenerationState {
        latest_generation_seq: 3,
        last_applied_generation_seq: Some(2),
        last_error: None,
    };
    let complete = TestHarnessGenerationState {
        latest_generation_seq: 3,
        last_applied_generation_seq: Some(3),
        last_error: None,
    };

    assert!(!test_harness_generation_state_reached_target(&pending, 3));
    assert!(test_harness_generation_state_reached_target(&complete, 3));
}

#[test]
fn daemon_start_args_use_foreground_http_mode() {
    let args = daemon_start_args("43127");
    assert_eq!(
        args,
        vec![
            "daemon",
            "start",
            "--create-default-config",
            "--no-telemetry",
            "--http",
            "--host",
            "127.0.0.1",
            "--port",
            "43127",
        ]
    );
    assert!(!args.iter().any(|arg| arg == "-d"));
}

#[test]
fn normalise_onboarding_agent_name_supports_aliases() {
    assert_eq!(
        normalise_onboarding_agent_name("claude"),
        AGENT_NAME_CLAUDE_CODE
    );
    assert_eq!(
        normalise_onboarding_agent_name("open-code"),
        AGENT_NAME_OPEN_CODE
    );
    assert_eq!(
        normalise_onboarding_agent_name(AGENT_NAME_COPILOT),
        AGENT_NAME_COPILOT
    );
}

#[test]
fn normalise_smoke_agent_name_supports_canonical_agents_and_aliases() {
    assert_eq!(normalise_smoke_agent_name("claude"), AGENT_NAME_CLAUDE_CODE);
    assert_eq!(normalise_smoke_agent_name("cursor"), AGENT_NAME_CURSOR);
    assert_eq!(normalise_smoke_agent_name("gemini"), AGENT_NAME_GEMINI);
    assert_eq!(normalise_smoke_agent_name("copilot"), AGENT_NAME_COPILOT);
    assert_eq!(normalise_smoke_agent_name("codex"), AGENT_NAME_CODEX);
    assert_eq!(
        normalise_smoke_agent_name("open-code"),
        AGENT_NAME_OPEN_CODE
    );
    assert_eq!(normalise_smoke_agent_name("opencode"), AGENT_NAME_OPEN_CODE);
}

#[test]
fn checkpoint_agent_candidates_expand_claude_aliases() {
    assert_eq!(
        checkpoint_agent_candidates("claude-code"),
        vec!["claude-code".to_string(), "claude".to_string()]
    );
    assert_eq!(
        checkpoint_agent_candidates("claude"),
        vec!["claude".to_string(), "claude-code".to_string()]
    );
    assert_eq!(
        checkpoint_agent_candidates("codex"),
        vec!["codex".to_string()]
    );
}

#[test]
fn build_chat_history_query_targets_specific_file() {
    let query = build_chat_history_query("my-app/src/App.tsx");
    assert_eq!(
        query,
        r#"repo("bitloops")->file("my-app/src/App.tsx")->artefacts()->chatHistory()->limit(10)"#
    );
}

#[test]
fn count_chat_history_edges_for_agent_only_counts_matching_agent_rows() {
    let payload = serde_json::json!([
        {
            "chatHistory": {
                "edges": [
                    { "node": { "agent": "claude-code", "content": "one" } },
                    { "node": { "agent": "cursor", "content": "two" } }
                ]
            }
        },
        {
            "chatHistory": {
                "edges": [
                    { "node": { "agent": "claude-code", "content": "three" } }
                ]
            }
        }
    ]);

    assert_eq!(
        count_chat_history_edges_for_agent(&payload, "claude-code"),
        2
    );
    assert_eq!(count_chat_history_edges_for_agent(&payload, "claude"), 2);
    assert_eq!(count_chat_history_edges_for_agent(&payload, "cursor"), 1);
    assert_eq!(count_chat_history_edges_for_agent(&payload, "codex"), 0);
}

#[test]
fn semantic_clone_health_is_ready_requires_runtime_checks_to_be_healthy() {
    let healthy = serde_json::json!({
        "health": [
            { "check_id": "semantic_clones.profile_resolution", "healthy": true, "message": "ok" },
            { "check_id": "semantic_clones.runtime_command", "healthy": true, "message": "ok" },
            { "check_id": "semantic_clones.runtime_handshake", "healthy": true, "message": "ok" }
        ]
    });
    let unhealthy = serde_json::json!({
        "health": [
            { "check_id": "semantic_clones.profile_resolution", "healthy": true, "message": "ok" },
            { "check_id": "semantic_clones.runtime_command", "healthy": false, "message": "missing" },
            { "check_id": "semantic_clones.runtime_handshake", "healthy": true, "message": "ok" }
        ]
    });

    assert!(semantic_clone_health_is_ready(&healthy));
    assert!(!semantic_clone_health_is_ready(&unhealthy));
}

#[test]
fn semantic_clone_store_evidence_accepts_persisted_edges_when_cli_metric_is_zero() {
    let evidence = SemanticCloneStoreEvidence {
        current_artefacts: 6,
        embeddings: 5,
        clone_edges: 6,
    };

    assert!(semantic_clone_store_evidence_proves_rebuild(
        Some(0),
        evidence
    ));
}

#[test]
fn semantic_clone_store_evidence_requires_persisted_rows_even_when_cli_metric_is_positive() {
    assert!(!semantic_clone_store_evidence_proves_rebuild(
        Some(3),
        SemanticCloneStoreEvidence {
            current_artefacts: 0,
            embeddings: 0,
            clone_edges: 0,
        }
    ));
}

#[test]
fn semantic_clone_store_evidence_rejects_missing_embeddings_or_edges() {
    assert!(!semantic_clone_store_evidence_proves_rebuild(
        Some(0),
        SemanticCloneStoreEvidence {
            current_artefacts: 6,
            embeddings: 0,
            clone_edges: 6,
        }
    ));
    assert!(!semantic_clone_store_evidence_proves_rebuild(
        None,
        SemanticCloneStoreEvidence {
            current_artefacts: 6,
            embeddings: 5,
            clone_edges: 0,
        }
    ));
}

#[test]
fn parse_enrichment_status_snapshot_reads_cli_lines() {
    let snapshot = parse_enrichment_status_snapshot(
        "Enrichment queue: available\n\
         Enrichment mode: paused\n\
         Enrichment pending jobs: 6\n\
         Enrichment pending semantic jobs: 3\n\
         Enrichment pending embedding jobs: 2\n\
         Enrichment pending clone-edge rebuild jobs: 1\n\
         Enrichment running jobs: 4\n\
         Enrichment running semantic jobs: 2\n\
         Enrichment running embedding jobs: 1\n\
         Enrichment running clone-edge rebuild jobs: 1\n\
         Enrichment failed jobs: 0\n\
         Enrichment failed semantic jobs: 0\n\
         Enrichment failed embedding jobs: 0\n\
         Enrichment failed clone-edge rebuild jobs: 0\n\
         Enrichment retried failed jobs: 5\n\
         Enrichment last action: paused\n\
         Enrichment pause reason: qa hold\n\
         Enrichment persisted: yes\n",
    )
    .expect("parse enrichments status");

    assert_eq!(snapshot.mode, "paused");
    assert_eq!(snapshot.pending_jobs, 6);
    assert_eq!(snapshot.pending_semantic_jobs, 3);
    assert_eq!(snapshot.pending_embedding_jobs, 2);
    assert_eq!(snapshot.pending_clone_edges_rebuild_jobs, 1);
    assert_eq!(snapshot.running_jobs, 4);
    assert_eq!(snapshot.running_semantic_jobs, 2);
    assert_eq!(snapshot.running_embedding_jobs, 1);
    assert_eq!(snapshot.running_clone_edges_rebuild_jobs, 1);
    assert_eq!(snapshot.failed_jobs, 0);
    assert_eq!(snapshot.failed_semantic_jobs, 0);
    assert_eq!(snapshot.failed_embedding_jobs, 0);
    assert_eq!(snapshot.failed_clone_edges_rebuild_jobs, 0);
    assert_eq!(snapshot.retried_failed_jobs, 5);
    assert_eq!(snapshot.last_action.as_deref(), Some("paused"));
    assert_eq!(snapshot.paused_reason.as_deref(), Some("qa hold"));
    assert!(snapshot.persisted);
}

#[test]
fn load_representation_kind_counts_normalizes_legacy_code_aliases() {
    let conn = rusqlite::Connection::open_in_memory().expect("open in-memory sqlite");
    conn.execute(
        "CREATE TABLE symbol_embeddings_current (repo_id TEXT NOT NULL, representation_kind TEXT NOT NULL)",
        [],
    )
    .expect("create symbol_embeddings_current");
    for kind in ["code", "baseline", "enriched", "summary", "summary"] {
        conn.execute(
            "INSERT INTO symbol_embeddings_current (repo_id, representation_kind) VALUES (?1, ?2)",
            rusqlite::params!["repo-1", kind],
        )
        .expect("insert representation kind row");
    }

    let counts =
        load_representation_kind_counts_for_repo(&conn, "symbol_embeddings_current", "repo-1")
            .expect("load representation counts");

    assert_eq!(counts.code, 3);
    assert_eq!(counts.summary, 2);
}

#[test]
fn extract_clone_nodes_accepts_flattened_clone_query_rows() {
    let rows = serde_json::json!([
        {
            "from": "src/render-invoice.ts::renderInvoice",
            "to": "src/render-invoice-document.ts::renderInvoiceDocument",
            "relationKind": "shared_logic_candidate",
            "score": 0.97
        }
    ]);

    assert_eq!(
        extract_clone_nodes(&rows),
        rows.as_array().cloned().unwrap_or_default()
    );
}

#[test]
fn extract_clone_summary_accepts_devql_summary_rows() {
    let value = serde_json::json!([
        {
            "total_count": 3,
            "groups": [
                { "relation_kind": "similar_implementation", "count": 2 },
                { "relation_kind": "weak_clone_candidate", "count": 1 }
            ]
        }
    ]);

    let summary = extract_clone_summary_from_devql_value(&value).expect("extract DevQL summary");

    assert_eq!(summary.total_count, 3);
    assert_eq!(summary.groups.len(), 2);
    assert_eq!(summary.groups[0].relation_kind, "similar_implementation");
    assert_eq!(summary.groups[0].count, 2);
    assert_eq!(summary.groups[1].relation_kind, "weak_clone_candidate");
    assert_eq!(summary.groups[1].count, 1);
}

#[test]
fn extract_clone_summary_accepts_devql_summary_rows_with_camel_case_total_count() {
    let value = serde_json::json!([
        {
            "totalCount": 3,
            "groups": [
                { "relationKind": "similar_implementation", "count": 2 },
                { "relationKind": "weak_clone_candidate", "count": 1 }
            ]
        }
    ]);

    let summary = extract_clone_summary_from_devql_value(&value).expect("extract DevQL summary");

    assert_eq!(summary.total_count, 3);
    assert_eq!(summary.groups.len(), 2);
    assert_eq!(summary.groups[0].relation_kind, "similar_implementation");
    assert_eq!(summary.groups[0].count, 2);
    assert_eq!(summary.groups[1].relation_kind, "weak_clone_candidate");
    assert_eq!(summary.groups[1].count, 1);
}

#[test]
fn extract_clone_summary_accepts_graphql_repo_payload() {
    let value = serde_json::json!({
        "repo": {
            "cloneSummary": {
                "totalCount": 4,
                "groups": [
                    { "relationKind": "similar_implementation", "count": 3 },
                    { "relationKind": "contextual_neighbor", "count": 1 }
                ]
            }
        }
    });

    let summary =
        extract_clone_summary_from_graphql_value(&value).expect("extract GraphQL clone summary");

    assert_eq!(summary.total_count, 4);
    assert_eq!(summary.groups.len(), 2);
    assert_eq!(summary.groups[0].relation_kind, "similar_implementation");
    assert_eq!(summary.groups[0].count, 3);
    assert_eq!(summary.groups[1].relation_kind, "contextual_neighbor");
    assert_eq!(summary.groups[1].count, 1);
}

#[test]
fn wait_for_semantic_clone_condition_retries_until_ready() {
    let mut attempts = 0_usize;

    let value = wait_for_semantic_clone_condition(
        StdDuration::from_millis(25),
        StdDuration::from_millis(1),
        "clone rows to become visible",
        || {
            attempts += 1;
            Ok(attempts)
        },
        |attempt| *attempt >= 3,
        |attempt| format!("attempt={attempt}"),
    )
    .expect("eventual wait should succeed");

    assert_eq!(value, 3);
    assert_eq!(attempts, 3);
}

#[test]
fn wait_for_semantic_clone_condition_times_out_with_last_observation() {
    let err = wait_for_semantic_clone_condition(
        StdDuration::from_millis(5),
        StdDuration::from_millis(1),
        "clone rows to become visible",
        || Ok(0_usize),
        |count| *count > 0,
        |count| format!("rows={count}"),
    )
    .expect_err("eventual wait should time out");

    let message = format!("{err:#}");
    assert!(message.contains("clone rows to become visible"));
    assert!(message.contains("last observation=value: rows=0"));
}

#[test]
fn wait_for_qat_condition_retries_until_ready() {
    let mut attempts = 0_usize;

    let value = wait_for_qat_condition(
        StdDuration::from_millis(25),
        StdDuration::from_millis(1),
        "checkpoint mappings to be persisted",
        || {
            attempts += 1;
            Ok(attempts)
        },
        |attempt| *attempt >= 3,
        |attempt| format!("attempt={attempt}"),
    )
    .expect("eventual wait should succeed");

    assert_eq!(value, 3);
    assert_eq!(attempts, 3);
}

#[test]
fn wait_for_qat_condition_times_out_with_last_observation() {
    let err = wait_for_qat_condition(
        StdDuration::from_millis(5),
        StdDuration::from_millis(1),
        "DevQL artefacts query to return results",
        || Ok(0_usize),
        |count| *count > 0,
        |count| format!("artefacts={count}"),
    )
    .expect_err("eventual wait should time out");

    let message = format!("{err:#}");
    assert!(message.contains("DevQL artefacts query to return results"));
    assert!(message.contains("last observation=value: artefacts=0"));
}

#[test]
fn clone_query_wait_condition_any_response_accepts_empty_rows() {
    let rows: Vec<serde_json::Value> = Vec::new();

    assert!(clone_query_meets_wait_condition(
        rows.as_slice(),
        &CloneQueryWaitCondition::AnyResponse
    ));
    assert!(!clone_query_meets_wait_condition(
        rows.as_slice(),
        &CloneQueryWaitCondition::NonEmptyResults
    ));
}

#[test]
fn smoke_session_id_uses_agent_and_run_slug() {
    let temp = tempfile::tempdir().expect("tempdir");
    let run_dir = temp.path().join("First Agent Smoke/Run");
    fs::create_dir_all(&run_dir).expect("create run dir");

    let world = QatWorld {
        run_dir: Some(run_dir),
        ..Default::default()
    };

    assert_eq!(
        smoke_session_id(&world, AGENT_NAME_CLAUDE_CODE),
        "claude-code-run"
    );
    assert_eq!(smoke_session_id(&world, AGENT_NAME_CODEX), "codex-run");
}

fn sample_interaction_session(session_id: &str, agent_type: &str) -> InteractionSession {
    InteractionSession {
        session_id: session_id.to_string(),
        repo_id: "repo-test".to_string(),
        agent_type: agent_type.to_string(),
        transcript_path: format!("/tmp/{session_id}.jsonl"),
        started_at: "2026-04-17T10:00:00Z".to_string(),
        last_event_at: "2026-04-17T10:00:00Z".to_string(),
        updated_at: "2026-04-17T10:00:00Z".to_string(),
        ..Default::default()
    }
}

fn sample_interaction_turn(
    turn_id: &str,
    session_id: &str,
    agent_type: &str,
    checkpoint_id: Option<&str>,
) -> InteractionTurn {
    InteractionTurn {
        turn_id: turn_id.to_string(),
        session_id: session_id.to_string(),
        repo_id: "repo-test".to_string(),
        turn_number: 1,
        prompt: "make the requested change".to_string(),
        agent_type: agent_type.to_string(),
        model: "gpt-5.4".to_string(),
        started_at: "2026-04-17T10:00:01Z".to_string(),
        ended_at: Some("2026-04-17T10:00:02Z".to_string()),
        files_modified: vec!["src/lib.rs".to_string()],
        checkpoint_id: checkpoint_id.map(str::to_string),
        updated_at: "2026-04-17T10:00:02Z".to_string(),
        ..Default::default()
    }
}

#[test]
fn collect_agent_pre_commit_interactions_returns_matching_session_and_uncheckpointed_turns() {
    let sessions = vec![
        sample_interaction_session("claude-session", AGENT_NAME_CLAUDE_CODE),
        sample_interaction_session("cursor-session", AGENT_NAME_CURSOR),
    ];
    let turns = vec![
        sample_interaction_turn(
            "claude-turn-1",
            "claude-session",
            AGENT_NAME_CLAUDE_CODE,
            None,
        ),
        sample_interaction_turn("cursor-turn-1", "cursor-session", AGENT_NAME_CURSOR, None),
    ];

    let snapshot = collect_agent_pre_commit_interactions(&sessions, &turns, AGENT_NAME_CLAUDE_CODE);

    assert_eq!(snapshot.session_ids, vec!["claude-session".to_string()]);
    assert_eq!(
        snapshot.uncheckpointed_turn_ids,
        vec!["claude-turn-1".to_string()]
    );
}

#[test]
fn collect_agent_pre_commit_interactions_ignores_turns_for_other_sessions() {
    let sessions = vec![sample_interaction_session(
        "claude-session",
        AGENT_NAME_CLAUDE_CODE,
    )];
    let turns = vec![sample_interaction_turn(
        "cursor-turn-1",
        "cursor-session",
        AGENT_NAME_CURSOR,
        None,
    )];

    let snapshot = collect_agent_pre_commit_interactions(&sessions, &turns, AGENT_NAME_CLAUDE_CODE);

    assert_eq!(snapshot.session_ids, vec!["claude-session".to_string()]);
    assert!(snapshot.uncheckpointed_turn_ids.is_empty());
}

#[test]
fn collect_agent_pre_commit_interactions_ignores_checkpointed_turns() {
    let sessions = vec![sample_interaction_session(
        "claude-session",
        AGENT_NAME_CLAUDE_CODE,
    )];
    let turns = vec![sample_interaction_turn(
        "claude-turn-1",
        "claude-session",
        AGENT_NAME_CLAUDE_CODE,
        Some("checkpoint-1"),
    )];

    let snapshot = collect_agent_pre_commit_interactions(&sessions, &turns, AGENT_NAME_CLAUDE_CODE);

    assert_eq!(snapshot.session_ids, vec!["claude-session".to_string()]);
    assert!(snapshot.uncheckpointed_turn_ids.is_empty());
}

fn sample_commit_checkpoint_row(commit_sha: &str, checkpoint_id: &str) -> CommitCheckpointRow {
    CommitCheckpointRow {
        commit_sha: commit_sha.to_string(),
        checkpoint_id: checkpoint_id.to_string(),
    }
}

#[test]
fn count_commit_checkpoint_rows_counts_duplicate_commit_shas_separately() {
    let rows = vec![
        sample_commit_checkpoint_row("sha-1", "cp-1"),
        sample_commit_checkpoint_row("sha-1", "cp-2"),
    ];

    assert_eq!(count_commit_checkpoint_rows(&rows), 2);
}

#[test]
fn latest_reachable_ledger_snapshot_uses_only_latest_reachable_commits() {
    let reachable = vec![
        "sha-3".to_string(),
        "sha-2".to_string(),
        "sha-1".to_string(),
    ];
    let completed = vec![
        "sha-3".to_string(),
        "sha-1".to_string(),
        "sha-old".to_string(),
    ];

    let snapshot = latest_reachable_ledger_snapshot(reachable.as_slice(), completed.as_slice(), 1)
        .expect("snapshot should be collected");

    assert_eq!(
        snapshot.expected_latest,
        std::collections::BTreeSet::from(["sha-3".to_string()])
    );
    assert_eq!(
        snapshot.completed_reachable,
        std::collections::BTreeSet::from(["sha-3".to_string(), "sha-1".to_string()])
    );
    assert_eq!(snapshot.reachable_total, 3);
    assert_eq!(snapshot.completed_total, 3);
}

#[test]
fn captured_commit_shas_with_checkpoint_rows_filters_to_persisted_captured_commits() {
    let rows = vec![sample_commit_checkpoint_row("sha-1", "cp-1")];
    let captured = vec!["sha-1".to_string(), "sha-2".to_string()];

    assert_eq!(
        captured_commit_shas_with_checkpoint_rows(&captured, &rows),
        vec!["sha-1".to_string()]
    );
}

#[test]
fn checkpoint_ids_for_commit_sha_returns_all_matching_checkpoint_ids() {
    let rows = vec![
        sample_commit_checkpoint_row("sha-1", "cp-1"),
        sample_commit_checkpoint_row("sha-1", "cp-2"),
        sample_commit_checkpoint_row("sha-2", "cp-3"),
    ];

    assert_eq!(
        checkpoint_ids_for_commit_sha(&rows, "sha-1"),
        vec!["cp-1".to_string(), "cp-2".to_string()]
    );
}

fn sample_knowledge_relation(
    source_knowledge_item_id: &str,
    target_type: &str,
    target_id: &str,
) -> KnowledgeRelationAssertionRecord {
    KnowledgeRelationAssertionRecord {
        knowledge_item_id: source_knowledge_item_id.to_string(),
        target_type: target_type.to_string(),
        target_id: target_id.to_string(),
        relation_type: "associated_with".to_string(),
        association_method: "manual_attachment".to_string(),
    }
}

#[test]
fn knowledge_relation_exists_for_target_requires_expected_source_and_target() {
    let relations = vec![
        sample_knowledge_relation("knowledge-alpha", "knowledge_item", "knowledge-beta"),
        sample_knowledge_relation("knowledge-alpha", "commit", "abc123"),
    ];

    assert!(knowledge_relation_exists_for_target(
        &relations,
        "knowledge-alpha",
        "knowledge_item",
        "knowledge-beta"
    ));
    assert!(!knowledge_relation_exists_for_target(
        &relations,
        "knowledge-gamma",
        "knowledge_item",
        "knowledge-beta"
    ));
    assert!(!knowledge_relation_exists_for_target(
        &relations,
        "knowledge-alpha",
        "knowledge_item",
        "knowledge-gamma"
    ));
}

#[test]
fn parse_association_target_from_output_extracts_expected_target_id() {
    let stdout = "Knowledge associated\n  relation assertion: rel-123\n  target: commit:abc123\n  relation: associated_with\n  method: manual_attachment\n";

    assert_eq!(
        parse_association_target_from_output(stdout, "commit"),
        Some("abc123".to_string())
    );
    assert_eq!(
        parse_association_target_from_output(stdout, "knowledge_item"),
        None
    );
}

#[test]
fn expected_smoke_transcript_path_uses_agent_specific_locations() {
    let temp = tempfile::tempdir().expect("tempdir");
    let run_dir = temp.path().join("Smoke Matrix Run");
    fs::create_dir_all(&run_dir).expect("create run dir");

    let world = QatWorld {
        run_dir: Some(run_dir.clone()),
        ..Default::default()
    };

    let claude_session_id = smoke_session_id(&world, AGENT_NAME_CLAUDE_CODE);
    let cursor_session_id = smoke_session_id(&world, AGENT_NAME_CURSOR);
    let gemini_session_id = smoke_session_id(&world, AGENT_NAME_GEMINI);
    let copilot_session_id = smoke_session_id(&world, AGENT_NAME_COPILOT);
    let codex_session_id = smoke_session_id(&world, AGENT_NAME_CODEX);
    let opencode_session_id = smoke_session_id(&world, AGENT_NAME_OPEN_CODE);

    assert_eq!(
        expected_smoke_transcript_path(&world, AGENT_NAME_CLAUDE_CODE),
        run_dir
            .join("agent-sessions")
            .join(AGENT_NAME_CLAUDE_CODE)
            .join(format!("{claude_session_id}.jsonl"))
    );
    assert_eq!(
        expected_smoke_transcript_path(&world, AGENT_NAME_CURSOR),
        run_dir
            .join("agent-sessions")
            .join(AGENT_NAME_CURSOR)
            .join(format!("{cursor_session_id}.jsonl"))
    );
    assert_eq!(
        expected_smoke_transcript_path(&world, AGENT_NAME_GEMINI),
        run_dir
            .join("agent-sessions")
            .join(AGENT_NAME_GEMINI)
            .join(format!("{gemini_session_id}.json"))
    );
    assert_eq!(
        expected_smoke_transcript_path(&world, AGENT_NAME_COPILOT),
        run_dir
            .join("home")
            .join(".copilot")
            .join("session-state")
            .join(copilot_session_id)
            .join("events.jsonl")
    );
    assert_eq!(
        expected_smoke_transcript_path(&world, AGENT_NAME_CODEX),
        run_dir
            .join("agent-sessions")
            .join(AGENT_NAME_CODEX)
            .join(format!("{codex_session_id}.jsonl"))
    );
    assert_eq!(
        expected_smoke_transcript_path(&world, AGENT_NAME_OPEN_CODE),
        run_dir
            .join("agent-sessions")
            .join(AGENT_NAME_OPEN_CODE)
            .join(format!("{opencode_session_id}.jsonl"))
    );
}

#[test]
fn parse_git_timeline_reads_sha_subject_and_timestamp() {
    let commits = parse_git_timeline(
        "aaa111|chore: initial commit|2026-04-15T12:00:00Z\nbbb222|test: committed today|2026-04-16T12:00:00Z",
    );

    assert_eq!(
        commits,
        vec![
            GitTimelineCommit {
                sha: "aaa111".to_string(),
                subject: "chore: initial commit".to_string(),
                author_iso: "2026-04-15T12:00:00Z".to_string(),
            },
            GitTimelineCommit {
                sha: "bbb222".to_string(),
                subject: "test: committed today".to_string(),
                author_iso: "2026-04-16T12:00:00Z".to_string(),
            },
        ]
    );
}

#[test]
fn captured_commit_history_is_ordered_accepts_monotonic_git_log_positions() {
    let commits = vec![
        GitTimelineCommit {
            sha: "ccc333".to_string(),
            subject: "test: committed today".to_string(),
            author_iso: "2026-04-16T12:00:00Z".to_string(),
        },
        GitTimelineCommit {
            sha: "bbb222".to_string(),
            subject: "test: committed yesterday".to_string(),
            author_iso: "2026-04-15T12:00:00Z".to_string(),
        },
        GitTimelineCommit {
            sha: "aaa111".to_string(),
            subject: "chore: initial commit".to_string(),
            author_iso: "2026-04-15T12:00:00Z".to_string(),
        },
    ];

    assert!(captured_commit_history_is_ordered(
        &commits,
        &[
            "aaa111".to_string(),
            "bbb222".to_string(),
            "ccc333".to_string(),
        ]
    ));
}

#[test]
fn captured_commit_history_is_ordered_rejects_out_of_order_captured_shas() {
    let commits = vec![
        GitTimelineCommit {
            sha: "ccc333".to_string(),
            subject: "test: committed today".to_string(),
            author_iso: "2026-04-16T12:00:00Z".to_string(),
        },
        GitTimelineCommit {
            sha: "bbb222".to_string(),
            subject: "test: committed yesterday".to_string(),
            author_iso: "2026-04-15T12:00:00Z".to_string(),
        },
        GitTimelineCommit {
            sha: "aaa111".to_string(),
            subject: "chore: initial commit".to_string(),
            author_iso: "2026-04-15T12:00:00Z".to_string(),
        },
    ];

    assert!(!captured_commit_history_is_ordered(
        &commits,
        &[
            "aaa111".to_string(),
            "ccc333".to_string(),
            "bbb222".to_string(),
        ]
    ));
}

fn command_env_value(command: &std::process::Command, key: &str) -> Option<std::ffi::OsString> {
    command.get_envs().find_map(|(env_key, value)| {
        if env_key == OsStr::new(key) {
            value.map(|v| v.to_os_string())
        } else {
            None
        }
    })
}

#[test]
fn semantic_clone_store_evidence_proves_rebuild_when_store_has_current_rows() {
    assert!(semantic_clone_store_evidence_proves_rebuild(
        Some(0),
        SemanticCloneStoreEvidence {
            current_artefacts: 4,
            embeddings: 4,
            clone_edges: 2,
        }
    ));
    assert!(!semantic_clone_store_evidence_proves_rebuild(
        Some(0),
        SemanticCloneStoreEvidence {
            current_artefacts: 4,
            embeddings: 4,
            clone_edges: 0,
        }
    ));
}
