use anyhow::Result;
use std::path::Path;

use crate::host::capability_host::gateways::{CapabilityWorkplaneGateway, CapabilityWorkplaneJob};
use crate::host::runtime_store::{CapabilityWorkplaneJobInsert, RepoSqliteRuntimeStore};

use super::descriptor::{
    CONTEXT_GUIDANCE_CAPABILITY_ID, CONTEXT_GUIDANCE_HISTORY_DISTILLATION_MAILBOX,
    CONTEXT_GUIDANCE_KNOWLEDGE_DISTILLATION_MAILBOX, CONTEXT_GUIDANCE_TARGET_COMPACTION_MAILBOX,
};
use super::distillation::GuidanceDistillationInput;
use super::evidence::{
    GuidanceEvidenceInput, GuidanceEvidenceSource, evidence_target_symbols, knowledge_source_label,
};
use super::history_input::{HistoryGuidanceInputSelector, hydrate_history_guidance_input};
use super::storage::guidance_input_hash;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ContextGuidanceMailboxPayload {
    #[serde(rename_all = "camelCase")]
    HistoryTurn {
        repo_id: String,
        checkpoint_id: Option<String>,
        session_id: String,
        turn_id: Option<String>,
        input_hash: String,
    },
    KnowledgeEvidence(Box<KnowledgeEvidencePayload>),
    #[serde(rename_all = "camelCase")]
    TargetCompaction {
        repo_id: String,
        target_type: String,
        target_value: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeEvidencePayload {
    pub repo_id: String,
    pub knowledge_item_id: String,
    pub knowledge_item_version_id: String,
    pub relation_assertion_id: Option<String>,
    pub provider: String,
    pub source_kind: String,
    pub title: Option<String>,
    pub url: Option<String>,
    pub updated_at: Option<String>,
    pub body_preview: Option<String>,
    pub normalized_fields_json: String,
    pub target_paths: Vec<String>,
    pub target_symbols: Vec<String>,
    pub input_hash: String,
}

pub fn history_turn_dedupe_key(
    session_id: &str,
    turn_id: Option<&str>,
    checkpoint_id: Option<&str>,
    input_hash: &str,
) -> String {
    format!(
        "history_turn:{}:{}:{}:{}",
        session_id,
        turn_id.unwrap_or("_"),
        checkpoint_id.unwrap_or("_"),
        input_hash
    )
}

pub fn history_source_scope_key(
    session_id: &str,
    turn_id: Option<&str>,
    checkpoint_id: Option<&str>,
) -> String {
    format!(
        "history_source:{}:{}:{}",
        session_id,
        turn_id.unwrap_or("_"),
        checkpoint_id.unwrap_or("_")
    )
}

pub fn context_guidance_work_item_count(payload: &ContextGuidanceMailboxPayload) -> u64 {
    match payload {
        ContextGuidanceMailboxPayload::HistoryTurn { .. } => 1,
        ContextGuidanceMailboxPayload::KnowledgeEvidence(payload) => {
            let evidence = knowledge_evidence_input_from_payload(payload);
            let _ = knowledge_source_label(&evidence);
            evidence
                .target_paths
                .len()
                .saturating_add(evidence_target_symbols(&evidence).len())
                .max(1) as u64
        }
        ContextGuidanceMailboxPayload::TargetCompaction { .. } => 1,
    }
}

pub fn history_turn_work_item_count(payload: &ContextGuidanceMailboxPayload) -> u64 {
    context_guidance_work_item_count(payload)
}

fn knowledge_evidence_input_from_payload(
    payload: &KnowledgeEvidencePayload,
) -> GuidanceEvidenceInput {
    GuidanceEvidenceInput {
        source: GuidanceEvidenceSource::Knowledge {
            knowledge_item_id: payload.knowledge_item_id.clone(),
            knowledge_item_version_id: payload.knowledge_item_version_id.clone(),
            relation_assertion_id: payload.relation_assertion_id.clone(),
            provider: payload.provider.clone(),
            source_kind: payload.source_kind.clone(),
            title: payload.title.clone(),
            url: payload.url.clone(),
            updated_at: payload.updated_at.clone(),
        },
        title: payload.title.clone(),
        body: payload.body_preview.clone(),
        prompt: None,
        transcript_fragment: None,
        target_paths: payload.target_paths.clone(),
        target_symbols: payload.target_symbols.clone(),
        tool_events: Vec::new(),
    }
}

pub fn enqueue_history_guidance_distillation(
    repo_id: &str,
    input: &GuidanceDistillationInput,
    workplane: &dyn CapabilityWorkplaneGateway,
) -> Result<()> {
    let input_hash = guidance_input_hash(input);
    let payload = ContextGuidanceMailboxPayload::HistoryTurn {
        repo_id: repo_id.to_string(),
        checkpoint_id: input.checkpoint_id.clone(),
        session_id: input.session_id.clone(),
        turn_id: input.turn_id.clone(),
        input_hash: input_hash.clone(),
    };
    let dedupe_key = history_turn_dedupe_key(
        input.session_id.as_str(),
        input.turn_id.as_deref(),
        input.checkpoint_id.as_deref(),
        input_hash.as_str(),
    );
    workplane.enqueue_jobs(vec![CapabilityWorkplaneJob::new(
        CONTEXT_GUIDANCE_HISTORY_DISTILLATION_MAILBOX,
        Some(dedupe_key),
        serde_json::to_value(payload)?,
    )])?;
    Ok(())
}

pub fn enqueue_knowledge_guidance_distillation(
    workplane: &dyn CapabilityWorkplaneGateway,
    repo_id: &str,
    payload: ContextGuidanceMailboxPayload,
) -> Result<()> {
    let dedupe_key = match &payload {
        ContextGuidanceMailboxPayload::KnowledgeEvidence(payload) => format!(
            "knowledge_evidence:{}:{}:{}",
            payload.knowledge_item_version_id,
            payload.relation_assertion_id.as_deref().unwrap_or("_"),
            payload.input_hash
        ),
        ContextGuidanceMailboxPayload::HistoryTurn { .. }
        | ContextGuidanceMailboxPayload::TargetCompaction { .. } => {
            format!("knowledge_evidence:{repo_id}")
        }
    };
    workplane.enqueue_jobs(vec![CapabilityWorkplaneJob::new_for_capability(
        CONTEXT_GUIDANCE_CAPABILITY_ID,
        CONTEXT_GUIDANCE_KNOWLEDGE_DISTILLATION_MAILBOX,
        Some(dedupe_key),
        serde_json::to_value(payload)?,
    )])?;
    Ok(())
}

pub fn enqueue_target_compaction(
    workplane: &dyn CapabilityWorkplaneGateway,
    repo_id: &str,
    target_type: &str,
    target_value: &str,
) -> Result<()> {
    let payload = ContextGuidanceMailboxPayload::TargetCompaction {
        repo_id: repo_id.to_string(),
        target_type: target_type.to_string(),
        target_value: target_value.to_string(),
    };
    let dedupe_key = format!("target_compaction:{repo_id}:{target_type}:{target_value}");
    workplane.enqueue_jobs(vec![CapabilityWorkplaneJob::new_for_capability(
        CONTEXT_GUIDANCE_CAPABILITY_ID,
        CONTEXT_GUIDANCE_TARGET_COMPACTION_MAILBOX,
        Some(dedupe_key),
        serde_json::to_value(payload)?,
    )])?;
    Ok(())
}

pub fn enqueue_stored_history_guidance_distillation(
    repo_root: &Path,
    repo_id: &str,
    session_id: &str,
    turn_id: Option<&str>,
    checkpoint_id: Option<&str>,
) -> Result<()> {
    let input = hydrate_history_guidance_input(
        repo_root,
        HistoryGuidanceInputSelector {
            repo_id,
            checkpoint_id,
            session_id,
            turn_id,
        },
    )?;
    let input_hash = guidance_input_hash(&input);
    let payload = ContextGuidanceMailboxPayload::HistoryTurn {
        repo_id: repo_id.to_string(),
        checkpoint_id: input.checkpoint_id.clone(),
        session_id: input.session_id.clone(),
        turn_id: input.turn_id.clone(),
        input_hash: input_hash.clone(),
    };
    let dedupe_key = history_turn_dedupe_key(
        input.session_id.as_str(),
        input.turn_id.as_deref(),
        input.checkpoint_id.as_deref(),
        input_hash.as_str(),
    );
    let runtime_store = RepoSqliteRuntimeStore::open(repo_root)?;
    runtime_store.enqueue_capability_workplane_jobs(
        CONTEXT_GUIDANCE_CAPABILITY_ID,
        vec![CapabilityWorkplaneJobInsert::new(
            CONTEXT_GUIDANCE_HISTORY_DISTILLATION_MAILBOX,
            Some(input.session_id),
            Some(dedupe_key),
            serde_json::to_value(payload)?,
        )],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::Mutex;

    use serde_json::json;

    use super::{
        ContextGuidanceMailboxPayload, KnowledgeEvidencePayload, context_guidance_work_item_count,
        enqueue_history_guidance_distillation, history_source_scope_key, history_turn_dedupe_key,
        history_turn_work_item_count,
    };
    use crate::capability_packs::context_guidance::distillation::{
        GuidanceDistillationInput, GuidanceToolEvidence,
    };
    use crate::capability_packs::context_guidance::storage::guidance_input_hash;
    use crate::host::capability_host::gateways::{
        CapabilityMailboxStatus, CapabilityWorkplaneEnqueueResult, CapabilityWorkplaneGateway,
        CapabilityWorkplaneJob,
    };

    struct CapturingWorkplane {
        jobs: Mutex<Vec<CapabilityWorkplaneJob>>,
    }

    impl CapabilityWorkplaneGateway for CapturingWorkplane {
        fn enqueue_jobs(
            &self,
            jobs: Vec<CapabilityWorkplaneJob>,
        ) -> anyhow::Result<CapabilityWorkplaneEnqueueResult> {
            let inserted_jobs = jobs.len() as u64;
            self.jobs.lock().expect("jobs").extend(jobs);
            Ok(CapabilityWorkplaneEnqueueResult {
                inserted_jobs,
                updated_jobs: 0,
            })
        }

        fn mailbox_status(&self) -> anyhow::Result<BTreeMap<String, CapabilityMailboxStatus>> {
            Ok(BTreeMap::new())
        }
    }

    fn distillation_input() -> GuidanceDistillationInput {
        GuidanceDistillationInput {
            checkpoint_id: Some("checkpoint-historical-primary".to_string()),
            session_id: "session-historical".to_string(),
            turn_id: Some("turn-historical-primary".to_string()),
            event_time: Some("2026-03-26T09:10:00Z".to_string()),
            agent_type: Some("codex".to_string()),
            model: Some("gpt-5.4".to_string()),
            prompt: Some("Improve attr parsing".to_string()),
            transcript_fragment: Some("Captured transcript".to_string()),
            files_modified: vec!["src/target.ts".to_string()],
            tool_events: vec![GuidanceToolEvidence {
                tool_kind: Some("shell".to_string()),
                input_summary: Some("cargo nextest".to_string()),
                output_summary: Some("nextest passed".to_string()),
                command: Some("cargo nextest run --lib context_guidance".to_string()),
            }],
        }
    }

    #[test]
    fn history_turn_dedupe_key_is_stable_for_same_source_and_input() {
        let left =
            history_turn_dedupe_key("session-1", Some("turn-1"), Some("checkpoint-1"), "abc");
        let right =
            history_turn_dedupe_key("session-1", Some("turn-1"), Some("checkpoint-1"), "abc");

        assert_eq!(left, right);
    }

    #[test]
    fn history_turn_dedupe_key_changes_with_input_hash() {
        let left =
            history_turn_dedupe_key("session-1", Some("turn-1"), Some("checkpoint-1"), "abc");
        let right =
            history_turn_dedupe_key("session-1", Some("turn-1"), Some("checkpoint-1"), "def");

        assert_ne!(left, right);
    }

    #[test]
    fn history_source_scope_key_excludes_input_hash() {
        let expected = history_source_scope_key("session-1", Some("turn-1"), Some("checkpoint-1"));
        let payload = ContextGuidanceMailboxPayload::HistoryTurn {
            repo_id: "repo-1".to_string(),
            checkpoint_id: Some("checkpoint-1".to_string()),
            session_id: "session-1".to_string(),
            turn_id: Some("turn-1".to_string()),
            input_hash: "abc".to_string(),
        };
        let alternate_payload = ContextGuidanceMailboxPayload::HistoryTurn {
            repo_id: "repo-1".to_string(),
            checkpoint_id: Some("checkpoint-1".to_string()),
            session_id: "session-1".to_string(),
            turn_id: Some("turn-1".to_string()),
            input_hash: "def".to_string(),
        };

        assert_eq!(
            expected,
            history_source_scope_key("session-1", Some("turn-1"), Some("checkpoint-1"))
        );
        assert_eq!(
            history_turn_work_item_count(&payload),
            history_turn_work_item_count(&alternate_payload)
        );
    }

    #[test]
    fn history_turn_work_item_count_is_one() {
        let payload = ContextGuidanceMailboxPayload::HistoryTurn {
            repo_id: "repo-1".to_string(),
            checkpoint_id: None,
            session_id: "session-1".to_string(),
            turn_id: None,
            input_hash: "abc".to_string(),
        };

        assert_eq!(history_turn_work_item_count(&payload), 1);
    }

    #[test]
    fn knowledge_evidence_payload_counts_targeted_work_items() {
        let payload =
            ContextGuidanceMailboxPayload::KnowledgeEvidence(Box::new(KnowledgeEvidencePayload {
                repo_id: "repo-1".to_string(),
                knowledge_item_id: "item-1".to_string(),
                knowledge_item_version_id: "version-1".to_string(),
                relation_assertion_id: Some("relation-1".to_string()),
                provider: "github".to_string(),
                source_kind: "github_issue".to_string(),
                title: Some("Issue title".to_string()),
                url: Some("https://github.com/org/repo/issues/1".to_string()),
                updated_at: Some("2026-04-30T10:00:00Z".to_string()),
                body_preview: Some("Decision context".to_string()),
                normalized_fields_json: "{}".to_string(),
                target_paths: vec!["src/lib.rs".to_string()],
                target_symbols: vec!["crate::lib::run".to_string()],
                input_hash: "hash-1".to_string(),
            }));

        assert_eq!(context_guidance_work_item_count(&payload), 2);
    }

    #[test]
    fn history_turn_payload_round_trips_optional_ids() -> anyhow::Result<()> {
        let payload = ContextGuidanceMailboxPayload::HistoryTurn {
            repo_id: "repo-1".to_string(),
            checkpoint_id: Some("checkpoint-1".to_string()),
            session_id: "session-1".to_string(),
            turn_id: Some("turn-1".to_string()),
            input_hash: "abc".to_string(),
        };

        let encoded = serde_json::to_value(&payload)?;
        assert_eq!(
            encoded,
            json!({
                "historyTurn": {
                    "repoId": "repo-1",
                    "checkpointId": "checkpoint-1",
                    "sessionId": "session-1",
                    "turnId": "turn-1",
                    "inputHash": "abc"
                }
            })
        );
        let decoded: ContextGuidanceMailboxPayload = serde_json::from_value(encoded)?;
        assert_eq!(decoded, payload);
        Ok(())
    }

    #[test]
    fn enqueue_history_guidance_distillation_enqueues_one_deduped_job() -> anyhow::Result<()> {
        let input = distillation_input();
        let workplane = CapturingWorkplane {
            jobs: Mutex::new(Vec::new()),
        };

        enqueue_history_guidance_distillation("repo-1", &input, &workplane)?;

        let jobs = workplane.jobs.lock().expect("jobs");
        assert_eq!(jobs.len(), 1);
        assert_eq!(
            jobs[0].mailbox_name,
            "context_guidance.history_distillation"
        );
        let expected_dedupe_key = history_turn_dedupe_key(
            "session-historical",
            Some("turn-historical-primary"),
            Some("checkpoint-historical-primary"),
            guidance_input_hash(&input).as_str(),
        );
        assert_eq!(
            jobs[0].dedupe_key.as_deref(),
            Some(expected_dedupe_key.as_str())
        );
        assert_eq!(
            jobs[0].payload,
            json!({
                "historyTurn": {
                    "repoId": "repo-1",
                    "checkpointId": "checkpoint-historical-primary",
                    "sessionId": "session-historical",
                    "turnId": "turn-historical-primary",
                    "inputHash": guidance_input_hash(&input)
                }
            })
        );
        Ok(())
    }
}
