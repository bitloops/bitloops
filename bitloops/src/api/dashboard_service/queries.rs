use std::collections::HashMap;

use crate::api::dashboard_file_diffs::{
    dashboard_file_diff_list_from_numstat, dashboard_zeroed_file_diff_list,
};
use crate::api::dashboard_params::{
    parse_commit_checkpoint_filter, parse_optional_unix_seconds, validate_time_window,
};
use crate::api::dashboard_types::{
    DashboardAgent, DashboardBranchSummary, DashboardCommitRow, DashboardKpis, DashboardRepository,
    DashboardUser,
};
use crate::api::{ApiError, CommitCheckpointQuery, DashboardState, dashboard_user, paginate};
use crate::graphql::HealthStatus;

use super::graphql::{
    build_dashboard_kpis_from_graphql_rows, checkpoint_agents, dashboard_commit_row_from_graphql,
    load_dashboard_branches_via_graphql, load_dashboard_commit_rows_via_graphql,
};
use super::repository::{
    dashboard_graphql_context, resolve_dashboard_repo_root, resolve_dashboard_repo_selector,
};

pub(in crate::api) async fn load_dashboard_health(state: &DashboardState) -> HealthStatus {
    dashboard_graphql_context(state).health_status().await
}

pub(in crate::api) async fn load_dashboard_repositories(
    state: &DashboardState,
) -> std::result::Result<Vec<DashboardRepository>, ApiError> {
    let repositories = dashboard_graphql_context(state)
        .list_known_repositories()
        .await
        .map_err(|err| {
            ApiError::internal(format!("failed to load dashboard repositories: {err:#}"))
        })?;

    Ok(repositories
        .into_iter()
        .map(|repository| DashboardRepository {
            repo_id: repository.repo_id().to_string(),
            identity: repository.identity().to_string(),
            name: repository.name().to_string(),
            provider: repository.provider().to_string(),
            organization: repository.organization().to_string(),
            default_branch: repository.default_branch().map(str::to_string),
        })
        .collect())
}

pub(in crate::api) async fn load_dashboard_kpis(
    state: &DashboardState,
    repo_id: Option<String>,
    filter: CommitCheckpointQuery,
) -> std::result::Result<DashboardKpis, ApiError> {
    let repo_selector = resolve_dashboard_repo_selector(state, repo_id.as_deref()).await?;
    let rows =
        load_dashboard_commit_rows_via_graphql(state, repo_selector.as_str(), &filter).await?;
    Ok(build_dashboard_kpis_from_graphql_rows(&rows))
}

pub(in crate::api) async fn load_dashboard_commits(
    state: &DashboardState,
    repo_id: Option<String>,
    filter: CommitCheckpointQuery,
) -> std::result::Result<Vec<DashboardCommitRow>, ApiError> {
    let repo_selector = resolve_dashboard_repo_selector(state, repo_id.as_deref()).await?;
    let repo_root = resolve_dashboard_repo_root(state, repo_id.as_deref()).await?;

    let rows =
        load_dashboard_commit_rows_via_graphql(state, repo_selector.as_str(), &filter).await?;
    let rows = paginate(&rows, filter.page);
    let mut result = Vec::with_capacity(rows.len());
    for row in rows {
        let files_touched = match crate::api::read_commit_numstat(&repo_root, &row.commit.sha) {
            Ok(stats) => dashboard_file_diff_list_from_numstat(stats),
            Err(err) => {
                log::warn!(
                    "dashboard commits query: failed to read numstat for {}: {:#}",
                    row.commit.sha,
                    err
                );
                dashboard_zeroed_file_diff_list(&row.checkpoint.files_touched)
            }
        };
        result.push(dashboard_commit_row_from_graphql(row, files_touched));
    }
    Ok(result)
}

pub(in crate::api) async fn load_dashboard_branches(
    state: &DashboardState,
    repo_id: Option<String>,
    from: Option<String>,
    to: Option<String>,
) -> std::result::Result<Vec<DashboardBranchSummary>, ApiError> {
    let repo_selector = resolve_dashboard_repo_selector(state, repo_id.as_deref()).await?;
    let from_unix = parse_optional_unix_seconds("from", from)?;
    let to_unix = parse_optional_unix_seconds("to", to)?;
    validate_time_window(from_unix, to_unix)?;
    load_dashboard_branches_via_graphql(state, repo_selector.as_str(), from_unix, to_unix).await
}

pub(in crate::api) async fn load_dashboard_users(
    state: &DashboardState,
    repo_id: Option<String>,
    branch: String,
    from: Option<String>,
    to: Option<String>,
    agent: Option<String>,
) -> std::result::Result<Vec<DashboardUser>, ApiError> {
    let repo_selector = resolve_dashboard_repo_selector(state, repo_id.as_deref()).await?;
    let filter = parse_commit_checkpoint_filter(Some(branch), from, to, None, agent)?;
    let rows =
        load_dashboard_commit_rows_via_graphql(state, repo_selector.as_str(), &filter).await?;

    let mut users_by_key: HashMap<String, DashboardUser> = HashMap::new();
    for row in rows {
        let user = dashboard_user(&row.commit.author_name, &row.commit.author_email);
        let key = user.key;
        let name = user.name;
        let email = user.email;

        let entry = users_by_key.entry(key.clone()).or_insert(DashboardUser {
            key,
            name: String::new(),
            email: String::new(),
        });
        if entry.name.is_empty() && !name.is_empty() {
            entry.name = name;
        }
        if entry.email.is_empty() && !email.is_empty() {
            entry.email = email;
        }
    }

    let mut users = users_by_key.into_values().collect::<Vec<_>>();
    users.sort_by(|left, right| {
        left.key
            .cmp(&right.key)
            .then_with(|| left.name.cmp(&right.name))
            .then_with(|| left.email.cmp(&right.email))
    });
    Ok(users)
}

pub(in crate::api) async fn load_dashboard_agents(
    state: &DashboardState,
    repo_id: Option<String>,
    branch: String,
    from: Option<String>,
    to: Option<String>,
    user: Option<String>,
) -> std::result::Result<Vec<DashboardAgent>, ApiError> {
    let repo_selector = resolve_dashboard_repo_selector(state, repo_id.as_deref()).await?;
    let filter = parse_commit_checkpoint_filter(Some(branch), from, to, user, None)?;
    let rows =
        load_dashboard_commit_rows_via_graphql(state, repo_selector.as_str(), &filter).await?;

    let mut agents: Vec<DashboardAgent> = Vec::new();
    for row in rows {
        for key in checkpoint_agents(&row.checkpoint) {
            agents.push(DashboardAgent { key });
        }
    }
    agents.sort_by(|left, right| left.key.cmp(&right.key));
    agents.dedup_by(|left, right| left.key == right.key);
    Ok(agents)
}
