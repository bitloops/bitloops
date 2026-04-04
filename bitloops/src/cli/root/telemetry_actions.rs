use serde_json::Value;
use std::collections::HashMap;

use super::args::{CleanArgs, DisableArgs, DoctorArgs, HelpArgs, ResetArgs, ResumeArgs};

fn new_action(
    event: &str,
    properties: HashMap<String, Value>,
) -> crate::telemetry::analytics::ActionDescriptor {
    crate::telemetry::analytics::ActionDescriptor {
        event: event.to_string(),
        surface: "cli",
        properties,
    }
}

fn insert_flags(props: &mut HashMap<String, Value>, flags: Vec<&'static str>) {
    if flags.is_empty() {
        return;
    }

    props.insert(
        "flags".to_string(),
        Value::Array(
            flags
                .into_iter()
                .map(|flag| Value::String(flag.to_string()))
                .collect(),
        ),
    );
}

fn insert_bool_property(props: &mut HashMap<String, Value>, key: &str, value: bool) {
    props.insert(key.to_string(), Value::Bool(value));
}

fn insert_count_property(props: &mut HashMap<String, Value>, key: &str, value: usize) {
    props.insert(
        key.to_string(),
        Value::Number(serde_json::Number::from(
            u64::try_from(value).unwrap_or(u64::MAX),
        )),
    );
}

fn insert_optional_count_property(
    props: &mut HashMap<String, Value>,
    key: &str,
    value: Option<usize>,
) {
    if let Some(value) = value {
        insert_count_property(props, key, value);
    }
}

fn insert_string_property(props: &mut HashMap<String, Value>, key: &str, value: &str) {
    props.insert(key.to_string(), Value::String(value.to_string()));
}

fn stage_sequence_from_devql_query(query: &str) -> Vec<String> {
    query
        .split("->")
        .map(str::trim)
        .filter(|stage| !stage.is_empty())
        .filter_map(|stage| {
            let name = stage
                .split_once('(')
                .map(|(prefix, _)| prefix)
                .unwrap_or(stage)
                .trim();
            (!name.is_empty()).then(|| name.to_string())
        })
        .collect()
}

pub(crate) fn telemetry_action_for_version(
    check: bool,
) -> crate::telemetry::analytics::ActionDescriptor {
    let mut props = HashMap::new();
    let mut flags = Vec::new();
    if check {
        flags.push("check");
    }
    insert_flags(&mut props, flags);
    new_action("bitloops version", props)
}

pub(crate) fn telemetry_action_for_connection_status()
-> crate::telemetry::analytics::ActionDescriptor {
    new_action("bitloops connection status", HashMap::new())
}

pub(crate) fn telemetry_action_for_command(
    command: &crate::cli::Commands,
) -> Option<crate::telemetry::analytics::ActionDescriptor> {
    match command {
        crate::cli::Commands::Daemon(args) => match args.command.as_ref()? {
            crate::cli::daemon::DaemonCommand::Start(args) => Some(daemon_start_action(args)),
            crate::cli::daemon::DaemonCommand::Stop(args) => Some(daemon_stop_action(args)),
            crate::cli::daemon::DaemonCommand::Status(args) => Some(daemon_status_action(args)),
            crate::cli::daemon::DaemonCommand::Restart(args) => Some(daemon_restart_action(args)),
            crate::cli::daemon::DaemonCommand::Enrichments(args) => daemon_enrichments_action(args),
            crate::cli::daemon::DaemonCommand::Logs(args) => Some(daemon_logs_action(args)),
        },
        crate::cli::Commands::Start(args) => Some(daemon_start_action(args)),
        crate::cli::Commands::Stop(args) => Some(daemon_stop_action(args)),
        crate::cli::Commands::Status(args) => Some(daemon_status_action(args)),
        crate::cli::Commands::Restart(args) => Some(daemon_restart_action(args)),
        crate::cli::Commands::Checkpoints(args) => checkpoints_action(args),
        crate::cli::Commands::Rewind(args) => Some(rewind_action(args)),
        crate::cli::Commands::Resume(args) => Some(resume_action(args)),
        crate::cli::Commands::Clean(args) => Some(clean_action(args)),
        crate::cli::Commands::Reset(args) => Some(reset_action(args)),
        crate::cli::Commands::Init(args) => Some(init_action(args)),
        crate::cli::Commands::Enable(args) => Some(enable_action(args)),
        crate::cli::Commands::Disable(args) => Some(disable_action(args)),
        crate::cli::Commands::Uninstall(args) => Some(uninstall_action(args)),
        crate::cli::Commands::Dashboard(_) => {
            Some(new_action("bitloops dashboard", HashMap::new()))
        }
        crate::cli::Commands::Hooks(_) => None,
        crate::cli::Commands::Version(args) => Some(telemetry_action_for_version(args.check)),
        crate::cli::Commands::Explain(args) => Some(explain_action(args)),
        crate::cli::Commands::Debug(_) => None,
        crate::cli::Commands::Devql(args) => devql_action(args),
        crate::cli::Commands::Testlens(args) => testlens_action(args),
        crate::cli::Commands::Embeddings(args) => embeddings_action(args),
        crate::cli::Commands::EmbeddingsRuntime(_) => None,
        crate::cli::Commands::DevqlWatcher(_) => None,
        crate::cli::Commands::DaemonProcess(_) => None,
        crate::cli::Commands::DaemonSupervisor(_) => None,
        crate::cli::Commands::Doctor(args) => Some(doctor_action(args)),
        crate::cli::Commands::SendAnalytics(_) => None,
        crate::cli::Commands::Completion(_) => None,
        crate::cli::Commands::CurlBashPostInstall => None,
        crate::cli::Commands::Help(args) => Some(help_action(args)),
    }
}

