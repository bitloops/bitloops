use super::dto::{
    ApiAgentDto, ApiAgentsQuery, ApiBackendHealthDto, ApiBranchSummaryDto, ApiBranchesQuery,
    ApiCheckBundleVersionResponse, ApiCheckpointDetailResponse, ApiCheckpointDto,
    ApiCheckpointSessionDetailDto, ApiCommitDto, ApiCommitFileDiffDto, ApiCommitRowDto,
    ApiCommitsQuery, ApiDbHealthResponse, ApiError, ApiErrorEnvelope, ApiFetchBundleResponse,
    ApiKpisQuery, ApiKpisResponse, ApiRootResponse, ApiTokenUsageDto, ApiUserDto, ApiUsersQuery,
    DashboardApiDoc,
};
use super::{
    API_DEFAULT_PAGE_LIMIT, API_GIT_SCAN_LIMIT, ApiPage, CommitCheckpointQuery, DashboardState,
    canonical_agent_key, db, read_checkpoint_info_for_filtering, read_commit_numstat,
    walk_branch_commits_with_checkpoints,
};
use super::{bundle, bundle_types::BundleError};
use crate::host::checkpoints::checkpoint_id::is_valid_checkpoint_id;
use crate::host::checkpoints::strategy::manual_commit::{CommittedInfo, read_session_content};
use async_graphql::{Request as GraphqlRequest, Variables};
use axum::{
    Json,
    extract::{Path as AxumPath, Query, State},
    http::StatusCode,
};
use chrono::{DateTime, TimeZone, Utc};
use serde::{Deserialize, de::DeserializeOwned};
use serde_json::json;
use std::collections::{HashMap, HashSet};
use std::path::Path;
use utoipa::OpenApi;

