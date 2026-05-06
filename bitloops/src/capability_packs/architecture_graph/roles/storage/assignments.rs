use std::collections::BTreeSet;

use anyhow::{Context, Result};
use serde_json::{Value, json};

use crate::host::devql::{RelationalStorage, sql_json_value, sql_now};

use super::facts::{count_role_facts_for_paths, delete_facts_for_paths_sql, insert_fact_sql};
use super::rows::{assignment_from_row, sql_opt_i64, sql_opt_text, sql_text};
use super::signals::{
    count_role_signals_for_paths, delete_signals_for_paths_sql, insert_signal_sql,
};
use crate::capability_packs::architecture_graph::roles::taxonomy::{
    ArchitectureArtefactFact, ArchitectureRoleAssignment, ArchitectureRoleRuleSignal,
    AssignmentSource, AssignmentStatus, RoleLifecycle, assignment_history_id, assignment_id,
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
    pub facts_deleted: usize,
    pub signals_written: usize,
    pub signals_deleted: usize,
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
    let facts_deleted = count_role_facts_for_paths(
        relational,
        replacement.repo_id,
        replacement.fact_and_signal_paths,
    )
    .await?;
    let signals_deleted = count_role_signals_for_paths(
        relational,
        replacement.repo_id,
        replacement.fact_and_signal_paths,
    )
    .await?;
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
        facts_deleted,
        signals_written: replacement.signals.len(),
        signals_deleted,
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

pub async fn load_active_assignment_paths_not_in(
    relational: &RelationalStorage,
    repo_id: &str,
    live_paths: &BTreeSet<String>,
) -> Result<Vec<String>> {
    let rows = relational
        .query_rows(&format!(
            "SELECT DISTINCT path
             FROM architecture_role_assignments_current
             WHERE repo_id = {} AND status = 'active'
             ORDER BY path ASC;",
            sql_text(repo_id)
        ))
        .await
        .context("loading active architecture role assignment paths")?;

    Ok(rows
        .into_iter()
        .filter_map(|row| {
            row.get("path")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned)
        })
        .filter(|path| !live_paths.contains(path))
        .collect())
}

pub async fn load_current_assignment_by_id(
    relational: &RelationalStorage,
    repo_id: &str,
    assignment_id: &str,
) -> Result<Option<ArchitectureRoleAssignment>> {
    let sql = format!(
        "SELECT repo_id, assignment_id, role_id, target_kind, artefact_id, symbol_id, path,
                priority, status, source, confidence, evidence_json, provenance_json,
                classifier_version, rule_version, generation_seq
         FROM architecture_role_assignments_current
         WHERE repo_id = {repo_id} AND assignment_id = {assignment_id}
         LIMIT 1",
        repo_id = sql_text(repo_id),
        assignment_id = sql_text(assignment_id),
    );
    relational
        .query_rows(&sql)
        .await
        .context("loading current architecture role assignment by id")?
        .into_iter()
        .map(assignment_from_row)
        .next()
        .transpose()
}

pub async fn list_current_assignments_for_role(
    relational: &RelationalStorage,
    repo_id: &str,
    role_id: &str,
) -> Result<Vec<ArchitectureRoleAssignment>> {
    let sql = format!(
        "SELECT repo_id, assignment_id, role_id, target_kind, artefact_id, symbol_id, path,
                priority, status, source, confidence, evidence_json, provenance_json,
                classifier_version, rule_version, generation_seq
         FROM architecture_role_assignments_current
         WHERE repo_id = {repo_id} AND role_id = {role_id}
         ORDER BY assignment_id ASC",
        repo_id = sql_text(repo_id),
        role_id = sql_text(role_id),
    );
    relational
        .query_rows(&sql)
        .await
        .context("listing current architecture role assignments for role")?
        .into_iter()
        .map(assignment_from_row)
        .collect()
}

pub async fn list_active_current_assignments_for_role(
    relational: &RelationalStorage,
    repo_id: &str,
    role_id: &str,
) -> Result<Vec<ArchitectureRoleAssignment>> {
    Ok(
        list_current_assignments_for_role(relational, repo_id, role_id)
            .await?
            .into_iter()
            .filter(|assignment| assignment.status == AssignmentStatus::Active)
            .collect(),
    )
}

