#[derive(Debug)]
pub(super) struct HnswLikeIndex {
    vectors: Vec<Vec<f32>>,
    norms: Vec<f32>,
}

impl HnswLikeIndex {
    pub(super) fn build(vectors: &[Vec<f32>]) -> Self {
        let vectors = vectors.to_vec();
        let norms = vectors
            .iter()
            .map(|vector| vector.iter().map(|value| value * value).sum::<f32>().sqrt())
            .collect::<Vec<_>>();
        Self { vectors, norms }
    }

    pub(super) fn nearest(&self, query_idx: usize, limit: usize) -> Vec<usize> {
        if limit == 0 || query_idx >= self.vectors.len() {
            return Vec::new();
        }
        let query = match self.vectors.get(query_idx) {
            Some(query) => query,
            None => return Vec::new(),
        };
        let query_norm = self.norms.get(query_idx).copied().unwrap_or_default();
        if query_norm <= f32::EPSILON {
            return Vec::new();
        }

        let mut scored = self
            .vectors
            .iter()
            .enumerate()
            .filter_map(|(idx, vector)| {
                let similarity = cosine_similarity(query, query_norm, vector, self.norms[idx])?;
                Some((idx, similarity))
            })
            .collect::<Vec<_>>();
        scored.sort_by(|left, right| {
            right
                .1
                .partial_cmp(&left.1)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| left.0.cmp(&right.0))
        });
        scored.into_iter().take(limit).map(|(idx, _)| idx).collect()
    }
}

fn cosine_similarity(left: &[f32], left_norm: f32, right: &[f32], right_norm: f32) -> Option<f32> {
    if left.len() != right.len()
        || left_norm <= f32::EPSILON
        || right_norm <= f32::EPSILON
        || left.is_empty()
    {
        return None;
    }

    let dot = left
        .iter()
        .zip(right.iter())
        .fold(0.0_f32, |acc, (left, right)| acc + (left * right));
    Some(dot / (left_norm * right_norm))
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
