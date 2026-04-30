use crate::config::InferenceTask;
use crate::host::capability_host::{CapabilityDependency, CapabilityDescriptor};
use crate::host::inference::InferenceSlotDescriptor;

pub const CONTEXT_GUIDANCE_CAPABILITY_ID: &str = "context_guidance";
pub const CONTEXT_GUIDANCE_HISTORY_DISTILLATION_MAILBOX: &str =
    "context_guidance.history_distillation";
pub const CONTEXT_GUIDANCE_HISTORY_DISTILLATION_INGESTER_ID: &str =
    "context_guidance.history_distillation";
pub const CONTEXT_GUIDANCE_KNOWLEDGE_DISTILLATION_MAILBOX: &str =
    "context_guidance.knowledge_distillation";
pub const CONTEXT_GUIDANCE_KNOWLEDGE_DISTILLATION_INGESTER_ID: &str =
    "context_guidance.knowledge_distillation";
pub const CONTEXT_GUIDANCE_TARGET_COMPACTION_MAILBOX: &str = "context_guidance.target_compaction";
pub const CONTEXT_GUIDANCE_TARGET_COMPACTION_INGESTER_ID: &str =
    "context_guidance.target_compaction";
pub const CONTEXT_GUIDANCE_STAGE_ID: &str = "context_guidance";
pub const CONTEXT_GUIDANCE_TEXT_GENERATION_SLOT: &str = "guidance_generation";

const CONTEXT_GUIDANCE_DEPENDENCIES: &[CapabilityDependency] = &[CapabilityDependency {
    capability_id: "knowledge",
    min_version: "0.0.11",
}];

pub static CONTEXT_GUIDANCE_DESCRIPTOR: CapabilityDescriptor = CapabilityDescriptor {
    id: CONTEXT_GUIDANCE_CAPABILITY_ID,
    display_name: "Context Guidance",
    version: env!("CARGO_PKG_VERSION"),
    api_version: 1,
    description: "Distills captured and linked evidence into artefact-scoped guidance.",
    default_enabled: true,
    experimental: false,
    dependencies: CONTEXT_GUIDANCE_DEPENDENCIES,
    required_host_features: &[],
    inference_slots: &[InferenceSlotDescriptor {
        name: CONTEXT_GUIDANCE_TEXT_GENERATION_SLOT,
        task: InferenceTask::TextGeneration,
    }],
};
