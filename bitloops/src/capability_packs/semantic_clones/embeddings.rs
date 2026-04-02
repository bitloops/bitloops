use anyhow::{Result, bail};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::path::Path;
use std::path::PathBuf;

use crate::adapters::model_providers::embeddings::{
    EmbeddingProvider, EmbeddingRuntimeClientConfig, build_embedding_provider,
};
use crate::capability_packs::semantic_clones::features::{
    SemanticFeatureInput, render_dependency_context,
};

const EMBEDDING_FINGERPRINT_VERSION: &str = "symbol-embedding-fingerprint-v3";
const MAX_EMBEDDING_BODY_CHARS: usize = 8_000;

#[derive(Debug, Clone, Default)]
pub struct EmbeddingProviderConfig {
    pub daemon_config_path: PathBuf,
    pub embedding_profile: Option<String>,
    pub runtime_command: String,
    pub runtime_args: Vec<String>,
    pub startup_timeout_secs: u64,
    pub request_timeout_secs: u64,
    pub warnings: Vec<String>,
}

pub fn build_symbol_embedding_provider(
    cfg: &EmbeddingProviderConfig,
    repo_root: Option<&Path>,
) -> Result<Option<Box<dyn EmbeddingProvider>>> {
    let Some(profile_name) = resolve_embedding_profile(cfg) else {
        return Ok(None);
    };

    Ok(Some(build_embedding_provider(
        &EmbeddingRuntimeClientConfig {
            command: cfg.runtime_command.clone(),
            args: cfg.runtime_args.clone(),
            startup_timeout_secs: cfg.startup_timeout_secs,
            request_timeout_secs: cfg.request_timeout_secs,
            config_path: cfg.daemon_config_path.clone(),
            profile_name,
            repo_root: repo_root.map(Path::to_path_buf),
        },
    )?))
}

fn resolve_embedding_profile(cfg: &EmbeddingProviderConfig) -> Option<String> {
    let profile = cfg
        .embedding_profile
        .as_deref()
        .unwrap_or_default()
        .trim()
        .to_string();
    if profile.is_empty() {
        return None;
    }
    Some(profile)
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
    pub dependency_signals: Vec<String>,
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
        .filter(|input| {
            crate::capability_packs::semantic_clones::features::is_semantic_enrichment_candidate(
                input,
            )
        })
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
                dependency_signals: input.dependency_signals.clone(),
                parent_kind: input.parent_kind.clone(),
                content_hash: input.content_hash.clone(),
            })
        })
        .collect()
}

