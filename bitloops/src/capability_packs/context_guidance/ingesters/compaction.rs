use anyhow::{Result, anyhow, bail};
use serde_json::json;

use crate::host::capability_host::{
    BoxFuture, CapabilityIngestContext, IngestRequest, IngestResult, IngesterHandler,
    IngesterRegistration,
};

use super::super::descriptor::{
    CONTEXT_GUIDANCE_CAPABILITY_ID, CONTEXT_GUIDANCE_TARGET_COMPACTION_INGESTER_ID,
};
use super::super::lifecycle::ApplyTargetCompactionInput;
use super::super::storage::{PersistedGuidanceFact, guidance_hash_for_parts};
use super::super::workplane::{ContextGuidanceMailboxPayload, context_guidance_work_item_count};

pub fn build_context_guidance_target_compaction_ingester() -> IngesterRegistration {
    IngesterRegistration::new(
        CONTEXT_GUIDANCE_CAPABILITY_ID,
        CONTEXT_GUIDANCE_TARGET_COMPACTION_INGESTER_ID,
        std::sync::Arc::new(ContextGuidanceTargetCompactionIngester),
    )
}

struct ContextGuidanceTargetCompactionIngester;

impl IngesterHandler for ContextGuidanceTargetCompactionIngester {
    fn ingest<'a>(
        &'a self,
        request: IngestRequest,
        ctx: &'a mut dyn CapabilityIngestContext,
    ) -> BoxFuture<'a, Result<IngestResult>> {
        Box::pin(async move {
            let payload: ContextGuidanceMailboxPayload = request.parse_json()?;
            match &payload {
                ContextGuidanceMailboxPayload::TargetCompaction {
                    repo_id,
                    target_type,
                    target_value,
                } => {
                    let store = ctx
                        .context_guidance_store()
                        .ok_or_else(|| anyhow!("context guidance store is not available"))?;
                    let facts = store.list_active_guidance_for_target(
                        repo_id,
                        target_type,
                        target_value,
                        100,
                    )?;
                    if facts.len() < 2 {
                        return Ok(IngestResult::new(
                            json!({
                                "accepted": true,
                                "skipped": true,
                                "reason": "fewer_than_two_active_facts",
                                "repoId": repo_id,
                                "targetType": target_type,
                                "targetValue": target_value,
                                "work_item_count": context_guidance_work_item_count(&payload)
                            }),
                            "skipped context guidance target compaction because fewer than two active facts exist",
                        ));
                    }
                    let plan = plan_target_compaction(&facts);
                    if plan.duplicate_guidance_ids.is_empty()
                        && plan.superseded_guidance_ids.is_empty()
                    {
                        return Ok(IngestResult::new(
                            json!({
                                "accepted": true,
                                "skipped": true,
                                "reason": "no_compaction_actions",
                                "repoId": repo_id,
                                "targetType": target_type,
                                "targetValue": target_value,
                                "work_item_count": context_guidance_work_item_count(&payload)
                            }),
                            "skipped context guidance target compaction because no duplicate facts were found",
                        ));
                    }
                    let compaction_run_id =
                        compaction_run_id(repo_id, target_type, target_value, &facts);
                    let outcome = store.apply_target_compaction(
                        repo_id,
                        ApplyTargetCompactionInput {
                            compaction_run_id,
                            target_type: target_type.clone(),
                            target_value: target_value.clone(),
                            retained_guidance_ids: plan.retained_guidance_ids,
                            duplicate_guidance_ids: plan.duplicate_guidance_ids,
                            superseded_guidance_ids: plan.superseded_guidance_ids,
                            summary_json: target_compaction_summary_json(
                                target_type,
                                target_value,
                                &facts,
                            )?,
                        },
                    )?;
                    Ok(IngestResult::new(
                        json!({
                            "accepted": true,
                            "retainedFacts": outcome.retained_count,
                            "compactedFacts": outcome.compacted_count,
                            "repoId": repo_id,
                            "targetType": target_type,
                            "targetValue": target_value,
                            "work_item_count": context_guidance_work_item_count(&payload)
                        }),
                        "completed context guidance target compaction work",
                    ))
                }
                ContextGuidanceMailboxPayload::HistoryTurn { .. }
                | ContextGuidanceMailboxPayload::KnowledgeEvidence(_) => {
                    bail!("target compaction ingester received incompatible payload")
                }
            }
        })
    }
}

