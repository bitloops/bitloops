//! Stage 1: semantic feature rows (`symbol_semantics`, `symbol_features`) for the semantic_clones pipeline.

mod storage;

use std::collections::{BTreeSet, HashMap};
use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result};
use serde_json::Value;

use self::storage::{
    build_current_repo_artefacts_sql, build_current_semantic_persist_rows_sql,
    build_delete_current_symbol_features_sql, build_delete_current_symbol_semantics_sql,
    build_semantic_get_artefacts_by_ids_sql, build_semantic_get_artefacts_sql,
    build_semantic_get_dependencies_sql, build_semantic_get_index_state_sql,
    build_semantic_get_summary_sql, build_semantic_persist_rows_sql,
    build_semantic_persist_summary_sql, parse_semantic_artefact_rows,
    parse_semantic_dependency_rows, parse_semantic_index_state_rows,
    semantic_features_postgres_schema_sql, semantic_features_postgres_upgrade_sql,
    semantic_features_sqlite_schema_sql, upgrade_sqlite_semantic_features_schema,
};
use crate::capability_packs::semantic_clones::features as semantic;
use crate::host::checkpoints::strategy::manual_commit::run_git;
use crate::host::devql::{RelationalStorage, postgres_exec, sqlite_exec_path_allow_create};

pub(crate) async fn init_postgres_semantic_features_schema(
    pg_client: &tokio_postgres::Client,
) -> Result<()> {
    postgres_exec(pg_client, semantic_features_postgres_schema_sql()).await?;
    postgres_exec(pg_client, semantic_features_postgres_upgrade_sql()).await
}

pub(crate) async fn init_sqlite_semantic_features_schema(sqlite_path: &Path) -> Result<()> {
    sqlite_exec_path_allow_create(sqlite_path, semantic_features_sqlite_schema_sql()).await?;
    upgrade_sqlite_semantic_features_schema(sqlite_path).await
}

pub(crate) async fn ensure_semantic_features_schema(relational: &RelationalStorage) -> Result<()> {
    init_sqlite_semantic_features_schema(&relational.local.path).await?;
    if let Some(remote) = relational.remote.as_ref() {
        init_postgres_semantic_features_schema(&remote.client).await?;
    }
    Ok(())
}

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
struct CurrentSemanticArtefactKey {
    path: String,
    canonical_kind: String,
    symbol_fqn: String,
}

