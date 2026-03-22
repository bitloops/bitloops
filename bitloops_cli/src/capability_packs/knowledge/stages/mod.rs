mod knowledge;

use std::sync::Arc;

use crate::host::devql::capability_host::StageRegistration;

use super::services::KnowledgeServices;

pub use knowledge::KnowledgeStageHandler;

pub fn build_knowledge_stage(services: Arc<KnowledgeServices>) -> StageRegistration {
    StageRegistration::new(
        "knowledge",
        "knowledge",
        Arc::new(KnowledgeStageHandler::new(services)),
    )
}
