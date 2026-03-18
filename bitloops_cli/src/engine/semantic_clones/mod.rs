use std::collections::{BTreeSet, HashSet};

use serde_json::{Value, json};
use sha2::{Digest, Sha256};

const SYMBOL_CLONE_FINGERPRINT_VERSION: &str = "symbol-clone-fingerprint-v1";
const MAX_CLONE_EDGES_PER_SOURCE: usize = 20;
const MIN_SIMILAR_IMPLEMENTATION_SCORE: f32 = 0.55;
const MIN_SEMANTIC_SCORE: f32 = 0.40;
const EXACT_DUPLICATE_SCORE_FLOOR: f32 = 0.99;
const PREFERRED_LOCAL_PATTERN_SCORE_THRESHOLD: f32 = 0.72;
const PREFERRED_LOCAL_PATTERN_MAX_CHURN_COUNT: usize = 2;
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
const STRUCTURAL_WEIGHT_ARITY: f32 = 0.20;
const STRUCTURAL_WEIGHT_PATH: f32 = 0.20;
const STRUCTURAL_WEIGHT_CALL: f32 = 0.15;
const STRUCTURAL_SCORE_FLOOR_SAME_KIND_WEIGHT: f32 = 0.25;
const STRUCTURAL_SCORE_FLOOR_NAME_MATCH_WEIGHT: f32 = 0.10;

const DIVERGED_NAME_MATCH_THRESHOLD: f32 = 0.75;
const DIVERGED_PATH_SCORE_THRESHOLD: f32 = 0.60;
const DIVERGED_IDENTIFIER_OVERLAP_THRESHOLD: f32 = 0.30;
const DIVERGED_MIN_SEMANTIC_SCORE: f32 = 0.35;
const DIVERGED_MAX_CALL_OVERLAP: f32 = 0.25;
const DIVERGED_MAX_BODY_OVERLAP: f32 = 0.45;

const SHARED_LOGIC_MIN_LEXICAL_SCORE: f32 = 0.68;
const SHARED_LOGIC_MIN_BODY_OVERLAP: f32 = 0.50;
const SHARED_LOGIC_MIN_STRUCTURAL_SCORE: f32 = 0.58;
const SHARED_LOGIC_MIN_SEMANTIC_SCORE: f32 = 0.42;

const MISSING_PARENT_KIND_SCORE: f32 = 0.40;
const MISSING_SIGNATURE_SCORE: f32 = 0.25;
const PARTIAL_NAME_MATCH_SCORE: f32 = 0.75;
const SINGLE_SHARED_NAME_PREFIX_SCORE: f32 = 0.50;
const MISSING_ARITY_SCORE: f32 = 0.25;
const ARITY_DELTA_ONE_SCORE: f32 = 0.75;
const ARITY_DELTA_TWO_SCORE: f32 = 0.50;
const SHARED_SIGNAL_EXPLANATION_LIMIT: usize = 8;

pub const RELATION_KIND_EXACT_DUPLICATE: &str = "exact_duplicate";
pub const RELATION_KIND_SIMILAR_IMPLEMENTATION: &str = "similar_implementation";
pub const RELATION_KIND_SHARED_LOGIC_CANDIDATE: &str = "shared_logic_candidate";
pub const RELATION_KIND_DIVERGED_IMPLEMENTATION: &str = "diverged_implementation";
pub const LABEL_PREFERRED_LOCAL_PATTERN: &str = "preferred_local_pattern";

const CLONE_CANDIDATE_KINDS: &[&str] = &[
    "file",
    "module",
    "function",
    "method",
    "class",
    "interface",
    "enum",
    "constructor",
    "test",
];

#[derive(Debug, Clone, PartialEq)]
pub struct SymbolCloneCandidateInput {
    pub repo_id: String,
    pub symbol_id: String,
    pub artefact_id: String,
    pub path: String,
    pub language: String,
    pub canonical_kind: String,
    pub language_kind: String,
    pub symbol_fqn: String,
    pub signature: Option<String>,
    pub summary: String,
    pub normalized_name: String,
    pub normalized_signature: Option<String>,
    pub identifier_tokens: Vec<String>,
    pub normalized_body_tokens: Vec<String>,
    pub parent_kind: Option<String>,
    pub context_tokens: Vec<String>,
    pub embedding: Vec<f32>,
    pub call_targets: Vec<String>,
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

pub fn clone_candidate_kinds() -> &'static [&'static str] {
    CLONE_CANDIDATE_KINDS
}

pub fn is_clone_candidate_kind(kind: &str) -> bool {
    CLONE_CANDIDATE_KINDS.contains(&kind.trim().to_ascii_lowercase().as_str())
}

