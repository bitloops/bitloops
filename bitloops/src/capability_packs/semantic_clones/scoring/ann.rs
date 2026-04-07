use std::fmt;

use hnsw_rs::prelude::{DistCosine, Hnsw};

const HNSW_MAX_CONNECTIONS: usize = 24;
const HNSW_MAX_LAYERS: usize = 16;
const HNSW_EF_CONSTRUCTION: usize = 200;
const HNSW_EF_SEARCH: usize = 64;

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
        let search_width = limit.max(HNSW_EF_SEARCH);
        self.index
            .search(query.as_slice(), limit, search_width)
            .into_iter()
            .map(|neighbor| neighbor.d_id)
            .collect()
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
}