const DASHBOARD_GRAPHQL_REPO_NAME: &str = "dashboard";

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DashboardGraphqlBranchesData {
    repo: DashboardGraphqlBranchesRepo,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DashboardGraphqlBranchesRepo {
    branches: Vec<DashboardGraphqlBranch>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DashboardGraphqlBranch {
    name: String,
    checkpoint_count: usize,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DashboardGraphqlCommitsData {
    repo: DashboardGraphqlCommitsRepo,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DashboardGraphqlCommitsRepo {
    commits: DashboardGraphqlCommitConnection,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DashboardGraphqlCommitConnection {
    edges: Vec<DashboardGraphqlCommitEdge>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DashboardGraphqlCommitEdge {
    node: DashboardGraphqlCommitNode,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DashboardGraphqlCommitNode {
    sha: String,
    parents: Vec<String>,
    author_name: String,
    author_email: String,
    committed_at: String,
    commit_message: String,
    checkpoints: DashboardGraphqlCheckpointConnection,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DashboardGraphqlCheckpointConnection {
    edges: Vec<DashboardGraphqlCheckpointEdge>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DashboardGraphqlCheckpointEdge {
    node: DashboardGraphqlCheckpointNode,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DashboardGraphqlCheckpointNode {
    id: String,
    branch: Option<String>,
    agent: Option<String>,
    strategy: Option<String>,
    files_touched: Vec<String>,
    checkpoints_count: usize,
    session_count: usize,
    token_usage: Option<DashboardGraphqlTokenUsage>,
    session_id: String,
    agents: Vec<String>,
    first_prompt_preview: Option<String>,
    created_at: Option<String>,
    is_task: bool,
    tool_use_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DashboardGraphqlTokenUsage {
    input_tokens: u64,
    output_tokens: u64,
    cache_creation_tokens: u64,
    cache_read_tokens: u64,
    api_call_count: u64,
}

#[utoipa::path(
    get,
    path = "/api",
    responses((status = 200, description = "Dashboard API root", body = ApiRootResponse))
)]
pub(crate) async fn handle_api_root() -> Json<ApiRootResponse> {
    Json(ApiRootResponse {
        name: "bitloops-dashboard-api".to_string(),
        openapi: "/api/openapi.json".to_string(),
    })
}

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
    let mut filter = parse_commit_checkpoint_filter(
        query.branch,
        query.from,
        query.to,
        query.user,
        query.agent,
    )?;

    let page = ApiPage {
        limit: parse_optional_usize("limit", query.limit)?.unwrap_or(API_DEFAULT_PAGE_LIMIT),
        offset: parse_optional_usize("offset", query.offset)?.unwrap_or(0),
    }
    .normalized();
    filter.page = page;

    let rows = load_dashboard_commit_rows_via_graphql(&state, &filter).await?;
    let rows = super::paginate(&rows, filter.page);
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
    let from_unix = parse_optional_unix_seconds("from", query.from)?;
    let to_unix = parse_optional_unix_seconds("to", query.to)?;
    validate_time_window(from_unix, to_unix)?;
    Ok(Json(
        load_dashboard_branches_via_graphql(&state, from_unix, to_unix).await?,
    ))
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
    let filter =
        parse_commit_checkpoint_filter(query.branch, query.from, query.to, None, query.agent)?;
    let rows = load_dashboard_commit_rows_via_graphql(&state, &filter).await?;

    let mut users_by_key: HashMap<String, ApiUserDto> = HashMap::new();
    for row in rows {
        let user = super::dashboard_user(&row.commit.author_name, &row.commit.author_email);
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

#[utoipa::path(
    get,
    path = "/api/checkpoints/{checkpoint_id}",
    params(
        ("checkpoint_id" = String, Path, description = "Checkpoint id (12 hex characters)")
    ),
    responses(
        (status = 200, description = "Checkpoint details with session transcript payloads", body = ApiCheckpointDetailResponse),
        (status = 400, description = "Bad request", body = ApiErrorEnvelope),
        (status = 404, description = "Not found", body = ApiErrorEnvelope),
        (status = 500, description = "Internal server error", body = ApiErrorEnvelope)
    )
)]
pub(crate) async fn handle_api_checkpoint(
    State(state): State<DashboardState>,
    AxumPath(checkpoint_id): AxumPath<String>,
) -> std::result::Result<Json<ApiCheckpointDetailResponse>, ApiError> {
    let checkpoint_id = normalize_checkpoint_id(checkpoint_id)?;
    let Some(info) =
        read_checkpoint_info_for_filtering(&state.repo_root, &checkpoint_id).map_err(|err| {
            ApiError::internal(format!(
                "failed to read checkpoint metadata for {checkpoint_id}: {err:#}"
            ))
        })?
    else {
        return Err(ApiError::not_found(format!(
            "checkpoint not found: {checkpoint_id}"
        )));
    };

    let mut sessions = Vec::new();
    for session_index in 0..info.session_count {
        let content = match read_session_content(&state.repo_root, &checkpoint_id, session_index) {
            Ok(content) => content,
            Err(err) => {
                log::warn!(
                    "dashboard checkpoint endpoint skipped unreadable session {}#{}: {:#}",
                    checkpoint_id,
                    session_index,
                    err
                );
                continue;
            }
        };

        let metadata = content.metadata;
        sessions.push(ApiCheckpointSessionDetailDto {
            session_index,
            session_id: metadata
                .get("session_id")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .to_string(),
            agent: metadata
                .get("agent")
                .and_then(serde_json::Value::as_str)
                .map(canonical_agent_key)
                .unwrap_or_default(),
            created_at: metadata
                .get("created_at")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .to_string(),
            is_task: metadata
                .get("is_task")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false),
            tool_use_id: metadata
                .get("tool_use_id")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .to_string(),
            metadata_json: serde_json::to_string_pretty(&metadata)
                .unwrap_or_else(|_| "{}".to_string()),
            transcript_jsonl: content.transcript,
            prompts_text: content.prompts,
            context_text: content.context,
        });
    }

    let files_touched = resolve_checkpoint_files_touched(
        &state.repo_root,
        &info.branch,
        &info.checkpoint_id,
        &info.files_touched,
    );
    let token_usage = api_token_usage_from_committed(&info);
    Ok(Json(ApiCheckpointDetailResponse {
        checkpoint_id: info.checkpoint_id,
        strategy: info.strategy,
        branch: info.branch,
        checkpoints_count: info.checkpoints_count,
        files_touched,
        session_count: info.session_count,
        token_usage,
        sessions,
    }))
}

#[utoipa::path(
    get,
    path = "/api/openapi.json",
    responses((status = 200, description = "Generated OpenAPI document"))
)]
pub(crate) async fn handle_api_openapi() -> Json<utoipa::openapi::OpenApi> {
    Json(DashboardApiDoc::openapi())
}

#[utoipa::path(
    get,
    path = "/api/db/health",
    responses((status = 200, description = "Live database backend health", body = ApiDbHealthResponse))
)]
pub(crate) async fn handle_api_db_health(
    State(state): State<DashboardState>,
) -> Json<ApiDbHealthResponse> {
    let health = state.db.health_check().await;

    Json(ApiDbHealthResponse {
        relational: map_backend_health(health.relational),
        events: map_backend_health(health.events),
        postgres: map_backend_health(health.postgres),
        clickhouse: map_backend_health(health.clickhouse),
    })
}

#[utoipa::path(
    get,
    path = "/api/check_bundle_version",
    responses(
        (status = 200, description = "Dashboard bundle install/update availability", body = ApiCheckBundleVersionResponse),
        (status = 502, description = "Manifest fetch failure", body = ApiErrorEnvelope),
        (status = 500, description = "Internal server error", body = ApiErrorEnvelope)
    )
)]
pub(crate) async fn handle_api_check_bundle_version(
    State(state): State<DashboardState>,
) -> std::result::Result<Json<ApiCheckBundleVersionResponse>, ApiError> {
    let bundle_dir = state.bundle_dir.display().to_string();
    log::info!(
        "event=dashboard.bundle.check.started operation=check_bundle_version status=started bundle_dir={bundle_dir}"
    );

    match bundle::check_bundle_version(&state).await {
        Ok(result) => {
            let latest_applicable_version = result
                .latest_applicable_version
                .clone()
                .unwrap_or_else(|| "null".to_string());
            log::info!(
                "event=dashboard.bundle.check.succeeded operation=check_bundle_version status=succeeded bundle_dir={} install_available={} latest_applicable_version={}",
                bundle_dir,
                result.install_available,
                latest_applicable_version
            );
            Ok(Json(ApiCheckBundleVersionResponse {
                current_version: result.current_version,
                latest_applicable_version: result.latest_applicable_version,
                install_available: result.install_available,
                reason: result.reason.as_str().to_string(),
            }))
        }
        Err(error) => {
            let error_code = bundle_error_code(&error);
            let api_error = map_bundle_error(error);
            log::error!(
                "event=dashboard.bundle.check.failed operation=check_bundle_version status=failed bundle_dir={} error_code={} error_message={}",
                bundle_dir,
                error_code,
                api_error.message
            );
            Err(api_error)
        }
    }
}

#[utoipa::path(
    post,
    path = "/api/fetch_bundle",
    request_body = inline(serde_json::Value),
    responses(
        (status = 200, description = "Bundle fetched and installed", body = ApiFetchBundleResponse),
        (status = 409, description = "No compatible version", body = ApiErrorEnvelope),
        (status = 422, description = "Checksum mismatch", body = ApiErrorEnvelope),
        (status = 502, description = "Download/manifest fetch failure", body = ApiErrorEnvelope),
        (status = 500, description = "Install failure", body = ApiErrorEnvelope)
    )
)]
pub(crate) async fn handle_api_fetch_bundle(
    State(state): State<DashboardState>,
) -> std::result::Result<Json<ApiFetchBundleResponse>, ApiError> {
    let bundle_dir = state.bundle_dir.display().to_string();
    log::info!(
        "event=dashboard.bundle.install.started operation=fetch_bundle status=started bundle_dir={bundle_dir}"
    );

    match bundle::fetch_bundle(&state).await {
        Ok(result) => {
            log::info!(
                "event=dashboard.bundle.install.succeeded operation=fetch_bundle status=succeeded bundle_dir={} installed_version={} checksum_verified={}",
                result.bundle_dir,
                result.installed_version,
                result.checksum_verified
            );
            Ok(Json(ApiFetchBundleResponse {
                installed_version: result.installed_version,
                bundle_dir: result.bundle_dir,
                status: result.status,
                checksum_verified: result.checksum_verified,
            }))
        }
        Err(BundleError::ChecksumMismatch) => {
            log::warn!(
                "event=dashboard.bundle.install.checksum_mismatch operation=fetch_bundle status=failed bundle_dir={} error_code=checksum_mismatch",
                bundle_dir
            );
            let api_error = map_bundle_error(BundleError::ChecksumMismatch);
            log::error!(
                "event=dashboard.bundle.install.failed operation=fetch_bundle status=failed bundle_dir={} error_code=checksum_mismatch error_message={}",
                bundle_dir,
                api_error.message
            );
            Err(api_error)
        }
        Err(error) => {
            let error_code = bundle_error_code(&error);
            let api_error = map_bundle_error(error);
            log::error!(
                "event=dashboard.bundle.install.failed operation=fetch_bundle status=failed bundle_dir={} error_code={} error_message={}",
                bundle_dir,
                error_code,
                api_error.message
            );
            Err(api_error)
        }
    }
}

