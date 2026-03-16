use serde_json::{Value, json};

use super::types::ParsedKnowledgeUrl;

pub fn build_provenance(parsed: &ParsedKnowledgeUrl) -> Value {
    json!({
        "capability": "knowledge",
        "plugin_type": "first_party",
        "operation": "knowledge.add",
        "command": "bitloops devql knowledge add",
        "provider": parsed.provider.as_str(),
        "source_kind": parsed.source_kind.as_str(),
        "source_url": parsed.canonical_url,
    })
}
