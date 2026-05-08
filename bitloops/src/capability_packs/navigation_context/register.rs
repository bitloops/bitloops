use anyhow::Result;
use std::sync::Arc;

use crate::host::capability_host::{
    CapabilityMailboxHandler, CapabilityMailboxPolicy, CapabilityMailboxRegistration,
    CapabilityRegistrar, CurrentStateConsumerRegistration,
};

use super::current_state::NavigationContextCurrentStateConsumer;
use super::query_examples::NAVIGATION_CONTEXT_QUERY_EXAMPLES;
use super::schema::NAVIGATION_CONTEXT_SCHEMA_MODULE;
use super::types::{NAVIGATION_CONTEXT_CAPABILITY_ID, NAVIGATION_CONTEXT_CONSUMER_ID};

pub fn register_navigation_context_pack(registrar: &mut dyn CapabilityRegistrar) -> Result<()> {
    registrar.register_mailbox(CapabilityMailboxRegistration::new(
        NAVIGATION_CONTEXT_CAPABILITY_ID,
        NAVIGATION_CONTEXT_CONSUMER_ID,
        CapabilityMailboxPolicy::Cursor,
        CapabilityMailboxHandler::CurrentStateConsumer(NAVIGATION_CONTEXT_CONSUMER_ID),
    ))?;
    registrar.register_current_state_consumer(CurrentStateConsumerRegistration::new(
        NAVIGATION_CONTEXT_CAPABILITY_ID,
        NAVIGATION_CONTEXT_CONSUMER_ID,
        Arc::new(NavigationContextCurrentStateConsumer),
    ))?;
    registrar.register_schema_module(NAVIGATION_CONTEXT_SCHEMA_MODULE)?;
    registrar.register_query_examples(NAVIGATION_CONTEXT_QUERY_EXAMPLES)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
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
    fn register_navigation_context_pack_registers_expected_contributions() -> Result<()> {
        let mut registrar = CollectingRegistrar::default();

        register_navigation_context_pack(&mut registrar)?;

        assert_eq!(
            registrar.current_state_consumers,
            vec![(
                NAVIGATION_CONTEXT_CAPABILITY_ID,
                NAVIGATION_CONTEXT_CONSUMER_ID
            )]
        );
        assert_eq!(
            registrar.mailboxes,
            vec![(
                NAVIGATION_CONTEXT_CAPABILITY_ID,
                NAVIGATION_CONTEXT_CONSUMER_ID
            )]
        );
        assert_eq!(
            registrar.schema_modules,
            vec![NAVIGATION_CONTEXT_SCHEMA_MODULE]
        );
        assert_eq!(registrar.query_examples, NAVIGATION_CONTEXT_QUERY_EXAMPLES);
        assert!(registrar.stages.is_empty());
        assert!(registrar.ingesters.is_empty());
        Ok(())
    }
}
