use std::collections::BTreeSet;
use std::sync::OnceLock;

use serde_json::{Value, json};

pub(super) struct SupportedRuleFact {
    pub(super) kind: &'static str,
    pub(super) key: &'static str,
    pub(super) ops: &'static [&'static str],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SupportedFactPredicate {
    pub id: &'static str,
    pub kind: &'static str,
    pub key: &'static str,
    pub op: &'static str,
    pub value_type: &'static str,
    pub description: &'static str,
}

const SUPPORTED_RULE_FACTS: &[SupportedRuleFact] = &[
    SupportedRuleFact {
        kind: "path",
        key: "full",
        ops: &["eq", "contains", "prefix", "suffix"],
    },
    SupportedRuleFact {
        kind: "path",
        key: "segment",
        ops: &["eq"],
    },
    SupportedRuleFact {
        kind: "path",
        key: "extension",
        ops: &["eq"],
    },
    SupportedRuleFact {
        kind: "language",
        key: "resolved",
        ops: &["eq"],
    },
    SupportedRuleFact {
        kind: "file",
        key: "analysis_mode",
        ops: &["eq"],
    },
    SupportedRuleFact {
        kind: "file",
        key: "role",
        ops: &["eq"],
    },
    SupportedRuleFact {
        kind: "artefact",
        key: "canonical_kind",
        ops: &["eq"],
    },
    SupportedRuleFact {
        kind: "artefact",
        key: "language_kind",
        ops: &["eq", "contains"],
    },
    SupportedRuleFact {
        kind: "artefact",
        key: "has_parent_artefact",
        ops: &["eq"],
    },
    SupportedRuleFact {
        kind: "symbol",
        key: "fqn",
        ops: &["contains", "prefix", "suffix", "eq"],
    },
    SupportedRuleFact {
        kind: "symbol",
        key: "name",
        ops: &["eq", "contains", "prefix", "suffix"],
    },
    SupportedRuleFact {
        kind: "symbol",
        key: "name_suffix",
        ops: &["eq"],
    },
    SupportedRuleFact {
        kind: "symbol",
        key: "has_signature",
        ops: &["eq"],
    },
    SupportedRuleFact {
        kind: "signature",
        key: "contains",
        ops: &["eq"],
    },
    SupportedRuleFact {
        kind: "dependency",
        key: "incoming_kind",
        ops: &["eq"],
    },
    SupportedRuleFact {
        kind: "dependency",
        key: "outgoing_kind",
        ops: &["eq"],
    },
    SupportedRuleFact {
        kind: "dependency",
        key: "incoming_count",
        ops: &["gte", "lte", "eq"],
    },
    SupportedRuleFact {
        kind: "dependency",
        key: "outgoing_count",
        ops: &["gte", "lte", "eq"],
    },
];

pub(super) fn catalog() -> Value {
    Value::Array(
        supported_fact_predicates()
            .into_iter()
            .map(|predicate| {
                json!({
                    "id": predicate.id,
                    "value_type": predicate.value_type,
                    "description": predicate.description,
                })
            })
            .collect(),
    )
}

pub(super) fn condition_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["predicate", "value", "score"],
        "properties": {
            "predicate": { "type": "string", "enum": supported_fact_predicate_ids() },
            "value": { "type": "string", "minLength": 1 },
            "score": { "type": "number", "minimum": 0, "maximum": 1 }
        }
    })
}

pub(super) fn ops_for(kind: &str, key: &str) -> Option<&'static [&'static str]> {
    SUPPORTED_RULE_FACTS
        .iter()
        .find(|fact| fact.kind == kind && fact.key == key)
        .map(|fact| fact.ops)
}

pub fn supported_fact_predicates() -> Vec<SupportedFactPredicate> {
    static PREDICATES: OnceLock<Vec<SupportedFactPredicate>> = OnceLock::new();
    PREDICATES
        .get_or_init(|| {
            SUPPORTED_RULE_FACTS
                .iter()
                .flat_map(|fact| {
                    fact.ops.iter().map(move |op| SupportedFactPredicate {
                        id: leaked_predicate_id(fact.kind, fact.key, op),
                        kind: fact.kind,
                        key: fact.key,
                        op,
                        value_type: predicate_value_type(fact.key),
                        description: predicate_description(fact.kind, fact.key, op),
                    })
                })
                .collect()
        })
        .clone()
}

pub fn supported_fact_predicate_ids() -> Vec<&'static str> {
    supported_fact_predicates()
        .into_iter()
        .map(|predicate| predicate.id)
        .collect()
}

pub fn parse_supported_fact_predicate(id: &str) -> Option<SupportedFactPredicate> {
    supported_fact_predicates()
        .into_iter()
        .find(|predicate| predicate.id == id)
}

fn leaked_predicate_id(kind: &'static str, key: &'static str, op: &'static str) -> &'static str {
    let id = format!("{kind}.{key}:{op}");
    Box::leak(id.into_boxed_str())
}

fn predicate_value_type(key: &str) -> &'static str {
    if key.ends_with("_count") {
        "number"
    } else {
        "string"
    }
}

fn predicate_description(kind: &str, key: &str, op: &str) -> &'static str {
    match (kind, key, op) {
        ("signature", "contains", "eq") => "Matches an extracted signature token exactly.",
        ("path", "extension", "eq") => "Matches a file extension exactly.",
        ("path", "full", "contains") => "Matches a substring of the repository-relative path.",
        ("path", "full", "prefix") => "Matches a repository-relative path prefix.",
        ("path", "full", "suffix") => "Matches a repository-relative path suffix.",
        ("path", "full", "eq") => "Matches a repository-relative path exactly.",
        ("dependency", _, "gte") | ("dependency", _, "lte") => {
            "Matches a numeric dependency fact threshold."
        }
        _ => "Matches the named architecture fact using the listed operator.",
    }
}

#[allow(dead_code)]
fn sorted_unique_values(values: impl Iterator<Item = &'static str>) -> Vec<&'static str> {
    values.collect::<BTreeSet<_>>().into_iter().collect()
}