struct PlannedTargetCompaction {
    retained_guidance_ids: Vec<String>,
    duplicate_guidance_ids: Vec<String>,
    superseded_guidance_ids: Vec<(String, String)>,
}

fn plan_target_compaction(facts: &[PersistedGuidanceFact]) -> PlannedTargetCompaction {
    let mut retained_guidance_ids = Vec::new();
    let mut duplicate_guidance_ids = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for fact in facts {
        if seen.insert(fact.fact_fingerprint.clone()) {
            retained_guidance_ids.push(fact.guidance_id.clone());
        } else {
            duplicate_guidance_ids.push(fact.guidance_id.clone());
        }
    }
    PlannedTargetCompaction {
        retained_guidance_ids,
        duplicate_guidance_ids,
        superseded_guidance_ids: Vec::new(),
    }
}

fn compaction_run_id(
    repo_id: &str,
    target_type: &str,
    target_value: &str,
    facts: &[PersistedGuidanceFact],
) -> String {
    let mut guidance_ids = facts
        .iter()
        .map(|fact| fact.guidance_id.as_str())
        .collect::<Vec<_>>();
    guidance_ids.sort_unstable();
    let mut parts = vec![repo_id, target_type, target_value];
    parts.extend(guidance_ids);
    format!("compaction-run:{}", guidance_hash_for_parts(&parts))
}

fn target_compaction_summary_json(
    target_type: &str,
    target_value: &str,
    facts: &[PersistedGuidanceFact],
) -> Result<String> {
    Ok(serde_json::to_string(&json!({
        "targetType": target_type,
        "targetValue": target_value,
        "sourceFactCount": facts.len(),
    }))?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capability_packs::context_guidance::storage::{
        PersistedGuidanceSource, PersistedGuidanceTarget,
    };
    use crate::capability_packs::context_guidance::types::{
        GuidanceFactCategory, GuidanceFactConfidence,
    };

    fn fact(id: &str, fingerprint: &str) -> PersistedGuidanceFact {
        PersistedGuidanceFact {
            guidance_id: id.to_string(),
            run_id: "run-1".to_string(),
            repo_id: "repo-1".to_string(),
            active: true,
            category: GuidanceFactCategory::Decision,
            kind: "preserve_parser_boundary".to_string(),
            guidance: "Keep parser boundary centralized in attr parsing helpers.".to_string(),
            evidence_excerpt: "Keep parser boundary centralized in attr parsing helpers."
                .to_string(),
            confidence: GuidanceFactConfidence::High,
            lifecycle_status: "active".to_string(),
            fact_fingerprint: fingerprint.to_string(),
            value_score: 0.99,
            superseded_by_guidance_id: None,
            source_model: None,
            generated_at: None,
            targets: vec![PersistedGuidanceTarget {
                target_type: "path".to_string(),
                target_value: "src/lib.rs".to_string(),
            }],
            sources: vec![PersistedGuidanceSource {
                source_type: "history.turn".to_string(),
                source_id: "source-1".to_string(),
                checkpoint_id: None,
                session_id: None,
                turn_id: None,
                tool_invocation_id: None,
                tool_kind: None,
                event_time: None,
                agent_type: None,
                model: None,
                evidence_kind: None,
                match_strength: None,
                knowledge_item_id: None,
                knowledge_item_version_id: None,
                relation_assertion_id: None,
                provider: None,
                source_kind: None,
                title: None,
                url: None,
                excerpt: None,
            }],
        }
    }

    #[test]
    fn planner_marks_repeated_fingerprints_as_duplicates() {
        let facts = vec![
            fact("guidance-1", "fingerprint-1"),
            fact("guidance-2", "fingerprint-1"),
            fact("guidance-3", "fingerprint-2"),
        ];

        let plan = plan_target_compaction(&facts);

        assert_eq!(plan.retained_guidance_ids, vec!["guidance-1", "guidance-3"]);
        assert_eq!(plan.duplicate_guidance_ids, vec!["guidance-2"]);
        assert!(plan.superseded_guidance_ids.is_empty());
    }
}
