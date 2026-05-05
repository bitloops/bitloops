use super::contracts::AdjudicationReason;

pub const HIGH_CONFIDENCE_THRESHOLD: f64 = 0.85;

#[derive(Debug, Clone, PartialEq)]
pub struct DeterministicRoleOutcomeInput {
    pub classification_known: bool,
    pub best_confidence: Option<f64>,
    pub has_conflict: bool,
    pub high_impact: bool,
    pub novel_pattern: bool,
    pub manual_review_requested: bool,
}

impl DeterministicRoleOutcomeInput {
    pub fn high_confidence_known() -> Self {
        Self {
            classification_known: true,
            best_confidence: Some(0.95),
            has_conflict: false,
            high_impact: false,
            novel_pattern: false,
            manual_review_requested: false,
        }
    }
}

pub fn select_adjudication_reason(
    input: &DeterministicRoleOutcomeInput,
) -> Option<AdjudicationReason> {
    if input.manual_review_requested {
        return Some(AdjudicationReason::ManualReview);
    }
    if input.high_impact {
        return Some(AdjudicationReason::HighImpact);
    }
    if !input.classification_known {
        return Some(AdjudicationReason::Unknown);
    }
    if input.has_conflict {
        return Some(AdjudicationReason::Conflict);
    }
    if input.novel_pattern {
        return Some(AdjudicationReason::NovelPattern);
    }
    let confidence = input.best_confidence.unwrap_or(0.0);
    if confidence < HIGH_CONFIDENCE_THRESHOLD {
        return Some(AdjudicationReason::LowConfidence);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn high_confidence_non_conflicting_result_skips_adjudication() {
        let input = DeterministicRoleOutcomeInput::high_confidence_known();
        assert_eq!(select_adjudication_reason(&input), None);
    }

    #[test]
    fn low_confidence_result_is_enqueued() {
        let input = DeterministicRoleOutcomeInput {
            best_confidence: Some(0.4),
            ..DeterministicRoleOutcomeInput::high_confidence_known()
        };

        assert_eq!(
            select_adjudication_reason(&input),
            Some(AdjudicationReason::LowConfidence)
        );
    }

    #[test]
    fn unknown_and_conflict_cases_are_enqueued() {
        let unknown = DeterministicRoleOutcomeInput {
            classification_known: false,
            ..DeterministicRoleOutcomeInput::high_confidence_known()
        };
        let conflict = DeterministicRoleOutcomeInput {
            has_conflict: true,
            ..DeterministicRoleOutcomeInput::high_confidence_known()
        };

        assert_eq!(
            select_adjudication_reason(&unknown),
            Some(AdjudicationReason::Unknown)
        );
        assert_eq!(
            select_adjudication_reason(&conflict),
            Some(AdjudicationReason::Conflict)
        );
    }
}
