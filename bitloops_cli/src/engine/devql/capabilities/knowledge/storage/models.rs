use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use serde_json::Value;
use sha2::Digest;

use crate::engine::devql::deterministic_uuid;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KnowledgePayloadRef {
    pub storage_backend: String,
    pub storage_path: String,
    pub mime_type: String,
    pub size_bytes: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KnowledgeSourceRow {
    pub knowledge_source_id: String,
    pub provider: String,
    pub source_kind: String,
    pub canonical_external_id: String,
    pub canonical_url: String,
    pub provenance_json: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KnowledgeItemRow {
    pub knowledge_item_id: String,
    pub repo_id: String,
    pub knowledge_source_id: String,
    pub item_kind: String,
    pub latest_knowledge_item_version_id: String,
    pub provenance_json: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct KnowledgeRelationAssertionRow {
    pub relation_assertion_id: String,
    pub repo_id: String,
    pub knowledge_item_id: String,
    pub source_knowledge_item_version_id: String,
    pub target_type: String,
    pub target_id: String,
    pub target_knowledge_item_version_id: Option<String>,
    pub relation_type: String,
    pub association_method: String,
    pub confidence: f64,
    pub provenance_json: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KnowledgeDocumentVersionRow {
    pub knowledge_item_version_id: String,
    pub knowledge_item_id: String,
    pub provider: String,
    pub source_kind: String,
    pub content_hash: String,
    pub title: String,
    pub state: Option<String>,
    pub author: Option<String>,
    pub updated_at: Option<String>,
    pub body_preview: Option<String>,
    pub normalized_fields_json: String,
    pub storage_backend: String,
    pub storage_path: String,
    pub payload_mime_type: String,
    pub payload_size_bytes: i64,
    pub provenance_json: String,
    pub created_at: Option<String>,
}

pub fn knowledge_payload_key(
    repo_id: &str,
    knowledge_item_id: &str,
    knowledge_item_version_id: &str,
) -> String {
    format!("knowledge/{repo_id}/{knowledge_item_id}/{knowledge_item_version_id}/payload.json")
}

pub fn knowledge_source_id(canonical_external_id: &str) -> String {
    deterministic_uuid(&format!("knowledge-source://{canonical_external_id}"))
}

pub fn knowledge_item_id(repo_id: &str, knowledge_source_id: &str) -> String {
    deterministic_uuid(&format!("knowledge-item://{repo_id}/{knowledge_source_id}"))
}

pub fn knowledge_item_version_id(knowledge_item_id: &str, content_hash: &str) -> String {
    deterministic_uuid(&format!(
        "knowledge-version://{knowledge_item_id}/{content_hash}"
    ))
}

pub fn relation_assertion_id(
    knowledge_item_id: &str,
    source_knowledge_item_version_id: &str,
    target_type: &str,
    target_id: &str,
    target_knowledge_item_version_id: Option<&str>,
    association_method: &str,
) -> String {
    deterministic_uuid(&format!(
        "knowledge-relation://{knowledge_item_id}/{source_knowledge_item_version_id}/{target_type}/{target_id}/{}/{association_method}",
        target_knowledge_item_version_id.unwrap_or("-")
    ))
}

pub fn serialize_payload(payload: &Value) -> Result<Vec<u8>> {
    serde_json::to_vec(payload).context("serialising knowledge payload JSON")
}

pub fn content_hash(bytes: &[u8]) -> String {
    let digest = sha2::Sha256::digest(bytes);
    format!("{digest:x}")
}

pub fn ensure_parent_dir(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("creating parent directory {}", parent.display()))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn payload_key_is_stable() {
        assert_eq!(
            knowledge_payload_key("repo-1", "item-1", "version-1"),
            "knowledge/repo-1/item-1/version-1/payload.json"
        );
    }

    #[test]
    fn relation_assertion_id_distinguishes_target_versions() {
        let base = relation_assertion_id(
            "item-1",
            "source-version-1",
            "knowledge_item",
            "target-item-1",
            None,
            "manual_attachment",
        );
        let v1 = relation_assertion_id(
            "item-1",
            "source-version-1",
            "knowledge_item",
            "target-item-1",
            Some("target-version-1"),
            "manual_attachment",
        );
        let v2 = relation_assertion_id(
            "item-1",
            "source-version-1",
            "knowledge_item",
            "target-item-1",
            Some("target-version-2"),
            "manual_attachment",
        );

        assert_ne!(base, v1);
        assert_ne!(v1, v2);
    }

    #[test]
    fn ensure_parent_dir_creates_directories() {
        let temp = TempDir::new().expect("temp dir");
        let path = temp.path().join("nested/file.txt");

        ensure_parent_dir(&path).expect("ensure parent dir");

        assert!(fs::metadata(temp.path().join("nested")).is_ok());
    }
}