pub async fn update_current_assignment_status(
    relational: &RelationalStorage,
    repo_id: &str,
    assignment_id: &str,
    status: AssignmentStatus,
    reason: &str,
    migration_id: Option<&str>,
    change_kind: &str,
) -> Result<bool> {
    let Some(previous) = load_current_assignment_by_id(relational, repo_id, assignment_id).await?
    else {
        return Ok(false);
    };
    let mut next = previous.clone();
    next.status = status;
    next.provenance = merge_provenance_patch(
        next.provenance,
        status_provenance_patch(reason, migration_id, None),
    );
    let statements = vec![
        insert_assignment_history_sql(relational, Some(&previous), &next, change_kind),
        insert_assignment_sql(relational, &next),
    ];
    relational
        .exec_serialized_batch_transactional(&statements)
        .await
        .context("updating current architecture role assignment status")?;
    Ok(true)
}

pub async fn migrate_current_assignment_to_role(
    relational: &RelationalStorage,
    previous: &ArchitectureRoleAssignment,
    target_role_id: &str,
    migration_id: &str,
    migration_kind: &str,
) -> Result<String> {
    let new_assignment_id = assignment_id(&previous.repo_id, target_role_id, &previous.target);
    let existing_target =
        load_current_assignment_by_id(relational, &previous.repo_id, &new_assignment_id).await?;
    let mut source_next = previous.clone();
    source_next.status = AssignmentStatus::Stale;
    source_next.provenance = merge_provenance_patch(
        source_next.provenance,
        status_provenance_patch(
            "migrated by proposal",
            Some(migration_id),
            Some(&new_assignment_id),
        ),
    );

    let mut statements = vec![
        insert_assignment_history_sql(relational, Some(previous), &source_next, migration_kind),
        insert_assignment_sql(relational, &source_next),
    ];
    if existing_target.is_none() {
        let mut target_next = previous.clone();
        target_next.assignment_id = new_assignment_id.clone();
        target_next.role_id = target_role_id.to_string();
        target_next.status = AssignmentStatus::Active;
        target_next.source = AssignmentSource::Migration;
        target_next.provenance = merge_provenance_patch(
            target_next.provenance,
            json_provenance_patch(&[
                ("source", migration_kind),
                ("migrationId", migration_id),
                ("migratedFromAssignmentId", previous.assignment_id.as_str()),
            ]),
        );
        statements.push(insert_assignment_history_sql(
            relational,
            None,
            &target_next,
            "role_migration_created",
        ));
        statements.push(insert_assignment_sql(relational, &target_next));
    }
    relational
        .exec_serialized_batch_transactional(&statements)
        .await
        .context("migrating current architecture role assignment")?;
    Ok(new_assignment_id)
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
         SET lifecycle_status = {lifecycle}, updated_at = {now}
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
    let identity = format!(
        "{}|{}|{}|{}|{}",
        change_kind,
        next.status.as_db(),
        next.confidence,
        next.provenance,
        next.evidence
    );
    assignment_history_id(
        &next.repo_id,
        &next.assignment_id,
        next.generation_seq,
        &identity,
    )
}

fn status_provenance_patch(
    reason: &str,
    migration_id: Option<&str>,
    migrated_to_assignment_id: Option<&str>,
) -> Value {
    let mut patch = json!({
        "statusReason": reason,
    });
    if let Value::Object(object) = &mut patch {
        if let Some(migration_id) = migration_id {
            object.insert("migrationId".to_string(), json!(migration_id));
        }
        if let Some(migrated_to_assignment_id) = migrated_to_assignment_id {
            object.insert(
                "migratedToAssignmentId".to_string(),
                json!(migrated_to_assignment_id),
            );
        }
    }
    patch
}

fn json_provenance_patch(values: &[(&str, &str)]) -> Value {
    let mut patch = serde_json::Map::new();
    for (key, value) in values {
        patch.insert((*key).to_string(), json!(value));
    }
    Value::Object(patch)
}

fn merge_provenance_patch(mut provenance: Value, patch: Value) -> Value {
    match (&mut provenance, patch) {
        (Value::Object(provenance), Value::Object(patch)) => {
            for (key, value) in patch {
                provenance.insert(key, value);
            }
            Value::Object(provenance.clone())
        }
        (_, patch) => patch,
    }
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
