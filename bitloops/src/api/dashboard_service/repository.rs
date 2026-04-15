use std::path::PathBuf;

use crate::api::handlers::{map_resolve_repository_error, resolve_repo_root_from_repo_id};
use crate::api::{ApiError, DashboardState};

pub(super) fn dashboard_graphql_context(
    state: &DashboardState,
) -> crate::graphql::DevqlGraphqlContext {
    crate::graphql::DevqlGraphqlContext::for_global_request(
        state.config_root.clone(),
        state.repo_root.clone(),
        state.repo_registry_path().map(std::path::Path::to_path_buf),
        state.db.clone(),
    )
}

pub(super) async fn resolve_dashboard_repo_selector(
    state: &DashboardState,
    repo_id: Option<&str>,
) -> std::result::Result<String, ApiError> {
    if let Some(repo_id) = repo_id.map(str::trim).filter(|repo_id| !repo_id.is_empty()) {
        let context = dashboard_graphql_context(state);
        let selection = context
            .resolve_repository_selection(repo_id)
            .await
            .map_err(|err| map_resolve_repository_error(repo_id, err))?;
        return Ok(selection.repo_id().to_string());
    }

    crate::host::devql::resolve_repo_identity(&state.repo_root)
        .map(|repo| repo.repo_id)
        .map_err(|err| {
            ApiError::internal(format!(
                "failed to resolve dashboard repository scope: {err:#}"
            ))
        })
}

pub(super) async fn resolve_dashboard_repo_root(
    state: &DashboardState,
    repo_id: Option<&str>,
) -> std::result::Result<PathBuf, ApiError> {
    match repo_id.map(str::trim).filter(|repo_id| !repo_id.is_empty()) {
        Some(repo_id) => resolve_repo_root_from_repo_id(state, repo_id).await,
        None => Ok(state.repo_root.clone()),
    }
}
