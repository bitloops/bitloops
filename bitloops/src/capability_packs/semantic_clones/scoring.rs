use std::collections::{BTreeSet, HashMap, HashSet};

use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::capability_packs::semantic_clones::embeddings::EmbeddingSetup;
use crate::vector_search::HnswLikeIndex;

const SYMBOL_CLONE_FINGERPRINT_VERSION: &str = "symbol-clone-fingerprint-v3";
const MAX_CLONE_EDGES_PER_SOURCE: usize = 20;
const MIN_SIMILAR_IMPLEMENTATION_SCORE: f32 = 0.55;
const MIN_SEMANTIC_SCORE: f32 = 0.40;
const EXACT_DUPLICATE_SCORE_FLOOR: f32 = 0.99;
const CONTEXTUAL_NEIGHBOR_MIN_SCORE: f32 = 0.50;
const CONTEXTUAL_NEIGHBOR_MIN_SEMANTIC_SCORE: f32 = 0.55;
const PREFERRED_LOCAL_PATTERN_SCORE_THRESHOLD: f32 = 0.72;
const PREFERRED_LOCAL_PATTERN_MAX_CHURN_COUNT: usize = 2;
const PREFERRED_LOCAL_PATTERN_MIN_CLONE_CONFIDENCE: f32 = 0.45;
const PREFERRED_LOCAL_PATTERN_SCORE_BOOST: f32 = 0.05;
const PREFERRED_LOCAL_PATTERN_SCORE_CAP: f32 = 0.98;

const CLONE_SCORE_WEIGHT_SEMANTIC: f32 = 0.55;
const CLONE_SCORE_WEIGHT_LEXICAL: f32 = 0.25;
const CLONE_SCORE_WEIGHT_STRUCTURAL: f32 = 0.20;
const SEMANTIC_WEIGHT_CODE_EMBEDDING: f32 = 0.50;
const SEMANTIC_WEIGHT_SUMMARY_EMBEDDING: f32 = 0.50;
const MULTI_VIEW_HIGH_SIMILARITY_THRESHOLD: f32 = 0.72;
const MULTI_VIEW_LOW_SIMILARITY_THRESHOLD: f32 = 0.45;
const MULTI_VIEW_SIMILARITY_GAP_THRESHOLD: f32 = 0.20;

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
const DIVERGED_IDENTIFIER_OVERLAP_THRESHOLD: f32 = 0.30;
const DIVERGED_MIN_BODY_OVERLAP: f32 = 0.08;

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
pub const DISABLE_ANN_ENV: &str = "BITLOOPS_SEMANTIC_CLONES_DISABLE_ANN";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CloneScoringOptions {
    pub ann_neighbors: usize,
    pub ann_enabled: bool,
}

impl Default for CloneScoringOptions {
    fn default() -> Self {
        Self {
            ann_neighbors: DEFAULT_ANN_NEIGHBORS,
            ann_enabled: true,
        }
    }
}

