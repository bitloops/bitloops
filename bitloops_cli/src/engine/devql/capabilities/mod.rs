pub mod knowledge;

use knowledge::{KnowledgeCapability, KnowledgePlugin};

pub struct DevqlCapabilityRegistry {
    knowledge: Box<dyn KnowledgeCapability>,
}

impl DevqlCapabilityRegistry {
    pub fn builtin() -> anyhow::Result<Self> {
        Ok(Self {
            knowledge: Box::new(KnowledgePlugin::builtin()?),
        })
    }

    pub fn knowledge(&self) -> &dyn KnowledgeCapability {
        self.knowledge.as_ref()
    }
}
