use anyhow::{Context, Result};
use serde_json::Value;

use crate::host::devql::{RelationalStorage, sql_json_value, sql_now};

use super::facts::{delete_facts_for_paths_sql, insert_fact_sql};
use super::rows::{assignment_from_row, sql_opt_i64, sql_opt_text, sql_text};
use super::signals::{delete_signals_for_paths_sql, insert_signal_sql};
use crate::capability_packs::architecture_graph::roles::taxonomy::{
    ArchitectureArtefactFact, ArchitectureRoleAssignment, ArchitectureRoleRuleSignal,
    AssignmentStatus, RoleLifecycle, assignment_history_id,
};

pub async fn upsert_assignment(
    relational: &RelationalStorage,
    assignment: &ArchitectureRoleAssignment,
) -> Result<()> {
    relational
        .exec_serialized(&insert_assignment_sql(relational, assignment))
        .await
        .context("upserting architecture role assignment")
}

pub async fn replace_assignments_for_paths(
    relational: &RelationalStorage,
    repo_id: &str,
    paths: &[String],
    assignments: &[ArchitectureRoleAssignment],
) -> Result<usize> {
    let (assignments_written, _) =
        replace_assignments_for_paths_with_history(relational, repo_id, paths, assignments, &[])
            .await?;
    Ok(assignments_written)
}

#[derive(Debug, Clone)]
pub struct AssignmentHistoryWrite {
    pub previous: Option<ArchitectureRoleAssignment>,
    pub next: ArchitectureRoleAssignment,
    pub change_kind: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RoleClassificationStateWriteCounts {
    pub facts_written: usize,
    pub signals_written: usize,
    pub assignments_written: usize,
    pub assignments_marked_stale: usize,
    pub assignment_history_rows: usize,
}

pub struct RoleClassificationStateReplacement<'a> {
    pub repo_id: &'a str,
    pub fact_and_signal_paths: &'a [String],
    pub facts: &'a [ArchitectureArtefactFact],
    pub signals: &'a [ArchitectureRoleRuleSignal],
    pub assignment_paths: &'a [String],
    pub assignments: &'a [ArchitectureRoleAssignment],
    pub assignment_history_writes: &'a [AssignmentHistoryWrite],
    pub removed_assignment_paths: &'a [String],
    pub generation_seq: u64,
}

pub async fn replace_role_classification_state(
    relational: &RelationalStorage,
    replacement: RoleClassificationStateReplacement<'_>,
) -> Result<RoleClassificationStateWriteCounts> {
    let removed_active_assignments = active_assignments_for_paths(
        relational,
        replacement.repo_id,
        replacement.removed_assignment_paths,
    )
    .await?;
    let mut stale_history_writes = Vec::with_capacity(removed_active_assignments.len());
    let mut stale_assignments = Vec::with_capacity(removed_active_assignments.len());
    for previous in &removed_active_assignments {
        let mut next = previous.clone();
        next.status = AssignmentStatus::Stale;
        next.generation_seq = replacement.generation_seq;
        stale_history_writes.push(AssignmentHistoryWrite {
            previous: Some(previous.clone()),
            next: next.clone(),
            change_kind: "path_removed".to_string(),
        });
        stale_assignments.push(next);
    }

    let mut all_history_writes = Vec::with_capacity(
        stale_history_writes.len() + replacement.assignment_history_writes.len(),
    );
    all_history_writes.extend(stale_history_writes);
    all_history_writes.extend(replacement.assignment_history_writes.iter().cloned());
    let all_history_writes = new_assignment_history_writes(relational, &all_history_writes).await?;

    let mut statements = Vec::new();
    if !replacement.fact_and_signal_paths.is_empty() {
        statements.push(delete_facts_for_paths_sql(
            replacement.repo_id,
            replacement.fact_and_signal_paths,
        ));
    }
    for fact in replacement.facts {
        statements.push(insert_fact_sql(relational, fact));
    }
    if !replacement.fact_and_signal_paths.is_empty() {
        statements.push(delete_signals_for_paths_sql(
            replacement.repo_id,
            replacement.fact_and_signal_paths,
        ));
    }
    for signal in replacement.signals {
        statements.push(insert_signal_sql(relational, signal));
    }
    for history in &all_history_writes {
        statements.push(insert_assignment_history_sql(
            relational,
            history.previous.as_ref(),
            &history.next,
            &history.change_kind,
        ));
    }
    for assignment in &stale_assignments {
        statements.push(insert_assignment_sql(relational, assignment));
    }
    if !replacement.assignment_paths.is_empty() {
        statements.push(delete_assignments_for_paths_sql(
            replacement.repo_id,
            replacement.assignment_paths,
        ));
    }
    for assignment in replacement.assignments {
        statements.push(insert_assignment_sql(relational, assignment));
    }
    if !statements.is_empty() {
        relational
            .exec_serialized_batch_transactional(&statements)
            .await
            .context("replacing architecture role classification state")?;
    }

    Ok(RoleClassificationStateWriteCounts {
        facts_written: replacement.facts.len(),
        signals_written: replacement.signals.len(),
        assignments_written: replacement.assignments.len(),
        assignments_marked_stale: removed_active_assignments.len(),
        assignment_history_rows: all_history_writes.len(),
    })
}

