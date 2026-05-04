use anyhow::Result;
use std::sync::Arc;

use crate::host::capability_host::CapabilityRegistrar;

use super::current_state::CodeCitySnapshotConsumer;
use super::query_examples::CODECITY_QUERY_EXAMPLES;
use super::schema::CODECITY_SCHEMA_MODULE;
use super::stages::{
    build_codecity_architecture_stage, build_codecity_arcs_stage, build_codecity_boundaries_stage,
    build_codecity_file_detail_stage, build_codecity_violations_stage, build_codecity_world_stage,
};
use super::types::{CODECITY_CAPABILITY_ID, CODECITY_SNAPSHOT_CONSUMER_ID};

pub fn register_codecity_pack(registrar: &mut dyn CapabilityRegistrar) -> Result<()> {
    registrar.register_stage(build_codecity_world_stage())?;
    registrar.register_stage(build_codecity_architecture_stage())?;
    registrar.register_stage(build_codecity_boundaries_stage())?;
    registrar.register_stage(build_codecity_violations_stage())?;
    registrar.register_stage(build_codecity_file_detail_stage())?;
    registrar.register_stage(build_codecity_arcs_stage())?;
    registrar.register_mailbox(
        crate::host::capability_host::CapabilityMailboxRegistration::new(
            CODECITY_CAPABILITY_ID,
            CODECITY_SNAPSHOT_CONSUMER_ID,
            crate::host::capability_host::CapabilityMailboxPolicy::Cursor,
            crate::host::capability_host::CapabilityMailboxHandler::CurrentStateConsumer(
                CODECITY_SNAPSHOT_CONSUMER_ID,
            ),
        ),
    )?;
    registrar.register_current_state_consumer(
        crate::host::capability_host::CurrentStateConsumerRegistration::new(
            CODECITY_CAPABILITY_ID,
            CODECITY_SNAPSHOT_CONSUMER_ID,
            Arc::new(CodeCitySnapshotConsumer),
        ),
    )?;
    registrar.register_schema_module(CODECITY_SCHEMA_MODULE)?;
    registrar.register_query_examples(CODECITY_QUERY_EXAMPLES)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use anyhow::Result;

    use super::*;
    use crate::capability_packs::codecity::types::{
        CODECITY_ARCHITECTURE_STAGE_ID, CODECITY_ARCS_STAGE_ID, CODECITY_BOUNDARIES_STAGE_ID,
        CODECITY_CAPABILITY_ID, CODECITY_FILE_DETAIL_STAGE_ID, CODECITY_VIOLATIONS_STAGE_ID,
        CODECITY_WORLD_STAGE_ID,
    };
    use crate::host::capability_host::{QueryExample, SchemaModule, StageRegistration};

    #[derive(Default)]
    struct CollectingRegistrar {
        stages: Vec<(&'static str, &'static str)>,
        schema_modules: Vec<SchemaModule>,
        query_examples: Vec<QueryExample>,
    }

    impl CapabilityRegistrar for CollectingRegistrar {
        fn register_stage(&mut self, stage: StageRegistration) -> Result<()> {
            self.stages.push((stage.capability_id, stage.stage_name));
            Ok(())
        }

        fn register_ingester(
            &mut self,
            _ingester: crate::host::capability_host::IngesterRegistration,
        ) -> Result<()> {
            unreachable!("codecity does not register ingesters")
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
    fn register_codecity_pack_registers_expected_contributions() -> Result<()> {
        let mut registrar = CollectingRegistrar::default();

        register_codecity_pack(&mut registrar)?;

        assert_eq!(
            registrar.stages,
            vec![
                (CODECITY_CAPABILITY_ID, CODECITY_WORLD_STAGE_ID),
                (CODECITY_CAPABILITY_ID, CODECITY_ARCHITECTURE_STAGE_ID),
                (CODECITY_CAPABILITY_ID, CODECITY_BOUNDARIES_STAGE_ID),
                (CODECITY_CAPABILITY_ID, CODECITY_VIOLATIONS_STAGE_ID),
                (CODECITY_CAPABILITY_ID, CODECITY_FILE_DETAIL_STAGE_ID),
                (CODECITY_CAPABILITY_ID, CODECITY_ARCS_STAGE_ID),
            ]
        );
        assert_eq!(registrar.schema_modules, vec![CODECITY_SCHEMA_MODULE]);
        assert_eq!(registrar.query_examples, CODECITY_QUERY_EXAMPLES);
        Ok(())
    }
}
