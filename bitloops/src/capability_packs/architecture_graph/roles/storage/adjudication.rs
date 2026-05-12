use anyhow::{Context, Result, anyhow};
use serde_json::{Value, json};

use crate::capability_packs::architecture_graph::roles::contracts::{
    AdjudicationOutcome, RoleAdjudicationFailure, RoleAdjudicationProvenance,
    RoleAdjudicationRequest, RoleAssignmentWriteEvent, RoleAssignmentWriteOutcome,
    RoleAssignmentWriter, RoleBoxFuture, RoleCandidateDescriptor, RoleFactsBundle, RoleFactsReader,
    RoleTaxonomyReader, RuleSignalFact,
};
use crate::capability_packs::architecture_graph::roles::taxonomy::{
    ArchitectureRoleAssignment, AssignmentPriority, AssignmentSource, AssignmentStatus, RoleTarget,
    TargetKind, assignment_id,
};
use crate::host::devql::RelationalStorage;

use super::assignments::{
    load_current_assignment_by_id, record_assignment_history, upsert_assignment,
};
use super::rows::sql_text;

pub struct DbRoleTaxonomyReader<'a> {
    relational: &'a RelationalStorage,
}

impl<'a> DbRoleTaxonomyReader<'a> {
    pub fn new(relational: &'a RelationalStorage) -> Self {
        Self { relational }
    }
}

impl RoleTaxonomyReader for DbRoleTaxonomyReader<'_> {
    fn load_active_roles<'a>(
        &'a self,
        repo_id: &'a str,
        _generation: u64,
    ) -> RoleBoxFuture<'a, Vec<RoleCandidateDescriptor>> {
        Box::pin(async move {
            let rows = self
                .relational
                .query_rows(&format!(
                    "SELECT role_id, canonical_key, family, display_name, description
                     FROM architecture_roles
                     WHERE repo_id = {} AND lifecycle_status = 'active'
                     ORDER BY family ASC, canonical_key ASC, role_id ASC;",
                    sql_text(repo_id)
                ))
                .await
                .context("loading active architecture role taxonomy")?;
            rows.into_iter()
                .map(role_candidate_descriptor_from_row)
                .collect::<Result<Vec<_>>>()
        })
    }
}

pub struct DbRoleFactsReader<'a> {
    relational: &'a RelationalStorage,
}

impl<'a> DbRoleFactsReader<'a> {
    pub fn new(relational: &'a RelationalStorage) -> Self {
        Self { relational }
    }
}

impl RoleFactsReader for DbRoleFactsReader<'_> {
    fn load_facts<'a>(
        &'a self,
        request: &'a RoleAdjudicationRequest,
    ) -> RoleBoxFuture<'a, RoleFactsBundle> {
        Box::pin(async move {
            let target_predicate = target_predicate_sql(request);
            let facts = self
                .relational
                .query_rows(&format!(
                    "SELECT fact_id, target_kind, artefact_id, symbol_id, path, language,
                            fact_kind, fact_key, fact_value, source, confidence, evidence_json,
                            generation_seq
                     FROM architecture_artefact_facts_current
                     WHERE repo_id = {} AND ({target_predicate})
                     ORDER BY fact_kind ASC, fact_key ASC, fact_value ASC;",
                    sql_text(&request.repo_id),
                ))
                .await
                .context("loading architecture role adjudication facts")?
                .into_iter()
                .map(fact_row_json)
                .collect::<Result<Vec<_>>>()?;

            let rule_signals = self
                .relational
                .query_rows(&format!(
                    "SELECT rule_id, polarity, score, evidence_json
                     FROM architecture_role_rule_signals_current
                     WHERE repo_id = {} AND ({target_predicate})
                     ORDER BY rule_id ASC, polarity ASC;",
                    sql_text(&request.repo_id),
                ))
                .await
                .context("loading architecture role adjudication rule signals")?
                .into_iter()
                .map(rule_signal_from_row)
                .collect::<Result<Vec<_>>>()?;

            Ok(RoleFactsBundle {
                facts,
                rule_signals,
                dependency_context: Vec::new(),
                related_artefacts: Vec::new(),
                source_snippets: Vec::new(),
                reachability: None,
            })
        })
    }
}

