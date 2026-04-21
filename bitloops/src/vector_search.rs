use std::fmt;

use hnsw_rs::prelude::{DistCosine, Hnsw};

const HNSW_MAX_CONNECTIONS: usize = 24;
const HNSW_MAX_LAYERS: usize = 16;
const HNSW_EF_CONSTRUCTION: usize = 200;
const HNSW_EF_SEARCH: usize = 64;
const EXACT_SEARCH_MAX_VECTORS: usize = 128;
const ANN_OVERSAMPLE_FACTOR: usize = 4;
const ANN_MIN_OVERSAMPLE: usize = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum VectorSearchMode {
    Auto,
    Exact,
    Ann,
}

pub(crate) struct HnswLikeIndex {
    vectors: Vec<Vec<f32>>,
    index: Hnsw<'static, f32, DistCosine>,
}

impl HnswLikeIndex {
    pub(crate) fn build(vectors: &[Vec<f32>]) -> Self {
        let vectors = vectors.to_vec();
        let mut index = Hnsw::<f32, DistCosine>::new(
            HNSW_MAX_CONNECTIONS,
            vectors.len().max(1),
            HNSW_MAX_LAYERS,
            HNSW_EF_CONSTRUCTION,
            DistCosine {},
        );
        for (idx, vector) in vectors.iter().enumerate() {
            index.insert((vector.as_slice(), idx));
        }
        index.set_searching_mode(true);
        Self { vectors, index }
    }

    pub(crate) fn nearest(&self, query_idx: usize, limit: usize) -> Vec<usize> {
        let Some(query) = self.vectors.get(query_idx) else {
            return Vec::new();
        };
        self.nearest_to_vector_internal(query, limit, VectorSearchMode::Auto)
    }

    pub(crate) fn nearest_to_vector_with_mode(
        &self,
        query: &[f32],
        limit: usize,
        mode: VectorSearchMode,
    ) -> Vec<usize> {
        self.nearest_to_vector_internal(query, limit, mode)
    }

    fn nearest_to_vector_internal(
        &self,
        query: &[f32],
        limit: usize,
        mode: VectorSearchMode,
    ) -> Vec<usize> {
        if limit == 0 || self.vectors.is_empty() || query.is_empty() {
            return Vec::new();
        }
        if self
            .vectors
            .first()
            .is_some_and(|vector| vector.len() != query.len())
        {
            return Vec::new();
        }

        match effective_mode(mode, self.vectors.len()) {
            VectorSearchMode::Exact => {
                self.nearest_by_exact_similarity(query, 0..self.vectors.len(), limit)
            }
            VectorSearchMode::Ann => {
                let oversampled_limit = limit
                    .saturating_mul(ANN_OVERSAMPLE_FACTOR)
                    .max(limit)
                    .max(ANN_MIN_OVERSAMPLE)
                    .min(self.vectors.len());
                let search_width = oversampled_limit.max(HNSW_EF_SEARCH);
                let candidates = self
                    .index
                    .search(query, oversampled_limit, search_width)
                    .into_iter()
                    .map(|neighbor| neighbor.d_id);
                self.nearest_by_exact_similarity(query, candidates, limit)
            }
            VectorSearchMode::Auto => unreachable!("auto mode is resolved above"),
        }
    }

    fn nearest_by_exact_similarity<I>(
        &self,
        query: &[f32],
        candidate_indices: I,
        limit: usize,
    ) -> Vec<usize>
    where
        I: IntoIterator<Item = usize>,
    {
        let mut scored = candidate_indices
            .into_iter()
            .filter_map(|candidate_idx| {
                let candidate = self.vectors.get(candidate_idx)?;
                Some((
                    candidate_idx,
                    cosine_similarity(query, candidate.as_slice()),
                ))
            })
            .collect::<Vec<_>>();
        scored.sort_by(|left, right| {
            right
                .1
                .partial_cmp(&left.1)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| left.0.cmp(&right.0))
        });
        scored
            .into_iter()
            .map(|(candidate_idx, _)| candidate_idx)
            .take(limit)
            .collect()
    }
}

pub(crate) fn cosine_similarity(left: &[f32], right: &[f32]) -> f32 {
    if left.is_empty() || left.len() != right.len() {
        return f32::NEG_INFINITY;
    }

    let mut dot = 0.0_f32;
    let mut left_norm = 0.0_f32;
    let mut right_norm = 0.0_f32;
    for (left_value, right_value) in left.iter().zip(right.iter()) {
        dot += left_value * right_value;
        left_norm += left_value * left_value;
        right_norm += right_value * right_value;
    }

    if left_norm <= f32::EPSILON || right_norm <= f32::EPSILON {
        return f32::NEG_INFINITY;
    }

    dot / (left_norm.sqrt() * right_norm.sqrt())
}

pub(crate) fn normalized_cosine_similarity(left: &[f32], right: &[f32]) -> Option<f32> {
    let similarity = cosine_similarity(left, right);
    similarity
        .is_finite()
        .then_some(((similarity + 1.0) / 2.0).clamp(0.0, 1.0))
}

fn effective_mode(mode: VectorSearchMode, vector_count: usize) -> VectorSearchMode {
    match mode {
        VectorSearchMode::Auto if vector_count <= EXACT_SEARCH_MAX_VECTORS => {
            VectorSearchMode::Exact
        }
        VectorSearchMode::Auto => VectorSearchMode::Ann,
        other => other,
    }
}

impl fmt::Debug for HnswLikeIndex {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HnswLikeIndex")
            .field("vector_count", &self.vectors.len())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nearest_returns_query_symbol_first_for_identical_vector() {
        let vectors = vec![vec![1.0, 0.0], vec![0.9, 0.1], vec![0.0, 1.0]];
        let index = HnswLikeIndex::build(&vectors);
        let nearest = index.nearest(0, 2);
        assert_eq!(nearest, vec![0, 1]);
    }

    #[test]
    fn nearest_to_vector_exact_and_ann_return_same_top_hits() {
        let mut vectors = (0..140)
            .map(|idx| vec![idx as f32 / 140.0, 1.0 - (idx as f32 / 140.0), 0.5])
            .collect::<Vec<_>>();
        vectors.push(vec![0.95, 0.05, 0.5]);
        vectors.push(vec![0.96, 0.04, 0.5]);
        vectors.push(vec![0.97, 0.03, 0.5]);
        let query = vec![1.0, 0.0, 0.5];

        let index = HnswLikeIndex::build(&vectors);
        let exact = index.nearest_to_vector_with_mode(&query, 5, VectorSearchMode::Exact);
        let ann = index.nearest_to_vector_with_mode(&query, 5, VectorSearchMode::Ann);

        assert_eq!(ann, exact);
    }

    #[test]
    fn normalized_cosine_similarity_rejects_incompatible_vectors() {
        assert_eq!(normalized_cosine_similarity(&[1.0], &[]), None);
        assert_eq!(normalized_cosine_similarity(&[], &[]), None);
    }
}
