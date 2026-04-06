pub(crate) mod bundle;
pub(crate) mod checkpoint;
pub(crate) mod dashboard;
pub(crate) mod git_blob;
pub(crate) mod repo;
/// Resolve dashboard `repo_id` to checkout path; shared by REST handlers that need `repo_root`.
pub(crate) use repo::resolve_repo_root_from_repo_id;
mod dashboard_graphql;
mod file_diffs;
pub(crate) mod health;
pub(crate) mod meta;
mod params;

pub(crate) use bundle::{handle_api_check_bundle_version, handle_api_fetch_bundle};
pub(crate) use checkpoint::handle_api_checkpoint;
pub(crate) use dashboard::{
    handle_api_agents, handle_api_branches, handle_api_commits, handle_api_kpis,
    handle_api_repositories, handle_api_users,
};
pub(crate) use git_blob::handle_api_git_blob;
pub(crate) use health::handle_api_db_health;
pub(crate) use meta::{handle_api_not_found, handle_api_openapi, handle_api_root};