pub(crate) fn should_attempt_watcher_autostart(command: &crate::cli::Commands) -> bool {
    matches!(
        command,
        crate::cli::Commands::Devql(_) | crate::cli::Commands::Testlens(_)
    )
}

fn daemon_start_action(
    args: &crate::cli::daemon::DaemonStartArgs,
) -> crate::telemetry::analytics::ActionDescriptor {
    let mut props = HashMap::new();
    let mut flags = Vec::new();
    if args.create_default_config {
        flags.push("create_default_config");
    }
    if args.detached {
        flags.push("detached");
    }
    if args.until_stopped {
        flags.push("until_stopped");
    }
    if args.http {
        flags.push("http");
    }
    if args.recheck_local_dashboard_net {
        flags.push("recheck_local_dashboard_net");
    }
    if args.telemetry.is_some() {
        flags.push("telemetry");
    }
    if args.no_telemetry {
        flags.push("no_telemetry");
    }
    insert_flags(&mut props, flags);
    insert_bool_property(&mut props, "has_config", args.config.is_some());
    insert_bool_property(&mut props, "has_host", args.host.is_some());
    insert_bool_property(&mut props, "has_bundle_dir", args.bundle_dir.is_some());
    new_action("bitloops daemon start", props)
}

fn daemon_stop_action(
    args: &crate::cli::daemon::DaemonStopArgs,
) -> crate::telemetry::analytics::ActionDescriptor {
    let mut props = HashMap::new();
    insert_bool_property(&mut props, "has_config", args.config.is_some());
    new_action("bitloops daemon stop", props)
}

fn daemon_status_action(
    args: &crate::cli::daemon::DaemonStatusArgs,
) -> crate::telemetry::analytics::ActionDescriptor {
    let mut props = HashMap::new();
    insert_bool_property(&mut props, "has_config", args.config.is_some());
    new_action("bitloops daemon status", props)
}

fn daemon_restart_action(
    args: &crate::cli::daemon::DaemonRestartArgs,
) -> crate::telemetry::analytics::ActionDescriptor {
    let mut props = HashMap::new();
    insert_bool_property(&mut props, "has_config", args.config.is_some());
    new_action("bitloops daemon restart", props)
}

fn daemon_logs_action(
    args: &crate::cli::daemon::DaemonLogsArgs,
) -> crate::telemetry::analytics::ActionDescriptor {
    let mut props = HashMap::new();
    let mut flags = Vec::new();
    if args.follow {
        flags.push("follow");
    }
    if args.path {
        flags.push("path");
    }
    insert_flags(&mut props, flags);
    insert_optional_count_property(&mut props, "tail_lines", args.tail);
    new_action("bitloops daemon logs", props)
}

fn daemon_enrichments_action(
    args: &crate::cli::daemon::EnrichmentArgs,
) -> Option<crate::telemetry::analytics::ActionDescriptor> {
    match args.command.as_ref()? {
        crate::cli::daemon::EnrichmentCommand::Status(_) => Some(new_action(
            "bitloops daemon enrichments status",
            HashMap::new(),
        )),
        crate::cli::daemon::EnrichmentCommand::Pause(args) => {
            let mut props = HashMap::new();
            insert_bool_property(&mut props, "has_reason", args.reason.is_some());
            Some(new_action("bitloops daemon enrichments pause", props))
        }
        crate::cli::daemon::EnrichmentCommand::Resume(_) => Some(new_action(
            "bitloops daemon enrichments resume",
            HashMap::new(),
        )),
        crate::cli::daemon::EnrichmentCommand::RetryFailed(_) => Some(new_action(
            "bitloops daemon enrichments retry-failed",
            HashMap::new(),
        )),
    }
}

