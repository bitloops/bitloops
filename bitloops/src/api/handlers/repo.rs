use std::path::PathBuf;

use anyhow::Error as AnyhowError;

use crate::api::{ApiError, DashboardState};

pub(crate) fn map_resolve_repository_error(repo_id: &str, err: AnyhowError) -> ApiError {
    let root = err
        .chain()
        .last()
        .map(|cause| cause.to_string())
        .unwrap_or_else(|| err.to_string());
    let lower = root.to_ascii_lowercase();
    if lower.contains("ambiguous") {
        return ApiError::bad_request(root);
    }
    if lower.contains("unknown repository") {
        return ApiError::not_found(format!("repository not found: {repo_id}"));
    }
    ApiError::internal(format!("repository resolution failed: {err:#}"))
}

pub(crate) async fn resolve_repo_root_from_repo_id(
    state: &DashboardState,
    repo_id: &str,
) -> Result<PathBuf, ApiError> {
    let repo_id = repo_id.trim();
    if repo_id.is_empty() {
        return Err(ApiError::bad_request("repo_id is required"));
    }

    let context = crate::graphql::DevqlGraphqlContext::for_global_request(
        state.config_root.clone(),
        state.repo_root.clone(),
        state.repo_registry_path().map(std::path::Path::to_path_buf),
        state.db.clone(),
    );
    let repository = context
        .resolve_repository_selection(repo_id)
        .await
        .map_err(|err| map_resolve_repository_error(repo_id, err))?;

    repository.repo_root().cloned().ok_or_else(|| {
        log::error!("repository checkout unknown for `{repo_id}`");
        ApiError::not_found(format!("repository checkout unknown for `{repo_id}`"))
    })
}