pub struct DbRoleAssignmentWriter<'a> {
    relational: &'a RelationalStorage,
}

impl<'a> DbRoleAssignmentWriter<'a> {
    pub fn new(relational: &'a RelationalStorage) -> Self {
        Self { relational }
    }
}

impl RoleAssignmentWriter for DbRoleAssignmentWriter<'_> {
    fn apply_llm_assignment<'a>(
        &'a self,
        event: RoleAssignmentWriteEvent,
    ) -> RoleBoxFuture<'a, RoleAssignmentWriteOutcome> {
        Box::pin(async move {
            if event.result.outcome != AdjudicationOutcome::Assigned
                || event.result.assignments.is_empty()
            {
                return self
                    .mark_needs_review(
                        &event.request,
                        &RoleAdjudicationFailure {
                            message: "LLM adjudication did not return an assignment".to_string(),
                            retryable: false,
                        },
                        &event.provenance,
                    )
                    .await;
            }

            let target = target_from_request(&event.request)?;
            let mut persisted = 0usize;
            for (index, decision) in event.result.assignments.iter().enumerate() {
                let priority = if decision.primary || index == 0 {
                    AssignmentPriority::Primary
                } else {
                    AssignmentPriority::Secondary
                };
                let assignment = ArchitectureRoleAssignment {
                    repo_id: event.request.repo_id.clone(),
                    assignment_id: assignment_id(
                        &event.request.repo_id,
                        &decision.role_id,
                        &target,
                    ),
                    role_id: decision.role_id.clone(),
                    target: target.clone(),
                    priority,
                    status: AssignmentStatus::Active,
                    source: AssignmentSource::Llm,
                    confidence: decision.confidence,
                    evidence: json!({
                        "decisionEvidence": decision.evidence,
                        "resultEvidence": event.result.evidence,
                        "reasoningSummary": event.result.reasoning_summary,
                    }),
                    provenance: serde_json::to_value(&event.provenance)?,
                    classifier_version: event.provenance.slot_name.clone(),
                    rule_version: None,
                    generation_seq: event.request.generation,
                };
                persist_assignment_with_history(self.relational, assignment, "llm_adjudication")
                    .await?;
                persisted += 1;
            }
            Ok(RoleAssignmentWriteOutcome {
                source: "db",
                persisted: persisted > 0,
            })
        })
    }

    fn mark_needs_review<'a>(
        &'a self,
        request: &'a RoleAdjudicationRequest,
        failure: &'a RoleAdjudicationFailure,
        provenance: &'a RoleAdjudicationProvenance,
    ) -> RoleBoxFuture<'a, RoleAssignmentWriteOutcome> {
        Box::pin(async move {
            let Some(role_id) = request
                .current_assignment
                .as_ref()
                .map(|assignment| assignment.role_id.clone())
                .or_else(|| request.candidate_role_ids.first().cloned())
            else {
                return Ok(RoleAssignmentWriteOutcome {
                    source: "db",
                    persisted: false,
                });
            };

            let target = target_from_request(request)?;
            let assignment = ArchitectureRoleAssignment {
                repo_id: request.repo_id.clone(),
                assignment_id: assignment_id(&request.repo_id, &role_id, &target),
                role_id,
                target,
                priority: AssignmentPriority::Primary,
                status: AssignmentStatus::NeedsReview,
                source: AssignmentSource::Llm,
                confidence: request.deterministic_confidence.unwrap_or(0.0),
                evidence: json!({
                    "failure": failure.message,
                    "retryable": failure.retryable,
                }),
                provenance: serde_json::to_value(provenance)?,
                classifier_version: provenance.slot_name.clone(),
                rule_version: None,
                generation_seq: request.generation,
            };
            persist_assignment_with_history(
                self.relational,
                assignment,
                "llm_adjudication_needs_review",
            )
            .await?;
            Ok(RoleAssignmentWriteOutcome {
                source: "db",
                persisted: true,
            })
        })
    }
}

