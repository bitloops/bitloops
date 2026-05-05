use anyhow::{Context, Result, anyhow};
use serde_json::{Value, json};

use crate::host::devql::{RelationalStorage, sql_json_value, sql_now};

use super::taxonomy::{
    ArchitectureRole, ArchitectureRoleAssignment, ArchitectureRoleChangeProposal,
    AssignmentPriority, AssignmentSource, AssignmentStatus, RoleLifecycle, RoleTarget, TargetKind,
    assignment_history_id,
};

pub async fn upsert_role(relational: &RelationalStorage, role: &ArchitectureRole) -> Result<()> {
    let sql = format!(
        "INSERT INTO architecture_roles (
            repo_id, role_id, family, slug, display_name, description, lifecycle, provenance_json, updated_at
         ) VALUES ({repo_id}, {role_id}, {family}, {slug}, {display_name}, {description}, {lifecycle}, {provenance}, {now})
         ON CONFLICT(repo_id, role_id) DO UPDATE SET
            family = excluded.family,
            slug = excluded.slug,
            display_name = excluded.display_name,
            description = excluded.description,
            lifecycle = excluded.lifecycle,
            provenance_json = excluded.provenance_json,
            updated_at = {now};",
        repo_id = sql_text(&role.repo_id),
        role_id = sql_text(&role.role_id),
        family = sql_text(&role.family),
        slug = sql_text(&role.slug),
        display_name = sql_text(&role.display_name),
        description = sql_text(&role.description),
        lifecycle = sql_text(role.lifecycle.as_db()),
        provenance = sql_json_value(relational, &role.provenance),
        now = sql_now(relational),
    );
    relational
        .exec_serialized(&sql)
        .await
        .context("upserting architecture role")
}

pub async fn rename_role(
    relational: &RelationalStorage,
    repo_id: &str,
    role_id: &str,
    display_name: &str,
    provenance: &Value,
) -> Result<()> {
    let sql = format!(
        "UPDATE architecture_roles
         SET display_name = {display_name}, provenance_json = {provenance}, updated_at = {now}
         WHERE repo_id = {repo_id} AND role_id = {role_id};",
        repo_id = sql_text(repo_id),
        role_id = sql_text(role_id),
        display_name = sql_text(display_name),
        provenance = sql_json_value(relational, provenance),
        now = sql_now(relational),
    );
    relational
        .exec_serialized(&sql)
        .await
        .context("renaming architecture role")
}

pub async fn set_role_lifecycle(
    relational: &RelationalStorage,
    repo_id: &str,
    role_id: &str,
    lifecycle: RoleLifecycle,
) -> Result<()> {
    let sql = format!(
        "UPDATE architecture_roles
         SET lifecycle = {lifecycle}, updated_at = {now}
         WHERE repo_id = {repo_id} AND role_id = {role_id};",
        repo_id = sql_text(repo_id),
        role_id = sql_text(role_id),
        lifecycle = sql_text(lifecycle.as_db()),
        now = sql_now(relational),
    );
    relational
        .exec_serialized(&sql)
        .await
        .context("updating architecture role lifecycle")
}

pub async fn load_roles(
    relational: &RelationalStorage,
    repo_id: &str,
) -> Result<Vec<ArchitectureRole>> {
    let sql = format!(
        "SELECT repo_id, role_id, family, slug, display_name, description, lifecycle, provenance_json
         FROM architecture_roles
         WHERE repo_id = {}
         ORDER BY family ASC, slug ASC",
        sql_text(repo_id)
    );
    relational
        .query_rows(&sql)
        .await
        .context("loading architecture roles")?
        .into_iter()
        .map(role_from_row)
        .collect()
}

pub async fn upsert_assignment(
    relational: &RelationalStorage,
    assignment: &ArchitectureRoleAssignment,
) -> Result<()> {
    relational
        .exec_serialized(&insert_assignment_sql(relational, assignment))
        .await
        .context("upserting architecture role assignment")
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
         SET lifecycle = {lifecycle}, updated_at = {now}
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
) -> Result<()> {
    let sql = insert_assignment_history_sql(relational, previous, next, change_kind);
    relational
        .exec_serialized(&sql)
        .await
        .context("recording architecture role assignment history")
}