fn checkpoints_action(
    args: &crate::cli::checkpoints::CheckpointsArgs,
) -> Option<crate::telemetry::analytics::ActionDescriptor> {
    match args.command.as_ref()? {
        crate::cli::checkpoints::CheckpointsCommand::Status(args) => {
            let mut props = HashMap::new();
            let mut flags = Vec::new();
            if args.detailed {
                flags.push("detailed");
            }
            insert_flags(&mut props, flags);
            Some(new_action("bitloops checkpoints status", props))
        }
    }
}

fn rewind_action(
    args: &crate::cli::rewind::RewindArgs,
) -> crate::telemetry::analytics::ActionDescriptor {
    let mut props = HashMap::new();
    let mut flags = Vec::new();
    if args.list {
        flags.push("list");
    }
    if args.logs_only {
        flags.push("logs_only");
    }
    if args.reset {
        flags.push("reset");
    }
    insert_flags(&mut props, flags);
    insert_bool_property(&mut props, "has_target", args.to.is_some());
    new_action("bitloops rewind", props)
}

fn resume_action(args: &ResumeArgs) -> crate::telemetry::analytics::ActionDescriptor {
    let mut props = HashMap::new();
    let mut flags = Vec::new();
    if args.force {
        flags.push("force");
    }
    insert_flags(&mut props, flags);
    new_action("bitloops resume", props)
}

fn clean_action(args: &CleanArgs) -> crate::telemetry::analytics::ActionDescriptor {
    let mut props = HashMap::new();
    let mut flags = Vec::new();
    if args.force {
        flags.push("force");
    }
    insert_flags(&mut props, flags);
    new_action("bitloops clean", props)
}

fn reset_action(args: &ResetArgs) -> crate::telemetry::analytics::ActionDescriptor {
    let mut props = HashMap::new();
    let mut flags = Vec::new();
    if args.force {
        flags.push("force");
    }
    insert_flags(&mut props, flags);
    insert_bool_property(&mut props, "has_session", args.session.is_some());
    new_action("bitloops reset", props)
}

fn init_action(args: &crate::cli::init::InitArgs) -> crate::telemetry::analytics::ActionDescriptor {
    let mut props = HashMap::new();
    let mut flags = Vec::new();
    if args.install_default_daemon {
        flags.push("install_default_daemon");
    }
    if args.force {
        flags.push("force");
    }
    if args.telemetry.is_some() {
        flags.push("telemetry");
    }
    if args.no_telemetry {
        flags.push("no_telemetry");
    }
    if args.skip_baseline {
        flags.push("skip_baseline");
    }
    insert_flags(&mut props, flags);
    insert_bool_property(&mut props, "has_agent", args.agent.is_some());
    insert_bool_property(&mut props, "has_sync_choice", args.sync.is_some());
    new_action("bitloops init", props)
}

fn enable_action(
    args: &crate::cli::enable::EnableArgs,
) -> crate::telemetry::analytics::ActionDescriptor {
    let mut props = HashMap::new();
    let mut flags = Vec::new();
    if args.local {
        flags.push("local");
    }
    if args.project {
        flags.push("project");
    }
    if args.force {
        flags.push("force");
    }
    if args.telemetry.is_some() {
        flags.push("telemetry");
    }
    if args.no_telemetry {
        flags.push("no_telemetry");
    }
    insert_flags(&mut props, flags);
    insert_bool_property(&mut props, "has_agent", args.agent.is_some());
    new_action("bitloops enable", props)
}

fn disable_action(args: &DisableArgs) -> crate::telemetry::analytics::ActionDescriptor {
    let mut props = HashMap::new();
    let mut flags = Vec::new();
    if args.project {
        flags.push("project");
    }
    insert_flags(&mut props, flags);
    new_action("bitloops disable", props)
}

fn uninstall_action(
    args: &crate::cli::uninstall::UninstallArgs,
) -> crate::telemetry::analytics::ActionDescriptor {
    let mut props = HashMap::new();
    let mut flags = Vec::new();
    if args.full {
        flags.push("full");
    }
    if args.binaries {
        flags.push("binaries");
    }
    if args.service {
        flags.push("service");
    }
    if args.data {
        flags.push("data");
    }
    if args.caching {
        flags.push("caching");
    }
    if args.config {
        flags.push("config");
    }
    if args.agent_hooks {
        flags.push("agent_hooks");
    }
    if args.git_hooks {
        flags.push("git_hooks");
    }
    if args.shell {
        flags.push("shell");
    }
    if args.only_current_project {
        flags.push("only_current_project");
    }
    if args.force {
        flags.push("force");
    }
    insert_flags(&mut props, flags);
    new_action("bitloops uninstall", props)
}

