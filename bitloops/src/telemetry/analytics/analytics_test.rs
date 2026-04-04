use super::*;
use crate::config::default_daemon_config_path;
use crate::test_support::process_state::{
    GIT_ENV_KEYS, git_command, with_env_var, with_process_state,
};
use serde_json::Value;
use std::collections::HashMap;
use std::path::Path;

const TEST_STATE_DIR_OVERRIDE_ENV: &str = "BITLOOPS_TEST_STATE_DIR_OVERRIDE";

fn setup_git_repo(path: &Path) {
    let status = git_command()
        .args(["init", "-q"])
        .current_dir(path)
        .status()
        .expect("git init");
    assert!(status.success(), "git init should succeed");
}

fn cleared_git_env() -> Vec<(&'static str, Option<&'static str>)> {
    GIT_ENV_KEYS.iter().map(|key| (*key, None)).collect()
}

fn write_daemon_telemetry_config(config_root: &Path, telemetry: Option<bool>) {
    let config_root_str = config_root.to_string_lossy().to_string();
    let _guard = crate::test_support::process_state::enter_process_state(
        None,
        &[(
            "BITLOOPS_TEST_CONFIG_DIR_OVERRIDE",
            Some(config_root_str.as_str()),
        )],
    );
    let path = default_daemon_config_path().expect("daemon config path");
    std::fs::create_dir_all(path.parent().expect("config parent")).expect("create config dir");

    let content = match telemetry {
        Some(enabled) => format!("[telemetry]\nenabled = {enabled}\n"),
        None => String::new(),
    };
    std::fs::write(&path, content).expect("write daemon config");
}

#[test]
#[allow(non_snake_case)]
fn TestEventPayloadSerialization() {
    let payload = EventPayload {
        event: "bitloops daemon start".to_string(),
        distinct_id: "test-machine-id".to_string(),
        properties: HashMap::from([
            ("surface".to_string(), Value::String("cli".to_string())),
            ("result".to_string(), Value::String("success".to_string())),
            (
                "strategy".to_string(),
                Value::String("manual-commit".to_string()),
            ),
            (
                "agent".to_string(),
                Value::String("claude-code".to_string()),
            ),
            (
                "cli_version".to_string(),
                Value::String("1.0.0".to_string()),
            ),
            ("os".to_string(), Value::String("darwin".to_string())),
            ("arch".to_string(), Value::String("arm64".to_string())),
        ]),
        timestamp: "2026-01-28T12:00:00Z".to_string(),
    };

    let data = serde_json::to_vec(&payload).expect("failed to marshal EventPayload");
    let decoded: EventPayload =
        serde_json::from_slice(&data).expect("failed to unmarshal EventPayload");

    assert_eq!(decoded.event, payload.event);
    assert_eq!(decoded.distinct_id, payload.distinct_id);
    assert_eq!(decoded.timestamp, payload.timestamp);
    assert_eq!(
        decoded.properties.get("surface").and_then(Value::as_str),
        Some("cli")
    );
}

#[test]
#[allow(non_snake_case)]
fn TestTrackActionDetachedSkipsNilAction() {
    let ctx = TelemetryDispatchContext {
        strategy: None,
        agent: None,
    };
    track_action_detached(None, &ctx, "1.0.0", None, true, 12);
}

#[test]
#[allow(non_snake_case)]
fn TestTrackActionDetachedRespectsOptOut() {
    with_env_var(TELEMETRY_OPTOUT_ENV, Some("1"), || {
        let action = ActionDescriptor {
            event: "bitloops daemon status".to_string(),
            surface: "cli",
            properties: HashMap::new(),
        };
        let ctx = TelemetryDispatchContext {
            strategy: None,
            agent: None,
        };

        track_action_detached(Some(&action), &ctx, "1.0.0", None, true, 10);
    });
}

