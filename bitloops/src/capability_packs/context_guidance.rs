mod descriptor;
pub mod distillation;
mod health;
mod history_input;
mod ingesters;
mod migrations;
mod pack;
mod query_examples;
mod register;
mod schema;
mod stages;
pub mod storage;
pub mod types;
mod workplane;

pub use descriptor::{
    CONTEXT_GUIDANCE_CAPABILITY_ID, CONTEXT_GUIDANCE_DESCRIPTOR,
    CONTEXT_GUIDANCE_HISTORY_DISTILLATION_INGESTER_ID,
    CONTEXT_GUIDANCE_HISTORY_DISTILLATION_MAILBOX, CONTEXT_GUIDANCE_STAGE_ID,
    CONTEXT_GUIDANCE_TEXT_GENERATION_SLOT,
};
pub use pack::ContextGuidancePack;
pub use workplane::{
    ContextGuidanceMailboxPayload, enqueue_history_guidance_distillation,
    enqueue_stored_history_guidance_distillation, history_source_scope_key,
    history_turn_dedupe_key, history_turn_work_item_count,
};

#[cfg(test)]
mod tests {
    use crate::config::InferenceTask;
    use crate::host::capability_host::CapabilityPack;

    use super::descriptor::{
        CONTEXT_GUIDANCE_CAPABILITY_ID, CONTEXT_GUIDANCE_DESCRIPTOR,
        CONTEXT_GUIDANCE_HISTORY_DISTILLATION_INGESTER_ID,
        CONTEXT_GUIDANCE_HISTORY_DISTILLATION_MAILBOX, CONTEXT_GUIDANCE_STAGE_ID,
        CONTEXT_GUIDANCE_TEXT_GENERATION_SLOT,
    };
    use super::pack::ContextGuidancePack;

    #[test]
    fn context_guidance_descriptor_matches_phase_two_contract() -> anyhow::Result<()> {
        let pack = ContextGuidancePack::new()?;
        let descriptor = pack.descriptor();

        assert_eq!(CONTEXT_GUIDANCE_CAPABILITY_ID, "context_guidance");
        assert_eq!(
            CONTEXT_GUIDANCE_HISTORY_DISTILLATION_MAILBOX,
            "context_guidance.history_distillation"
        );
        assert_eq!(
            CONTEXT_GUIDANCE_HISTORY_DISTILLATION_INGESTER_ID,
            "context_guidance.history_distillation"
        );
        assert_eq!(CONTEXT_GUIDANCE_STAGE_ID, "context_guidance");
        assert_eq!(CONTEXT_GUIDANCE_TEXT_GENERATION_SLOT, "guidance_generation");

        assert_eq!(descriptor.id, "context_guidance");
        assert_eq!(descriptor.api_version, 1);
        assert!(descriptor.default_enabled);
        assert!(descriptor.dependencies.is_empty());
        assert_eq!(descriptor.inference_slots.len(), 1);
        assert_eq!(
            descriptor.inference_slots[0].name,
            CONTEXT_GUIDANCE_TEXT_GENERATION_SLOT
        );
        assert_eq!(
            descriptor.inference_slots[0].task,
            InferenceTask::TextGeneration
        );
        assert_eq!(descriptor, &CONTEXT_GUIDANCE_DESCRIPTOR);
        Ok(())
    }

    #[test]
    fn context_guidance_pack_exposes_initial_migration() -> anyhow::Result<()> {
        let pack = ContextGuidancePack::new()?;

        assert!(!pack.migrations().is_empty());
        assert_eq!(pack.migrations()[0].capability_id, "context_guidance");
        Ok(())
    }
}
