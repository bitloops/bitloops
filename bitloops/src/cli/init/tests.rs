use super::agent_hooks::{
    AGENT_CLAUDE_CODE, AGENT_CODEX, AGENT_CURSOR, AGENT_GEMINI, DEFAULT_AGENT,
};
use super::telemetry::{
    TELEMETRY_OPTOUT_ENV, maybe_capture_telemetry_consent, prompt_telemetry_consent,
};
use super::*;
use crate::cli::{Cli, Commands};
use crate::config::settings;
use crate::test_support::process_state::{with_cwd, with_env_var, with_process_state};
use crate::utils::paths;

use clap::Parser;
use std::io::Cursor;

fn setup_git_repo(dir: &tempfile::TempDir) {
    std::process::Command::new("git")
        .args(["init", "-q"])
        .current_dir(dir.path())
        .status()
        .expect("git init");
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
fn run_init_with_unknown_agent_returns_error() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    with_cwd(dir.path(), || {
        let mut out = Vec::new();
        let err = run_with_writer(
            InitArgs {
                force: false,
                agent: Some("bad-agent".to_string()),
                telemetry: true,
                skip_baseline: false,
            },
            &mut out,
            None,
        )
        .unwrap_err();
        assert!(format!("{err:#}").contains("unknown agent name"));
    });
}

#[test]
fn run_init_creates_default_store_databases_and_blob_directory() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    with_cwd(dir.path(), || {
        let mut out = Vec::new();
        run_with_writer(
            InitArgs {
                force: false,
                agent: Some(AGENT_CLAUDE_CODE.to_string()),
                telemetry: true,
                skip_baseline: false,
            },
            &mut out,
            None,
        )
        .unwrap();

        let sqlite_path = dir
            .path()
            .join(paths::BITLOOPS_RELATIONAL_STORE_DIR)
            .join(paths::RELATIONAL_DB_FILE_NAME);
        let duckdb_path = dir
            .path()
            .join(paths::BITLOOPS_EVENT_STORE_DIR)
            .join(paths::EVENTS_DB_FILE_NAME);
        let blob_dir = dir.path().join(paths::BITLOOPS_BLOB_STORE_DIR);

        assert!(
            sqlite_path.is_file(),
            "expected sqlite db at {}",
            sqlite_path.display()
        );
        assert!(
            duckdb_path.is_file(),
            "expected duckdb db at {}",
            duckdb_path.display()
        );
        assert!(
            blob_dir.is_dir(),
            "expected blob dir at {}",
            blob_dir.display()
        );

        let sqlite = rusqlite::Connection::open(&sqlite_path).expect("open sqlite db");
        let sessions_table_count: i64 = sqlite
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'sessions'",
                [],
                |row| row.get(0),
            )
            .expect("query sqlite sessions table");
        assert_eq!(sessions_table_count, 1);

        let duckdb = duckdb::Connection::open(&duckdb_path).expect("open duckdb db");
        let mut stmt = duckdb
            .prepare(
                "SELECT COUNT(*) FROM information_schema.tables WHERE table_name = 'checkpoint_events'",
            )
            .expect("prepare duckdb schema query");
        let mut rows = stmt.query([]).expect("query duckdb schema");
        let row = rows
            .next()
            .expect("read duckdb row")
            .expect("duckdb row exists");
        let table_count: i64 = row.get(0).expect("read duckdb count");
        assert_eq!(table_count, 1);
    });
}

#[test]
fn run_init_respects_repo_level_configured_store_paths() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    let bitloops_dir = dir.path().join(".bitloops");
    std::fs::create_dir_all(&bitloops_dir).expect("create .bitloops directory");
    std::fs::write(
        bitloops_dir.join("config.json"),
        r#"{
  "version": "1.0",
  "scope": "project",
  "settings": {
    "stores": {
      "relational": {
        "sqlite_path": ".custom/relational/custom-relational.db"
      },
      "event": {
        "duckdb_path": ".custom/event/custom-events.duckdb"
      },
      "blob": {
        "local_path": ".custom/blob-store"
      }
    }
  }
}"#,
    )
    .expect("write repo-level config");

    with_cwd(dir.path(), || {
        let mut out = Vec::new();
        run_with_writer(
            InitArgs {
                force: false,
                agent: Some(AGENT_CLAUDE_CODE.to_string()),
                telemetry: true,
                skip_baseline: false,
            },
            &mut out,
            None,
        )
        .unwrap();

        assert!(
            dir.path()
                .join(".custom/relational/custom-relational.db")
                .is_file()
        );
        assert!(
            dir.path()
                .join(".custom/event/custom-events.duckdb")
                .is_file()
        );
        assert!(dir.path().join(".custom/blob-store").is_dir());
    });
}