#[test]
#[allow(non_snake_case)]
fn TestBuildActionPayloadCommonEnvelope() {
    with_env_var(
        "BITLOOPS_TELEMETRY_DISTINCT_ID",
        Some("fixed-test-id"),
        || {
            let payload = build_action_payload(
                &ActionDescriptor {
                    event: "bitloops daemon start".to_string(),
                    surface: "cli",
                    properties: HashMap::from([(
                        "flags".to_string(),
                        Value::Array(vec![Value::String("detached".to_string())]),
                    )]),
                },
                &TelemetryDispatchContext {
                    strategy: Some("manual-commit".to_string()),
                    agent: Some("claude-code".to_string()),
                },
                "1.0.0",
                true,
                42,
                Some("session-123".to_string()),
            )
            .expect("payload");

            assert_eq!(payload.event, "bitloops daemon start");
            assert_eq!(
                payload.properties.get("surface").and_then(Value::as_str),
                Some("cli")
            );
            assert_eq!(
                payload.properties.get("result").and_then(Value::as_str),
                Some("success")
            );
            assert_eq!(
                payload
                    .properties
                    .get("duration_ms")
                    .and_then(Value::as_u64),
                Some(42)
            );
            assert_eq!(
                payload.properties.get("agent").and_then(Value::as_str),
                Some("claude-code")
            );
            assert_eq!(
                payload
                    .properties
                    .get("$session_id")
                    .and_then(Value::as_str),
                Some("session-123")
            );
        },
    );
}

#[test]
#[allow(non_snake_case)]
fn TestBuildActionPayloadPreservesDescriptorAgent() {
    with_env_var(
        "BITLOOPS_TELEMETRY_DISTINCT_ID",
        Some("fixed-test-id"),
        || {
            let payload = build_action_payload(
                &ActionDescriptor {
                    event: "bitloops hook".to_string(),
                    surface: "hook",
                    properties: HashMap::from([(
                        "agent".to_string(),
                        Value::String("codex".to_string()),
                    )]),
                },
                &TelemetryDispatchContext {
                    strategy: Some("manual-commit".to_string()),
                    agent: Some("claude-code,codex,cursor,opencode".to_string()),
                },
                "1.0.0",
                true,
                5,
                None,
            )
            .expect("payload");

            assert_eq!(
                payload.properties.get("agent").and_then(Value::as_str),
                Some("codex"),
                "descriptor-specific agent should not be overwritten by dispatch context"
            );
        },
    );
}

#[test]
#[allow(non_snake_case)]
fn TestBuildSessionLifecyclePayloadUsesPosthogEventNames() {
    with_env_var(
        "BITLOOPS_TELEMETRY_DISTINCT_ID",
        Some("fixed-test-id"),
        || {
            let tmp = tempfile::tempdir().unwrap();
            let start = build_session_start_payload("session-123", "manual-commit", "cli")
                .expect("start payload");

            assert_eq!(start.event, SESSION_STARTED_EVENT);
            assert_eq!(
                start.properties.get("$session_id").and_then(Value::as_str),
                Some("session-123")
            );
            assert_eq!(
                start.properties.get("source").and_then(Value::as_str),
                Some("cli")
            );
            assert!(
                !start.properties.contains_key("repo_root"),
                "session start should not include repo_root"
            );

            let ended = crate::telemetry::sessions::EndedSession {
                session_id: "session-123".to_string(),
                repo_root: tmp.path().to_string_lossy().to_string(),
                started_at: 1_700_000_000,
                ended_at: 1_700_000_600,
                duration_secs: 600,
            };
            let end = build_session_end_payload(&ended, "dashboard").expect("end payload");

            assert_eq!(end.event, SESSION_ENDED_EVENT);
            assert_eq!(
                end.properties.get("$session_id").and_then(Value::as_str),
                Some("session-123")
            );
            assert_eq!(
                end.properties
                    .get("$session_duration")
                    .and_then(Value::as_u64),
                Some(600)
            );
            assert_eq!(
                end.properties.get("source").and_then(Value::as_str),
                Some("dashboard")
            );
            assert!(
                !end.properties.contains_key("repo_root"),
                "session end should not include repo_root"
            );
        },
    );
}

#[test]
#[allow(non_snake_case)]
fn TestTrackSessionActivityCreatesSessionStoreEntry() {
    let tmp = tempfile::tempdir().unwrap();
    let state_root = tempfile::tempdir().unwrap();
    let state_root_str = state_root.path().to_string_lossy().to_string();

    with_process_state(
        None,
        &[
            (TEST_STATE_DIR_OVERRIDE_ENV, Some(state_root_str.as_str())),
            ("BITLOOPS_TELEMETRY_DISTINCT_ID", Some("fixed-test-id")),
        ],
        || {
            track_session_activity_detached(tmp.path(), "dashboard", "dashboard");

            let state_dir = crate::utils::platform_dirs::bitloops_state_dir().expect("state dir");
            let store = crate::telemetry::sessions::SessionStore::load(&state_dir);
            assert_eq!(store.sessions().count(), 1);
        },
    );
}

