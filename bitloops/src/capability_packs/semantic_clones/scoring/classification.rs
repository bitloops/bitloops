use super::*;

pub(super) fn likely_diverged_implementation(
    semantic_score: f32,
    lexical: &LexicalSignals,
    structural: &StructuralSignals,
    derived: &DerivedCloneSignals,
) -> bool {
    let embedding_backed_drift = derived.summary_signal_source == SummarySignalSource::Embedding
        && derived.interpretation == SemanticInterpretation::ImplementationReuseDrift;
    let reuse_drift_evidence = lexical.name_match >= DIVERGED_NAME_MATCH_THRESHOLD
        || lexical.identifier_overlap >= DIVERGED_IDENTIFIER_OVERLAP_THRESHOLD
        || lexical.body_overlap >= DIVERGED_MIN_BODY_OVERLAP
        || !structural.shared_call_targets.is_empty()
        || !structural.shared_dependency_targets.is_empty();

    embedding_backed_drift
        && reuse_drift_evidence
        && derived.code_embedding_similarity >= MULTI_VIEW_HIGH_SIMILARITY_THRESHOLD
        && semantic_score >= MIN_SEMANTIC_SCORE
        && derived.clone_confidence >= CLONE_CONFIDENCE_MEDIUM_THRESHOLD
        && derived.primary_semantic_driver != PrimarySemanticDriver::Summary
}

pub(super) fn likely_shared_logic_candidate(
    semantic_score: f32,
    lexical: &LexicalSignals,
    structural: &StructuralSignals,
    derived: &DerivedCloneSignals,
) -> bool {
    let classic_shared_logic = derived.interpretation
        != SemanticInterpretation::SameBehaviourSimilarImplementation
        && derived.primary_semantic_driver != PrimarySemanticDriver::Code
        && lexical.score >= SHARED_LOGIC_MIN_LEXICAL_SCORE
        && lexical.body_overlap >= SHARED_LOGIC_MIN_BODY_OVERLAP
        && structural.score >= SHARED_LOGIC_MIN_STRUCTURAL_SCORE
        && semantic_score >= SHARED_LOGIC_MIN_SEMANTIC_SCORE
        && derived.clone_confidence >= SHARED_LOGIC_MIN_CLONE_CONFIDENCE;
    let multi_view_shared_logic = derived.interpretation
        == SemanticInterpretation::SameBehaviourDifferentImplementation
        && structural.same_kind >= 1.0
        && semantic_score >= SHARED_LOGIC_MIN_SEMANTIC_SCORE
        && (lexical.identifier_overlap >= DIVERGED_IDENTIFIER_OVERLAP_THRESHOLD
            || lexical.context_overlap >= DIVERGED_IDENTIFIER_OVERLAP_THRESHOLD
            || !structural.shared_call_targets.is_empty()
            || !structural.shared_dependency_targets.is_empty());

    classic_shared_logic || multi_view_shared_logic
}

pub(super) fn likely_similar_implementation(
    candidate_score: f32,
    semantic_score: f32,
    derived: &DerivedCloneSignals,
) -> bool {
    candidate_score >= MIN_SIMILAR_IMPLEMENTATION_SCORE
        && semantic_score >= MIN_SEMANTIC_SCORE
        && derived.clone_confidence >= CLONE_CONFIDENCE_MEDIUM_THRESHOLD
        && derived.code_embedding_similarity >= MIN_SEMANTIC_SCORE
        && matches!(
            derived.primary_semantic_driver,
            PrimarySemanticDriver::Code | PrimarySemanticDriver::Balanced
        )
        && !matches!(
            derived.interpretation,
            SemanticInterpretation::ImplementationReuseDrift
                | SemanticInterpretation::SameBehaviourDifferentImplementation
                | SemanticInterpretation::Unrelated
        )
}

pub(super) fn likely_contextual_neighbor(
    candidate_score: f32,
    semantic_score: f32,
    derived: &DerivedCloneSignals,
) -> bool {
    candidate_score >= CONTEXTUAL_NEIGHBOR_MIN_SCORE
        && semantic_score >= CONTEXTUAL_NEIGHBOR_MIN_SEMANTIC_SCORE
        && derived.locality_dominates
}
