use super::dto::{
    ApiAgentDto, ApiAgentsQuery, ApiBackendHealthDto, ApiBranchSummaryDto, ApiBranchesQuery,
    ApiCheckBundleVersionResponse, ApiCheckpointDetailResponse, ApiCheckpointDto,
    ApiCheckpointSessionDetailDto, ApiCommitDto, ApiCommitFileDiffDto, ApiCommitRowDto,
    ApiCommitsQuery, ApiDbHealthResponse, ApiError, ApiErrorEnvelope, ApiFetchBundleResponse,
    ApiKpisQuery, ApiKpisResponse, ApiRootResponse, ApiTokenUsageDto, ApiUserDto, ApiUsersQuery,
    DashboardApiDoc,
};
use super::{
    API_DEFAULT_PAGE_LIMIT, API_GIT_SCAN_LIMIT, ApiPage, CommitCheckpointPair,
    CommitCheckpointQuery, DashboardState, build_committed_info_map, canonical_agent_key, db,
    list_dashboard_branches, query_commit_checkpoint_pairs, query_commit_checkpoint_pairs_all,
    read_checkpoint_info_for_filtering, read_commit_numstat, walk_branch_commits_with_checkpoints,
};
use super::{bundle, bundle_types::BundleError};
use crate::engine::strategy::manual_commit::{CommittedInfo, read_session_content};
use crate::engine::trailers::is_valid_checkpoint_id;
use axum::{
    Json,
    extract::{Path as AxumPath, Query, State},
    http::StatusCode,
};
use std::collections::{HashMap, HashSet};
use utoipa::OpenApi;

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
    let pairs = query_commit_checkpoint_pairs_all(&state.repo_root, &filter)
        .map_err(|err| ApiError::internal(format!("failed to query dashboard KPIs: {err:#}")))?;

    Ok(Json(build_kpis_response(&pairs)))
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

    let rows = query_commit_checkpoint_pairs(&state.repo_root, &filter)
        .map_err(|err| ApiError::internal(format!("failed to query dashboard commits: {err:#}")))?;

    let mut result = Vec::with_capacity(rows.len());
    for pair in rows {
        let files_touched = match read_commit_numstat(&state.repo_root, &pair.commit.sha) {
            Ok(stats) => stats
                .into_iter()
                .map(|(path, (adds, dels))| {
                    (
                        path,
                        ApiCommitFileDiffDto {
                            additions_count: adds,
                            deletions_count: dels,
                        },
                    )
                })
                .collect(),
            Err(err) => {
                log::warn!(
                    "dashboard commits endpoint: failed to read numstat for {}: {:#}",
                    pair.commit.sha,
                    err
                );
                HashMap::new()
            }
        };
        result.push(api_commit_row_from_pair(pair, files_touched));
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

    let branches = list_dashboard_branches(&state.repo_root)
        .map_err(|err| ApiError::internal(format!("failed to list dashboard branches: {err:#}")))?;
    let committed_map = build_committed_info_map(&state.repo_root).map_err(|err| {
        ApiError::internal(format!(
            "failed to read committed checkpoint metadata for branch filtering: {err:#}"
        ))
    })?;

    let mut items = Vec::new();
    for branch in branches {
        let commits = walk_branch_commits_with_checkpoints(
            &state.repo_root,
            &branch,
            from_unix,
            to_unix,
            API_GIT_SCAN_LIMIT,
        )
        .map_err(|err| {
            ApiError::internal(format!(
                "failed to walk branch {branch} for dashboard: {err:#}"
            ))
        })?;

        let checkpoint_commits = commits
            .iter()
            .filter(|commit| {
                !commit.checkpoint_id.is_empty()
                    && committed_map.contains_key(&commit.checkpoint_id)
            })
            .count();

        if checkpoint_commits > 0 {
            items.push(ApiBranchSummaryDto {
                branch,
                checkpoint_commits,
            });
        }
    }

    Ok(Json(items))
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
    let pairs = query_commit_checkpoint_pairs_all(&state.repo_root, &filter)
        .map_err(|err| ApiError::internal(format!("failed to query dashboard users: {err:#}")))?;

    let mut users_by_key: HashMap<String, ApiUserDto> = HashMap::new();
    for pair in pairs {
        let key = pair.user.key;
        let name = pair.user.name;
        let email = pair.user.email;

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
    let pairs = query_commit_checkpoint_pairs_all(&state.repo_root, &filter)
        .map_err(|err| ApiError::internal(format!("Failed to query dashboard agents: {err:#}")))?;

    let mut agents: Vec<ApiAgentDto> = pairs
        .into_iter()
        .filter_map(|pair| {
            let key = canonical_agent_key(&pair.checkpoint.agent);
            if key.is_empty() {
                None
            } else {
                Some(ApiAgentDto { key })
            }
        })
        .collect();
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

    let token_usage = api_token_usage_from_committed(&info);
    Ok(Json(ApiCheckpointDetailResponse {
        checkpoint_id: info.checkpoint_id,
        strategy: info.strategy,
        branch: info.branch,
        checkpoints_count: info.checkpoints_count,
        files_touched: info.files_touched,
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

fn build_kpis_response(pairs: &[CommitCheckpointPair]) -> ApiKpisResponse {
    let mut unique_checkpoint_ids: HashSet<String> = HashSet::new();
    let mut unique_agents: HashSet<String> = HashSet::new();
    let mut total_sessions = 0usize;
    let mut files_touched: HashSet<String> = HashSet::new();
    let mut input_tokens = 0u64;
    let mut output_tokens = 0u64;
    let mut cache_creation_tokens = 0u64;
    let mut cache_read_tokens = 0u64;
    let mut api_call_count = 0u64;

    for pair in pairs {
        if !unique_checkpoint_ids.insert(pair.checkpoint.checkpoint_id.clone()) {
            continue;
        }

        let agent_key = canonical_agent_key(&pair.checkpoint.agent);
        if !agent_key.is_empty() {
            unique_agents.insert(agent_key);
        }
        total_sessions += pair.checkpoint.session_count;
        for file in &pair.checkpoint.files_touched {
            files_touched.insert(file.clone());
        }

        if let Some(token_usage) = pair.checkpoint.token_usage.as_ref() {
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
        total_commits: pairs.len(),
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

fn api_token_usage_from_committed(info: &CommittedInfo) -> Option<ApiTokenUsageDto> {
    info.token_usage.as_ref().map(|usage| ApiTokenUsageDto {
        input_tokens: usage.input_tokens,
        output_tokens: usage.output_tokens,
        cache_creation_tokens: usage.cache_creation_tokens,
        cache_read_tokens: usage.cache_read_tokens,
        api_call_count: usage.api_call_count,
    })
}

fn api_checkpoint_from_committed(info: CommittedInfo) -> ApiCheckpointDto {
    let token_usage = api_token_usage_from_committed(&info);

    ApiCheckpointDto {
        checkpoint_id: info.checkpoint_id,
        strategy: info.strategy,
        branch: info.branch,
        checkpoints_count: info.checkpoints_count,
        files_touched: info.files_touched,
        session_count: info.session_count,
        token_usage,
        session_id: info.session_id,
        agent: info.agent,
        created_at: info.created_at,
        is_task: info.is_task,
        tool_use_id: info.tool_use_id,
    }
}

fn api_commit_row_from_pair(
    pair: CommitCheckpointPair,
    files_touched: HashMap<String, ApiCommitFileDiffDto>,
) -> ApiCommitRowDto {
    ApiCommitRowDto {
        commit: ApiCommitDto {
            sha: pair.commit.sha,
            parents: pair.commit.parents,
            author_name: pair.commit.author_name,
            author_email: pair.commit.author_email,
            timestamp: pair.commit.timestamp,
            message: pair.commit.message,
            files_touched,
        },
        checkpoint: api_checkpoint_from_committed(pair.checkpoint),
    }
}
