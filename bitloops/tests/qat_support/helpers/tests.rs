use super::*;
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::qat_support::world::QatRunConfig;
use bitloops::cli::versioncheck::DISABLE_VERSION_CHECK_ENV;
use bitloops::host::devql::watch::DISABLE_WATCHER_AUTOSTART_ENV;

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
        "claude-code",
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
fn build_bitloops_command_applies_daemon_hardening_env() {
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
fn daemon_runtime_store_candidate_paths_cover_isolated_state_dirs() {
    let run_dir = Path::new("/tmp/qat-run");
    let paths = daemon_runtime_store_candidate_paths(run_dir);

    assert_eq!(
        paths,
        vec![
            PathBuf::from("/tmp/qat-run/home/xdg-state/bitloops/daemon/runtime.sqlite"),
            PathBuf::from("/tmp/qat-run/home/.local/state/bitloops/daemon/runtime.sqlite"),
            PathBuf::from(
                "/tmp/qat-run/home/Library/Application Support/bitloops/daemon/runtime.sqlite"
            ),
        ]
    );
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

fn command_env_value(command: &std::process::Command, key: &str) -> Option<std::ffi::OsString> {
    command.get_envs().find_map(|(env_key, value)| {
        if env_key == OsStr::new(key) {
            value.map(|v| v.to_os_string())
        } else {
            None
        }
    })
}
