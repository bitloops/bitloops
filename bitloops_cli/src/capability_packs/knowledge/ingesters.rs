mod add;
mod associate;
mod refresh;
mod versions;

use std::sync::Arc;

use crate::host::capability_host::IngesterRegistration;

use super::services::KnowledgeServices;

pub use add::KnowledgeAddIngester;
pub use associate::KnowledgeAssociateIngester;
pub use refresh::KnowledgeRefreshIngester;
pub use versions::KnowledgeVersionsIngester;

pub fn build_knowledge_add_ingester(services: Arc<KnowledgeServices>) -> IngesterRegistration {
    IngesterRegistration::new(
        "knowledge",
        "knowledge.add",
        Arc::new(KnowledgeAddIngester::new(services)),
    )
}

pub fn build_knowledge_associate_ingester(
    services: Arc<KnowledgeServices>,
) -> IngesterRegistration {
    IngesterRegistration::new(
        "knowledge",
        "knowledge.associate",
        Arc::new(KnowledgeAssociateIngester::new(services)),
    )
}

pub fn build_knowledge_refresh_ingester(services: Arc<KnowledgeServices>) -> IngesterRegistration {
    IngesterRegistration::new(
        "knowledge",
        "knowledge.refresh",
        Arc::new(KnowledgeRefreshIngester::new(services)),
    )
}

pub fn build_knowledge_versions_ingester(services: Arc<KnowledgeServices>) -> IngesterRegistration {
    IngesterRegistration::new(
        "knowledge",
        "knowledge.versions",
        Arc::new(KnowledgeVersionsIngester::new(services)),
    )
}
