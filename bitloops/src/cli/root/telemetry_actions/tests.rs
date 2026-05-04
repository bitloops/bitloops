use super::*;
use clap::Parser;

#[test]
fn stage_sequence_from_devql_query_splits_arrow_stages_and_strips_calls() {
    assert_eq!(
        stage_sequence_from_devql_query("a -> b( x ) -> c"),
        vec!["a", "b", "c"]
    );
}

#[test]
fn stage_sequence_from_devql_query_skips_empty_segments() {
    assert_eq!(
        stage_sequence_from_devql_query("  foo  ->   -> bar "),
        vec!["foo", "bar"]
    );
}

#[test]
fn telemetry_action_for_version_includes_check_flag() {
    let with_check = telemetry_action_for_version(true);
    assert_eq!(with_check.event, "bitloops version");
    let flags = with_check
        .properties
        .get("flags")
        .and_then(|v| v.as_array())
        .expect("flags array");
    assert_eq!(flags.len(), 1);
    assert_eq!(flags[0].as_str(), Some("check"));

    let plain = telemetry_action_for_version(false);
    assert!(!plain.properties.contains_key("flags"));
}

#[test]
fn telemetry_action_for_init_with_repeated_agents_sets_has_agent() {
    let cli = crate::cli::Cli::try_parse_from([
        "bitloops",
        "init",
        "--agent",
        "cursor",
        "--agent",
        "codex",
        "--sync=false",
        "--ingest=false",
    ])
    .expect("init command should parse");
    let action = telemetry_action_for_command(
        cli.command
            .as_ref()
            .expect("init command should produce a subcommand"),
    )
    .expect("init telemetry action should be emitted");

    assert_eq!(action.event, "bitloops init");
    assert_eq!(
        action.properties.get("has_agent").and_then(Value::as_bool),
        Some(true)
    );
}

#[test]
fn telemetry_action_for_devql_tasks_enqueue_ingest_has_no_legacy_checkpoint_limit_property() {
    let cli = crate::cli::Cli::try_parse_from([
        "bitloops", "devql", "tasks", "enqueue", "--kind", "ingest",
    ])
    .expect("devql task enqueue should parse");
    let action = telemetry_action_for_command(
        cli.command
            .as_ref()
            .expect("devql task enqueue should produce a subcommand"),
    )
    .expect("devql task enqueue telemetry action should be emitted");

    assert_eq!(action.event, "bitloops devql tasks enqueue");
    assert_eq!(
        action.properties.get("task_kind").and_then(Value::as_str),
        Some("ingest")
    );
    assert!(
        !action.properties.contains_key("max_checkpoints"),
        "devql task enqueue should not emit the removed max_checkpoints property"
    );
}

#[test]
fn telemetry_action_for_daemon_logs_records_level_filters() {
    let cli = crate::cli::Cli::try_parse_from([
        "bitloops", "daemon", "logs", "--level", "warning", "--level", "ERROR", "--follow",
    ])
    .expect("daemon logs should parse");
    let action = telemetry_action_for_command(
        cli.command
            .as_ref()
            .expect("daemon logs should produce a subcommand"),
    )
    .expect("daemon logs telemetry action should be emitted");

    assert_eq!(action.event, "bitloops daemon logs");
    assert_eq!(
        action
            .properties
            .get("flags")
            .and_then(Value::as_array)
            .map(|flags| { flags.iter().filter_map(Value::as_str).collect::<Vec<_>>() }),
        Some(vec!["follow"])
    );
    assert_eq!(
        action
            .properties
            .get("levels")
            .and_then(Value::as_array)
            .map(|levels| { levels.iter().filter_map(Value::as_str).collect::<Vec<_>>() }),
        Some(vec!["warn", "error"])
    );
}
