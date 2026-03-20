mod config;
mod connectors;
mod storage;

use crate::engine::devql::capability_host::CapabilityHealthCheck;

pub use config::check_knowledge_config;
pub use connectors::check_knowledge_connectors;
pub use storage::check_knowledge_storage;

pub static KNOWLEDGE_HEALTH_CHECKS: &[CapabilityHealthCheck] = &[
    CapabilityHealthCheck {
        name: "knowledge.config",
        run: check_knowledge_config,
    },
    CapabilityHealthCheck {
        name: "knowledge.storage",
        run: check_knowledge_storage,
    },
    CapabilityHealthCheck {
        name: "knowledge.connectors",
        run: check_knowledge_connectors,
    },
];