fn current_semantic_artefact_key_from_row(
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

fn remap_semantic_input_to_current_artefact(
    input: semantic::SemanticFeatureInput,
    current_by_key: &HashMap<CurrentSemanticArtefactKey, semantic::PreStageArtefactRow>,
) -> Option<semantic::SemanticFeatureInput> {
    let current = current_by_key.get(&current_semantic_artefact_key_from_input(&input))?;
    let mut remapped = input;
    remapped.artefact_id = current.artefact_id.clone();
    remapped.symbol_id = current.symbol_id.clone();
    Some(remapped)
}

pub(crate) async fn upsert_semantic_feature_rows(
    relational: &RelationalStorage,
    inputs: &[semantic::SemanticFeatureInput],
    summary_provider: Arc<dyn semantic::SemanticSummaryProvider>,
) -> Result<semantic::SemanticFeatureIngestionStats> {
    let mut stats = semantic::SemanticFeatureIngestionStats::default();

    for input in inputs {
        let next_input_hash =
            semantic::build_semantic_feature_input_hash(input, summary_provider.as_ref());
        let state = load_semantic_index_state(relational, &input.artefact_id).await?;
        if !semantic::semantic_features_require_reindex(&state, &next_input_hash) {
            stats.skipped += 1;
            continue;
        }

        let input = input.clone();
        let summary_provider = Arc::clone(&summary_provider);
        let rows = tokio::task::spawn_blocking(move || {
            semantic::build_semantic_feature_rows(&input, summary_provider.as_ref())
        })
        .await
        .context("building semantic feature rows on blocking worker")?;
        persist_semantic_feature_rows(relational, &rows).await?;
        stats.upserted += 1;
    }

    Ok(stats)
}

#[allow(dead_code)]
pub(crate) async fn upsert_current_semantic_feature_rows(
    relational: &RelationalStorage,
    path: &str,
    content_id: &str,
    inputs: &[semantic::SemanticFeatureInput],
    summary_provider: Arc<dyn semantic::SemanticSummaryProvider>,
) -> Result<semantic::SemanticFeatureIngestionStats> {
    ensure_semantic_features_schema(relational).await?;
    let Some(first) = inputs.first() else {
        return Ok(semantic::SemanticFeatureIngestionStats::default());
    };

    clear_current_semantic_feature_rows_for_path(relational, &first.repo_id, path).await?;

    let mut stats = semantic::SemanticFeatureIngestionStats::default();
    for input in inputs {
        let symbol_id = input.symbol_id.clone();
        let input = input.clone();
        let summary_provider = Arc::clone(&summary_provider);
        let rows = tokio::task::spawn_blocking(move || {
            semantic::build_semantic_feature_rows(&input, summary_provider.as_ref())
        })
        .await
        .context("building current semantic feature rows on blocking worker")?;
        persist_current_semantic_feature_rows(
            relational,
            symbol_id.as_deref(),
            path,
            content_id,
            &rows,
        )
        .await?;
        stats.upserted += 1;
    }

    Ok(stats)
}

#[allow(dead_code)]
pub(crate) async fn clear_current_semantic_feature_rows_for_path(
    relational: &RelationalStorage,
    repo_id: &str,
    path: &str,
) -> Result<()> {
    ensure_semantic_features_schema(relational).await?;
    relational
        .exec_batch_transactional(&[
            build_delete_current_symbol_features_sql(repo_id, path),
            build_delete_current_symbol_semantics_sql(repo_id, path),
        ])
        .await
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SemanticSummarySnapshot {
    pub semantic_features_input_hash: String,
    pub summary: String,
    pub llm_summary: Option<String>,
    pub source_model: Option<String>,
}

impl SemanticSummarySnapshot {
    pub(crate) fn is_llm_enriched(&self) -> bool {
        self.llm_summary
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty())
            || self
                .source_model
                .as_deref()
                .is_some_and(|value| !value.trim().is_empty())
    }
}

pub(crate) async fn load_semantic_summary_snapshot(
    relational: &RelationalStorage,
    artefact_id: &str,
) -> Result<Option<SemanticSummarySnapshot>> {
    let rows = relational
        .query_rows(&build_semantic_get_summary_sql(artefact_id))
        .await?;
    let Some(row) = rows.first() else {
        return Ok(None);
    };

    let Some(input_hash) = row
        .get("semantic_features_input_hash")
        .and_then(Value::as_str)
        .map(str::to_string)
    else {
        return Ok(None);
    };
    let Some(summary) = row
        .get("summary")
        .and_then(Value::as_str)
        .map(str::to_string)
    else {
        return Ok(None);
    };
    let llm_summary = row
        .get("llm_summary")
        .and_then(Value::as_str)
        .map(str::to_string);
    let source_model = row
        .get("source_model")
        .and_then(Value::as_str)
        .map(str::to_string);

    Ok(Some(SemanticSummarySnapshot {
        semantic_features_input_hash: input_hash,
        summary,
        llm_summary,
        source_model,
    }))
}

pub(crate) async fn persist_semantic_summary_row(
    relational: &RelationalStorage,
    semantics: &semantic::SymbolSemanticsRow,
    semantic_features_input_hash: &str,
) -> Result<()> {
    relational
        .exec(&build_semantic_persist_summary_sql(
            semantics,
            semantic_features_input_hash,
            relational.dialect(),
        )?)
        .await
}

async fn load_semantic_index_state(
    relational: &RelationalStorage,
    artefact_id: &str,
) -> Result<semantic::SemanticFeatureIndexState> {
    let rows = relational
        .query_rows(&build_semantic_get_index_state_sql(artefact_id))
        .await?;
    Ok(parse_semantic_index_state_rows(&rows))
}

async fn persist_semantic_feature_rows(
    relational: &RelationalStorage,
    rows: &semantic::SemanticFeatureRows,
) -> Result<()> {
    relational
        .exec(&build_semantic_persist_rows_sql(
            rows,
            relational.dialect(),
        )?)
        .await
}

#[allow(dead_code)]
async fn persist_current_semantic_feature_rows(
    relational: &RelationalStorage,
    symbol_id: Option<&str>,
    path: &str,
    content_id: &str,
    rows: &semantic::SemanticFeatureRows,
) -> Result<()> {
    relational
        .exec(&build_current_semantic_persist_rows_sql(
            rows,
            symbol_id,
            path,
            content_id,
            relational.dialect(),
        )?)
        .await
}

fn load_blob_content_from_git(repo_root: &Path, blob_sha: &str) -> Result<String> {
    run_git(repo_root, &["cat-file", "-p", blob_sha])
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::{
        SemanticSummarySnapshot, current_semantic_artefact_key_from_row,
        remap_semantic_input_to_current_artefact,
    };
    use crate::capability_packs::semantic_clones::features as semantic;

    #[test]
    fn semantic_summary_snapshot_marks_llm_enrichment_from_summary_or_model() {
        assert!(
            SemanticSummarySnapshot {
                semantic_features_input_hash: "hash-1".to_string(),
                summary: "Function load user.".to_string(),
                llm_summary: Some("Loads a user by id.".to_string()),
                source_model: None,
            }
            .is_llm_enriched()
        );

        assert!(
            SemanticSummarySnapshot {
                semantic_features_input_hash: "hash-1".to_string(),
                summary: "Function load user.".to_string(),
                llm_summary: None,
                source_model: Some("openai:gpt-test".to_string()),
            }
            .is_llm_enriched()
        );

        assert!(
            !SemanticSummarySnapshot {
                semantic_features_input_hash: "hash-1".to_string(),
                summary: "Function load user.".to_string(),
                llm_summary: None,
                source_model: None,
            }
            .is_llm_enriched()
        );
    }

    #[test]
    fn remap_semantic_input_to_current_artefact_uses_current_sync_ids() {
        let current = semantic::PreStageArtefactRow {
            artefact_id: "current-artefact".to_string(),
            symbol_id: Some("current-symbol".to_string()),
            repo_id: "repo-1".to_string(),
            blob_sha: "blob-1".to_string(),
            path: "src/render.ts".to_string(),
            language: "typescript".to_string(),
            canonical_kind: "function".to_string(),
            language_kind: "function".to_string(),
            symbol_fqn: "src/render.ts::renderInvoice".to_string(),
            parent_artefact_id: None,
            start_line: Some(1),
            end_line: Some(3),
            start_byte: Some(0),
            end_byte: Some(64),
            signature: Some("renderInvoice(orderId: string): string".to_string()),
            modifiers: vec!["export".to_string()],
            docstring: None,
            content_hash: Some("hash-1".to_string()),
        };
        let historical = semantic::SemanticFeatureInput {
            artefact_id: "historical-artefact".to_string(),
            symbol_id: Some("historical-symbol".to_string()),
            repo_id: "repo-1".to_string(),
            blob_sha: "blob-1".to_string(),
            path: "src/render.ts".to_string(),
            language: "typescript".to_string(),
            canonical_kind: "function".to_string(),
            language_kind: "function".to_string(),
            symbol_fqn: "src/render.ts::renderInvoice".to_string(),
            name: "renderInvoice".to_string(),
            signature: Some("renderInvoice(orderId: string): string".to_string()),
            modifiers: vec!["export".to_string()],
            body: "return orderId;".to_string(),
            docstring: None,
            parent_kind: Some("file".to_string()),
            dependency_signals: Vec::new(),
            content_hash: Some("hash-1".to_string()),
        };
        let current_by_key = HashMap::from([(
            current_semantic_artefact_key_from_row(&current),
            current.clone(),
        )]);

        let remapped = remap_semantic_input_to_current_artefact(historical, &current_by_key)
            .expect("expected current artefact match");

        assert_eq!(remapped.artefact_id, current.artefact_id);
        assert_eq!(remapped.symbol_id, current.symbol_id);
        assert_eq!(remapped.symbol_fqn, current.symbol_fqn);
    }
}
