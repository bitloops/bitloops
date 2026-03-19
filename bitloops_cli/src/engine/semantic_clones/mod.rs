use std::collections::{BTreeSet, HashSet};

use serde_json::{Value, json};
use sha2::{Digest, Sha256};

const SYMBOL_CLONE_FINGERPRINT_VERSION: &str = "symbol-clone-fingerprint-v2";
const MAX_CLONE_EDGES_PER_SOURCE: usize = 20;
const MIN_SIMILAR_IMPLEMENTATION_SCORE: f32 = 0.55;
const MIN_SEMANTIC_SCORE: f32 = 0.40;
const EXACT_DUPLICATE_SCORE_FLOOR: f32 = 0.99;
const CONTEXTUAL_NEIGHBOR_MIN_SCORE: f32 = 0.55;
const CONTEXTUAL_NEIGHBOR_MIN_SEMANTIC_SCORE: f32 = 0.55;
const PREFERRED_LOCAL_PATTERN_SCORE_THRESHOLD: f32 = 0.72;
const PREFERRED_LOCAL_PATTERN_MAX_CHURN_COUNT: usize = 2;
const PREFERRED_LOCAL_PATTERN_MIN_CLONE_CONFIDENCE: f32 = 0.45;
const PREFERRED_LOCAL_PATTERN_SCORE_BOOST: f32 = 0.05;
const PREFERRED_LOCAL_PATTERN_SCORE_CAP: f32 = 0.98;

const CLONE_SCORE_WEIGHT_SEMANTIC: f32 = 0.55;
const CLONE_SCORE_WEIGHT_LEXICAL: f32 = 0.25;
const CLONE_SCORE_WEIGHT_STRUCTURAL: f32 = 0.20;

const LEXICAL_WEIGHT_IDENTIFIER_OVERLAP: f32 = 0.30;
const LEXICAL_WEIGHT_BODY_OVERLAP: f32 = 0.25;
const LEXICAL_WEIGHT_CONTEXT_OVERLAP: f32 = 0.20;
const LEXICAL_WEIGHT_SIGNATURE_SIMILARITY: f32 = 0.15;
const LEXICAL_WEIGHT_NAME_MATCH: f32 = 0.10;

const STRUCTURAL_WEIGHT_SAME_KIND: f32 = 0.30;
const STRUCTURAL_WEIGHT_SAME_PARENT_KIND: f32 = 0.15;
const STRUCTURAL_WEIGHT_PATH: f32 = 0.20;
const STRUCTURAL_WEIGHT_CALL: f32 = 0.20;
const STRUCTURAL_WEIGHT_DEPENDENCY: f32 = 0.15;
const STRUCTURAL_SCORE_FLOOR_SAME_KIND_WEIGHT: f32 = 0.25;
const STRUCTURAL_SCORE_FLOOR_NAME_MATCH_WEIGHT: f32 = 0.10;

const DIVERGED_NAME_MATCH_THRESHOLD: f32 = 0.75;
const DIVERGED_SUMMARY_SIMILARITY_THRESHOLD: f32 = 0.25;
const DIVERGED_IDENTIFIER_OVERLAP_THRESHOLD: f32 = 0.30;
const DIVERGED_MIN_SEMANTIC_SCORE: f32 = 0.55;
const DIVERGED_MIN_BODY_OVERLAP: f32 = 0.08;
const DIVERGED_MAX_CALL_OVERLAP: f32 = 0.25;
const DIVERGED_MAX_BODY_OVERLAP: f32 = 0.45;

const SHARED_LOGIC_MIN_LEXICAL_SCORE: f32 = 0.68;
const SHARED_LOGIC_MIN_BODY_OVERLAP: f32 = 0.50;
const SHARED_LOGIC_MIN_STRUCTURAL_SCORE: f32 = 0.58;
const SHARED_LOGIC_MIN_SEMANTIC_SCORE: f32 = 0.42;
const SHARED_LOGIC_MIN_CLONE_CONFIDENCE: f32 = 0.55;