impl CloneScoringOptions {
    pub fn new(ann_neighbors: usize) -> Self {
        Self {
            ann_neighbors: ann_neighbors.clamp(MIN_ANN_NEIGHBORS, MAX_ANN_NEIGHBORS),
            ann_enabled: true,
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

    pub fn with_ann_enabled(mut self, ann_enabled: bool) -> Self {
        self.ann_enabled = ann_enabled;
        self
    }

    #[cfg(test)]
    fn apply_ann_override_raw(mut self, raw: Option<&str>) -> Self {
        if raw.is_some_and(ann_disabled_from_raw) {
            self.ann_enabled = false;
        }
        self
    }

    fn apply_env_overrides(self) -> Self {
        if ann_disabled_from_env() {
            self.with_ann_enabled(false)
        } else {
            self
        }
    }
}

fn ann_disabled_from_raw(raw: &str) -> bool {
    matches!(
        raw.trim().to_ascii_lowercase().as_str(),
        "1" | "true" | "yes" | "on"
    )
}

fn ann_disabled_from_env() -> bool {
    std::env::var(DISABLE_ANN_ENV)
        .ok()
        .map(|raw| ann_disabled_from_raw(&raw))
        .unwrap_or(false)
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
    pub embedding_setup: EmbeddingSetup,
    pub embedding: Vec<f32>,
    pub summary_embedding_setup: Option<EmbeddingSetup>,
    pub summary_embedding: Vec<f32>,
    pub call_targets: Vec<String>,
    pub dependency_targets: Vec<String>,
    pub churn_count: usize,
}

impl SymbolCloneCandidateInput {
    fn has_summary_embedding(&self) -> bool {
        self.summary_embedding_setup.is_some() && !self.summary_embedding.is_empty()
    }
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
    representation_kind: String,
    setup_fingerprint: String,
}

#[derive(Debug)]
struct GroupAnnIndex {
    global_indices: Vec<usize>,
    local_by_global: HashMap<usize, usize>,
    index: HnswLikeIndex,
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
    let options = options.apply_env_overrides();

    let mut group_indices = HashMap::<CandidateGroupKey, Vec<usize>>::new();
    let mut summary_group_indices = HashMap::<CandidateGroupKey, Vec<usize>>::new();
    for (idx, candidate) in candidates.iter().enumerate() {
        group_indices
            .entry(candidate_group_key(candidate))
            .or_default()
            .push(idx);
        if let Some(summary_group_key) = summary_candidate_group_key(candidate) {
            summary_group_indices
                .entry(summary_group_key)
                .or_default()
                .push(idx);
        }
    }

    let group_ann_indexes = if options.ann_enabled {
        build_group_ann_indexes(candidates, &group_indices, |candidate| {
            Some(candidate.embedding.as_slice())
        })
    } else {
        HashMap::new()
    };
    let summary_group_ann_indexes = if options.ann_enabled {
        build_group_ann_indexes(candidates, &summary_group_indices, |candidate| {
            if candidate.has_summary_embedding() {
                Some(candidate.summary_embedding.as_slice())
            } else {
                None
            }
        })
    } else {
        HashMap::new()
    };
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

        if options.ann_enabled {
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
            if let Some(summary_group_key) = summary_candidate_group_key(source)
                && let Some(group_ann_index) = summary_group_ann_indexes.get(&summary_group_key)
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
        } else if let Some(group_member_indices) = group_indices.get(&group_key) {
            for global_idx in group_member_indices {
                if *global_idx != source_idx {
                    target_indices.insert(*global_idx);
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

fn build_group_ann_indexes<F>(
    candidates: &[&SymbolCloneCandidateInput],
    group_indices: &HashMap<CandidateGroupKey, Vec<usize>>,
    embedding_of: F,
) -> HashMap<CandidateGroupKey, GroupAnnIndex>
where
    F: Fn(&SymbolCloneCandidateInput) -> Option<&[f32]>,
{
    let mut out = HashMap::with_capacity(group_indices.len());
    for (group_key, global_indices) in group_indices {
        let mut indexed_global_indices = Vec::with_capacity(global_indices.len());
        let mut vectors = Vec::with_capacity(global_indices.len());
        for global_idx in global_indices {
            let Some(candidate) = candidates.get(*global_idx).copied() else {
                continue;
            };
            let Some(embedding) = embedding_of(candidate) else {
                continue;
            };
            if embedding.is_empty() {
                continue;
            }
            indexed_global_indices.push(*global_idx);
            vectors.push(embedding.to_vec());
        }
        if indexed_global_indices.len() < 2 {
            continue;
        }

        let index = HnswLikeIndex::build(&vectors);
        let local_by_global = indexed_global_indices
            .iter()
            .enumerate()
            .map(|(local, global)| (*global, local))
            .collect::<HashMap<_, _>>();
        out.insert(
            group_key.clone(),
            GroupAnnIndex {
                global_indices: indexed_global_indices,
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
    candidate_group_key_for_setup(candidate, "code", &candidate.embedding_setup)
}

fn summary_candidate_group_key(candidate: &SymbolCloneCandidateInput) -> Option<CandidateGroupKey> {
    let setup = candidate.summary_embedding_setup.as_ref()?;
    if !candidate.has_summary_embedding() {
        return None;
    }
    Some(candidate_group_key_for_setup(candidate, "summary", setup))
}

fn candidate_group_key_for_setup(
    candidate: &SymbolCloneCandidateInput,
    representation_kind: &str,
    setup: &EmbeddingSetup,
) -> CandidateGroupKey {
    CandidateGroupKey {
        repo_id: candidate.repo_id.clone(),
        effective_kind: candidate.canonical_kind.trim().to_ascii_lowercase(),
        representation_kind: representation_kind.to_string(),
        setup_fingerprint: setup.setup_fingerprint.trim().to_ascii_lowercase(),
    }
}

fn build_symbol_clone_edge(
    source: &SymbolCloneCandidateInput,
    target: &SymbolCloneCandidateInput,
) -> Option<SymbolCloneEdgeRow> {
    if !same_clone_kind(&source.canonical_kind, &target.canonical_kind) {
        return None;
    }

    let code_embedding_similarity = semantic_similarity(source, target);
    let summary_embedding_similarity = summary_embedding_similarity(source, target);
    let semantic_score =
        combined_semantic_similarity(code_embedding_similarity, summary_embedding_similarity);
    let lexical = lexical_signals(source, target);
    let structural = structural_signals(source, target, lexical.name_match);
    let derived = derived_clone_signals(
        source,
        target,
        code_embedding_similarity,
        summary_embedding_similarity,
        &lexical,
        &structural,
    );
    let base_score = (CLONE_SCORE_WEIGHT_SEMANTIC * semantic_score)
        + (CLONE_SCORE_WEIGHT_LEXICAL * lexical.score)
        + (CLONE_SCORE_WEIGHT_STRUCTURAL * structural.score);
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
    } else if likely_similar_implementation(score, semantic_score, &derived) {
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
        relation_kind: relation_kind.as_str(),
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
use self::classification::*;
use self::core::*;
use self::explanation::*;
use self::utils::*;

#[cfg(test)]
mod tests;
