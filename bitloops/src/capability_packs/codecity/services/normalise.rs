pub fn min_max_normalise(values: &[f64]) -> Vec<f64> {
    if values.is_empty() {
        return Vec::new();
    }

    let min = values.iter().copied().fold(f64::INFINITY, f64::min);
    let max = values.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    if !min.is_finite() || !max.is_finite() || (max - min).abs() < f64::EPSILON {
        return vec![0.0; values.len()];
    }

    values
        .iter()
        .map(|value| (value - min) / (max - min))
        .collect()
}

pub fn finite(value: f64) -> Option<f64> {
    value.is_finite().then_some(value)
}

#[cfg(test)]
mod tests {
    use super::min_max_normalise;

    #[test]
    fn min_max_normalise_handles_empty_single_and_equal_inputs() {
        assert!(min_max_normalise(&[]).is_empty());
        assert_eq!(min_max_normalise(&[7.0]), vec![0.0]);
        assert_eq!(min_max_normalise(&[2.0, 2.0]), vec![0.0, 0.0]);
    }

    #[test]
    fn min_max_normalise_scales_values() {
        assert_eq!(min_max_normalise(&[-1.0, 1.0, 3.0]), vec![0.0, 0.5, 1.0]);
    }

    #[test]
    fn min_max_normalise_sanitises_non_finite_range() {
        assert_eq!(min_max_normalise(&[1.0, f64::INFINITY]), vec![0.0, 0.0]);
    }
}
