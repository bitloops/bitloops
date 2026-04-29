use anyhow::Result;
use serde_json::Value;

use crate::capability_packs::codecity::services::architecture_diagnostics::codecity_legends;
use crate::capability_packs::codecity::services::config::CodeCityConfig;
use crate::capability_packs::codecity::storage::{
    SqliteCodeCityRepository, missing_snapshot_status, normalise_project_path, snapshot_key_for,
};
use crate::capability_packs::codecity::types::{
    CODECITY_CAPABILITY_ID, CODECITY_WORLD_STAGE_ID, CodeCityArchitectureDiagnosticsSnapshot,
    CodeCityDependencyConnectionPayload, CodeCityDiagnostic, CodeCityFileArchitectureContext,
    CodeCityFileDetailPayload, CodeCityHealthOverview, CodeCityHealthWeights,
    CodeCityLayoutSummary, CodeCitySnapshotStatus, CodeCityWorldPayload,
    codecity_current_scope_required_stage_response,
};
use crate::host::capability_host::{CapabilityExecutionContext, StageRequest, StageResponse};

pub(super) struct CodeCitySnapshotStageData {
    pub repo_id: String,
    pub project_path: Option<String>,
    pub snapshot_key: String,
    pub snapshot_status: CodeCitySnapshotStatus,
    pub world: Option<CodeCityWorldPayload>,
    pub snapshot: CodeCityArchitectureDiagnosticsSnapshot,
    pub config: CodeCityConfig,
}

pub(super) fn build_snapshot_stage_data(
    stage_id: &str,
    request: &StageRequest,
    ctx: &mut dyn CapabilityExecutionContext,
    config: CodeCityConfig,
) -> Result<Result<CodeCitySnapshotStageData, StageResponse>> {
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
        return Ok(Ok(CodeCitySnapshotStageData {
            repo_id: repo_id.clone(),
            project_path,
            snapshot_key,
            snapshot_status: status,
            world: None,
            snapshot: empty_architecture_diagnostics_snapshot(&repo_id),
            config,
        }));
    };

    if let Some(world) = stored.world.as_mut() {
        world.snapshot_status = stored.status.clone();
    }

    Ok(Ok(CodeCitySnapshotStageData {
        repo_id,
        project_path,
        snapshot_key,
        snapshot_status: stored.status,
        world: stored.world,
        snapshot: stored.architecture_diagnostics,
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

pub(super) fn empty_architecture_diagnostics_snapshot(
    repo_id: &str,
) -> CodeCityArchitectureDiagnosticsSnapshot {
    CodeCityArchitectureDiagnosticsSnapshot {
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

pub(super) fn missing_world_payload(
    repo_id: &str,
    project_path: Option<&str>,
    _snapshot_key: &str,
    status: CodeCitySnapshotStatus,
    config: &CodeCityConfig,
) -> CodeCityWorldPayload {
    CodeCityWorldPayload {
        capability: CODECITY_CAPABILITY_ID.to_string(),
        stage: CODECITY_WORLD_STAGE_ID.to_string(),
        status: "missing".to_string(),
        repo_id: repo_id.to_string(),
        commit_sha: None,
        config_fingerprint: config.fingerprint().unwrap_or_default(),
        snapshot_status: status,
        summary: Default::default(),
        health: CodeCityHealthOverview::not_requested(
            config.health.analysis_window_months,
            CodeCityHealthWeights::from(&config.health),
        ),
        legends: codecity_legends(),
        layout: CodeCityLayoutSummary::default(),
        boundaries: Vec::new(),
        macro_graph: None,
        architecture: None,
        boundary_layouts: Vec::new(),
        buildings: Vec::new(),
        arcs: Vec::new(),
        dependency_arcs: Vec::new(),
        diagnostics: vec![CodeCityDiagnostic {
            code: "codecity.snapshot.missing".to_string(),
            severity: "warning".to_string(),
            message: match project_path {
                Some(path) => format!(
                    "No CodeCity snapshot is available for project `{path}`. Run DevQL sync and refresh Code Atlas."
                ),
                None => "No CodeCity snapshot is available. Run DevQL sync and refresh Code Atlas."
                    .to_string(),
            },
            path: project_path.map(str::to_string),
            boundary_id: None,
        }],
    }
}

pub(super) fn empty_file_detail_payload(
    path: String,
    status: String,
    snapshot_status: CodeCitySnapshotStatus,
) -> CodeCityFileDetailPayload {
    CodeCityFileDetailPayload {
        status,
        path,
        snapshot_status,
        building: None,
        architecture_context: CodeCityFileArchitectureContext {
            boundary_id: None,
            boundary_name: None,
            primary_pattern: None,
        },
        incoming_dependencies: empty_dependency_connection(),
        outgoing_dependencies: empty_dependency_connection(),
        violations: Vec::new(),
        related_arcs: Vec::new(),
    }
}

fn empty_dependency_connection() -> CodeCityDependencyConnectionPayload {
    CodeCityDependencyConnectionPayload {
        total_count: 0,
        edges: Vec::new(),
    }
}
