use anyhow::Result;
use std::sync::Arc;

use crate::host::capability_host::{
    CapabilityMailboxBacklogPolicy, CapabilityMailboxHandler, CapabilityMailboxPolicy,
    CapabilityMailboxReadinessPolicy, CapabilityMailboxRegistration, CapabilityRegistrar,
    CurrentStateConsumerRegistration,
};

use super::current_state::ArchitectureGraphCurrentStateConsumer;
use super::ingesters::{
    build_assert_ingester, build_revoke_ingester, build_role_adjudication_ingester,
};
use super::query_examples::ARCHITECTURE_GRAPH_QUERY_EXAMPLES;
use super::roles::ArchitectureGraphRoleCurrentStateConsumer;
use super::schema::ARCHITECTURE_GRAPH_SCHEMA_MODULE;
use super::types::{
    ARCHITECTURE_GRAPH_CAPABILITY_ID, ARCHITECTURE_GRAPH_CONSUMER_ID,
    ARCHITECTURE_GRAPH_ROLE_ADJUDICATION_INGESTER_ID, ARCHITECTURE_GRAPH_ROLE_ADJUDICATION_MAILBOX,
    ARCHITECTURE_GRAPH_ROLE_ADJUDICATION_SLOT, ARCHITECTURE_GRAPH_ROLE_CURRENT_STATE_CONSUMER_ID,
};

