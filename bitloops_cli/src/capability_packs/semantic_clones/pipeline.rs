//! Symbol clone edge rebuild orchestration for the **semantic_clones** capability pack.
//!
//! Pure clone scoring lives in [`semantic_clones_pack::build_symbol_clone_edges`]. This module
//! loads candidates from DevQL relational storage, applies pack DDL when needed, and persists
//! edges. **DevQL ingestion** should trigger rebuild only via the registered ingester
//! ([`super::SEMANTIC_CLONES_REBUILD_INGESTER_ID`]) or [`rebuild_symbol_clone_edges`](fn@rebuild_symbol_clone_edges) (also re-exported at `crate::host::devql` under `cfg(test)` for integration tests),
//! not by duplicating this pipeline.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use anyhow::{Context, Result};
use serde_json::Value;
use tokio_postgres::Client;

use crate::host::capability_packs::builtin::semantic_clones as semantic_clones_pack;
use crate::host::devql::{
    EDGE_KIND_CALLS, EDGE_KIND_EXPORTS, RelationalStorage, esc_pg, postgres_exec, sql_json_value,
    sql_now, sqlite_exec_path_allow_create,
};

use super::ensure_semantic_embeddings_schema;
use super::features as semantic;
use super::scoring;

use super::schema::{semantic_clones_postgres_schema_sql, semantic_clones_sqlite_schema_sql};

async fn init_sqlite_semantic_clones_schema(sqlite_path: &Path) -> Result<()> {
    sqlite_exec_path_allow_create(sqlite_path, semantic_clones_sqlite_schema_sql())
        .await
        .context("creating SQLite semantic clone tables")?;
    Ok(())
}

pub(crate) async fn init_postgres_semantic_clones_schema(pg_client: &Client) -> Result<()> {
    postgres_exec(pg_client, semantic_clones_postgres_schema_sql())
        .await
        .context("creating Postgres semantic clone tables")?;
    Ok(())
}

async fn ensure_semantic_clones_schema(relational: &RelationalStorage) -> Result<()> {
    match relational {
        RelationalStorage::Postgres(client) => init_postgres_semantic_clones_schema(client).await,
        RelationalStorage::Sqlite { path } => init_sqlite_semantic_clones_schema(path).await,
    }
}

pub(crate) async fn rebuild_symbol_clone_edges(
    relational: &RelationalStorage,
    repo_id: &str,
) -> Result<scoring::SymbolCloneBuildResult> {
    ensure_semantic_clones_schema(relational).await?;
    ensure_semantic_embeddings_schema(relational).await?;
    let candidates = load_symbol_clone_candidate_inputs(relational, repo_id).await?;
    let build_result = tokio::task::spawn_blocking(move || {
        semantic_clones_pack::build_symbol_clone_edges(&candidates)
    })
    .await
    .context("building semantic clone edges on blocking worker")?;

    delete_repo_symbol_clone_edges(relational, repo_id).await?;
    persist_symbol_clone_edges(relational, &build_result.edges).await?;
    Ok(build_result)
}

