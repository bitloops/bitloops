#[path = "semantic_features.rs"]
mod core;

pub use core::{
    NoopSemanticSummaryProvider, PreStageArtefactRow, PreStageDependencyRow,
    SemanticFeatureIndexState, SemanticFeatureIngestionStats, SemanticFeatureInput,
    SemanticFeatureRows, SemanticSummaryCandidate, SemanticSummaryProvider,
    SemanticSummaryProviderConfig, build_semantic_feature_input_hash,
    build_semantic_feature_inputs_from_artefacts,
    build_semantic_feature_inputs_from_artefacts_with_dependencies, build_semantic_feature_rows,
    build_semantic_summary_provider, is_semantic_enrichment_candidate,
    resolve_semantic_summary_endpoint, semantic_features_require_reindex,
};
pub(crate) use core::{build_dependency_context_signal, render_dependency_context};