pub async fn replace_assignments_for_paths_with_history(
    relational: &RelationalStorage,
    repo_id: &str,
    paths: &[String],
    assignments: &[ArchitectureRoleAssignment],
    history_writes: &[AssignmentHistoryWrite],
) -> Result<(usize, usize)> {
    let current_assignments = if paths.is_empty() {
        Vec::new()
    } else {
        load_assignments_for_paths(relational, repo_id, paths).await?
    };
    let history_writes = new_assignment_history_writes(relational, history_writes).await?;
    let mut statements = Vec::with_capacity(
        current_assignments.len() + assignments.len() + history_writes.len() + 1,
    );
    for history in &history_writes {
        statements.push(insert_assignment_history_sql(
            relational,
            history.previous.as_ref(),
            &history.next,
            &history.change_kind,
        ));
    }
    if !paths.is_empty() {
        statements.push(delete_assignments_for_paths_sql(repo_id, paths));
    }
    for assignment in assignments {
        statements.push(insert_assignment_sql(relational, assignment));
    }
    if !statements.is_empty() {
        relational
            .exec_serialized_batch_transactional(&statements)
            .await
            .context("replacing architecture role assignments for paths")?;
    }
    Ok((assignments.len(), history_writes.len()))
}

async fn new_assignment_history_writes(
    relational: &RelationalStorage,
    history_writes: &[AssignmentHistoryWrite],
) -> Result<Vec<AssignmentHistoryWrite>> {
    let mut new_writes = Vec::new();
    for history in history_writes {
        if !assignment_history_exists(relational, &history.next, &history.change_kind).await? {
            new_writes.push(history.clone());
        }
    }
    Ok(new_writes)
}

async fn assignment_history_exists(
    relational: &RelationalStorage,
    next: &ArchitectureRoleAssignment,
    change_kind: &str,
) -> Result<bool> {
    let history_id = assignment_history_id_for(next, change_kind);
    let sql = format!(
        "SELECT COUNT(*) AS count
         FROM architecture_role_assignment_history
         WHERE repo_id = {} AND history_id = {};",
        sql_text(&next.repo_id),
        sql_text(&history_id)
    );
    let rows = relational.query_rows(&sql).await?;
    Ok(rows
        .first()
        .and_then(|row| row.get("count"))
        .and_then(Value::as_i64)
        .unwrap_or(0)
        > 0)
}

pub async fn load_assignments_for_path(
    relational: &RelationalStorage,
    repo_id: &str,
    path: &str,
) -> Result<Vec<ArchitectureRoleAssignment>> {
    let sql = format!(
        "SELECT repo_id, assignment_id, role_id, target_kind, artefact_id, symbol_id, path,
                priority, status, source, confidence, evidence_json, provenance_json,
                classifier_version, rule_version, generation_seq
         FROM architecture_role_assignments_current
         WHERE repo_id = {} AND path = {}
         ORDER BY priority ASC, confidence DESC, assignment_id ASC",
        sql_text(repo_id),
        sql_text(path)
    );
    relational
        .query_rows(&sql)
        .await
        .context("loading architecture role assignments for path")?
        .into_iter()
        .map(assignment_from_row)
        .collect()
}

