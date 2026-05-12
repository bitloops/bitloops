use anyhow::{Context, Result, anyhow};
use serde_json::Value;

use crate::capability_packs::architecture_graph::roles::contracts::{
    RoleAdjudicationAttemptEvent, RoleAdjudicationAttemptWriteResult,
    RoleAdjudicationAttemptWriter, RoleBoxFuture,
};
use crate::host::devql::{RelationalStorage, sql_json_value, sql_now};

use super::rows::{sql_opt_text, sql_text};

pub struct DbRoleAdjudicationAttemptWriter<'a> {
    relational: &'a RelationalStorage,
}

impl<'a> DbRoleAdjudicationAttemptWriter<'a> {
    pub fn new(relational: &'a RelationalStorage) -> Self {
        Self { relational }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct RoleAdjudicationAttemptRecord {
    pub attempt_id: String,
    pub scope_key: String,
    pub generation: u64,
    pub target_kind: Option<String>,
    pub artefact_id: Option<String>,
    pub symbol_id: Option<String>,
    pub path: Option<String>,
    pub reason: String,
    pub outcome: String,
    pub model_descriptor: String,
    pub assignment_write_persisted: bool,
    pub assignment_write_source: Option<String>,
    pub failure_message: Option<String>,
    pub reasoning_summary: Option<String>,
    pub observed_at_unix: u64,
    pub updated_at: Option<String>,
}

impl RoleAdjudicationAttemptWriter for DbRoleAdjudicationAttemptWriter<'_> {
    fn record_attempt<'a>(
        &'a self,
        event: RoleAdjudicationAttemptEvent,
    ) -> RoleBoxFuture<'a, RoleAdjudicationAttemptWriteResult> {
        Box::pin(async move {
            let candidate_roles_json = serde_json::to_value(&event.candidate_roles)?;
            let current_assignment_json = match &event.current_assignment {
                Some(value) => serde_json::to_value(value)?,
                None => Value::Null,
            };
            let raw_response_json = event.raw_response_json.clone().unwrap_or(Value::Null);
            let validated_result_json = event.validated_result_json.clone().unwrap_or(Value::Null);
            let deterministic_confidence = event
                .deterministic_confidence
                .map(|value| value.to_string())
                .unwrap_or_else(|| "NULL".to_string());
            let now = sql_now(self.relational);

            let sql = format!(
                "INSERT INTO architecture_role_adjudication_attempts (
                    repo_id, attempt_id, scope_key, generation_seq, target_kind, artefact_id,
                    symbol_id, path, reason, deterministic_confidence, candidate_roles_json,
                    current_assignment_json, request_json, evidence_packet_sha256,
                    evidence_packet_json, model_descriptor, slot_name, outcome, raw_response_json,
                    validated_result_json, failure_message, retryable, assignment_write_persisted,
                    assignment_write_source, observed_at_unix, updated_at
                 ) VALUES (
                    {repo_id}, {attempt_id}, {scope_key}, {generation_seq}, {target_kind}, {artefact_id},
                    {symbol_id}, {path}, {reason}, {deterministic_confidence}, {candidate_roles_json},
                    {current_assignment_json}, {request_json}, {evidence_packet_sha256},
                    {evidence_packet_json}, {model_descriptor}, {slot_name}, {outcome}, {raw_response_json},
                    {validated_result_json}, {failure_message}, {retryable}, 0,
                    NULL, {observed_at_unix}, {now}
                 )
                 ON CONFLICT(repo_id, attempt_id) DO UPDATE SET
                    raw_response_json = excluded.raw_response_json,
                    validated_result_json = excluded.validated_result_json,
                    failure_message = excluded.failure_message,
                    outcome = excluded.outcome,
                    retryable = excluded.retryable,
                    updated_at = {now};",
                repo_id = sql_text(&event.repo_id),
                attempt_id = sql_text(&event.attempt_id),
                scope_key = sql_text(&event.scope_key),
                generation_seq = event.generation,
                target_kind = sql_opt_text(event.target_kind.as_deref()),
                artefact_id = sql_opt_text(event.artefact_id.as_deref()),
                symbol_id = sql_opt_text(event.symbol_id.as_deref()),
                path = sql_opt_text(event.path.as_deref()),
                reason = sql_text(event.reason.as_str()),
                deterministic_confidence = deterministic_confidence,
                candidate_roles_json = sql_json_value(self.relational, &candidate_roles_json),
                current_assignment_json = sql_json_value(self.relational, &current_assignment_json),
                request_json = sql_json_value(self.relational, &event.request_json),
                evidence_packet_sha256 = sql_text(&event.evidence_packet_sha256),
                evidence_packet_json = sql_json_value(self.relational, &event.evidence_packet_json),
                model_descriptor = sql_text(&event.model_descriptor),
                slot_name = sql_text(&event.slot_name),
                outcome = sql_text(event.outcome.as_str()),
                raw_response_json = sql_json_value(self.relational, &raw_response_json),
                validated_result_json = sql_json_value(self.relational, &validated_result_json),
                failure_message = sql_opt_text(event.failure_message.as_deref()),
                retryable = if event.retryable { 1 } else { 0 },
                observed_at_unix = event.observed_at_unix,
                now = now,
            );
            self.relational
                .exec_serialized(&sql)
                .await
                .context("recording architecture role adjudication attempt")?;
            Ok(RoleAdjudicationAttemptWriteResult {
                attempt_id: event.attempt_id,
            })
        })
    }

    fn mark_assignment_write_result<'a>(
        &'a self,
        repo_id: &'a str,
        attempt_id: &'a str,
        assignment_write_persisted: bool,
        assignment_write_source: &'a str,
    ) -> RoleBoxFuture<'a, ()> {
        Box::pin(async move {
            let sql = format!(
                "UPDATE architecture_role_adjudication_attempts
                 SET assignment_write_persisted = {persisted},
                     assignment_write_source = {source},
                     updated_at = {now}
                 WHERE repo_id = {repo_id} AND attempt_id = {attempt_id};",
                persisted = if assignment_write_persisted { 1 } else { 0 },
                source = sql_text(assignment_write_source),
                now = sql_now(self.relational),
                repo_id = sql_text(repo_id),
                attempt_id = sql_text(attempt_id),
            );
            self.relational
                .exec_serialized(&sql)
                .await
                .context("updating architecture role adjudication attempt write result")?;
            Ok(())
        })
    }
}