async fn persist_assignment_with_history(
    relational: &RelationalStorage,
    assignment: ArchitectureRoleAssignment,
    change_kind: &str,
) -> Result<()> {
    let previous =
        load_current_assignment_by_id(relational, &assignment.repo_id, &assignment.assignment_id)
            .await?;
    record_assignment_history(relational, previous.as_ref(), &assignment, change_kind).await?;
    upsert_assignment(relational, &assignment).await
}

fn target_from_request(request: &RoleAdjudicationRequest) -> Result<RoleTarget> {
    let path = request
        .path
        .clone()
        .unwrap_or_else(|| "<unknown>".to_string());
    match request.target_kind.as_deref() {
        Some("file") => {
            return Ok(RoleTarget::file(path));
        }
        Some("artefact") => {
            let Some(artefact_id) = request.artefact_id.as_ref() else {
                return Err(anyhow!(
                    "role adjudication request target_kind=artefact did not include artefact_id"
                ));
            };
            return Ok(RoleTarget {
                target_kind: TargetKind::Artefact,
                artefact_id: Some(artefact_id.clone()),
                symbol_id: request.symbol_id.clone(),
                path,
            });
        }
        Some("symbol") => {
            let Some(symbol_id) = request.symbol_id.as_ref() else {
                return Err(anyhow!(
                    "role adjudication request target_kind=symbol did not include symbol_id"
                ));
            };
            return Ok(RoleTarget {
                target_kind: TargetKind::Symbol,
                artefact_id: request.artefact_id.clone(),
                symbol_id: Some(symbol_id.clone()),
                path,
            });
        }
        Some(other) => {
            return Err(anyhow!(
                "unsupported role adjudication request target_kind `{other}`"
            ));
        }
        None => {}
    }

    if let Some(symbol_id) = request.symbol_id.as_ref() {
        return Ok(RoleTarget {
            target_kind: TargetKind::Symbol,
            artefact_id: request.artefact_id.clone(),
            symbol_id: Some(symbol_id.clone()),
            path,
        });
    }
    if let Some(artefact_id) = request.artefact_id.as_ref() {
        return Ok(RoleTarget {
            target_kind: TargetKind::Artefact,
            artefact_id: Some(artefact_id.clone()),
            symbol_id: None,
            path,
        });
    }
    if request.path.is_some() {
        return Ok(RoleTarget::file(path));
    }
    Err(anyhow!(
        "role adjudication request did not include a target path, artefact, or symbol"
    ))
}

fn target_predicate_sql(request: &RoleAdjudicationRequest) -> String {
    let mut predicates = Vec::new();
    if let Some(symbol_id) = request.symbol_id.as_deref() {
        predicates.push(format!("symbol_id = {}", sql_text(symbol_id)));
    }
    if let Some(artefact_id) = request.artefact_id.as_deref() {
        predicates.push(format!("artefact_id = {}", sql_text(artefact_id)));
    }
    if let Some(path) = request.path.as_deref() {
        predicates.push(format!("path = {}", sql_text(path)));
    }
    if predicates.is_empty() {
        "1 = 0".to_string()
    } else {
        predicates.join(" OR ")
    }
}

fn fact_row_json(row: Value) -> Result<Value> {
    Ok(json!({
        "factId": string_field(&row, "fact_id")?,
        "targetKind": string_field(&row, "target_kind")?,
        "artefactId": optional_string_field(&row, "artefact_id"),
        "symbolId": optional_string_field(&row, "symbol_id"),
        "path": string_field(&row, "path")?,
        "language": optional_string_field(&row, "language"),
        "kind": string_field(&row, "fact_kind")?,
        "key": string_field(&row, "fact_key")?,
        "value": string_field(&row, "fact_value")?,
        "source": string_field(&row, "source")?,
        "confidence": f64_field(&row, "confidence")?,
        "evidence": json_field(&row, "evidence_json")?,
        "generation": u64_field(&row, "generation_seq")?,
    }))
}