fn explain_action(
    args: &crate::cli::explain::ExplainArgs,
) -> crate::telemetry::analytics::ActionDescriptor {
    let mut props = HashMap::new();
    let mut flags = Vec::new();
    if args.no_pager {
        flags.push("no_pager");
    }
    if args.short {
        flags.push("short");
    }
    if args.full {
        flags.push("full");
    }
    if args.raw_transcript {
        flags.push("raw_transcript");
    }
    if args.generate {
        flags.push("generate");
    }
    if args.force {
        flags.push("force");
    }
    if args.search_all {
        flags.push("search_all");
    }
    insert_flags(&mut props, flags);
    insert_bool_property(&mut props, "has_session", args.session.is_some());
    insert_bool_property(&mut props, "has_commit", args.commit.is_some());
    insert_bool_property(&mut props, "has_checkpoint", args.checkpoint.is_some());
    new_action("bitloops explain", props)
}

fn doctor_action(args: &DoctorArgs) -> crate::telemetry::analytics::ActionDescriptor {
    let mut props = HashMap::new();
    let mut flags = Vec::new();
    if args.force {
        flags.push("force");
    }
    insert_flags(&mut props, flags);
    new_action("bitloops doctor", props)
}

fn help_action(args: &HelpArgs) -> crate::telemetry::analytics::ActionDescriptor {
    let mut props = HashMap::new();
    let mut flags = Vec::new();
    if args.tree {
        flags.push("tree");
    }
    insert_flags(&mut props, flags);
    insert_bool_property(&mut props, "has_command_target", !args.command.is_empty());
    new_action("bitloops help", props)
}

fn devql_action(
    args: &crate::cli::devql::DevqlArgs,
) -> Option<crate::telemetry::analytics::ActionDescriptor> {
    match args.command.as_ref()? {
        crate::cli::devql::DevqlCommand::Init(_) => {
            Some(new_action("bitloops devql init", HashMap::new()))
        }
        crate::cli::devql::DevqlCommand::Ingest(args) => {
            let mut props = HashMap::new();
            insert_count_property(&mut props, "max_checkpoints", args.max_checkpoints);
            Some(new_action("bitloops devql ingest", props))
        }
        crate::cli::devql::DevqlCommand::Sync(args) => {
            let mut props = HashMap::new();
            let mut flags = Vec::new();
            if args.status {
                flags.push("status");
            }
            insert_flags(&mut props, flags);
            let sync_mode = if args.full {
                "full"
            } else if args.paths.is_some() {
                "paths"
            } else if args.repair {
                "repair"
            } else if args.validate {
                "validate"
            } else {
                "incremental"
            };
            insert_string_property(&mut props, "sync_mode", sync_mode);
            insert_bool_property(&mut props, "status_follow", args.status);
            insert_optional_count_property(
                &mut props,
                "paths_count",
                args.paths.as_ref().map(Vec::len),
            );
            Some(new_action("bitloops devql sync", props))
        }
        crate::cli::devql::DevqlCommand::Projection(args) => match &args.command {
            crate::cli::devql::DevqlProjectionCommand::CheckpointFileSnapshots(args) => {
                let mut props = HashMap::new();
                let mut flags = Vec::new();
                if args.dry_run {
                    flags.push("dry_run");
                }
                insert_flags(&mut props, flags);
                insert_count_property(&mut props, "batch_size", args.batch_size);
                insert_optional_count_property(&mut props, "max_checkpoints", args.max_checkpoints);
                insert_bool_property(&mut props, "has_resume_after", args.resume_after.is_some());
                Some(new_action(
                    "bitloops devql projection checkpoint-file-snapshots",
                    props,
                ))
            }
        },
        crate::cli::devql::DevqlCommand::Query(args) => {
            let mut props = HashMap::new();
            let mut flags = Vec::new();
            if args.graphql {
                flags.push("graphql");
            }
            if args.compact {
                flags.push("compact");
            }
            insert_flags(&mut props, flags);
            let query_mode = if crate::host::devql::use_raw_graphql_mode(&args.query, args.graphql)
            {
                "raw_graphql"
            } else {
                "dsl"
            };
            insert_string_property(&mut props, "query_mode", query_mode);
            insert_string_property(
                &mut props,
                "output_mode",
                if args.compact { "compact" } else { "text" },
            );
            if query_mode == "dsl" {
                let stage_sequence = stage_sequence_from_devql_query(&args.query);
                insert_count_property(&mut props, "stage_count", stage_sequence.len());
                props.insert(
                    "stage_sequence".to_string(),
                    Value::Array(stage_sequence.into_iter().map(Value::String).collect()),
                );
            }
            Some(new_action("bitloops devql query", props))
        }
        crate::cli::devql::DevqlCommand::ConnectionStatus(_) => Some(new_action(
            "bitloops devql connection-status",
            HashMap::new(),
        )),
        crate::cli::devql::DevqlCommand::Packs(args) => {
            let mut props = HashMap::new();
            let mut flags = Vec::new();
            if args.with_health {
                flags.push("with_health");
            }
            if args.apply_migrations {
                flags.push("apply_migrations");
            }
            if args.with_extensions {
                flags.push("with_extensions");
            }
            insert_flags(&mut props, flags);
            insert_string_property(
                &mut props,
                "output_mode",
                if args.json { "json" } else { "text" },
            );
            Some(new_action("bitloops devql packs", props))
        }
        crate::cli::devql::DevqlCommand::Knowledge(args) => match &args.command {
            crate::cli::devql::DevqlKnowledgeCommand::Add(args) => {
                let mut props = HashMap::new();
                insert_bool_property(&mut props, "has_url", true);
                insert_bool_property(&mut props, "has_commit", args.commit.is_some());
                Some(new_action("bitloops devql knowledge add", props))
            }
            crate::cli::devql::DevqlKnowledgeCommand::Associate(args) => {
                let mut props = HashMap::new();
                insert_bool_property(&mut props, "has_source_ref", !args.source_ref.is_empty());
                insert_bool_property(&mut props, "has_target_ref", !args.target_ref.is_empty());
                Some(new_action("bitloops devql knowledge associate", props))
            }
            crate::cli::devql::DevqlKnowledgeCommand::Refresh(_) => Some(new_action(
                "bitloops devql knowledge refresh",
                HashMap::new(),
            )),
            crate::cli::devql::DevqlKnowledgeCommand::Versions(_) => Some(new_action(
                "bitloops devql knowledge versions",
                HashMap::new(),
            )),
        },
    }
}

