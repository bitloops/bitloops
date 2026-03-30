use super::*;
use crate::config::default_daemon_config_path;
use crate::test_support::process_state::{
    GIT_ENV_KEYS, git_command, with_env_var, with_process_state,
};
use serde_json::Value;
use std::collections::HashMap;
use std::path::Path;

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
        event: "cli_command_executed".to_string(),
        distinct_id: "test-machine-id".to_string(),
        properties: HashMap::from([
            (
                "command".to_string(),
                Value::String("bitloops status".to_string()),
            ),
            (
                "strategy".to_string(),
                Value::String("manual-commit".to_string()),
            ),
            (
                "agent".to_string(),
                Value::String("claude-code".to_string()),
            ),
            ("isBitloopsEnabled".to_string(), Value::Bool(true)),
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
        decoded.properties.get("command").and_then(Value::as_str),
        Some("bitloops status")
    );
}

#[test]
#[allow(non_snake_case)]
fn TestTrackCommandDetachedSkipsNilCommand() {
    track_command_detached(None, "manual-commit", "claude-code", true, "1.0.0");
}

#[test]
#[allow(non_snake_case)]
fn TestTrackCommandDetachedSkipsHiddenCommands() {
    let hidden_cmd = CommandInfo {
        command_path: "__send_analytics".to_string(),
        hidden: true,
        flag_names: Vec::new(),
    };

    track_command_detached(
        Some(&hidden_cmd),
        "manual-commit",
        "claude-code",
        true,
        "1.0.0",
    );
}

#[test]
#[allow(non_snake_case)]
fn TestTrackCommandDetachedRespectsOptOut() {
    with_env_var(TELEMETRY_OPTOUT_ENV, Some("1"), || {
        let cmd = CommandInfo {
            command_path: "status".to_string(),
            hidden: false,
            flag_names: Vec::new(),
        };

        track_command_detached(Some(&cmd), "manual-commit", "claude-code", true, "1.0.0");
    });
}

#[test]
#[allow(non_snake_case)]
fn TestBuildEventPayloadAgent() {
    with_env_var(
        "BITLOOPS_TELEMETRY_DISTINCT_ID",
        Some("fixed-test-id"),
        || {
            let tests = [
                ("defaults empty to auto", "", "auto"),
                ("preserves explicit agent", "claude-code", "claude-code"),
            ];

            for (name, input_agent, expected_agent) in tests {
                let cmd = CommandInfo {
                    command_path: "test".to_string(),
                    hidden: false,
                    flag_names: Vec::new(),
                };
                let payload =
                    build_event_payload(Some(&cmd), "manual-commit", input_agent, true, "1.0.0");
                assert!(payload.is_some(), "case {name}: expected non-nil payload");

                if let Some(payload) = payload {
                    let agent = payload
                        .properties
                        .get("agent")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string();
                    assert_eq!(
                        agent, expected_agent,
                        "case {name}: agent property mismatch"
                    );
                }
            }
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
        assert_eq!(context.strategy, "manual-commit");
        assert!(context.is_bitloops_enabled);
        assert_eq!(context.agent, "claude-code,codex,gemini,cursor,opencode");
    });
}
