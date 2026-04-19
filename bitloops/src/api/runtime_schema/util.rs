use crate::daemon::SummaryBootstrapAction;

pub(crate) fn to_graphql_i32(value: impl TryInto<i32>) -> i32 {
    value.try_into().unwrap_or(i32::MAX)
}

pub(crate) fn to_graphql_i64(value: impl TryInto<i64>) -> i64 {
    value.try_into().unwrap_or(i64::MAX)
}

pub(crate) fn current_unix_timestamp() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

pub(crate) fn summary_bootstrap_action_name(action: SummaryBootstrapAction) -> &'static str {
    match action {
        SummaryBootstrapAction::InstallRuntimeOnly => "install_runtime_only",
        SummaryBootstrapAction::InstallRuntimeOnlyPendingProbe => {
            "install_runtime_only_pending_probe"
        }
        SummaryBootstrapAction::ConfigureLocal => "configure_local",
        SummaryBootstrapAction::ConfigureCloud => "configure_cloud",
    }
}