pub fn register_architecture_graph_pack(registrar: &mut dyn CapabilityRegistrar) -> Result<()> {
    registrar.register_ingester(build_assert_ingester())?;
    registrar.register_ingester(build_revoke_ingester())?;
    registrar.register_ingester(build_role_adjudication_ingester())?;
    registrar.register_mailbox(CapabilityMailboxRegistration::new(
        ARCHITECTURE_GRAPH_CAPABILITY_ID,
        ARCHITECTURE_GRAPH_CONSUMER_ID,
        CapabilityMailboxPolicy::Cursor,
        CapabilityMailboxHandler::CurrentStateConsumer(ARCHITECTURE_GRAPH_CONSUMER_ID),
    ))?;
    registrar.register_mailbox(CapabilityMailboxRegistration::new(
        ARCHITECTURE_GRAPH_CAPABILITY_ID,
        ARCHITECTURE_GRAPH_ROLE_CURRENT_STATE_CONSUMER_ID,
        CapabilityMailboxPolicy::Cursor,
        CapabilityMailboxHandler::CurrentStateConsumer(
            ARCHITECTURE_GRAPH_ROLE_CURRENT_STATE_CONSUMER_ID,
        ),
    ))?;
    registrar.register_mailbox(
        CapabilityMailboxRegistration::new(
            ARCHITECTURE_GRAPH_CAPABILITY_ID,
            ARCHITECTURE_GRAPH_ROLE_ADJUDICATION_MAILBOX,
            CapabilityMailboxPolicy::Job,
            CapabilityMailboxHandler::Ingester(ARCHITECTURE_GRAPH_ROLE_ADJUDICATION_INGESTER_ID),
        )
        .readiness_policy(CapabilityMailboxReadinessPolicy::StructuredGenerationSlot(
            ARCHITECTURE_GRAPH_ROLE_ADJUDICATION_SLOT,
        ))
        .backlog_policy(CapabilityMailboxBacklogPolicy::ArtefactCompaction),
    )?;
    registrar.register_current_state_consumer(CurrentStateConsumerRegistration::new(
        ARCHITECTURE_GRAPH_CAPABILITY_ID,
        ARCHITECTURE_GRAPH_CONSUMER_ID,
        Arc::new(ArchitectureGraphCurrentStateConsumer),
    ))?;
    registrar.register_current_state_consumer(CurrentStateConsumerRegistration::new(
        ARCHITECTURE_GRAPH_CAPABILITY_ID,
        ARCHITECTURE_GRAPH_ROLE_CURRENT_STATE_CONSUMER_ID,
        Arc::new(ArchitectureGraphRoleCurrentStateConsumer),
    ))?;
    registrar.register_schema_module(ARCHITECTURE_GRAPH_SCHEMA_MODULE)?;
    registrar.register_query_examples(ARCHITECTURE_GRAPH_QUERY_EXAMPLES)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::types::{
        ARCHITECTURE_GRAPH_ASSERT_INGESTER_ID, ARCHITECTURE_GRAPH_REVOKE_INGESTER_ID,
        ARCHITECTURE_GRAPH_ROLE_ADJUDICATION_INGESTER_ID,
        ARCHITECTURE_GRAPH_ROLE_ADJUDICATION_MAILBOX,
        ARCHITECTURE_GRAPH_ROLE_CURRENT_STATE_CONSUMER_ID,
    };
    use super::*;
    use crate::host::capability_host::{
        CapabilityMailboxInitPolicy, CurrentStateConsumerRegistration, IngesterRegistration,
        QueryExample, SchemaModule, StageRegistration,
    };

    #[derive(Default)]
    struct CollectingRegistrar {
        stages: Vec<(&'static str, &'static str)>,
        ingesters: Vec<(&'static str, &'static str)>,
        current_state_consumers: Vec<(&'static str, &'static str)>,
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

        fn register_current_state_consumer(
            &mut self,
            registration: CurrentStateConsumerRegistration,
        ) -> Result<()> {
            self.current_state_consumers
                .push((registration.capability_id, registration.consumer_id));
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
    fn register_architecture_graph_pack_registers_expected_contributions() -> Result<()> {
        let mut registrar = CollectingRegistrar::default();

        register_architecture_graph_pack(&mut registrar)?;

        assert_eq!(
            registrar.ingesters,
            vec![
                (
                    ARCHITECTURE_GRAPH_CAPABILITY_ID,
                    ARCHITECTURE_GRAPH_ASSERT_INGESTER_ID
                ),
                (
                    ARCHITECTURE_GRAPH_CAPABILITY_ID,
                    ARCHITECTURE_GRAPH_REVOKE_INGESTER_ID
                ),
                (
                    ARCHITECTURE_GRAPH_CAPABILITY_ID,
                    ARCHITECTURE_GRAPH_ROLE_ADJUDICATION_INGESTER_ID
                ),
            ]
        );
        assert_eq!(
            registrar.current_state_consumers,
            vec![
                (
                    ARCHITECTURE_GRAPH_CAPABILITY_ID,
                    ARCHITECTURE_GRAPH_CONSUMER_ID
                ),
                (
                    ARCHITECTURE_GRAPH_CAPABILITY_ID,
                    ARCHITECTURE_GRAPH_ROLE_CURRENT_STATE_CONSUMER_ID
                ),
            ]
        );
        assert_eq!(
            registrar
                .mailboxes
                .iter()
                .map(|mailbox| (mailbox.capability_id, mailbox.mailbox_name))
                .collect::<Vec<_>>(),
            vec![
                (
                    ARCHITECTURE_GRAPH_CAPABILITY_ID,
                    ARCHITECTURE_GRAPH_CONSUMER_ID
                ),
                (
                    ARCHITECTURE_GRAPH_CAPABILITY_ID,
                    ARCHITECTURE_GRAPH_ROLE_CURRENT_STATE_CONSUMER_ID
                ),
                (
                    ARCHITECTURE_GRAPH_CAPABILITY_ID,
                    ARCHITECTURE_GRAPH_ROLE_ADJUDICATION_MAILBOX
                ),
            ]
        );
        let snapshot_mailbox = registrar
            .mailboxes
            .iter()
            .find(|mailbox| mailbox.mailbox_name == ARCHITECTURE_GRAPH_CONSUMER_ID)
            .expect("architecture graph snapshot mailbox to be registered");
        assert_eq!(
            snapshot_mailbox.init_policy,
            CapabilityMailboxInitPolicy::Background
        );
        let role_current_state_mailbox = registrar
            .mailboxes
            .iter()
            .find(|mailbox| {
                mailbox.mailbox_name == ARCHITECTURE_GRAPH_ROLE_CURRENT_STATE_CONSUMER_ID
            })
            .expect("architecture role current-state mailbox to be registered");
        assert_eq!(
            role_current_state_mailbox.init_policy,
            CapabilityMailboxInitPolicy::Background
        );
        let adjudication_mailbox = registrar
            .mailboxes
            .iter()
            .find(|mailbox| mailbox.mailbox_name == ARCHITECTURE_GRAPH_ROLE_ADJUDICATION_MAILBOX)
            .expect("role adjudication mailbox to be registered");
        assert_eq!(
            adjudication_mailbox.readiness_policy,
            CapabilityMailboxReadinessPolicy::StructuredGenerationSlot(
                ARCHITECTURE_GRAPH_ROLE_ADJUDICATION_SLOT
            )
        );
        assert_eq!(
            registrar.schema_modules,
            vec![ARCHITECTURE_GRAPH_SCHEMA_MODULE]
        );
        assert_eq!(registrar.query_examples, ARCHITECTURE_GRAPH_QUERY_EXAMPLES);
        assert!(registrar.stages.is_empty());
        Ok(())
    }
}
