use serde_json::{Value, json};

use super::supported_facts;

fn strict_empty_object_schema() -> Value {
    json!({
        "type": "object",
        "properties": {},
        "required": [],
        "additionalProperties": false
    })
}

fn string_array_schema() -> Value {
    json!({
        "type": "array",
        "items": { "type": "string" }
    })
}

fn evidence_example_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["path", "symbol_fqn", "canonical_kind", "why"],
        "properties": {
            "path": { "type": "string" },
            "symbol_fqn": { "type": ["string", "null"] },
            "canonical_kind": { "type": ["string", "null"] },
            "why": { "type": "string" }
        }
    })
}

fn seeded_role_evidence_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": [
            "inspected_paths",
            "supporting_paths",
            "supporting_symbols",
            "db_sections_used",
            "reasoning_summary",
            "confidence_reason",
            "uncertainty"
        ],
        "properties": {
            "inspected_paths": string_array_schema(),
            "supporting_paths": string_array_schema(),
            "supporting_symbols": string_array_schema(),
            "db_sections_used": string_array_schema(),
            "reasoning_summary": { "type": "string" },
            "confidence_reason": { "type": "string" },
            "uncertainty": { "type": "string" }
        }
    })
}

fn seeded_rule_evidence_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": [
            "inspected_paths",
            "positive_examples",
            "negative_examples",
            "db_sections_used",
            "reasoning_summary",
            "confidence_reason",
            "uncertainty"
        ],
        "properties": {
            "inspected_paths": string_array_schema(),
            "positive_examples": {
                "type": "array",
                "items": evidence_example_schema()
            },
            "negative_examples": {
                "type": "array",
                "items": evidence_example_schema()
            },
            "db_sections_used": string_array_schema(),
            "reasoning_summary": { "type": "string" },
            "confidence_reason": { "type": "string" },
            "uncertainty": { "type": "string" }
        }
    })
}

fn role_condition_schema() -> Value {
    supported_facts::condition_schema()
}

fn seeded_role_schema() -> Value {
    let strict_object = strict_empty_object_schema();
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": [
            "canonical_key",
            "display_name",
            "description",
            "family",
            "provenance",
            "evidence"
        ],
        "properties": {
            "canonical_key": { "type": "string", "minLength": 1 },
            "display_name": { "type": "string", "minLength": 1 },
            "description": { "type": "string" },
            "family": { "type": ["string", "null"] },
            "provenance": strict_object.clone(),
            "evidence": seeded_role_evidence_schema()
        }
    })
}

pub(crate) fn seeded_rule_candidate_schema() -> Value {
    let strict_object = strict_empty_object_schema();
    let condition_schema = role_condition_schema();
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": [
            "target_role_key",
            "candidate_selector",
            "positive_conditions",
            "negative_conditions",
            "score",
            "evidence",
            "metadata"
        ],
        "properties": {
            "target_role_key": { "type": "string", "minLength": 1 },
            "candidate_selector": {
                "type": "object",
                "additionalProperties": false,
                "required": [
                    "target_kinds",
                    "path_prefixes",
                    "path_suffixes",
                    "path_contains",
                    "languages",
                    "canonical_kinds",
                    "symbol_fqn_contains",
                    "required_facts",
                    "required_fact_any_groups"
                ],
                "properties": {
                    "target_kinds": {
                        "type": "array",
                        "items": { "type": "string", "enum": ["file", "artefact", "symbol"] }
                    },
                    "path_prefixes": { "type": "array", "items": { "type": "string" } },
                    "path_suffixes": { "type": "array", "items": { "type": "string" } },
                    "path_contains": { "type": "array", "items": { "type": "string" } },
                    "languages": { "type": "array", "items": { "type": "string" } },
                    "canonical_kinds": { "type": "array", "items": { "type": "string" } },
                    "symbol_fqn_contains": { "type": "array", "items": { "type": "string" } },
                    "required_facts": {
                        "type": "array",
                        "items": condition_schema.clone()
                    },
                    "required_fact_any_groups": {
                        "type": "array",
                        "items": {
                            "type": "array",
                            "items": condition_schema.clone()
                        }
                    }
                }
            },
            "positive_conditions": {
                "type": "array",
                "items": condition_schema.clone()
            },
            "negative_conditions": {
                "type": "array",
                "items": condition_schema
            },
            "score": {
                "type": "object",
                "additionalProperties": false,
                "required": ["base_confidence", "priority_hint", "min_positive_ratio"],
                "properties": {
                    "base_confidence": { "type": ["number", "null"], "minimum": 0, "maximum": 1 },
                    "priority_hint": { "type": ["integer", "null"] },
                    "min_positive_ratio": { "type": ["number", "null"], "minimum": 0, "maximum": 1 }
                }
            },
            "evidence": seeded_rule_evidence_schema(),
            "metadata": strict_object
        }
    })
}

pub fn architecture_roles_seed_roles_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["roles"],
        "properties": {
            "roles": {
                "type": "array",
                "minItems": 1,
                "items": seeded_role_schema()
            }
        }
    })
}

pub fn architecture_roles_seed_rule_candidates_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["rule_candidates"],
        "properties": {
            "rule_candidates": {
                "type": "array",
                "items": seeded_rule_candidate_schema()
            }
        }
    })
}

pub fn architecture_roles_seed_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["roles", "rule_candidates"],
        "properties": {
            "roles": {
                "type": "array",
                "minItems": 1,
                "items": seeded_role_schema()
            },
            "rule_candidates": {
                "type": "array",
                "items": seeded_rule_candidate_schema()
            }
        }
    })
}