pub(crate) async fn handle_api_not_found() -> ApiError {
    ApiError::not_found("route not found")
}

fn map_backend_health(health: db::BackendHealth) -> ApiBackendHealthDto {
    ApiBackendHealthDto {
        status: health.status_label().to_string(),
        detail: health.detail,
    }
}

fn map_bundle_error(error: BundleError) -> ApiError {
    match error {
        BundleError::ManifestFetchFailed(message) => {
            ApiError::with_code(StatusCode::BAD_GATEWAY, "manifest_fetch_failed", message)
        }
        BundleError::ManifestParseFailed(message) => {
            ApiError::with_code(StatusCode::INTERNAL_SERVER_ERROR, "internal", message)
        }
        BundleError::NoCompatibleVersion => ApiError::with_code(
            StatusCode::CONFLICT,
            "no_compatible_version",
            "no compatible dashboard bundle version is available for this CLI version",
        ),
        BundleError::BundleDownloadFailed(message) => {
            ApiError::with_code(StatusCode::BAD_GATEWAY, "bundle_download_failed", message)
        }
        BundleError::ChecksumMismatch => ApiError::with_code(
            StatusCode::UNPROCESSABLE_ENTITY,
            "checksum_mismatch",
            "downloaded bundle checksum did not match",
        ),
        BundleError::BundleInstallFailed(message) => ApiError::with_code(
            StatusCode::INTERNAL_SERVER_ERROR,
            "bundle_install_failed",
            message,
        ),
        BundleError::Internal(message) => {
            ApiError::with_code(StatusCode::INTERNAL_SERVER_ERROR, "internal", message)
        }
    }
}

