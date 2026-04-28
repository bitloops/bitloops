use super::*;
use crate::capability_packs::semantic_clones::features::SemanticFeatureInput;
use crate::host::inference::{EmbeddingInputType as HostEmbeddingInputType, EmbeddingService};
use anyhow::Result;
use std::sync::atomic::{AtomicUsize, Ordering};

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

struct CountingEmbeddingProvider {
    dimension: usize,
    embed_calls: AtomicUsize,
    embed_batch_calls: AtomicUsize,
}

impl CountingEmbeddingProvider {
    fn new(dimension: usize) -> Self {
        Self {
            dimension,
            embed_calls: AtomicUsize::new(0),
            embed_batch_calls: AtomicUsize::new(0),
        }
    }

    fn embed_calls(&self) -> usize {
        self.embed_calls.load(Ordering::SeqCst)
    }

    fn embed_batch_calls(&self) -> usize {
        self.embed_batch_calls.load(Ordering::SeqCst)
    }
}

impl EmbeddingService for CountingEmbeddingProvider {
    fn provider_name(&self) -> &str {
        "counting"
    }

    fn model_name(&self) -> &str {
        "counting-model"
    }

    fn output_dimension(&self) -> Option<usize> {
        Some(self.dimension)
    }

    fn cache_key(&self) -> String {
        format!(
            "provider=counting::model=counting-model::dimension={}",
            self.dimension
        )
    }

    fn embed(&self, _input: &str, _input_type: HostEmbeddingInputType) -> Result<Vec<f32>> {
        self.embed_calls.fetch_add(1, Ordering::SeqCst);
        Ok(vec![0.0; self.dimension])
    }

    fn embed_batch(
        &self,
        inputs: &[String],
        input_type: HostEmbeddingInputType,
    ) -> Result<Vec<Vec<f32>>> {
        assert_eq!(input_type, HostEmbeddingInputType::Document);
        self.embed_batch_calls.fetch_add(1, Ordering::SeqCst);
        Ok(inputs.iter().map(|_| vec![0.0; self.dimension]).collect())
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
        summary: "Function normalize email. Normalizes email addresses before storage.".to_string(),
        dependency_signals: vec![
            "calls:user_repo::find_by_id".to_string(),
            "references:user::email".to_string(),
        ],
        parent_kind: Some("file".to_string()),
        content_hash: Some("hash-1".to_string()),
    }
}

fn sample_symbol_embedding_input(artefact_id: &str) -> SymbolEmbeddingInput {
    let mut input = sample_input();
    input.artefact_id = artefact_id.to_string();
    input.name = artefact_id.replace('-', "_");
    input.symbol_fqn = format!("src/services/user.ts::{}", input.name);
    input
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

    let summary_rows =
        build_symbol_embedding_inputs(&inputs, EmbeddingRepresentationKind::Summary, &summaries);
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
fn build_symbol_embedding_rows_uses_provider_batch_once() {
    let provider = CountingEmbeddingProvider::new(4);
    let inputs = vec![
        sample_symbol_embedding_input("artefact-1"),
        sample_symbol_embedding_input("artefact-2"),
    ];

    let rows = build_symbol_embedding_rows(&inputs, &provider).expect("build embedding rows");

    assert_eq!(rows.len(), 2);
    assert_eq!(provider.embed_batch_calls(), 1);
    assert_eq!(provider.embed_calls(), 0);
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
        cache_key:
            "runtime_profile=local-a::provider=openai::model=text-embedding-3-large::dimension=3072"
                .to_string(),
    };
    let second = MockEmbeddingSetupProvider {
        cache_key:
            "runtime_profile=local-b::provider=openai::model=text-embedding-3-large::dimension=3072"
                .to_string(),
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
        cache_key:
            "runtime_profile=local-a::provider=openai::model=text-embedding-3-large::dimension=3072"
                .to_string(),
    })
    .expect("left setup");
    let right = resolve_embedding_setup(&MockEmbeddingSetupProvider {
        cache_key:
            "runtime_profile=local-b::provider=openai::model=text-embedding-3-large::dimension=3072"
                .to_string(),
    })
    .expect("right setup");

    assert_eq!(left, right);
}
