use super::agent_hooks::{
    AGENT_CLAUDE_CODE, AGENT_CODEX, AGENT_CURSOR, AGENT_GEMINI, DEFAULT_AGENT,
};
use super::telemetry::{
    TELEMETRY_OPTOUT_ENV, maybe_capture_telemetry_consent, prompt_telemetry_consent,
};
use super::*;
use crate::cli::{Cli, Commands};
use crate::config::load_daemon_settings;
use crate::test_support::process_state::with_process_state;

use clap::Parser;
use std::io::Cursor;
use tempfile::TempDir;

fn setup_git_repo(dir: &TempDir) {
    std::process::Command::new("git")
        .args(["init", "-q"])
        .current_dir(dir.path())
        .status()
        .expect("git init");
}

fn app_dir_env(temp: &TempDir) -> [(&'static str, Option<String>); 4] {
    [
        (
            "BITLOOPS_TEST_CONFIG_DIR_OVERRIDE",
            Some(temp.path().join("config-root").display().to_string()),
        ),
        (
            "BITLOOPS_TEST_DATA_DIR_OVERRIDE",
            Some(temp.path().join("data-root").display().to_string()),
        ),
        (
            "BITLOOPS_TEST_CACHE_DIR_OVERRIDE",
            Some(temp.path().join("cache-root").display().to_string()),
        ),
        (
            "BITLOOPS_TEST_STATE_DIR_OVERRIDE",
            Some(temp.path().join("state-root").display().to_string()),
        ),
    ]
}

fn with_temp_app_dirs<T>(repo_root: &std::path::Path, temp: &TempDir, f: impl FnOnce() -> T) -> T {
    let env_vars = app_dir_env(temp);
    let env_refs = env_vars
        .iter()
        .map(|(key, value)| (*key, value.as_deref()))
        .collect::<Vec<_>>();
    with_process_state(Some(repo_root), &env_refs, f)
}

#[test]
fn init_args_supports_agent_flag() {
    let parsed =
        Cli::try_parse_from(["bitloops", "init", "--agent", "cursor"]).expect("parse init");
    let Some(Commands::Init(args)) = parsed.command else {
        panic!("expected init command");
    };
    assert_eq!(args.agent.as_deref(), Some("cursor"));
}

#[test]
fn init_args_supports_skip_baseline_flag() {
    let parsed = Cli::try_parse_from(["bitloops", "init", "--skip-baseline"]).expect("parse init");
    let Some(Commands::Init(args)) = parsed.command else {
        panic!("expected init command");
    };
    assert!(args.skip_baseline);
}

#[test]
fn init_cmd_agent_flag_no_value_errors() {
    let err = Cli::try_parse_from(["bitloops", "init", "--agent"])
        .err()
        .expect("expected clap parsing error");
    let rendered = err.to_string();
    assert!(
        rendered.contains("a value is required") || rendered.contains("requires a value"),
        "unexpected clap error: {rendered}"
    );
}

#[test]
fn run_init_creates_global_daemon_config() {
    let repo = tempfile::tempdir().expect("repo tempdir");
    let app_dirs = tempfile::tempdir().expect("app tempdir");
    setup_git_repo(&repo);

    with_temp_app_dirs(repo.path(), &app_dirs, || {
        let mut out = Vec::new();
        run_with_writer(
            InitArgs {
                force: false,
                agent: None,
                telemetry: true,
                skip_baseline: false,
            },
            &mut out,
            None,
        )
        .expect("run init");

        let loaded = load_daemon_settings(None).expect("load daemon settings");
        assert!(loaded.path.exists(), "daemon config should exist");

        let rendered = String::from_utf8(out).expect("utf8 output");
        assert!(rendered.contains("Daemon config:"));
        assert!(rendered.contains("Bitloops daemon configuration is ready."));
    });
}

#[test]
fn run_init_with_deprecated_flags_prints_note_and_does_not_install_hooks() {
    let repo = tempfile::tempdir().expect("repo tempdir");
    let app_dirs = tempfile::tempdir().expect("app tempdir");
    setup_git_repo(&repo);

    with_temp_app_dirs(repo.path(), &app_dirs, || {
        let mut out = Vec::new();
        run_with_writer(
            InitArgs {
                force: true,
                agent: Some(AGENT_CLAUDE_CODE.to_string()),
                telemetry: true,
                skip_baseline: true,
            },
            &mut out,
            None,
        )
        .expect("run init");

        let rendered = String::from_utf8(out).expect("utf8 output");
        assert!(rendered.contains("Use `bitloops enable` for hooks."));
        assert!(!repo.path().join(".claude/settings.json").exists());
        assert!(!repo.path().join(".cursor/hooks.json").exists());
        assert!(!repo.path().join(".codex/hooks.json").exists());
        assert!(!repo.path().join(".gemini/settings.json").exists());
    });
}

#[test]
fn detect_or_select_agent_no_detection_no_tty_falls_back_to_default() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    with_process_state(
        Some(dir.path()),
        &[("BITLOOPS_TEST_TTY", Some("0"))],
        || {
            let mut out = Vec::new();
            let selected = detect_or_select_agent(dir.path(), &mut out, None).unwrap();
            assert_eq!(selected, vec![DEFAULT_AGENT.to_string()]);
        },
    );
}

