use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result, anyhow, bail};

use crate::config::InferenceRuntimeConfig;

use super::super::{BITLOOPS_EMBEDDINGS_IPC_DRIVER, EmbeddingInputType, EmbeddingService};
use super::runtime::{
    embeddings_runtime_launch_artifact_fingerprint, process_environment_fingerprint,
};
use super::session::PythonEmbeddingsSessionConfig;
use super::shared::{SharedBitloopsEmbeddingsSession, shared_bitloops_embeddings_session_registry};

pub(crate) struct BitloopsEmbeddingsIpcService {
    profile_name: String,
    model_name: String,
    output_dimension: usize,
    cache_key: String,
    shared_session: Arc<SharedBitloopsEmbeddingsSession>,
}

impl BitloopsEmbeddingsIpcService {
    pub(crate) fn new(
        profile_name: &str,
        runtime: &InferenceRuntimeConfig,
        model: &str,
        cache_dir: Option<&Path>,
        platform_backed: bool,
    ) -> Result<Self> {
        let session_config = PythonEmbeddingsSessionConfig {
            command: runtime.command.clone(),
            args: runtime.args.clone(),
            startup_timeout_secs: runtime.startup_timeout_secs,
            request_timeout_secs: runtime.request_timeout_secs,
            model: model.to_string(),
            cache_dir: cache_dir.map(Path::to_path_buf),
            platform_backed,
            launch_artifact_fingerprint: embeddings_runtime_launch_artifact_fingerprint(
                &runtime.command,
                &runtime.args,
            ),
            process_environment_fingerprint: process_environment_fingerprint(),
        };
        let shared_session =
            shared_bitloops_embeddings_session_registry().get_or_create(&session_config)?;
        let output_dimension = shared_session.output_dimension()?;
        let cache_key = format!(
            "profile={profile_name}::driver={BITLOOPS_EMBEDDINGS_IPC_DRIVER}::model={model}::dimension={output_dimension}"
        );

        Ok(Self {
            profile_name: profile_name.to_string(),
            model_name: model.to_string(),
            output_dimension,
            cache_key,
            shared_session,
        })
    }
}

impl EmbeddingService for BitloopsEmbeddingsIpcService {
    fn provider_name(&self) -> &str {
        BITLOOPS_EMBEDDINGS_IPC_DRIVER
    }

    fn model_name(&self) -> &str {
        &self.model_name
    }

    fn output_dimension(&self) -> Option<usize> {
        Some(self.output_dimension)
    }

    fn cache_key(&self) -> String {
        self.cache_key.clone()
    }

    fn embed(&self, input: &str, _input_type: EmbeddingInputType) -> Result<Vec<f32>> {
        let input = input.trim();
        if input.is_empty() {
            bail!("embedding input cannot be empty");
        }

        let texts = vec![input.to_string()];
        let mut vectors = self.shared_session.embed(&texts).with_context(|| {
            format!(
                "requesting standalone `bitloops-local-embeddings` runtime for profile `{}`",
                self.profile_name
            )
        })?;
        let vector = vectors
            .drain(..)
            .next()
            .ok_or_else(|| anyhow!("standalone embeddings runtime returned no vectors"))?;
        if vector.is_empty() {
            bail!("standalone embeddings runtime returned an empty vector");
        }
        if vector.len() != self.output_dimension {
            bail!(
                "standalone embeddings runtime returned dimension {} but expected {}",
                vector.len(),
                self.output_dimension
            );
        }
        Ok(vector)
    }
}
