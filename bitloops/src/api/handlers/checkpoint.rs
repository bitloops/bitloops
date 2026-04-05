use std::path::Path;
use std::time::Instant;

use axum::{
    Json,
    extract::{Path as AxumPath, State},
    http::StatusCode,
};
use serde_json::Value;
use std::collections::HashMap;

use super::super::dto::{
    ApiCheckpointDetailResponse, ApiCheckpointSessionDetailDto, ApiError, ApiErrorEnvelope,
    ApiTokenUsageDto,
};
use super::super::{
    API_GIT_SCAN_LIMIT, DashboardState, canonical_agent_key, read_checkpoint_info_for_filtering,
    read_commit_numstat, walk_branch_commits_with_checkpoints,
};
use super::file_diffs::{
    api_checkpoint_file_diff_list_from_relations, api_file_diff_list_from_numstat,
    api_zeroed_file_diff_list,
};
use super::params::normalize_checkpoint_id;
use super::resolve_repo_root_from_repo_id;
use crate::host::checkpoints::strategy::manual_commit::{CommittedInfo, read_session_content};
use crate::host::devql::RelationalStorage;
use crate::host::devql::checkpoint_provenance::{
    CheckpointFileGateway, CheckpointFileProvenanceDetailRow,
};

#[utoipa::path(
    get,
    path = "/api/checkpoints/{repo_id}/{checkpoint_id}",
    params(
        ("repo_id" = String, Path, description = "Repository id"),
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
    AxumPath((repo_id, checkpoint_id)): AxumPath<(String, String)>,
) -> std::result::Result<Json<ApiCheckpointDetailResponse>, ApiError> {
    let started = Instant::now();
    let (tracked_repo_root, response) = match resolve_repo_root_from_repo_id(&state, &repo_id).await
    {
        Ok(repo_root) => {
            let response = load_checkpoint_detail(&repo_root, &repo_id, checkpoint_id)
                .await
                .map(Json);
            (Some(repo_root), response)
        }
        Err(err) => (None, Err(err)),
    };

    let status = match &response {
        Ok(_) => StatusCode::OK,
        Err(err) => err.status_code(),
    };
    let mut properties = HashMap::new();
    properties.insert("http_method".to_string(), Value::String("GET".to_string()));
    properties.insert("repo_id".to_string(), Value::String(repo_id));
    properties.insert(
        "status_code_class".to_string(),
        Value::String(super::super::status_code_class(status).to_string()),
    );
    if let Some(repo_root) = tracked_repo_root.as_deref() {
        super::super::track_repo_action(
            repo_root,
            crate::telemetry::analytics::ActionDescriptor {
                event: "bitloops dashboard api checkpoint".to_string(),
                surface: "dashboard",
                properties,
            },
            status.is_success(),
            started.elapsed(),
        );
    }

    response
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

async fn load_checkpoint_detail(
    repo_root: &Path,
    repo_id: &str,
    checkpoint_id: String,
) -> std::result::Result<ApiCheckpointDetailResponse, ApiError> {
    let checkpoint_id = normalize_checkpoint_id(checkpoint_id)?;
    let Some(info) =
        read_checkpoint_info_for_filtering(repo_root, &checkpoint_id).map_err(|err| {
            ApiError::internal(format!(
                "failed to read checkpoint metadata for {checkpoint_id}: {err:#}"
            ))
        })?
    else {
        return Err(ApiError::not_found(format!(
            "checkpoint not found: {checkpoint_id}"
        )));
    };

    let checkpoint_file_relations =
        load_checkpoint_file_relations(repo_root, repo_id, &checkpoint_id)
            .await
            .map_err(|err| {
                ApiError::internal(format!(
                    "failed to read checkpoint file provenance for {checkpoint_id}: {err:#}"
                ))
            })?;

    let mut sessions = Vec::new();
    for session_index in 0..info.session_count {
        let content = match read_session_content(repo_root, &checkpoint_id, session_index) {
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
        repo_root,
        &info.branch,
        &info.checkpoint_id,
        &checkpoint_file_relations,
        &info.files_touched,
    );
    let token_usage = api_token_usage_from_committed(&info);
    Ok(ApiCheckpointDetailResponse {
        checkpoint_id: info.checkpoint_id,
        strategy: info.strategy,
        branch: info.branch,
        checkpoints_count: info.checkpoints_count,
        files_touched,
        session_count: info.session_count,
        token_usage,
        sessions,
    })
}

fn resolve_checkpoint_files_touched(
    repo_root: &Path,
    branch: &str,
    checkpoint_id: &str,
    file_relations: &[CheckpointFileProvenanceDetailRow],
    fallback_files_touched: &[String],
) -> Vec<super::super::dto::ApiCommitFileDiffDto> {
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
            return fallback_checkpoint_files_touched(file_relations, fallback_files_touched);
        }
    };

    let Some(commit_sha) = branch_commits
        .into_iter()
        .find(|commit| commit.checkpoint_id == checkpoint_id)
        .map(|commit| commit.sha)
    else {
        return fallback_checkpoint_files_touched(file_relations, fallback_files_touched);
    };

    match read_commit_numstat(repo_root, &commit_sha) {
        Ok(stats) => {
            if file_relations.is_empty() {
                api_file_diff_list_from_numstat(stats)
            } else {
                api_checkpoint_file_diff_list_from_relations(file_relations, Some(&stats))
            }
        }
        Err(err) => {
            log::warn!(
                "dashboard checkpoint endpoint: failed to read numstat for {} (checkpoint {}): {:#}",
                commit_sha,
                checkpoint_id,
                err
            );
            fallback_checkpoint_files_touched(file_relations, fallback_files_touched)
        }
    }
}

fn fallback_checkpoint_files_touched(
    file_relations: &[CheckpointFileProvenanceDetailRow],
    fallback_files_touched: &[String],
) -> Vec<super::super::dto::ApiCommitFileDiffDto> {
    if file_relations.is_empty() {
        api_zeroed_file_diff_list(fallback_files_touched)
    } else {
        api_checkpoint_file_diff_list_from_relations(file_relations, None)
    }
}

async fn load_checkpoint_file_relations(
    repo_root: &Path,
    repo_id: &str,
    checkpoint_id: &str,
) -> anyhow::Result<Vec<CheckpointFileProvenanceDetailRow>> {
    let sqlite_path =
        crate::host::checkpoints::strategy::manual_commit::resolve_temporary_checkpoint_sqlite_path(
            repo_root,
        )?;
    if !sqlite_path.is_file() {
        return Ok(Vec::new());
    }

    let relational = RelationalStorage::local_only(sqlite_path);
    CheckpointFileGateway::new(&relational)
        .list_checkpoint_files(repo_id, checkpoint_id)
        .await
}
