use std::collections::HashMap;
use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

#[path = "common.rs"]
mod common;
#[path = "features.rs"]
mod features;
#[path = "semantic.rs"]
mod semantic;

use self::common::{build_body_tokens, normalize_name, normalize_string_list};
use self::features::{
    SymbolFeaturesRow, build_features_row, count_parameters_from_signature,
    infer_return_shape_hint, normalize_signature,
};
pub use self::semantic::{
    NoopSemanticSummaryProvider, SemanticSummaryCandidate, SemanticSummaryProvider,
    SemanticSummaryProviderConfig, SemanticSummarySource, build_semantic_summary_provider,
    resolve_semantic_summary_endpoint,
};
use self::semantic::{SymbolSemanticsRow, build_semantics_row, normalize_summary_text};

const SYMBOL_FEATURES_PROMPT_VERSION: &str = "symbol-features-v2";
const SEMANTIC_SUMMARY_PROMPT_VERSION: &str = "semantic-summary-v1";
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
    pub parameter_count: Option<i32>,
    pub return_shape_hint: Option<String>,
    pub modifiers: Vec<String>,
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
    let sql = format!(
        "SELECT artefact_id, symbol_id, repo_id, blob_sha, path, language, canonical_kind, language_kind, symbol_fqn, parent_artefact_id, start_line, end_line, start_byte, end_byte, signature, content_hash \
FROM artefacts \
WHERE repo_id = '{repo_id}' AND blob_sha = '{blob_sha}' AND path = '{path}' \
ORDER BY coalesce(start_byte, 0), coalesce(start_line, 0), artefact_id",
        repo_id = esc_pg(repo_id),
        blob_sha = esc_pg(blob_sha),
        path = esc_pg(path),
    );

    let rows = pg_query_rows(pg_client, &sql).await?;
    let mut artefacts = Vec::new();
    for row in rows {
        artefacts.push(serde_json::from_value::<PreStageArtefactRow>(row)?);
    }
    Ok(artefacts)
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
        .filter(|row| is_semantic_feature_candidate_kind(&row.canonical_kind))
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
    let doc_comment = if row.canonical_kind == "file" {
        extract_file_header_comment(blob_content)
    } else {
        extract_symbol_doc_comment(blob_content, row.start_line)
    };
    let name = derive_symbol_name(row);
    let local_relationships = child_kinds
        .get(&row.artefact_id)
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .map(|kind| format!("contains:{kind}"))
        .collect::<Vec<_>>();
    let return_shape_hint = infer_return_shape_hint(row.signature.as_deref(), &body, None);
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
        doc_comment,
        parent_kind: parent.map(|parent_row| parent_row.canonical_kind.clone()),
        parent_symbol: parent.map(|parent_row| parent_row.symbol_fqn.clone()),
        parameter_count: row
            .signature
            .as_deref()
            .and_then(count_parameters_from_signature),
        return_shape_hint,
        modifiers: extract_modifiers_from_signature(row.signature.as_deref()),
        local_relationships,
        context_hints,
        content_hash: row.content_hash.clone(),
    }
}

fn is_semantic_feature_candidate_kind(kind: &str) -> bool {
    matches!(
        kind,
        "file"
            | "module"
            | "function"
            | "method"
            | "class"
            | "interface"
            | "type"
            | "enum"
            | "variable"
            | "constructor"
            | "test"
    )
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

fn extract_symbol_doc_comment(blob_content: &str, start_line: Option<i32>) -> Option<String> {
    let start_line = start_line?;
    if start_line <= 1 {
        return None;
    }

    let lines = blob_content.lines().collect::<Vec<_>>();
    let start_index = (start_line - 1) as usize;
    if start_index >= lines.len() {
        return None;
    }

    let mut comment_lines = Vec::new();
    let mut in_block = false;
    for index in (0..start_index).rev() {
        let trimmed = lines[index].trim();
        if trimmed.is_empty() {
            break;
        }

        if in_block {
            comment_lines.push(trimmed.to_string());
            if trimmed.contains("/*") {
                break;
            }
            continue;
        }

        let is_comment = trimmed.starts_with("//")
            || trimmed.starts_with("/*")
            || trimmed.starts_with('*')
            || trimmed.ends_with("*/");
        if !is_comment {
            break;
        }

        comment_lines.push(trimmed.to_string());
        if trimmed.ends_with("*/") && !trimmed.contains("/*") {
            in_block = true;
        } else if trimmed.starts_with("/*") {
            break;
        }
    }

    if comment_lines.is_empty() {
        None
    } else {
        comment_lines.reverse();
        Some(comment_lines.join("\n"))
    }
}

fn extract_file_header_comment(content: &str) -> Option<String> {
    let lines = content.lines().collect::<Vec<_>>();
    if lines.is_empty() {
        return None;
    }

    let mut collected = Vec::new();
    let mut in_block = false;
    for line in lines {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            if collected.is_empty() {
                continue;
            }
            break;
        }

        if in_block {
            collected.push(trimmed.to_string());
            if trimmed.contains("*/") {
                break;
            }
            continue;
        }

        if trimmed.starts_with("//") || trimmed.starts_with("/*") {
            collected.push(trimmed.to_string());
            if trimmed.starts_with("/*") && !trimmed.contains("*/") {
                in_block = true;
            } else if trimmed.starts_with("/*") {
                break;
            }
            continue;
        }

        break;
    }

    if collected.is_empty() {
        None
    } else {
        Some(collected.join("\n"))
    }
}

