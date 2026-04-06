use std::collections::{BTreeSet, HashMap, HashSet};

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

pub const DEFAULT_ANN_NEIGHBORS: usize = 5;
pub const MIN_ANN_NEIGHBORS: usize = 1;
pub const MAX_ANN_NEIGHBORS: usize = 50;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CloneScoringOptions {
    pub ann_neighbors: usize,
}

impl Default for CloneScoringOptions {
    fn default() -> Self {
        Self {
            ann_neighbors: DEFAULT_ANN_NEIGHBORS,
        }
    }
}

impl CloneScoringOptions {
    pub fn new(ann_neighbors: usize) -> Self {
        Self {
            ann_neighbors: ann_neighbors.clamp(MIN_ANN_NEIGHBORS, MAX_ANN_NEIGHBORS),
        }
    }

    pub fn from_i64_clamped(value: i64) -> Self {
        let value = if value < MIN_ANN_NEIGHBORS as i64 {
            MIN_ANN_NEIGHBORS
        } else if value > MAX_ANN_NEIGHBORS as i64 {
            MAX_ANN_NEIGHBORS
        } else {
            value as usize
        };
        Self::new(value)
    }
}

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
    pub embedding_provider: String,
    pub embedding_model: String,
    pub embedding_dimension: usize,
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

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct CandidateGroupKey {
    repo_id: String,
    effective_kind: String,
    embedding_provider: String,
    embedding_model: String,
    embedding_dimension: usize,
}

#[derive(Debug)]
struct GroupAnnIndex {
    global_indices: Vec<usize>,
    local_by_global: HashMap<usize, usize>,
    index: ann::HnswLikeIndex,
}

pub fn build_symbol_clone_edges(inputs: &[SymbolCloneCandidateInput]) -> SymbolCloneBuildResult {
    build_symbol_clone_edges_with_options(inputs, CloneScoringOptions::default())
}

pub fn build_symbol_clone_edges_with_options(
    inputs: &[SymbolCloneCandidateInput],
    options: CloneScoringOptions,
) -> SymbolCloneBuildResult {
    let candidates = inputs
        .iter()
        .filter(|input| is_meaningful_clone_candidate(input))
        .collect::<Vec<_>>();
    build_symbol_clone_edges_for_sources(&candidates, &candidates, options)
}

pub fn build_symbol_clone_edges_for_source_with_options(
    inputs: &[SymbolCloneCandidateInput],
    source_symbol_id: &str,
    options: CloneScoringOptions,
) -> SymbolCloneBuildResult {
    let candidates = inputs
        .iter()
        .filter(|input| is_meaningful_clone_candidate(input))
        .collect::<Vec<_>>();
    let sources = candidates
        .iter()
        .copied()
        .filter(|candidate| candidate.symbol_id == source_symbol_id)
        .collect::<Vec<_>>();
    build_symbol_clone_edges_for_sources(&candidates, &sources, options)
}

fn build_symbol_clone_edges_for_sources(
    candidates: &[&SymbolCloneCandidateInput],
    sources: &[&SymbolCloneCandidateInput],
    options: CloneScoringOptions,
) -> SymbolCloneBuildResult {
    if candidates.is_empty() || sources.is_empty() {
        return SymbolCloneBuildResult {
            edges: Vec::new(),
            sources_considered: sources.len(),
        };
    }

    let mut group_indices = HashMap::<CandidateGroupKey, Vec<usize>>::new();
    for (idx, candidate) in candidates.iter().enumerate() {
        group_indices
            .entry(candidate_group_key(candidate))
            .or_default()
            .push(idx);
    }

    let group_ann_indexes = build_group_ann_indexes(candidates, &group_indices);
    let duplicate_buckets = build_duplicate_buckets(candidates, &group_indices);

    let mut candidate_index_by_symbol_id =
        HashMap::<String, usize>::with_capacity(candidates.len());
    for (idx, candidate) in candidates.iter().enumerate() {
        candidate_index_by_symbol_id.insert(candidate.symbol_id.clone(), idx);
    }

    let mut edges = Vec::new();
    for source in sources {
        let Some(source_idx) = candidate_index_by_symbol_id
            .get(source.symbol_id.as_str())
            .copied()
        else {
            continue;
        };

        let group_key = candidate_group_key(source);
        let mut target_indices = HashSet::<usize>::new();

        if let Some(group_ann_index) = group_ann_indexes.get(&group_key)
            && let Some(source_local_idx) = group_ann_index.local_by_global.get(&source_idx)
        {
            let ann_local = group_ann_index
                .index
                .nearest(*source_local_idx, options.ann_neighbors.saturating_add(1));
            for local_idx in ann_local {
                if let Some(global_idx) = group_ann_index.global_indices.get(local_idx).copied()
                    && global_idx != source_idx
                {
                    target_indices.insert(global_idx);
                }
            }
        }

        // Exact-duplicate recall: always include deterministic duplicate-bucket peers.
        if !source.normalized_body_tokens.is_empty() {
            let bucket_key = (
                normalized_body_hash(source),
                normalized_signature_hash(source),
            );
            if let Some(group_buckets) = duplicate_buckets.get(&group_key)
                && let Some(bucket_members) = group_buckets.get(&bucket_key)
            {
                for member_idx in bucket_members {
                    if *member_idx != source_idx {
                        target_indices.insert(*member_idx);
                    }
                }
            }
        }

        let mut source_edges = target_indices
            .into_iter()
            .filter_map(|target_idx| {
                let target = candidates.get(target_idx).copied()?;
                build_symbol_clone_edge(source, target)
            })
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
        sources_considered: sources.len(),
    }
}