fn rule_signal_from_row(row: Value) -> Result<RuleSignalFact> {
    Ok(RuleSignalFact {
        rule_id: string_field(&row, "rule_id")?,
        polarity: string_field(&row, "polarity")?,
        weight: f64_field(&row, "score")?,
        evidence: json_field(&row, "evidence_json")?,
    })
}

fn role_candidate_descriptor_from_row(row: Value) -> Result<RoleCandidateDescriptor> {
    Ok(RoleCandidateDescriptor {
        role_id: string_field(&row, "role_id")?,
        canonical_key: string_field(&row, "canonical_key")?,
        family: string_field(&row, "family")?,
        display_name: string_field(&row, "display_name")?,
        description: string_field(&row, "description")?,
    })
}

fn string_field(row: &Value, key: &str) -> Result<String> {
    row.get(key)
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .ok_or_else(|| anyhow!("row missing string field `{key}`"))
}

fn optional_string_field(row: &Value, key: &str) -> Option<String> {
    row.get(key)
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .filter(|value| !value.is_empty())
}

fn f64_field(row: &Value, key: &str) -> Result<f64> {
    row.get(key)
        .and_then(Value::as_f64)
        .or_else(|| {
            row.get(key)
                .and_then(Value::as_i64)
                .map(|value| value as f64)
        })
        .ok_or_else(|| anyhow!("row missing float field `{key}`"))
}

fn u64_field(row: &Value, key: &str) -> Result<u64> {
    row.get(key)
        .and_then(Value::as_u64)
        .ok_or_else(|| anyhow!("row missing integer field `{key}`"))
}

