use crate::host::devql::capability_host::CapabilityDescriptor;

pub static SEMANTIC_CLONES_DESCRIPTOR: CapabilityDescriptor = CapabilityDescriptor {
    id: super::types::SEMANTIC_CLONES_CAPABILITY_ID,
    display_name: "Semantic Clones",
    version: "0.1.0",
    api_version: 1,
    description: "Semantic clone detection: embeddings-backed candidate scoring and symbol_clone_edges materialisation.",
    default_enabled: true,
    experimental: false,
    dependencies: &[],
    required_host_features: &[],
};