fn build_group_ann_indexes(
    candidates: &[&SymbolCloneCandidateInput],
    group_indices: &HashMap<CandidateGroupKey, Vec<usize>>,
) -> HashMap<CandidateGroupKey, GroupAnnIndex> {
    let mut out = HashMap::with_capacity(group_indices.len());
    for (group_key, global_indices) in group_indices {
        if global_indices.len() < 2 {
            continue;
        }

        let vectors = global_indices
            .iter()
            .filter_map(|idx| candidates.get(*idx))
            .map(|candidate| candidate.embedding.clone())
            .collect::<Vec<_>>();
        if vectors.is_empty() {
            continue;
        }

        let index = ann::HnswLikeIndex::build(&vectors);
        let local_by_global = global_indices
            .iter()
            .enumerate()
            .map(|(local, global)| (*global, local))
            .collect::<HashMap<_, _>>();
        out.insert(
            group_key.clone(),
            GroupAnnIndex {
                global_indices: global_indices.clone(),
                local_by_global,
                index,
            },
        );
    }
    out
}

fn build_duplicate_buckets(
    candidates: &[&SymbolCloneCandidateInput],
    group_indices: &HashMap<CandidateGroupKey, Vec<usize>>,
) -> HashMap<CandidateGroupKey, HashMap<(String, String), Vec<usize>>> {
    let mut out = HashMap::with_capacity(group_indices.len());
    for (group_key, global_indices) in group_indices {
        let mut buckets = HashMap::<(String, String), Vec<usize>>::new();
        for candidate_idx in global_indices {
            let Some(candidate) = candidates.get(*candidate_idx).copied() else {
                continue;
            };
            if candidate.normalized_body_tokens.is_empty() {
                continue;
            }
            let key = (
                normalized_body_hash(candidate),
                normalized_signature_hash(candidate),
            );
            buckets.entry(key).or_default().push(*candidate_idx);
        }
        if !buckets.is_empty() {
            out.insert(group_key.clone(), buckets);
        }
    }
    out
}