async fn load_symbol_clone_candidate_inputs(
    relational: &RelationalStorage,
    repo_id: &str,
) -> Result<Vec<scoring::SymbolCloneCandidateInput>> {
    let churn_by_symbol_id = load_symbol_churn_counts(relational, repo_id).await?;
    let call_targets_by_symbol_id = load_symbol_call_targets(relational, repo_id).await?;
    let dependency_targets_by_symbol_id =
        load_symbol_dependency_targets(relational, repo_id).await?;
    let rows = relational
        .query_rows(&build_symbol_clone_candidate_lookup_sql(repo_id))
        .await?;

    let mut candidates = Vec::with_capacity(rows.len());
    for row in rows {
        let Some(symbol_id) = row.get("symbol_id").and_then(Value::as_str) else {
            continue;
        };
        let Some(artefact_id) = row.get("artefact_id").and_then(Value::as_str) else {
            continue;
        };
        let embedding = parse_json_f32_array(row.get("embedding"));
        if embedding.is_empty() {
            continue;
        }

        candidates.push(scoring::SymbolCloneCandidateInput {
            repo_id: row
                .get("repo_id")
                .and_then(Value::as_str)
                .unwrap_or(repo_id)
                .to_string(),
            symbol_id: symbol_id.to_string(),
            artefact_id: artefact_id.to_string(),
            path: row
                .get("path")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            canonical_kind: row
                .get("canonical_kind")
                .and_then(Value::as_str)
                .unwrap_or("symbol")
                .to_string(),
            symbol_fqn: row
                .get("symbol_fqn")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            summary: row
                .get("summary")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            normalized_name: row
                .get("normalized_name")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            normalized_signature: row
                .get("normalized_signature")
                .and_then(Value::as_str)
                .map(str::to_string),
            identifier_tokens: parse_clone_json_string_array(row.get("identifier_tokens")),
            normalized_body_tokens: parse_clone_json_string_array(
                row.get("normalized_body_tokens"),
            ),
            parent_kind: row
                .get("parent_kind")
                .and_then(Value::as_str)
                .map(str::to_string),
            context_tokens: parse_clone_json_string_array(row.get("context_tokens")),
            embedding,
            call_targets: call_targets_by_symbol_id
                .get(symbol_id)
                .cloned()
                .unwrap_or_default(),
            dependency_targets: dependency_targets_by_symbol_id
                .get(symbol_id)
                .cloned()
                .unwrap_or_default(),
            churn_count: churn_by_symbol_id.get(symbol_id).copied().unwrap_or(0),
        });
    }

    Ok(candidates)
}

async fn load_symbol_churn_counts(
    relational: &RelationalStorage,
    repo_id: &str,
) -> Result<HashMap<String, usize>> {
    let sql = format!(
        "SELECT symbol_id, COUNT(DISTINCT blob_sha) AS churn_count \
FROM artefacts \
WHERE repo_id = '{}' AND symbol_id IS NOT NULL \
GROUP BY symbol_id",
        esc_pg(repo_id),
    );
    let rows = relational.query_rows(&sql).await?;
    let mut out = HashMap::with_capacity(rows.len());
    for row in rows {
        let Some(symbol_id) = row.get("symbol_id").and_then(Value::as_str) else {
            continue;
        };
        let churn = row
            .get("churn_count")
            .and_then(value_as_usize)
            .unwrap_or_default();
        out.insert(symbol_id.to_string(), churn);
    }
    Ok(out)
}

async fn load_symbol_call_targets(
    relational: &RelationalStorage,
    repo_id: &str,
) -> Result<HashMap<String, Vec<String>>> {
    let sql = format!(
        "SELECT e.from_symbol_id, COALESCE(target.symbol_fqn, target.path, e.to_symbol_ref, e.to_symbol_id, '') AS target_ref \
FROM artefact_edges_current e \
LEFT JOIN artefacts_current target ON target.repo_id = e.repo_id AND target.artefact_id = e.to_artefact_id \
WHERE e.repo_id = '{}' AND e.edge_kind = '{}'",
        esc_pg(repo_id),
        esc_pg(EDGE_KIND_CALLS),
    );
    let rows = relational.query_rows(&sql).await?;
    let mut out = HashMap::<String, HashSet<String>>::new();
    for row in rows {
        let Some(from_symbol_id) = row.get("from_symbol_id").and_then(Value::as_str) else {
            continue;
        };
        let Some(target_ref) = row.get("target_ref").and_then(Value::as_str) else {
            continue;
        };
        if target_ref.trim().is_empty() {
            continue;
        }
        out.entry(from_symbol_id.to_string())
            .or_default()
            .insert(target_ref.to_string());
    }

    Ok(out
        .into_iter()
        .map(|(symbol_id, targets)| {
            let mut targets = targets.into_iter().collect::<Vec<_>>();
            targets.sort();
            (symbol_id, targets)
        })
        .collect())
}