#[test]
fn run_init_with_agent_claude_installs_claude_hooks() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    with_cwd(dir.path(), || {
        let mut out = Vec::new();
        run_with_writer(
            InitArgs {
                force: false,
                agent: Some(AGENT_CLAUDE_CODE.to_string()),
                telemetry: true,
                skip_baseline: false,
            },
            &mut out,
            None,
        )
        .unwrap();
        assert!(dir.path().join(".claude/settings.json").exists());
    });
}

#[test]
fn run_init_with_agent_cursor_installs_cursor_hooks() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    with_cwd(dir.path(), || {
        let mut out = Vec::new();
        run_with_writer(
            InitArgs {
                force: false,
                agent: Some(AGENT_CURSOR.to_string()),
                telemetry: true,
                skip_baseline: false,
            },
            &mut out,
            None,
        )
        .unwrap();

        let hooks = std::fs::read_to_string(dir.path().join(".cursor/hooks.json")).unwrap();
        assert!(hooks.contains("bitloops hooks cursor session-start"));
    });
}

#[test]
fn run_init_with_agent_codex_installs_codex_hooks() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    with_cwd(dir.path(), || {
        let mut out = Vec::new();
        run_with_writer(
            InitArgs {
                force: false,
                agent: Some(AGENT_CODEX.to_string()),
                telemetry: true,
                skip_baseline: false,
            },
            &mut out,
            None,
        )
        .unwrap();

        let hooks = std::fs::read_to_string(dir.path().join(".codex/hooks.json")).unwrap();
        assert!(hooks.contains("bitloops hooks codex session-start"));
        assert!(hooks.contains("bitloops hooks codex stop"));
    });
}

#[test]
fn run_init_with_agent_gemini_installs_gemini_hooks() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    with_cwd(dir.path(), || {
        let mut out = Vec::new();
        run_with_writer(
            InitArgs {
                force: false,
                agent: Some(AGENT_GEMINI.to_string()),
                telemetry: true,
                skip_baseline: false,
            },
            &mut out,
            None,
        )
        .unwrap();
        assert!(dir.path().join(".gemini/settings.json").exists());
    });
}

#[test]
fn run_init_with_force_reinstalls_claude_hooks() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    with_cwd(dir.path(), || {
        let mut first_out = Vec::new();
        run_with_writer(
            InitArgs {
                force: false,
                agent: Some(AGENT_CLAUDE_CODE.to_string()),
                telemetry: true,
                skip_baseline: false,
            },
            &mut first_out,
            None,
        )
        .unwrap();
        let mut second_out = Vec::new();
        run_with_writer(
            InitArgs {
                force: true,
                agent: Some(AGENT_CLAUDE_CODE.to_string()),
                telemetry: true,
                skip_baseline: false,
            },
            &mut second_out,
            None,
        )
        .unwrap();
        let second = String::from_utf8(second_out).unwrap();
        assert!(second.contains("Installed"));
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
                vec![AGENT_GEMINI.to_string(), AGENT_CLAUDE_CODE.to_string()]
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
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);

    let mut out = Vec::new();
    maybe_capture_telemetry_consent(dir.path(), false, true, &mut out).expect("telemetry config");

    let merged = settings::load_settings(dir.path()).expect("load settings");
    assert_eq!(merged.telemetry, Some(false));
}

#[test]
fn maybe_capture_telemetry_consent_env_optout_disables() {
    with_env_var(TELEMETRY_OPTOUT_ENV, Some("1"), || {
        let dir = tempfile::tempdir().unwrap();
        setup_git_repo(&dir);

        let mut out = Vec::new();
        maybe_capture_telemetry_consent(dir.path(), true, true, &mut out)
            .expect("telemetry config");

        let merged = settings::load_settings(dir.path()).expect("load settings");
        assert_eq!(merged.telemetry, Some(false));
    });
}

#[test]
fn maybe_capture_telemetry_consent_no_tty_leaves_unset() {
    with_env_var("BITLOOPS_TEST_TTY", Some("0"), || {
        let dir = tempfile::tempdir().unwrap();
        setup_git_repo(&dir);

        let mut out = Vec::new();
        maybe_capture_telemetry_consent(dir.path(), true, true, &mut out)
            .expect("telemetry config");

        let merged = settings::load_settings(dir.path()).expect("load settings");
        assert_eq!(merged.telemetry, None);
    });
}
