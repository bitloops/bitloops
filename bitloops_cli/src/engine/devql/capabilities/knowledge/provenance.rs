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
    source_document_version_id: &str,
    target_type: &str,
    target_id: &str,
    association_method: &str,
) -> Value {
    json!({
        "capability": "knowledge",
        "plugin_type": "first_party",
        "operation": "knowledge.associate",
        "command": command,
        "association_method": association_method,
        "target_type": target_type,
        "target_id": target_id,
        "source_document_version_id": source_document_version_id,
    })
}
