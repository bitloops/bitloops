use anyhow::{Result, anyhow, bail};
use serde_json::json;
use sha2::{Digest, Sha256};

use crate::engine::providers::embeddings::{
    EmbeddingProvider, build_embedding_provider, default_embedding_model,
    default_embedding_provider, embedding_provider_requires_api_key,
};
use crate::engine::semantic_features::SemanticFeatureInput;

const EMBEDDING_FINGERPRINT_VERSION: &str = "symbol-embedding-fingerprint-v1";
const MAX_EMBEDDING_BODY_CHARS: usize = 8_000;

#[derive(Debug, Clone, Default)]
pub struct EmbeddingProviderConfig {
    pub embedding_provider: Option<String>,
    pub embedding_model: Option<String>,
    pub embedding_api_key: Option<String>,
}

pub fn build_symbol_embedding_provider(
    cfg: &EmbeddingProviderConfig,
) -> Result<Option<Box<dyn EmbeddingProvider>>> {
    let Some(provider) = resolve_embedding_provider(cfg) else {
        return Ok(None);
    };

    let model = cfg
        .embedding_model
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .map(str::trim)
        .map(str::to_string)
        .unwrap_or_else(|| {
            default_embedding_model(&provider)
                .unwrap_or_default()
                .to_string()
        });
    if model.is_empty() {
        return Err(anyhow!(
            "BITLOOPS_DEVQL_EMBEDDING_MODEL is required when embedding provider is configured"
        ));
    }

    let api_key = cfg
        .embedding_api_key
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .map(str::trim)
        .map(str::to_string);
    if embedding_provider_requires_api_key(&provider) && api_key.is_none() {
        return Err(anyhow!(
            "BITLOOPS_DEVQL_EMBEDDING_API_KEY is required when embedding provider `{provider}` is configured"
        ));
    }

    Ok(Some(build_embedding_provider(&provider, model, api_key)?))
}

fn resolve_embedding_provider(cfg: &EmbeddingProviderConfig) -> Option<String> {
    let provider = cfg
        .embedding_provider
        .as_deref()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    if provider == "none" || provider == "disabled" {
        return None;
    }
    if !provider.is_empty() {
        return Some(provider);
    }

    Some(default_embedding_provider().to_string())
}

