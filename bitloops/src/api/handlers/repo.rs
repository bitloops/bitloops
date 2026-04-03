use std::path::PathBuf;

use crate::api::DashboardState;
use crate::api::dto::ApiError;

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
        .map_err(|_| ApiError::not_found(format!("repository not found: {repo_id}")))?;

    repository
        .repo_root()
        .cloned()
        .ok_or_else(|| ApiError::not_found(format!("repository checkout unknown for `{repo_id}`")))
}
