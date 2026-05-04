use anyhow::{Context, Result};
use serde_json::Value;

use crate::capability_packs::knowledge::storage::{
    KnowledgeDocumentVersionRow, KnowledgeRelationAssertionRow,
};

use super::storage::guidance_hash_for_parts;
use super::workplane::{ContextGuidanceMailboxPayload, KnowledgeEvidencePayload};

pub fn knowledge_guidance_payloads(
    repo_id: &str,
    version: &KnowledgeDocumentVersionRow,
    relations: &[KnowledgeRelationAssertionRow],
) -> Result<Vec<ContextGuidanceMailboxPayload>> {
    let mut payloads = Vec::new();
    for relation in relations {
        let (target_paths, target_symbols) = relation_targets(relation);
        if target_paths.is_empty() && target_symbols.is_empty() {
            continue;
        }
        payloads.push(ContextGuidanceMailboxPayload::KnowledgeEvidence(Box::new(
            KnowledgeEvidencePayload {
                repo_id: repo_id.to_string(),
                knowledge_item_id: version.knowledge_item_id.clone(),
                knowledge_item_version_id: version.knowledge_item_version_id.clone(),
                relation_assertion_id: Some(relation.relation_assertion_id.clone()),
                provider: version.provider.clone(),
                source_kind: version.source_kind.clone(),
                title: Some(version.title.clone()),
                url: knowledge_url_from_provenance(version.provenance_json.as_str())?,
                updated_at: version.updated_at.clone(),
                body_preview: version.body_preview.clone(),
                normalized_fields_json: version.normalized_fields_json.clone(),
                target_paths,
                target_symbols,
                input_hash: knowledge_guidance_input_hash(version, relation),
            },
        )));
    }
    Ok(payloads)
}

fn relation_targets(relation: &KnowledgeRelationAssertionRow) -> (Vec<String>, Vec<String>) {
    match relation.target_type.as_str() {
        "artefact" | "path" => (vec![relation.target_id.clone()], Vec::new()),
        "symbol_fqn" => (Vec::new(), vec![relation.target_id.clone()]),
        _ => (Vec::new(), Vec::new()),
    }
}

fn knowledge_url_from_provenance(provenance_json: &str) -> Result<Option<String>> {
    let value: Value = serde_json::from_str(provenance_json)
        .context("parsing knowledge provenance for context guidance URL")?;
    Ok(value
        .get("canonicalUrl")
        .or_else(|| value.get("canonical_url"))
        .or_else(|| value.get("source_url"))
        .and_then(Value::as_str)
        .map(str::to_string))
}

fn knowledge_guidance_input_hash(
    version: &KnowledgeDocumentVersionRow,
    relation: &KnowledgeRelationAssertionRow,
) -> String {
    guidance_hash_for_parts(&[
        version.knowledge_item_version_id.as_str(),
        version.content_hash.as_str(),
        relation.relation_assertion_id.as_str(),
        relation.target_type.as_str(),
        relation.target_id.as_str(),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn version() -> KnowledgeDocumentVersionRow {
        KnowledgeDocumentVersionRow {
            knowledge_item_version_id: "version-1".to_string(),
            knowledge_item_id: "item-1".to_string(),
            provider: "github".to_string(),
            source_kind: "github_issue".to_string(),
            content_hash: "hash-1".to_string(),
            title: "Issue title".to_string(),
            state: Some("open".to_string()),
            author: Some("spiros".to_string()),
            updated_at: Some("2026-04-30T10:00:00Z".to_string()),
            body_preview: Some("Preserve parser boundary decision.".to_string()),
            normalized_fields_json: "{}".to_string(),
            storage_backend: "local".to_string(),
            storage_path: "knowledge/repo/item/version/payload.json".to_string(),
            payload_mime_type: "application/json".to_string(),
            payload_size_bytes: 10,
            provenance_json: r#"{"source_url":"https://github.com/org/repo/issues/1"}"#.to_string(),
            created_at: None,
        }
    }

    fn relation(target_type: &str, target_id: &str) -> KnowledgeRelationAssertionRow {
        KnowledgeRelationAssertionRow {
            relation_assertion_id: format!("relation-{target_type}"),
            repo_id: "repo-1".to_string(),
            knowledge_item_id: "item-1".to_string(),
            source_knowledge_item_version_id: "version-1".to_string(),
            target_type: target_type.to_string(),
            target_id: target_id.to_string(),
            target_knowledge_item_version_id: None,
            relation_type: "associated_with".to_string(),
            association_method: "manual_attachment".to_string(),
            confidence: 1.0,
            provenance_json: "{}".to_string(),
        }
    }

    #[test]
    fn bridge_builds_payload_for_path_relation() -> Result<()> {
        let version = version();
        let relation = relation("path", "src/lib.rs");

        let payloads = knowledge_guidance_payloads("repo-1", &version, &[relation])?;

        assert_eq!(payloads.len(), 1);
        let ContextGuidanceMailboxPayload::KnowledgeEvidence(payload) = &payloads[0] else {
            panic!("expected knowledge payload");
        };
        assert_eq!(payload.target_paths.as_slice(), &["src/lib.rs".to_string()]);
        assert_eq!(
            payload.url.as_deref(),
            Some("https://github.com/org/repo/issues/1")
        );
        Ok(())
    }

    #[test]
    fn bridge_builds_payload_for_symbol_relation_and_skips_unsupported_targets() -> Result<()> {
        let version = version();
        let relations = vec![
            relation("commit", "abc123"),
            relation("symbol_fqn", "crate::lib::run"),
        ];

        let payloads = knowledge_guidance_payloads("repo-1", &version, &relations)?;

        assert_eq!(payloads.len(), 1);
        let ContextGuidanceMailboxPayload::KnowledgeEvidence(payload) = &payloads[0] else {
            panic!("expected knowledge payload");
        };
        assert_eq!(
            payload.target_symbols.as_slice(),
            &["crate::lib::run".to_string()]
        );
        Ok(())
    }
}
