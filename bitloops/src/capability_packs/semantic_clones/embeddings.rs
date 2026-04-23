use anyhow::{Result, anyhow, bail};
use regex::Regex;
use serde_json::json;
use sha2::{Digest, Sha256};
use std::fmt;
use std::sync::OnceLock;

use crate::capability_packs::semantic_clones::features::{
    SemanticFeatureInput, render_dependency_context,
};
use crate::host::inference::{EmbeddingInputType as HostEmbeddingInputType, EmbeddingService};

const EMBEDDING_FINGERPRINT_VERSION: &str = "symbol-embedding-fingerprint-v3";
const MAX_EMBEDDING_BODY_CHARS: usize = 8_000;

#[derive(
    Debug,
    Clone,
    Copy,
    Default,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    serde::Serialize,
    serde::Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum EmbeddingRepresentationKind {
    #[default]
    #[serde(alias = "baseline", alias = "enriched")]
    Code,
    Summary,
    #[serde(alias = "locator")]
    Identity,
}

impl fmt::Display for EmbeddingRepresentationKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Code => write!(f, "code"),
            Self::Summary => write!(f, "summary"),
            Self::Identity => write!(f, "identity"),
        }
    }
}

impl EmbeddingRepresentationKind {
    pub const fn storage_values(self) -> &'static [&'static str] {
        match self {
            Self::Code => &["code", "baseline", "enriched"],
            Self::Summary => &["summary"],
            Self::Identity => &["identity", "locator"],
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct SymbolEmbeddingInput {
    pub artefact_id: String,
    pub repo_id: String,
    pub blob_sha: String,
    pub representation_kind: EmbeddingRepresentationKind,
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
    pub representation_kind: EmbeddingRepresentationKind,
    pub setup_fingerprint: String,
    pub provider: String,
    pub model: String,
    pub dimension: usize,
    pub embedding_input_hash: String,
    pub embedding: Vec<f32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct EmbeddingSetup {
    pub provider: String,
    pub model: String,
    pub dimension: usize,
    pub setup_fingerprint: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActiveEmbeddingRepresentationState {
    pub representation_kind: EmbeddingRepresentationKind,
    pub setup: EmbeddingSetup,
}

impl ActiveEmbeddingRepresentationState {
    pub fn new(representation_kind: EmbeddingRepresentationKind, setup: EmbeddingSetup) -> Self {
        Self {
            representation_kind,
            setup,
        }
    }
}

impl EmbeddingSetup {
    pub fn new(provider: impl Into<String>, model: impl Into<String>, dimension: usize) -> Self {
        let provider = provider.into();
        let model = model.into();
        let setup_fingerprint = build_embedding_setup_fingerprint(&provider, &model, dimension);
        Self {
            provider,
            model,
            dimension,
            setup_fingerprint,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SymbolEmbeddingIndexState {
    pub embedding_hash: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SymbolEmbeddingIngestionStats {
    pub eligible: usize,
    pub upserted: usize,
    pub skipped: usize,
}

pub fn build_symbol_embedding_inputs(
    inputs: &[SemanticFeatureInput],
    representation_kind: EmbeddingRepresentationKind,
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

pub fn build_symbol_embedding_text(input: &SymbolEmbeddingInput) -> String {
    match input.representation_kind {
        EmbeddingRepresentationKind::Code => build_code_embedding_text(input),
        EmbeddingRepresentationKind::Summary => build_summary_embedding_text(input),
        EmbeddingRepresentationKind::Identity => build_identity_embedding_text(input),
    }
}

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

pub fn build_symbol_embedding_row(
    input: &SymbolEmbeddingInput,
    provider: &dyn EmbeddingService,
) -> Result<SymbolEmbeddingRow> {
    let setup = resolve_embedding_setup(provider)?;
    let embedding = provider.embed(
        &build_symbol_embedding_text(input),
        HostEmbeddingInputType::Document,
    )?;
    if embedding.is_empty() {
        bail!("embedding provider returned an empty vector");
    }

    Ok(SymbolEmbeddingRow {
        artefact_id: input.artefact_id.clone(),
        repo_id: input.repo_id.clone(),
        blob_sha: input.blob_sha.clone(),
        representation_kind: input.representation_kind,
        setup_fingerprint: setup.setup_fingerprint.clone(),
        provider: setup.provider,
        model: setup.model,
        dimension: setup.dimension,
        embedding_input_hash: build_symbol_embedding_input_hash(input, provider),
        embedding,
    })
}

fn build_code_embedding_text(input: &SymbolEmbeddingInput) -> String {
    let body = truncate_chars(normalize_whitespace(&input.body), MAX_EMBEDDING_BODY_CHARS);
    let signature = input
        .signature
        .as_deref()
        .map(normalize_whitespace)
        .unwrap_or_default();
    let dependencies = render_dependency_context(&input.dependency_signals);

    format!(
        "kind: {kind}\n\
language: {language}\n\
language_kind: {language_kind}\n\
name: {name}\n\
signature: {signature}\n\
dependencies: {dependencies}\n\
body:\n{body}",
        kind = input.canonical_kind,
        language = input.language,
        language_kind = input.language_kind,
        name = input.name,
        signature = signature,
        dependencies = dependencies,
        body = body,
    )
}

fn build_summary_embedding_text(input: &SymbolEmbeddingInput) -> String {
    format!(
        "kind: {kind}\n\
language: {language}\n\
name: {name}\n\
summary: {summary}",
        kind = input.canonical_kind,
        language = input.language,
        name = input.name,
        summary = normalize_whitespace(&input.summary),
    )
}

fn build_identity_embedding_text(input: &SymbolEmbeddingInput) -> String {
    let container = identity_container_raw(input);
    let path = normalize_identity_path(&input.path);
    format!(
        "kind: {kind}\n\
language: {language}\n\
name: {name}\n\
name_terms: {name_terms}\n\
container: {container}\n\
container_terms: {container_terms}\n\
path: {path}\n\
path_terms: {path_terms}",
        kind = input.canonical_kind,
        language = input.language,
        name = input.name,
        name_terms = normalize_identity_terms(&input.name),
        container = container,
        container_terms = normalize_identity_terms(&container),
        path = path,
        path_terms = normalize_identity_path_terms(&path),
    )
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

fn normalize_identity_path(path: &str) -> String {
    let mut normalized = path.trim().replace('\\', "/");
    while normalized.starts_with("./") {
        normalized = normalized[2..].to_string();
    }
    normalized.trim_start_matches('/').to_string()
}

fn normalize_identity_terms(input: &str) -> String {
    split_identity_tokens(input).join(" ")
}

fn normalize_identity_path_terms(path: &str) -> String {
    let mut tokens = split_identity_tokens(path);
    strip_trailing_identity_path_suffix(&mut tokens);
    tokens.join(" ")
}

fn identity_container_raw(input: &SymbolEmbeddingInput) -> String {
    let normalized_path = normalize_identity_path(&input.path);
    let mut segments = input
        .symbol_fqn
        .trim()
        .replace('\\', "/")
        .split("::")
        .map(str::trim)
        .filter(|segment| !segment.is_empty())
        .map(str::to_string)
        .collect::<Vec<_>>();
    if segments
        .first()
        .is_some_and(|segment| normalize_identity_path(segment) == normalized_path)
    {
        segments.remove(0);
    }
    if !segments.is_empty() {
        segments.pop();
    }
    segments.join("::")
}

fn split_identity_tokens(input: &str) -> Vec<String> {
    let regex = identity_identifier_regex();
    let mut out = Vec::new();
    for capture in regex.find_iter(input) {
        let raw = capture.as_str();
        for chunk in raw.split('_') {
            for piece in split_identity_camel_case_word(chunk) {
                let lowered = piece.to_ascii_lowercase();
                if lowered.is_empty() {
                    continue;
                }
                out.push(lowered);
            }
        }
    }
    out
}

fn strip_trailing_identity_path_suffix(tokens: &mut Vec<String>) {
    let should_strip = tokens.last().is_some_and(|token| {
        let len = token.len();
        (1..=4).contains(&len) && token.chars().all(|ch| ch.is_ascii_lowercase())
    });
    if should_strip {
        tokens.pop();
    }
}

fn identity_identifier_regex() -> &'static Regex {
    static IDENTIFIER_REGEX: OnceLock<Regex> = OnceLock::new();
    IDENTIFIER_REGEX.get_or_init(|| Regex::new(r"[A-Za-z_][A-Za-z0-9_]*").expect("valid regex"))
}

fn split_identity_camel_case_word(input: &str) -> Vec<String> {
    if input.is_empty() {
        return Vec::new();
    }

    let chars = input.chars().collect::<Vec<_>>();
    let mut pieces = Vec::new();
    let mut current = String::new();

    for (idx, ch) in chars.iter().enumerate() {
        if !current.is_empty() {
            let prev = chars[idx - 1];
            let next = chars.get(idx + 1).copied().unwrap_or('\0');
            let boundary = (prev.is_ascii_lowercase() && ch.is_ascii_uppercase())
                || (prev.is_ascii_alphabetic() && ch.is_ascii_digit())
                || (prev.is_ascii_digit() && ch.is_ascii_alphabetic())
                || (prev.is_ascii_uppercase()
                    && ch.is_ascii_uppercase()
                    && next.is_ascii_lowercase());
            if boundary {
                pieces.push(current.clone());
                current.clear();
            }
        }
        current.push(*ch);
    }

    if !current.is_empty() {
        pieces.push(current);
    }

    pieces
}

fn sha256_hex(input: &str) -> String {
    let digest = Sha256::digest(input.as_bytes());
    let mut out = String::with_capacity(digest.len() * 2);
    for byte in digest {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

fn build_embedding_setup_fingerprint(provider: &str, model: &str, dimension: usize) -> String {
    format!(
        "provider={provider}|model={model}|dimension={dimension}",
        provider = provider,
        model = model,
        dimension = dimension,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::inference::{EmbeddingInputType as HostEmbeddingInputType, EmbeddingService};

    struct MockEmbeddingProvider;

    impl EmbeddingService for MockEmbeddingProvider {
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

        fn embed(&self, _input: &str, input_type: HostEmbeddingInputType) -> Result<Vec<f32>> {
            assert_eq!(input_type, HostEmbeddingInputType::Document);
            Ok(vec![0.1, 0.2, 0.3])
        }
    }

    struct MockEmbeddingSetupProvider {
        cache_key: String,
    }

    impl EmbeddingService for MockEmbeddingSetupProvider {
        fn provider_name(&self) -> &str {
            "openai"
        }

        fn model_name(&self) -> &str {
            "text-embedding-3-large"
        }

        fn output_dimension(&self) -> Option<usize> {
            Some(3072)
        }

        fn cache_key(&self) -> String {
            self.cache_key.clone()
        }

        fn embed(&self, _input: &str, _input_type: HostEmbeddingInputType) -> Result<Vec<f32>> {
            Ok(vec![0.1, 0.2, 0.3])
        }
    }

    fn sample_input() -> SymbolEmbeddingInput {
        SymbolEmbeddingInput {
            artefact_id: "artefact-1".to_string(),
            repo_id: "repo-1".to_string(),
            blob_sha: "blob-1".to_string(),
            representation_kind: EmbeddingRepresentationKind::Code,
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

        let rows =
            build_symbol_embedding_inputs(&inputs, EmbeddingRepresentationKind::Code, &summaries);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].artefact_id, "function-1");
        assert_eq!(
            rows[0].representation_kind,
            EmbeddingRepresentationKind::Code
        );
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

        let code_rows =
            build_symbol_embedding_inputs(&inputs, EmbeddingRepresentationKind::Code, &summaries);
        assert_eq!(code_rows.len(), 2);
        assert_eq!(code_rows[0].artefact_id, "function-1");
        assert_eq!(code_rows[1].artefact_id, "function-2");

        let summary_rows = build_symbol_embedding_inputs(
            &inputs,
            EmbeddingRepresentationKind::Summary,
            &summaries,
        );
        assert_eq!(summary_rows.len(), 1);
        assert_eq!(summary_rows[0].artefact_id, "function-1");
    }

    #[test]
    fn code_embedding_hash_ignores_summary_changes() {
        let provider = MockEmbeddingProvider;
        let base = sample_input();
        let mut changed = base.clone();
        changed.summary = "Function normalize email. Normalizes email for storage.".to_string();

        assert_eq!(
            build_symbol_embedding_input_hash(&base, &provider),
            build_symbol_embedding_input_hash(&changed, &provider)
        );
    }

    #[test]
    fn summary_embedding_hash_changes_when_summary_changes() {
        let provider = MockEmbeddingProvider;
        let base = sample_input();
        let mut changed = base.clone();
        changed.summary = "Function normalize email. Normalizes email for storage.".to_string();
        let mut summary_base = base.clone();
        summary_base.representation_kind = EmbeddingRepresentationKind::Summary;
        let mut summary_changed = changed.clone();
        summary_changed.representation_kind = EmbeddingRepresentationKind::Summary;

        assert_ne!(
            build_symbol_embedding_input_hash(&summary_base, &provider),
            build_symbol_embedding_input_hash(&summary_changed, &provider)
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
    fn code_embedding_text_includes_dependencies_and_body_but_omits_summary_and_metadata() {
        let text = build_symbol_embedding_text(&sample_input());
        assert!(text.contains("dependencies: calls:user repo::find by id"));
        assert!(text.contains("body:"));
        assert!(text.contains("return email.trim().toLowerCase();"));
        assert!(!text.contains("summary:"));
        assert!(!text.contains("path:"));
        assert!(!text.contains("symbol_fqn:"));
        assert!(!text.contains("parent_kind:"));
    }

    #[test]
    fn identity_embedding_text_includes_normalized_name_container_and_path() {
        let mut input = sample_input();
        input.representation_kind = EmbeddingRepresentationKind::Identity;
        input.path = "src/models/user_profile.rs".to_string();
        input.symbol_fqn = "src/models/user_profile.rs::UserProfile::displayName".to_string();
        input.name = "displayName".to_string();

        let text = build_symbol_embedding_text(&input);
        assert!(text.contains("name: displayName"));
        assert!(text.contains("name_terms: display name"));
        assert!(text.contains("container: UserProfile"));
        assert!(text.contains("container_terms: user profile"));
        assert!(text.contains("path: src/models/user_profile.rs"));
        assert!(text.contains("path_terms: src models user profile"));
        assert!(!text.contains("body:"));
        assert!(!text.contains("summary:"));
        assert!(!text.contains("dependencies:"));
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
    fn identity_embedding_hash_changes_when_path_or_container_changes() {
        let provider = MockEmbeddingProvider;
        let mut base = sample_input();
        base.representation_kind = EmbeddingRepresentationKind::Identity;
        let mut changed = base.clone();
        changed.path = "src/renamed/user.ts".to_string();
        changed.symbol_fqn = "src/renamed/user.ts::AccountUser::normalizeEmail".to_string();

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
    fn symbol_embedding_hash_changes_when_representation_changes() {
        let provider = MockEmbeddingProvider;
        let base = sample_input();
        let mut changed = base.clone();
        changed.representation_kind = EmbeddingRepresentationKind::Summary;

        assert_ne!(
            build_symbol_embedding_input_hash(&base, &provider),
            build_symbol_embedding_input_hash(&changed, &provider)
        );
    }

    #[test]
    fn symbol_embedding_hash_ignores_profile_rename_when_runtime_setup_matches() {
        let input = sample_input();
        let first = MockEmbeddingSetupProvider {
            cache_key: "runtime_profile=local-a::provider=openai::model=text-embedding-3-large::dimension=3072".to_string(),
        };
        let second = MockEmbeddingSetupProvider {
            cache_key: "runtime_profile=local-b::provider=openai::model=text-embedding-3-large::dimension=3072".to_string(),
        };

        assert_eq!(
            build_symbol_embedding_input_hash(&input, &first),
            build_symbol_embedding_input_hash(&input, &second)
        );
    }

    #[test]
    fn summary_embedding_text_omits_body_and_dependencies() {
        let mut input = sample_input();
        input.representation_kind = EmbeddingRepresentationKind::Summary;

        let text = build_symbol_embedding_text(&input);
        assert!(text.contains("summary: Function normalize email."));
        assert!(!text.contains("dependencies:"));
        assert!(!text.contains("body:"));
        assert!(!text.contains("signature:"));
    }

    #[test]
    fn legacy_representation_aliases_map_to_code() {
        let baseline = serde_json::from_str::<EmbeddingRepresentationKind>("\"baseline\"")
            .expect("baseline alias");
        let enriched = serde_json::from_str::<EmbeddingRepresentationKind>("\"enriched\"")
            .expect("enriched alias");
        let identity = serde_json::from_str::<EmbeddingRepresentationKind>("\"identity\"")
            .expect("identity representation");
        let locator =
            serde_json::from_str::<EmbeddingRepresentationKind>("\"locator\"").expect("alias");

        assert_eq!(baseline, EmbeddingRepresentationKind::Code);
        assert_eq!(enriched, EmbeddingRepresentationKind::Code);
        assert_eq!(identity, EmbeddingRepresentationKind::Identity);
        assert_eq!(locator, EmbeddingRepresentationKind::Identity);
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

    #[test]
    fn resolve_embedding_setup_ignores_cache_key_and_profile_name() {
        let left = resolve_embedding_setup(&MockEmbeddingSetupProvider {
            cache_key: "runtime_profile=local-a::provider=openai::model=text-embedding-3-large::dimension=3072".to_string(),
        })
        .expect("left setup");
        let right = resolve_embedding_setup(&MockEmbeddingSetupProvider {
            cache_key: "runtime_profile=local-b::provider=openai::model=text-embedding-3-large::dimension=3072".to_string(),
        })
        .expect("right setup");

        assert_eq!(left, right);
    }
}
