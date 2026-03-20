use crate::engine::devql::capability_host::CapabilityDescriptor;

pub static KNOWLEDGE_DESCRIPTOR: CapabilityDescriptor = CapabilityDescriptor {
    id: "knowledge",
    display_name: "Knowledge",
    version: "0.1.0",
    api_version: 1,
    description: "Repository-scoped external knowledge ingestion, versioning, relations, and retrieval.",
    default_enabled: true,
    experimental: false,
    dependencies: &[],
    required_host_features: &[],
};
