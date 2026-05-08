use crate::host::capability_host::CapabilityDescriptor;

use super::types::HTTP_CAPABILITY_ID;

pub static HTTP_DESCRIPTOR: CapabilityDescriptor = CapabilityDescriptor {
    id: HTTP_CAPABILITY_ID,
    display_name: "HTTP",
    version: "0.1.0",
    api_version: 1,
    description: "Protocol-level HTTP primitives, role-indexed query projections, and causal bundles for agent reasoning.",
    default_enabled: true,
    experimental: true,
    dependencies: &[],
    required_host_features: &[],
    inference_slots: &[],
};
