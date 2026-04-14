//! Stage 1: semantic feature rows (`symbol_semantics`, `symbol_features`) for the semantic_clones pipeline.

mod storage;

use std::collections::{BTreeSet, HashMap};
use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result};
use serde_json::Value;

use self::storage::{
    build_current_projection_targets_by_artefact_ids_sql, build_current_repo_artefacts_sql,
    build_current_semantic_persist_rows_sql, build_delete_current_symbol_features_sql,
    build_delete_current_symbol_semantics_sql, build_historical_repo_artefacts_sql,
    build_semantic_get_artefacts_by_ids_sql, build_semantic_get_artefacts_sql,
    build_semantic_get_dependencies_sql, build_semantic_get_index_state_sql,
    build_semantic_get_summary_sql, build_semantic_persist_rows_sql, parse_semantic_artefact_rows,
    parse_semantic_dependency_rows, parse_semantic_index_state_rows,
    semantic_features_postgres_schema_sql, semantic_features_postgres_upgrade_sql,
    semantic_features_sqlite_schema_sql, upgrade_sqlite_semantic_features_schema,
};
use crate::capability_packs::semantic_clones::features as semantic;
use crate::host::checkpoints::strategy::manual_commit::run_git;
use crate::host::devql::{RelationalStorage, postgres_exec, sqlite_exec_path_allow_create};

#[derive(Debug, Clone, PartialEq, Eq)]
struct CurrentSemanticProjectionTarget {
    path: String,
    content_id: String,
    symbol_id: Option<String>,
}

