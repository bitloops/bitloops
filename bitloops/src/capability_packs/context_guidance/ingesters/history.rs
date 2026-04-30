use anyhow::{Context, Result, anyhow, bail};
use serde_json::json;

use crate::host::capability_host::{
    BoxFuture, CapabilityIngestContext, IngestRequest, IngestResult, IngesterHandler,
    IngesterRegistration,
};

use super::super::descriptor::{
    CONTEXT_GUIDANCE_CAPABILITY_ID, CONTEXT_GUIDANCE_HISTORY_DISTILLATION_INGESTER_ID,
    CONTEXT_GUIDANCE_TEXT_GENERATION_SLOT,
};
use super::super::distillation::GuidanceDistiller;
use super::super::history_input::{HistoryGuidanceInputSelector, hydrate_history_guidance_input};
use super::super::storage::{PersistGuidanceOutcome, guidance_input_hash};
use super::super::workplane::{
    ContextGuidanceMailboxPayload, enqueue_target_compaction, history_turn_work_item_count,
};

pub fn build_context_guidance_history_distillation_ingester() -> IngesterRegistration {
    IngesterRegistration::new(
        CONTEXT_GUIDANCE_CAPABILITY_ID,
        CONTEXT_GUIDANCE_HISTORY_DISTILLATION_INGESTER_ID,
        std::sync::Arc::new(ContextGuidanceHistoryDistillationIngester),
    )
}

struct ContextGuidanceHistoryDistillationIngester;

impl IngesterHandler for ContextGuidanceHistoryDistillationIngester {
    fn ingest<'a>(
        &'a self,
        request: IngestRequest,
        ctx: &'a mut dyn CapabilityIngestContext,
    ) -> BoxFuture<'a, Result<IngestResult>> {
        Box::pin(async move {
            let payload: ContextGuidanceMailboxPayload = request.parse_json()?;
            let ContextGuidanceMailboxPayload::HistoryTurn {
                repo_id,
                checkpoint_id,
                session_id,
                turn_id,
                input_hash,
            } = &payload
            else {
                bail!("history distillation ingester received incompatible payload");
            };
            let input = hydrate_history_guidance_input(
                ctx.repo_root(),
                HistoryGuidanceInputSelector {
                    repo_id,
                    checkpoint_id: checkpoint_id.as_deref(),
                    session_id,
                    turn_id: turn_id.as_deref(),
                },
            )?;
            let expected_input_hash = guidance_input_hash(&input);
            if input_hash != &expected_input_hash {
                log::warn!(
                    "context guidance history distillation input hash changed before execution: repo_id={} queued_hash={} current_hash={}",
                    repo_id,
                    input_hash,
                    expected_input_hash
                );
            }
            let service = match ctx
                .inference()
                .text_generation(CONTEXT_GUIDANCE_TEXT_GENERATION_SLOT)
            {
                Ok(service) => service,
                Err(err) => {
                    log::warn!(
                        "context guidance history distillation skipped: repo_id={} reason={err:#}",
                        repo_id
                    );
                    return Ok(IngestResult::new(
                        json!({
                            "accepted": true,
                            "skipped": true,
                            "reason": "text_generation_unavailable",
                            "work_item_count": history_turn_work_item_count(&payload)
                        }),
                        "skipped context guidance history distillation because text generation is unavailable",
                    ));
                }
            };
            let service_descriptor = service.descriptor();
            let slot = ctx
                .inference()
                .describe(CONTEXT_GUIDANCE_TEXT_GENERATION_SLOT);
            let output = GuidanceDistiller::new(service)
                .distill(&input)
                .context("distilling context guidance history")?;
            let store = ctx
                .context_guidance_store()
                .ok_or_else(|| anyhow!("context guidance store is not available"))?;
            let source_model = slot
                .as_ref()
                .and_then(|slot| slot.model.clone())
                .or(Some(service_descriptor));
            let source_profile = slot.as_ref().map(|slot| slot.profile_name.clone());
            let outcome = store.persist_history_guidance_distillation(
                repo_id,
                &input,
                &output,
                source_model.as_deref(),
                source_profile.as_deref(),
            )?;
            enqueue_target_compactions(repo_id, &outcome, ctx.workplane())?;
            Ok(IngestResult::new(
                json!({
                    "accepted": true,
                    "insertedRun": outcome.inserted_run,
                    "insertedFacts": outcome.inserted_facts,
                    "unchanged": outcome.unchanged,
                    "work_item_count": history_turn_work_item_count(&payload)
                }),
                "completed context guidance history distillation work",
            ))
        })
    }
}

fn enqueue_target_compactions(
    repo_id: &str,
    outcome: &PersistGuidanceOutcome,
    workplane: Option<&dyn crate::host::capability_host::gateways::CapabilityWorkplaneGateway>,
) -> Result<()> {
    if outcome.inserted_facts == 0 {
        return Ok(());
    }
    let Some(workplane) = workplane else {
        return Ok(());
    };
    for target in &outcome.touched_targets {
        enqueue_target_compaction(
            workplane,
            repo_id,
            target.target_type.as_str(),
            target.target_value.as_str(),
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::Mutex;

    use serde_json::json;

    use super::*;
    use crate::capability_packs::context_guidance::storage::PersistedGuidanceTarget;
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

    #[test]
    fn inserted_history_guidance_enqueues_target_compaction() -> anyhow::Result<()> {
        let workplane = CapturingWorkplane {
            jobs: Mutex::new(Vec::new()),
        };
        let outcome = PersistGuidanceOutcome {
            inserted_run: true,
            inserted_facts: 1,
            unchanged: false,
            touched_targets: vec![PersistedGuidanceTarget {
                target_type: "path".to_string(),
                target_value: "src/target.rs".to_string(),
            }],
        };

        enqueue_target_compactions("repo-1", &outcome, Some(&workplane))?;

        let jobs = workplane.jobs.lock().expect("jobs");
        assert_eq!(jobs.len(), 1);
        let queued = &jobs[0];
        assert_eq!(queued.mailbox_name, "context_guidance.target_compaction");
        assert_eq!(
            queued.target_capability_id.as_deref(),
            Some("context_guidance")
        );
        assert!(queued.payload.to_string().contains("src/target.rs"));
        assert_eq!(
            queued.payload,
            json!({
                "targetCompaction": {
                    "repoId": "repo-1",
                    "targetType": "path",
                    "targetValue": "src/target.rs"
                }
            })
        );
        Ok(())
    }
}
