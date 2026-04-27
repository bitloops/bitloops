use serde_json::json;
use sha2::{Digest, Sha256};

use crate::host::inference::EmbeddingService;

use super::identity::{identity_container_raw, normalize_identity_path};
use super::text::{MAX_EMBEDDING_BODY_CHARS, normalize_whitespace, truncate_chars};
use super::types::{EmbeddingRepresentationKind, SymbolEmbeddingIndexState, SymbolEmbeddingInput};

const EMBEDDING_FINGERPRINT_VERSION: &str = "symbol-embedding-fingerprint-v3";

pub fn build_symbol_embedding_input_hash(
    input: &SymbolEmbeddingInput,
    provider: &dyn EmbeddingService,
) -> String {
    let mut value = json!({
        "fingerprint_version": EMBEDDING_FINGERPRINT_VERSION,
        "provider": embedding_provider_hash_identity(provider),
        "artefact_id": &input.artefact_id,
        "repo_id": &input.repo_id,
        "blob_sha": &input.blob_sha,
        "representation_kind": input.representation_kind,
        "language": input.language.to_ascii_lowercase(),
        "canonical_kind": input.canonical_kind.to_ascii_lowercase(),
        "language_kind": input.language_kind.to_ascii_lowercase(),
        "name": &input.name,
        "content_hash": &input.content_hash,
    });
    if let Some(map) = value.as_object_mut() {
        match input.representation_kind {
            EmbeddingRepresentationKind::Code => {
                map.insert(
                    "signature".to_string(),
                    json!(input.signature.as_deref().map(normalize_whitespace)),
                );
                map.insert(
                    "dependency_signals".to_string(),
                    json!(&input.dependency_signals),
                );
                map.insert(
                    "body".to_string(),
                    json!(truncate_chars(
                        normalize_whitespace(&input.body),
                        MAX_EMBEDDING_BODY_CHARS
                    )),
                );
            }
            EmbeddingRepresentationKind::Summary => {
                map.insert(
                    "summary".to_string(),
                    json!(normalize_whitespace(&input.summary)),
                );
            }
            EmbeddingRepresentationKind::Identity => {
                map.insert(
                    "path".to_string(),
                    json!(normalize_identity_path(&input.path)),
                );
                map.insert(
                    "container".to_string(),
                    json!(identity_container_raw(input)),
                );
            }
        }
    }
    sha256_hex(&value.to_string())
}

fn embedding_provider_hash_identity(provider: &dyn EmbeddingService) -> serde_json::Value {
    match provider.output_dimension() {
        Some(dimension) => json!({
            "provider": provider.provider_name(),
            "model": provider.model_name(),
            "dimension": dimension,
        }),
        None => json!({
            "provider": provider.provider_name(),
            "model": provider.model_name(),
            "cache_key": provider.cache_key(),
        }),
    }
}

pub fn symbol_embeddings_require_reindex(
    state: &SymbolEmbeddingIndexState,
    next_input_hash: &str,
) -> bool {
    state.embedding_hash.as_deref() != Some(next_input_hash)
}

fn sha256_hex(input: &str) -> String {
    let digest = Sha256::digest(input.as_bytes());
    let mut out = String::with_capacity(digest.len() * 2);
    for byte in digest {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}
