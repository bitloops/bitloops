use std::collections::HashMap;
use std::time::Instant;

use axum::{
    Json,
    extract::{Query, State},
    http::StatusCode,
};
use serde_json::Value;

use super::super::dto::{
    ApiAgentDto, ApiAgentsQuery, ApiBranchSummaryDto, ApiBranchesQuery, ApiCommitRowDto,
    ApiCommitsQuery, ApiError, ApiErrorEnvelope, ApiKpisQuery, ApiKpisResponse, ApiRepositoryDto,
    ApiUserDto, ApiUsersQuery,
};
use super::super::{API_DEFAULT_PAGE_LIMIT, ApiPage, DashboardState, read_commit_numstat};
use super::dashboard_graphql::{
    api_commit_row_from_graphql, build_kpis_response_from_graphql_rows, checkpoint_agents,
    load_dashboard_branches_via_graphql, load_dashboard_commit_rows_via_graphql,
    load_dashboard_repositories,
};
use super::file_diffs::{api_file_diff_list_from_numstat, api_zeroed_file_diff_list};
use super::params::{
    parse_commit_checkpoint_filter, parse_optional_unix_seconds, parse_optional_usize,
    validate_time_window,
};

#[utoipa::path(
    get,
    path = "/api/kpis",
    params(ApiKpisQuery),
    responses(
        (status = 200, description = "Aggregated KPI metrics", body = ApiKpisResponse),
        (status = 400, description = "Bad request", body = ApiErrorEnvelope),
        (status = 500, description = "Internal server error", body = ApiErrorEnvelope)
    )
)]
pub(crate) async fn handle_api_kpis(
    State(state): State<DashboardState>,
    Query(query): Query<ApiKpisQuery>,
) -> std::result::Result<Json<ApiKpisResponse>, ApiError> {
    let started = Instant::now();
    let query_properties = kpis_query_properties(&query);
    let result = async {
        let filter = parse_commit_checkpoint_filter(
            query.branch,
            query.from,
            query.to,
            query.user,
            query.agent,
        )?;
        let rows = load_dashboard_commit_rows_via_graphql(&state, &filter).await?;
        Ok(Json(build_kpis_response_from_graphql_rows(&rows)))
    }
    .await;
    track_dashboard_api_action(
        &state,
        "bitloops dashboard api kpis",
        started.elapsed(),
        query_properties,
        &result,
    );
    result
}

#[utoipa::path(
    get,
    path = "/api/commits",
    params(ApiCommitsQuery),
    responses(
        (status = 200, description = "Filtered commit + checkpoint rows", body = [ApiCommitRowDto]),
        (status = 400, description = "Bad request", body = ApiErrorEnvelope),
        (status = 500, description = "Internal server error", body = ApiErrorEnvelope)
    )
)]
pub(crate) async fn handle_api_commits(
    State(state): State<DashboardState>,
    Query(query): Query<ApiCommitsQuery>,
) -> std::result::Result<Json<Vec<ApiCommitRowDto>>, ApiError> {
    let started = Instant::now();
    let query_properties = commits_query_properties(&query);
    let response = async {
        let mut filter = parse_commit_checkpoint_filter(
            query.branch,
            query.from,
            query.to,
            query.user,
            query.agent,
        )?;

        filter.page = ApiPage {
            limit: parse_optional_usize("limit", query.limit)?.unwrap_or(API_DEFAULT_PAGE_LIMIT),
            offset: parse_optional_usize("offset", query.offset)?.unwrap_or(0),
        }
        .normalized();

        let rows = load_dashboard_commit_rows_via_graphql(&state, &filter).await?;
        let rows = super::super::paginate(&rows, filter.page);
        let mut result = Vec::with_capacity(rows.len());
        for row in rows {
            let files_touched = match read_commit_numstat(&state.repo_root, &row.commit.sha) {
                Ok(stats) => api_file_diff_list_from_numstat(stats),
                Err(err) => {
                    log::warn!(
                        "dashboard commits endpoint: failed to read numstat for {}: {:#}",
                        row.commit.sha,
                        err
                    );
                    api_zeroed_file_diff_list(&row.checkpoint.files_touched)
                }
            };
            result.push(api_commit_row_from_graphql(row, files_touched));
        }
        Ok(Json(result))
    }
    .await;
    track_dashboard_api_action(
        &state,
        "bitloops dashboard api commits",
        started.elapsed(),
        query_properties,
        &response,
    );
    response
}