fn candidate_group_key(candidate: &SymbolCloneCandidateInput) -> CandidateGroupKey {
    CandidateGroupKey {
        repo_id: candidate.repo_id.clone(),
        effective_kind: candidate.canonical_kind.trim().to_ascii_lowercase(),
        embedding_provider: candidate.embedding_provider.trim().to_ascii_lowercase(),
        embedding_model: candidate.embedding_model.trim().to_ascii_lowercase(),
        embedding_dimension: candidate.embedding_dimension,
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

// scoring: signal structs and score computation
mod core;
// classification: relation-kind predicates
mod classification;
// explanation: build_explanation, limiting signals, confidence band
mod explanation;
// utils: jaccard, hashing, token helpers, path/name similarity
mod utils;
// ann: in-memory HNSW-like nearest-neighbour index for semantic prefiltering
mod ann;

use self::classification::*;
use self::core::*;
use self::explanation::*;
use self::utils::*;

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
            embedding_provider: "local_fastembed".to_string(),
            embedding_model: "jinaai/jina-embeddings-v2-base-code".to_string(),
            embedding_dimension: 3,
            embedding: vec![0.9, 0.1, 0.0],
            call_targets: vec!["db.fetchOrder".to_string()],
            dependency_targets: vec!["references:order_repository::entity".to_string()],
            churn_count: 1,
        }
    }

    #[test]
    fn clone_scoring_options_clamp_neighbors() {
        assert_eq!(
            CloneScoringOptions::default().ann_neighbors,
            DEFAULT_ANN_NEIGHBORS
        );
        assert_eq!(CloneScoringOptions::new(0).ann_neighbors, MIN_ANN_NEIGHBORS);
        assert_eq!(
            CloneScoringOptions::new(MAX_ANN_NEIGHBORS + 10).ann_neighbors,
            MAX_ANN_NEIGHBORS
        );
        assert_eq!(
            CloneScoringOptions::from_i64_clamped(-10).ann_neighbors,
            MIN_ANN_NEIGHBORS
        );
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

    #[test]
    fn semantic_similarity_requires_matching_provider_model_and_dimension() {
        let source = sample_input("source", "fetch_order");
        let mut target = sample_input("target", "fetch_order_copy");
        target.embedding_provider = "voyage".to_string();
        assert_eq!(semantic_similarity(&source, &target), 0.0);

        let mut target = sample_input("target2", "fetch_order_copy_2");
        target.embedding_model = "other-model".to_string();
        assert_eq!(semantic_similarity(&source, &target), 0.0);

        let mut target = sample_input("target3", "fetch_order_copy_3");
        target.embedding_dimension = 6;
        assert_eq!(semantic_similarity(&source, &target), 0.0);
    }

    #[test]
    fn build_symbol_clone_edges_for_source_respects_ann_neighbors_prefilter() {
        let source = sample_input("source", "alpha");

        let mut top = sample_input("top", "alpha_top");
        top.path = "src/services/top.ts".to_string();
        top.symbol_fqn = "src/services/top.ts::alpha_top".to_string();
        top.embedding = vec![0.91, 0.09, 0.0];

        let mut other = sample_input("other", "alpha_other");
        other.path = "src/services/other.ts".to_string();
        other.symbol_fqn = "src/services/other.ts::alpha_other".to_string();
        other.embedding = vec![0.2, 0.8, 0.0];

        let result = build_symbol_clone_edges_for_source_with_options(
            &[source, top, other],
            "source",
            CloneScoringOptions::new(1),
        );

        let source_edges = result
            .edges
            .iter()
            .filter(|edge| edge.source_symbol_id == "source")
            .collect::<Vec<_>>();
        assert!(source_edges.len() <= 1);
    }

    #[test]
    fn low_ann_neighbors_keeps_exact_duplicate_recall_via_duplicate_bucket() {
        let mut source = sample_input("source", "fetch_order");
        source.embedding = vec![1.0, 0.0, 0.0];

        let mut nearest_non_duplicate = sample_input("nearest", "fetch_order_nearest");
        nearest_non_duplicate.path = "src/services/nearest.ts".to_string();
        nearest_non_duplicate.symbol_fqn =
            "src/services/nearest.ts::fetch_order_nearest".to_string();
        nearest_non_duplicate.normalized_name = "fetch_order_nearest".to_string();
        nearest_non_duplicate.normalized_body_tokens =
            vec!["fetch".to_string(), "nearest".to_string()];
        nearest_non_duplicate.normalized_signature =
            Some("function fetch_order_nearest(id: string)".to_string());
        nearest_non_duplicate.embedding = vec![0.99, 0.01, 0.0];

        let mut exact_duplicate = sample_input("duplicate", "fetch_order");
        exact_duplicate.path = "src/services/duplicate.ts".to_string();
        exact_duplicate.symbol_fqn = "src/services/duplicate.ts::fetch_order".to_string();
        // Make the duplicate semantically far so ANN(1) would likely miss it without backfill.
        exact_duplicate.embedding = vec![0.0, 1.0, 0.0];

        let result = build_symbol_clone_edges_for_source_with_options(
            &[source, nearest_non_duplicate, exact_duplicate],
            "source",
            CloneScoringOptions::new(1),
        );

        assert!(result.edges.iter().any(|edge| {
            edge.source_symbol_id == "source"
                && edge.target_symbol_id == "duplicate"
                && edge.relation_kind == RELATION_KIND_EXACT_DUPLICATE
        }));
    }
}