fn bundle_error_code(error: &BundleError) -> &'static str {
    match error {
        BundleError::ManifestFetchFailed(_) => "manifest_fetch_failed",
        BundleError::ManifestParseFailed(_) => "internal",
        BundleError::NoCompatibleVersion => "no_compatible_version",
        BundleError::BundleDownloadFailed(_) => "bundle_download_failed",
        BundleError::ChecksumMismatch => "checksum_mismatch",
        BundleError::BundleInstallFailed(_) => "bundle_install_failed",
        BundleError::Internal(_) => "internal",
    }
}

fn normalize_optional_query(value: Option<String>) -> Option<String> {
    value.and_then(|raw| {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    })
}

fn require_query_value(
    field: &str,
    value: Option<String>,
) -> std::result::Result<String, ApiError> {
    normalize_optional_query(value)
        .ok_or_else(|| ApiError::bad_request(format!("{field} is required")))
}

fn parse_optional_unix_seconds(
    field: &str,
    value: Option<String>,
) -> std::result::Result<Option<i64>, ApiError> {
    let Some(raw) = normalize_optional_query(value) else {
        return Ok(None);
    };
    raw.parse::<i64>()
        .map(Some)
        .map_err(|_| ApiError::bad_request(format!("invalid {field}; expected unix seconds")))
}

