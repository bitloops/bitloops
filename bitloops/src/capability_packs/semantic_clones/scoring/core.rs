use super::*;

#[derive(Debug, Clone)]
pub(super) struct LexicalSignals {
    pub(super) score: f32,
    pub(super) name_match: f32,
    pub(super) signature_similarity: f32,
    pub(super) identifier_overlap: f32,
    pub(super) body_overlap: f32,
    pub(super) context_overlap: f32,
    pub(super) shared_body_tokens: Vec<String>,
    pub(super) shared_identifier_tokens: Vec<String>,
    pub(super) shared_context_tokens: Vec<String>,
}

#[derive(Debug, Clone)]
pub(super) struct StructuralSignals {
    pub(super) score: f32,
    pub(super) same_kind: f32,
    pub(super) same_parent_kind: f32,
    pub(super) path_score: f32,
    pub(super) call_score: f32,
    pub(super) dependency_score: f32,
    pub(super) shared_call_targets: Vec<String>,
    pub(super) shared_dependency_targets: Vec<String>,
}

#[derive(Debug, Clone)]
pub(super) struct DerivedCloneSignals {
    pub(super) implementation_score: f32,
    pub(super) locality_score: f32,
    pub(super) clone_confidence: f32,
    pub(super) summary_similarity: f32,
    pub(super) same_file: bool,
    pub(super) same_container: bool,
    pub(super) shared_summary_tokens: Vec<String>,
    pub(super) locality_dominates: bool,
    pub(super) bias_warning: Option<String>,
}

pub(super) fn semantic_similarity(
    source: &SymbolCloneCandidateInput,
    target: &SymbolCloneCandidateInput,
) -> f32 {
    if source.embedding.is_empty()
        || target.embedding.is_empty()
        || source.embedding.len() != target.embedding.len()
    {
        return 0.0;
    }

    let mut dot = 0.0_f32;
    let mut left_norm = 0.0_f32;
    let mut right_norm = 0.0_f32;
    for (left, right) in source.embedding.iter().zip(target.embedding.iter()) {
        dot += left * right;
        left_norm += left * left;
        right_norm += right * right;
    }

    if left_norm <= f32::EPSILON || right_norm <= f32::EPSILON {
        return 0.0;
    }

    let cosine = dot / (left_norm.sqrt() * right_norm.sqrt());
    ((cosine + 1.0) / 2.0).clamp(0.0, 1.0)
}

pub(super) fn lexical_signals(
    source: &SymbolCloneCandidateInput,
    target: &SymbolCloneCandidateInput,
) -> LexicalSignals {
    let (identifier_overlap, shared_identifier_tokens) =
        jaccard_with_shared(&source.identifier_tokens, &target.identifier_tokens);
    let (body_overlap, shared_body_tokens) = jaccard_with_shared(
        &source.normalized_body_tokens,
        &target.normalized_body_tokens,
    );
    let (context_overlap, shared_context_tokens) =
        jaccard_with_shared(&source.context_tokens, &target.context_tokens);
    let signature_similarity = signature_similarity(source, target);
    let name_match = name_match_score(&source.normalized_name, &target.normalized_name);
    let score = ((LEXICAL_WEIGHT_IDENTIFIER_OVERLAP * identifier_overlap)
        + (LEXICAL_WEIGHT_BODY_OVERLAP * body_overlap)
        + (LEXICAL_WEIGHT_CONTEXT_OVERLAP * context_overlap)
        + (LEXICAL_WEIGHT_SIGNATURE_SIMILARITY * signature_similarity)
        + (LEXICAL_WEIGHT_NAME_MATCH * name_match))
        .clamp(0.0, 1.0);

    LexicalSignals {
        score,
        name_match,
        signature_similarity,
        identifier_overlap,
        body_overlap,
        context_overlap,
        shared_body_tokens: filter_signal_tokens(shared_body_tokens),
        shared_identifier_tokens: filter_signal_tokens(shared_identifier_tokens),
        shared_context_tokens: filter_signal_tokens(shared_context_tokens),
    }
}