fn extract_modifiers_from_signature(signature: Option<&str>) -> Vec<String> {
    let Some(signature) = signature else {
        return Vec::new();
    };
    let lowered = signature.to_ascii_lowercase();
    let mut modifiers = Vec::new();
    for modifier in [
        "export",
        "default",
        "async",
        "pub",
        "static",
        "private",
        "protected",
        "readonly",
    ] {
        if lowered.contains(modifier) {
            modifiers.push(modifier.to_string());
        }
    }
    modifiers
}

pub async fn upsert_semantic_feature_rows(
    pg_client: &tokio_postgres::Client,
    inputs: &[SemanticFeatureInput],
    summary_provider: &dyn SemanticSummaryProvider,
) -> Result<SemanticFeatureIngestionStats> {
    let mut stats = SemanticFeatureIngestionStats::default();

    for input in inputs {
        let rows = build_semantic_feature_rows(input, summary_provider);
        let state = load_semantic_feature_index_state(pg_client, &input.artefact_id).await?;
        if !semantic_features_require_reindex(
            &state,
            &rows.semantic_features_input_hash,
            &rows.semantics.prompt_version,
            &rows.features.prompt_version,
        ) {
            stats.skipped += 1;
            continue;
        }

        persist_semantic_feature_rows(pg_client, &rows).await?;
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
            "parameter_count": input.parameter_count,
            "return_shape_hint": input
                .return_shape_hint
                .as_deref()
                .map(|value| value.to_ascii_lowercase()),
            "modifiers": normalize_string_list(&input.modifiers),
            "local_relationships": normalize_string_list(&input.local_relationships),
            "context_hints": normalize_string_list(&input.context_hints),
            "content_hash": &input.content_hash,
        })
        .to_string(),
    )
}

