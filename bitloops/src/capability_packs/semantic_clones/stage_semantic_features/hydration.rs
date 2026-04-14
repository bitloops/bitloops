use std::collections::{BTreeSet, HashMap};
use std::path::Path;

use anyhow::{Context, Result};

use super::storage::{
    build_current_repo_artefacts_sql, build_historical_repo_artefacts_sql,
    build_semantic_get_artefacts_by_ids_sql, build_semantic_get_artefacts_sql,
    build_semantic_get_dependencies_sql, parse_semantic_artefact_rows,
    parse_semantic_dependency_rows,
};
use crate::capability_packs::semantic_clones::features as semantic;
use crate::host::checkpoints::strategy::manual_commit::run_git;
use crate::host::devql::RelationalStorage;

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

fn load_blob_content_from_git(repo_root: &Path, blob_sha: &str) -> Result<String> {
    run_git(repo_root, &["cat-file", "-p", blob_sha])
}