#[utoipa::path(
    get,
    path = "/api/branches",
    params(ApiBranchesQuery),
    responses(
        (status = 200, description = "Branches with at least one checkpoint commit", body = [ApiBranchSummaryDto]),
        (status = 400, description = "Bad request", body = ApiErrorEnvelope),
        (status = 500, description = "Internal server error", body = ApiErrorEnvelope)
    )
)]
pub(crate) async fn handle_api_branches(
    State(state): State<DashboardState>,
    Query(query): Query<ApiBranchesQuery>,
) -> std::result::Result<Json<Vec<ApiBranchSummaryDto>>, ApiError> {
    let started = Instant::now();
    let query_properties = branches_query_properties(&query);
    let response = async {
        let from_unix = parse_optional_unix_seconds("from", query.from)?;
        let to_unix = parse_optional_unix_seconds("to", query.to)?;
        validate_time_window(from_unix, to_unix)?;
        Ok(Json(
            load_dashboard_branches_via_graphql(&state, from_unix, to_unix).await?,
        ))
    }
    .await;
    track_dashboard_api_action(
        &state,
        "bitloops dashboard api branches",
        started.elapsed(),
        query_properties,
        &response,
    );
    response
}

#[utoipa::path(
    get,
    path = "/api/repositories",
    responses(
        (status = 200, description = "Known repositories for the dashboard", body = [ApiRepositoryDto]),
        (status = 500, description = "Internal server error", body = ApiErrorEnvelope)
    )
)]
pub(crate) async fn handle_api_repositories(
    State(state): State<DashboardState>,
) -> std::result::Result<Json<Vec<ApiRepositoryDto>>, ApiError> {
    let started = Instant::now();
    let response = Ok(Json(load_dashboard_repositories(&state).await?));
    track_dashboard_api_action(
        &state,
        "bitloops dashboard api repositories",
        started.elapsed(),
        HashMap::new(),
        &response,
    );
    response
}

