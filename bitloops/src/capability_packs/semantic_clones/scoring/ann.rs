use std::fmt;

use hnsw_rs::prelude::{DistCosine, Hnsw};

const HNSW_MAX_CONNECTIONS: usize = 24;
const HNSW_MAX_LAYERS: usize = 16;
const HNSW_EF_CONSTRUCTION: usize = 200;
const HNSW_EF_SEARCH: usize = 64;
const EXACT_SEARCH_MAX_VECTORS: usize = 128;
const ANN_OVERSAMPLE_FACTOR: usize = 4;
const ANN_MIN_OVERSAMPLE: usize = 8;

pub(super) struct HnswLikeIndex {
    vectors: Vec<Vec<f32>>,
    index: Hnsw<'static, f32, DistCosine>,
}

impl HnswLikeIndex {
    pub(super) fn build(vectors: &[Vec<f32>]) -> Self {
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

    pub(super) fn nearest(&self, query_idx: usize, limit: usize) -> Vec<usize> {
        if limit == 0 {
            return Vec::new();
        }
        let Some(query) = self.vectors.get(query_idx) else {
            return Vec::new();
        };
        if self.vectors.len() <= EXACT_SEARCH_MAX_VECTORS {
            return self.nearest_by_exact_similarity(query_idx, 0..self.vectors.len(), limit);
        }

        let oversampled_limit = limit
            .saturating_mul(ANN_OVERSAMPLE_FACTOR)
            .max(limit)
            .max(ANN_MIN_OVERSAMPLE)
            .min(self.vectors.len());
        let search_width = oversampled_limit.max(HNSW_EF_SEARCH);
        let candidates = self
            .index
            .search(query.as_slice(), oversampled_limit, search_width)
            .into_iter()
            .map(|neighbor| neighbor.d_id);
        self.nearest_by_exact_similarity(query_idx, candidates, limit)
    }

    fn nearest_by_exact_similarity<I>(
        &self,
        query_idx: usize,
        candidate_indices: I,
        limit: usize,
    ) -> Vec<usize>
    where
        I: IntoIterator<Item = usize>,
    {
        let Some(query) = self.vectors.get(query_idx) else {
            return Vec::new();
        };

        let mut scored = candidate_indices
            .into_iter()
            .filter_map(|candidate_idx| {
                let candidate = self.vectors.get(candidate_idx)?;
                Some((
                    candidate_idx,
                    cosine_similarity(query.as_slice(), candidate.as_slice()),
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

fn cosine_similarity(left: &[f32], right: &[f32]) -> f32 {
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
}