pub async fn load_assignments_for_paths(
    relational: &RelationalStorage,
    repo_id: &str,
    paths: &[String],
) -> Result<Vec<ArchitectureRoleAssignment>> {
    if paths.is_empty() {
        return Ok(Vec::new());
    }
    let sql = format!(
        "SELECT repo_id, assignment_id, role_id, target_kind, artefact_id, symbol_id, path,
                priority, status, source, confidence, evidence_json, provenance_json,
                classifier_version, rule_version, generation_seq
         FROM architecture_role_assignments_current
         WHERE repo_id = {} AND path IN ({})
         ORDER BY path ASC, priority ASC, confidence DESC, assignment_id ASC",
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
        .context("loading architecture role assignments for paths")?
        .into_iter()
        .map(assignment_from_row)
        .collect()
}

pub async fn mark_assignments_for_paths_stale(
    relational: &RelationalStorage,
    repo_id: &str,
    paths: &[String],
    generation_seq: u64,
) -> Result<usize> {
    let active_assignments = active_assignments_for_paths(relational, repo_id, paths).await?;
    let mut statements = Vec::with_capacity(active_assignments.len() * 2);
    for previous in &active_assignments {
        let mut next = previous.clone();
        next.status = AssignmentStatus::Stale;
        next.generation_seq = generation_seq;
        statements.push(insert_assignment_history_sql(
            relational,
            Some(previous),
            &next,
            "path_removed",
        ));
        statements.push(insert_assignment_sql(relational, &next));
    }
    if !statements.is_empty() {
        relational
            .exec_serialized_batch_transactional(&statements)
            .await
            .context("marking architecture role assignments stale for paths")?;
    }
    Ok(active_assignments.len())
}

async fn active_assignments_for_paths(
    relational: &RelationalStorage,
    repo_id: &str,
    paths: &[String],
) -> Result<Vec<ArchitectureRoleAssignment>> {
    Ok(load_assignments_for_paths(relational, repo_id, paths)
        .await?
        .into_iter()
        .filter(|assignment| assignment.status == AssignmentStatus::Active)
        .collect())
}

fn delete_assignments_for_paths_sql(repo_id: &str, paths: &[String]) -> String {
    if paths.is_empty() {
        return String::new();
    }
    format!(
        "DELETE FROM architecture_role_assignments_current
         WHERE repo_id = {} AND path IN ({});",
        sql_text(repo_id),
        paths
            .iter()
            .map(|path| sql_text(path))
            .collect::<Vec<_>>()
            .join(", ")
    )
}

pub async fn retire_role_and_mark_assignments(
    relational: &RelationalStorage,
    repo_id: &str,
    role_id: &str,
    lifecycle: RoleLifecycle,
    assignment_status: AssignmentStatus,
) -> Result<()> {
    let active_assignments =
        load_assignments_for_role_status(relational, repo_id, role_id, AssignmentStatus::Active)
            .await?;
    let mut statements = Vec::with_capacity(active_assignments.len() + 2);
    statements.push(format!(
        "UPDATE architecture_roles
         SET lifecycle = {lifecycle}, lifecycle_status = {lifecycle}, updated_at = {now}
         WHERE repo_id = {repo_id} AND role_id = {role_id};",
        repo_id = sql_text(repo_id),
        role_id = sql_text(role_id),
        lifecycle = sql_text(lifecycle.as_db()),
        now = sql_now(relational),
    ));

    for previous in &active_assignments {
        let mut next = previous.clone();
        next.status = assignment_status;
        statements.push(insert_assignment_history_sql(
            relational,
            Some(previous),
            &next,
            "role_lifecycle_changed",
        ));
    }

    statements.push(format!(
        "UPDATE architecture_role_assignments_current
         SET status = {assignment_status}, updated_at = {now}
         WHERE repo_id = {repo_id} AND role_id = {role_id} AND status = 'active';",
        repo_id = sql_text(repo_id),
        role_id = sql_text(role_id),
        assignment_status = sql_text(assignment_status.as_db()),
        now = sql_now(relational),
    ));
    relational
        .exec_serialized_batch_transactional(&statements)
        .await
        .context("retiring architecture role and marking assignments")
}

async fn load_assignments_for_role_status(
    relational: &RelationalStorage,
    repo_id: &str,
    role_id: &str,
    status: AssignmentStatus,
) -> Result<Vec<ArchitectureRoleAssignment>> {
    let sql = format!(
        "SELECT repo_id, assignment_id, role_id, target_kind, artefact_id, symbol_id, path,
                priority, status, source, confidence, evidence_json, provenance_json,
                classifier_version, rule_version, generation_seq
         FROM architecture_role_assignments_current
         WHERE repo_id = {repo_id} AND role_id = {role_id} AND status = {status}
         ORDER BY assignment_id ASC",
        repo_id = sql_text(repo_id),
        role_id = sql_text(role_id),
        status = sql_text(status.as_db()),
    );
    relational
        .query_rows(&sql)
        .await
        .context("loading architecture role assignments for role status")?
        .into_iter()
        .map(assignment_from_row)
        .collect()
}

pub async fn record_assignment_history(
    relational: &RelationalStorage,
    previous: Option<&ArchitectureRoleAssignment>,
    next: &ArchitectureRoleAssignment,
    change_kind: &str,
) -> Result<bool> {
    if assignment_history_exists(relational, next, change_kind).await? {
        return Ok(false);
    }
    let sql = insert_assignment_history_sql(relational, previous, next, change_kind);
    relational
        .exec_serialized(&sql)
        .await
        .context("recording architecture role assignment history")?;
    Ok(true)
}

fn insert_assignment_history_sql(
    relational: &RelationalStorage,
    previous: Option<&ArchitectureRoleAssignment>,
    next: &ArchitectureRoleAssignment,
    change_kind: &str,
) -> String {
    let history_id = assignment_history_id_for(next, change_kind);
    format!(
        "INSERT INTO architecture_role_assignment_history (
            repo_id, history_id, assignment_id, role_id, target_kind, artefact_id, symbol_id, path,
            previous_status, new_status, previous_confidence, new_confidence, change_kind,
            evidence_json, provenance_json, generation_seq
         ) VALUES (
            {repo_id}, {history_id}, {assignment_id}, {role_id}, {target_kind}, {artefact_id}, {symbol_id}, {path},
            {previous_status}, {new_status}, {previous_confidence}, {new_confidence}, {change_kind},
            {evidence}, {provenance}, {generation_seq}
         )
         ON CONFLICT(repo_id, history_id) DO NOTHING;",
        repo_id = sql_text(&next.repo_id),
        history_id = sql_text(&history_id),
        assignment_id = sql_text(&next.assignment_id),
        role_id = sql_text(&next.role_id),
        target_kind = sql_text(next.target.target_kind.as_db()),
        artefact_id = sql_opt_text(next.target.artefact_id.as_deref()),
        symbol_id = sql_opt_text(next.target.symbol_id.as_deref()),
        path = sql_text(&next.target.path),
        previous_status = previous
            .map(|assignment| sql_text(assignment.status.as_db()))
            .unwrap_or_else(|| "NULL".to_string()),
        new_status = sql_text(next.status.as_db()),
        previous_confidence = previous
            .map(|assignment| assignment.confidence.to_string())
            .unwrap_or_else(|| "NULL".to_string()),
        new_confidence = next.confidence,
        change_kind = sql_text(change_kind),
        evidence = sql_json_value(relational, &next.evidence),
        provenance = sql_json_value(relational, &next.provenance),
        generation_seq = next.generation_seq,
    )
}