pub fn build_symbol_embedding_text(input: &SymbolEmbeddingInput) -> String {
    let body = truncate_chars(normalize_whitespace(&input.body), MAX_EMBEDDING_BODY_CHARS);
    let signature = input
        .signature
        .as_deref()
        .map(normalize_whitespace)
        .unwrap_or_default();
    let dependencies = render_dependency_context(&input.dependency_signals);

    // Keep the clone semantic basis focused on symbol behavior rather than location.
    format!(
        "kind: {kind}\n\
language: {language}\n\
language_kind: {language_kind}\n\
name: {name}\n\
signature: {signature}\n\
summary: {summary}\n\
dependencies: {dependencies}\n\
body:\n{body}",
        kind = input.canonical_kind,
        language = input.language,
        language_kind = input.language_kind,
        name = input.name,
        signature = signature,
        summary = normalize_whitespace(&input.summary),
        dependencies = dependencies,
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
            "language": input.language.to_ascii_lowercase(),
            "canonical_kind": input.canonical_kind.to_ascii_lowercase(),
            "language_kind": input.language_kind.to_ascii_lowercase(),
            "name": &input.name,
            "signature": input.signature.as_deref().map(normalize_whitespace),
            "summary": normalize_whitespace(&input.summary),
            "dependency_signals": &input.dependency_signals,
            "body": truncate_chars(normalize_whitespace(&input.body), MAX_EMBEDDING_BODY_CHARS),
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
        crate::adapters::model_providers::embeddings::EmbeddingInputType::Document,
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
    use crate::adapters::model_providers::embeddings::{EmbeddingInputType, EmbeddingProvider};

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
            dependency_signals: vec![
                "calls:user_repo::find_by_id".to_string(),
                "references:user::email".to_string(),
            ],
            parent_kind: Some("file".to_string()),
            content_hash: Some("hash-1".to_string()),
        }
    }

    #[test]
    fn symbol_embedding_inputs_exclude_non_semantic_candidates() {
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
                modifiers: vec!["export".to_string()],
                body: "return email.trim().toLowerCase();".to_string(),
                docstring: None,
                parent_kind: Some("file".to_string()),
                dependency_signals: vec!["calls:user_repo::find_by_id".to_string()],
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
                modifiers: vec!["type-only".to_string()],
                body: "import x from 'y';".to_string(),
                docstring: None,
                parent_kind: Some("file".to_string()),
                dependency_signals: vec!["imports:y".to_string()],
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
    fn symbol_embedding_inputs_skip_missing_or_empty_summaries() {
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
                modifiers: vec!["export".to_string()],
                body: "return email.trim().toLowerCase();".to_string(),
                docstring: None,
                parent_kind: Some("file".to_string()),
                dependency_signals: vec!["calls:user_repo::find_by_id".to_string()],
                content_hash: None,
            },
            SemanticFeatureInput {
                artefact_id: "function-2".to_string(),
                symbol_id: None,
                repo_id: "repo-1".to_string(),
                blob_sha: "blob-1".to_string(),
                path: "src/services/user.ts".to_string(),
                language: "typescript".to_string(),
                canonical_kind: "function".to_string(),
                language_kind: "function".to_string(),
                symbol_fqn: "src/services/user.ts::normalizeName".to_string(),
                name: "normalizeName".to_string(),
                signature: None,
                modifiers: vec!["export".to_string()],
                body: "return name.trim().replace(/\\s+/g, ' ');".to_string(),
                docstring: None,
                parent_kind: Some("file".to_string()),
                dependency_signals: vec!["references:user_profile::name".to_string()],
                content_hash: None,
            },
        ];
        let summaries = std::collections::HashMap::from([
            (
                "function-1".to_string(),
                "Function normalize email. Normalizes email addresses.".to_string(),
            ),
            ("function-2".to_string(), "   ".to_string()),
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
        assert_eq!(provider.output_dimension(), Some(3));
        assert_eq!(row.provider, "mock");
        assert_eq!(row.model, "voyage-code-3");
        assert_eq!(row.dimension, 3);
        assert_eq!(row.embedding, vec![0.1_f32, 0.2_f32, 0.3_f32]);
    }

    #[test]
    fn symbol_embedding_text_includes_summary_and_body() {
        let text = build_symbol_embedding_text(&sample_input());
        assert!(text.contains("summary: Function normalize email."));
        assert!(text.contains("dependencies: calls:user repo::find by id"));
        assert!(text.contains("body:"));
        assert!(text.contains("return email.trim().toLowerCase();"));
        assert!(!text.contains("path:"));
        assert!(!text.contains("symbol_fqn:"));
        assert!(!text.contains("parent_kind:"));
    }

    #[test]
    fn symbol_embedding_hash_changes_when_dependencies_change() {
        let provider = MockEmbeddingProvider;
        let base = sample_input();
        let mut changed = base.clone();
        changed.dependency_signals = vec!["calls:user_repo::save".to_string()];

        assert_ne!(
            build_symbol_embedding_input_hash(&base, &provider),
            build_symbol_embedding_input_hash(&changed, &provider)
        );
    }

    #[test]
    fn symbol_embedding_hash_ignores_path_and_symbol_location() {
        let provider = MockEmbeddingProvider;
        let base = sample_input();
        let mut changed = base.clone();
        changed.path = "src/renamed/user.ts".to_string();
        changed.symbol_fqn = "src/renamed/user.ts::normalizeEmail".to_string();
        changed.parent_kind = Some("module".to_string());

        assert_eq!(
            build_symbol_embedding_input_hash(&base, &provider),
            build_symbol_embedding_input_hash(&changed, &provider)
        );
    }

    #[test]
    fn symbol_embedding_provider_defaults_voyage_model_and_dimension() {
        let profile = resolve_embedding_profile(&EmbeddingProviderConfig {
            daemon_config_path: PathBuf::from("/config.toml"),
            embedding_profile: Some("voyage-prod".to_string()),
            runtime_command: "bitloops-embeddings".to_string(),
            runtime_args: Vec::new(),
            startup_timeout_secs: 10,
            request_timeout_secs: 60,
            warnings: Vec::new(),
        });

        assert_eq!(profile.as_deref(), Some("voyage-prod"));
    }

    #[test]
    fn symbol_embedding_provider_returns_none_when_disabled() {
        let provider = build_symbol_embedding_provider(
            &EmbeddingProviderConfig {
                daemon_config_path: PathBuf::from("/config.toml"),
                embedding_profile: None,
                runtime_command: "bitloops-embeddings".to_string(),
                runtime_args: Vec::new(),
                startup_timeout_secs: 10,
                request_timeout_secs: 60,
                warnings: Vec::new(),
            },
            None,
        )
        .expect("disabled provider should not error");

        assert!(provider.is_none());
    }

    #[test]
    fn symbol_embedding_provider_keeps_profile_name_case() {
        let profile = resolve_embedding_profile(&EmbeddingProviderConfig {
            daemon_config_path: PathBuf::from("/config.toml"),
            embedding_profile: Some("Local-Code".to_string()),
            runtime_command: "bitloops-embeddings".to_string(),
            runtime_args: Vec::new(),
            startup_timeout_secs: 10,
            request_timeout_secs: 60,
            warnings: Vec::new(),
        });

        assert_eq!(profile.as_deref(), Some("Local-Code"));
    }

    #[test]
    fn symbol_embedding_reindex_only_when_hash_changes() {
        let state = SymbolEmbeddingIndexState {
            embedding_hash: Some("hash-1".to_string()),
        };

        assert!(!symbol_embeddings_require_reindex(&state, "hash-1"));
        assert!(symbol_embeddings_require_reindex(&state, "hash-2"));
        assert!(symbol_embeddings_require_reindex(
            &SymbolEmbeddingIndexState::default(),
            "hash-1"
        ));
    }
}
