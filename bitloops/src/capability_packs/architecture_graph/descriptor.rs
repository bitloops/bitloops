use crate::host::capability_host::CapabilityDescriptor;

use super::types::ARCHITECTURE_GRAPH_CAPABILITY_ID;

pub static ARCHITECTURE_GRAPH_DESCRIPTOR: CapabilityDescriptor = CapabilityDescriptor {
    id: ARCHITECTURE_GRAPH_CAPABILITY_ID,
    display_name: "Architecture Graph",
    version: "0.1.0",
    api_version: 1,
    description: "C4, DDD, ArchiMate, and runtime-trace inspired architecture graph facts for DevQL.",
    default_enabled: true,
    experimental: false,
    dependencies: &[],
    required_host_features: &[],
    inference_slots: &[],
};