fn assignment_history_id_for(next: &ArchitectureRoleAssignment, change_kind: &str) -> String {
    assignment_history_id(
        &next.repo_id,
        &next.assignment_id,
        next.generation_seq,
        change_kind,
    )
}
fn insert_assignment_sql(
    relational: &RelationalStorage,
    assignment: &ArchitectureRoleAssignment,
) -> String {
    format!(
        "INSERT INTO architecture_role_assignments_current (
            repo_id, assignment_id, role_id, target_kind, artefact_id, symbol_id, path,
            priority, status, source, confidence, evidence_json, provenance_json,
            classifier_version, rule_version, generation_seq, updated_at
         ) VALUES (
            {repo_id}, {assignment_id}, {role_id}, {target_kind}, {artefact_id}, {symbol_id}, {path},
            {priority}, {status}, {source}, {confidence}, {evidence}, {provenance},
            {classifier_version}, {rule_version}, {generation_seq}, {now}
         )
         ON CONFLICT(repo_id, assignment_id) DO UPDATE SET
            priority = excluded.priority,
            status = excluded.status,
            source = excluded.source,
            confidence = excluded.confidence,
            evidence_json = excluded.evidence_json,
            provenance_json = excluded.provenance_json,
            classifier_version = excluded.classifier_version,
            rule_version = excluded.rule_version,
            generation_seq = excluded.generation_seq,
            updated_at = {now};",
        repo_id = sql_text(&assignment.repo_id),
        assignment_id = sql_text(&assignment.assignment_id),
        role_id = sql_text(&assignment.role_id),
        target_kind = sql_text(assignment.target.target_kind.as_db()),
        artefact_id = sql_opt_text(assignment.target.artefact_id.as_deref()),
        symbol_id = sql_opt_text(assignment.target.symbol_id.as_deref()),
        path = sql_text(&assignment.target.path),
        priority = sql_text(assignment.priority.as_db()),
        status = sql_text(assignment.status.as_db()),
        source = sql_text(assignment.source.as_db()),
        confidence = assignment.confidence,
        evidence = sql_json_value(relational, &assignment.evidence),
        provenance = sql_json_value(relational, &assignment.provenance),
        classifier_version = sql_text(&assignment.classifier_version),
        rule_version = sql_opt_i64(assignment.rule_version),
        generation_seq = assignment.generation_seq,
        now = sql_now(relational),
    )
}
