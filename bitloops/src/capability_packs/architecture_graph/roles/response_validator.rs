use std::collections::BTreeSet;

use anyhow::Result;
use serde_json::Value;

use super::contracts::{
    AdjudicationOutcome, RoleAdjudicationResult, RoleAdjudicationValidationError,
};
use super::taxonomy::{role_rule_condition_contract, validate_supported_fact_condition};

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

    for suggestion in &parsed.rule_suggestions {
        if !active_role_ids.contains(&suggestion.target_role_id) {
            return Err(RoleAdjudicationValidationError::UnknownRoleId(
                suggestion.target_role_id.clone(),
            ));
        }
        if suggestion.rule_candidate.target_role_key != suggestion.target_role_id {
            return Err(RoleAdjudicationValidationError::InvalidOutcome(
                "rule suggestion target_role_key must match target_role_id".to_string(),
            ));
        }
        for condition in suggestion
            .rule_candidate
            .positive_conditions
            .iter()
            .chain(suggestion.rule_candidate.negative_conditions.iter())
        {
            let fact_condition = role_rule_condition_contract(condition)
                .map_err(|err| RoleAdjudicationValidationError::Schema(err.to_string()))?;
            validate_supported_fact_condition("rule_suggestion.condition", &fact_condition)
                .map_err(|err| RoleAdjudicationValidationError::Schema(err.to_string()))?;
        }
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

    #[test]
    fn adjudication_response_accepts_draftable_rule_suggestion() {
        let result = validate_adjudication_result(
            json!({
                "outcome": "unknown",
                "assignments": [],
                "confidence": 0.6,
                "evidence": [],
                "reasoning_summary": "similar unresolved storage targets share a suffix.",
                "rule_suggestions": [{
                    "target_role_id": "storage_adapter",
                    "title": "Repository suffix storage adapter",
                    "summary": "Repository suffixes under storage paths are reusable.",
                    "rule_candidate": {
                        "target_role_key": "storage_adapter",
                        "candidate_selector": {
                            "target_kinds": ["artefact"],
                            "path_prefixes": ["src/storage"],
                            "path_suffixes": [".rs"],
                            "path_contains": [],
                            "languages": [],
                            "canonical_kinds": [],
                            "symbol_fqn_contains": [],
                            "required_facts": [],
                            "required_fact_any_groups": []
                        },
                        "positive_conditions": [
                            { "kind": "symbol", "key": "name_suffix", "op": "eq", "value": "Repository", "score": 1.0 }
                        ],
                        "negative_conditions": [],
                        "score": {
                            "base_confidence": 0.82,
                            "priority_hint": 100,
                            "min_positive_ratio": 1.0
                        },
                        "evidence": {},
                        "metadata": {}
                    },
                    "source_cluster_key": null
                }]
            }),
            &active_roles(),
        )
        .expect("draftable suggestion should pass");

        assert_eq!(result.rule_suggestions.len(), 1);
    }

    #[test]
    fn adjudication_response_rejects_suggestion_for_unknown_role() {
        let err = validate_adjudication_result(
            json!({
                "outcome": "unknown",
                "assignments": [],
                "confidence": 0.6,
                "evidence": [],
                "reasoning_summary": "suggestion references missing role.",
                "rule_suggestions": [{
                    "target_role_id": "missing",
                    "title": "Missing",
                    "summary": "Missing",
                    "rule_candidate": {
                        "target_role_key": "missing",
                        "candidate_selector": {
                            "target_kinds": ["file"],
                            "path_prefixes": [],
                            "path_suffixes": [],
                            "path_contains": [],
                            "languages": [],
                            "canonical_kinds": [],
                            "symbol_fqn_contains": [],
                            "required_facts": [],
                            "required_fact_any_groups": []
                        },
                        "positive_conditions": [
                            { "kind": "path", "key": "segment", "op": "eq", "value": "storage", "score": 1.0 }
                        ],
                        "negative_conditions": [],
                        "score": {
                            "base_confidence": 0.82,
                            "priority_hint": 100,
                            "min_positive_ratio": 1.0
                        },
                        "evidence": {},
                        "metadata": {}
                    },
                    "source_cluster_key": null
                }]
            }),
            &active_roles(),
        )
        .expect_err("unknown suggestion role must fail");

        assert!(
            matches!(err, RoleAdjudicationValidationError::UnknownRoleId(role) if role == "missing")
        );
    }
}