fn insert_assignment_history_sql(
    relational: &RelationalStorage,
    previous: Option<&ArchitectureRoleAssignment>,
    next: &ArchitectureRoleAssignment,
    change_kind: &str,
) -> String {
    let history_id = assignment_history_id(
        &next.repo_id,
        &next.assignment_id,
        next.generation_seq,
        change_kind,
    );
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

pub async fn insert_change_proposal(
    relational: &RelationalStorage,
    proposal: &ArchitectureRoleChangeProposal,
) -> Result<()> {
    let sql = format!(
        "INSERT INTO architecture_role_change_proposals (
            repo_id, proposal_id, proposal_kind, status, payload_json, impact_preview_json, provenance_json
         ) VALUES ({repo_id}, {proposal_id}, {proposal_kind}, {status}, {payload}, {impact_preview}, {provenance})
         ON CONFLICT(repo_id, proposal_id) DO UPDATE SET
            status = excluded.status,
            payload_json = excluded.payload_json,
            impact_preview_json = excluded.impact_preview_json,
            provenance_json = excluded.provenance_json;",
        repo_id = sql_text(&proposal.repo_id),
        proposal_id = sql_text(&proposal.proposal_id),
        proposal_kind = sql_text(&proposal.proposal_kind),
        status = sql_text(proposal.status.as_db()),
        payload = sql_json_value(relational, &proposal.payload),
        impact_preview = sql_json_value(relational, &proposal.impact_preview),
        provenance = sql_json_value(relational, &proposal.provenance),
    );
    relational
        .exec_serialized(&sql)
        .await
        .context("inserting architecture role change proposal")
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

fn role_from_row(row: Value) -> Result<ArchitectureRole> {
    let provenance = row_json(&row, "provenance_json", json!({}))?;
    Ok(ArchitectureRole {
        repo_id: row_string(&row, "repo_id")?,
        role_id: row_string(&row, "role_id")?,
        family: row_string(&row, "family")?,
        slug: row_string(&row, "slug")?,
        display_name: row_string(&row, "display_name")?,
        description: row_string(&row, "description")?,
        lifecycle: role_lifecycle_from_db(&row_string(&row, "lifecycle")?)?,
        provenance,
    })
}

fn assignment_from_row(row: Value) -> Result<ArchitectureRoleAssignment> {
    let target = RoleTarget {
        target_kind: target_kind_from_db(&row_string(&row, "target_kind")?)?,
        artefact_id: row_opt_string(&row, "artefact_id"),
        symbol_id: row_opt_string(&row, "symbol_id"),
        path: row_string(&row, "path")?,
    };
    Ok(ArchitectureRoleAssignment {
        repo_id: row_string(&row, "repo_id")?,
        assignment_id: row_string(&row, "assignment_id")?,
        role_id: row_string(&row, "role_id")?,
        target,
        priority: assignment_priority_from_db(&row_string(&row, "priority")?)?,
        status: assignment_status_from_db(&row_string(&row, "status")?)?,
        source: assignment_source_from_db(&row_string(&row, "source")?)?,
        confidence: row_f64(&row, "confidence")?,
        evidence: row_json(&row, "evidence_json", json!([]))?,
        provenance: row_json(&row, "provenance_json", json!({}))?,
        classifier_version: row_string(&row, "classifier_version")?,
        rule_version: row.get("rule_version").and_then(Value::as_i64),
        generation_seq: row_u64(&row, "generation_seq")?,
    })
}

fn assignment_priority_from_db(value: &str) -> Result<AssignmentPriority> {
    match value {
        "primary" => Ok(AssignmentPriority::Primary),
        "secondary" => Ok(AssignmentPriority::Secondary),
        other => Err(anyhow!(
            "unknown architecture role assignment priority `{other}`"
        )),
    }
}

fn assignment_source_from_db(value: &str) -> Result<AssignmentSource> {
    match value {
        "rule" => Ok(AssignmentSource::Rule),
        "llm" => Ok(AssignmentSource::Llm),
        "human" => Ok(AssignmentSource::Human),
        "migration" => Ok(AssignmentSource::Migration),
        other => Err(anyhow!(
            "unknown architecture role assignment source `{other}`"
        )),
    }
}

fn assignment_status_from_db(value: &str) -> Result<AssignmentStatus> {
    match value {
        "active" => Ok(AssignmentStatus::Active),
        "stale" => Ok(AssignmentStatus::Stale),
        "needs_review" => Ok(AssignmentStatus::NeedsReview),
        "rejected" => Ok(AssignmentStatus::Rejected),
        other => Err(anyhow!(
            "unknown architecture role assignment status `{other}`"
        )),
    }
}

fn role_lifecycle_from_db(value: &str) -> Result<RoleLifecycle> {
    match value {
        "active" => Ok(RoleLifecycle::Active),
        "deprecated" => Ok(RoleLifecycle::Deprecated),
        "removed" => Ok(RoleLifecycle::Removed),
        other => Err(anyhow!("unknown architecture role lifecycle `{other}`")),
    }
}

fn target_kind_from_db(value: &str) -> Result<TargetKind> {
    match value {
        "file" => Ok(TargetKind::File),
        "artefact" => Ok(TargetKind::Artefact),
        "symbol" => Ok(TargetKind::Symbol),
        other => Err(anyhow!("unknown architecture role target kind `{other}`")),
    }
}

fn sql_text(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

fn sql_opt_text(value: Option<&str>) -> String {
    value.map(sql_text).unwrap_or_else(|| "NULL".to_string())
}

fn sql_opt_i64(value: Option<i64>) -> String {
    value
        .map(|value| value.to_string())
        .unwrap_or_else(|| "NULL".to_string())
}

fn row_string(row: &Value, key: &str) -> Result<String> {
    row.get(key)
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .ok_or_else(|| anyhow!("architecture role row missing string field `{key}`"))
}

fn row_opt_string(row: &Value, key: &str) -> Option<String> {
    row.get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn row_f64(row: &Value, key: &str) -> Result<f64> {
    row.get(key)
        .and_then(Value::as_f64)
        .ok_or_else(|| anyhow!("architecture role row missing numeric field `{key}`"))
}

fn row_u64(row: &Value, key: &str) -> Result<u64> {
    row.get(key)
        .and_then(Value::as_i64)
        .and_then(|value| u64::try_from(value).ok())
        .ok_or_else(|| anyhow!("architecture role row missing unsigned integer field `{key}`"))
}

fn row_json(row: &Value, key: &str, default: Value) -> Result<Value> {
    match row.get(key) {
        Some(Value::String(raw)) if !raw.trim().is_empty() => serde_json::from_str(raw)
            .with_context(|| format!("parsing architecture role JSON field `{key}`")),
        Some(Value::String(_)) | Some(Value::Null) | None => Ok(default),
        Some(value) => Ok(value.clone()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capability_packs::architecture_graph::roles::taxonomy::{
        ArchitectureRole, ArchitectureRoleAssignment, ArchitectureRoleChangeProposal,
        AssignmentPriority, AssignmentSource, AssignmentStatus, ProposalStatus, RoleTarget,
        proposal_id, stable_role_id,
    };
    use crate::capability_packs::architecture_graph::schema::architecture_graph_sqlite_schema_sql;
    use tempfile::TempDir;

    fn test_relational() -> anyhow::Result<(TempDir, RelationalStorage)> {
        let temp = TempDir::new()?;
        let sqlite_path = temp.path().join("devql.sqlite");
        let conn = rusqlite::Connection::open(&sqlite_path)?;
        conn.execute_batch(architecture_graph_sqlite_schema_sql())?;
        drop(conn);
        Ok((temp, RelationalStorage::local_only(sqlite_path)))
    }

    fn role_fixture(
        repo_id: &str,
        family: &str,
        slug: &str,
        display_name: &str,
    ) -> ArchitectureRole {
        ArchitectureRole {
            repo_id: repo_id.to_string(),
            role_id: stable_role_id(repo_id, family, slug),
            family: family.to_string(),
            slug: slug.to_string(),
            display_name: display_name.to_string(),
            description: "Test role".to_string(),
            lifecycle: RoleLifecycle::Active,
            provenance: serde_json::json!({"source": "test"}),
        }
    }

    fn assignment_fixture(role: &ArchitectureRole, path: &str) -> ArchitectureRoleAssignment {
        let target = RoleTarget::artefact("art-1", "sym-1", path);
        ArchitectureRoleAssignment {
            repo_id: role.repo_id.clone(),
            assignment_id: super::super::taxonomy::assignment_id(
                &role.repo_id,
                &role.role_id,
                &target,
            ),
            role_id: role.role_id.clone(),
            target,
            priority: AssignmentPriority::Primary,
            status: AssignmentStatus::Active,
            source: AssignmentSource::Rule,
            confidence: 0.91,
            evidence: serde_json::json!([{ "fact": "path:suffix:main.rs" }]),
            provenance: serde_json::json!({ "classifier": "test" }),
            classifier_version: "test.classifier.v1".to_string(),
            rule_version: Some(3),
            generation_seq: 7,
        }
    }

    async fn assignment_history_count(
        relational: &RelationalStorage,
        repo_id: &str,
        assignment_id: &str,
    ) -> Result<usize> {
        let sql = format!(
            "SELECT COUNT(*) AS count
             FROM architecture_role_assignment_history
             WHERE repo_id = {} AND assignment_id = {}",
            sql_text(repo_id),
            sql_text(assignment_id)
        );
        let rows = relational.query_rows(&sql).await?;
        let count = rows
            .first()
            .and_then(|row| row.get("count"))
            .and_then(Value::as_i64)
            .unwrap_or(0);
        Ok(usize::try_from(count).unwrap_or(0))
    }

    #[tokio::test]
    async fn upsert_and_load_role_preserves_stable_id_and_display_name() -> anyhow::Result<()> {
        let (_temp, relational) = test_relational()?;
        let role = role_fixture("repo-1", "application", "entrypoint", "Entrypoint");

        upsert_role(&relational, &role).await?;

        let roles = load_roles(&relational, "repo-1").await?;
        assert_eq!(roles.len(), 1);
        assert_eq!(roles[0].role_id, role.role_id);
        assert_eq!(roles[0].display_name, "Entrypoint");
        assert_eq!(roles[0].lifecycle, RoleLifecycle::Active);
        Ok(())
    }

    #[tokio::test]
    async fn rename_role_keeps_stable_role_id() -> anyhow::Result<()> {
        let (_temp, relational) = test_relational()?;
        let role = role_fixture("repo-1", "application", "entrypoint", "Entrypoint");
        upsert_role(&relational, &role).await?;

        rename_role(
            &relational,
            "repo-1",
            &role.role_id,
            "Process Entrypoint",
            &serde_json::json!({"source": "rename-test"}),
        )
        .await?;

        let roles = load_roles(&relational, "repo-1").await?;
        assert_eq!(roles[0].role_id, role.role_id);
        assert_eq!(roles[0].display_name, "Process Entrypoint");
        Ok(())
    }

    #[tokio::test]
    async fn assignment_persistence_preserves_contract_fields() -> anyhow::Result<()> {
        let (_temp, relational) = test_relational()?;
        let role = role_fixture("repo-1", "application", "entrypoint", "Entrypoint");
        upsert_role(&relational, &role).await?;
        let assignment = assignment_fixture(&role, "src/main.rs");

        upsert_assignment(&relational, &assignment).await?;

        let loaded = load_assignments_for_path(&relational, "repo-1", "src/main.rs").await?;
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].role_id, role.role_id);
        assert_eq!(loaded[0].source, AssignmentSource::Rule);
        assert_eq!(loaded[0].confidence, 0.91);
        assert_eq!(loaded[0].classifier_version, "test.classifier.v1");
        assert_eq!(loaded[0].rule_version, Some(3));
        assert_eq!(loaded[0].generation_seq, 7);
        assert_eq!(loaded[0].evidence[0]["fact"], "path:suffix:main.rs");
        Ok(())
    }

    #[tokio::test]
    async fn removing_role_marks_active_assignments_needs_review() -> anyhow::Result<()> {
        let (_temp, relational) = test_relational()?;
        let role = role_fixture("repo-1", "application", "entrypoint", "Entrypoint");
        upsert_role(&relational, &role).await?;
        upsert_assignment(&relational, &assignment_fixture(&role, "src/main.rs")).await?;

        retire_role_and_mark_assignments(
            &relational,
            "repo-1",
            &role.role_id,
            RoleLifecycle::Removed,
            AssignmentStatus::NeedsReview,
        )
        .await?;

        let roles = load_roles(&relational, "repo-1").await?;
        assert_eq!(roles[0].lifecycle, RoleLifecycle::Removed);

        let assignments = load_assignments_for_path(&relational, "repo-1", "src/main.rs").await?;
        assert_eq!(assignments[0].status, AssignmentStatus::NeedsReview);
        assert_eq!(assignments[0].role_id, role.role_id);
        assert_eq!(
            assignment_history_count(&relational, "repo-1", &assignments[0].assignment_id).await?,
            1
        );
        Ok(())
    }

    #[tokio::test]
    async fn assignment_history_records_meaningful_status_change() -> anyhow::Result<()> {
        let (_temp, relational) = test_relational()?;
        let role = role_fixture("repo-1", "application", "entrypoint", "Entrypoint");
        upsert_role(&relational, &role).await?;
        let previous = assignment_fixture(&role, "src/main.rs");
        upsert_assignment(&relational, &previous).await?;

        let mut next = previous.clone();
        next.status = AssignmentStatus::NeedsReview;
        next.confidence = 0.62;
        next.generation_seq = 8;
        record_assignment_history(&relational, Some(&previous), &next, "status_changed").await?;

        assert_eq!(
            assignment_history_count(&relational, "repo-1", &next.assignment_id).await?,
            1
        );
        Ok(())
    }

    #[tokio::test]
    async fn change_proposal_persists_payload_and_preview() -> anyhow::Result<()> {
        let (_temp, relational) = test_relational()?;
        let payload = serde_json::json!({
            "roleId": "role-1",
            "displayName": "New Name"
        });
        let proposal = ArchitectureRoleChangeProposal {
            repo_id: "repo-1".to_string(),
            proposal_id: proposal_id("repo-1", "role_rename", &payload),
            proposal_kind: "role_rename".to_string(),
            status: ProposalStatus::Previewed,
            payload,
            impact_preview: serde_json::json!({ "affectedAssignments": 0 }),
            provenance: serde_json::json!({ "source": "test" }),
        };

        insert_change_proposal(&relational, &proposal).await?;

        let rows = relational
            .query_rows(
                "SELECT proposal_kind, status, payload_json, impact_preview_json FROM architecture_role_change_proposals",
            )
            .await?;
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["proposal_kind"], "role_rename");
        assert_eq!(rows[0]["status"], "previewed");
        let payload: serde_json::Value = serde_json::from_str(
            rows[0]["payload_json"]
                .as_str()
                .expect("payload JSON should be stored as text"),
        )?;
        let impact_preview: serde_json::Value = serde_json::from_str(
            rows[0]["impact_preview_json"]
                .as_str()
                .expect("impact preview JSON should be stored as text"),
        )?;
        assert_eq!(payload["displayName"], "New Name");
        assert_eq!(impact_preview["affectedAssignments"], 0);
        Ok(())
    }
}
