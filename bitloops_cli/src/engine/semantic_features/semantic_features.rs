use std::collections::HashMap;
use std::path::Path;

use anyhow::Result;
use serde_json::json;
use sha2::{Digest, Sha256};

#[path = "common.rs"]
mod common;
#[path = "features.rs"]
mod features;
#[path = "semantic.rs"]
mod semantic;
#[path = "store.rs"]
mod store;

use self::common::{build_body_tokens, normalize_name, normalize_repo_path, normalize_string_list};
use self::features::{SymbolFeaturesRow, build_features_row, normalize_signature};
pub use self::semantic::{
    NoopSemanticSummaryProvider, SemanticSummaryCandidate, SemanticSummaryProvider,
    SemanticSummaryProviderConfig, SemanticSummarySource, build_semantic_summary_provider,
    resolve_semantic_summary_endpoint,
};
use self::semantic::{SymbolSemanticsRow, build_semantics_row, normalize_summary_text};

const SYMBOL_FEATURES_PROMPT_VERSION: &str = "symbol-features-v2";
const SEMANTIC_SUMMARY_PROMPT_VERSION: &str = "semantic-summary-v4";
const MAX_IDENTIFIER_TOKENS: usize = 64;
const MAX_BODY_TOKENS: usize = 256;
const MAX_CONTEXT_TOKENS: usize = 64;
const MAX_SUMMARY_BODY_CHARS: usize = 2_000;

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct PreStageArtefactRow {
    pub artefact_id: String,
    #[serde(default)]
    pub symbol_id: Option<String>,
    pub repo_id: String,
    pub blob_sha: String,
    pub path: String,
    pub language: String,
    pub canonical_kind: String,
    pub language_kind: String,
    pub symbol_fqn: String,
    #[serde(default)]
    pub parent_artefact_id: Option<String>,
    #[serde(default)]
    pub start_line: Option<i32>,
    #[serde(default)]
    pub end_line: Option<i32>,
    #[serde(default)]
    pub start_byte: Option<i32>,
    #[serde(default)]
    pub end_byte: Option<i32>,
    #[serde(default)]
    pub signature: Option<String>,
    #[serde(default)]
    pub doc_comment: Option<String>,
    #[serde(default)]
    pub content_hash: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SemanticFeatureInput {
    pub artefact_id: String,
    pub symbol_id: Option<String>,
    pub repo_id: String,
    pub blob_sha: String,
    pub path: String,
    pub language: String,
    pub canonical_kind: String,
    pub language_kind: String,
    pub symbol_fqn: String,
    pub name: String,
    pub signature: Option<String>,
    pub body: String,
    pub doc_comment: Option<String>,
    pub parent_kind: Option<String>,
    pub parent_symbol: Option<String>,
    pub local_relationships: Vec<String>,
    pub context_hints: Vec<String>,
    pub content_hash: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SemanticFeatureRows {
    pub semantics: SymbolSemanticsRow,
    pub features: SymbolFeaturesRow,
    pub semantic_features_input_hash: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SemanticFeatureIndexState {
    pub semantics_hash: Option<String>,
    pub semantics_prompt_version: Option<String>,
    pub features_hash: Option<String>,
    pub features_prompt_version: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SemanticFeatureIngestionStats {
    pub upserted: usize,
    pub skipped: usize,
}

pub async fn load_pre_stage_artefacts_for_blob(
    pg_client: &tokio_postgres::Client,
    repo_id: &str,
    blob_sha: &str,
    path: &str,
) -> Result<Vec<PreStageArtefactRow>> {
    store::get_artefacts(pg_client, repo_id, blob_sha, path).await
}

pub fn build_semantic_feature_inputs_from_artefacts(
    artefacts: &[PreStageArtefactRow],
    blob_content: &str,
) -> Vec<SemanticFeatureInput> {
    let by_id = artefacts
        .iter()
        .map(|row| (row.artefact_id.clone(), row))
        .collect::<HashMap<_, _>>();
    let child_kinds = build_child_kind_index(artefacts);

    artefacts
        .iter()
        .map(|row| {
            build_semantic_feature_input_from_artefact(row, blob_content, &by_id, &child_kinds)
        })
        .collect()
}

fn build_child_kind_index(artefacts: &[PreStageArtefactRow]) -> HashMap<String, Vec<String>> {
    let mut out: HashMap<String, Vec<String>> = HashMap::new();
    for row in artefacts {
        let Some(parent_id) = row.parent_artefact_id.as_ref() else {
            continue;
        };
        out.entry(parent_id.clone())
            .or_default()
            .push(row.canonical_kind.clone());
    }
    out
}

fn build_semantic_feature_input_from_artefact(
    row: &PreStageArtefactRow,
    blob_content: &str,
    by_id: &HashMap<String, &PreStageArtefactRow>,
    child_kinds: &HashMap<String, Vec<String>>,
) -> SemanticFeatureInput {
    let parent = row
        .parent_artefact_id
        .as_ref()
        .and_then(|parent_id| by_id.get(parent_id))
        .copied();
    let body = extract_symbol_body(row, blob_content);
    let name = derive_symbol_name(row);
    let local_relationships = child_kinds
        .get(&row.artefact_id)
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .map(|kind| format!("contains:{kind}"))
        .collect::<Vec<_>>();
    let mut context_hints = vec![normalize_repo_path(&row.path)];
    context_hints.extend(
        parent
            .into_iter()
            .map(|parent_row| parent_row.symbol_fqn.clone())
            .collect::<Vec<_>>(),
    );

    SemanticFeatureInput {
        artefact_id: row.artefact_id.clone(),
        symbol_id: row.symbol_id.clone(),
        repo_id: row.repo_id.clone(),
        blob_sha: row.blob_sha.clone(),
        path: row.path.clone(),
        language: row.language.clone(),
        canonical_kind: row.canonical_kind.clone(),
        language_kind: row.language_kind.clone(),
        symbol_fqn: row.symbol_fqn.clone(),
        name,
        signature: row.signature.clone(),
        body,
        doc_comment: row.doc_comment.clone(),
        parent_kind: parent.map(|parent_row| parent_row.canonical_kind.clone()),
        parent_symbol: parent.map(|parent_row| parent_row.symbol_fqn.clone()),
        local_relationships,
        context_hints,
        content_hash: row.content_hash.clone(),
    }
}

fn derive_symbol_name(row: &PreStageArtefactRow) -> String {
    if row.canonical_kind == "file" {
        return Path::new(&row.path)
            .file_stem()
            .and_then(|value| value.to_str())
            .unwrap_or(&row.path)
            .to_string();
    }

    row.symbol_fqn
        .rsplit("::")
        .next()
        .filter(|value| !value.is_empty())
        .unwrap_or(&row.symbol_fqn)
        .to_string()
}

fn extract_symbol_body(row: &PreStageArtefactRow, blob_content: &str) -> String {
    if row.canonical_kind == "file" {
        return blob_content.to_string();
    }

    if let (Some(start_byte), Some(end_byte)) = (row.start_byte, row.end_byte) {
        let start_byte = start_byte.max(0) as usize;
        let end_byte = end_byte.max(start_byte as i32) as usize;
        if let Some(slice) = blob_content.get(start_byte..end_byte.min(blob_content.len()))
            && !slice.trim().is_empty()
        {
            return slice.to_string();
        }
    }

    if let (Some(start_line), Some(end_line)) = (row.start_line, row.end_line) {
        let lines = blob_content.lines().collect::<Vec<_>>();
        let start = start_line.max(1) as usize - 1;
        let end = end_line.max(start_line) as usize;
        if start < lines.len() {
            return lines[start..end.min(lines.len())].join("\n");
        }
    }

    row.signature.clone().unwrap_or_default()
}

pub async fn upsert_semantic_feature_rows(
    pg_client: &tokio_postgres::Client,
    inputs: &[SemanticFeatureInput],
    summary_provider: &dyn SemanticSummaryProvider,
) -> Result<SemanticFeatureIngestionStats> {
    let mut stats = SemanticFeatureIngestionStats::default();

    for input in inputs {
        let rows = build_semantic_feature_rows(input, summary_provider);
        let state = store::get_index_state(pg_client, &input.artefact_id).await?;
        if !semantic_features_require_reindex(
            &state,
            &rows.semantic_features_input_hash,
            &rows.semantics.prompt_version,
            &rows.features.prompt_version,
        ) {
            stats.skipped += 1;
            continue;
        }

        store::persist_rows(pg_client, &rows).await?;
        stats.upserted += 1;
    }

    Ok(stats)
}

pub fn build_semantic_feature_rows(
    input: &SemanticFeatureInput,
    summary_provider: &dyn SemanticSummaryProvider,
) -> SemanticFeatureRows {
    let semantics = build_semantics_row(input, summary_provider);
    let features = build_features_row(input);
    let semantic_features_input_hash = build_semantic_features_input_hash(input);
    SemanticFeatureRows {
        semantics,
        features,
        semantic_features_input_hash,
    }
}

fn build_semantic_features_input_hash(input: &SemanticFeatureInput) -> String {
    sha256_hex(
        &json!({
            "artefact_id": &input.artefact_id,
            "symbol_id": &input.symbol_id,
            "repo_id": &input.repo_id,
            "blob_sha": &input.blob_sha,
            "path": normalize_repo_path(&input.path),
            "language": input.language.to_ascii_lowercase(),
            "canonical_kind": input.canonical_kind.to_ascii_lowercase(),
            "language_kind": input.language_kind.to_ascii_lowercase(),
            "symbol_fqn": &input.symbol_fqn,
            "name": normalize_name(&input.name),
            "signature": input.signature.as_deref().map(normalize_signature),
            "body_tokens": build_body_tokens(&input.body),
            "doc_comment": input
                .doc_comment
                .as_deref()
                .map(normalize_summary_text)
                .filter(|value| !value.is_empty()),
            "parent_kind": input.parent_kind.as_deref().map(|value| value.to_ascii_lowercase()),
            "parent_symbol": &input.parent_symbol,
            "local_relationships": normalize_string_list(&input.local_relationships),
            "context_hints": normalize_string_list(&input.context_hints),
            "content_hash": &input.content_hash,
        })
        .to_string(),
    )
}

// Incremental indexing rule: recompute enrichment only when symbol inputs or prompt versions change.
pub fn semantic_features_require_reindex(
    state: &SemanticFeatureIndexState,
    next_input_hash: &str,
    semantics_prompt_version: &str,
    features_prompt_version: &str,
) -> bool {
    state.semantics_hash.as_deref() != Some(next_input_hash)
        || state.features_hash.as_deref() != Some(next_input_hash)
        || state.semantics_prompt_version.as_deref() != Some(semantics_prompt_version)
        || state.features_prompt_version.as_deref() != Some(features_prompt_version)
}

fn sha256_hex(input: &str) -> String {
    let digest = Sha256::digest(input.as_bytes());
    let mut out = String::with_capacity(digest.len() * 2);
    for byte in digest {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_row() -> PreStageArtefactRow {
        PreStageArtefactRow {
            artefact_id: "artefact-1".to_string(),
            symbol_id: Some("symbol-1".to_string()),
            repo_id: "repo-1".to_string(),
            blob_sha: "blob-1".to_string(),
            path: "src/services/user.ts".to_string(),
            language: "typescript".to_string(),
            canonical_kind: "method".to_string(),
            language_kind: "method".to_string(),
            symbol_fqn: "src/services/user.ts::UserService::getById".to_string(),
            parent_artefact_id: Some("parent-1".to_string()),
            start_line: Some(4),
            end_line: Some(6),
            start_byte: None,
            end_byte: None,
            signature: Some("async getById(id: string): Promise<User> {".to_string()),
            doc_comment: Some("Fetch a user by id.".to_string()),
            content_hash: Some("hash-1".to_string()),
        }
    }

    #[test]
    fn semantic_features_extract_symbol_body_falls_back_to_line_range() {
        let row = sample_row();
        let content = r#"export class UserService {
  // Fetch a user by id.
  // Returns null when missing.
  async getById(id: string) {
    return db.users.findById(id);
  }
}"#;

        let body = extract_symbol_body(&row, content);
        assert!(body.contains("async getById"));
        assert!(body.contains("findById"));
    }

    #[test]
    fn semantic_features_input_hash_changes_when_doc_comment_changes() {
        let base = SemanticFeatureInput {
            artefact_id: "artefact-1".to_string(),
            symbol_id: Some("symbol-1".to_string()),
            repo_id: "repo-1".to_string(),
            blob_sha: "blob-1".to_string(),
            path: "src/services/user.ts".to_string(),
            language: "typescript".to_string(),
            canonical_kind: "function".to_string(),
            language_kind: "function".to_string(),
            symbol_fqn: "src/services/user.ts::normalizeEmail".to_string(),
            name: "normalizeEmail".to_string(),
            signature: Some("export function normalizeEmail(email: string): string {".to_string()),
            body: "return email.trim().toLowerCase();".to_string(),
            doc_comment: Some("Normalize email addresses.".to_string()),
            parent_kind: Some("file".to_string()),
            parent_symbol: Some("src/services/user.ts".to_string()),
            local_relationships: vec![],
            context_hints: vec!["src/services/user.ts".to_string()],
            content_hash: Some("hash-1".to_string()),
        };
        let mut changed = base.clone();
        changed.doc_comment = Some("Normalizes email for storage.".to_string());

        assert_ne!(
            build_semantic_features_input_hash(&base),
            build_semantic_features_input_hash(&changed)
        );
    }
}
