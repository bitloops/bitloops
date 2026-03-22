use std::sync::Arc;

use anyhow::Result;

use crate::host::capability_host::CapabilityRegistrar;

use super::ingesters::{
    build_knowledge_add_ingester, build_knowledge_associate_ingester,
    build_knowledge_refresh_ingester, build_knowledge_versions_ingester,
};
use super::query_examples::KNOWLEDGE_QUERY_EXAMPLES;
use super::schema::KNOWLEDGE_SCHEMA_MODULE;
use super::services::KnowledgeServices;
use super::stages::build_knowledge_stage;

pub fn register_knowledge_pack(
    services: Arc<KnowledgeServices>,
    registrar: &mut dyn CapabilityRegistrar,
) -> Result<()> {
    registrar.register_ingester(build_knowledge_add_ingester(services.clone()))?;
    registrar.register_ingester(build_knowledge_associate_ingester(services.clone()))?;
    registrar.register_ingester(build_knowledge_refresh_ingester(services.clone()))?;
    registrar.register_ingester(build_knowledge_versions_ingester(services.clone()))?;
    registrar.register_stage(build_knowledge_stage(services))?;
    registrar.register_schema_module(KNOWLEDGE_SCHEMA_MODULE)?;
    registrar.register_query_examples(KNOWLEDGE_QUERY_EXAMPLES)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::capability_host::{
        IngesterRegistration, QueryExample, SchemaModule, StageRegistration,
    };
    use anyhow::Result;

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
    fn register_knowledge_pack_registers_expected_contributions() -> Result<()> {
        let mut registrar = CollectingRegistrar::default();

        register_knowledge_pack(Arc::new(KnowledgeServices::new()), &mut registrar)?;

        assert_eq!(registrar.stages, vec![("knowledge", "knowledge")]);
        assert_eq!(
            registrar.ingesters,
            vec![
                ("knowledge", "knowledge.add"),
                ("knowledge", "knowledge.associate"),
                ("knowledge", "knowledge.refresh"),
                ("knowledge", "knowledge.versions"),
            ]
        );
        assert_eq!(registrar.schema_modules, vec![KNOWLEDGE_SCHEMA_MODULE]);
        assert_eq!(registrar.query_examples, KNOWLEDGE_QUERY_EXAMPLES);
        Ok(())
    }
}
