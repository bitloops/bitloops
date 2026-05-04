#[path = "features/semantic_features.rs"]
mod core;

pub use core::SymbolSemanticsRow;
pub(crate) use core::synthesize_deterministic_summary;
pub use core::{
    DeterministicFallbackSummaryProvider, DocstringOnlySummaryProvider,
    NoopSemanticSummaryProvider, PreStageArtefactRow, PreStageDependencyRow,
    SemanticFeatureIndexState, SemanticFeatureIngestionStats, SemanticFeatureInput,
    SemanticFeatureRows, SemanticSummaryCandidate, SemanticSummaryProvider,
    build_semantic_feature_input_hash, build_semantic_feature_inputs_from_artefacts,
    build_semantic_feature_inputs_from_artefacts_with_dependencies, build_semantic_feature_rows,
    is_semantic_enrichment_candidate, semantic_features_require_reindex,
    summary_provider_from_service,
};
pub(crate) use core::{build_dependency_context_signal, render_dependency_context};