fn ensure_required_llm_summary_output(
    rows: &semantic::SemanticFeatureRows,
    summary_provider: &dyn semantic::SemanticSummaryProvider,
) -> Result<()> {
    if !summary_provider.requires_model_output() || rows.semantics.is_llm_enriched() {
        return Ok(());
    }

    anyhow::bail!(
        "configured semantic summary provider returned no model-backed summary for artefact `{}`",
        rows.semantics.artefact_id
    );
}

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
    let current_projection_targets = load_current_projection_targets_for_artefacts(
        relational,
        &inputs
            .iter()
            .map(|input| input.artefact_id.clone())
            .collect::<Vec<_>>(),
    )
    .await?;

    for input in inputs {
        let next_input_hash =
            semantic::build_semantic_feature_input_hash(input, summary_provider.as_ref());
        let state = load_semantic_index_state(relational, &input.artefact_id).await?;
        if !semantic::semantic_features_require_reindex(&state, &next_input_hash) {
            stats.skipped += 1;
            continue;
        }

        let input = input.clone();
        let summary_provider_for_row = Arc::clone(&summary_provider);
        let input_for_row = input.clone();
        let rows = tokio::task::spawn_blocking(move || {
            semantic::build_semantic_feature_rows(&input_for_row, summary_provider_for_row.as_ref())
        })
        .await
        .context("building semantic feature rows on blocking worker")?;
        ensure_required_llm_summary_output(&rows, summary_provider.as_ref())?;
        persist_semantic_feature_rows(relational, &rows).await?;
        if let Some(target) = current_projection_targets.get(&input.artefact_id)
            && target.content_id == input.blob_sha
        {
            persist_current_semantic_feature_rows(
                relational,
                target.symbol_id.as_deref(),
                &target.path,
                &target.content_id,
                &rows,
            )
            .await?;
        }
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
        let summary_provider_for_row = Arc::clone(&summary_provider);
        let rows = tokio::task::spawn_blocking(move || {
            semantic::build_semantic_feature_rows(&input, summary_provider_for_row.as_ref())
        })
        .await
        .context("building current semantic feature rows on blocking worker")?;
        ensure_required_llm_summary_output(&rows, summary_provider.as_ref())?;
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

async fn load_semantic_index_state(
    relational: &RelationalStorage,
    artefact_id: &str,
) -> Result<semantic::SemanticFeatureIndexState> {
    let rows = relational
        .query_rows(&build_semantic_get_index_state_sql(artefact_id))
        .await?;
    Ok(parse_semantic_index_state_rows(&rows))
}

async fn load_current_projection_targets_for_artefacts(
    relational: &RelationalStorage,
    artefact_ids: &[String],
) -> Result<HashMap<String, CurrentSemanticProjectionTarget>> {
    if artefact_ids.is_empty() {
        return Ok(HashMap::new());
    }

    let rows = match relational
        .query_rows(&build_current_projection_targets_by_artefact_ids_sql(
            artefact_ids,
        ))
        .await
    {
        Ok(rows) => rows,
        Err(err) => {
            let message = err.to_string();
            if message.contains("no such table: artefacts_current")
                || message.contains("no such table: current_file_state")
            {
                return Ok(HashMap::new());
            }
            return Err(err).context("loading current semantic projection targets");
        }
    };
    let mut targets = HashMap::with_capacity(rows.len());
    for row in rows {
        let Some(artefact_id) = row
            .get("artefact_id")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
        else {
            continue;
        };
        let Some(path) = row
            .get("path")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
        else {
            continue;
        };
        let Some(content_id) = row
            .get("content_id")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
        else {
            continue;
        };
        let symbol_id = row
            .get("symbol_id")
            .and_then(Value::as_str)
            .map(str::to_string);
        targets.insert(
            artefact_id.to_string(),
            CurrentSemanticProjectionTarget {
                path: path.to_string(),
                content_id: content_id.to_string(),
                symbol_id,
            },
        );
    }
    Ok(targets)
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
    use std::sync::Arc;

    use super::{
        SemanticSummarySnapshot, current_semantic_artefact_key_from_row,
        ensure_required_llm_summary_output, remap_semantic_input_to_current_artefact,
        semantic_features_sqlite_schema_sql, upsert_semantic_feature_rows,
    };
    use crate::capability_packs::semantic_clones::features as semantic;
    use crate::capability_packs::semantic_clones::features::SemanticSummaryCandidate;
    use crate::host::devql::{RelationalStorage, sqlite_exec_path_allow_create};
    use serde_json::Value;
    use tempfile::tempdir;

    struct StrictNoopSummaryProvider;

    impl semantic::SemanticSummaryProvider for StrictNoopSummaryProvider {
        fn cache_key(&self) -> String {
            "strict-noop".to_string()
        }

        fn generate(
            &self,
            _input: &semantic::SemanticFeatureInput,
        ) -> Option<SemanticSummaryCandidate> {
            None
        }

        fn requires_model_output(&self) -> bool {
            true
        }
    }

    struct TestSummaryProvider;

    impl semantic::SemanticSummaryProvider for TestSummaryProvider {
        fn cache_key(&self) -> String {
            "provider=ollama:ministral-3:3b".to_string()
        }

        fn generate(
            &self,
            _input: &semantic::SemanticFeatureInput,
        ) -> Option<SemanticSummaryCandidate> {
            Some(SemanticSummaryCandidate {
                summary: "Summarises the symbol.".to_string(),
                confidence: 0.91,
                source_model: Some("ollama:ministral-3:3b".to_string()),
            })
        }

        fn requires_model_output(&self) -> bool {
            true
        }
    }

    async fn sqlite_relational_with_current_projection_schema() -> RelationalStorage {
        let temp = tempdir().expect("temp dir");
        let db_path = temp.path().join("semantic-features.sqlite");
        sqlite_exec_path_allow_create(
            &db_path,
            &format!(
                "{}\nCREATE TABLE artefacts_current (
    repo_id TEXT NOT NULL,
    path TEXT NOT NULL,
    content_id TEXT NOT NULL,
    symbol_id TEXT NOT NULL,
    artefact_id TEXT NOT NULL,
    language TEXT NOT NULL,
    extraction_fingerprint TEXT,
    canonical_kind TEXT,
    language_kind TEXT,
    symbol_fqn TEXT,
    parent_symbol_id TEXT,
    parent_artefact_id TEXT,
    start_line INTEGER NOT NULL,
    end_line INTEGER NOT NULL,
    start_byte INTEGER NOT NULL,
    end_byte INTEGER NOT NULL,
    signature TEXT,
    modifiers TEXT NOT NULL DEFAULT '[]',
    docstring TEXT,
    updated_at TEXT NOT NULL,
    PRIMARY KEY (repo_id, path, symbol_id),
    UNIQUE (repo_id, artefact_id)
);
CREATE TABLE current_file_state (
    repo_id TEXT NOT NULL,
    path TEXT NOT NULL,
    analysis_mode TEXT NOT NULL,
    PRIMARY KEY (repo_id, path)
);",
                semantic_features_sqlite_schema_sql(),
            ),
        )
        .await
        .expect("create sqlite schema");
        std::mem::forget(temp);
        RelationalStorage::local_only(db_path)
    }

    fn sample_semantic_input(artefact_id: &str, blob_sha: &str) -> semantic::SemanticFeatureInput {
        semantic::SemanticFeatureInput {
            artefact_id: artefact_id.to_string(),
            symbol_id: Some(format!("symbol-{artefact_id}")),
            repo_id: "repo-1".to_string(),
            blob_sha: blob_sha.to_string(),
            path: "src/lib.rs".to_string(),
            language: "rust".to_string(),
            canonical_kind: "function".to_string(),
            language_kind: "function".to_string(),
            symbol_fqn: format!("src/lib.rs::{artefact_id}"),
            name: artefact_id.to_string(),
            signature: Some(format!("fn {artefact_id}()")),
            modifiers: vec!["pub".to_string()],
            body: "do_work()".to_string(),
            docstring: Some("Performs work.".to_string()),
            parent_kind: Some("file".to_string()),
            dependency_signals: vec!["calls:worker::do_work".to_string()],
            content_hash: Some(blob_sha.to_string()),
        }
    }

    #[tokio::test]
    async fn historical_semantic_upsert_mirrors_model_backed_rows_into_current_projection() {
        let relational = sqlite_relational_with_current_projection_schema().await;
        relational
            .exec(
                "INSERT INTO artefacts_current (
                    repo_id, path, content_id, symbol_id, artefact_id, language,
                    canonical_kind, language_kind, symbol_fqn, start_line, end_line,
                    start_byte, end_byte, modifiers, updated_at
                ) VALUES (
                    'repo-1', 'src/lib.rs', 'content-1', 'symbol-artefact-1', 'artefact-1', 'rust',
                    'function', 'function', 'src/lib.rs::artefact-1', 1, 3, 0, 24, '[]', datetime('now')
                );
                INSERT INTO current_file_state (repo_id, path, analysis_mode)
                VALUES ('repo-1', 'src/lib.rs', 'code');",
            )
            .await
            .expect("seed current projection rows");

        let stats = upsert_semantic_feature_rows(
            &relational,
            &[sample_semantic_input("artefact-1", "content-1")],
            Arc::new(TestSummaryProvider),
        )
        .await
        .expect("upsert historical semantic rows");

        assert_eq!(stats.upserted, 1);
        let rows = relational
            .query_rows(
                "SELECT summary, llm_summary, source_model, content_id
                 FROM symbol_semantics_current
                 WHERE artefact_id = 'artefact-1'",
            )
            .await
            .expect("load mirrored current summary row");
        assert_eq!(rows.len(), 1);
        assert_eq!(
            rows[0].get("llm_summary").and_then(Value::as_str),
            Some("Summarises the symbol.")
        );
        assert_eq!(
            rows[0].get("source_model").and_then(Value::as_str),
            Some("ollama:ministral-3:3b")
        );
        assert_eq!(
            rows[0].get("content_id").and_then(Value::as_str),
            Some("content-1")
        );
    }

    #[tokio::test]
    async fn historical_semantic_upsert_does_not_overwrite_diverged_current_projection() {
        let relational = sqlite_relational_with_current_projection_schema().await;
        relational
            .exec(
                "INSERT INTO artefacts_current (
                    repo_id, path, content_id, symbol_id, artefact_id, language,
                    canonical_kind, language_kind, symbol_fqn, start_line, end_line,
                    start_byte, end_byte, modifiers, updated_at
                ) VALUES (
                    'repo-1', 'src/lib.rs', 'content-new', 'symbol-artefact-1', 'artefact-1', 'rust',
                    'function', 'function', 'src/lib.rs::artefact-1', 1, 3, 0, 24, '[]', datetime('now')
                );
                INSERT INTO current_file_state (repo_id, path, analysis_mode)
                VALUES ('repo-1', 'src/lib.rs', 'code');",
            )
            .await
            .expect("seed diverged current projection rows");

        let stats = upsert_semantic_feature_rows(
            &relational,
            &[sample_semantic_input("artefact-1", "content-old")],
            Arc::new(TestSummaryProvider),
        )
        .await
        .expect("upsert historical semantic rows");

        assert_eq!(stats.upserted, 1);
        let rows = relational
            .query_rows(
                "SELECT summary, llm_summary, source_model, content_id
                 FROM symbol_semantics_current
                 WHERE artefact_id = 'artefact-1'",
            )
            .await
            .expect("load current summary rows");
        assert!(
            rows.is_empty(),
            "historical summary refresh must not overwrite a newer current projection"
        );
    }

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

    #[test]
    fn strict_summary_provider_rejects_template_only_rows() {
        let rows = semantic::SemanticFeatureRows {
            semantics: semantic::SymbolSemanticsRow {
                artefact_id: "artefact-1".to_string(),
                repo_id: "repo-1".to_string(),
                blob_sha: "blob-1".to_string(),
                docstring_summary: None,
                llm_summary: None,
                template_summary: "Defines the rust source file.".to_string(),
                summary: "Defines the rust source file.".to_string(),
                confidence: 0.35,
                source_model: None,
            },
            features: semantic::build_semantic_feature_rows(
                &semantic::SemanticFeatureInput {
                    artefact_id: "artefact-1".to_string(),
                    symbol_id: Some("symbol-1".to_string()),
                    repo_id: "repo-1".to_string(),
                    blob_sha: "blob-1".to_string(),
                    path: "src/lib.rs".to_string(),
                    language: "rust".to_string(),
                    canonical_kind: "file".to_string(),
                    language_kind: "source_file".to_string(),
                    symbol_fqn: "src/lib.rs".to_string(),
                    name: "lib".to_string(),
                    signature: None,
                    modifiers: Vec::new(),
                    body: "fn main() {}".to_string(),
                    docstring: None,
                    parent_kind: None,
                    dependency_signals: Vec::new(),
                    content_hash: Some("hash-1".to_string()),
                },
                &semantic::NoopSemanticSummaryProvider,
            )
            .features,
            semantic_features_input_hash: "hash-1".to_string(),
        };

        let err = ensure_required_llm_summary_output(&rows, &StrictNoopSummaryProvider)
            .expect_err("strict provider should reject template-only summaries");
        assert!(
            err.to_string()
                .contains("configured semantic summary provider returned no model-backed summary")
        );
    }

    #[test]
    fn strict_summary_provider_accepts_model_backed_rows() {
        let rows = semantic::SemanticFeatureRows {
            semantics: semantic::SymbolSemanticsRow {
                artefact_id: "artefact-1".to_string(),
                repo_id: "repo-1".to_string(),
                blob_sha: "blob-1".to_string(),
                docstring_summary: None,
                llm_summary: Some("Summarises the symbol.".to_string()),
                template_summary: "Defines the rust source file.".to_string(),
                summary: "Defines the rust source file. Summarises the symbol.".to_string(),
                confidence: 0.91,
                source_model: Some("ollama:ministral-3:3b".to_string()),
            },
            features: semantic::build_semantic_feature_rows(
                &semantic::SemanticFeatureInput {
                    artefact_id: "artefact-1".to_string(),
                    symbol_id: Some("symbol-1".to_string()),
                    repo_id: "repo-1".to_string(),
                    blob_sha: "blob-1".to_string(),
                    path: "src/lib.rs".to_string(),
                    language: "rust".to_string(),
                    canonical_kind: "file".to_string(),
                    language_kind: "source_file".to_string(),
                    symbol_fqn: "src/lib.rs".to_string(),
                    name: "lib".to_string(),
                    signature: None,
                    modifiers: Vec::new(),
                    body: "fn main() {}".to_string(),
                    docstring: None,
                    parent_kind: None,
                    dependency_signals: Vec::new(),
                    content_hash: Some("hash-1".to_string()),
                },
                &semantic::NoopSemanticSummaryProvider,
            )
            .features,
            semantic_features_input_hash: "hash-1".to_string(),
        };

        ensure_required_llm_summary_output(&rows, &StrictNoopSummaryProvider)
            .expect("strict provider should accept model-backed summaries");
    }
}