async fn load_semantic_feature_index_state(
    pg_client: &tokio_postgres::Client,
    artefact_id: &str,
) -> Result<SemanticFeatureIndexState> {
    let sql = format!(
        "SELECT \
            (SELECT semantic_features_input_hash FROM symbol_semantics WHERE artefact_id = '{artefact_id}') AS semantics_hash, \
            (SELECT prompt_version FROM symbol_semantics WHERE artefact_id = '{artefact_id}') AS semantics_prompt_version, \
            (SELECT semantic_features_input_hash FROM symbol_features WHERE artefact_id = '{artefact_id}') AS features_hash, \
            (SELECT prompt_version FROM symbol_features WHERE artefact_id = '{artefact_id}') AS features_prompt_version",
        artefact_id = esc_pg(artefact_id),
    );

    let rows = pg_query_rows(pg_client, &sql).await?;
    let Some(row) = rows.first() else {
        return Ok(SemanticFeatureIndexState::default());
    };

    Ok(SemanticFeatureIndexState {
        semantics_hash: row
            .get("semantics_hash")
            .and_then(Value::as_str)
            .map(str::to_string),
        semantics_prompt_version: row
            .get("semantics_prompt_version")
            .and_then(Value::as_str)
            .map(str::to_string),
        features_hash: row
            .get("features_hash")
            .and_then(Value::as_str)
            .map(str::to_string),
        features_prompt_version: row
            .get("features_prompt_version")
            .and_then(Value::as_str)
            .map(str::to_string),
    })
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

async fn persist_semantic_feature_rows(
    pg_client: &tokio_postgres::Client,
    rows: &SemanticFeatureRows,
) -> Result<()> {
    let semantics = &rows.semantics;
    let features = &rows.features;

    let doc_comment_summary_expr = match semantics.doc_comment_summary.as_deref() {
        Some(value) => format!("'{}'", esc_pg(value)),
        None => "NULL".to_string(),
    };
    let llm_summary_expr = match semantics.llm_summary.as_deref() {
        Some(value) => format!("'{}'", esc_pg(value)),
        None => "NULL".to_string(),
    };
    let source_model_expr = match semantics.source_model.as_deref() {
        Some(value) => format!("'{}'", esc_pg(value)),
        None => "NULL".to_string(),
    };
    let normalized_signature_expr = match features.normalized_signature.as_deref() {
        Some(value) => format!("'{}'", esc_pg(value)),
        None => "NULL".to_string(),
    };
    let parent_kind_expr = match features.parent_kind.as_deref() {
        Some(value) => format!("'{}'", esc_pg(value)),
        None => "NULL".to_string(),
    };
    let parent_symbol_expr = match features.parent_symbol.as_deref() {
        Some(value) => format!("'{}'", esc_pg(value)),
        None => "NULL".to_string(),
    };
    let parameter_count_expr = match features.parameter_count {
        Some(value) => value.to_string(),
        None => "NULL".to_string(),
    };
    let return_shape_expr = match features.return_shape_hint.as_deref() {
        Some(value) => format!("'{}'", esc_pg(value)),
        None => "NULL".to_string(),
    };
    let identifier_tokens = format!(
        "'{}'::jsonb",
        esc_pg(&serde_json::to_string(&features.identifier_tokens)?)
    );
    let body_tokens = format!(
        "'{}'::jsonb",
        esc_pg(&serde_json::to_string(&features.normalized_body_tokens)?)
    );
    let modifiers = format!(
        "'{}'::jsonb",
        esc_pg(&serde_json::to_string(&features.modifiers)?)
    );
    let local_relationships = format!(
        "'{}'::jsonb",
        esc_pg(&serde_json::to_string(&features.local_relationships)?)
    );
    let context_tokens = format!(
        "'{}'::jsonb",
        esc_pg(&serde_json::to_string(&features.context_tokens)?)
    );

    let sql = format!(
        "INSERT INTO symbol_semantics (artefact_id, repo_id, blob_sha, semantic_features_input_hash, prompt_version, doc_comment_summary, llm_summary, template_summary, summary, confidence, summary_source, source_model) \
VALUES ('{artefact_id}', '{repo_id}', '{blob_sha}', '{input_hash}', '{prompt_version}', {doc_comment_summary}, {llm_summary}, '{template_summary}', '{summary}', {confidence:.4}, '{summary_source}', {source_model}) \
ON CONFLICT (artefact_id) DO UPDATE SET repo_id = EXCLUDED.repo_id, blob_sha = EXCLUDED.blob_sha, semantic_features_input_hash = EXCLUDED.semantic_features_input_hash, prompt_version = EXCLUDED.prompt_version, doc_comment_summary = EXCLUDED.doc_comment_summary, llm_summary = EXCLUDED.llm_summary, template_summary = EXCLUDED.template_summary, summary = EXCLUDED.summary, confidence = EXCLUDED.confidence, summary_source = EXCLUDED.summary_source, source_model = EXCLUDED.source_model, generated_at = now(); \
INSERT INTO symbol_features (artefact_id, repo_id, blob_sha, semantic_features_input_hash, prompt_version, normalized_name, normalized_signature, identifier_tokens, normalized_body_tokens, parent_kind, parent_symbol, parameter_count, return_shape_hint, modifiers, local_relationships, context_tokens) \
VALUES ('{features_artefact_id}', '{features_repo_id}', '{features_blob_sha}', '{features_input_hash}', '{features_prompt_version}', '{normalized_name}', {normalized_signature}, {identifier_tokens}, {body_tokens}, {parent_kind}, {parent_symbol}, {parameter_count}, {return_shape_hint}, {modifiers}, {local_relationships}, {context_tokens}) \
ON CONFLICT (artefact_id) DO UPDATE SET repo_id = EXCLUDED.repo_id, blob_sha = EXCLUDED.blob_sha, semantic_features_input_hash = EXCLUDED.semantic_features_input_hash, prompt_version = EXCLUDED.prompt_version, normalized_name = EXCLUDED.normalized_name, normalized_signature = EXCLUDED.normalized_signature, identifier_tokens = EXCLUDED.identifier_tokens, normalized_body_tokens = EXCLUDED.normalized_body_tokens, parent_kind = EXCLUDED.parent_kind, parent_symbol = EXCLUDED.parent_symbol, parameter_count = EXCLUDED.parameter_count, return_shape_hint = EXCLUDED.return_shape_hint, modifiers = EXCLUDED.modifiers, local_relationships = EXCLUDED.local_relationships, context_tokens = EXCLUDED.context_tokens, generated_at = now()",
        artefact_id = esc_pg(&semantics.artefact_id),
        repo_id = esc_pg(&semantics.repo_id),
        blob_sha = esc_pg(&semantics.blob_sha),
        input_hash = esc_pg(&rows.semantic_features_input_hash),
        prompt_version = esc_pg(&semantics.prompt_version),
        doc_comment_summary = doc_comment_summary_expr,
        llm_summary = llm_summary_expr,
        template_summary = esc_pg(&semantics.template_summary),
        summary = esc_pg(&semantics.summary),
        confidence = semantics.confidence,
        summary_source = semantics.summary_source.as_str(),
        source_model = source_model_expr,
        features_artefact_id = esc_pg(&features.artefact_id),
        features_repo_id = esc_pg(&features.repo_id),
        features_blob_sha = esc_pg(&features.blob_sha),
        features_input_hash = esc_pg(&rows.semantic_features_input_hash),
        features_prompt_version = esc_pg(&features.prompt_version),
        normalized_name = esc_pg(&features.normalized_name),
        normalized_signature = normalized_signature_expr,
        identifier_tokens = identifier_tokens,
        body_tokens = body_tokens,
        parent_kind = parent_kind_expr,
        parent_symbol = parent_symbol_expr,
        parameter_count = parameter_count_expr,
        return_shape_hint = return_shape_expr,
        modifiers = modifiers,
        local_relationships = local_relationships,
        context_tokens = context_tokens,
    );

    postgres_exec(pg_client, &sql).await
}

fn sha256_hex(input: &str) -> String {
    let digest = Sha256::digest(input.as_bytes());
    let mut out = String::with_capacity(digest.len() * 2);
    for byte in digest {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

async fn postgres_exec(pg_client: &tokio_postgres::Client, sql: &str) -> Result<()> {
    tokio::time::timeout(Duration::from_secs(30), pg_client.batch_execute(sql))
        .await
        .context("Postgres statement timeout after 30s")?
        .context("executing Postgres statements")?;
    Ok(())
}

async fn pg_query_rows(pg_client: &tokio_postgres::Client, sql: &str) -> Result<Vec<Value>> {
    let wrapped = format!(
        "SELECT coalesce(json_agg(t), '[]'::json)::text FROM ({}) t",
        sql.trim().trim_end_matches(';')
    );
    let raw = tokio::time::timeout(Duration::from_secs(30), pg_client.query_one(&wrapped, &[]))
        .await
        .context("Postgres query timeout after 30s")?
        .context("executing Postgres query")?
        .try_get::<_, String>(0)
        .context("reading Postgres scalar text result")?;
    let parsed: Value = serde_json::from_str(raw.trim()).with_context(|| {
        format!(
            "parsing Postgres JSON payload failed: {}",
            truncate_for_error(&raw)
        )
    })?;
    match parsed {
        Value::Array(rows) => Ok(rows),
        Value::Object(_) => Ok(vec![parsed]),
        Value::Null => Ok(vec![]),
        other => bail!("unexpected Postgres JSON payload type: {other}"),
    }
}

fn esc_pg(value: &str) -> String {
    value.replace('\'', "''")
}

fn normalize_repo_path(path: &str) -> String {
    let mut normalized = path.trim().replace('\\', "/");
    while normalized.starts_with("./") {
        normalized = normalized[2..].to_string();
    }
    normalized.trim_start_matches('/').to_string()
}

fn truncate_for_error(input: &str) -> String {
    const MAX: usize = 500;
    let mut out = input.to_string();
    if out.len() > MAX {
        out.truncate(MAX);
        out.push_str("...");
    }
    out
}