pub fn build_symbol_clone_edges(inputs: &[SymbolCloneCandidateInput]) -> SymbolCloneBuildResult {
    let mut edges = Vec::new();

    for source in inputs {
        let mut source_edges = inputs
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
        sources_considered: inputs.len(),
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
    let mut score = (CLONE_SCORE_WEIGHT_SEMANTIC * semantic_score)
        + (CLONE_SCORE_WEIGHT_LEXICAL * lexical.score)
        + (CLONE_SCORE_WEIGHT_STRUCTURAL * structural.score);

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
    } else if likely_diverged_implementation(semantic_score, &lexical, &structural) {
        RELATION_KIND_DIVERGED_IMPLEMENTATION.to_string()
    } else if likely_shared_logic_candidate(semantic_score, &lexical, &structural) {
        RELATION_KIND_SHARED_LOGIC_CANDIDATE.to_string()
    } else if score >= MIN_SIMILAR_IMPLEMENTATION_SCORE && semantic_score >= MIN_SEMANTIC_SCORE {
        RELATION_KIND_SIMILAR_IMPLEMENTATION.to_string()
    } else {
        return None;
    };

    let mut labels = Vec::new();
    if relation_kind != RELATION_KIND_EXACT_DUPLICATE
        && score >= PREFERRED_LOCAL_PATTERN_SCORE_THRESHOLD
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
        relation_kind: &relation_kind,
        semantic_score,
        lexical: &lexical,
        structural: &structural,
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
    shared_identifier_tokens: Vec<String>,
    shared_context_tokens: Vec<String>,
}

fn lexical_signals(
    source: &SymbolCloneCandidateInput,
    target: &SymbolCloneCandidateInput,
) -> LexicalSignals {
    let (identifier_overlap, shared_identifier_tokens) =
        jaccard_with_shared(&source.identifier_tokens, &target.identifier_tokens);
    let (body_overlap, _) = jaccard_with_shared(
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
        shared_identifier_tokens,
        shared_context_tokens,
    }
}

#[derive(Debug, Clone)]
struct StructuralSignals {
    score: f32,
    same_kind: f32,
    same_parent_kind: f32,
    arity_score: f32,
    path_score: f32,
    call_score: f32,
    shared_call_targets: Vec<String>,
}

struct ExplanationContext<'a> {
    source: &'a SymbolCloneCandidateInput,
    target: &'a SymbolCloneCandidateInput,
    relation_kind: &'a str,
    semantic_score: f32,
    lexical: &'a LexicalSignals,
    structural: &'a StructuralSignals,
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
    let arity_score = arity_score(source.signature.as_deref(), target.signature.as_deref());
    let path_score = path_similarity(&source.path, &target.path);
    let (call_score, shared_call_targets) =
        jaccard_with_shared(&source.call_targets, &target.call_targets);
    let score = ((STRUCTURAL_WEIGHT_SAME_KIND * same_kind)
        + (STRUCTURAL_WEIGHT_SAME_PARENT_KIND * same_parent_kind)
        + (STRUCTURAL_WEIGHT_ARITY * arity_score)
        + (STRUCTURAL_WEIGHT_PATH * path_score)
        + (STRUCTURAL_WEIGHT_CALL * call_score))
        .clamp(0.0, 1.0)
        .max(
            (same_kind * STRUCTURAL_SCORE_FLOOR_SAME_KIND_WEIGHT)
                + (name_match * STRUCTURAL_SCORE_FLOOR_NAME_MATCH_WEIGHT),
        );

    StructuralSignals {
        score,
        same_kind,
        same_parent_kind,
        arity_score,
        path_score,
        call_score,
        shared_call_targets,
    }
}

fn likely_diverged_implementation(
    semantic_score: f32,
    lexical: &LexicalSignals,
    structural: &StructuralSignals,
) -> bool {
    (lexical.name_match >= DIVERGED_NAME_MATCH_THRESHOLD
        || structural.path_score >= DIVERGED_PATH_SCORE_THRESHOLD)
        && lexical.identifier_overlap >= DIVERGED_IDENTIFIER_OVERLAP_THRESHOLD
        && semantic_score >= DIVERGED_MIN_SEMANTIC_SCORE
        && structural.call_score <= DIVERGED_MAX_CALL_OVERLAP
        && lexical.body_overlap <= DIVERGED_MAX_BODY_OVERLAP
}

fn likely_shared_logic_candidate(
    semantic_score: f32,
    lexical: &LexicalSignals,
    structural: &StructuralSignals,
) -> bool {
    lexical.score >= SHARED_LOGIC_MIN_LEXICAL_SCORE
        && lexical.body_overlap >= SHARED_LOGIC_MIN_BODY_OVERLAP
        && structural.score >= SHARED_LOGIC_MIN_STRUCTURAL_SCORE
        && semantic_score >= SHARED_LOGIC_MIN_SEMANTIC_SCORE
}