#[test]
fn detect_or_select_agent_agent_detected() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    std::fs::create_dir_all(dir.path().join(".claude")).unwrap();
    with_process_state(
        Some(dir.path()),
        &[("BITLOOPS_TEST_TTY", Some("0"))],
        || {
            let mut out = Vec::new();
            let selected = detect_or_select_agent(dir.path(), &mut out, None).unwrap();
            assert_eq!(selected, vec![AGENT_CLAUDE_CODE.to_string()]);
        },
    );
}

#[test]
fn detect_or_select_agent_single_detected_with_tty_uses_selector() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    std::fs::create_dir_all(dir.path().join(".claude")).unwrap();

    let select = |_available: &[String]| -> std::result::Result<Vec<String>, String> {
        Ok(vec![AGENT_CURSOR.to_string()])
    };

    with_process_state(
        Some(dir.path()),
        &[("BITLOOPS_TEST_TTY", Some("1"))],
        || {
            let mut out = Vec::new();
            let selected = detect_or_select_agent(dir.path(), &mut out, Some(&select)).unwrap();
            assert_eq!(selected, vec![AGENT_CURSOR.to_string()]);
        },
    );
}

#[test]
fn detect_or_select_agent_selection_cancelled() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    let select = |_available: &[String]| -> std::result::Result<Vec<String>, String> {
        Err("user cancelled".to_string())
    };
    with_process_state(
        Some(dir.path()),
        &[("BITLOOPS_TEST_TTY", Some("1"))],
        || {
            let mut out = Vec::new();
            let err = detect_or_select_agent(dir.path(), &mut out, Some(&select)).unwrap_err();
            assert!(format!("{err:#}").contains("user cancelled"));
        },
    );
}

#[test]
fn detect_or_select_agent_none_selected_errors() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    let select = |_available: &[String]| -> std::result::Result<Vec<String>, String> { Ok(vec![]) };
    with_process_state(
        Some(dir.path()),
        &[("BITLOOPS_TEST_TTY", Some("1"))],
        || {
            let mut out = Vec::new();
            let err = detect_or_select_agent(dir.path(), &mut out, Some(&select)).unwrap_err();
            assert!(format!("{err:#}").contains("no agents selected"));
        },
    );
}

#[test]
fn detect_or_select_agent_no_tty_returns_all_detected() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    std::fs::create_dir_all(dir.path().join(".claude")).unwrap();
    std::fs::create_dir_all(dir.path().join(".gemini")).unwrap();
    with_process_state(
        Some(dir.path()),
        &[("BITLOOPS_TEST_TTY", Some("0"))],
        || {
            let mut out = Vec::new();
            let selected = detect_or_select_agent(dir.path(), &mut out, None).unwrap();
            assert_eq!(selected.len(), 2);
            assert!(selected.contains(&AGENT_CLAUDE_CODE.to_string()));
            assert!(selected.contains(&AGENT_GEMINI.to_string()));
        },
    );
}

