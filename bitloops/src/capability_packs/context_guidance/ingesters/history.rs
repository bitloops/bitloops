use anyhow::{Context, Result, anyhow};
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
use super::super::storage::guidance_input_hash;
use super::super::workplane::{ContextGuidanceMailboxPayload, history_turn_work_item_count};

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
            } = &payload;
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