#[utoipa::path(
    get,
    path = "/api/users",
    params(ApiUsersQuery),
    responses(
        (status = 200, description = "Users in filtered checkpoint commits", body = [ApiUserDto]),
        (status = 400, description = "Bad request", body = ApiErrorEnvelope),
        (status = 500, description = "Internal server error", body = ApiErrorEnvelope)
    )
)]
pub(crate) async fn handle_api_users(
    State(state): State<DashboardState>,
    Query(query): Query<ApiUsersQuery>,
) -> std::result::Result<Json<Vec<ApiUserDto>>, ApiError> {
    let started = Instant::now();
    let query_properties = users_query_properties(&query);
    let response = async {
        let filter =
            parse_commit_checkpoint_filter(query.branch, query.from, query.to, None, query.agent)?;
        let rows = load_dashboard_commit_rows_via_graphql(&state, &filter).await?;

        let mut users_by_key: HashMap<String, ApiUserDto> = HashMap::new();
        for row in rows {
            let user =
                super::super::dashboard_user(&row.commit.author_name, &row.commit.author_email);
            let key = user.key;
            let name = user.name;
            let email = user.email;

            let entry = users_by_key.entry(key.clone()).or_insert(ApiUserDto {
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

        Ok(Json(users))
    }
    .await;
    track_dashboard_api_action(
        &state,
        "bitloops dashboard api users",
        started.elapsed(),
        query_properties,
        &response,
    );
    response
}

#[utoipa::path(
    get,
    path = "/api/agents",
    params(ApiAgentsQuery),
    responses(
        (status = 200, description = "Agents in filtered checkpoint commits", body = [ApiAgentDto]),
        (status = 400, description = "Bad request", body = ApiErrorEnvelope),
        (status = 500, description = "Internal server error", body = ApiErrorEnvelope)
    )
)]
pub(crate) async fn handle_api_agents(
    State(state): State<DashboardState>,
    Query(query): Query<ApiAgentsQuery>,
) -> std::result::Result<Json<Vec<ApiAgentDto>>, ApiError> {
    let started = Instant::now();
    let query_properties = agents_query_properties(&query);
    let response = async {
        let filter =
            parse_commit_checkpoint_filter(query.branch, query.from, query.to, query.user, None)?;
        let rows = load_dashboard_commit_rows_via_graphql(&state, &filter).await?;

        let mut agents: Vec<ApiAgentDto> = Vec::new();
        for row in rows {
            for key in checkpoint_agents(&row.checkpoint) {
                agents.push(ApiAgentDto { key });
            }
        }
        agents.sort_by(|left, right| left.key.cmp(&right.key));
        agents.dedup_by(|left, right| left.key == right.key);

        Ok(Json(agents))
    }
    .await;
    track_dashboard_api_action(
        &state,
        "bitloops dashboard api agents",
        started.elapsed(),
        query_properties,
        &response,
    );
    response
}

fn track_dashboard_api_action<T>(
    state: &DashboardState,
    event: &str,
    duration: std::time::Duration,
    mut properties: HashMap<String, Value>,
    result: &std::result::Result<Json<T>, ApiError>,
) {
    let status = match result {
        Ok(_) => StatusCode::OK,
        Err(err) => err.status_code(),
    };
    properties.insert("http_method".to_string(), Value::String("GET".to_string()));
    properties.insert(
        "status_code_class".to_string(),
        Value::String(super::super::status_code_class(status).to_string()),
    );

    super::super::track_repo_action(
        &state.repo_root,
        crate::telemetry::analytics::ActionDescriptor {
            event: event.to_string(),
            surface: "dashboard",
            properties,
        },
        status.is_success(),
        duration,
    );
}

fn insert_presence(properties: &mut HashMap<String, Value>, key: &str, present: bool) {
    properties.insert(key.to_string(), Value::Bool(present));
}

fn kpis_query_properties(query: &ApiKpisQuery) -> HashMap<String, Value> {
    let mut properties = HashMap::new();
    insert_presence(&mut properties, "has_branch_filter", query.branch.is_some());
    insert_presence(&mut properties, "has_user_filter", query.user.is_some());
    insert_presence(&mut properties, "has_agent_filter", query.agent.is_some());
    insert_presence(&mut properties, "has_from", query.from.is_some());
    insert_presence(&mut properties, "has_to", query.to.is_some());
    properties
}

fn commits_query_properties(query: &ApiCommitsQuery) -> HashMap<String, Value> {
    let mut properties = kpis_query_properties(&ApiKpisQuery {
        branch: query.branch.clone(),
        from: query.from.clone(),
        to: query.to.clone(),
        user: query.user.clone(),
        agent: query.agent.clone(),
    });
    insert_presence(&mut properties, "has_limit", query.limit.is_some());
    insert_presence(&mut properties, "has_offset", query.offset.is_some());
    properties
}

fn branches_query_properties(query: &ApiBranchesQuery) -> HashMap<String, Value> {
    let mut properties = HashMap::new();
    insert_presence(&mut properties, "has_from", query.from.is_some());
    insert_presence(&mut properties, "has_to", query.to.is_some());
    properties
}

fn users_query_properties(query: &ApiUsersQuery) -> HashMap<String, Value> {
    let mut properties = HashMap::new();
    insert_presence(&mut properties, "has_branch_filter", query.branch.is_some());
    insert_presence(&mut properties, "has_agent_filter", query.agent.is_some());
    insert_presence(&mut properties, "has_from", query.from.is_some());
    insert_presence(&mut properties, "has_to", query.to.is_some());
    properties
}

fn agents_query_properties(query: &ApiAgentsQuery) -> HashMap<String, Value> {
    let mut properties = HashMap::new();
    insert_presence(&mut properties, "has_branch_filter", query.branch.is_some());
    insert_presence(&mut properties, "has_user_filter", query.user.is_some());
    insert_presence(&mut properties, "has_from", query.from.is_some());
    insert_presence(&mut properties, "has_to", query.to.is_some());
    properties
}

#[cfg(test)]
mod telemetry_tests {
    use super::*;

    #[test]
    fn commits_query_properties_only_capture_filter_presence() {
        let properties = commits_query_properties(&ApiCommitsQuery {
            branch: Some("main".to_string()),
            from: None,
            to: Some("123".to_string()),
            user: None,
            agent: Some("codex".to_string()),
            limit: Some("20".to_string()),
            offset: None,
        });

        assert_eq!(
            properties.get("has_branch_filter").and_then(Value::as_bool),
            Some(true)
        );
        assert_eq!(
            properties.get("has_agent_filter").and_then(Value::as_bool),
            Some(true)
        );
        assert_eq!(
            properties.get("has_to").and_then(Value::as_bool),
            Some(true)
        );
        assert_eq!(
            properties.get("has_limit").and_then(Value::as_bool),
            Some(true)
        );

        let rendered = serde_json::to_string(&properties).expect("serialize properties");
        assert!(
            !rendered.contains("main") && !rendered.contains("codex") && !rendered.contains("20"),
            "dashboard telemetry should not capture raw filter values"
        );
    }
}