#[derive(Debug, Clone, PartialEq)]
pub struct SymbolEmbeddingInput {
    pub artefact_id: String,
    pub repo_id: String,
    pub blob_sha: String,
    pub path: String,
    pub language: String,
    pub canonical_kind: String,
    pub language_kind: String,
    pub symbol_fqn: String,
    pub name: String,
    pub signature: Option<String>,
    pub body: String,
    pub summary: String,
    pub parent_kind: Option<String>,
    pub content_hash: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SymbolEmbeddingRow {
    pub artefact_id: String,
    pub repo_id: String,
    pub blob_sha: String,
    pub provider: String,
    pub model: String,
    pub dimension: usize,
    pub embedding_input_hash: String,
    pub embedding: Vec<f32>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SymbolEmbeddingIndexState {
    pub embedding_hash: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SymbolEmbeddingIngestionStats {
    pub upserted: usize,
    pub skipped: usize,
}

pub fn build_symbol_embedding_inputs(
    inputs: &[SemanticFeatureInput],
    summary_by_artefact_id: &std::collections::HashMap<String, String>,
) -> Vec<SymbolEmbeddingInput> {
    inputs
        .iter()
        .filter(|input| should_embed_kind(&input.canonical_kind))
        .filter_map(|input| {
            let summary = summary_by_artefact_id
                .get(&input.artefact_id)?
                .trim()
                .to_string();
            if summary.is_empty() {
                return None;
            }

            Some(SymbolEmbeddingInput {
                artefact_id: input.artefact_id.clone(),
                repo_id: input.repo_id.clone(),
                blob_sha: input.blob_sha.clone(),
                path: input.path.clone(),
                language: input.language.clone(),
                canonical_kind: input.canonical_kind.clone(),
                language_kind: input.language_kind.clone(),
                symbol_fqn: input.symbol_fqn.clone(),
                name: input.name.clone(),
                signature: input.signature.clone(),
                body: input.body.clone(),
                summary,
                parent_kind: input.parent_kind.clone(),
                content_hash: input.content_hash.clone(),
            })
        })
        .collect()
}

pub fn should_embed_kind(kind: &str) -> bool {
    matches!(
        kind.to_ascii_lowercase().as_str(),
        "file"
            | "module"
            | "function"
            | "method"
            | "interface"
            | "enum"
            | "constructor"
            | "class"
            | "class_declaration"
            | "type"
            | "struct_item"
            | "trait_item"
            | "mod_item"
            | "test"
    )
}

pub fn build_symbol_embedding_text(input: &SymbolEmbeddingInput) -> String {
    let body = truncate_chars(normalize_whitespace(&input.body), MAX_EMBEDDING_BODY_CHARS);
    let signature = input
        .signature
        .as_deref()
        .map(normalize_whitespace)
        .unwrap_or_default();
    let parent_kind = input.parent_kind.as_deref().unwrap_or_default();

    format!(
        "kind: {kind}\n\
language: {language}\n\
language_kind: {language_kind}\n\
path: {path}\n\
symbol_fqn: {symbol_fqn}\n\
name: {name}\n\
signature: {signature}\n\
parent_kind: {parent_kind}\n\
summary: {summary}\n\
body:\n{body}",
        kind = input.canonical_kind,
        language = input.language,
        language_kind = input.language_kind,
        path = input.path,
        symbol_fqn = input.symbol_fqn,
        name = input.name,
        signature = signature,
        parent_kind = parent_kind,
        summary = normalize_whitespace(&input.summary),
        body = body,
    )
}

pub fn build_symbol_embedding_input_hash(
    input: &SymbolEmbeddingInput,
    provider: &dyn EmbeddingProvider,
) -> String {
    sha256_hex(
        &json!({
            "fingerprint_version": EMBEDDING_FINGERPRINT_VERSION,
            "provider": provider.cache_key(),
            "artefact_id": &input.artefact_id,
            "repo_id": &input.repo_id,
            "blob_sha": &input.blob_sha,
            "path": &input.path,
            "language": input.language.to_ascii_lowercase(),
            "canonical_kind": input.canonical_kind.to_ascii_lowercase(),
            "language_kind": input.language_kind.to_ascii_lowercase(),
            "symbol_fqn": &input.symbol_fqn,
            "name": &input.name,
            "signature": input.signature.as_deref().map(normalize_whitespace),
            "summary": normalize_whitespace(&input.summary),
            "body": truncate_chars(normalize_whitespace(&input.body), MAX_EMBEDDING_BODY_CHARS),
            "parent_kind": input.parent_kind.as_deref().map(|value| value.to_ascii_lowercase()),
            "content_hash": &input.content_hash,
        })
        .to_string(),
    )
}

pub fn symbol_embeddings_require_reindex(
    state: &SymbolEmbeddingIndexState,
    next_input_hash: &str,
) -> bool {
    state.embedding_hash.as_deref() != Some(next_input_hash)
}

pub fn build_symbol_embedding_row(
    input: &SymbolEmbeddingInput,
    provider: &dyn EmbeddingProvider,
) -> Result<SymbolEmbeddingRow> {
    let embedding = provider.embed(
        &build_symbol_embedding_text(input),
        crate::engine::providers::embeddings::EmbeddingInputType::Document,
    )?;
    if embedding.is_empty() {
        bail!("embedding provider returned an empty vector");
    }

    Ok(SymbolEmbeddingRow {
        artefact_id: input.artefact_id.clone(),
        repo_id: input.repo_id.clone(),
        blob_sha: input.blob_sha.clone(),
        provider: provider.provider_name().to_string(),
        model: provider.model_name().to_string(),
        dimension: embedding.len(),
        embedding_input_hash: build_symbol_embedding_input_hash(input, provider),
        embedding,
    })
}

fn truncate_chars(input: String, max_chars: usize) -> String {
    if input.chars().count() <= max_chars {
        input
    } else {
        input.chars().take(max_chars).collect::<String>()
    }
}

fn normalize_whitespace(input: &str) -> String {
    input.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn sha256_hex(input: &str) -> String {
    let digest = Sha256::digest(input.as_bytes());
    let mut out = String::with_capacity(digest.len() * 2);
    for byte in digest {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::providers::embeddings::{EmbeddingInputType, EmbeddingProvider};

    struct MockEmbeddingProvider;

    impl EmbeddingProvider for MockEmbeddingProvider {
        fn provider_name(&self) -> &str {
            "mock"
        }

        fn model_name(&self) -> &str {
            "voyage-code-3"
        }

        fn output_dimension(&self) -> Option<usize> {
            Some(3)
        }

        fn cache_key(&self) -> String {
            "provider=mock::model=voyage-code-3::dimension=3".to_string()
        }

        fn embed(&self, _input: &str, input_type: EmbeddingInputType) -> Result<Vec<f32>> {
            assert_eq!(input_type, EmbeddingInputType::Document);
            Ok(vec![0.1, 0.2, 0.3])
        }
    }

    fn sample_input() -> SymbolEmbeddingInput {
        SymbolEmbeddingInput {
            artefact_id: "artefact-1".to_string(),
            repo_id: "repo-1".to_string(),
            blob_sha: "blob-1".to_string(),
            path: "src/services/user.ts".to_string(),
            language: "typescript".to_string(),
            canonical_kind: "function".to_string(),
            language_kind: "function".to_string(),
            symbol_fqn: "src/services/user.ts::normalizeEmail".to_string(),
            name: "normalizeEmail".to_string(),
            signature: Some("export function normalizeEmail(email: string): string {".to_string()),
            body: "return email.trim().toLowerCase();".to_string(),
            summary: "Function normalize email. Normalizes email addresses before storage."
                .to_string(),
            parent_kind: Some("file".to_string()),
            content_hash: Some("hash-1".to_string()),
        }
    }

    #[test]
    fn symbol_embedding_inputs_filter_out_non_retrieval_kinds() {
        let inputs = vec![
            SemanticFeatureInput {
                artefact_id: "function-1".to_string(),
                symbol_id: None,
                repo_id: "repo-1".to_string(),
                blob_sha: "blob-1".to_string(),
                path: "src/services/user.ts".to_string(),
                language: "typescript".to_string(),
                canonical_kind: "function".to_string(),
                language_kind: "function".to_string(),
                symbol_fqn: "src/services/user.ts::normalizeEmail".to_string(),
                name: "normalizeEmail".to_string(),
                signature: None,
                body: "return email;".to_string(),
                docstring: None,
                parent_kind: Some("file".to_string()),
                content_hash: None,
            },
            SemanticFeatureInput {
                artefact_id: "import-1".to_string(),
                symbol_id: None,
                repo_id: "repo-1".to_string(),
                blob_sha: "blob-1".to_string(),
                path: "src/services/user.ts".to_string(),
                language: "typescript".to_string(),
                canonical_kind: "import".to_string(),
                language_kind: "import_statement".to_string(),
                symbol_fqn: "src/services/user.ts::import::import@1".to_string(),
                name: "import@1".to_string(),
                signature: None,
                body: "import x from 'y';".to_string(),
                docstring: None,
                parent_kind: Some("file".to_string()),
                content_hash: None,
            },
        ];
        let summaries = std::collections::HashMap::from([
            (
                "function-1".to_string(),
                "Function normalize email. Normalizes email addresses.".to_string(),
            ),
            ("import-1".to_string(), "Import statement.".to_string()),
        ]);

        let rows = build_symbol_embedding_inputs(&inputs, &summaries);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].artefact_id, "function-1");
    }

    #[test]
    fn symbol_embedding_hash_changes_when_summary_changes() {
        let provider = MockEmbeddingProvider;
        let base = sample_input();
        let mut changed = base.clone();
        changed.summary = "Function normalize email. Normalizes email for storage.".to_string();

        assert_ne!(
            build_symbol_embedding_input_hash(&base, &provider),
            build_symbol_embedding_input_hash(&changed, &provider)
        );
    }

    #[test]
    fn symbol_embedding_row_uses_provider_vector_and_dimension() {
        let provider = MockEmbeddingProvider;
        let row = build_symbol_embedding_row(&sample_input(), &provider).expect("embedding row");
        assert_eq!(row.provider, "mock");
        assert_eq!(row.model, "voyage-code-3");
        assert_eq!(row.dimension, 3);
        assert_eq!(row.embedding, vec![0.1_f32, 0.2_f32, 0.3_f32]);
    }

    #[test]
    fn symbol_embedding_text_includes_summary_and_body() {
        let text = build_symbol_embedding_text(&sample_input());
        assert!(text.contains("summary: Function normalize email."));
        assert!(text.contains("body:"));
        assert!(text.contains("return email.trim().toLowerCase();"));
    }

    #[test]
    fn symbol_embedding_provider_defaults_voyage_model_and_dimension() {
        let provider = build_symbol_embedding_provider(&EmbeddingProviderConfig {
            embedding_provider: Some("voyage".to_string()),
            embedding_model: None,
            embedding_api_key: Some("test-key".to_string()),
        })
        .expect("provider should build")
        .expect("provider should be enabled");

        assert_eq!(provider.provider_name(), "voyage");
        assert_eq!(
            provider.model_name(),
            crate::engine::providers::embeddings::default_embedding_model("voyage")
                .expect("voyage default model")
        );
        assert_eq!(provider.output_dimension(), Some(1024));
    }

    #[test]
    fn symbol_embedding_provider_resolves_local_defaults_without_api_key() {
        let provider = resolve_embedding_provider(&EmbeddingProviderConfig {
            embedding_provider: Some("local".to_string()),
            embedding_model: None,
            embedding_api_key: None,
        });

        assert_eq!(provider.as_deref(), Some("local"));
        assert_eq!(
            crate::engine::providers::embeddings::default_embedding_provider(),
            "local"
        );
        assert_eq!(
            crate::engine::providers::embeddings::default_embedding_model("local")
                .expect("local default model")
                .to_string(),
            "jinaai/jina-embeddings-v2-base-code"
        );
    }

    #[test]
    fn symbol_embedding_provider_defaults_to_local_when_provider_is_omitted() {
        assert_eq!(
            resolve_embedding_provider(&EmbeddingProviderConfig {
                embedding_provider: None,
                embedding_model: None,
                embedding_api_key: None,
            })
            .as_deref(),
            Some(crate::engine::providers::embeddings::default_embedding_provider())
        );
        assert_eq!(
            crate::engine::providers::embeddings::default_embedding_model(
                crate::engine::providers::embeddings::default_embedding_provider()
            ),
            Some("jinaai/jina-embeddings-v2-base-code")
        );
    }

    #[test]
    fn symbol_embedding_provider_returns_none_when_disabled() {
        let provider = build_symbol_embedding_provider(&EmbeddingProviderConfig {
            embedding_provider: Some("disabled".to_string()),
            embedding_model: None,
            embedding_api_key: None,
        })
        .expect("disabled provider should not error");

        assert!(provider.is_none());
    }

    #[test]
    fn symbol_embedding_provider_requires_api_key_for_openai() {
        let err = build_symbol_embedding_provider(&EmbeddingProviderConfig {
            embedding_provider: Some("openai".to_string()),
            embedding_model: Some("text-embedding-3-large".to_string()),
            embedding_api_key: None,
        })
        .err()
        .expect("openai provider without api key should fail");

        assert!(
            err.to_string()
                .contains("BITLOOPS_DEVQL_EMBEDDING_API_KEY is required")
        );
    }
}