#[test]
#[allow(non_snake_case)]
fn TestProcessSessionActivityScopesExpiredEndEventsToCurrentRepo() {
    let repo_active = tempfile::tempdir().unwrap();
    let repo_other = tempfile::tempdir().unwrap();
    setup_git_repo(repo_active.path());
    setup_git_repo(repo_other.path());

    let state_root = tempfile::tempdir().unwrap();
    let state_root_str = state_root.path().to_string_lossy().to_string();

    let sessions_path = state_root.path().join("telemetry_sessions.json");
    let sessions_json = serde_json::json!({
        "sessions": {
            repo_active.path().to_string_lossy().to_string(): {
                "session_id": "active-expired",
                "started_at": 1,
                "last_event_at": 0
            },
            repo_other.path().to_string_lossy().to_string(): {
                "session_id": "other-expired",
                "started_at": 1,
                "last_event_at": 0
            }
        }
    });
    std::fs::write(&sessions_path, sessions_json.to_string()).expect("write sessions json");

    with_process_state(
        None,
        &[
            (TEST_STATE_DIR_OVERRIDE_ENV, Some(state_root_str.as_str())),
            ("BITLOOPS_TELEMETRY_DISTINCT_ID", Some("fixed-test-id")),
        ],
        || {
            let activity =
                process_session_activity(repo_active.path(), "manual-commit", "hook").expect("activity");
            let end_events = activity
                .lifecycle_events
                .iter()
                .filter(|event| event.event == SESSION_ENDED_EVENT)
                .count();
            assert_eq!(
                end_events, 1,
                "only the current repo should emit an expired session-end event"
            );
        },
    );
}

#[test]
#[allow(non_snake_case)]
fn TestSendEventHandlesInvalidJSON() {
    send_event("invalid json");
    send_event("");
    send_event("{}");
}

#[test]
#[allow(non_snake_case)]
fn TestLoadDispatchContextRequiresExplicitEnablement() {
    let temp_none = tempfile::tempdir().expect("temp dir");
    setup_git_repo(temp_none.path());
    let config_none = tempfile::tempdir().expect("config dir");
    write_daemon_telemetry_config(config_none.path(), None);
    let config_none_str = config_none.path().to_string_lossy().to_string();
    let mut env = cleared_git_env();
    env.push((
        "BITLOOPS_TEST_CONFIG_DIR_OVERRIDE",
        Some(config_none_str.as_str()),
    ));
    with_process_state(Some(temp_none.path()), &env, || {
        assert!(load_dispatch_context().is_none());
    });

    let temp_false = tempfile::tempdir().expect("temp dir");
    setup_git_repo(temp_false.path());
    let config_false = tempfile::tempdir().expect("config dir");
    write_daemon_telemetry_config(config_false.path(), Some(false));
    let config_false_str = config_false.path().to_string_lossy().to_string();
    let mut env = cleared_git_env();
    env.push((
        "BITLOOPS_TEST_CONFIG_DIR_OVERRIDE",
        Some(config_false_str.as_str()),
    ));
    with_process_state(Some(temp_false.path()), &env, || {
        assert!(load_dispatch_context().is_none());
    });
}

#[test]
#[allow(non_snake_case)]
fn TestLoadDispatchContextDetectsAgents() {
    let temp = tempfile::tempdir().expect("temp dir");
    setup_git_repo(temp.path());
    let config = tempfile::tempdir().expect("config dir");
    write_daemon_telemetry_config(config.path(), Some(true));
    std::fs::create_dir_all(temp.path().join(".claude")).expect("create .claude");
    std::fs::create_dir_all(temp.path().join(".codex")).expect("create .codex");
    std::fs::create_dir_all(temp.path().join(".gemini")).expect("create .gemini");
    std::fs::create_dir_all(temp.path().join(".cursor")).expect("create .cursor");
    std::fs::create_dir_all(temp.path().join(".opencode")).expect("create .opencode");

    let config_str = config.path().to_string_lossy().to_string();
    let mut env = cleared_git_env();
    env.push((
        "BITLOOPS_TEST_CONFIG_DIR_OVERRIDE",
        Some(config_str.as_str()),
    ));
    with_process_state(Some(temp.path()), &env, || {
        let context = load_dispatch_context().expect("dispatch context");
        assert_eq!(context.strategy.as_deref(), Some("manual-commit"));
        assert_eq!(
            context.agent.as_deref(),
            Some("claude-code,codex,gemini,cursor,opencode")
        );
    });
}
