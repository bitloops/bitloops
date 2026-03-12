use anyhow::Result;
use serde_json::Value;

use crate::engine::infra::postgres::{esc_pg, pg_query_rows, postgres_exec};

use super::{PreStageArtefactRow, SemanticFeatureIndexState, SemanticFeatureRows};

pub(super) async fn get_artefacts(
    pg_client: &tokio_postgres::Client,
    repo_id: &str,
    blob_sha: &str,
    path: &str,
) -> Result<Vec<PreStageArtefactRow>> {
    let rows = pg_query_rows(pg_client, &build_get_artefacts_sql(repo_id, blob_sha, path)).await?;
    parse_artefact_rows(rows)
}

pub(super) async fn get_index_state(
    pg_client: &tokio_postgres::Client,
    artefact_id: &str,
) -> Result<SemanticFeatureIndexState> {
    let rows = pg_query_rows(pg_client, &build_get_index_state_sql(artefact_id)).await?;
    Ok(parse_index_state_rows(&rows))
}

pub(super) async fn persist_rows(
    pg_client: &tokio_postgres::Client,
    rows: &SemanticFeatureRows,
) -> Result<()> {
    postgres_exec(pg_client, &build_persist_rows_sql(rows)?).await
}

fn build_get_artefacts_sql(repo_id: &str, blob_sha: &str, path: &str) -> String {
    format!(
        "SELECT artefact_id, symbol_id, repo_id, blob_sha, path, language, canonical_kind, language_kind, symbol_fqn, parent_artefact_id, start_line, end_line, start_byte, end_byte, signature, content_hash \
FROM artefacts \
WHERE repo_id = '{repo_id}' AND blob_sha = '{blob_sha}' AND path = '{path}' \
ORDER BY coalesce(start_byte, 0), coalesce(start_line, 0), artefact_id",
        repo_id = esc_pg(repo_id),
        blob_sha = esc_pg(blob_sha),
        path = esc_pg(path),
    )
}

fn parse_artefact_rows(rows: Vec<Value>) -> Result<Vec<PreStageArtefactRow>> {
    let mut artefacts = Vec::new();
    for row in rows {
        artefacts.push(serde_json::from_value::<PreStageArtefactRow>(row)?);
    }
    Ok(artefacts)
}

fn build_get_index_state_sql(artefact_id: &str) -> String {
    format!(
        "SELECT \
            (SELECT semantic_features_input_hash FROM symbol_semantics WHERE artefact_id = '{artefact_id}') AS semantics_hash, \
            (SELECT prompt_version FROM symbol_semantics WHERE artefact_id = '{artefact_id}') AS semantics_prompt_version, \
            (SELECT semantic_features_input_hash FROM symbol_features WHERE artefact_id = '{artefact_id}') AS features_hash, \
            (SELECT prompt_version FROM symbol_features WHERE artefact_id = '{artefact_id}') AS features_prompt_version",
        artefact_id = esc_pg(artefact_id),
    )
}

fn parse_index_state_rows(rows: &[Value]) -> SemanticFeatureIndexState {
    let Some(row) = rows.first() else {
        return SemanticFeatureIndexState::default();
    };

    SemanticFeatureIndexState {
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
    }
}

