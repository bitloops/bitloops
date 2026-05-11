use async_graphql::{Result as GraphqlResult, SimpleObject};

use super::config::{map_runtime_api_error, resolve_runtime_devql_config};
use super::roots::RuntimeRequestContext;
use crate::api::DashboardState;
use crate::graphql::graphql_error;

#[derive(Debug, Clone, SimpleObject)]
pub(crate) struct RuntimeWatcherReconcileResultObject {
    #[graphql(name = "repoId")]
    pub repo_id: String,
    #[graphql(name = "repoRoot")]
    pub repo_root: String,
    #[graphql(name = "watcherEnabled")]
    pub watcher_enabled: bool,
    pub action: String,
}

pub(crate) async fn reconcile_runtime_watcher(
    state: &DashboardState,
    request_context: RuntimeRequestContext,
    repo_id: &str,
) -> GraphqlResult<RuntimeWatcherReconcileResultObject> {
    let cfg = resolve_runtime_devql_config(state, &request_context, repo_id)
        .await
        .map_err(map_runtime_api_error)?;
    let daemon_config = crate::daemon::resolve_daemon_config(Some(state.config_path.as_path()))
        .map_err(|err| {
            graphql_error(
                "internal",
                format!("failed to resolve daemon config for watcher reconciliation: {err:#}"),
            )
        })?;
    let result =
        crate::daemon::reconcile_bound_repo_watcher_explicit(&cfg.repo_root, &daemon_config)
            .map_err(|err| {
                graphql_error(
                    "internal",
                    format!("failed to reconcile DevQL watcher: {err:#}"),
                )
            })?;

    Ok(RuntimeWatcherReconcileResultObject {
        repo_id: cfg.repo.repo_id,
        repo_root: result.repo_root.display().to_string(),
        watcher_enabled: result.watcher_enabled,
        action: result.action.as_str().to_string(),
    })
}