pub async fn list_recent_role_adjudication_attempts(
    relational: &RelationalStorage,
    repo_id: &str,
    limit: usize,
) -> Result<Vec<RoleAdjudicationAttemptRecord>> {
    let rows = relational
        .query_rows(&format!(
            "SELECT attempt_id, scope_key, generation_seq, target_kind, artefact_id, symbol_id,
                    path, reason, outcome, model_descriptor, assignment_write_persisted,
                    assignment_write_source, failure_message, validated_result_json,
                    observed_at_unix, updated_at
             FROM architecture_role_adjudication_attempts
             WHERE repo_id = {repo_id}
             ORDER BY observed_at_unix DESC, updated_at DESC
             LIMIT {limit};",
            repo_id = sql_text(repo_id),
            limit = limit,
        ))
        .await
        .context("loading recent architecture role adjudication attempts")?;

    rows.into_iter().map(parse_attempt_record).collect()
}

fn parse_attempt_record(row: Value) -> Result<RoleAdjudicationAttemptRecord> {
    let validated_result = json_field(&row, "validated_result_json").unwrap_or(Value::Null);
    Ok(RoleAdjudicationAttemptRecord {
        attempt_id: string_field(&row, "attempt_id")?,
        scope_key: string_field(&row, "scope_key")?,
        generation: u64_field(&row, "generation_seq")?,
        target_kind: optional_string_field(&row, "target_kind"),
        artefact_id: optional_string_field(&row, "artefact_id"),
        symbol_id: optional_string_field(&row, "symbol_id"),
        path: optional_string_field(&row, "path"),
        reason: string_field(&row, "reason")?,
        outcome: string_field(&row, "outcome")?,
        model_descriptor: string_field(&row, "model_descriptor")?,
        assignment_write_persisted: bool_field(&row, "assignment_write_persisted"),
        assignment_write_source: optional_string_field(&row, "assignment_write_source"),
        failure_message: optional_string_field(&row, "failure_message"),
        reasoning_summary: validated_result
            .get("reasoning_summary")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
        observed_at_unix: u64_field(&row, "observed_at_unix")?,
        updated_at: optional_string_field(&row, "updated_at"),
    })
}

fn string_field(row: &Value, key: &str) -> Result<String> {
    row.get(key)
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .ok_or_else(|| anyhow!("row missing string field `{key}`"))
}

fn optional_string_field(row: &Value, key: &str) -> Option<String> {
    row.get(key).and_then(Value::as_str).map(ToOwned::to_owned)
}

