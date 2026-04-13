use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LimitingSignal {
    LowBodyOverlap,
    NoSharedCalls,
    LowCallOverlap,
    DifferentName,
    SummaryGap,
}

impl LimitingSignal {
    const fn as_str(self) -> &'static str {
        match self {
            Self::LowBodyOverlap => "low_body_overlap",
            Self::NoSharedCalls => "no_shared_calls",
            Self::LowCallOverlap => "low_call_overlap",
            Self::DifferentName => "different_name",
            Self::SummaryGap => "summary_gap",
        }
    }
}

pub(super) struct ExplanationContext<'a> {
    pub(super) source: &'a SymbolCloneCandidateInput,
    pub(super) target: &'a SymbolCloneCandidateInput,
    pub(super) candidate_score: f32,
    pub(super) semantic_score: f32,
    pub(super) lexical: &'a LexicalSignals,
    pub(super) structural: &'a StructuralSignals,
    pub(super) derived: &'a DerivedCloneSignals,
    pub(super) duplicate_body_hash_match: bool,
    pub(super) signature_shape_hash_match: bool,
    pub(super) labels: &'a [String],
}

pub(super) fn build_explanation(ctx: &ExplanationContext<'_>) -> Value {
    let limiting_signals = build_limiting_signals(ctx)
        .into_iter()
        .map(LimitingSignal::as_str)
        .collect::<Vec<_>>();

    json!({
        "limiting_signals": limiting_signals,
        "source_summary": ctx.source.summary,
        "target_summary": ctx.target.summary,
        "confidence": {
            "candidate_score": ctx.candidate_score,
            "clone_confidence": ctx.derived.clone_confidence,
            "confidence_band": confidence_band(ctx.derived.clone_confidence),
        },
        "evidence": {
            "implementation_score": ctx.derived.implementation_score,
            "locality_score": ctx.derived.locality_score,
            "summary_similarity": ctx.derived.summary_similarity,
            "semantic_views": {
                "code_embedding_similarity": ctx.derived.code_embedding_similarity,
                "summary_embedding_similarity": ctx.derived.summary_embedding_similarity,
                "summary_embedding_available": ctx.derived.summary_embedding_available,
                "summary_text_similarity": ctx.derived.summary_text_similarity,
                "match_pattern": multi_view_match_pattern(ctx.derived),
            },
            "facts": {
                "same_file": ctx.derived.same_file,
                "same_container": ctx.derived.same_container,
                "same_kind": ctx.structural.same_kind >= 1.0,
                "same_parent_kind": ctx.structural.same_parent_kind >= 1.0,
                "locality_dominates": ctx.derived.locality_dominates,
            },
            "bias_warning": ctx.derived.bias_warning,
            "shared_signals": {
                "body_tokens": ctx.lexical.shared_body_tokens,
                "identifier_tokens": ctx.lexical.shared_identifier_tokens,
                "context_tokens": ctx.lexical.shared_context_tokens,
                "summary_tokens": ctx.derived.shared_summary_tokens,
                "call_targets": ctx.structural.shared_call_targets,
                "dependency_targets": ctx.structural.shared_dependency_targets,
            },
        },
        "duplicate_signals": {
            "body_hash_match": ctx.duplicate_body_hash_match,
            "signature_shape_match": ctx.signature_shape_hash_match,
        },
        "scores": {
            "candidate": ctx.candidate_score,
            "clone_confidence": ctx.derived.clone_confidence,
            "semantic": ctx.semantic_score,
            "code_embedding": ctx.derived.code_embedding_similarity,
            "summary_embedding": ctx.derived.summary_embedding_similarity,
            "lexical": ctx.lexical.score,
            "structural": ctx.structural.score,
            "implementation": ctx.derived.implementation_score,
            "locality": ctx.derived.locality_score,
            "summary_similarity": ctx.derived.summary_similarity,
            "summary_text_similarity": ctx.derived.summary_text_similarity,
            "identifier_overlap": ctx.lexical.identifier_overlap,
            "body_overlap": ctx.lexical.body_overlap,
            "context_overlap": ctx.lexical.context_overlap,
            "signature_similarity": ctx.lexical.signature_similarity,
            "same_kind": ctx.structural.same_kind,
            "same_parent_kind": ctx.structural.same_parent_kind,
            "path_ancestry": ctx.structural.path_score,
            "call_overlap": ctx.structural.call_score,
            "dependency_overlap": ctx.structural.dependency_score,
        },
        "labels": ctx.labels,
    })
}

fn build_limiting_signals(ctx: &ExplanationContext<'_>) -> Vec<LimitingSignal> {
    let mut out = Vec::new();
    if ctx.lexical.body_overlap < LIMITING_SIGNAL_LOW_BODY_OVERLAP_THRESHOLD {
        out.push(LimitingSignal::LowBodyOverlap);
    }
    if ctx.structural.call_score <= f32::EPSILON {
        out.push(LimitingSignal::NoSharedCalls);
    } else if ctx.structural.call_score < LIMITING_SIGNAL_LOW_CALL_OVERLAP_THRESHOLD {
        out.push(LimitingSignal::LowCallOverlap);
    }
    if ctx.lexical.name_match < LIMITING_SIGNAL_LOW_NAME_MATCH_THRESHOLD {
        out.push(LimitingSignal::DifferentName);
    }
    if ctx.derived.summary_similarity > f32::EPSILON
        && ctx.derived.summary_similarity < LIMITING_SIGNAL_SUMMARY_GAP_THRESHOLD
    {
        out.push(LimitingSignal::SummaryGap);
    }
    out.truncate(4);
    out
}

fn confidence_band(clone_confidence: f32) -> &'static str {
    if clone_confidence >= CLONE_CONFIDENCE_STRONG_THRESHOLD {
        "strong"
    } else if clone_confidence >= CLONE_CONFIDENCE_MEDIUM_THRESHOLD {
        "medium"
    } else {
        "weak"
    }
}

fn multi_view_match_pattern(derived: &DerivedCloneSignals) -> &'static str {
    let code_high = derived.code_embedding_similarity >= MULTI_VIEW_HIGH_SIMILARITY_THRESHOLD;
    let summary_high = derived.summary_similarity >= MULTI_VIEW_HIGH_SIMILARITY_THRESHOLD;
    let code_low = derived.code_embedding_similarity <= MULTI_VIEW_LOW_SIMILARITY_THRESHOLD;
    let summary_low = derived.summary_similarity <= MULTI_VIEW_LOW_SIMILARITY_THRESHOLD;

    match (code_high, summary_high, code_low, summary_low) {
        (true, true, _, _) => "high_high",
        (true, false, _, true) => "high_low",
        (false, true, true, _) => "low_high",
        (_, _, true, true) => "low_low",
        _ => "mixed",
    }
}
