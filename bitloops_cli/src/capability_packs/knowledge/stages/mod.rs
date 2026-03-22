mod knowledge;

use std::sync::Arc;

use crate::host::devql::capability_host::KnowledgeStageRegistration;

use super::services::KnowledgeServices;

pub use knowledge::KnowledgeStageHandler;

pub fn build_knowledge_stage(services: Arc<KnowledgeServices>) -> KnowledgeStageRegistration {
    KnowledgeStageRegistration::new(
        "knowledge",
        "knowledge",
        Arc::new(KnowledgeStageHandler::new(services)),
    )
}
