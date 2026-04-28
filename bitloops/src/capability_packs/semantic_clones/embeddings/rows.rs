use anyhow::{Result, anyhow, bail};

use crate::host::inference::{EmbeddingInputType as HostEmbeddingInputType, EmbeddingService};

use super::hash::build_symbol_embedding_input_hash;
use super::text::build_symbol_embedding_text;
use super::types::{EmbeddingSetup, SymbolEmbeddingInput, SymbolEmbeddingRow};

pub fn resolve_embedding_setup(provider: &dyn EmbeddingService) -> Result<EmbeddingSetup> {
    let dimension = provider
        .output_dimension()
        .ok_or_else(|| anyhow!("embedding provider did not expose an output dimension"))?;
    Ok(EmbeddingSetup::new(
        provider.provider_name(),
        provider.model_name(),
        dimension,
    ))
}

pub fn build_symbol_embedding_rows(
    inputs: &[SymbolEmbeddingInput],
    provider: &dyn EmbeddingService,
) -> Result<Vec<SymbolEmbeddingRow>> {
    if inputs.is_empty() {
        return Ok(Vec::new());
    }

    let setup = resolve_embedding_setup(provider)?;
    let texts = inputs
        .iter()
        .map(build_symbol_embedding_text)
        .collect::<Vec<_>>();
    let vectors = provider.embed_batch(&texts, HostEmbeddingInputType::Document)?;
    if vectors.len() != inputs.len() {
        bail!(
            "embedding provider returned {} vectors for {} inputs",
            vectors.len(),
            inputs.len()
        );
    }

    inputs
        .iter()
        .zip(vectors)
        .map(|(input, embedding)| {
            if embedding.is_empty() {
                bail!("embedding provider returned an empty vector");
            }
            Ok(SymbolEmbeddingRow {
                artefact_id: input.artefact_id.clone(),
                repo_id: input.repo_id.clone(),
                blob_sha: input.blob_sha.clone(),
                representation_kind: input.representation_kind,
                setup_fingerprint: setup.setup_fingerprint.clone(),
                provider: setup.provider.clone(),
                model: setup.model.clone(),
                dimension: setup.dimension,
                embedding_input_hash: build_symbol_embedding_input_hash(input, provider),
                embedding,
            })
        })
        .collect()
}

pub fn build_symbol_embedding_row(
    input: &SymbolEmbeddingInput,
    provider: &dyn EmbeddingService,
) -> Result<SymbolEmbeddingRow> {
    build_symbol_embedding_rows(std::slice::from_ref(input), provider)?
        .into_iter()
        .next()
        .ok_or_else(|| anyhow!("embedding provider returned no row"))
}