fn u64_field(row: &Value, key: &str) -> Result<u64> {
    row.get(key)
        .and_then(Value::as_u64)
        .ok_or_else(|| anyhow!("row missing integer field `{key}`"))
}

fn bool_field(row: &Value, key: &str) -> bool {
    row.get(key)
        .and_then(Value::as_i64)
        .map(|value| value != 0)
        .or_else(|| row.get(key).and_then(Value::as_bool))
        .unwrap_or(false)
}

fn json_field(row: &Value, key: &str) -> Option<Value> {
    match row.get(key) {
        Some(Value::String(text)) => serde_json::from_str(text).ok(),
        Some(value) => Some(value.clone()),
        None => None,
    }
}

#[cfg(test)]
mod tests {
    use anyhow::Result;
    use serde_json::json;
    use tempfile::TempDir;

    use super::*;
    use crate::capability_packs::architecture_graph::roles::contracts::{
        AdjudicationReason, RoleAdjudicationAttemptEvent, RoleAdjudicationAttemptOutcome,
        RoleCandidateDescriptor,
    };
    use crate::capability_packs::architecture_graph::schema::architecture_graph_sqlite_schema_sql;
    use crate::host::devql::RelationalStorage;

    fn test_relational() -> Result<(TempDir, RelationalStorage)> {
        let temp = TempDir::new()?;
        let sqlite_path = temp.path().join("devql.sqlite");
        let conn = rusqlite::Connection::open(&sqlite_path)?;
        conn.execute_batch(architecture_graph_sqlite_schema_sql())?;
        drop(conn);
        Ok((temp, RelationalStorage::local_only(sqlite_path)))
    }

    fn attempt_event() -> RoleAdjudicationAttemptEvent {
        RoleAdjudicationAttemptEvent {
            attempt_id: "attempt-1".to_string(),
            repo_id: "repo-1".to_string(),
            scope_key: "repo-1:7:artefact:a1:unknown".to_string(),
            generation: 7,
            target_kind: Some("artefact".to_string()),
            artefact_id: Some("a1".to_string()),
            symbol_id: Some("s1".to_string()),
            path: Some("src/application/create_user.rs".to_string()),
            reason: AdjudicationReason::Unknown,
            deterministic_confidence: None,
            candidate_roles: vec![RoleCandidateDescriptor {
                role_id: "role-application".to_string(),
                canonical_key: "application_use_case".to_string(),
                family: "application".to_string(),
                display_name: "Application Use Case".to_string(),
                description: "Coordinates user creation.".to_string(),
            }],
            current_assignment: None,
            request_json: json!({"path": "src/application/create_user.rs"}),
            evidence_packet_sha256: "abc123".to_string(),
            evidence_packet_json: json!({"candidate_roles": [{"role_id": "role-application"}]}),
            model_descriptor: "codex:gpt-5.4-mini".to_string(),
            slot_name: "role_adjudication".to_string(),
            outcome: RoleAdjudicationAttemptOutcome::Assigned,
            raw_response_json: Some(json!({
                "outcome": "assigned",
                "assignments": [{"role_id": "role-application", "confidence": 0.87, "primary": true, "evidence": {}}],
                "confidence": 0.87,
                "evidence": {},
                "reasoning_summary": "The artefact coordinates the use case.",
                "rule_suggestions": []
            })),
            validated_result_json: Some(json!({
                "outcome": "assigned",
                "reasoning_summary": "The artefact coordinates the use case."
            })),
            failure_message: None,
            retryable: false,
            observed_at_unix: 1_778_572_288,
        }
    }

    #[tokio::test]
    async fn db_attempt_writer_records_raw_response_and_write_result() -> Result<()> {
        let (_temp, relational) = test_relational()?;
        let writer = DbRoleAdjudicationAttemptWriter::new(&relational);
        let result = writer.record_attempt(attempt_event()).await?;

        assert_eq!(result.attempt_id, "attempt-1");

        writer
            .mark_assignment_write_result("repo-1", "attempt-1", true, "db")
            .await?;

        let attempts = list_recent_role_adjudication_attempts(&relational, "repo-1", 10).await?;
        assert_eq!(attempts.len(), 1);
        assert_eq!(attempts[0].attempt_id, "attempt-1");
        assert_eq!(attempts[0].outcome, "assigned");
        assert!(attempts[0].assignment_write_persisted);
        assert_eq!(
            attempts[0].reasoning_summary.as_deref(),
            Some("The artefact coordinates the use case.")
        );
        Ok(())
    }
}
