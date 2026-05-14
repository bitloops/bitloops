use std::path::Path;

use crate::api::dashboard_file_diffs::{
    dashboard_checkpoint_file_diff_list_from_relations, dashboard_file_diff_list_from_numstat,
    dashboard_zeroed_file_diff_list,
};
use crate::api::dashboard_params::normalize_checkpoint_id;
use crate::adapters::agents::AgentRegistry;
use crate::api::dashboard_types::{
    DashboardCheckpointDetail, DashboardCheckpointSessionDetail, DashboardCommitFileDiff,
    DashboardTokenUsage, DashboardTranscriptEntry,
};
use crate::api::{
    API_GIT_SCAN_LIMIT, ApiError, DashboardState, canonical_agent_key,
    read_checkpoint_info_for_filtering, read_commit_numstat, walk_branch_commits_with_checkpoints,
};
use crate::host::checkpoints::strategy::manual_commit::{CommittedInfo, read_session_content};
use crate::host::devql::checkpoint_provenance::{
    CheckpointFileGateway, CheckpointFileProvenanceDetailRow,
};

use super::repository::{resolve_dashboard_repo_root, resolve_dashboard_repo_selector};

pub(in crate::api) async fn load_dashboard_checkpoint(
    state: &DashboardState,
    repo_id: Option<String>,
    checkpoint_id: String,
) -> std::result::Result<DashboardCheckpointDetail, ApiError> {
    let repo_selector = resolve_dashboard_repo_selector(state, repo_id.as_deref()).await?;
    let repo_root = resolve_dashboard_repo_root(state, repo_id.as_deref()).await?;
    load_checkpoint_detail(&repo_root, repo_selector.as_str(), checkpoint_id).await
}

fn dashboard_token_usage_from_committed(info: &CommittedInfo) -> Option<DashboardTokenUsage> {
    info.token_usage.as_ref().map(|usage| DashboardTokenUsage {
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
) -> std::result::Result<DashboardCheckpointDetail, ApiError> {
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
                    "dashboard checkpoint query skipped unreadable session {}#{}: {:#}",
                    checkpoint_id,
                    session_index,
                    err
                );
                continue;
            }
        };

        let metadata = content.metadata;
        let session_id_str = metadata
            .get("session_id")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_string();
        let agent_key = metadata
            .get("agent")
            .and_then(serde_json::Value::as_str)
            .map(canonical_agent_key)
            .unwrap_or_default();

        // Derive canonical transcript entries via the agent's deriver. Falls
        // through to an empty vec when the agent is unknown, has no deriver,
        // or the transcript can't be parsed — `transcript_jsonl` is still
        // returned for debug/export use either way.
        let transcript_entries: Vec<DashboardTranscriptEntry> = AgentRegistry::builtin()
            .get_by_agent_type(&agent_key)
            .ok()
            .and_then(|agent| agent.as_transcript_entry_deriver())
            .and_then(|deriver| {
                deriver
                    .derive_transcript_entries(&session_id_str, None, &content.transcript)
                    .ok()
            })
            .unwrap_or_default()
            .into_iter()
            .map(DashboardTranscriptEntry::from)
            .collect();

        sessions.push(DashboardCheckpointSessionDetail {
            session_index,
            session_id: session_id_str,
            agent: agent_key,
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
            transcript_entries,
        });
    }

    let files_touched = resolve_checkpoint_files_touched(
        repo_root,
        &info.branch,
        &info.checkpoint_id,
        &checkpoint_file_relations,
        &info.files_touched,
    );
    let token_usage = dashboard_token_usage_from_committed(&info);
    Ok(DashboardCheckpointDetail {
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
) -> Vec<DashboardCommitFileDiff> {
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
                "dashboard checkpoint query: failed to walk branch {} while resolving files_touched for {}: {:#}",
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
                dashboard_file_diff_list_from_numstat(stats)
            } else {
                dashboard_checkpoint_file_diff_list_from_relations(file_relations, Some(&stats))
            }
        }
        Err(err) => {
            log::warn!(
                "dashboard checkpoint query: failed to read numstat for {} (checkpoint {}): {:#}",
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
) -> Vec<DashboardCommitFileDiff> {
    if file_relations.is_empty() {
        dashboard_zeroed_file_diff_list(fallback_files_touched)
    } else {
        dashboard_checkpoint_file_diff_list_from_relations(file_relations, None)
    }
}

async fn load_checkpoint_file_relations(
    repo_root: &Path,
    repo_id: &str,
    checkpoint_id: &str,
) -> anyhow::Result<Vec<CheckpointFileProvenanceDetailRow>> {
    let relational_store =
        crate::host::relational_store::DefaultRelationalStore::open_local_for_repo_root(repo_root)?;
    let sqlite_path = relational_store.sqlite_path().to_path_buf();
    if !sqlite_path.is_file() {
        return Ok(Vec::new());
    }

    let relational = relational_store.to_local_inner();
    CheckpointFileGateway::new(&relational)
        .list_checkpoint_files(repo_id, checkpoint_id)
        .await
}
