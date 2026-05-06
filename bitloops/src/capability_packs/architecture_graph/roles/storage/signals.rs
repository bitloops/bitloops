use anyhow::{Context, Result};
use serde_json::Value;

use crate::host::devql::{RelationalStorage, sql_json_value, sql_now};

use super::rows::{sql_opt_text, sql_text};
use crate::capability_packs::architecture_graph::roles::taxonomy::ArchitectureRoleRuleSignal;

pub async fn replace_signals_for_paths(
    relational: &RelationalStorage,
    repo_id: &str,
    paths: &[String],
    signals: &[ArchitectureRoleRuleSignal],
) -> Result<usize> {
    let mut statements = Vec::new();
    if !paths.is_empty() {
        statements.push(delete_signals_for_paths_sql(repo_id, paths));
    }
    for signal in signals {
        statements.push(insert_signal_sql(relational, signal));
    }
    if !statements.is_empty() {
        relational
            .exec_serialized_batch_transactional(&statements)
            .await
            .context("replacing architecture role rule signals")?;
    }
    Ok(signals.len())
}

pub(super) fn delete_signals_for_paths_sql(repo_id: &str, paths: &[String]) -> String {
    if paths.is_empty() {
        return String::new();
    }
    format!(
        "DELETE FROM architecture_role_rule_signals_current
         WHERE repo_id = {} AND path IN ({});",
        sql_text(repo_id),
        paths
            .iter()
            .map(|path| sql_text(path))
            .collect::<Vec<_>>()
            .join(", ")
    )
}

pub async fn count_role_signals_for_paths(
    relational: &RelationalStorage,
    repo_id: &str,
    paths: &[String],
) -> Result<usize> {
    if paths.is_empty() {
        return Ok(0);
    }
    let sql = format!(
        "SELECT COUNT(*) AS count
         FROM architecture_role_rule_signals_current
         WHERE repo_id = {} AND path IN ({});",
        sql_text(repo_id),
        paths
            .iter()
            .map(|path| sql_text(path))
            .collect::<Vec<_>>()
            .join(", ")
    );
    let rows = relational.query_rows(&sql).await?;
    Ok(rows
        .first()
        .and_then(|row| row.get("count"))
        .and_then(Value::as_i64)
        .and_then(|value| usize::try_from(value).ok())
        .unwrap_or(0))
}

pub(super) fn insert_signal_sql(
    relational: &RelationalStorage,
    signal: &ArchitectureRoleRuleSignal,
) -> String {
    format!(
        "INSERT INTO architecture_role_rule_signals_current (
            repo_id, signal_id, rule_id, rule_version, role_id, target_kind,
            artefact_id, symbol_id, path, polarity, score, evidence_json, generation_seq, updated_at
         ) VALUES (
            {repo_id}, {signal_id}, {rule_id}, {rule_version}, {role_id}, {target_kind},
            {artefact_id}, {symbol_id}, {path}, {polarity}, {score}, {evidence}, {generation_seq}, {now}
         )
         ON CONFLICT(repo_id, signal_id) DO UPDATE SET
            score = excluded.score,
            evidence_json = excluded.evidence_json,
            generation_seq = excluded.generation_seq,
            updated_at = {now};",
        repo_id = sql_text(&signal.repo_id),
        signal_id = sql_text(&signal.signal_id),
        rule_id = sql_text(&signal.rule_id),
        rule_version = signal.rule_version,
        role_id = sql_text(&signal.role_id),
        target_kind = sql_text(signal.target.target_kind.as_db()),
        artefact_id = sql_opt_text(signal.target.artefact_id.as_deref()),
        symbol_id = sql_opt_text(signal.target.symbol_id.as_deref()),
        path = sql_text(&signal.target.path),
        polarity = sql_text(signal.polarity.as_db()),
        score = signal.score,
        evidence = sql_json_value(relational, &signal.evidence),
        generation_seq = signal.generation_seq,
        now = sql_now(relational),
    )
}