fn build_persist_rows_sql(rows: &SemanticFeatureRows) -> Result<String> {
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
    let identifier_tokens = format!(
        "'{}'::jsonb",
        esc_pg(&serde_json::to_string(&features.identifier_tokens)?)
    );
    let body_tokens = format!(
        "'{}'::jsonb",
        esc_pg(&serde_json::to_string(&features.normalized_body_tokens)?)
    );
    let local_relationships = format!(
        "'{}'::jsonb",
        esc_pg(&serde_json::to_string(&features.local_relationships)?)
    );
    let context_tokens = format!(
        "'{}'::jsonb",
        esc_pg(&serde_json::to_string(&features.context_tokens)?)
    );

    Ok(format!(
        "INSERT INTO symbol_semantics (artefact_id, repo_id, blob_sha, semantic_features_input_hash, prompt_version, doc_comment_summary, llm_summary, template_summary, summary, confidence, summary_source, source_model) \
VALUES ('{artefact_id}', '{repo_id}', '{blob_sha}', '{input_hash}', '{prompt_version}', {doc_comment_summary}, {llm_summary}, '{template_summary}', '{summary}', {confidence:.4}, '{summary_source}', {source_model}) \
ON CONFLICT (artefact_id) DO UPDATE SET repo_id = EXCLUDED.repo_id, blob_sha = EXCLUDED.blob_sha, semantic_features_input_hash = EXCLUDED.semantic_features_input_hash, prompt_version = EXCLUDED.prompt_version, doc_comment_summary = EXCLUDED.doc_comment_summary, llm_summary = EXCLUDED.llm_summary, template_summary = EXCLUDED.template_summary, summary = EXCLUDED.summary, confidence = EXCLUDED.confidence, summary_source = EXCLUDED.summary_source, source_model = EXCLUDED.source_model, generated_at = now(); \
INSERT INTO symbol_features (artefact_id, repo_id, blob_sha, semantic_features_input_hash, prompt_version, normalized_name, normalized_signature, identifier_tokens, normalized_body_tokens, parent_kind, parent_symbol, local_relationships, context_tokens) \
VALUES ('{features_artefact_id}', '{features_repo_id}', '{features_blob_sha}', '{features_input_hash}', '{features_prompt_version}', '{normalized_name}', {normalized_signature}, {identifier_tokens}, {body_tokens}, {parent_kind}, {parent_symbol}, {local_relationships}, {context_tokens}) \
ON CONFLICT (artefact_id) DO UPDATE SET repo_id = EXCLUDED.repo_id, blob_sha = EXCLUDED.blob_sha, semantic_features_input_hash = EXCLUDED.semantic_features_input_hash, prompt_version = EXCLUDED.prompt_version, normalized_name = EXCLUDED.normalized_name, normalized_signature = EXCLUDED.normalized_signature, identifier_tokens = EXCLUDED.identifier_tokens, normalized_body_tokens = EXCLUDED.normalized_body_tokens, parent_kind = EXCLUDED.parent_kind, parent_symbol = EXCLUDED.parent_symbol, local_relationships = EXCLUDED.local_relationships, context_tokens = EXCLUDED.context_tokens, generated_at = now()",
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
        local_relationships = local_relationships,
        context_tokens = context_tokens,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    use super::super::features::SymbolFeaturesRow;
    use super::super::semantic::{SemanticSummarySource, SymbolSemanticsRow};

    fn sample_rows() -> SemanticFeatureRows {
        SemanticFeatureRows {
            semantics: SymbolSemanticsRow {
                artefact_id: "artefact'1".to_string(),
                repo_id: "repo-1".to_string(),
                blob_sha: "blob-1".to_string(),
                prompt_version: "semantic-summary-v4::provider=noop".to_string(),
                doc_comment_summary: Some("Fetches O'Brien by id.".to_string()),
                llm_summary: Some("Loads a user by id".to_string()),
                template_summary: "Method get by id.".to_string(),
                summary: "Loads a user by id.".to_string(),
                confidence: 0.82,
                summary_source: SemanticSummarySource::Llm,
                source_model: Some("mock:model".to_string()),
            },
            features: SymbolFeaturesRow {
                artefact_id: "artefact'1".to_string(),
                repo_id: "repo-1".to_string(),
                blob_sha: "blob-1".to_string(),
                prompt_version: "symbol-features-v2".to_string(),
                normalized_name: "get_by_id".to_string(),
                normalized_signature: Some("async getById(id: string)".to_string()),
                identifier_tokens: vec!["get".to_string(), "id".to_string()],
                normalized_body_tokens: vec!["return".to_string(), "find".to_string()],
                parent_kind: Some("class".to_string()),
                parent_symbol: Some("src/services/user.ts::UserService".to_string()),
                local_relationships: vec!["contains:method".to_string()],
                context_tokens: vec!["services".to_string(), "user".to_string()],
            },
            semantic_features_input_hash: "hash-1".to_string(),
        }
    }

    #[test]
    fn semantic_features_store_builds_get_artefacts_sql_with_escaped_values() {
        let sql = build_get_artefacts_sql("repo'1", "blob'1", "src/o'brien.ts");
        assert!(sql.contains("repo_id = 'repo''1'"));
        assert!(sql.contains("blob_sha = 'blob''1'"));
        assert!(sql.contains("path = 'src/o''brien.ts'"));
    }

    #[test]
    fn semantic_features_store_parses_index_state_rows_and_defaults() {
        let empty = parse_index_state_rows(&[]);
        assert_eq!(empty, SemanticFeatureIndexState::default());

        let rows = vec![json!({
            "semantics_hash": "hash-a",
            "semantics_prompt_version": "semantic-v1",
            "features_hash": "hash-b",
            "features_prompt_version": "features-v1"
        })];
        let parsed = parse_index_state_rows(&rows);
        assert_eq!(parsed.semantics_hash.as_deref(), Some("hash-a"));
        assert_eq!(
            parsed.features_prompt_version.as_deref(),
            Some("features-v1")
        );
    }

    #[test]
    fn semantic_features_store_builds_persist_sql_for_semantics_and_features() {
        let sql = build_persist_rows_sql(&sample_rows()).expect("persist SQL");
        assert!(sql.contains("INSERT INTO symbol_semantics"));
        assert!(sql.contains("INSERT INTO symbol_features"));
        assert!(sql.contains("doc_comment_summary"));
        assert!(sql.contains("Fetches O''Brien by id."));
        assert!(sql.contains("'[\"get\",\"id\"]'::jsonb"));
        assert!(sql.contains("summary_source"));
    }

    #[test]
    fn semantic_features_store_parses_artefact_rows() {
        let rows = vec![json!({
            "artefact_id": "artefact-1",
            "symbol_id": "symbol-1",
            "repo_id": "repo-1",
            "blob_sha": "blob-1",
            "path": "src/services/user.ts",
            "language": "typescript",
            "canonical_kind": "function",
            "language_kind": "function",
            "symbol_fqn": "src/services/user.ts::normalizeEmail",
            "parent_artefact_id": null,
            "start_line": 1,
            "end_line": 3,
            "start_byte": null,
            "end_byte": null,
            "signature": "export function normalizeEmail(email: string): string {",
            "content_hash": "hash-1"
        })];
        let parsed = parse_artefact_rows(rows).expect("artefact rows should parse");
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].canonical_kind, "function");
    }
}