fn parse_optional_usize(
    field: &str,
    value: Option<String>,
) -> std::result::Result<Option<usize>, ApiError> {
    let Some(raw) = normalize_optional_query(value) else {
        return Ok(None);
    };
    raw.parse::<usize>().map(Some).map_err(|_| {
        ApiError::bad_request(format!("invalid {field}; expected non-negative integer"))
    })
}

fn normalize_checkpoint_id(checkpoint_id: String) -> std::result::Result<String, ApiError> {
    let normalized = checkpoint_id.trim().to_ascii_lowercase();
    if !is_valid_checkpoint_id(&normalized) {
        return Err(ApiError::bad_request(
            "invalid checkpoint_id; expected 12 lowercase hex characters",
        ));
    }
    Ok(normalized)
}

fn validate_time_window(from: Option<i64>, to: Option<i64>) -> std::result::Result<(), ApiError> {
    if let (Some(from), Some(to)) = (from, to)
        && from > to
    {
        return Err(ApiError::bad_request(
            "from must be less than or equal to to",
        ));
    }
    Ok(())
}

fn parse_commit_checkpoint_filter(
    branch: Option<String>,
    from: Option<String>,
    to: Option<String>,
    user: Option<String>,
    agent: Option<String>,
) -> std::result::Result<CommitCheckpointQuery, ApiError> {
    let branch = require_query_value("branch", branch)?;
    let from_unix = parse_optional_unix_seconds("from", from)?;
    let to_unix = parse_optional_unix_seconds("to", to)?;
    validate_time_window(from_unix, to_unix)?;

    Ok(CommitCheckpointQuery {
        branch,
        from_unix,
        to_unix,
        user: normalize_optional_query(user),
        agent: normalize_optional_query(agent),
        page: ApiPage::default(),
    })
}

#[derive(Debug, Clone)]
struct DashboardGraphqlCommitRow {
    commit: DashboardGraphqlCommitNode,
    checkpoint: DashboardGraphqlCheckpointNode,
}

async fn load_dashboard_branches_via_graphql(
    state: &DashboardState,
    from_unix: Option<i64>,
    to_unix: Option<i64>,
) -> std::result::Result<Vec<ApiBranchSummaryDto>, ApiError> {
    let data: DashboardGraphqlBranchesData = execute_dashboard_graphql(
        state,
        r#"
        query DashboardBranches($repo: String!, $since: DateTime, $until: DateTime) {
          repo(name: $repo) {
            branches(since: $since, until: $until) {
              name
              checkpointCount
            }
          }
        }
        "#,
        json!({
            "repo": DASHBOARD_GRAPHQL_REPO_NAME,
            "since": optional_rfc3339_from_unix_seconds(from_unix),
            "until": optional_rfc3339_from_unix_seconds(to_unix),
        }),
    )
    .await?;

    Ok(data
        .repo
        .branches
        .into_iter()
        .map(|branch| ApiBranchSummaryDto {
            branch: branch.name,
            checkpoint_commits: branch.checkpoint_count,
        })
        .collect())
}