pub(super) fn structural_signals(
    source: &SymbolCloneCandidateInput,
    target: &SymbolCloneCandidateInput,
    name_match: f32,
) -> StructuralSignals {
    let same_kind = compatible_kind_score(&source.canonical_kind, &target.canonical_kind);
    let same_parent_kind = match (&source.parent_kind, &target.parent_kind) {
        (Some(left), Some(right)) if left.eq_ignore_ascii_case(right) => 1.0,
        (Some(_), Some(_)) => 0.0,
        _ => MISSING_PARENT_KIND_SCORE,
    };
    let path_score = path_similarity(&source.path, &target.path);
    let (call_score, shared_call_targets) =
        jaccard_with_shared(&source.call_targets, &target.call_targets);
    let (dependency_score, shared_dependency_targets) =
        jaccard_with_shared(&source.dependency_targets, &target.dependency_targets);
    let score = ((STRUCTURAL_WEIGHT_SAME_KIND * same_kind)
        + (STRUCTURAL_WEIGHT_SAME_PARENT_KIND * same_parent_kind)
        + (STRUCTURAL_WEIGHT_PATH * path_score)
        + (STRUCTURAL_WEIGHT_CALL * call_score)
        + (STRUCTURAL_WEIGHT_DEPENDENCY * dependency_score))
        .clamp(0.0, 1.0)
        .max(
            (same_kind * STRUCTURAL_SCORE_FLOOR_SAME_KIND_WEIGHT)
                + (name_match * STRUCTURAL_SCORE_FLOOR_NAME_MATCH_WEIGHT),
        );

    StructuralSignals {
        score,
        same_kind,
        same_parent_kind,
        path_score,
        call_score,
        dependency_score,
        shared_call_targets,
        shared_dependency_targets,
    }
}

pub(super) fn derived_clone_signals(
    source: &SymbolCloneCandidateInput,
    target: &SymbolCloneCandidateInput,
    semantic_score: f32,
    lexical: &LexicalSignals,
    structural: &StructuralSignals,
) -> DerivedCloneSignals {
    let same_file = source.path == target.path;
    let same_container = container_identity(&source.symbol_fqn)
        .zip(container_identity(&target.symbol_fqn))
        .map(|(left, right)| left == right)
        .unwrap_or(false);
    let (summary_similarity, shared_summary_tokens) = summary_similarity(source, target);
    let implementation_score = ((IMPLEMENTATION_WEIGHT_BODY_OVERLAP * lexical.body_overlap)
        + (IMPLEMENTATION_WEIGHT_CALL_OVERLAP * structural.call_score)
        + (IMPLEMENTATION_WEIGHT_DEPENDENCY_OVERLAP * structural.dependency_score)
        + (IMPLEMENTATION_WEIGHT_IDENTIFIER_OVERLAP * lexical.identifier_overlap)
        + (IMPLEMENTATION_WEIGHT_SIGNATURE_SIMILARITY * lexical.signature_similarity)
        + (IMPLEMENTATION_WEIGHT_SEMANTIC * semantic_score))
        .clamp(0.0, 1.0);
    let locality_score = ((LOCALITY_WEIGHT_SAME_FILE * bool_score(same_file))
        + (LOCALITY_WEIGHT_SAME_CONTAINER * bool_score(same_container))
        + (LOCALITY_WEIGHT_PATH * structural.path_score)
        + (LOCALITY_WEIGHT_CONTEXT * lexical.context_overlap)
        + (LOCALITY_WEIGHT_PARENT_KIND * structural.same_parent_kind.clamp(0.0, 1.0)))
    .clamp(0.0, 1.0);
    let locality_dominates = same_file
        && locality_score >= LOCALITY_DOMINANCE_MIN_SCORE
        && implementation_score <= LOCALITY_DOMINANCE_MAX_IMPLEMENTATION_SCORE
        && (locality_score - implementation_score) >= LOCALITY_DOMINANCE_MIN_GAP;
    let mut clone_confidence = implementation_score;
    let mut bias_warning = None;
    if locality_dominates {
        clone_confidence = clone_confidence.min(LOCALITY_DOMINANCE_CLONE_CONFIDENCE_CAP);
        bias_warning = Some("same_file_bias".to_string());
    }

    DerivedCloneSignals {
        implementation_score,
        locality_score,
        clone_confidence,
        summary_similarity,
        same_file,
        same_container,
        shared_summary_tokens: filter_signal_tokens(shared_summary_tokens),
        locality_dominates,
        bias_warning,
    }
}

pub(super) fn penalized_candidate_score(base_score: f32, derived: &DerivedCloneSignals) -> f32 {
    if !derived.locality_dominates {
        return base_score;
    }

    ((base_score * PENALIZED_CANDIDATE_SCORE_BASE_WEIGHT)
        + (derived.clone_confidence * PENALIZED_CANDIDATE_SCORE_CLONE_CONFIDENCE_WEIGHT))
        .min(PENALIZED_CANDIDATE_SCORE_CAP)
        .clamp(0.0, 1.0)
}
