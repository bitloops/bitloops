use anyhow::Result;

use crate::host::capability_host::{
    CapabilityMailboxHandler, CapabilityMailboxPolicy, CapabilityMailboxReadinessPolicy,
    CapabilityMailboxRegistration, CapabilityRegistrar,
};

use super::descriptor::{
    CONTEXT_GUIDANCE_CAPABILITY_ID, CONTEXT_GUIDANCE_HISTORY_DISTILLATION_INGESTER_ID,
    CONTEXT_GUIDANCE_HISTORY_DISTILLATION_MAILBOX,
    CONTEXT_GUIDANCE_KNOWLEDGE_DISTILLATION_INGESTER_ID,
    CONTEXT_GUIDANCE_KNOWLEDGE_DISTILLATION_MAILBOX,
    CONTEXT_GUIDANCE_TARGET_COMPACTION_INGESTER_ID, CONTEXT_GUIDANCE_TARGET_COMPACTION_MAILBOX,
    CONTEXT_GUIDANCE_TEXT_GENERATION_SLOT,
};
use super::ingesters::{
    build_context_guidance_history_distillation_ingester,
    build_context_guidance_knowledge_distillation_ingester,
    build_context_guidance_target_compaction_ingester,
};
use super::query_examples::CONTEXT_GUIDANCE_QUERY_EXAMPLES;
use super::schema::CONTEXT_GUIDANCE_SCHEMA_MODULE;
use super::stages::build_context_guidance_stage;

