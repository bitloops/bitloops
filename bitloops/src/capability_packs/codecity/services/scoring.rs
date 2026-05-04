use crate::capability_packs::codecity::services::config::CodeCityHealthConfig;
use crate::capability_packs::codecity::types::{HealthSignal, HealthStatus};

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct NormalizedHealthSignals {
    pub churn: Option<f64>,
    pub complexity: Option<f64>,
    pub bugs: Option<f64>,
    pub coverage_risk: Option<f64>,
    pub author_concentration: Option<f64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct HealthScore {
    pub health_risk: Option<f64>,
    pub status: HealthStatus,
    pub confidence: f64,
    pub missing_signals: Vec<HealthSignal>,
}

pub fn score_health(
    signals: NormalizedHealthSignals,
    config: &CodeCityHealthConfig,
) -> HealthScore {
    let weighted = [
        (HealthSignal::Churn, config.churn_weight, signals.churn),
        (
            HealthSignal::Complexity,
            config.complexity_weight,
            signals.complexity,
        ),
        (HealthSignal::Bugs, config.bug_weight, signals.bugs),
        (
            HealthSignal::Coverage,
            config.coverage_weight,
            signals.coverage_risk,
        ),
        (
            HealthSignal::AuthorConcentration,
            config.author_concentration_weight,
            signals.author_concentration,
        ),
    ];

    let total_positive_weight = weighted
        .iter()
        .filter(|(_, weight, _)| *weight > 0.0)
        .map(|(_, weight, _)| *weight)
        .sum::<f64>();
    if total_positive_weight <= 0.0 {
        return HealthScore {
            health_risk: None,
            status: HealthStatus::InsufficientData,
            confidence: 0.0,
            missing_signals: Vec::new(),
        };
    }

    let mut available = Vec::new();
    let mut missing_signals = Vec::new();
    for (signal, weight, value) in weighted {
        if weight <= 0.0 {
            continue;
        }
        if let Some(value) = value.filter(|value| value.is_finite()) {
            available.push((signal, weight, value.clamp(0.0, 1.0)));
        } else {
            missing_signals.push(signal);
        }
    }

    let only_complexity_available = available.len() == 1
        && available
            .first()
            .is_some_and(|(signal, _, _)| *signal == HealthSignal::Complexity);
    if available.is_empty()
        || (config.insufficient_data_requires_non_complexity_signal && only_complexity_available)
    {
        missing_signals.sort();
        return HealthScore {
            health_risk: None,
            status: HealthStatus::InsufficientData,
            confidence: 0.0,
            missing_signals,
        };
    }

    let available_weight = available.iter().map(|(_, weight, _)| *weight).sum::<f64>();
    let risk = available
        .iter()
        .map(|(_, weight, value)| weight * value)
        .sum::<f64>()
        / available_weight;
    let confidence = (available_weight / total_positive_weight).clamp(0.0, 1.0);
    missing_signals.sort();

    HealthScore {
        health_risk: Some(risk.clamp(0.0, 1.0)),
        status: if missing_signals.is_empty() {
            HealthStatus::Ok
        } else {
            HealthStatus::Partial
        },
        confidence,
        missing_signals,
    }
}

#[cfg(test)]
mod tests {
    use super::{NormalizedHealthSignals, score_health};
    use crate::capability_packs::codecity::services::config::CodeCityConfig;
    use crate::capability_packs::codecity::types::{HealthSignal, HealthStatus};

    fn config() -> crate::capability_packs::codecity::services::config::CodeCityHealthConfig {
        CodeCityConfig::default().health
    }

    #[test]
    fn all_signals_available_scores_ok() {
        let score = score_health(
            NormalizedHealthSignals {
                churn: Some(1.0),
                complexity: Some(0.5),
                bugs: Some(0.0),
                coverage_risk: Some(0.2),
                author_concentration: Some(0.4),
            },
            &config(),
        );

        assert_eq!(score.status, HealthStatus::Ok);
        assert_eq!(score.confidence, 1.0);
        assert!(score.health_risk.expect("risk") > 0.0);
        assert!(score.missing_signals.is_empty());
    }

    #[test]
    fn missing_coverage_scores_partial_and_rebalances() {
        let score = score_health(
            NormalizedHealthSignals {
                churn: Some(1.0),
                complexity: Some(0.0),
                bugs: Some(0.0),
                coverage_risk: None,
                author_concentration: Some(0.0),
            },
            &config(),
        );

        assert_eq!(score.status, HealthStatus::Partial);
        assert_eq!(score.missing_signals, vec![HealthSignal::Coverage]);
        assert!((score.confidence - 0.85).abs() < 1e-9);
        assert!((score.health_risk.expect("risk") - (0.30 / 0.85)).abs() < 1e-9);
    }

    #[test]
    fn complexity_alone_is_insufficient_by_default() {
        let score = score_health(
            NormalizedHealthSignals {
                complexity: Some(0.8),
                ..NormalizedHealthSignals::default()
            },
            &config(),
        );

        assert_eq!(score.status, HealthStatus::InsufficientData);
        assert_eq!(score.health_risk, None);
        assert_eq!(score.confidence, 0.0);
    }

    #[test]
    fn zero_weight_missing_signal_does_not_lower_confidence() {
        let mut config = config();
        config.coverage_weight = 0.0;
        let score = score_health(
            NormalizedHealthSignals {
                churn: Some(0.5),
                complexity: Some(0.5),
                bugs: Some(0.5),
                coverage_risk: None,
                author_concentration: Some(0.5),
            },
            &config,
        );

        assert_eq!(score.status, HealthStatus::Ok);
        assert_eq!(score.confidence, 1.0);
        assert!(score.missing_signals.is_empty());
    }
}