async fn load_symbol_dependency_targets(
    relational: &RelationalStorage,
    repo_id: &str,
) -> Result<HashMap<String, Vec<String>>> {
    let sql = format!(
        "SELECT e.from_symbol_id, LOWER(e.edge_kind) AS edge_kind, \
COALESCE(target.symbol_fqn, target.path, e.to_symbol_ref, e.to_symbol_id, '') AS target_ref \
FROM artefact_edges_current e \
LEFT JOIN artefacts_current target ON target.repo_id = e.repo_id AND target.artefact_id = e.to_artefact_id \
WHERE e.repo_id = '{}' AND e.edge_kind <> '{}' AND e.edge_kind <> '{}'",
        esc_pg(repo_id),
        esc_pg(EDGE_KIND_CALLS),
        esc_pg(EDGE_KIND_EXPORTS),
    );
    let rows = relational.query_rows(&sql).await?;
    let mut out = HashMap::<String, HashSet<String>>::new();
    for row in rows {
        let Some(from_symbol_id) = row.get("from_symbol_id").and_then(Value::as_str) else {
            continue;
        };
        let Some(edge_kind) = row.get("edge_kind").and_then(Value::as_str) else {
            continue;
        };
        let Some(target_ref) = row.get("target_ref").and_then(Value::as_str) else {
            continue;
        };
        let Some(signal) = semantic::build_dependency_context_signal(edge_kind, target_ref) else {
            continue;
        };
        out.entry(from_symbol_id.to_string())
            .or_default()
            .insert(signal);
    }

    Ok(out
        .into_iter()
        .map(|(symbol_id, targets)| {
            let mut targets = targets.into_iter().collect::<Vec<_>>();
            targets.sort();
            (symbol_id, targets)
        })
        .collect())
}

fn build_symbol_clone_candidate_lookup_sql(repo_id: &str) -> String {
    format!(
        "SELECT a.repo_id, a.symbol_id, a.artefact_id, a.path, \
LOWER(COALESCE(a.canonical_kind, COALESCE(a.language_kind, 'symbol'))) AS canonical_kind, \
COALESCE(a.symbol_fqn, a.path) AS symbol_fqn, ss.summary, \
sf.normalized_name, sf.normalized_signature, sf.identifier_tokens, sf.normalized_body_tokens, sf.parent_kind, sf.context_tokens, \
e.embedding \
FROM artefacts_current a \
JOIN symbol_semantics ss ON ss.artefact_id = a.artefact_id \
JOIN symbol_features sf ON sf.artefact_id = a.artefact_id \
JOIN symbol_embeddings e ON e.artefact_id = a.artefact_id \
WHERE a.repo_id = '{}' \
ORDER BY a.path, a.start_line, a.symbol_id",
        esc_pg(repo_id),
    )
}

async fn delete_repo_symbol_clone_edges(
    relational: &RelationalStorage,
    repo_id: &str,
) -> Result<()> {
    let sql = format!(
        "DELETE FROM symbol_clone_edges WHERE repo_id = '{}'",
        esc_pg(repo_id),
    );
    relational.exec(&sql).await
}

