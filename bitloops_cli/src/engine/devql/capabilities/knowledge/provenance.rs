use serde_json::{Value, json};

use super::types::ParsedKnowledgeUrl;

pub fn build_ingestion_provenance(parsed: &ParsedKnowledgeUrl) -> Value {
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

pub fn build_association_provenance(
    command: &str,
    source_knowledge_item_version_id: &str,
    target_type: &str,
    target_id: &str,
    target_knowledge_item_version_id: Option<&str>,
    association_method: &str,
) -> Value {
    let mut doc = json!({
        "capability": "knowledge",
        "plugin_type": "first_party",
        "operation": "knowledge.associate",
        "command": command,
        "association_method": association_method,
        "target_type": target_type,
        "target_id": target_id,
        "source_knowledge_item_version_id": source_knowledge_item_version_id,
    });

    if let Some(version_id) = target_knowledge_item_version_id {
        doc["target_knowledge_item_version_id"] = Value::String(version_id.to_string());
    }

    doc
}
