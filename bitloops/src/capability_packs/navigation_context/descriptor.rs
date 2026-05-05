use crate::host::capability_host::CapabilityDescriptor;

use super::types::NAVIGATION_CONTEXT_CAPABILITY_ID;

pub static NAVIGATION_CONTEXT_DESCRIPTOR: CapabilityDescriptor = CapabilityDescriptor {
    id: NAVIGATION_CONTEXT_CAPABILITY_ID,
    display_name: "Navigation Context",
    version: "0.1.0",
    api_version: 1,
    description: "Hashed codebase navigation primitives and freshness signatures for human and agent context artefacts.",
    default_enabled: true,
    experimental: true,
    dependencies: &[],
    required_host_features: &[],
    inference_slots: &[],
};
