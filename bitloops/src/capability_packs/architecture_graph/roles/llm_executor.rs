use std::path::Path;

use anyhow::{Context, Result};
use serde_json::{Map, Value, json};

use crate::capability_packs::architecture_graph::types::{
    ARCHITECTURE_GRAPH_CAPABILITY_ID, ARCHITECTURE_GRAPH_ROLE_ADJUDICATION_SLOT,
};
use crate::host::inference::{InferenceGateway, StructuredGenerationRequest};

use super::evidence_packet_builder::RoleEvidencePacket;

pub fn execute_llm_adjudication(
    inference: &dyn InferenceGateway,
    repo_root: &Path,
    packet: &RoleEvidencePacket,
) -> Result<(Value, String)> {
    let service = inference
        .structured_generation(ARCHITECTURE_GRAPH_ROLE_ADJUDICATION_SLOT)
        .with_context(|| {
            format!(
                "resolving architecture role adjudication slot `{}`",
                ARCHITECTURE_GRAPH_ROLE_ADJUDICATION_SLOT
            )
        })?;

    let mut metadata = Map::new();
    metadata.insert(
        "capability_id".to_string(),
        Value::String(ARCHITECTURE_GRAPH_CAPABILITY_ID.to_string()),
    );
    metadata.insert(
        "slot_name".to_string(),
        Value::String(ARCHITECTURE_GRAPH_ROLE_ADJUDICATION_SLOT.to_string()),
    );
    metadata.insert(
        "repo_id".to_string(),
        Value::String(packet.request.repo_id.clone()),
    );
    metadata.insert(
        "generation".to_string(),
        Value::Number(packet.request.generation.into()),
    );

    let response = service.generate(StructuredGenerationRequest {
        system_prompt: adjudication_system_prompt().to_string(),
        user_prompt: adjudication_user_prompt(packet),
        json_schema: adjudication_response_schema(),
        workspace_path: Some(repo_root.display().to_string()),
        metadata,
    })?;

    Ok((response, service.descriptor()))
}

fn adjudication_system_prompt() -> &'static str {
    "You adjudicate architectural roles for one artefact at a time. Use only the provided evidence packet. Return strict JSON that matches the schema. Use only role IDs from packet.candidate_roles[].role_id. Prefer needs_review over speculation."
}

fn adjudication_user_prompt(packet: &RoleEvidencePacket) -> String {
    json!({
        "task": "Classify architectural role assignment for one artefact using compact evidence.",
        "rules": [
            "Use only role IDs from packet.candidate_roles[].role_id.",
            "Use candidate role canonical_key, family, display_name, and description to decide between roles.",
            "Return assigned only when the evidence supports one or more supplied role IDs.",
            "If evidence is insufficient or conflicting, return outcome needs_review or unknown.",
            "Confidence must be a number between 0 and 1.",
            "Return concise reasoning_summary without hidden chain-of-thought."
        ],
        "packet": packet,
    })
    .to_string()
}

fn adjudication_response_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "outcome": {
                "type": "string",
                "enum": ["assigned", "unknown", "needs_review"]
            },
            "assignments": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "role_id": { "type": "string", "minLength": 1 },
                        "primary": { "type": "boolean" },
                        "confidence": { "type": "number", "minimum": 0, "maximum": 1 },
                        "evidence": {}
                    },
                    "required": ["role_id", "primary", "confidence", "evidence"],
                    "additionalProperties": false
                }
            },
            "confidence": { "type": "number", "minimum": 0, "maximum": 1 },
            "evidence": {},
            "reasoning_summary": { "type": "string", "minLength": 1 },
            "rule_suggestions": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "title": { "type": "string", "minLength": 1 },
                        "summary": { "type": "string", "minLength": 1 },
                        "rationale": { "type": ["string", "null"] }
                    },
                    "required": ["title", "summary", "rationale"],
                    "additionalProperties": false
                }
            }
        },
        "required": ["outcome", "assignments", "confidence", "reasoning_summary", "evidence", "rule_suggestions"],
        "additionalProperties": false
    })
}

#[cfg(test)]
mod tests {
    use serde_json::Value;

    use super::*;

    #[test]
    fn adjudication_response_schema_requires_all_declared_properties() {
        let schema = adjudication_response_schema();

        assert_schema_objects_require_every_property("$", &schema);
    }

    #[test]
    fn adjudication_response_schema_keeps_evidence_generic() {
        let schema = adjudication_response_schema();

        assert_eq!(
            schema.pointer("/properties/evidence"),
            Some(&Value::Object(Default::default()))
        );
        assert_eq!(
            schema.pointer("/properties/assignments/items/properties/evidence"),
            Some(&Value::Object(Default::default()))
        );
    }

    fn assert_schema_objects_require_every_property(path: &str, value: &Value) {
        match value {
            Value::Object(map) => {
                if let Some(properties) = map.get("properties").and_then(Value::as_object) {
                    let required = map
                        .get("required")
                        .and_then(Value::as_array)
                        .unwrap_or_else(|| {
                            panic!("{path}: object with properties must declare required fields")
                        })
                        .iter()
                        .map(|value| value.as_str().expect("required field must be a string"))
                        .collect::<std::collections::BTreeSet<_>>();
                    let property_keys = properties
                        .keys()
                        .map(String::as_str)
                        .collect::<std::collections::BTreeSet<_>>();
                    assert_eq!(
                        required, property_keys,
                        "{path}: required fields must match declared properties"
                    );
                }
                for (key, child) in map {
                    assert_schema_objects_require_every_property(&format!("{path}/{key}"), child);
                }
            }
            Value::Array(items) => {
                for (index, item) in items.iter().enumerate() {
                    assert_schema_objects_require_every_property(&format!("{path}/{index}"), item);
                }
            }
            _ => {}
        }
    }
}
