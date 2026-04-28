use std::collections::HashMap;

use crate::capability_packs::semantic_clones::features::{
    SemanticFeatureInput, is_semantic_enrichment_candidate,
};

use super::types::{EmbeddingRepresentationKind, SymbolEmbeddingInput};

pub fn build_symbol_embedding_inputs(
    inputs: &[SemanticFeatureInput],
    representation_kind: EmbeddingRepresentationKind,
    summary_by_artefact_id: &HashMap<String, String>,
) -> Vec<SymbolEmbeddingInput> {
    inputs
        .iter()
        .filter(|input| is_semantic_enrichment_candidate(input))
        .filter_map(|input| {
            let summary = summary_by_artefact_id
                .get(&input.artefact_id)
                .map(|summary| summary.trim().to_string())
                .unwrap_or_default();
            if representation_kind == EmbeddingRepresentationKind::Summary && summary.is_empty() {
                return None;
            }

            Some(SymbolEmbeddingInput {
                artefact_id: input.artefact_id.clone(),
                repo_id: input.repo_id.clone(),
                blob_sha: input.blob_sha.clone(),
                representation_kind,
                path: input.path.clone(),
                language: input.language.clone(),
                canonical_kind: input.canonical_kind.clone(),
                language_kind: input.language_kind.clone(),
                symbol_fqn: input.symbol_fqn.clone(),
                name: input.name.clone(),
                signature: input.signature.clone(),
                body: input.body.clone(),
                summary,
                dependency_signals: input.dependency_signals.clone(),
                parent_kind: input.parent_kind.clone(),
                content_hash: input.content_hash.clone(),
            })
        })
        .collect()
}
