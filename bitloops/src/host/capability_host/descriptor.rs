use crate::host::inference::InferenceSlotDescriptor;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CapabilityDependency {
    pub capability_id: &'static str,
    pub min_version: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CapabilityDescriptor {
    pub id: &'static str,
    pub display_name: &'static str,
    pub version: &'static str,
    pub api_version: u32,
    pub description: &'static str,
    pub default_enabled: bool,
    pub experimental: bool,
    pub dependencies: &'static [CapabilityDependency],
    pub required_host_features: &'static [&'static str],
    pub inference_slots: &'static [InferenceSlotDescriptor],
}
