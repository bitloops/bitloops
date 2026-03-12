use super::*;
use crate::test_support::logger_lock::with_logger_test_lock;
use crate::test_support::process_state::{with_cwd, with_process_state};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, SystemTime};
use tempfile::TempDir;

const TEST_SESSION_ID: &str = "2025-01-15-test-session";
const TEST_COMPONENT: &str = "hooks";
const TEST_AGENT: &str = "claude-code";

struct TestWorkspace {
    tmp_dir: TempDir,
}

impl TestWorkspace {
    fn setup() -> Self {
        let tmp_dir = tempfile::tempdir().expect("create temp dir");
        init_git_repo(tmp_dir.path());
        Self { tmp_dir }
    }

    fn path(&self) -> &Path {
        self.tmp_dir.path()
    }
}

fn run_cmd(dir: &Path, name: &str, args: &[&str]) {
    let mut cmd = Command::new(name);
    cmd.args(args).current_dir(dir).stdin(Stdio::null());

    // For git commands, avoid reading system/global config files
    if name == "git" {
        cmd.env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_SYSTEM", "/dev/null");
    }

    let output = cmd.output().expect("execute command");
    assert!(
        output.status.success(),
        "command failed: {} {:?}\nstdout: {}\nstderr: {}",
        name,
        args,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn init_git_repo(path: &Path) {
    run_cmd(path, "git", &["init"]);
    run_cmd(path, "git", &["config", "user.email", "test@test.com"]);
    run_cmd(path, "git", &["config", "user.name", "Test User"]);
}

fn test_log_file_path(tmp_dir: &Path) -> PathBuf {
    tmp_dir.join(".bitloops").join("logs").join("bitloops.log")
}

fn read_single_log_entry(log_path: &Path) -> serde_json::Value {
    let content = fs::read_to_string(log_path)
        .unwrap_or_else(|err| panic!("failed to read {}: {err}", log_path.display()));
    let line = content.lines().next().unwrap_or("");
    serde_json::from_str(line).unwrap_or_else(|err| {
        panic!(
            "log output should be valid JSON ({}): {err}\ncontent: {}",
            log_path.display(),
            content
        )
    })
}

#[test]
#[allow(non_snake_case)]
fn TestParseLogLevel() {
    let tests = [
        ("empty defaults to INFO", "", LogLevel::Info),
        ("DEBUG lowercase", "debug", LogLevel::Debug),
        ("DEBUG uppercase", "DEBUG", LogLevel::Debug),
        ("INFO lowercase", "info", LogLevel::Info),
        ("INFO uppercase", "INFO", LogLevel::Info),
        ("WARN lowercase", "warn", LogLevel::Warn),
        ("WARN uppercase", "WARN", LogLevel::Warn),
        ("ERROR lowercase", "error", LogLevel::Error),
        ("ERROR uppercase", "ERROR", LogLevel::Error),
        ("invalid defaults to INFO", "invalid", LogLevel::Info),
        ("warning alias", "warning", LogLevel::Warn),
    ];

    for (name, env_value, want) in tests {
        let got = parse_log_level(env_value);
        assert_eq!(got, want, "case {name}: parse_log_level mismatch");
    }
}

#[test]
#[allow(non_snake_case)]
fn TestInit_CreatesLogDirectory() {
    let ws = TestWorkspace::setup();
    with_cwd(ws.path(), || {
        with_logger_test_lock(|| {
            reset_logger_for_tests();
            let result = init(TEST_SESSION_ID);
            assert!(result.is_ok(), "init should succeed: {result:?}");
            close();

            let logs_dir = ws.path().join(".bitloops").join("logs");
            assert!(
                logs_dir.exists(),
                "init should create .bitloops/logs directory at {}",
                logs_dir.display()
            );
        });
    });
}

#[test]
#[allow(non_snake_case)]
fn TestInit_CreatesLogFile() {
    let ws = TestWorkspace::setup();
    with_cwd(ws.path(), || {
        with_logger_test_lock(|| {
            reset_logger_for_tests();
            let result = init(TEST_SESSION_ID);
            assert!(result.is_ok(), "init should succeed: {result:?}");
            close();

            let log_file = test_log_file_path(ws.path());
            assert!(
                log_file.exists(),
                "init should create log file at {}",
                log_file.display()
            );
        });
    });
}

#[test]
#[allow(non_snake_case)]
fn TestInit_WritesJSONLogs() {
    let ws = TestWorkspace::setup();
    with_cwd(ws.path(), || {
        with_logger_test_lock(|| {
            reset_logger_for_tests();
            let result = init("2025-01-15-json-test");
            assert!(result.is_ok(), "init should succeed: {result:?}");

            info(
                &background(),
                "test message",
                &[string_attr("key", "value")],
            );
            close();

            let log_entry = read_single_log_entry(&test_log_file_path(ws.path()));
            assert_eq!(log_entry["msg"], "test message", "msg field mismatch");
            assert_eq!(log_entry["key"], "value", "custom attr mismatch");
            assert!(
                log_entry.get("time").is_some(),
                "time field should be present"
            );
            assert!(
                log_entry.get("level").is_some(),
                "level field should be present"
            );
        });
    });
}

#[test]
#[allow(non_snake_case)]
fn TestInit_RespectsLogLevel() {
    let ws = TestWorkspace::setup();
    with_process_state(
        Some(ws.path()),
        &[(LOG_LEVEL_ENV_VAR, Some("WARN"))],
        || {
            with_logger_test_lock(|| {
                let session_id = "2025-01-15-level-test";

                reset_logger_for_tests();

                let result = init(session_id);
                assert!(result.is_ok(), "init should succeed: {result:?}");

                let ctx = background();
                debug(&ctx, "debug message", &[]);
                info(&ctx, "info message", &[]);
                warn(&ctx, "warn message", &[]);
                close();

                let content = fs::read_to_string(test_log_file_path(ws.path()))
                    .expect("log file should exist and be readable");
                let entries: Vec<serde_json::Value> = content
                    .lines()
                    .map(|line| {
                        serde_json::from_str(line).expect("log output lines should be valid JSON")
                    })
                    .collect();

                let session_entries: Vec<&serde_json::Value> = entries
                    .iter()
                    .filter(|entry| {
                        entry.get("session_id").and_then(serde_json::Value::as_str)
                            == Some(session_id)
                    })
                    .collect();

                assert!(
                    !session_entries.is_empty(),
                    "expected at least one entry for session {session_id}"
                );

                let has_debug = session_entries.iter().any(|entry| {
                    entry.get("msg").and_then(serde_json::Value::as_str) == Some("debug message")
                });
                let has_info = session_entries.iter().any(|entry| {
                    entry.get("msg").and_then(serde_json::Value::as_str) == Some("info message")
                });
                let has_warn = session_entries.iter().any(|entry| {
                    entry.get("msg").and_then(serde_json::Value::as_str) == Some("warn message")
                });

                assert!(
                    !has_debug,
                    "DEBUG messages should be filtered out at WARN level"
                );
                assert!(
                    !has_info,
                    "INFO messages should be filtered out at WARN level"
                );
                assert!(has_warn, "WARN messages should be included at WARN level");
            });
        },
    );
}

#[test]
#[allow(non_snake_case)]
fn TestInit_InvalidLogLevelWarns() {
    let ws = TestWorkspace::setup();
    with_process_state(
        Some(ws.path()),
        &[(LOG_LEVEL_ENV_VAR, Some("INVALID_LEVEL"))],
        || {
            with_logger_test_lock(|| {
                reset_logger_for_tests();
                start_stderr_capture_for_tests();

                let result = init("2025-01-15-invalid-level");
                assert!(
                    result.is_ok(),
                    "init should not fail when log level is invalid: {result:?}"
                );
                close();

                let stderr_output = take_stderr_capture_for_tests();

                assert_eq!(
                    parse_log_level("INVALID_LEVEL"),
                    LogLevel::Info,
                    "invalid log level should default to INFO"
                );
                assert!(
                    stderr_output.contains("invalid log level"),
                    "expected warning about invalid log level on stderr, got: {stderr_output}"
                );
            });
        },
    );
}

#[test]
#[allow(non_snake_case)]
fn TestInit_FallsBackToStderrOnError() {
    let ws = TestWorkspace::setup();
    with_cwd(ws.path(), || {
        with_logger_test_lock(|| {
            reset_logger_for_tests();

            let logs_dir = ws.path().join(".bitloops").join("logs");
            fs::create_dir_all(&logs_dir).expect("create logs dir");
            let blocker = test_log_file_path(ws.path());
            fs::create_dir_all(&blocker).expect("create blocking directory");

            let result = init(TEST_SESSION_ID);
            assert!(
                result.is_ok(),
                "init should fall back to stderr instead of returning error"
            );

            info(&background(), "fallback test", &[]);
            close();
        });
    });
}

#[test]
#[allow(non_snake_case)]
fn TestClose_SafeToCallMultipleTimes() {
    let ws = TestWorkspace::setup();
    with_cwd(ws.path(), || {
        with_logger_test_lock(|| {
            reset_logger_for_tests();
            let result = init("2025-01-15-close-test");
            assert!(result.is_ok(), "init should succeed: {result:?}");

            close();
            close();
            close();
        });
    });
}

#[test]
#[allow(non_snake_case)]
fn TestLogging_BeforeInit() {
    with_logger_test_lock(|| {
        reset_logger_for_tests();
        let ctx = background();

        let result = std::panic::catch_unwind(|| {
            debug(&ctx, "debug before init", &[]);
            info(&ctx, "info before init", &[]);
            warn(&ctx, "warn before init", &[]);
            error(&ctx, "error before init", &[]);
        });

        assert!(
            result.is_ok(),
            "logging before init should not panic and should use stderr fallback"
        );
    });
}

#[test]
#[allow(non_snake_case)]
fn TestLogging_IncludesContextValues() {
    let ws = TestWorkspace::setup();
    with_cwd(ws.path(), || {
        with_logger_test_lock(|| {
            reset_logger_for_tests();
            let session_id = "2025-01-15-context-test";
            let result = init(session_id);
            assert!(result.is_ok(), "init should succeed: {result:?}");

            let mut ctx = background();
            ctx = with_session(ctx, "context-session-id");
            ctx = with_tool_call(ctx, "toolu_123");
            ctx = with_component(ctx, TEST_COMPONENT);
            ctx = with_agent(ctx, TEST_AGENT);

            info(&ctx, "context test message", &[]);
            close();

            let log_entry = read_single_log_entry(&test_log_file_path(ws.path()));
            assert_eq!(
                log_entry["session_id"], session_id,
                "session_id should come from init global session"
            );
            assert_eq!(
                log_entry["tool_call_id"], "toolu_123",
                "tool_call_id mismatch"
            );
            assert_eq!(log_entry["component"], TEST_COMPONENT, "component mismatch");
            assert_eq!(log_entry["agent"], TEST_AGENT, "agent mismatch");
        });
    });
}

#[test]
#[allow(non_snake_case)]
fn TestLogging_ParentSessionID() {
    let ws = TestWorkspace::setup();
    with_cwd(ws.path(), || {
        with_logger_test_lock(|| {
            reset_logger_for_tests();
            let session_id = "2025-01-15-parent-test";
            let result = init(session_id);
            assert!(result.is_ok(), "init should succeed: {result:?}");

            let mut ctx = background();
            ctx = with_session(ctx, "parent-session");
            ctx = with_session(ctx, "child-session");

            info(&ctx, "nested session test", &[]);
            close();

            let log_entry = read_single_log_entry(&test_log_file_path(ws.path()));
            assert_eq!(
                log_entry["session_id"], session_id,
                "session_id should come from init global session"
            );
            assert_eq!(
                log_entry["parent_session_id"], "parent-session",
                "parent_session_id should be preserved from context"
            );
        });
    });
}

#[test]
#[allow(non_snake_case)]
fn TestLogging_AdditionalAttrs() {
    let ws = TestWorkspace::setup();
    with_cwd(ws.path(), || {
        with_logger_test_lock(|| {
            reset_logger_for_tests();
            let session_id = "2025-01-15-attrs-test";
            let result = init(session_id);
            assert!(result.is_ok(), "init should succeed: {result:?}");

            let ctx = with_session(background(), "context-session");
            info(
                &ctx,
                "attrs test",
                &[
                    string_attr("hook", "pre-push"),
                    int_attr("duration_ms", 150),
                    bool_attr("success", true),
                ],
            );
            close();

            let log_entry = read_single_log_entry(&test_log_file_path(ws.path()));
            assert_eq!(
                log_entry["session_id"], session_id,
                "session_id should come from init global session"
            );
            assert_eq!(log_entry["hook"], "pre-push", "hook attr mismatch");
            assert_eq!(log_entry["duration_ms"], 150, "duration_ms attr mismatch");
            assert_eq!(log_entry["success"], true, "success attr mismatch");
        });
    });
}

#[test]
#[allow(non_snake_case)]
fn TestLogDuration() {
    let ws = TestWorkspace::setup();
    with_cwd(ws.path(), || {
        with_logger_test_lock(|| {
            reset_logger_for_tests();
            let session_id = "2025-01-15-duration-test";
            let result = init(session_id);
            assert!(result.is_ok(), "init should succeed: {result:?}");

            let mut ctx = with_session(background(), "context-session");
            ctx = with_component(ctx, TEST_COMPONENT);

            let start = SystemTime::now()
                .checked_sub(Duration::from_millis(100))
                .expect("subtract 100ms");
            log_duration(
                &ctx,
                LogLevel::Info,
                "operation completed",
                start,
                &[string_attr("hook", "pre-push"), bool_attr("success", true)],
            );
            close();

            let log_entry = read_single_log_entry(&test_log_file_path(ws.path()));
            let duration_ms = log_entry["duration_ms"]
                .as_i64()
                .expect("duration_ms should be a number");
            assert!(
                (90..=200).contains(&duration_ms),
                "duration_ms should be around 100ms, got {duration_ms}"
            );
            assert_eq!(
                log_entry["session_id"], session_id,
                "session_id should come from init global session"
            );
            assert_eq!(log_entry["component"], TEST_COMPONENT, "component mismatch");
            assert_eq!(log_entry["hook"], "pre-push", "hook attr mismatch");
            assert_eq!(log_entry["success"], true, "success attr mismatch");
            assert_eq!(log_entry["level"], "INFO", "level should be INFO");
        });
    });
}

#[test]
#[allow(non_snake_case)]
fn TestLogging_ContextSessionID_WhenNoGlobalSet() {
    with_logger_test_lock(|| {
        reset_logger_for_tests();
        set_test_logger_without_global_session_for_tests();

        let mut ctx = background();
        ctx = with_session(ctx, "context-only-session");
        ctx = with_component(ctx, TEST_COMPONENT);

        info(&ctx, "context session test", &[]);

        let entry = take_last_log_entry_for_tests().expect("expected captured log entry");
        assert_eq!(
            entry["session_id"], "context-only-session",
            "when no global session is set, session_id should come from context"
        );
    });
}

#[test]
#[allow(non_snake_case)]
fn TestInit_RejectsInvalidSessionIDs() {
    with_logger_test_lock(reset_logger_for_tests);

    let tests = [
        ("empty session ID is allowed", "", false),
        ("path traversal with slash", "../../../tmp/evil", true),
        ("path traversal with backslash", "..\\..\\tmp\\evil", true),
        ("contains forward slash", "2025-01-15/session", true),
        ("contains backslash", "2025-01-15\\session", true),
        ("valid session ID", "2025-01-15-valid-session", false),
        ("valid UUID-like ID", "abc123-def456-ghi789", false),
    ];

    for (name, session_id, want_err) in tests {
        with_logger_test_lock(reset_logger_for_tests);

        if !want_err {
            let ws = TestWorkspace::setup();
            with_cwd(ws.path(), || {
                with_logger_test_lock(|| {
                    let err = init(session_id).err();
                    assert_eq!(
                        err.is_some(),
                        want_err,
                        "case {name}: init({session_id:?}) error mismatch"
                    );
                    close();
                });
            });
            continue;
        }

        let err = with_logger_test_lock(|| init(session_id).err());
        assert_eq!(
            err.is_some(),
            want_err,
            "case {name}: init({session_id:?}) error mismatch"
        );
        if let Some(err) = err {
            let msg = err.to_string().to_lowercase();
            assert!(
                msg.contains("session id"),
                "case {name}: error should mention session ID, got {msg}"
            );
        }
        with_logger_test_lock(close);
    }
}