const IMPLEMENTATION_WEIGHT_BODY_OVERLAP: f32 = 0.35;
const IMPLEMENTATION_WEIGHT_CALL_OVERLAP: f32 = 0.20;
const IMPLEMENTATION_WEIGHT_DEPENDENCY_OVERLAP: f32 = 0.10;
const IMPLEMENTATION_WEIGHT_IDENTIFIER_OVERLAP: f32 = 0.15;
const IMPLEMENTATION_WEIGHT_SIGNATURE_SIMILARITY: f32 = 0.10;
const IMPLEMENTATION_WEIGHT_SEMANTIC: f32 = 0.10;

const LOCALITY_WEIGHT_SAME_FILE: f32 = 0.30;
const LOCALITY_WEIGHT_SAME_CONTAINER: f32 = 0.25;
const LOCALITY_WEIGHT_PATH: f32 = 0.20;
const LOCALITY_WEIGHT_CONTEXT: f32 = 0.15;
const LOCALITY_WEIGHT_PARENT_KIND: f32 = 0.10;

const LOCALITY_DOMINANCE_MIN_SCORE: f32 = 0.75;
const LOCALITY_DOMINANCE_MAX_IMPLEMENTATION_SCORE: f32 = 0.40;
const LOCALITY_DOMINANCE_MIN_GAP: f32 = 0.25;
const LOCALITY_DOMINANCE_CLONE_CONFIDENCE_CAP: f32 = 0.34;
const CLONE_CONFIDENCE_MEDIUM_THRESHOLD: f32 = 0.45;
const CLONE_CONFIDENCE_STRONG_THRESHOLD: f32 = 0.70;
const PENALIZED_CANDIDATE_SCORE_BASE_WEIGHT: f32 = 0.60;
const PENALIZED_CANDIDATE_SCORE_CLONE_CONFIDENCE_WEIGHT: f32 = 0.40;
const PENALIZED_CANDIDATE_SCORE_CAP: f32 = 0.74;

const LIMITING_SIGNAL_LOW_BODY_OVERLAP_THRESHOLD: f32 = 0.25;
const LIMITING_SIGNAL_LOW_CALL_OVERLAP_THRESHOLD: f32 = 0.15;
const LIMITING_SIGNAL_LOW_NAME_MATCH_THRESHOLD: f32 = 0.50;
const LIMITING_SIGNAL_SUMMARY_GAP_THRESHOLD: f32 = 0.20;

const MISSING_PARENT_KIND_SCORE: f32 = 0.40;
const MISSING_SIGNATURE_SCORE: f32 = 0.25;
const PARTIAL_NAME_MATCH_SCORE: f32 = 0.75;
const SINGLE_SHARED_NAME_PREFIX_SCORE: f32 = 0.50;
const SHARED_SIGNAL_EXPLANATION_LIMIT: usize = 6;

pub const RELATION_KIND_EXACT_DUPLICATE: &str = "exact_duplicate";
pub const RELATION_KIND_SIMILAR_IMPLEMENTATION: &str = "similar_implementation";
pub const RELATION_KIND_SHARED_LOGIC_CANDIDATE: &str = "shared_logic_candidate";
pub const RELATION_KIND_DIVERGED_IMPLEMENTATION: &str = "diverged_implementation";
pub const RELATION_KIND_WEAK_CLONE_CANDIDATE: &str = "weak_clone_candidate";
pub const LABEL_PREFERRED_LOCAL_PATTERN: &str = "preferred_local_pattern";