#[test]
fn detect_or_select_agent_multiple_with_selector() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    std::fs::create_dir_all(dir.path().join(".claude")).unwrap();
    std::fs::create_dir_all(dir.path().join(".gemini")).unwrap();
    let select = |_available: &[String]| -> std::result::Result<Vec<String>, String> {
        Ok(vec![
            AGENT_GEMINI.to_string(),
            AGENT_CODEX.to_string(),
            AGENT_CLAUDE_CODE.to_string(),
        ])
    };
    with_process_state(
        Some(dir.path()),
        &[("BITLOOPS_TEST_TTY", Some("1"))],
        || {
            let mut out = Vec::new();
            let selected = detect_or_select_agent(dir.path(), &mut out, Some(&select)).unwrap();
            assert_eq!(
                selected,
                vec![
                    AGENT_GEMINI.to_string(),
                    AGENT_CODEX.to_string(),
                    AGENT_CLAUDE_CODE.to_string()
                ]
            );
        },
    );
}

#[test]
fn init_args_supports_telemetry_flag() {
    let parsed = Cli::try_parse_from(["bitloops", "init", "--telemetry=false"])
        .expect("parse init telemetry flag");
    let Some(Commands::Init(args)) = parsed.command else {
        panic!("expected init command");
    };
    assert!(!args.telemetry);
}

#[test]
fn prompt_telemetry_consent_defaults_yes() {
    let mut out = Vec::new();
    let mut input = Cursor::new("\n");
    let consent = prompt_telemetry_consent(&mut out, &mut input).expect("telemetry prompt");
    assert!(consent);
}

#[test]
fn prompt_telemetry_consent_accepts_no() {
    let mut out = Vec::new();
    let mut input = Cursor::new("no\n");
    let consent = prompt_telemetry_consent(&mut out, &mut input).expect("telemetry prompt");
    assert!(!consent);
}

#[test]
fn maybe_capture_telemetry_consent_flag_false_disables() {
    let repo = tempfile::tempdir().unwrap();
    let app_dirs = tempfile::tempdir().unwrap();
    setup_git_repo(&repo);

    with_temp_app_dirs(repo.path(), &app_dirs, || {
        let mut out = Vec::new();
        maybe_capture_telemetry_consent(repo.path(), false, true, &mut out)
            .expect("telemetry config");

        let loaded = load_daemon_settings(None).expect("load daemon settings");
        assert_eq!(loaded.cli.telemetry, Some(false));
    });
}

#[test]
fn maybe_capture_telemetry_consent_env_optout_disables() {
    let repo = tempfile::tempdir().unwrap();
    let app_dirs = tempfile::tempdir().unwrap();
    setup_git_repo(&repo);

    let env_vars = app_dir_env(&app_dirs);
    let mut combined = env_vars
        .iter()
        .map(|(key, value)| (*key, value.as_deref()))
        .collect::<Vec<_>>();
    combined.push((TELEMETRY_OPTOUT_ENV, Some("1")));

    with_process_state(Some(repo.path()), &combined, || {
        let mut out = Vec::new();
        maybe_capture_telemetry_consent(repo.path(), true, true, &mut out)
            .expect("telemetry config");

        let loaded = load_daemon_settings(None).expect("load daemon settings");
        assert_eq!(loaded.cli.telemetry, Some(false));
    });
}

#[test]
fn maybe_capture_telemetry_consent_no_tty_leaves_unset() {
    let repo = tempfile::tempdir().unwrap();
    let app_dirs = tempfile::tempdir().unwrap();
    setup_git_repo(&repo);

    with_process_state(
        Some(repo.path()),
        &[
            ("BITLOOPS_TEST_TTY", Some("0")),
            (
                "BITLOOPS_TEST_CONFIG_DIR_OVERRIDE",
                Some(app_dirs.path().join("config-root").to_str().unwrap()),
            ),
            (
                "BITLOOPS_TEST_DATA_DIR_OVERRIDE",
                Some(app_dirs.path().join("data-root").to_str().unwrap()),
            ),
            (
                "BITLOOPS_TEST_CACHE_DIR_OVERRIDE",
                Some(app_dirs.path().join("cache-root").to_str().unwrap()),
            ),
            (
                "BITLOOPS_TEST_STATE_DIR_OVERRIDE",
                Some(app_dirs.path().join("state-root").to_str().unwrap()),
            ),
        ],
        || {
            let mut out = Vec::new();
            maybe_capture_telemetry_consent(repo.path(), true, true, &mut out)
                .expect("telemetry config");

            let loaded = load_daemon_settings(None).expect("load daemon settings");
            assert_eq!(loaded.cli.telemetry, None);
        },
    );
}
