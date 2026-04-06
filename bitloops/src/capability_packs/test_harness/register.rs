use anyhow::Result;
use std::sync::Arc;

use crate::host::capability_host::CapabilityRegistrar;

use super::event_handlers::TestHarnessSyncHandler;
use super::ingesters::{
    build_classification_ingester, build_coverage_ingester, build_linkage_ingester,
};
use super::query_examples::TEST_HARNESS_QUERY_EXAMPLES;
use super::schema::TEST_HARNESS_SCHEMA_MODULE;
use super::stages::{
    build_coverage_stage, build_coverage_stage_alias, build_tests_stage, build_tests_stage_alias,
    build_tests_summary_stage,
};

pub fn register_test_harness_pack(registrar: &mut dyn CapabilityRegistrar) -> Result<()> {
    registrar.register_ingester(build_linkage_ingester())?;
    registrar.register_ingester(build_coverage_ingester())?;
    registrar.register_ingester(build_classification_ingester())?;
    registrar.register_stage(build_tests_stage())?;
    registrar.register_stage(build_tests_stage_alias())?;
    registrar.register_stage(build_tests_summary_stage())?;
    registrar.register_stage(build_coverage_stage())?;
    registrar.register_stage(build_coverage_stage_alias())?;
    registrar.register_event_handler(Arc::new(TestHarnessSyncHandler))?;
    registrar.register_schema_module(TEST_HARNESS_SCHEMA_MODULE)?;
    registrar.register_query_examples(TEST_HARNESS_QUERY_EXAMPLES)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capability_packs::test_harness::types::{
        TEST_HARNESS_CLASSIFICATION_INGESTER_ID, TEST_HARNESS_COVERAGE_INGESTER_ID,
        TEST_HARNESS_COVERAGE_STAGE_ALIAS_ID, TEST_HARNESS_COVERAGE_STAGE_ID,
        TEST_HARNESS_LINKAGE_INGESTER_ID, TEST_HARNESS_TESTS_STAGE_ALIAS_ID,
        TEST_HARNESS_TESTS_STAGE_ID, TEST_HARNESS_TESTS_SUMMARY_STAGE_ID,
    };
    use crate::host::capability_host::{
        HostEventHandler, IngesterRegistration, QueryExample, SchemaModule, StageRegistration,
    };
    use anyhow::Result;

    #[derive(Default)]
    struct CollectingRegistrar {
        stages: Vec<(&'static str, &'static str)>,
        ingesters: Vec<(&'static str, &'static str)>,
        event_handlers: Vec<String>,
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

        fn register_event_handler(&mut self, handler: Arc<dyn HostEventHandler>) -> Result<()> {
            self.event_handlers
                .push(handler.capability_id().to_string());
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
    fn register_test_harness_pack_registers_expected_contributions() -> Result<()> {
        let mut registrar = CollectingRegistrar::default();

        register_test_harness_pack(&mut registrar)?;

        assert_eq!(
            registrar.stages,
            vec![
                ("test_harness", TEST_HARNESS_TESTS_STAGE_ID),
                ("test_harness", TEST_HARNESS_TESTS_STAGE_ALIAS_ID),
                ("test_harness", TEST_HARNESS_TESTS_SUMMARY_STAGE_ID),
                ("test_harness", TEST_HARNESS_COVERAGE_STAGE_ID),
                ("test_harness", TEST_HARNESS_COVERAGE_STAGE_ALIAS_ID),
            ]
        );
        assert_eq!(
            registrar.ingesters,
            vec![
                ("test_harness", TEST_HARNESS_LINKAGE_INGESTER_ID),
                ("test_harness", TEST_HARNESS_COVERAGE_INGESTER_ID),
                ("test_harness", TEST_HARNESS_CLASSIFICATION_INGESTER_ID),
            ]
        );
        assert_eq!(registrar.event_handlers, vec!["test_harness".to_string()]);
        assert_eq!(registrar.schema_modules, vec![TEST_HARNESS_SCHEMA_MODULE]);
        assert_eq!(registrar.query_examples, TEST_HARNESS_QUERY_EXAMPLES);
        Ok(())
    }
}