#[derive(Debug, Clone, PartialEq)]
pub struct SymbolCloneCandidateInput {
    pub repo_id: String,
    pub symbol_id: String,
    pub artefact_id: String,
    pub path: String,
    pub canonical_kind: String,
    pub symbol_fqn: String,
    pub summary: String,
    pub normalized_name: String,
    pub normalized_signature: Option<String>,
    pub identifier_tokens: Vec<String>,
    pub normalized_body_tokens: Vec<String>,
    pub parent_kind: Option<String>,
    pub context_tokens: Vec<String>,
    pub embedding: Vec<f32>,
    pub call_targets: Vec<String>,
    pub dependency_targets: Vec<String>,
    pub churn_count: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SymbolCloneEdgeRow {
    pub repo_id: String,
    pub source_symbol_id: String,
    pub source_artefact_id: String,
    pub target_symbol_id: String,
    pub target_artefact_id: String,
    pub relation_kind: String,
    pub score: f32,
    pub semantic_score: f32,
    pub lexical_score: f32,
    pub structural_score: f32,
    pub clone_input_hash: String,
    pub explanation_json: Value,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct SymbolCloneBuildResult {
    pub edges: Vec<SymbolCloneEdgeRow>,
    pub sources_considered: usize,
}

pub fn build_symbol_clone_edges(inputs: &[SymbolCloneCandidateInput]) -> SymbolCloneBuildResult {
    let candidates = inputs
        .iter()
        .filter(|input| is_meaningful_clone_candidate(input))
        .collect::<Vec<_>>();
    let mut edges = Vec::new();

    for source in &candidates {
        let mut source_edges = candidates
            .iter()
            .filter(|target| {
                target.symbol_id != source.symbol_id && target.repo_id == source.repo_id
            })
            .filter_map(|target| build_symbol_clone_edge(source, target))
            .collect::<Vec<_>>();

        source_edges.sort_by(|left, right| {
            right
                .score
                .partial_cmp(&left.score)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| left.target_symbol_id.cmp(&right.target_symbol_id))
        });
        source_edges.truncate(MAX_CLONE_EDGES_PER_SOURCE);
        edges.extend(source_edges);
    }

    SymbolCloneBuildResult {
        edges,
        sources_considered: candidates.len(),
    }
}

fn build_symbol_clone_edge(
    source: &SymbolCloneCandidateInput,
    target: &SymbolCloneCandidateInput,
) -> Option<SymbolCloneEdgeRow> {
    if !same_clone_kind(&source.canonical_kind, &target.canonical_kind) {
        return None;
    }

    let semantic_score = semantic_similarity(source, target);
    let lexical = lexical_signals(source, target);
    let structural = structural_signals(source, target, lexical.name_match);
    let base_score = (CLONE_SCORE_WEIGHT_SEMANTIC * semantic_score)
        + (CLONE_SCORE_WEIGHT_LEXICAL * lexical.score)
        + (CLONE_SCORE_WEIGHT_STRUCTURAL * structural.score);
    let derived = derived_clone_signals(source, target, semantic_score, &lexical, &structural);
    let mut score = penalized_candidate_score(base_score, &derived);

    let duplicate_body_hash_match = normalized_body_hash(source) == normalized_body_hash(target)
        && !source.normalized_body_tokens.is_empty();
    let signature_shape_hash_match =
        normalized_signature_hash(source) == normalized_signature_hash(target);

    let relation_kind = if duplicate_body_hash_match
        && signature_shape_hash_match
        && compatible_kind_score(&source.canonical_kind, &target.canonical_kind) >= 1.0
    {
        score = score.max(EXACT_DUPLICATE_SCORE_FLOOR);
        RELATION_KIND_EXACT_DUPLICATE.to_string()
    } else if likely_shared_logic_candidate(semantic_score, &lexical, &structural, &derived) {
        RELATION_KIND_SHARED_LOGIC_CANDIDATE.to_string()
    } else if likely_diverged_implementation(semantic_score, &lexical, &structural, &derived) {
        RELATION_KIND_DIVERGED_IMPLEMENTATION.to_string()
    } else if likely_contextual_neighbor(score, semantic_score, &derived) {
        RELATION_KIND_WEAK_CLONE_CANDIDATE.to_string()
    } else if score >= MIN_SIMILAR_IMPLEMENTATION_SCORE
        && semantic_score >= MIN_SEMANTIC_SCORE
        && derived.clone_confidence >= CLONE_CONFIDENCE_MEDIUM_THRESHOLD
    {
        RELATION_KIND_SIMILAR_IMPLEMENTATION.to_string()
    } else {
        return None;
    };

    let mut labels = Vec::new();
    if relation_kind != RELATION_KIND_EXACT_DUPLICATE
        && score >= PREFERRED_LOCAL_PATTERN_SCORE_THRESHOLD
        && derived.clone_confidence >= PREFERRED_LOCAL_PATTERN_MIN_CLONE_CONFIDENCE
        && !derived.locality_dominates
        && target.churn_count <= PREFERRED_LOCAL_PATTERN_MAX_CHURN_COUNT
        && !is_experimental_path(&target.path)
    {
        labels.push(LABEL_PREFERRED_LOCAL_PATTERN.to_string());
        score =
            (score + PREFERRED_LOCAL_PATTERN_SCORE_BOOST).min(PREFERRED_LOCAL_PATTERN_SCORE_CAP);
    }

    let explanation = build_explanation(&ExplanationContext {
        source,
        target,
        candidate_score: score,
        semantic_score,
        lexical: &lexical,
        structural: &structural,
        derived: &derived,
        duplicate_body_hash_match,
        signature_shape_hash_match,
        labels: &labels,
    });

    Some(SymbolCloneEdgeRow {
        repo_id: source.repo_id.clone(),
        source_symbol_id: source.symbol_id.clone(),
        source_artefact_id: source.artefact_id.clone(),
        target_symbol_id: target.symbol_id.clone(),
        target_artefact_id: target.artefact_id.clone(),
        relation_kind,
        score,
        semantic_score,
        lexical_score: lexical.score,
        structural_score: structural.score,
        clone_input_hash: build_clone_input_hash(source, target),
        explanation_json: explanation,
    })
}

fn semantic_similarity(
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

#[derive(Debug, Clone)]
struct LexicalSignals {
    score: f32,
    name_match: f32,
    signature_similarity: f32,
    identifier_overlap: f32,
    body_overlap: f32,
    context_overlap: f32,
    shared_body_tokens: Vec<String>,
    shared_identifier_tokens: Vec<String>,
    shared_context_tokens: Vec<String>,
}

fn lexical_signals(
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

#[derive(Debug, Clone)]
struct StructuralSignals {
    score: f32,
    same_kind: f32,
    same_parent_kind: f32,
    path_score: f32,
    call_score: f32,
    dependency_score: f32,
    shared_call_targets: Vec<String>,
    shared_dependency_targets: Vec<String>,
}

#[derive(Debug, Clone)]
struct DerivedCloneSignals {
    implementation_score: f32,
    locality_score: f32,
    clone_confidence: f32,
    summary_similarity: f32,
    same_file: bool,
    same_container: bool,
    shared_summary_tokens: Vec<String>,
    locality_dominates: bool,
    bias_warning: Option<String>,
}

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

struct ExplanationContext<'a> {
    source: &'a SymbolCloneCandidateInput,
    target: &'a SymbolCloneCandidateInput,
    candidate_score: f32,
    semantic_score: f32,
    lexical: &'a LexicalSignals,
    structural: &'a StructuralSignals,
    derived: &'a DerivedCloneSignals,
    duplicate_body_hash_match: bool,
    signature_shape_hash_match: bool,
    labels: &'a [String],
}

fn structural_signals(
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

fn build_explanation(ctx: &ExplanationContext<'_>) -> Value {
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
            "lexical": ctx.lexical.score,
            "structural": ctx.structural.score,
            "implementation": ctx.derived.implementation_score,
            "locality": ctx.derived.locality_score,
            "summary_similarity": ctx.derived.summary_similarity,
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

fn likely_contextual_neighbor(
    candidate_score: f32,
    semantic_score: f32,
    derived: &DerivedCloneSignals,
) -> bool {
    candidate_score >= CONTEXTUAL_NEIGHBOR_MIN_SCORE
        && semantic_score >= CONTEXTUAL_NEIGHBOR_MIN_SEMANTIC_SCORE
        && derived.locality_dominates
}

fn derived_clone_signals(
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

fn penalized_candidate_score(base_score: f32, derived: &DerivedCloneSignals) -> f32 {
    if !derived.locality_dominates {
        return base_score;
    }

    ((base_score * PENALIZED_CANDIDATE_SCORE_BASE_WEIGHT)
        + (derived.clone_confidence * PENALIZED_CANDIDATE_SCORE_CLONE_CONFIDENCE_WEIGHT))
        .min(PENALIZED_CANDIDATE_SCORE_CAP)
        .clamp(0.0, 1.0)
}

fn summary_similarity(
    source: &SymbolCloneCandidateInput,
    target: &SymbolCloneCandidateInput,
) -> (f32, Vec<String>) {
    let source_tokens = summary_tokens(&source.summary);
    let target_tokens = summary_tokens(&target.summary);
    jaccard_with_shared(&source_tokens, &target_tokens)
}

fn summary_tokens(summary: &str) -> Vec<String> {
    summary
        .split(|ch: char| !ch.is_ascii_alphanumeric())
        .filter_map(|token| {
            let token = token.trim().to_ascii_lowercase();
            if is_informative_signal_token(&token) {
                Some(token)
            } else {
                None
            }
        })
        .collect::<Vec<_>>()
}

fn filter_signal_tokens(tokens: Vec<String>) -> Vec<String> {
    tokens
        .into_iter()
        .filter(|token| is_informative_signal_token(token))
        .take(SHARED_SIGNAL_EXPLANATION_LIMIT)
        .collect()
}

fn is_informative_signal_token(token: &str) -> bool {
    token.len() >= 3 && token.chars().any(|ch| ch.is_ascii_alphabetic())
}

fn container_identity(symbol_fqn: &str) -> Option<String> {
    let segments = symbol_fqn
        .split("::")
        .map(str::trim)
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>();
    if segments.len() < 2 {
        return None;
    }

    Some(segments[..segments.len() - 1].join("::"))
}

fn bool_score(value: bool) -> f32 {
    if value { 1.0 } else { 0.0 }
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

fn build_clone_input_hash(
    source: &SymbolCloneCandidateInput,
    target: &SymbolCloneCandidateInput,
) -> String {
    sha256_hex(
        &json!({
            "fingerprint_version": SYMBOL_CLONE_FINGERPRINT_VERSION,
            "source_symbol_id": &source.symbol_id,
            "target_symbol_id": &target.symbol_id,
            "source_artefact_id": &source.artefact_id,
            "target_artefact_id": &target.artefact_id,
            "source_summary": &source.summary,
            "target_summary": &target.summary,
            "source_name": &source.normalized_name,
            "target_name": &target.normalized_name,
            "source_signature": &source.normalized_signature,
            "target_signature": &target.normalized_signature,
            "source_body_tokens": &source.normalized_body_tokens,
            "target_body_tokens": &target.normalized_body_tokens,
            "source_calls": &source.call_targets,
            "target_calls": &target.call_targets,
            "source_dependencies": &source.dependency_targets,
            "target_dependencies": &target.dependency_targets,
            "source_churn": source.churn_count,
            "target_churn": target.churn_count,
        })
        .to_string(),
    )
}

fn compatible_kind_score(left: &str, right: &str) -> f32 {
    if same_clone_kind(left, right) {
        return 1.0;
    }
    0.0
}

fn same_clone_kind(left: &str, right: &str) -> bool {
    left.trim().eq_ignore_ascii_case(right.trim())
}

fn is_meaningful_clone_candidate(input: &SymbolCloneCandidateInput) -> bool {
    if input.canonical_kind.eq_ignore_ascii_case("import") {
        return false;
    }
    true
}

fn signature_similarity(
    source: &SymbolCloneCandidateInput,
    target: &SymbolCloneCandidateInput,
) -> f32 {
    match (&source.normalized_signature, &target.normalized_signature) {
        (Some(left), Some(right)) if left == right => 1.0,
        (Some(_), Some(_)) => MISSING_SIGNATURE_SCORE,
        (None, None) => 1.0,
        _ => MISSING_SIGNATURE_SCORE,
    }
}

fn name_match_score(left: &str, right: &str) -> f32 {
    if left.is_empty() || right.is_empty() {
        return 0.0;
    }
    if left == right {
        return 1.0;
    }
    if left.starts_with(right) || right.starts_with(left) {
        return PARTIAL_NAME_MATCH_SCORE;
    }

    let left_tokens = left.split('_').collect::<Vec<_>>();
    let right_tokens = right.split('_').collect::<Vec<_>>();
    let shared_prefix = left_tokens
        .iter()
        .zip(right_tokens.iter())
        .take_while(|(left, right)| left == right)
        .count();
    match shared_prefix {
        2.. => PARTIAL_NAME_MATCH_SCORE,
        1 => SINGLE_SHARED_NAME_PREFIX_SCORE,
        _ => 0.0,
    }
}

fn path_similarity(left: &str, right: &str) -> f32 {
    let left_segments = left
        .split('/')
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>();
    let right_segments = right
        .split('/')
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>();
    if left_segments.is_empty() || right_segments.is_empty() {
        return 0.0;
    }

    let mut shared = 0usize;
    for (left, right) in left_segments.iter().zip(right_segments.iter()) {
        if left == right {
            shared += 1;
        } else {
            break;
        }
    }

    shared as f32 / left_segments.len().max(right_segments.len()) as f32
}

fn jaccard_with_shared(left: &[String], right: &[String]) -> (f32, Vec<String>) {
    let left_set = left
        .iter()
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .collect::<HashSet<_>>();
    let right_set = right
        .iter()
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .collect::<HashSet<_>>();

    if left_set.is_empty() && right_set.is_empty() {
        return (1.0, Vec::new());
    }
    if left_set.is_empty() || right_set.is_empty() {
        return (0.0, Vec::new());
    }

    let shared = left_set
        .intersection(&right_set)
        .cloned()
        .collect::<BTreeSet<_>>();
    let union_count = left_set.union(&right_set).count();
    (
        shared.len() as f32 / union_count as f32,
        shared
            .into_iter()
            .take(SHARED_SIGNAL_EXPLANATION_LIMIT)
            .collect(),
    )
}

fn normalized_body_hash(input: &SymbolCloneCandidateInput) -> String {
    sha256_hex(&input.normalized_body_tokens.join("|"))
}

fn normalized_signature_hash(input: &SymbolCloneCandidateInput) -> String {
    sha256_hex(
        &json!({
            "kind": input.canonical_kind,
            "parent_kind": input.parent_kind,
            "normalized_signature": input.normalized_signature,
        })
        .to_string(),
    )
}

fn is_experimental_path(path: &str) -> bool {
    let normalized = path.to_ascii_lowercase();
    normalized.contains("/experimental/")
        || normalized.contains("/playground/")
        || normalized.contains("/tmp/")
        || normalized.contains("/fixtures/")
}

fn sha256_hex(input: &str) -> String {
    let digest = Sha256::digest(input.as_bytes());
    let mut out = String::with_capacity(digest.len() * 2);
    for byte in digest {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_input(symbol_id: &str, name: &str) -> SymbolCloneCandidateInput {
        SymbolCloneCandidateInput {
            repo_id: "repo-1".to_string(),
            symbol_id: symbol_id.to_string(),
            artefact_id: format!("artefact-{symbol_id}"),
            path: "src/services/orders.ts".to_string(),
            canonical_kind: "function".to_string(),
            symbol_fqn: format!("src/services/orders.ts::{name}"),
            summary: format!("Function {name}."),
            normalized_name: name.to_string(),
            normalized_signature: Some(format!("function {name}(id: string, opts: number)")),
            identifier_tokens: vec!["order".to_string(), "fetch".to_string(), "id".to_string()],
            normalized_body_tokens: vec![
                "return".to_string(),
                "db".to_string(),
                "order".to_string(),
                "fetch".to_string(),
            ],
            parent_kind: Some("module".to_string()),
            context_tokens: vec!["services".to_string(), "orders".to_string()],
            embedding: vec![0.9, 0.1, 0.0],
            call_targets: vec!["db.fetchOrder".to_string()],
            dependency_targets: vec!["references:order_repository::entity".to_string()],
            churn_count: 1,
        }
    }

    #[test]
    fn build_symbol_clone_edges_marks_exact_duplicates() {
        let source = sample_input("source", "fetch_order");
        let mut target = sample_input("target", "fetch_order");
        target.path = "src/services/order_copies.ts".to_string();
        target.symbol_fqn = "src/services/order_copies.ts::fetch_order".to_string();

        let result = build_symbol_clone_edges(&[source, target]);

        assert_eq!(result.edges.len(), 2);
        assert!(
            result
                .edges
                .iter()
                .all(|edge| edge.relation_kind == RELATION_KIND_EXACT_DUPLICATE)
        );
    }

    #[test]
    fn build_symbol_clone_edges_marks_diverged_implementations() {
        let mut source = sample_input("source", "validate_order_checkout");
        source.embedding = vec![0.9, 0.2, 0.0];
        source.call_targets = vec!["rules.checkout".to_string()];
        source.normalized_body_tokens = vec!["validate".to_string(), "checkout".to_string()];

        let mut target = sample_input("target", "validate_order_draft");
        target.embedding = vec![0.7, 0.3, 0.0];
        target.call_targets = vec!["rules.draft".to_string()];
        target.normalized_body_tokens = vec!["validate".to_string(), "draft".to_string()];

        let result = build_symbol_clone_edges(&[source, target]);

        assert!(
            result
                .edges
                .iter()
                .any(|edge| edge.relation_kind == RELATION_KIND_DIVERGED_IMPLEMENTATION)
        );
    }

    #[test]
    fn build_symbol_clone_edges_skips_cross_kind_matches() {
        let source = sample_input("source", "get_root_handler");

        let mut target = sample_input("target", "root_ts");
        target.canonical_kind = "file".to_string();
        target.symbol_fqn = "src/services/orders.ts".to_string();
        target.normalized_signature = None;

        let result = build_symbol_clone_edges(&[source, target]);

        assert!(result.edges.is_empty());
    }

    #[test]
    fn build_symbol_clone_edges_skips_import_candidates() {
        let mut source = sample_input("source", "import_src");
        source.canonical_kind = "import".to_string();
        source.normalized_signature = Some("import foo from 'bar'".to_string());
        source.identifier_tokens = vec!["foo".to_string(), "bar".to_string(), "import".to_string()];
        source.normalized_body_tokens = vec!["import".to_string(), "foo".to_string()];

        let mut target = sample_input("target", "import_target");
        target.canonical_kind = "import".to_string();
        target.normalized_signature = Some("import baz from 'bar'".to_string());
        target.identifier_tokens = vec!["baz".to_string(), "bar".to_string(), "import".to_string()];
        target.normalized_body_tokens = vec!["import".to_string(), "baz".to_string()];

        let result = build_symbol_clone_edges(&[source, target]);

        assert!(result.edges.is_empty());
        assert_eq!(result.sources_considered, 0);
    }

    #[test]
    fn build_symbol_clone_edges_labels_preferred_local_patterns() {
        let mut source = sample_input("source", "render_invoice_document");
        source.embedding = vec![0.8, 0.2, 0.1];

        let mut target = sample_input("target", "create_invoice_pdf");
        target.embedding = vec![0.82, 0.18, 0.1];
        target.churn_count = 1;
        target.path = "src/billing/invoice.ts".to_string();

        let result = build_symbol_clone_edges(&[source, target]);
        let labels = result.edges[0]
            .explanation_json
            .get("labels")
            .and_then(Value::as_array);
        assert!(labels.is_some());
    }

    #[test]
    fn build_symbol_clone_edges_marks_contextual_neighbors_when_locality_dominates() {
        let mut source = sample_input("source", "execute");
        source.canonical_kind = "method".to_string();
        source.parent_kind = Some("class_declaration".to_string());
        source.path = "src/handlers/change-path.ts".to_string();
        source.symbol_fqn =
            "src/handlers/change-path.ts::ChangePathOfCodeFileCommandHandler::execute".to_string();
        source.summary = "Method execute. Applies the path change workflow.".to_string();
        source.identifier_tokens = vec![
            "change".to_string(),
            "path".to_string(),
            "code".to_string(),
            "file".to_string(),
        ];
        source.normalized_body_tokens = vec![
            "load".to_string(),
            "validate".to_string(),
            "rename".to_string(),
        ];
        source.call_targets = vec!["repo.loadFile".to_string(), "domain.renamePath".to_string()];

        let mut target = sample_input("target", "command");
        target.canonical_kind = "method".to_string();
        target.parent_kind = Some("class_declaration".to_string());
        target.path = source.path.clone();
        target.symbol_fqn =
            "src/handlers/change-path.ts::ChangePathOfCodeFileCommandHandler::command".to_string();
        target.summary = "Method command. Returns the command payload.".to_string();
        target.identifier_tokens = vec![
            "change".to_string(),
            "path".to_string(),
            "command".to_string(),
            "file".to_string(),
        ];
        target.normalized_body_tokens = vec![
            "return".to_string(),
            "command".to_string(),
            "payload".to_string(),
        ];
        target.call_targets = vec!["factory.buildCommand".to_string()];

        let result = build_symbol_clone_edges(&[source, target]);
        let edge = result
            .edges
            .iter()
            .find(|edge| edge.target_symbol_id == "target")
            .expect("contextual neighbor edge");

        assert_eq!(edge.relation_kind, RELATION_KIND_WEAK_CLONE_CANDIDATE);
        assert!(edge.score < 0.75);
        assert_eq!(
            edge.explanation_json["confidence"]["confidence_band"],
            Value::String("weak".to_string())
        );
        assert!(
            edge.explanation_json["evidence"]["bias_warning"].as_str() == Some("same_file_bias")
        );
    }

    #[test]
    fn build_symbol_clone_edges_keeps_same_file_clone_confidence_when_impl_is_strong() {
        let mut source = sample_input("source", "apply_path_change");
        source.canonical_kind = "method".to_string();
        source.parent_kind = Some("class_declaration".to_string());
        source.path = "src/handlers/change-path.ts".to_string();
        source.symbol_fqn =
            "src/handlers/change-path.ts::ChangePathOfCodeFileCommandHandler::apply_path_change"
                .to_string();
        source.summary =
            "Method apply path change. Applies the path change to the file.".to_string();
        source.identifier_tokens = vec![
            "apply".to_string(),
            "path".to_string(),
            "change".to_string(),
            "file".to_string(),
        ];
        source.normalized_body_tokens = vec![
            "load".to_string(),
            "validate".to_string(),
            "rename".to_string(),
            "persist".to_string(),
        ];
        source.call_targets = vec![
            "repo.loadFile".to_string(),
            "domain.renamePath".to_string(),
            "repo.persistFile".to_string(),
        ];

        let mut target = sample_input("target", "apply_path_change_for_move");
        target.canonical_kind = "method".to_string();
        target.parent_kind = Some("class_declaration".to_string());
        target.path = source.path.clone();
        target.symbol_fqn = "src/handlers/change-path.ts::ChangePathOfCodeFileCommandHandler::apply_path_change_for_move".to_string();
        target.summary =
            "Method apply path change for move. Applies the path change and persists it."
                .to_string();
        target.identifier_tokens = vec![
            "apply".to_string(),
            "path".to_string(),
            "change".to_string(),
            "move".to_string(),
        ];
        target.normalized_body_tokens = vec![
            "load".to_string(),
            "validate".to_string(),
            "rename".to_string(),
            "persist".to_string(),
            "emit".to_string(),
        ];
        target.call_targets = vec![
            "repo.loadFile".to_string(),
            "domain.renamePath".to_string(),
            "repo.persistFile".to_string(),
        ];

        let result = build_symbol_clone_edges(&[source, target]);
        let edge = result
            .edges
            .iter()
            .find(|edge| edge.target_symbol_id == "target")
            .expect("same-file strong clone edge");

        assert_ne!(edge.relation_kind, RELATION_KIND_WEAK_CLONE_CANDIDATE);
        assert!(
            edge.explanation_json["confidence"]["clone_confidence"]
                .as_f64()
                .expect("clone confidence")
                >= CLONE_CONFIDENCE_MEDIUM_THRESHOLD as f64
        );
        assert!(edge.explanation_json["evidence"]["bias_warning"].is_null());
    }

    #[test]
    fn build_symbol_clone_edges_exposes_dependency_overlap() {
        let mut source = sample_input("source", "validate_path");
        source.call_targets = vec!["repo.loadFile".to_string()];
        source.dependency_targets = vec![
            "references:path_service::path".to_string(),
            "implements:path_validator".to_string(),
        ];

        let mut target = sample_input("target", "validate_moved_path");
        target.call_targets = vec!["repo.loadMovedFile".to_string()];
        target.dependency_targets = vec![
            "references:path_service::path".to_string(),
            "implements:path_validator".to_string(),
        ];

        let result = build_symbol_clone_edges(&[source, target]);
        let edge = result
            .edges
            .iter()
            .find(|edge| edge.target_symbol_id == "target")
            .expect("dependency-aware clone edge");

        assert!(
            edge.explanation_json["scores"]["dependency_overlap"]
                .as_f64()
                .expect("dependency overlap")
                > 0.0
        );
        assert_eq!(
            edge.explanation_json["evidence"]["shared_signals"]["dependency_targets"][0],
            Value::String("implements:path_validator".to_string())
        );
    }
}
