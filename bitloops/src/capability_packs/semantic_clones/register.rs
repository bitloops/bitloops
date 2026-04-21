use anyhow::Result;
use std::sync::Arc;

use crate::host::capability_host::{
    CapabilityMailboxBacklogPolicy, CapabilityMailboxHandler, CapabilityMailboxPolicy,
    CapabilityMailboxReadinessPolicy, CapabilityMailboxRegistration, CapabilityRegistrar,
};

use super::current_state::SemanticClonesCurrentStateConsumer;
use super::ingesters::{
    build_semantic_features_refresh_ingester, build_symbol_clone_edges_rebuild_ingester,
    build_symbol_embeddings_refresh_ingester,
};
use super::query_examples::SEMANTIC_CLONES_QUERY_EXAMPLES;
use super::schema_module::SEMANTIC_CLONES_SCHEMA_MODULE;
use super::stages::build_summary_stage;

pub fn register_semantic_clones_pack(registrar: &mut dyn CapabilityRegistrar) -> Result<()> {
    registrar.register_ingester(build_semantic_features_refresh_ingester())?;
    registrar.register_ingester(build_symbol_embeddings_refresh_ingester())?;
    registrar.register_ingester(build_symbol_clone_edges_rebuild_ingester())?;
    registrar.register_mailbox(CapabilityMailboxRegistration::new(
        super::types::SEMANTIC_CLONES_CAPABILITY_ID,
        super::types::SEMANTIC_CLONES_INBOUND_CURRENT_STATE_MAILBOX,
        CapabilityMailboxPolicy::Cursor,
        CapabilityMailboxHandler::CurrentStateConsumer(
            super::types::SEMANTIC_CLONES_CURRENT_STATE_CONSUMER_ID,
        ),
    ))?;
    registrar.register_mailbox(
        CapabilityMailboxRegistration::new(
            super::types::SEMANTIC_CLONES_CAPABILITY_ID,
            super::types::SEMANTIC_CLONES_SUMMARY_REFRESH_MAILBOX,
            CapabilityMailboxPolicy::Job,
            CapabilityMailboxHandler::Ingester(
                super::types::SEMANTIC_CLONES_SEMANTIC_FEATURES_REFRESH_INGESTER_ID,
            ),
        )
        .readiness_policy(CapabilityMailboxReadinessPolicy::TextGenerationSlot(
            super::types::SEMANTIC_CLONES_SUMMARY_GENERATION_SLOT,
        ))
        .backlog_policy(CapabilityMailboxBacklogPolicy::ArtefactCompaction),
    )?;
    registrar.register_mailbox(
        CapabilityMailboxRegistration::new(
            super::types::SEMANTIC_CLONES_CAPABILITY_ID,
            super::types::SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX,
            CapabilityMailboxPolicy::Job,
            CapabilityMailboxHandler::Ingester(
                super::types::SEMANTIC_CLONES_SYMBOL_EMBEDDINGS_REFRESH_INGESTER_ID,
            ),
        )
        .readiness_policy(CapabilityMailboxReadinessPolicy::EmbeddingsSlot(
            super::types::SEMANTIC_CLONES_CODE_EMBEDDINGS_SLOT,
        ))
        .backlog_policy(CapabilityMailboxBacklogPolicy::ArtefactCompaction),
    )?;
    registrar.register_mailbox(
        CapabilityMailboxRegistration::new(
            super::types::SEMANTIC_CLONES_CAPABILITY_ID,
            super::types::SEMANTIC_CLONES_IDENTITY_EMBEDDING_MAILBOX,
            CapabilityMailboxPolicy::Job,
            CapabilityMailboxHandler::Ingester(
                super::types::SEMANTIC_CLONES_SYMBOL_EMBEDDINGS_REFRESH_INGESTER_ID,
            ),
        )
        .readiness_policy(CapabilityMailboxReadinessPolicy::EmbeddingsSlot(
            super::types::SEMANTIC_CLONES_CODE_EMBEDDINGS_SLOT,
        ))
        .backlog_policy(CapabilityMailboxBacklogPolicy::ArtefactCompaction),
    )?;
    registrar.register_mailbox(
        CapabilityMailboxRegistration::new(
            super::types::SEMANTIC_CLONES_CAPABILITY_ID,
            super::types::SEMANTIC_CLONES_SUMMARY_EMBEDDING_MAILBOX,
            CapabilityMailboxPolicy::Job,
            CapabilityMailboxHandler::Ingester(
                super::types::SEMANTIC_CLONES_SYMBOL_EMBEDDINGS_REFRESH_INGESTER_ID,
            ),
        )
        .readiness_policy(CapabilityMailboxReadinessPolicy::EmbeddingsSlot(
            super::types::SEMANTIC_CLONES_SUMMARY_EMBEDDINGS_SLOT,
        ))
        .backlog_policy(CapabilityMailboxBacklogPolicy::ArtefactCompaction),
    )?;
    registrar.register_mailbox(
        CapabilityMailboxRegistration::new(
            super::types::SEMANTIC_CLONES_CAPABILITY_ID,
            super::types::SEMANTIC_CLONES_CLONE_REBUILD_MAILBOX,
            CapabilityMailboxPolicy::Job,
            CapabilityMailboxHandler::Ingester(
                super::types::SEMANTIC_CLONES_CLONE_EDGES_REBUILD_INGESTER_ID,
            ),
        )
        .backlog_policy(CapabilityMailboxBacklogPolicy::RepoCoalesced),
    )?;
    registrar.register_current_state_consumer(
        crate::host::capability_host::CurrentStateConsumerRegistration::new(
            super::types::SEMANTIC_CLONES_CAPABILITY_ID,
            super::types::SEMANTIC_CLONES_CURRENT_STATE_CONSUMER_ID,
            Arc::new(SemanticClonesCurrentStateConsumer),
        ),
    )?;
    registrar.register_stage(build_summary_stage())?;
    registrar.register_schema_module(SEMANTIC_CLONES_SCHEMA_MODULE)?;
    registrar.register_query_examples(SEMANTIC_CLONES_QUERY_EXAMPLES)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capability_packs::semantic_clones::types::{
        SEMANTIC_CLONES_CAPABILITY_ID, SEMANTIC_CLONES_CLONE_EDGES_REBUILD_INGESTER_ID,
        SEMANTIC_CLONES_CLONE_REBUILD_MAILBOX, SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX,
        SEMANTIC_CLONES_CURRENT_STATE_CONSUMER_ID, SEMANTIC_CLONES_IDENTITY_EMBEDDING_MAILBOX,
        SEMANTIC_CLONES_INBOUND_CURRENT_STATE_MAILBOX,
        SEMANTIC_CLONES_SEMANTIC_FEATURES_REFRESH_INGESTER_ID,
        SEMANTIC_CLONES_SUMMARY_EMBEDDING_MAILBOX, SEMANTIC_CLONES_SUMMARY_REFRESH_MAILBOX,
        SEMANTIC_CLONES_SUMMARY_STAGE_ID, SEMANTIC_CLONES_SYMBOL_EMBEDDINGS_REFRESH_INGESTER_ID,
    };
    use crate::host::capability_host::{
        CapabilityMailboxRegistration, CurrentStateConsumerRegistration, IngesterRegistration,
        QueryExample, SchemaModule, StageRegistration,
    };

    #[derive(Default)]
    struct CollectingRegistrar {
        stages: Vec<(&'static str, &'static str)>,
        ingesters: Vec<(&'static str, &'static str)>,
        current_state_consumers: Vec<(&'static str, &'static str)>,
        mailboxes: Vec<(&'static str, &'static str)>,
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

        fn register_current_state_consumer(
            &mut self,
            registration: CurrentStateConsumerRegistration,
        ) -> Result<()> {
            self.current_state_consumers
                .push((registration.capability_id, registration.consumer_id));
            Ok(())
        }

        fn register_mailbox(&mut self, registration: CapabilityMailboxRegistration) -> Result<()> {
            self.mailboxes
                .push((registration.capability_id, registration.mailbox_name));
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
    fn register_semantic_clones_pack_registers_expected_contributions() -> Result<()> {
        let mut registrar = CollectingRegistrar::default();
        register_semantic_clones_pack(&mut registrar)?;
        assert_eq!(
            registrar.stages,
            vec![(
                SEMANTIC_CLONES_CAPABILITY_ID,
                SEMANTIC_CLONES_SUMMARY_STAGE_ID
            )]
        );
        assert_eq!(
            registrar.ingesters,
            vec![
                (
                    SEMANTIC_CLONES_CAPABILITY_ID,
                    SEMANTIC_CLONES_SEMANTIC_FEATURES_REFRESH_INGESTER_ID
                ),
                (
                    SEMANTIC_CLONES_CAPABILITY_ID,
                    SEMANTIC_CLONES_SYMBOL_EMBEDDINGS_REFRESH_INGESTER_ID
                ),
                (
                    SEMANTIC_CLONES_CAPABILITY_ID,
                    SEMANTIC_CLONES_CLONE_EDGES_REBUILD_INGESTER_ID
                )
            ]
        );
        assert_eq!(
            registrar.mailboxes,
            vec![
                (
                    SEMANTIC_CLONES_CAPABILITY_ID,
                    SEMANTIC_CLONES_INBOUND_CURRENT_STATE_MAILBOX
                ),
                (
                    SEMANTIC_CLONES_CAPABILITY_ID,
                    SEMANTIC_CLONES_SUMMARY_REFRESH_MAILBOX
                ),
                (
                    SEMANTIC_CLONES_CAPABILITY_ID,
                    SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX
                ),
                (
                    SEMANTIC_CLONES_CAPABILITY_ID,
                    SEMANTIC_CLONES_IDENTITY_EMBEDDING_MAILBOX
                ),
                (
                    SEMANTIC_CLONES_CAPABILITY_ID,
                    SEMANTIC_CLONES_SUMMARY_EMBEDDING_MAILBOX
                ),
                (
                    SEMANTIC_CLONES_CAPABILITY_ID,
                    SEMANTIC_CLONES_CLONE_REBUILD_MAILBOX
                )
            ]
        );
        assert_eq!(
            registrar.current_state_consumers,
            vec![(
                SEMANTIC_CLONES_CAPABILITY_ID,
                SEMANTIC_CLONES_CURRENT_STATE_CONSUMER_ID
            )]
        );
        assert_eq!(
            registrar.schema_modules,
            vec![SEMANTIC_CLONES_SCHEMA_MODULE]
        );
        assert_eq!(registrar.query_examples, SEMANTIC_CLONES_QUERY_EXAMPLES);
        Ok(())
    }
}
