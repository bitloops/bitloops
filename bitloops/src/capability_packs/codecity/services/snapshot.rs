use std::path::Path;
use std::sync::Mutex;

use anyhow::{Context, Result};

use super::architecture::analyse_codecity_architecture;
use super::config::CodeCityConfig;
use super::health::apply_health_overlay;
use super::phase4::enrich_world_with_phase4;
use super::source_graph::load_current_source_graph;
use super::world::build_codecity_world_from_analysis;
use crate::capability_packs::codecity::storage::{normalise_project_path, snapshot_key_for};
use crate::capability_packs::codecity::types::{
    CodeCityPhase4Snapshot, CodeCitySnapshotState, CodeCitySnapshotStatus, CodeCityWorldPayload,
};
use crate::capability_packs::test_harness::storage::BitloopsTestHarnessRepository;
use crate::host::capability_host::gateways::{GitHistoryGateway, RelationalGateway};

pub struct BuiltCodeCitySnapshot {
    pub snapshot_key: String,
    pub project_path: Option<String>,
    pub world: CodeCityWorldPayload,
    pub phase4: CodeCityPhase4Snapshot,
}

#[allow(clippy::too_many_arguments)]
pub fn build_codecity_snapshot_from_current_rows(
    relational: &dyn RelationalGateway,
    repo_id: &str,
    repo_root: &Path,
    project_path: Option<&str>,
    config: &CodeCityConfig,
    source_generation_seq: u64,
    git_history: &dyn GitHistoryGateway,
    test_harness_store: Option<&Mutex<BitloopsTestHarnessRepository>>,
) -> Result<BuiltCodeCitySnapshot> {
    let project_path = normalise_project_path(project_path);
    let snapshot_key = snapshot_key_for(project_path.as_deref());
    let source = load_current_source_graph(relational, repo_id, project_path.as_deref(), config)
        .context("loading current DevQL graph for CodeCity snapshot")?;
    let current_head = git_history.resolve_head(repo_root).unwrap_or(None);
    let analysis = analyse_codecity_architecture(&source, config, repo_root);
    let mut world =
        build_codecity_world_from_analysis(&source, repo_id, current_head, config, &analysis)?;
    apply_health_overlay(
        &mut world,
        &source,
        config,
        repo_root,
        git_history,
        test_harness_store,
    )?;
    let phase4 = enrich_world_with_phase4(&source, &analysis, &mut world, config);
    let now = chrono::Utc::now().to_rfc3339();
    world.snapshot_status = CodeCitySnapshotStatus {
        state: CodeCitySnapshotState::Ready,
        stale: false,
        repo_id: repo_id.to_string(),
        project_path: project_path.clone(),
        snapshot_key: snapshot_key.clone(),
        config_fingerprint: world.config_fingerprint.clone(),
        source_generation_seq: Some(source_generation_seq),
        last_success_generation_seq: Some(source_generation_seq),
        run_id: Some(phase4.run_id.clone()),
        commit_sha: world.commit_sha.clone(),
        generated_at: world
            .health
            .generated_at
            .clone()
            .or_else(|| Some(now.clone())),
        updated_at: Some(now),
        last_error: None,
    };

    Ok(BuiltCodeCitySnapshot {
        snapshot_key,
        project_path,
        world,
        phase4,
    })
}
