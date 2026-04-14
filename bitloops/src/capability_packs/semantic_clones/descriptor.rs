use crate::config::InferenceTask;
use crate::host::capability_host::CapabilityDescriptor;
use crate::host::inference::InferenceSlotDescriptor;

pub static SEMANTIC_CLONES_DESCRIPTOR: CapabilityDescriptor = CapabilityDescriptor {
    id: super::types::SEMANTIC_CLONES_CAPABILITY_ID,
    display_name: "Semantic Clones",
    version: "0.0.11",
    api_version: 1,
    description: "Semantic clone detection: embeddings-backed candidate scoring and symbol_clone_edges materialisation.",
    default_enabled: true,
    experimental: false,
    dependencies: &[],
    required_host_features: &[],
    inference_slots: &[
        InferenceSlotDescriptor {
            name: super::types::SEMANTIC_CLONES_SUMMARY_GENERATION_SLOT,
            task: InferenceTask::TextGeneration,
        },
        InferenceSlotDescriptor {
            name: super::types::SEMANTIC_CLONES_CODE_EMBEDDINGS_SLOT,
            task: InferenceTask::Embeddings,
        },
        InferenceSlotDescriptor {
            name: super::types::SEMANTIC_CLONES_SUMMARY_EMBEDDINGS_SLOT,
            task: InferenceTask::Embeddings,
        },
    ],
};
