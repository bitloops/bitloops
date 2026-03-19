fn likely_diverged_implementation(
    semantic_score: f32,
    lexical: &LexicalSignals,
    structural: &StructuralSignals,
    derived: &DerivedCloneSignals,
) -> bool {
    (lexical.name_match >= DIVERGED_NAME_MATCH_THRESHOLD
        || derived.summary_similarity >= DIVERGED_SUMMARY_SIMILARITY_THRESHOLD
        || !structural.shared_call_targets.is_empty()
        || !structural.shared_dependency_targets.is_empty())
        && lexical.identifier_overlap >= DIVERGED_IDENTIFIER_OVERLAP_THRESHOLD
        && semantic_score >= DIVERGED_MIN_SEMANTIC_SCORE
        && lexical.body_overlap >= DIVERGED_MIN_BODY_OVERLAP
        && structural.call_score <= DIVERGED_MAX_CALL_OVERLAP
        && lexical.body_overlap <= DIVERGED_MAX_BODY_OVERLAP
        && derived.clone_confidence >= CLONE_CONFIDENCE_MEDIUM_THRESHOLD
}

fn likely_shared_logic_candidate(
    semantic_score: f32,
    lexical: &LexicalSignals,
    structural: &StructuralSignals,
    derived: &DerivedCloneSignals,
) -> bool {
    lexical.score >= SHARED_LOGIC_MIN_LEXICAL_SCORE
        && lexical.body_overlap >= SHARED_LOGIC_MIN_BODY_OVERLAP
        && structural.score >= SHARED_LOGIC_MIN_STRUCTURAL_SCORE
        && semantic_score >= SHARED_LOGIC_MIN_SEMANTIC_SCORE
        && derived.clone_confidence >= SHARED_LOGIC_MIN_CLONE_CONFIDENCE
}

fn likely_contextual_neighbor(
    candidate_score: f32,
    semantic_score: f32,
    derived: &DerivedCloneSignals,
) -> bool {
    candidate_score >= CONTEXTUAL_NEIGHBOR_MIN_SCORE
        && semantic_score >= CONTEXTUAL_NEIGHBOR_MIN_SEMANTIC_SCORE
        && derived.locality_dominates
}
