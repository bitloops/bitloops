use std::collections::BTreeSet;

use anyhow::Result;
use serde_json::Value;

use super::contracts::{
    AdjudicationOutcome, RoleAdjudicationResult, RoleAdjudicationValidationError,
};

pub fn validate_adjudication_result(
    raw: Value,
    active_role_ids: &BTreeSet<String>,
) -> Result<RoleAdjudicationResult, RoleAdjudicationValidationError> {
    let parsed: RoleAdjudicationResult = serde_json::from_value(raw)
        .map_err(|err| RoleAdjudicationValidationError::Schema(err.to_string()))?;

    validate_confidence(parsed.confidence, "result.confidence")?;
    if parsed.reasoning_summary.trim().is_empty() {
        return Err(RoleAdjudicationValidationError::Schema(
            "reasoning_summary must not be empty".to_string(),
        ));
    }

    for assignment in &parsed.assignments {
        validate_confidence(
            assignment.confidence,
            &format!("assignment `{}` confidence", assignment.role_id),
        )?;
        if !active_role_ids.contains(&assignment.role_id) {
            return Err(RoleAdjudicationValidationError::UnknownRoleId(
                assignment.role_id.clone(),
            ));
        }
    }

    match parsed.outcome {
        AdjudicationOutcome::Assigned => {
            if parsed.assignments.is_empty() {
                return Err(RoleAdjudicationValidationError::InvalidOutcome(
                    "outcome=assigned requires at least one assignment".to_string(),
                ));
            }
        }
        AdjudicationOutcome::Unknown | AdjudicationOutcome::NeedsReview => {
            if !parsed.assignments.is_empty() {
                return Err(RoleAdjudicationValidationError::InvalidOutcome(
                    "unknown/needs_review outcomes must not include assignments".to_string(),
                ));
            }
        }
    }

    if parsed
        .assignments
        .iter()
        .filter(|assignment| assignment.primary)
        .count()
        > 1
    {
        return Err(RoleAdjudicationValidationError::InvalidOutcome(
            "at most one assignment can be marked primary".to_string(),
        ));
    }

    Ok(parsed)
}

fn validate_confidence(value: f64, field: &str) -> Result<(), RoleAdjudicationValidationError> {
    if !(0.0..=1.0).contains(&value) {
        return Err(RoleAdjudicationValidationError::InvalidConfidence(format!(
            "{field} must be between 0 and 1"
        )));
    }
    if value.is_nan() {
        return Err(RoleAdjudicationValidationError::InvalidConfidence(format!(
            "{field} must not be NaN"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use serde_json::json;

    use super::*;

    fn active_roles() -> BTreeSet<String> {
        BTreeSet::from(["entrypoint".to_string(), "storage_adapter".to_string()])
    }

    #[test]
    fn rejects_unknown_role_id() {
        let err = validate_adjudication_result(
            json!({
                "outcome": "assigned",
                "assignments": [{
                    "role_id": "missing-role",
                    "confidence": 0.8,
                    "primary": true,
                    "evidence": []
                }],
                "confidence": 0.82,
                "evidence": [],
                "reasoning_summary": "strong signal",
                "rule_suggestions": []
            }),
            &active_roles(),
        )
        .expect_err("unknown role id must be rejected");

        assert!(matches!(
            err,
            RoleAdjudicationValidationError::UnknownRoleId(role) if role == "missing-role"
        ));
    }

    #[test]
    fn rejects_unknown_fields_via_schema() {
        let err = validate_adjudication_result(
            json!({
                "outcome": "unknown",
                "assignments": [],
                "confidence": 0.4,
                "evidence": [],
                "reasoning_summary": "not enough evidence",
                "rule_suggestions": [],
                "extra": true
            }),
            &active_roles(),
        )
        .expect_err("extra fields must fail deny_unknown_fields");

        assert!(matches!(err, RoleAdjudicationValidationError::Schema(_)));
    }

    #[test]
    fn accepts_valid_assignment_response() {
        let result = validate_adjudication_result(
            json!({
                "outcome": "assigned",
                "assignments": [{
                    "role_id": "entrypoint",
                    "primary": true,
                    "confidence": 0.93,
                    "evidence": ["main.rs"]
                }],
                "confidence": 0.91,
                "evidence": ["rule_match:entrypoint"],
                "reasoning_summary": "evidence packet strongly supports entrypoint.",
                "rule_suggestions": []
            }),
            &active_roles(),
        )
        .expect("valid response should pass");

        assert_eq!(result.assignments.len(), 1);
    }
}