fn json_field(row: &Value, key: &str) -> Result<Value> {
    match row.get(key) {
        Some(Value::String(text)) => {
            serde_json::from_str(text).with_context(|| format!("parsing JSON field `{key}`"))
        }
        Some(value) => Ok(value.clone()),
        None => Ok(Value::Null),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capability_packs::architecture_graph::roles::storage::{
        load_current_assignment_by_id, upsert_classification_role, upsert_detection_rule,
    };
    use crate::capability_packs::architecture_graph::roles::taxonomy::{
        ArchitectureRole, ArchitectureRoleDetectionRule, RoleLifecycle, RoleRuleLifecycle,
        RoleSignalPolarity, stable_role_id,
    };
    use crate::capability_packs::architecture_graph::schema::architecture_graph_sqlite_schema_sql;
    use tempfile::TempDir;

    fn test_relational() -> Result<(TempDir, RelationalStorage)> {
        let temp = TempDir::new()?;
        let sqlite_path = temp.path().join("devql.sqlite");
        let conn = rusqlite::Connection::open(&sqlite_path)?;
        conn.execute_batch(architecture_graph_sqlite_schema_sql())?;
        drop(conn);
        Ok((temp, RelationalStorage::local_only(sqlite_path)))
    }

    fn role() -> ArchitectureRole {
        ArchitectureRole {
            repo_id: "repo-1".to_string(),
            role_id: stable_role_id("repo-1", "application", "entrypoint"),
            family: "application".to_string(),
            slug: "entrypoint".to_string(),
            display_name: "Entrypoint".to_string(),
            description: "Entrypoint role".to_string(),
            lifecycle: RoleLifecycle::Active,
            provenance: json!({"source": "test"}),
        }
    }

    #[tokio::test]
    async fn db_taxonomy_reader_loads_active_roles() -> Result<()> {
        let (_temp, relational) = test_relational()?;
        upsert_classification_role(&relational, &role()).await?;

        let roles = DbRoleTaxonomyReader::new(&relational)
            .load_active_roles("repo-1", 1)
            .await?;

        assert_eq!(
            roles
                .iter()
                .map(|role| role.role_id.clone())
                .collect::<std::collections::BTreeSet<_>>(),
            std::collections::BTreeSet::from([role().role_id])
        );
        Ok(())
    }

    #[tokio::test]
    async fn db_taxonomy_reader_ignores_non_active_role_rows() -> Result<()> {
        let (_temp, relational) = test_relational()?;
        let mut active = role();
        active.role_id = stable_role_id("repo-1", "application", "active_entrypoint");
        active.slug = "active_entrypoint".to_string();
        active.display_name = "Active Entrypoint".to_string();
        upsert_classification_role(&relational, &active).await?;

        relational
            .exec_serialized(
                "INSERT INTO architecture_roles (
                    repo_id, role_id, family, canonical_key, display_name, description,
                    lifecycle_status, provenance_json, evidence_json, metadata_json
                ) VALUES (
                    'repo-1', 'stable-role', 'application', 'stable_entrypoint',
                    'Stable Entrypoint', 'Bad legacy row', 'stable', '{}', '{}', '{}'
                );",
            )
            .await?;

        let roles = DbRoleTaxonomyReader::new(&relational)
            .load_active_roles("repo-1", 1)
            .await?;

        assert_eq!(roles.len(), 1);
        assert_eq!(roles[0].role_id, active.role_id);
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn db_role_taxonomy_reader_loads_active_roles_without_blocking_bridge() -> Result<()> {
        let (_temp, relational) = test_relational()?;
        let role = ArchitectureRole {
            repo_id: "repo-async-reader".to_string(),
            role_id: stable_role_id("repo-async-reader", "application", "entrypoint"),
            family: "application".to_string(),
            slug: "entrypoint".to_string(),
            display_name: "Entrypoint".to_string(),
            description: "Entrypoint role".to_string(),
            lifecycle: RoleLifecycle::Active,
            provenance: json!({"source": "test"}),
        };
        upsert_classification_role(&relational, &role).await?;

        let reader = DbRoleTaxonomyReader::new(&relational);
        let roles = reader.load_active_roles("repo-async-reader", 1).await?;

        assert_eq!(
            roles
                .iter()
                .map(|role| role.role_id.clone())
                .collect::<std::collections::BTreeSet<_>>(),
            std::collections::BTreeSet::from([role.role_id])
        );
        Ok(())
    }

    #[tokio::test]
    async fn db_assignment_writer_persists_llm_assignment() -> Result<()> {
        let (_temp, relational) = test_relational()?;
        let role = role();
        upsert_classification_role(&relational, &role).await?;
        let request = request(vec![role.role_id.clone()]);
        let provenance = provenance();

        let outcome = DbRoleAssignmentWriter::new(&relational)
            .apply_llm_assignment(RoleAssignmentWriteEvent {
                request: request.clone(),
                result: crate::capability_packs::architecture_graph::roles::contracts::RoleAdjudicationResult {
                        outcome: AdjudicationOutcome::Assigned,
                        assignments: vec![crate::capability_packs::architecture_graph::roles::contracts::RoleAssignmentDecision {
                            role_id: role.role_id.clone(),
                            primary: true,
                            confidence: 0.91,
                            evidence: json!(["main.rs"]),
                        }],
                        confidence: 0.91,
                        evidence: json!(["signal"]),
                        reasoning_summary: "clear role".to_string(),
                        rule_suggestions: vec![],
                    },
                provenance,
            })
            .await?;

        let target = target_from_request(&request)?;
        let assignment_id = assignment_id("repo-1", &role.role_id, &target);
        let assignment = load_current_assignment_by_id(&relational, "repo-1", &assignment_id)
            .await?
            .expect("assignment");
        assert!(outcome.persisted);
        assert_eq!(assignment.source, AssignmentSource::Llm);
        assert_eq!(assignment.status, AssignmentStatus::Active);
        assert_eq!(assignment.confidence, 0.91);
        Ok(())
    }

    #[tokio::test]
    async fn db_assignment_writer_marks_needs_review() -> Result<()> {
        let (_temp, relational) = test_relational()?;
        let role = role();
        upsert_classification_role(&relational, &role).await?;
        let request = request(vec![role.role_id.clone()]);

        let outcome = DbRoleAssignmentWriter::new(&relational)
            .mark_needs_review(
                &request,
                &RoleAdjudicationFailure {
                    message: "invalid response".to_string(),
                    retryable: false,
                },
                &provenance(),
            )
            .await?;

        let target = target_from_request(&request)?;
        let assignment_id = assignment_id("repo-1", &role.role_id, &target);
        let assignment = load_current_assignment_by_id(&relational, "repo-1", &assignment_id)
            .await?
            .expect("assignment");
        assert!(outcome.persisted);
        assert_eq!(assignment.status, AssignmentStatus::NeedsReview);
        assert_eq!(assignment.source, AssignmentSource::Llm);
        Ok(())
    }

    #[tokio::test]
    async fn db_facts_reader_loads_facts_and_rule_signals() -> Result<()> {
        let (_temp, relational) = test_relational()?;
        let role = role();
        upsert_classification_role(&relational, &role).await?;
        let rule = ArchitectureRoleDetectionRule {
            repo_id: "repo-1".to_string(),
            rule_id: "rule-1".to_string(),
            role_id: role.role_id,
            version: 1,
            lifecycle: RoleRuleLifecycle::Active,
            priority: 10,
            score: 1.0,
            candidate_selector: json!({"targetKinds": ["file"]}),
            positive_conditions: json!([]),
            negative_conditions: json!([]),
            provenance: json!({"source": "test"}),
        };
        upsert_detection_rule(&relational, &rule).await?;
        let target = RoleTarget::file("src/main.rs");
        let fact = crate::capability_packs::architecture_graph::roles::taxonomy::ArchitectureArtefactFact {
            repo_id: "repo-1".to_string(),
            fact_id: "fact-1".to_string(),
            target: target.clone(),
            language: Some("rust".to_string()),
            fact_kind: "path".to_string(),
            fact_key: "segment".to_string(),
            fact_value: "main.rs".to_string(),
            source: "test".to_string(),
            confidence: 1.0,
            evidence: json!([]),
            generation_seq: 1,
        };
        let signal = crate::capability_packs::architecture_graph::roles::taxonomy::ArchitectureRoleRuleSignal {
            repo_id: "repo-1".to_string(),
            signal_id: "signal-1".to_string(),
            rule_id: "rule-1".to_string(),
            rule_version: 1,
            role_id: rule.role_id.clone(),
            target,
            polarity: RoleSignalPolarity::Positive,
            score: 0.8,
            evidence: json!([]),
            generation_seq: 1,
        };
        super::super::facts::replace_facts_for_paths(
            &relational,
            "repo-1",
            &[String::from("src/main.rs")],
            &[fact],
        )
        .await?;
        super::super::signals::replace_signals_for_paths(
            &relational,
            "repo-1",
            &[String::from("src/main.rs")],
            &[signal],
        )
        .await?;

        let bundle = DbRoleFactsReader::new(&relational)
            .load_facts(&request(Vec::new()))
            .await?;

        assert_eq!(bundle.facts.len(), 1);
        assert_eq!(bundle.rule_signals.len(), 1);
        assert_eq!(bundle.rule_signals[0].rule_id, "rule-1");
        Ok(())
    }

    fn request(candidate_role_ids: Vec<String>) -> RoleAdjudicationRequest {
        RoleAdjudicationRequest {
            repo_id: "repo-1".to_string(),
            generation: 7,
            target_kind: Some("artefact".to_string()),
            artefact_id: Some("artefact-1".to_string()),
            symbol_id: Some("symbol-1".to_string()),
            path: Some("src/main.rs".to_string()),
            language: Some("rust".to_string()),
            canonical_kind: Some("function".to_string()),
            reason: crate::capability_packs::architecture_graph::roles::contracts::AdjudicationReason::LowConfidence,
            deterministic_confidence: Some(0.5),
            candidate_role_ids,
            current_assignment: None,
        }
    }

    fn provenance() -> RoleAdjudicationProvenance {
        RoleAdjudicationProvenance {
            source: "llm".to_string(),
            model_descriptor: "fake:model".to_string(),
            slot_name: "role_adjudication".to_string(),
            packet_sha256: "packet".to_string(),
            adjudication_reason: crate::capability_packs::architecture_graph::roles::contracts::AdjudicationReason::LowConfidence,
            adjudicated_at_unix: 1,
        }
    }
}
