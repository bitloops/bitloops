use crate::host::capability_host::CapabilityDescriptor;

pub static CODECITY_DESCRIPTOR: CapabilityDescriptor = CapabilityDescriptor {
    id: "codecity",
    display_name: "CodeCity",
    version: "0.1.0",
    api_version: 1,
    description: "CodeCity visualisation metrics and geometry derived from DevQL current artefacts and dependency edges.",
    default_enabled: true,
    experimental: true,
    dependencies: &[],
    required_host_features: &[],
    inference_slots: &[],
};
