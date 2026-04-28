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

const PLATFORM_EMBEDDINGS_MAX_CLIENT_BATCH_SIZE: usize = 32;

pub(crate) struct BitloopsEmbeddingsIpcService {
    profile_name: String,
    model_name: String,
    output_dimension: usize,
    cache_key: String,
    shared_session: Arc<SharedBitloopsEmbeddingsSession>,
    max_request_batch_size: Option<usize>,
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
            max_request_batch_size: platform_backed
                .then_some(PLATFORM_EMBEDDINGS_MAX_CLIENT_BATCH_SIZE),
        })
    }

    fn embed_texts(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        let vectors = self.shared_session.embed(texts).with_context(|| {
            format!(
                "requesting standalone `bitloops-local-embeddings` runtime for profile `{}`",
                self.profile_name
            )
        })?;
        self.validate_vectors(texts.len(), vectors)
    }

    fn embed_texts_individually_after_batch_error(
        &self,
        texts: &[String],
        batch_error: anyhow::Error,
    ) -> Result<Vec<Vec<f32>>> {
        let batch_error = format!("{batch_error:#}");
        let mut vectors = Vec::with_capacity(texts.len());
        for text in texts {
            let mut single = self
                .embed_texts(std::slice::from_ref(text))
                .with_context(|| {
                    format!(
                        "single-input fallback failed after batch embedding request failed: {batch_error}"
                    )
                })?;
            vectors.append(&mut single);
        }
        Ok(vectors)
    }

    fn embed_texts_in_supported_batches(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        let Some(max_request_batch_size) = self.max_request_batch_size else {
            return match self.embed_texts(texts) {
                Ok(vectors) => Ok(vectors),
                Err(err) if texts.len() > 1 => {
                    self.embed_texts_individually_after_batch_error(texts, err)
                }
                Err(err) => Err(err),
            };
        };

        let mut vectors = Vec::with_capacity(texts.len());
        for chunk in texts.chunks(max_request_batch_size) {
            match self.embed_texts(chunk) {
                Ok(mut chunk_vectors) => vectors.append(&mut chunk_vectors),
                Err(err) if chunk.len() > 1 => {
                    let mut fallback_vectors =
                        self.embed_texts_individually_after_batch_error(chunk, err)?;
                    vectors.append(&mut fallback_vectors);
                }
                Err(err) => return Err(err),
            }
        }
        Ok(vectors)
    }

    fn validate_vectors(
        &self,
        expected_count: usize,
        vectors: Vec<Vec<f32>>,
    ) -> Result<Vec<Vec<f32>>> {
        if vectors.len() != expected_count {
            bail!(
                "standalone embeddings runtime returned {} vectors for {} inputs",
                vectors.len(),
                expected_count
            );
        }
        for vector in &vectors {
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
        }
        Ok(vectors)
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

    fn embed(&self, input: &str, input_type: EmbeddingInputType) -> Result<Vec<f32>> {
        let mut vectors = self.embed_batch(&[input.to_string()], input_type)?;
        vectors
            .drain(..)
            .next()
            .ok_or_else(|| anyhow!("standalone embeddings runtime returned no vectors"))
    }

    fn embed_batch(
        &self,
        inputs: &[String],
        _input_type: EmbeddingInputType,
    ) -> Result<Vec<Vec<f32>>> {
        if inputs.is_empty() {
            return Ok(Vec::new());
        }

        let texts = inputs
            .iter()
            .map(|input| input.trim())
            .map(|input| {
                if input.is_empty() {
                    bail!("embedding input cannot be empty");
                }
                Ok(input.to_string())
            })
            .collect::<Result<Vec<_>>>()?;

        self.embed_texts_in_supported_batches(&texts)
    }
}
