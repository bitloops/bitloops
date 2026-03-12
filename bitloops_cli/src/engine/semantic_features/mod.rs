#[path = "semantic_features.rs"]
mod core;

pub use core::{
    NoopSemanticSummaryProvider, PreStageArtefactRow, SemanticFeatureIndexState,
    SemanticFeatureIngestionStats, SemanticFeatureInput, SemanticFeatureRows,
    SemanticSummaryCandidate, SemanticSummaryProvider, SemanticSummaryProviderConfig,
    build_semantic_feature_input_hash, build_semantic_feature_inputs_from_artefacts,
    build_semantic_feature_rows, build_semantic_summary_provider,
    resolve_semantic_summary_endpoint, semantic_features_require_reindex,
};