async fn load_dashboard_commit_rows_via_graphql(
    state: &DashboardState,
    filter: &CommitCheckpointQuery,
) -> std::result::Result<Vec<DashboardGraphqlCommitRow>, ApiError> {
    let data: DashboardGraphqlCommitsData = execute_dashboard_graphql(
        state,
        r#"
        query DashboardCommits($repo: String!, $branch: String!, $since: DateTime, $until: DateTime) {
          repo(name: $repo) {
            commits(branch: $branch, since: $since, until: $until, first: 5000) {
              edges {
                node {
                  sha
                  parents
                  authorName
                  authorEmail
                  committedAt
                  commitMessage
                  checkpoints(first: 1) {
                    edges {
                      node {
                        id
                        branch
                        agent
                        strategy
                        filesTouched
                        checkpointsCount
                        sessionCount
                        tokenUsage {
                          inputTokens
                          outputTokens
                          cacheCreationTokens
                          cacheReadTokens
                          apiCallCount
                        }
                        sessionId
                        agents
                        firstPromptPreview
                        createdAt
                        isTask
                        toolUseId
                      }
                    }
                  }
                }
              }
            }
          }
        }
        "#,
        json!({
            "repo": DASHBOARD_GRAPHQL_REPO_NAME,
            "branch": filter.branch.as_str(),
            "since": optional_rfc3339_from_unix_seconds(filter.from_unix),
            "until": optional_rfc3339_from_unix_seconds(filter.to_unix),
        }),
    )
    .await?;

    let mut rows = Vec::new();
    for edge in data.repo.commits.edges {
        let commit = edge.node;
        let Some(checkpoint) = commit
            .checkpoints
            .edges
            .first()
            .cloned()
            .map(|edge| edge.node)
        else {
            continue;
        };

        let user = super::dashboard_user(&commit.author_name, &commit.author_email);
        if !super::user_matches_filter(&user, filter.user.as_deref()) {
            continue;
        }
        if !graphql_checkpoint_matches_agent_filter(&checkpoint, filter.agent.as_deref()) {
            continue;
        }

        rows.push(DashboardGraphqlCommitRow { commit, checkpoint });
    }

    Ok(rows)
}

async fn execute_dashboard_graphql<T: DeserializeOwned>(
    state: &DashboardState,
    query: &str,
    variables: serde_json::Value,
) -> std::result::Result<T, ApiError> {
    let response = state
        .devql_schema()
        .execute(GraphqlRequest::new(query).variables(Variables::from_json(variables)))
        .await;

    if let Some(error) = response.errors.first() {
        return Err(map_dashboard_graphql_error(error));
    }

    let data = response.data.into_json().map_err(|err| {
        ApiError::internal(format!("failed to decode dashboard GraphQL data: {err:#}"))
    })?;

    serde_json::from_value(data).map_err(|err| {
        ApiError::internal(format!(
            "failed to decode dashboard GraphQL response payload: {err}"
        ))
    })
}

fn map_dashboard_graphql_error(error: &async_graphql::ServerError) -> ApiError {
    let code = error
        .extensions
        .as_ref()
        .and_then(|extensions| extensions.get("code"))
        .and_then(|value| match value {
            async_graphql::Value::String(value) => Some(value.as_str()),
            _ => None,
        });
    match code {
        Some("BAD_USER_INPUT") | Some("BAD_CURSOR") => ApiError::bad_request(error.message.clone()),
        _ => ApiError::internal(format!(
            "dashboard GraphQL wrapper failed: {}",
            error.message
        )),
    }
}

fn optional_rfc3339_from_unix_seconds(value: Option<i64>) -> Option<String> {
    value.and_then(|timestamp| {
        Utc.timestamp_opt(timestamp, 0)
            .single()
            .map(|value| value.to_rfc3339())
    })
}

