use std::sync::Arc;

use anyhow::Result;

use crate::config::InferenceTask;

#[path = "inference/embeddings.rs"]
mod embeddings;
#[path = "inference/gateway.rs"]
mod gateway;
#[path = "inference/text_generation.rs"]
mod text_generation;

pub use gateway::{EmptyInferenceGateway, LocalInferenceGateway, ScopedInferenceGateway};

pub const BITLOOPS_EMBEDDINGS_IPC_DRIVER: &str = "bitloops_embeddings_ipc";
pub const BITLOOPS_LOCAL_EMBEDDINGS_RUNTIME_ID: &str = "bitloops_local_embeddings";
pub const BITLOOPS_PLATFORM_EMBEDDINGS_RUNTIME_ID: &str = "bitloops_platform_embeddings";
pub const BITLOOPS_PLATFORM_CHAT_DRIVER: &str = "bitloops_platform_chat";
pub const BITLOOPS_INFERENCE_RUNTIME_ID: &str = "bitloops_inference";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmbeddingInputType {
    Document,
    Query,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InferenceSlotDescriptor {
    pub name: &'static str,
    pub task: InferenceTask,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedInferenceSlot {
    pub capability_id: String,
    pub slot_name: String,
    pub profile_name: String,
    pub task: Option<InferenceTask>,
    pub driver: Option<String>,
    pub runtime: Option<String>,
    pub model: Option<String>,
}

pub trait EmbeddingService: Send + Sync {
    fn provider_name(&self) -> &str;
    fn model_name(&self) -> &str;
    fn output_dimension(&self) -> Option<usize>;
    fn cache_key(&self) -> String;
    fn embed(&self, input: &str, input_type: EmbeddingInputType) -> Result<Vec<f32>>;
}

pub trait TextGenerationService: Send + Sync {
    fn descriptor(&self) -> String;
    fn cache_key(&self) -> String {
        self.descriptor()
    }
    fn complete(&self, system_prompt: &str, user_prompt: &str) -> Result<String>;
}

pub trait InferenceGateway: Send + Sync {
    fn embeddings(&self, slot_name: &str) -> Result<Arc<dyn EmbeddingService>>;
    fn text_generation(&self, slot_name: &str) -> Result<Arc<dyn TextGenerationService>>;

    fn has_slot(&self, _slot_name: &str) -> bool {
        false
    }

    fn describe(&self, _slot_name: &str) -> Option<ResolvedInferenceSlot> {
        None
    }
}
