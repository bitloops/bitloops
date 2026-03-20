use std::sync::Arc;

use anyhow::Result;

use crate::engine::devql::capability_host::CapabilityRegistrar;

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
