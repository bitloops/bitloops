use std::path::{Path, PathBuf};

use super::roots::RuntimeRequestContext;
use crate::api::handlers::resolve_repo_root_from_repo_id;
use crate::api::{ApiError, DashboardState};
use crate::graphql::{bad_user_input_error, graphql_error};

pub(crate) async fn resolve_runtime_devql_config(
    state: &DashboardState,
    request_context: &RuntimeRequestContext,
    repo_id: &str,
) -> std::result::Result<crate::host::devql::DevqlConfig, ApiError> {
    let repo_id = repo_id.trim();
    if repo_id.is_empty() {
        return Err(ApiError::bad_request("repo_id is required"));
    }

    if let Some(bound_repo_root) = request_context.bound_repo_root.as_deref() {
        let (bound_repo_root, bound_repo) = resolve_runtime_repo_identity(bound_repo_root)?;
        if bound_repo.repo_id != repo_id {
            return Err(ApiError::bad_request(format!(
                "runtime request repoId `{repo_id}` does not match bound repository `{}`",
                bound_repo.repo_id
            )));
        }
        return build_runtime_devql_config(state, bound_repo_root, bound_repo);
    }

    if let Ok((state_repo_root, state_repo)) = resolve_runtime_repo_identity(&state.repo_root)
        && state_repo.repo_id == repo_id
    {
        return build_runtime_devql_config(state, state_repo_root, state_repo);
    }

    let repo_root = resolve_repo_root_from_repo_id(state, repo_id).await?;
    let (repo_root, repo) = resolve_runtime_repo_identity(&repo_root)?;
    build_runtime_devql_config(state, repo_root, repo)
}

fn resolve_runtime_repo_identity(
    repo_root: &Path,
) -> std::result::Result<(PathBuf, crate::host::devql::RepoIdentity), ApiError> {
    let repo_root = repo_root
        .canonicalize()
        .unwrap_or_else(|_| repo_root.to_path_buf());
    let repo = crate::host::devql::resolve_repo_identity(&repo_root).map_err(|err| {
        ApiError::internal(format!("failed to resolve repository identity: {err:#}"))
    })?;
    Ok((repo_root, repo))
}

fn build_runtime_devql_config(
    state: &DashboardState,
    repo_root: PathBuf,
    repo: crate::host::devql::RepoIdentity,
) -> std::result::Result<crate::host::devql::DevqlConfig, ApiError> {
    crate::host::devql::DevqlConfig::from_roots(state.config_root.clone(), repo_root, repo)
        .map_err(|err| ApiError::internal(format!("failed to resolve runtime config: {err:#}")))
}

pub(crate) fn map_runtime_api_error(error: ApiError) -> async_graphql::Error {
    match error.code {
        "bad_request" | "not_found" => bad_user_input_error(error.message),
        other => graphql_error(other, error.message),
    }
}
