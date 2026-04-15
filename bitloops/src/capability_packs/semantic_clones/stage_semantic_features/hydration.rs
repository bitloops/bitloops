use std::collections::{BTreeSet, HashMap};
use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use serde_json::Value;

use super::storage::{
    build_current_repo_artefacts_sql, build_historical_repo_artefacts_sql,
    build_semantic_get_artefacts_by_ids_sql, build_semantic_get_artefacts_sql,
    build_semantic_get_dependencies_sql, parse_semantic_artefact_rows,
    parse_semantic_dependency_rows,
};
use crate::capability_packs::semantic_clones::features as semantic;
use crate::host::checkpoints::strategy::manual_commit::run_git;
use crate::host::devql::sync::content_cache::lookup_cached_content;
use crate::host::devql::sync::semantic_projector::{
    pre_stage_artefacts_for_projection, pre_stage_dependencies_for_projection,
};
use crate::host::devql::sync::types::{DesiredFileState, EffectiveSource};
use crate::host::devql::{self, RelationalStorage, esc_pg};

pub(crate) async fn load_pre_stage_artefacts_for_blob(
    relational: &RelationalStorage,
    repo_id: &str,
    blob_sha: &str,
    path: &str,
) -> Result<Vec<semantic::PreStageArtefactRow>> {
    let rows = relational
        .query_rows(&build_semantic_get_artefacts_sql(repo_id, blob_sha, path))
        .await?;
    parse_semantic_artefact_rows(rows)
}

