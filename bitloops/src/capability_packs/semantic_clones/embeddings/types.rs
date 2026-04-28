use std::fmt;

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

fn build_embedding_setup_fingerprint(provider: &str, model: &str, dimension: usize) -> String {
    format!(
        "provider={provider}|model={model}|dimension={dimension}",
        provider = provider,
        model = model,
        dimension = dimension,
    )
}