fn build_kpis_response_from_graphql_rows(rows: &[DashboardGraphqlCommitRow]) -> ApiKpisResponse {
    let mut unique_checkpoint_ids: HashSet<String> = HashSet::new();
    let mut unique_agents: HashSet<String> = HashSet::new();
    let mut total_sessions = 0usize;
    let mut files_touched: HashSet<String> = HashSet::new();
    let mut input_tokens = 0u64;
    let mut output_tokens = 0u64;
    let mut cache_creation_tokens = 0u64;
    let mut cache_read_tokens = 0u64;
    let mut api_call_count = 0u64;

    for row in rows {
        if !unique_checkpoint_ids.insert(row.checkpoint.id.clone()) {
            continue;
        }

        for agent_key in checkpoint_agents(&row.checkpoint) {
            unique_agents.insert(agent_key);
        }
        total_sessions += row.checkpoint.session_count;
        for file in &row.checkpoint.files_touched {
            files_touched.insert(file.clone());
        }

        if let Some(token_usage) = row.checkpoint.token_usage.as_ref() {
            input_tokens += token_usage.input_tokens;
            output_tokens += token_usage.output_tokens;
            cache_creation_tokens += token_usage.cache_creation_tokens;
            cache_read_tokens += token_usage.cache_read_tokens;
            api_call_count += token_usage.api_call_count;
        }
    }

    let total_checkpoints = unique_checkpoint_ids.len();
    let total_tokens = input_tokens + output_tokens + cache_creation_tokens + cache_read_tokens;
    let average_tokens_per_checkpoint = if total_checkpoints == 0 {
        0.0
    } else {
        total_tokens as f64 / total_checkpoints as f64
    };
    let average_sessions_per_checkpoint = if total_checkpoints == 0 {
        0.0
    } else {
        total_sessions as f64 / total_checkpoints as f64
    };

    ApiKpisResponse {
        total_commits: rows.len(),
        total_checkpoints,
        total_agents: unique_agents.len(),
        total_sessions,
        files_touched_count: files_touched.len(),
        input_tokens,
        output_tokens,
        cache_creation_tokens,
        cache_read_tokens,
        api_call_count,
        average_tokens_per_checkpoint,
        average_sessions_per_checkpoint,
    }
}

fn graphql_checkpoint_matches_agent_filter(
    checkpoint: &DashboardGraphqlCheckpointNode,
    agent_filter: Option<&str>,
) -> bool {
    let Some(filter) = agent_filter else {
        return true;
    };

    let normalized = canonical_agent_key(filter);
    if normalized.is_empty() {
        return true;
    }

    checkpoint_agents(checkpoint)
        .into_iter()
        .any(|agent| agent == normalized)
}

fn checkpoint_agents(info: &DashboardGraphqlCheckpointNode) -> Vec<String> {
    let mut keys: Vec<String> = Vec::new();

    if info.agents.is_empty() {
        let key = canonical_agent_key(info.agent.as_deref().unwrap_or_default());
        if !key.is_empty() {
            keys.push(key);
        }
        return keys;
    }

    for agent in &info.agents {
        let key = canonical_agent_key(agent);
        if key.is_empty() || keys.iter().any(|existing| existing == &key) {
            continue;
        }
        keys.push(key);
    }
    keys
}

fn api_token_usage_from_graphql(
    usage: Option<&DashboardGraphqlTokenUsage>,
) -> Option<ApiTokenUsageDto> {
    usage.map(|usage| ApiTokenUsageDto {
        input_tokens: usage.input_tokens,
        output_tokens: usage.output_tokens,
        cache_creation_tokens: usage.cache_creation_tokens,
        cache_read_tokens: usage.cache_read_tokens,
        api_call_count: usage.api_call_count,
    })
}

fn api_token_usage_from_committed(info: &CommittedInfo) -> Option<ApiTokenUsageDto> {
    info.token_usage.as_ref().map(|usage| ApiTokenUsageDto {
        input_tokens: usage.input_tokens,
        output_tokens: usage.output_tokens,
        cache_creation_tokens: usage.cache_creation_tokens,
        cache_read_tokens: usage.cache_read_tokens,
        api_call_count: usage.api_call_count,
    })
}

