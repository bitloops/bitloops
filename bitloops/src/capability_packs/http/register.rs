use anyhow::Result;

use crate::host::capability_host::CapabilityRegistrar;

use super::query_examples::HTTP_QUERY_EXAMPLES;
use super::schema::HTTP_SCHEMA_MODULE;

pub fn register_http_pack(registrar: &mut dyn CapabilityRegistrar) -> Result<()> {
    registrar.register_schema_module(HTTP_SCHEMA_MODULE)?;
    registrar.register_query_examples(HTTP_QUERY_EXAMPLES)?;
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

        fn register_mailbox(&mut self, _registration: CapabilityMailboxRegistration) -> Result<()> {
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

        fn register_current_state_consumer(
            &mut self,
            _registration: CurrentStateConsumerRegistration,
        ) -> Result<()> {
            Ok(())
        }
    }

    #[test]
    fn register_http_pack_registers_schema_and_examples_only() -> Result<()> {
        let mut registrar = CollectingRegistrar::default();

        register_http_pack(&mut registrar)?;

        assert!(registrar.stages.is_empty());
        assert!(registrar.ingesters.is_empty());
        assert_eq!(registrar.schema_modules, vec![HTTP_SCHEMA_MODULE]);
        assert_eq!(registrar.query_examples, HTTP_QUERY_EXAMPLES);
        Ok(())
    }
}
