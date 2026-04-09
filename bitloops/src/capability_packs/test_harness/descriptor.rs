use crate::host::capability_host::CapabilityDescriptor;

pub static TEST_HARNESS_DESCRIPTOR: CapabilityDescriptor = CapabilityDescriptor {
    id: "test_harness",
    display_name: "Test Harness",
    version: "0.0.11",
    api_version: 1,
    description: "Verification mapping across tests, coverage, classification, and artefact-level confidence/strength.",
    default_enabled: true,
    experimental: false,
    dependencies: &[],
    required_host_features: &[],
    inference_slots: &[],
};
