use crate::config::InferenceTask;
use crate::host::capability_host::CapabilityDescriptor;
use crate::host::inference::InferenceSlotDescriptor;

pub const CONTEXT_GUIDANCE_CAPABILITY_ID: &str = "context_guidance";
pub const CONTEXT_GUIDANCE_HISTORY_DISTILLATION_MAILBOX: &str =
    "context_guidance.history_distillation";
pub const CONTEXT_GUIDANCE_HISTORY_DISTILLATION_INGESTER_ID: &str =
    "context_guidance.history_distillation";
pub const CONTEXT_GUIDANCE_STAGE_ID: &str = "context_guidance";
pub const CONTEXT_GUIDANCE_TEXT_GENERATION_SLOT: &str = "guidance_generation";

pub static CONTEXT_GUIDANCE_DESCRIPTOR: CapabilityDescriptor = CapabilityDescriptor {
    id: CONTEXT_GUIDANCE_CAPABILITY_ID,
    display_name: "Context Guidance",
    version: env!("CARGO_PKG_VERSION"),
    api_version: 1,
    description: "Distills captured and linked evidence into artefact-scoped guidance.",
    default_enabled: true,
    experimental: false,
    dependencies: &[],
    required_host_features: &[],
    inference_slots: &[InferenceSlotDescriptor {
        name: CONTEXT_GUIDANCE_TEXT_GENERATION_SLOT,
        task: InferenceTask::TextGeneration,
    }],
};