pub(crate) async fn load_pre_stage_dependencies_for_blob(
    relational: &RelationalStorage,
    repo_id: &str,
    blob_sha: &str,
    path: &str,
) -> Result<Vec<semantic::PreStageDependencyRow>> {
    let rows = relational
        .query_rows(&build_semantic_get_dependencies_sql(
            repo_id, blob_sha, path,
        ))
        .await?;
    parse_semantic_dependency_rows(rows)
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(super) struct CurrentSemanticArtefactKey {
    path: String,
    canonical_kind: String,
    symbol_fqn: String,
}

pub(super) fn current_semantic_artefact_key_from_row(
    row: &semantic::PreStageArtefactRow,
) -> CurrentSemanticArtefactKey {
    CurrentSemanticArtefactKey {
        path: row.path.clone(),
        canonical_kind: row.canonical_kind.to_ascii_lowercase(),
        symbol_fqn: row.symbol_fqn.clone(),
    }
}

fn current_semantic_artefact_key_from_input(
    input: &semantic::SemanticFeatureInput,
) -> CurrentSemanticArtefactKey {
    CurrentSemanticArtefactKey {
        path: input.path.clone(),
        canonical_kind: input.canonical_kind.to_ascii_lowercase(),
        symbol_fqn: input.symbol_fqn.clone(),
    }
}

pub(super) fn remap_semantic_input_to_current_artefact(
    input: semantic::SemanticFeatureInput,
    current_by_key: &HashMap<CurrentSemanticArtefactKey, semantic::PreStageArtefactRow>,
) -> Option<semantic::SemanticFeatureInput> {
    let current = current_by_key.get(&current_semantic_artefact_key_from_input(&input))?;
    let mut remapped = input;
    remapped.artefact_id = current.artefact_id.clone();
    remapped.symbol_id = current.symbol_id.clone();
    Some(remapped)
}

pub(crate) async fn load_semantic_feature_inputs_for_artefacts(
    relational: &RelationalStorage,
    repo_root: &Path,
    artefact_ids: &[String],
) -> Result<Vec<semantic::SemanticFeatureInput>> {
    if artefact_ids.is_empty() {
        return Ok(Vec::new());
    }

    let requested_order = artefact_ids
        .iter()
        .enumerate()
        .map(|(index, artefact_id)| (artefact_id.clone(), index))
        .collect::<HashMap<_, _>>();
    let requested_ids = artefact_ids.iter().cloned().collect::<BTreeSet<_>>();

    let target_rows = relational
        .query_rows(&build_semantic_get_artefacts_by_ids_sql(artefact_ids))
        .await?;
    let target_artefacts = parse_semantic_artefact_rows(target_rows)?;
    hydrate_semantic_feature_inputs(
        relational,
        repo_root,
        target_artefacts,
        &requested_ids,
        &requested_order,
    )
    .await
}

pub(crate) async fn load_semantic_feature_inputs_for_current_repo(
    relational: &RelationalStorage,
    repo_root: &Path,
    repo_id: &str,
) -> Result<Vec<semantic::SemanticFeatureInput>> {
    let current_paths = load_current_projection_path_states(relational, repo_id).await?;
    let current_cfg = current_projection_cfg(repo_root, repo_id);
    let mut hydrated_inputs = Vec::new();

    for state in &current_paths {
        let Some(extraction) = lookup_cached_content(
            relational,
            &state.effective_content_id,
            &state.language,
            &state.extraction_fingerprint,
            &state.parser_version,
            &state.extractor_version,
        )
        .await?
        else {
            continue;
        };

        let content = load_current_projection_content(repo_root, state)?;
        let desired = desired_file_state_from_current_projection(state);
        let artefacts = pre_stage_artefacts_for_projection(&current_cfg, &desired, &extraction)?;
        let dependencies =
            pre_stage_dependencies_for_projection(&current_cfg, &desired, &extraction)?;
        hydrated_inputs.extend(
            semantic::build_semantic_feature_inputs_from_artefacts_with_dependencies(
                &artefacts,
                &dependencies,
                &content,
            ),
        );
    }

    if !hydrated_inputs.is_empty() {
        hydrated_inputs.sort_by(|left, right| {
            left.path
                .cmp(&right.path)
                .then(left.symbol_fqn.cmp(&right.symbol_fqn))
                .then(left.artefact_id.cmp(&right.artefact_id))
        });
        hydrated_inputs.dedup_by(|left, right| left.artefact_id == right.artefact_id);
        return Ok(hydrated_inputs);
    }

    load_semantic_feature_inputs_for_current_repo_from_historical(relational, repo_root, repo_id)
        .await
}

pub(crate) async fn load_semantic_feature_inputs_for_historical_repo(
    relational: &RelationalStorage,
    repo_root: &Path,
    repo_id: &str,
) -> Result<Vec<semantic::SemanticFeatureInput>> {
    let target_rows = relational
        .query_rows(&build_historical_repo_artefacts_sql(repo_id))
        .await?;
    let target_artefacts = parse_semantic_artefact_rows(target_rows)?;
    let requested_order = target_artefacts
        .iter()
        .enumerate()
        .map(|(index, row)| (row.artefact_id.clone(), index))
        .collect::<HashMap<_, _>>();
    let requested_ids = target_artefacts
        .iter()
        .map(|row| row.artefact_id.clone())
        .collect::<BTreeSet<_>>();

    hydrate_semantic_feature_inputs(
        relational,
        repo_root,
        target_artefacts,
        &requested_ids,
        &requested_order,
    )
    .await
}

async fn hydrate_semantic_feature_inputs(
    relational: &RelationalStorage,
    repo_root: &Path,
    target_artefacts: Vec<semantic::PreStageArtefactRow>,
    requested_ids: &BTreeSet<String>,
    requested_order: &HashMap<String, usize>,
) -> Result<Vec<semantic::SemanticFeatureInput>> {
    let groups = target_artefacts
        .iter()
        .map(|row| (row.repo_id.clone(), row.blob_sha.clone(), row.path.clone()))
        .collect::<BTreeSet<_>>();

    let mut hydrated_inputs = Vec::with_capacity(requested_ids.len());
    for (repo_id, blob_sha, path) in groups {
        let artefacts =
            load_pre_stage_artefacts_for_blob(relational, &repo_id, &blob_sha, &path).await?;
        let dependencies =
            load_pre_stage_dependencies_for_blob(relational, &repo_id, &blob_sha, &path).await?;
        let blob_content = load_blob_content_from_git(repo_root, &blob_sha)
            .with_context(|| format!("loading blob `{blob_sha}` for `{path}`"))?;

        hydrated_inputs.extend(
            semantic::build_semantic_feature_inputs_from_artefacts_with_dependencies(
                &artefacts,
                &dependencies,
                &blob_content,
            )
            .into_iter()
            .filter(|input| requested_ids.contains(&input.artefact_id)),
        );
    }

    hydrated_inputs.sort_by_key(|input| {
        requested_order
            .get(&input.artefact_id)
            .copied()
            .unwrap_or(usize::MAX)
    });
    hydrated_inputs.dedup_by(|left, right| left.artefact_id == right.artefact_id);
    Ok(hydrated_inputs)
}

#[derive(Debug, Clone)]
struct CurrentProjectionPathState {
    path: String,
    language: String,
    extraction_fingerprint: String,
    head_content_id: Option<String>,
    index_content_id: Option<String>,
    worktree_content_id: Option<String>,
    effective_content_id: String,
    effective_source: EffectiveSource,
    parser_version: String,
    extractor_version: String,
}

async fn load_semantic_feature_inputs_for_current_repo_from_historical(
    relational: &RelationalStorage,
    repo_root: &Path,
    repo_id: &str,
) -> Result<Vec<semantic::SemanticFeatureInput>> {
    let target_rows = relational
        .query_rows(&build_current_repo_artefacts_sql(repo_id))
        .await?;
    let target_artefacts = parse_semantic_artefact_rows(target_rows)?;
    let current_by_key = target_artefacts
        .iter()
        .map(|row| (current_semantic_artefact_key_from_row(row), row.clone()))
        .collect::<HashMap<_, _>>();
    let requested_order = target_artefacts
        .iter()
        .enumerate()
        .map(|(index, row)| (row.artefact_id.clone(), index))
        .collect::<HashMap<_, _>>();
    let groups = target_artefacts
        .iter()
        .map(|row| (row.repo_id.clone(), row.blob_sha.clone(), row.path.clone()))
        .collect::<BTreeSet<_>>();

    let mut hydrated_inputs = Vec::with_capacity(target_artefacts.len());
    for (group_repo_id, blob_sha, path) in groups {
        let artefacts =
            load_pre_stage_artefacts_for_blob(relational, &group_repo_id, &blob_sha, &path).await?;
        let dependencies =
            load_pre_stage_dependencies_for_blob(relational, &group_repo_id, &blob_sha, &path)
                .await?;
        let blob_content = load_blob_content_from_git(repo_root, &blob_sha)
            .with_context(|| format!("loading blob `{blob_sha}` for `{path}`"))?;

        hydrated_inputs.extend(
            semantic::build_semantic_feature_inputs_from_artefacts_with_dependencies(
                &artefacts,
                &dependencies,
                &blob_content,
            )
            .into_iter()
            .filter_map(|input| remap_semantic_input_to_current_artefact(input, &current_by_key)),
        );
    }

    hydrated_inputs.sort_by_key(|input| {
        requested_order
            .get(&input.artefact_id)
            .copied()
            .unwrap_or(usize::MAX)
    });
    hydrated_inputs.dedup_by(|left, right| left.artefact_id == right.artefact_id);
    Ok(hydrated_inputs)
}

async fn load_current_projection_path_states(
    relational: &RelationalStorage,
    repo_id: &str,
) -> Result<Vec<CurrentProjectionPathState>> {
    let rows = relational
        .query_rows(&format!(
            "SELECT path, language, extraction_fingerprint, head_content_id, index_content_id, worktree_content_id, effective_content_id, effective_source, parser_version, extractor_version \
FROM current_file_state \
WHERE repo_id = '{repo_id}' AND analysis_mode = 'code' \
ORDER BY path",
            repo_id = esc_pg(repo_id),
        ))
        .await?;

    let mut states = Vec::with_capacity(rows.len());
    for row in rows {
        let Some(path) = row.get("path").and_then(Value::as_str).map(str::to_string) else {
            continue;
        };
        let Some(language) = row
            .get("language")
            .and_then(Value::as_str)
            .map(str::to_string)
        else {
            continue;
        };
        let Some(extraction_fingerprint) = row
            .get("extraction_fingerprint")
            .and_then(Value::as_str)
            .map(str::to_string)
        else {
            continue;
        };
        let Some(effective_content_id) = row
            .get("effective_content_id")
            .and_then(Value::as_str)
            .map(str::to_string)
        else {
            continue;
        };
        let Some(effective_source) = row
            .get("effective_source")
            .and_then(Value::as_str)
            .and_then(parse_effective_source)
        else {
            continue;
        };
        let Some(parser_version) = row
            .get("parser_version")
            .and_then(Value::as_str)
            .map(str::to_string)
        else {
            continue;
        };
        let Some(extractor_version) = row
            .get("extractor_version")
            .and_then(Value::as_str)
            .map(str::to_string)
        else {
            continue;
        };

        states.push(CurrentProjectionPathState {
            path,
            language,
            extraction_fingerprint,
            head_content_id: row
                .get("head_content_id")
                .and_then(Value::as_str)
                .map(str::to_string),
            index_content_id: row
                .get("index_content_id")
                .and_then(Value::as_str)
                .map(str::to_string),
            worktree_content_id: row
                .get("worktree_content_id")
                .and_then(Value::as_str)
                .map(str::to_string),
            effective_content_id,
            effective_source,
            parser_version,
            extractor_version,
        });
    }

    Ok(states)
}

fn parse_effective_source(raw: &str) -> Option<EffectiveSource> {
    match raw {
        "head" => Some(EffectiveSource::Head),
        "index" => Some(EffectiveSource::Index),
        "worktree" => Some(EffectiveSource::Worktree),
        _ => None,
    }
}

fn current_projection_cfg(repo_root: &Path, repo_id: &str) -> devql::DevqlConfig {
    devql::DevqlConfig {
        daemon_config_root: repo_root.to_path_buf(),
        repo_root: repo_root.to_path_buf(),
        repo: devql::RepoIdentity {
            provider: String::new(),
            organization: String::new(),
            name: String::new(),
            identity: String::new(),
            repo_id: repo_id.to_string(),
        },
        pg_dsn: None,
        clickhouse_url: String::new(),
        clickhouse_user: None,
        clickhouse_password: None,
        clickhouse_database: String::new(),
    }
}

fn desired_file_state_from_current_projection(
    state: &CurrentProjectionPathState,
) -> DesiredFileState {
    DesiredFileState {
        path: state.path.clone(),
        analysis_mode: devql::AnalysisMode::Code,
        file_role: devql::FileRole::SourceCode,
        text_index_mode: devql::TextIndexMode::None,
        language: state.language.clone(),
        resolved_language: state.language.clone(),
        dialect: None,
        primary_context_id: None,
        secondary_context_ids: Vec::new(),
        frameworks: Vec::new(),
        runtime_profile: None,
        classification_reason: "semantic_clones_current_projection".to_string(),
        context_fingerprint: None,
        extraction_fingerprint: state.extraction_fingerprint.clone(),
        head_content_id: state.head_content_id.clone(),
        index_content_id: state.index_content_id.clone(),
        worktree_content_id: state.worktree_content_id.clone(),
        effective_content_id: state.effective_content_id.clone(),
        effective_source: state.effective_source.clone(),
        exists_in_head: state.head_content_id.is_some(),
        exists_in_index: state.index_content_id.is_some(),
        exists_in_worktree: state.worktree_content_id.is_some(),
    }
}

fn load_current_projection_content(
    repo_root: &Path,
    state: &CurrentProjectionPathState,
) -> Result<String> {
    match state.effective_source {
        EffectiveSource::Head => load_blob_content_from_git(
            repo_root,
            state
                .head_content_id
                .as_deref()
                .unwrap_or(&state.effective_content_id),
        )
        .with_context(|| format!("loading HEAD blob for `{}`", state.path)),
        EffectiveSource::Index => load_blob_content_from_git(
            repo_root,
            state
                .index_content_id
                .as_deref()
                .unwrap_or(&state.effective_content_id),
        )
        .with_context(|| format!("loading index blob for `{}`", state.path)),
        EffectiveSource::Worktree => {
            let raw = fs::read(repo_root.join(&state.path))
                .with_context(|| format!("reading `{}` from worktree", state.path))?;
            String::from_utf8(raw)
                .with_context(|| format!("decoding `{}` from worktree as UTF-8", state.path))
        }
    }
}

fn load_blob_content_from_git(repo_root: &Path, blob_sha: &str) -> Result<String> {
    run_git(repo_root, &["cat-file", "-p", blob_sha])
}