fn testlens_action(
    args: &crate::cli::testlens::TestLensArgs,
) -> Option<crate::telemetry::analytics::ActionDescriptor> {
    match args.command.as_ref()? {
        crate::cli::testlens::TestLensCommand::Init(_) => {
            Some(new_action("bitloops testlens init", HashMap::new()))
        }
        crate::cli::testlens::TestLensCommand::IngestTests(_) => {
            Some(new_action("bitloops testlens ingest-tests", HashMap::new()))
        }
        crate::cli::testlens::TestLensCommand::IngestCoverage(args) => {
            let mut props = HashMap::new();
            insert_bool_property(&mut props, "has_lcov", args.lcov.is_some());
            insert_bool_property(&mut props, "has_input", args.input.is_some());
            insert_bool_property(
                &mut props,
                "has_test_artefact_id",
                args.test_artefact_id.is_some(),
            );
            insert_bool_property(&mut props, "has_format", args.format.is_some());
            Some(new_action("bitloops testlens ingest-coverage", props))
        }
        crate::cli::testlens::TestLensCommand::IngestCoverageBatch(_) => Some(new_action(
            "bitloops testlens ingest-coverage-batch",
            HashMap::new(),
        )),
        crate::cli::testlens::TestLensCommand::IngestResults(_) => Some(new_action(
            "bitloops testlens ingest-results",
            HashMap::new(),
        )),
    }
}

fn embeddings_action(
    args: &crate::cli::embeddings::EmbeddingsArgs,
) -> Option<crate::telemetry::analytics::ActionDescriptor> {
    match args.command.as_ref()? {
        crate::cli::embeddings::EmbeddingsCommand::Pull(_) => {
            Some(new_action("bitloops embeddings pull", HashMap::new()))
        }
        crate::cli::embeddings::EmbeddingsCommand::Doctor(args) => {
            let mut props = HashMap::new();
            insert_bool_property(&mut props, "has_profile", args.profile.is_some());
            Some(new_action("bitloops embeddings doctor", props))
        }
        crate::cli::embeddings::EmbeddingsCommand::ClearCache(_) => Some(new_action(
            "bitloops embeddings clear-cache",
            HashMap::new(),
        )),
    }
}