pub fn register_context_guidance_pack(registrar: &mut dyn CapabilityRegistrar) -> Result<()> {
    registrar.register_stage(build_context_guidance_stage())?;
    registrar.register_ingester(build_context_guidance_history_distillation_ingester())?;
    registrar.register_ingester(build_context_guidance_knowledge_distillation_ingester())?;
    registrar.register_ingester(build_context_guidance_target_compaction_ingester())?;
    registrar.register_mailbox(
        CapabilityMailboxRegistration::new(
            CONTEXT_GUIDANCE_CAPABILITY_ID,
            CONTEXT_GUIDANCE_HISTORY_DISTILLATION_MAILBOX,
            CapabilityMailboxPolicy::Job,
            CapabilityMailboxHandler::Ingester(CONTEXT_GUIDANCE_HISTORY_DISTILLATION_INGESTER_ID),
        )
        .readiness_policy(CapabilityMailboxReadinessPolicy::TextGenerationSlot(
            CONTEXT_GUIDANCE_TEXT_GENERATION_SLOT,
        )),
    )?;
    registrar.register_mailbox(
        CapabilityMailboxRegistration::new(
            CONTEXT_GUIDANCE_CAPABILITY_ID,
            CONTEXT_GUIDANCE_KNOWLEDGE_DISTILLATION_MAILBOX,
            CapabilityMailboxPolicy::Job,
            CapabilityMailboxHandler::Ingester(CONTEXT_GUIDANCE_KNOWLEDGE_DISTILLATION_INGESTER_ID),
        )
        .readiness_policy(CapabilityMailboxReadinessPolicy::TextGenerationSlot(
            CONTEXT_GUIDANCE_TEXT_GENERATION_SLOT,
        )),
    )?;
    registrar.register_mailbox(
        CapabilityMailboxRegistration::new(
            CONTEXT_GUIDANCE_CAPABILITY_ID,
            CONTEXT_GUIDANCE_TARGET_COMPACTION_MAILBOX,
            CapabilityMailboxPolicy::Job,
            CapabilityMailboxHandler::Ingester(CONTEXT_GUIDANCE_TARGET_COMPACTION_INGESTER_ID),
        )
        .readiness_policy(CapabilityMailboxReadinessPolicy::TextGenerationSlot(
            CONTEXT_GUIDANCE_TEXT_GENERATION_SLOT,
        )),
    )?;
    registrar.register_schema_module(CONTEXT_GUIDANCE_SCHEMA_MODULE)?;
    registrar.register_query_examples(CONTEXT_GUIDANCE_QUERY_EXAMPLES)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use anyhow::Result;

    use crate::host::capability_host::{
        CapabilityMailboxBacklogPolicy, CapabilityMailboxHandler, CapabilityMailboxPolicy,
        CapabilityMailboxReadinessPolicy, CapabilityMailboxRegistration, CapabilityRegistrar,
        IngesterRegistration, QueryExample, SchemaModule, StageRegistration,
    };

    use super::super::descriptor::{
        CONTEXT_GUIDANCE_CAPABILITY_ID, CONTEXT_GUIDANCE_HISTORY_DISTILLATION_INGESTER_ID,
        CONTEXT_GUIDANCE_HISTORY_DISTILLATION_MAILBOX,
        CONTEXT_GUIDANCE_KNOWLEDGE_DISTILLATION_INGESTER_ID,
        CONTEXT_GUIDANCE_KNOWLEDGE_DISTILLATION_MAILBOX, CONTEXT_GUIDANCE_STAGE_ID,
        CONTEXT_GUIDANCE_TARGET_COMPACTION_INGESTER_ID, CONTEXT_GUIDANCE_TARGET_COMPACTION_MAILBOX,
        CONTEXT_GUIDANCE_TEXT_GENERATION_SLOT,
    };
    use super::register_context_guidance_pack;

    #[derive(Default)]
    struct CollectingRegistrar {
        stages: Vec<(&'static str, &'static str)>,
        ingesters: Vec<(&'static str, &'static str)>,
        mailboxes: Vec<CapabilityMailboxRegistration>,
        schema_modules: Vec<SchemaModule>,
        query_examples: Vec<QueryExample>,
    }

    impl CapabilityRegistrar for CollectingRegistrar {
        fn register_stage(&mut self, stage: StageRegistration) -> Result<()> {
            self.stages.push((stage.capability_id, stage.stage_name));
            Ok(())
        }

        fn register_ingester(&mut self, ingester: IngesterRegistration) -> Result<()> {
            self.ingesters
                .push((ingester.capability_id, ingester.ingester_name));
            Ok(())
        }

        fn register_mailbox(&mut self, registration: CapabilityMailboxRegistration) -> Result<()> {
            self.mailboxes.push(registration);
            Ok(())
        }

        fn register_schema_module(&mut self, module: SchemaModule) -> Result<()> {
            self.schema_modules.push(module);
            Ok(())
        }

        fn register_query_examples(&mut self, examples: &'static [QueryExample]) -> Result<()> {
            self.query_examples.extend_from_slice(examples);
            Ok(())
        }
    }

    #[test]
    fn register_context_guidance_pack_registers_phase_two_contributions() -> Result<()> {
        let mut registrar = CollectingRegistrar::default();

        register_context_guidance_pack(&mut registrar)?;

        assert_eq!(
            registrar.stages,
            vec![(CONTEXT_GUIDANCE_CAPABILITY_ID, CONTEXT_GUIDANCE_STAGE_ID)]
        );
        assert_eq!(
            registrar.ingesters,
            vec![
                (
                    CONTEXT_GUIDANCE_CAPABILITY_ID,
                    CONTEXT_GUIDANCE_HISTORY_DISTILLATION_INGESTER_ID
                ),
                (
                    CONTEXT_GUIDANCE_CAPABILITY_ID,
                    CONTEXT_GUIDANCE_KNOWLEDGE_DISTILLATION_INGESTER_ID
                ),
                (
                    CONTEXT_GUIDANCE_CAPABILITY_ID,
                    CONTEXT_GUIDANCE_TARGET_COMPACTION_INGESTER_ID
                )
            ]
        );
        assert_eq!(registrar.mailboxes.len(), 3);
        let mailbox = registrar.mailboxes[0];
        assert_eq!(mailbox.capability_id, CONTEXT_GUIDANCE_CAPABILITY_ID);
        assert_eq!(
            mailbox.mailbox_name,
            CONTEXT_GUIDANCE_HISTORY_DISTILLATION_MAILBOX
        );
        assert_eq!(mailbox.policy, CapabilityMailboxPolicy::Job);
        assert_eq!(
            mailbox.handler,
            CapabilityMailboxHandler::Ingester(CONTEXT_GUIDANCE_HISTORY_DISTILLATION_INGESTER_ID)
        );
        assert_eq!(
            mailbox.readiness_policy,
            CapabilityMailboxReadinessPolicy::TextGenerationSlot(
                CONTEXT_GUIDANCE_TEXT_GENERATION_SLOT
            )
        );
        assert_eq!(mailbox.backlog_policy, CapabilityMailboxBacklogPolicy::None);
        let mailbox = registrar.mailboxes[1];
        assert_eq!(mailbox.capability_id, CONTEXT_GUIDANCE_CAPABILITY_ID);
        assert_eq!(
            mailbox.mailbox_name,
            CONTEXT_GUIDANCE_KNOWLEDGE_DISTILLATION_MAILBOX
        );
        assert_eq!(mailbox.policy, CapabilityMailboxPolicy::Job);
        assert_eq!(
            mailbox.handler,
            CapabilityMailboxHandler::Ingester(CONTEXT_GUIDANCE_KNOWLEDGE_DISTILLATION_INGESTER_ID)
        );
        assert_eq!(
            mailbox.readiness_policy,
            CapabilityMailboxReadinessPolicy::TextGenerationSlot(
                CONTEXT_GUIDANCE_TEXT_GENERATION_SLOT
            )
        );
        let mailbox = registrar.mailboxes[2];
        assert_eq!(mailbox.capability_id, CONTEXT_GUIDANCE_CAPABILITY_ID);
        assert_eq!(
            mailbox.mailbox_name,
            CONTEXT_GUIDANCE_TARGET_COMPACTION_MAILBOX
        );
        assert_eq!(mailbox.policy, CapabilityMailboxPolicy::Job);
        assert_eq!(
            mailbox.handler,
            CapabilityMailboxHandler::Ingester(CONTEXT_GUIDANCE_TARGET_COMPACTION_INGESTER_ID)
        );
        assert_eq!(
            mailbox.readiness_policy,
            CapabilityMailboxReadinessPolicy::TextGenerationSlot(
                CONTEXT_GUIDANCE_TEXT_GENERATION_SLOT
            )
        );
        assert_eq!(registrar.schema_modules.len(), 1);
        assert_eq!(
            registrar.schema_modules[0].capability_id,
            CONTEXT_GUIDANCE_CAPABILITY_ID
        );
        assert!(
            registrar
                .query_examples
                .iter()
                .any(|example| example.name == "context_guidance.selected_history_guidance")
        );
        assert!(
            registrar
                .query_examples
                .iter()
                .any(|example| example.name == "context_guidance.rejected_approaches")
        );
        Ok(())
    }
}
