use anyhow::{Context, Result};
use serde_json::Value;

use crate::host::devql::{RelationalStorage, sql_json_value, sql_now};

use super::rows::{fact_from_row, sql_opt_text, sql_text};
use crate::capability_packs::architecture_graph::roles::taxonomy::ArchitectureArtefactFact;

pub async fn replace_facts_for_paths(
    relational: &RelationalStorage,
    repo_id: &str,
    paths: &[String],
    facts: &[ArchitectureArtefactFact],
) -> Result<usize> {
    let mut statements = Vec::new();
    if !paths.is_empty() {
        statements.push(delete_facts_for_paths_sql(repo_id, paths));
    }
    for fact in facts {
        statements.push(insert_fact_sql(relational, fact));
    }
    if !statements.is_empty() {
        relational
            .exec_serialized_batch_transactional(&statements)
            .await
            .context("replacing architecture role facts for paths")?;
    }
    Ok(facts.len())
}

pub(super) fn insert_fact_sql(
    relational: &RelationalStorage,
    fact: &ArchitectureArtefactFact,
) -> String {
    format!(
        "INSERT INTO architecture_artefact_facts_current (
            repo_id, fact_id, target_kind, artefact_id, symbol_id, path, language,
            fact_kind, fact_key, fact_value, source, confidence, evidence_json,
            generation_seq, updated_at
         ) VALUES (
            {repo_id}, {fact_id}, {target_kind}, {artefact_id}, {symbol_id}, {path}, {language},
            {fact_kind}, {fact_key}, {fact_value}, {source}, {confidence}, {evidence},
            {generation_seq}, {now}
         )
         ON CONFLICT(repo_id, fact_id) DO UPDATE SET
            fact_value = excluded.fact_value,
            source = excluded.source,
            confidence = excluded.confidence,
            evidence_json = excluded.evidence_json,
            generation_seq = excluded.generation_seq,
            updated_at = {now};",
        repo_id = sql_text(&fact.repo_id),
        fact_id = sql_text(&fact.fact_id),
        target_kind = sql_text(fact.target.target_kind.as_db()),
        artefact_id = sql_opt_text(fact.target.artefact_id.as_deref()),
        symbol_id = sql_opt_text(fact.target.symbol_id.as_deref()),
        path = sql_text(&fact.target.path),
        language = sql_opt_text(fact.language.as_deref()),
        fact_kind = sql_text(&fact.fact_kind),
        fact_key = sql_text(&fact.fact_key),
        fact_value = sql_text(&fact.fact_value),
        source = sql_text(&fact.source),
        confidence = fact.confidence,
        evidence = sql_json_value(relational, &fact.evidence),
        generation_seq = fact.generation_seq,
        now = sql_now(relational),
    )
}

pub async fn delete_role_facts_for_paths(
    relational: &RelationalStorage,
    repo_id: &str,
    paths: &[String],
) -> Result<usize> {
    if paths.is_empty() {
        return Ok(0);
    }
    let before = count_role_facts_for_paths(relational, repo_id, paths).await?;
    relational
        .exec_serialized(&delete_facts_for_paths_sql(repo_id, paths))
        .await
        .context("deleting architecture role facts for paths")?;
    Ok(before)
}

pub(super) fn delete_facts_for_paths_sql(repo_id: &str, paths: &[String]) -> String {
    if paths.is_empty() {
        return String::new();
    }
    format!(
        "DELETE FROM architecture_artefact_facts_current
         WHERE repo_id = {} AND path IN ({});",
        sql_text(repo_id),
        paths
            .iter()
            .map(|path| sql_text(path))
            .collect::<Vec<_>>()
            .join(", ")
    )
}

pub async fn count_role_facts_for_paths(
    relational: &RelationalStorage,
    repo_id: &str,
    paths: &[String],
) -> Result<usize> {
    if paths.is_empty() {
        return Ok(0);
    }
    let sql = format!(
        "SELECT COUNT(*) AS count
         FROM architecture_artefact_facts_current
         WHERE repo_id = {} AND path IN ({});",
        sql_text(repo_id),
        paths
            .iter()
            .map(|path| sql_text(path))
            .collect::<Vec<_>>()
            .join(", ")
    );
    let rows = relational.query_rows(&sql).await?;
    let count = rows
        .first()
        .and_then(|row| row.get("count"))
        .and_then(Value::as_i64)
        .unwrap_or(0);
    Ok(usize::try_from(count).unwrap_or(0))
}

pub async fn load_facts_for_paths(
    relational: &RelationalStorage,
    repo_id: &str,
    paths: &[String],
) -> Result<Vec<ArchitectureArtefactFact>> {
    if paths.is_empty() {
        return Ok(Vec::new());
    }
    let sql = format!(
        "SELECT repo_id, fact_id, target_kind, artefact_id, symbol_id, path, language,
                fact_kind, fact_key, fact_value, source, confidence, evidence_json, generation_seq
         FROM architecture_artefact_facts_current
         WHERE repo_id = {} AND path IN ({})
         ORDER BY path ASC, target_kind ASC, fact_kind ASC, fact_key ASC, fact_value ASC",
        sql_text(repo_id),
        paths
            .iter()
            .map(|path| sql_text(path))
            .collect::<Vec<_>>()
            .join(", ")
    );
    relational
        .query_rows(&sql)
        .await
        .context("loading architecture role facts for paths")?
        .into_iter()
        .map(fact_from_row)
        .collect()
}