async fn persist_symbol_clone_edges(
    relational: &RelationalStorage,
    rows: &[scoring::SymbolCloneEdgeRow],
) -> Result<()> {
    for row in rows {
        let explanation_expr = sql_json_value(relational, &row.explanation_json);
        let generated_at = sql_now(relational);
        let sql = format!(
            "INSERT INTO symbol_clone_edges (repo_id, source_symbol_id, source_artefact_id, target_symbol_id, target_artefact_id, relation_kind, score, semantic_score, lexical_score, structural_score, clone_input_hash, explanation_json) \
VALUES ('{repo_id}', '{source_symbol_id}', '{source_artefact_id}', '{target_symbol_id}', '{target_artefact_id}', '{relation_kind}', {score}, {semantic_score}, {lexical_score}, {structural_score}, '{clone_input_hash}', {explanation_json}) \
ON CONFLICT (repo_id, source_symbol_id, target_symbol_id) DO UPDATE SET source_artefact_id = EXCLUDED.source_artefact_id, target_artefact_id = EXCLUDED.target_artefact_id, relation_kind = EXCLUDED.relation_kind, score = EXCLUDED.score, semantic_score = EXCLUDED.semantic_score, lexical_score = EXCLUDED.lexical_score, structural_score = EXCLUDED.structural_score, clone_input_hash = EXCLUDED.clone_input_hash, explanation_json = EXCLUDED.explanation_json, generated_at = {generated_at}",
            repo_id = esc_pg(&row.repo_id),
            source_symbol_id = esc_pg(&row.source_symbol_id),
            source_artefact_id = esc_pg(&row.source_artefact_id),
            target_symbol_id = esc_pg(&row.target_symbol_id),
            target_artefact_id = esc_pg(&row.target_artefact_id),
            relation_kind = esc_pg(&row.relation_kind),
            score = row.score,
            semantic_score = row.semantic_score,
            lexical_score = row.lexical_score,
            structural_score = row.structural_score,
            clone_input_hash = esc_pg(&row.clone_input_hash),
            explanation_json = explanation_expr,
            generated_at = generated_at,
        );
        relational.exec(&sql).await?;
    }
    Ok(())
}

fn parse_clone_json_string_array(value: Option<&Value>) -> Vec<String> {
    match value {
        Some(Value::Array(values)) => values
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect(),
        Some(Value::String(raw)) => serde_json::from_str::<Vec<String>>(raw).unwrap_or_default(),
        _ => Vec::new(),
    }
}

fn parse_json_f32_array(value: Option<&Value>) -> Vec<f32> {
    match value {
        Some(Value::Array(values)) => values
            .iter()
            .filter_map(Value::as_f64)
            .map(|value| value as f32)
            .filter(|value| value.is_finite())
            .collect(),
        Some(Value::String(raw)) => serde_json::from_str::<Vec<f32>>(raw)
            .unwrap_or_default()
            .into_iter()
            .filter(|value| value.is_finite())
            .collect(),
        _ => Vec::new(),
    }
}

fn value_as_usize(value: &Value) -> Option<usize> {
    if let Some(value) = value.as_u64() {
        return usize::try_from(value).ok();
    }
    if let Some(value) = value.as_i64() {
        return usize::try_from(value).ok();
    }
    value.as_str()?.trim().parse::<usize>().ok()
}

#[cfg(test)]
mod semantic_clone_pipeline_tests {
    use super::super::schema::{
        semantic_clones_postgres_schema_sql, semantic_clones_sqlite_schema_sql,
    };

    use super::build_symbol_clone_candidate_lookup_sql;

    #[test]
    fn semantic_clone_schema_includes_clone_edge_table() {
        let pg = semantic_clones_postgres_schema_sql();
        let sqlite = semantic_clones_sqlite_schema_sql();

        assert!(pg.contains("CREATE TABLE IF NOT EXISTS symbol_clone_edges"));
        assert!(sqlite.contains("CREATE TABLE IF NOT EXISTS symbol_clone_edges"));
        assert!(pg.contains("PRIMARY KEY (repo_id, source_symbol_id, target_symbol_id)"));
    }

    #[test]
    fn semantic_clone_candidate_lookup_sql_loads_all_indexed_candidates() {
        let sql = build_symbol_clone_candidate_lookup_sql("repo'1");

        assert!(sql.contains("FROM artefacts_current a"));
        assert!(sql.contains("JOIN symbol_embeddings e"));
        assert!(sql.contains("repo''1"));
        assert!(!sql.contains(" IN ("));
    }
}
