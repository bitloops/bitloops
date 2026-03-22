use serde_json::{Value, json};

use crate::host::capability_host::CapabilityIngestContext;

use super::descriptor::KNOWLEDGE_DESCRIPTOR;
use super::types::ParsedKnowledgeUrl;

/// Optional host dispatch metadata (`None` outside `DevqlCapabilityHost::invoke_ingester`).
#[derive(Debug, Clone, Copy, Default)]
pub struct IngestInvocation<'a> {
    pub capability_id: Option<&'a str>,
    pub ingester_id: Option<&'a str>,
}

impl<'a> IngestInvocation<'a> {
    pub fn from_context(ctx: &'a dyn CapabilityIngestContext) -> Self {
        Self {
            capability_id: ctx.invoking_capability_id(),
            ingester_id: ctx.invoking_ingester_id(),
        }
    }
}

fn merge_invocation(doc: &mut Value, invocation: IngestInvocation<'_>) {
    let Some(obj) = doc.as_object_mut() else {
        return;
    };
    if let Some(c) = invocation.capability_id {
        obj.insert("invoking_capability_id".into(), json!(c));
    }
    if let Some(i) = invocation.ingester_id {
        obj.insert("ingester_id".into(), json!(i));
    }
}

/// Labels for persisted ingestion provenance (`operation` / `command` surface).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IngestWriteLabels {
    pub operation: &'static str,
    pub command: &'static str,
}

/// Host / `knowledge.add` ingester path.
pub const INGEST_WRITE_ADD: IngestWriteLabels = IngestWriteLabels {
    operation: "knowledge.add",
    command: "bitloops devql knowledge add",
};

/// `knowledge.refresh` ingester path.
pub const INGEST_WRITE_REFRESH: IngestWriteLabels = IngestWriteLabels {
    operation: "knowledge.refresh",
    command: "bitloops devql knowledge refresh",
};

pub fn build_ingestion_provenance(
    parsed: &ParsedKnowledgeUrl,
    labels: IngestWriteLabels,
    invocation: IngestInvocation<'_>,
) -> Value {
    let mut doc = json!({
        "capability": KNOWLEDGE_DESCRIPTOR.id,
        "capability_version": KNOWLEDGE_DESCRIPTOR.version,
        "api_version": KNOWLEDGE_DESCRIPTOR.api_version,
        "plugin_type": "first_party",
        "operation": labels.operation,
        "command": labels.command,
        "provider": parsed.provider.as_str(),
        "source_kind": parsed.source_kind.as_str(),
        "source_url": parsed.canonical_url,
    });
    merge_invocation(&mut doc, invocation);
    doc
}

pub fn build_association_provenance(
    command: &str,
    source_knowledge_item_version_id: &str,
    target_type: &str,
    target_id: &str,
    target_knowledge_item_version_id: Option<&str>,
    association_method: &str,
    invocation: IngestInvocation<'_>,
) -> Value {
    let mut doc = json!({
        "capability": KNOWLEDGE_DESCRIPTOR.id,
        "capability_version": KNOWLEDGE_DESCRIPTOR.version,
        "api_version": KNOWLEDGE_DESCRIPTOR.api_version,
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

    merge_invocation(&mut doc, invocation);
    doc
}
