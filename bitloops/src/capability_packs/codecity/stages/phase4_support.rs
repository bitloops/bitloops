use anyhow::Result;
use serde_json::Value;

use crate::capability_packs::codecity::services::config::CodeCityConfig;
use crate::capability_packs::codecity::storage::{
    SqliteCodeCityRepository, missing_snapshot_status, normalise_project_path, snapshot_key_for,
};
use crate::capability_packs::codecity::types::{
    CodeCityPhase4Snapshot, CodeCitySnapshotStatus, CodeCityWorldPayload,
    codecity_current_scope_required_stage_response,
};
use crate::host::capability_host::{CapabilityExecutionContext, StageRequest, StageResponse};

pub(super) struct CodeCityPhase4StageData {
    pub repo_id: String,
    pub snapshot_status: CodeCitySnapshotStatus,
    pub world: Option<CodeCityWorldPayload>,
    pub snapshot: CodeCityPhase4Snapshot,
    pub config: CodeCityConfig,
}

pub(super) fn build_phase4_stage_data(
    stage_id: &str,
    request: &StageRequest,
    ctx: &mut dyn CapabilityExecutionContext,
    config: CodeCityConfig,
) -> Result<Result<CodeCityPhase4StageData, StageResponse>> {
    let repo_id = repo_id_from_request(request, ctx);
    if resolved_commit_from_request(request).is_some() {
        return Ok(Err(codecity_current_scope_required_stage_response(
            stage_id,
        )));
    }

    let project_path = project_path_from_request(request);
    let snapshot_key = snapshot_key_for(project_path.as_deref());
    let latest_generation = crate::daemon::capability_event_latest_generation(&repo_id)
        .ok()
        .flatten();
    let config_fingerprint = config.fingerprint()?;
    let repo = SqliteCodeCityRepository::open_for_repo_root(ctx.repo_root()).and_then(|repo| {
        repo.initialise_schema()?;
        Ok(repo)
    })?;

    let Some(mut stored) =
        repo.load_codecity_snapshot(&repo_id, &snapshot_key, latest_generation)?
    else {
        let status = missing_snapshot_status(
            &repo_id,
            &snapshot_key,
            project_path.as_deref(),
            &config_fingerprint,
        );
        return Ok(Ok(CodeCityPhase4StageData {
            repo_id: repo_id.clone(),
            snapshot_status: status,
            world: None,
            snapshot: empty_phase4_snapshot(&repo_id),
            config,
        }));
    };

    if let Some(world) = stored.world.as_mut() {
        world.snapshot_status = stored.status.clone();
    }

    Ok(Ok(CodeCityPhase4StageData {
        repo_id,
        snapshot_status: stored.status,
        world: stored.world,
        snapshot: stored.phase4,
        config,
    }))
}

pub(super) fn repo_id_from_request(
    request: &StageRequest,
    ctx: &mut dyn CapabilityExecutionContext,
) -> String {
    request
        .payload
        .get("query_context")
        .and_then(|query_context| query_context.get("repo_id"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(ctx.repo().repo_id.as_str())
        .to_string()
}

pub(super) fn resolved_commit_from_request(request: &StageRequest) -> Option<String> {
    request
        .payload
        .get("query_context")
        .and_then(|query_context| query_context.get("resolved_commit_sha"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

pub(super) fn project_path_from_request(request: &StageRequest) -> Option<String> {
    normalise_project_path(
        request
            .payload
            .get("query_context")
            .and_then(|query_context| query_context.get("project_path"))
            .and_then(Value::as_str),
    )
}

pub(super) fn empty_phase4_snapshot(repo_id: &str) -> CodeCityPhase4Snapshot {
    CodeCityPhase4Snapshot {
        repo_id: repo_id.to_string(),
        run_id: String::new(),
        commit_sha: None,
        evidence: Vec::new(),
        file_arcs: Vec::new(),
        violations: Vec::new(),
        render_arcs: Vec::new(),
        diagnostics: Vec::new(),
    }
}