fn api_file_diff_list_from_numstat(
    stats: HashMap<String, (u64, u64)>,
) -> Vec<ApiCommitFileDiffDto> {
    let mut files_touched: Vec<ApiCommitFileDiffDto> = stats
        .into_iter()
        .map(|(filepath, (adds, dels))| ApiCommitFileDiffDto {
            filepath,
            additions_count: adds,
            deletions_count: dels,
        })
        .collect();
    files_touched.sort_by(|left, right| left.filepath.cmp(&right.filepath));
    files_touched
}

fn api_zeroed_file_diff_list(files_touched: &[String]) -> Vec<ApiCommitFileDiffDto> {
    let mut files_touched: Vec<ApiCommitFileDiffDto> = files_touched
        .iter()
        .cloned()
        .map(|filepath| ApiCommitFileDiffDto {
            filepath,
            additions_count: 0,
            deletions_count: 0,
        })
        .collect();
    files_touched.sort_by(|left, right| left.filepath.cmp(&right.filepath));
    files_touched
}

fn resolve_checkpoint_files_touched(
    repo_root: &Path,
    branch: &str,
    checkpoint_id: &str,
    fallback_files_touched: &[String],
) -> Vec<ApiCommitFileDiffDto> {
    let branch_commits = match walk_branch_commits_with_checkpoints(
        repo_root,
        branch,
        None,
        None,
        API_GIT_SCAN_LIMIT,
    ) {
        Ok(commits) => commits,
        Err(err) => {
            log::warn!(
                "dashboard checkpoint endpoint: failed to walk branch {} while resolving files_touched for {}: {:#}",
                branch,
                checkpoint_id,
                err
            );
            return api_zeroed_file_diff_list(fallback_files_touched);
        }
    };

    let Some(commit_sha) = branch_commits
        .into_iter()
        .find(|commit| commit.checkpoint_id == checkpoint_id)
        .map(|commit| commit.sha)
    else {
        return api_zeroed_file_diff_list(fallback_files_touched);
    };

    match read_commit_numstat(repo_root, &commit_sha) {
        Ok(stats) => api_file_diff_list_from_numstat(stats),
        Err(err) => {
            log::warn!(
                "dashboard checkpoint endpoint: failed to read numstat for {} (checkpoint {}): {:#}",
                commit_sha,
                checkpoint_id,
                err
            );
            api_zeroed_file_diff_list(fallback_files_touched)
        }
    }
}

fn api_commit_row_from_graphql(
    row: DashboardGraphqlCommitRow,
    files_touched: Vec<ApiCommitFileDiffDto>,
) -> ApiCommitRowDto {
    let DashboardGraphqlCommitRow { commit, checkpoint } = row;
    let agents = checkpoint_agents(&checkpoint);
    let checkpoint_files_touched = files_touched.clone();
    let timestamp = DateTime::parse_from_rfc3339(&commit.committed_at)
        .map(|value| value.timestamp())
        .unwrap_or(0);
    ApiCommitRowDto {
        commit: ApiCommitDto {
            sha: commit.sha,
            parents: commit.parents,
            author_name: commit.author_name,
            author_email: commit.author_email,
            timestamp,
            message: commit.commit_message,
            files_touched,
        },
        checkpoint: ApiCheckpointDto {
            checkpoint_id: checkpoint.id,
            strategy: checkpoint.strategy.unwrap_or_default(),
            branch: checkpoint.branch.unwrap_or_default(),
            checkpoints_count: checkpoint.checkpoints_count.try_into().unwrap_or(u32::MAX),
            files_touched: checkpoint_files_touched,
            session_count: checkpoint.session_count,
            token_usage: api_token_usage_from_graphql(checkpoint.token_usage.as_ref()),
            session_id: checkpoint.session_id,
            agents,
            first_prompt_preview: checkpoint.first_prompt_preview.unwrap_or_default(),
            created_at: checkpoint.created_at.unwrap_or_default(),
            is_task: checkpoint.is_task,
            tool_use_id: checkpoint.tool_use_id.unwrap_or_default(),
        },
    }
}
