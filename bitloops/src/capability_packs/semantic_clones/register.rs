use anyhow::Result;

use crate::host::capability_host::CapabilityRegistrar;

use super::ingesters::build_symbol_clone_edges_rebuild_ingester;
use super::query_examples::SEMANTIC_CLONES_QUERY_EXAMPLES;
use super::schema_module::SEMANTIC_CLONES_SCHEMA_MODULE;
use super::stages::build_summary_stage;

pub fn register_semantic_clones_pack(registrar: &mut dyn CapabilityRegistrar) -> Result<()> {
    registrar.register_ingester(build_symbol_clone_edges_rebuild_ingester())?;
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
        SEMANTIC_CLONES_SUMMARY_STAGE_ID,
    };
    use crate::host::capability_host::{
        IngesterRegistration, QueryExample, SchemaModule, StageRegistration,
    };

    #[derive(Default)]
    struct CollectingRegistrar {
        stages: Vec<(&'static str, &'static str)>,
        ingesters: Vec<(&'static str, &'static str)>,
        schema_modules: Vec<SchemaModule>,
        query_examples: Vec<QueryExample>,
    }

    impl CapabilityRegistrar for CollectingRegistrar {
        fn register_stage(
            &mut self,
            stage: StageRegistration,
        ) -> Result<()> {
            self.stages.push((stage.capability_id, stage.stage_name));
            Ok(())
        }

        fn register_ingester(&mut self, ingester: IngesterRegistration) -> Result<()> {
            self.ingesters
                .push((ingester.capability_id, ingester.ingester_name));
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
            vec![(SEMANTIC_CLONES_CAPABILITY_ID, SEMANTIC_CLONES_SUMMARY_STAGE_ID)]
        );
        assert_eq!(
            registrar.ingesters,
            vec![(
                SEMANTIC_CLONES_CAPABILITY_ID,
                SEMANTIC_CLONES_CLONE_EDGES_REBUILD_INGESTER_ID
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