fn build_explanation(ctx: &ExplanationContext<'_>) -> Value {
    let explanation = match ctx.relation_kind {
        RELATION_KIND_EXACT_DUPLICATE => {
            "Same normalized body tokens and signature shape; treat as an exact duplicate."
                .to_string()
        }
        RELATION_KIND_DIVERGED_IMPLEMENTATION => {
            "Name or path ancestry stays close, but the implementation has diverged in body tokens and call targets."
                .to_string()
        }
        RELATION_KIND_SHARED_LOGIC_CANDIDATE => {
            "High lexical and structural overlap suggests repeated local logic that could be extracted."
                .to_string()
        }
        _ => "Strong semantic match with overlapping identifiers, context, and compatible structure."
            .to_string(),
    };

    json!({
        "explanation": explanation,
        "source_summary": ctx.source.summary,
        "target_summary": ctx.target.summary,
        "shared_identifier_tokens": ctx.lexical.shared_identifier_tokens,
        "shared_context_tokens": ctx.lexical.shared_context_tokens,
        "shared_call_targets": ctx.structural.shared_call_targets,
        "duplicate_signals": {
            "body_hash_match": ctx.duplicate_body_hash_match,
            "signature_shape_match": ctx.signature_shape_hash_match,
        },
        "scores": {
            "semantic": ctx.semantic_score,
            "lexical": ctx.lexical.score,
            "structural": ctx.structural.score,
            "identifier_overlap": ctx.lexical.identifier_overlap,
            "body_overlap": ctx.lexical.body_overlap,
            "context_overlap": ctx.lexical.context_overlap,
            "signature_similarity": ctx.lexical.signature_similarity,
            "same_kind": ctx.structural.same_kind,
            "same_parent_kind": ctx.structural.same_parent_kind,
            "arity": ctx.structural.arity_score,
            "path_ancestry": ctx.structural.path_score,
            "call_overlap": ctx.structural.call_score,
        },
        "labels": ctx.labels,
    })
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

fn signature_similarity(
    source: &SymbolCloneCandidateInput,
    target: &SymbolCloneCandidateInput,
) -> f32 {
    match (&source.normalized_signature, &target.normalized_signature) {
        (Some(left), Some(right)) if left == right => 1.0,
        (Some(_), Some(_)) => arity_score(source.signature.as_deref(), target.signature.as_deref()),
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

fn arity_score(left: Option<&str>, right: Option<&str>) -> f32 {
    match (extract_arity(left), extract_arity(right)) {
        (Some(left), Some(right)) if left == right => 1.0,
        (Some(left), Some(right)) => {
            let delta = left.abs_diff(right);
            match delta {
                0 => 1.0,
                1 => ARITY_DELTA_ONE_SCORE,
                2 => ARITY_DELTA_TWO_SCORE,
                _ => 0.0,
            }
        }
        (None, None) => 1.0,
        _ => MISSING_ARITY_SCORE,
    }
}

fn extract_arity(signature: Option<&str>) -> Option<usize> {
    let signature = signature?.trim();
    let open_idx = signature.find('(')?;
    let mut depth = 0_i32;
    let mut close_idx = None;
    for (idx, ch) in signature[open_idx..].char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    close_idx = Some(open_idx + idx);
                    break;
                }
            }
            _ => {}
        }
    }
    let close_idx = close_idx?;
    let params = &signature[(open_idx + 1)..close_idx];
    if params.trim().is_empty() {
        return Some(0);
    }

    let mut count = 1usize;
    let mut nesting = 0_i32;
    for ch in params.chars() {
        match ch {
            '<' | '[' | '{' | '(' => nesting += 1,
            '>' | ']' | '}' | ')' => nesting = (nesting - 1).max(0),
            ',' if nesting == 0 => count += 1,
            _ => {}
        }
    }
    Some(count)
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
            "arity": extract_arity(input.signature.as_deref()),
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
            language: "typescript".to_string(),
            canonical_kind: "function".to_string(),
            language_kind: "function_declaration".to_string(),
            symbol_fqn: format!("src/services/orders.ts::{name}"),
            signature: Some(format!("function {name}(id: string, opts: number)")),
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
            churn_count: 1,
        }
    }

    #[test]
    fn clone_candidate_kind_whitelist_covers_mvp_symbols() {
        assert!(is_clone_candidate_kind("function"));
        assert!(is_clone_candidate_kind("method"));
        assert!(is_clone_candidate_kind("file"));
        assert!(!is_clone_candidate_kind("variable"));
        assert!(!is_clone_candidate_kind("import_statement"));
    }

    #[test]
    fn build_symbol_clone_edges_marks_exact_duplicates() {
        let source = sample_input("source", "fetch_order");
        let target = sample_input("target", "fetch_order_copy");

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
        target.language_kind = "file".to_string();
        target.symbol_fqn = "src/services/orders.ts".to_string();
        target.signature = None;
        target.normalized_signature = None;

        let result = build_symbol_clone_edges(&[source, target]);

        assert!(result.edges.is_empty());
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
}
