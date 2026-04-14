use crate::host::capability_host::{CapabilityDependency, CapabilityDescriptor};

const KNOWLEDGE_DEPENDENCIES: &[CapabilityDependency] = &[CapabilityDependency {
    capability_id: "test_harness",
    min_version: "0.0.11",
}];

pub static KNOWLEDGE_DESCRIPTOR: CapabilityDescriptor = CapabilityDescriptor {
    id: "knowledge",
    display_name: "Knowledge",
    version: "0.0.11",
    api_version: 1,
    description: "Repository-scoped external knowledge ingestion, versioning, relations, and retrieval.",
    default_enabled: true,
    experimental: false,
    dependencies: KNOWLEDGE_DEPENDENCIES,
    required_host_features: &[],
    inference_slots: &[],
};
