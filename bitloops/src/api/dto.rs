use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, OpenApi, ToSchema};

#[derive(Debug, Clone, Serialize, ToSchema)]
pub(super) struct ApiErrorEnvelope {
    pub(super) error: ApiErrorBody,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub(super) struct ApiErrorBody {
    pub(super) code: String,
    pub(super) message: String,
}

#[derive(Debug)]
pub(super) struct ApiError {
    pub(super) status: StatusCode,
    pub(super) code: &'static str,
    pub(super) message: String,
}

impl ApiError {
    pub(super) fn bad_request(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            code: "bad_request",
            message: message.into(),
        }
    }

    pub(super) fn internal(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            code: "internal",
            message: message.into(),
        }
    }

    pub(super) fn not_found(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            code: "not_found",
            message: message.into(),
        }
    }

    pub(super) fn payload_too_large(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::PAYLOAD_TOO_LARGE,
            code: "payload_too_large",
            message: message.into(),
        }
    }

    pub(super) fn with_code(
        status: StatusCode,
        code: &'static str,
        message: impl Into<String>,
    ) -> Self {
        Self {
            status,
            code,
            message: message.into(),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let payload = ApiErrorEnvelope {
            error: ApiErrorBody {
                code: self.code.to_string(),
                message: self.message,
            },
        };
        (self.status, Json(payload)).into_response()
    }
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub(super) struct ApiTokenUsageDto {
    pub(super) input_tokens: u64,
    pub(super) output_tokens: u64,
    pub(super) cache_creation_tokens: u64,
    pub(super) cache_read_tokens: u64,
    pub(super) api_call_count: u64,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub(super) struct ApiCheckpointDto {
    pub(super) checkpoint_id: String,
    pub(super) strategy: String,
    pub(super) branch: String,
    pub(super) checkpoints_count: u32,
    pub(super) files_touched: Vec<ApiCommitFileDiffDto>,
    pub(super) session_count: usize,
    pub(super) token_usage: Option<ApiTokenUsageDto>,
    pub(super) session_id: String,
    pub(super) agents: Vec<String>,
    pub(super) first_prompt_preview: String,
    pub(super) created_at: String,
    pub(super) is_task: bool,
    pub(super) tool_use_id: String,
}

#[derive(Debug, Clone, Serialize, ToSchema, Default)]
#[serde(rename_all = "camelCase")]
pub(super) struct ApiCommitFileDiffDto {
    pub(super) filepath: String,
    pub(super) additions_count: u64,
    pub(super) deletions_count: u64,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub(super) struct ApiCommitDto {
    pub(super) sha: String,
    pub(super) parents: Vec<String>,
    pub(super) author_name: String,
    pub(super) author_email: String,
    pub(super) timestamp: i64,
    pub(super) message: String,
    pub(super) files_touched: Vec<ApiCommitFileDiffDto>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub(super) struct ApiCommitRowDto {
    pub(super) commit: ApiCommitDto,
    pub(super) checkpoint: ApiCheckpointDto,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub(super) struct ApiKpisResponse {
    pub(super) total_commits: usize,
    pub(super) total_checkpoints: usize,
    pub(super) total_agents: usize,
    pub(super) total_sessions: usize,
    pub(super) files_touched_count: usize,
    pub(super) input_tokens: u64,
    pub(super) output_tokens: u64,
    pub(super) cache_creation_tokens: u64,
    pub(super) cache_read_tokens: u64,
    pub(super) api_call_count: u64,
    pub(super) average_tokens_per_checkpoint: f64,
    pub(super) average_sessions_per_checkpoint: f64,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub(super) struct ApiBranchSummaryDto {
    pub(super) branch: String,
    pub(super) checkpoint_commits: usize,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub(super) struct ApiUserDto {
    pub(super) key: String,
    pub(super) name: String,
    pub(super) email: String,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub(super) struct ApiAgentDto {
    pub(super) key: String,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub(super) struct ApiRepositoryDto {
    pub(super) repo_id: String,
    pub(super) identity: String,
    pub(super) name: String,
    pub(super) provider: String,
    pub(super) organization: String,
    pub(super) default_branch: Option<String>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub(super) struct ApiRootResponse {
    pub(super) name: String,
    pub(super) openapi: String,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub(super) struct ApiBackendHealthDto {
    pub(super) status: String,
    pub(super) detail: String,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub(super) struct ApiDbHealthResponse {
    pub(super) relational: ApiBackendHealthDto,
    pub(super) events: ApiBackendHealthDto,
    pub(super) postgres: ApiBackendHealthDto,
    pub(super) clickhouse: ApiBackendHealthDto,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub(super) struct ApiCheckpointSessionDetailDto {
    pub(super) session_index: usize,
    pub(super) session_id: String,
    pub(super) agent: String,
    pub(super) created_at: String,
    pub(super) is_task: bool,
    pub(super) tool_use_id: String,
    pub(super) metadata_json: String,
    pub(super) transcript_jsonl: String,
    pub(super) prompts_text: String,
    pub(super) context_text: String,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub(super) struct ApiCheckpointDetailResponse {
    pub(super) checkpoint_id: String,
    pub(super) strategy: String,
    pub(super) branch: String,
    pub(super) checkpoints_count: u32,
    pub(super) files_touched: Vec<ApiCommitFileDiffDto>,
    pub(super) session_count: usize,
    pub(super) token_usage: Option<ApiTokenUsageDto>,
    pub(super) sessions: Vec<ApiCheckpointSessionDetailDto>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub(super) struct ApiCheckBundleVersionResponse {
    pub(super) current_version: Option<String>,
    pub(super) latest_applicable_version: Option<String>,
    pub(super) install_available: bool,
    pub(super) reason: String,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub(super) struct ApiFetchBundleResponse {
    pub(super) installed_version: String,
    pub(super) bundle_dir: String,
    pub(super) status: String,
    pub(super) checksum_verified: bool,
}

#[derive(Debug, Clone, Deserialize, IntoParams)]
pub(super) struct ApiKpisQuery {
    pub(super) branch: Option<String>,
    pub(super) from: Option<String>,
    pub(super) to: Option<String>,
    pub(super) user: Option<String>,
    pub(super) agent: Option<String>,
}

#[derive(Debug, Clone, Deserialize, IntoParams)]
pub(super) struct ApiCommitsQuery {
    pub(super) branch: Option<String>,
    pub(super) from: Option<String>,
    pub(super) to: Option<String>,
    pub(super) user: Option<String>,
    pub(super) agent: Option<String>,
    pub(super) limit: Option<String>,
    pub(super) offset: Option<String>,
}

#[derive(Debug, Clone, Deserialize, IntoParams)]
pub(super) struct ApiBranchesQuery {
    pub(super) from: Option<String>,
    pub(super) to: Option<String>,
}

#[derive(Debug, Clone, Deserialize, IntoParams)]
pub(super) struct ApiUsersQuery {
    pub(super) branch: Option<String>,
    pub(super) from: Option<String>,
    pub(super) to: Option<String>,
    pub(super) agent: Option<String>,
}

#[derive(Debug, Clone, Deserialize, IntoParams)]
pub(super) struct ApiAgentsQuery {
    pub(super) branch: Option<String>,
    pub(super) from: Option<String>,
    pub(super) to: Option<String>,
    pub(super) user: Option<String>,
}

#[derive(OpenApi)]
#[openapi(
    paths(
        super::handlers::meta::handle_api_root,
        super::handlers::dashboard::handle_api_kpis,
        super::handlers::dashboard::handle_api_commits,
        super::handlers::dashboard::handle_api_branches,
        super::handlers::dashboard::handle_api_repositories,
        super::handlers::dashboard::handle_api_users,
        super::handlers::dashboard::handle_api_agents,
        super::handlers::checkpoint::handle_api_checkpoint,
        super::handlers::git_blob::handle_api_git_blob,
        super::handlers::health::handle_api_db_health,
        super::handlers::bundle::handle_api_check_bundle_version,
        super::handlers::bundle::handle_api_fetch_bundle,
        super::handlers::meta::handle_api_openapi
    ),
    components(schemas(
        ApiErrorEnvelope,
        ApiErrorBody,
        ApiTokenUsageDto,
        ApiCheckpointDto,
        ApiCommitFileDiffDto,
        ApiCommitDto,
        ApiCommitRowDto,
        ApiKpisResponse,
        ApiBranchSummaryDto,
        ApiRepositoryDto,
        ApiUserDto,
        ApiAgentDto,
        ApiRootResponse,
        ApiBackendHealthDto,
        ApiDbHealthResponse,
        ApiCheckpointSessionDetailDto,
        ApiCheckpointDetailResponse,
        ApiCheckBundleVersionResponse,
        ApiFetchBundleResponse
    ))
)]
pub(super) struct DashboardApiDoc;
