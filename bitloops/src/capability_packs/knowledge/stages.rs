mod knowledge;

use std::sync::Arc;

use crate::host::capability_host::KnowledgeStageRegistration;

use super::services::KnowledgeServices;

pub use knowledge::KnowledgeStageHandlerImpl;

pub fn build_knowledge_stage(services: Arc<KnowledgeServices>) -> KnowledgeStageRegistration {
    KnowledgeStageRegistration::new(
        "knowledge",
        "knowledge",
        Arc::new(KnowledgeStageHandlerImpl::new(services)),
    )
}
