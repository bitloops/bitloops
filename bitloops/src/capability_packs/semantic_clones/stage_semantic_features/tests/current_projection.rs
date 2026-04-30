use std::collections::HashMap;
use std::sync::Arc;

use serde_json::Value;

use super::super::hydration::{
    current_semantic_artefact_key_from_row, remap_semantic_input_to_current_artefact,
};
use super::super::persistence::persist_current_semantic_feature_rows_for_matching_input;
use super::super::{
    init_sqlite_semantic_features_schema, repair_current_semantic_feature_rows_from_historical,
    upsert_semantic_feature_rows,
};
use super::support::{
    TestSummaryProvider, sample_semantic_input, sqlite_relational_with_current_projection_schema,
};
use crate::capability_packs::semantic_clones::features as semantic;
use crate::host::devql::{RelationalStorage, sqlite_exec_path_allow_create};
use tempfile::tempdir;

#[tokio::test]
async fn historical_semantic_upsert_mirrors_model_backed_rows_into_current_projection() {
    let relational = sqlite_relational_with_current_projection_schema().await;
    relational
        .exec(
            "INSERT INTO artefacts_current (
                repo_id, path, content_id, symbol_id, artefact_id, language,
                canonical_kind, language_kind, symbol_fqn, start_line, end_line,
                start_byte, end_byte, modifiers, updated_at
            ) VALUES (
                'repo-1', 'src/lib.rs', 'content-1', 'symbol-artefact-1', 'artefact-1', 'rust',
                'function', 'function', 'src/lib.rs::artefact-1', 1, 3, 0, 24, '[]', datetime('now')
            );
            INSERT INTO current_file_state (repo_id, path, analysis_mode, effective_content_id)
            VALUES ('repo-1', 'src/lib.rs', 'code', 'content-1');",
        )
        .await
        .expect("seed current projection rows");

    let stats = upsert_semantic_feature_rows(
        &relational,
        &[sample_semantic_input("artefact-1", "content-1")],
        Arc::new(TestSummaryProvider),
    )
    .await
    .expect("upsert historical semantic rows");

    assert_eq!(stats.upserted, 1);
    let rows = relational
        .query_rows(
            "SELECT summary, llm_summary, source_model, content_id
             FROM symbol_semantics_current
             WHERE artefact_id = 'artefact-1'",
        )
        .await
        .expect("load mirrored current summary row");
    assert_eq!(rows.len(), 1);
    assert_eq!(
        rows[0].get("llm_summary").and_then(Value::as_str),
        Some("Summarises the symbol.")
    );
    assert_eq!(
        rows[0].get("source_model").and_then(Value::as_str),
        Some("ollama:ministral-3:3b")
    );
    assert_eq!(
        rows[0].get("content_id").and_then(Value::as_str),
        Some("content-1")
    );
}

#[tokio::test]
async fn historical_semantic_upsert_without_summary_provider_persists_features_only() {
    let relational = sqlite_relational_with_current_projection_schema().await;
    relational
        .exec(
            "INSERT INTO artefacts_current (
                repo_id, path, content_id, symbol_id, artefact_id, language,
                canonical_kind, language_kind, symbol_fqn, start_line, end_line,
                start_byte, end_byte, modifiers, updated_at
            ) VALUES (
                'repo-1', 'src/lib.rs', 'content-1', 'symbol-artefact-1', 'artefact-1', 'rust',
                'function', 'function', 'src/lib.rs::artefact-1', 1, 3, 0, 24, '[]', datetime('now')
            );
            INSERT INTO current_file_state (repo_id, path, analysis_mode, effective_content_id)
            VALUES ('repo-1', 'src/lib.rs', 'code', 'content-1');",
        )
        .await
        .expect("seed current projection rows");

    let stats = upsert_semantic_feature_rows(
        &relational,
        &[sample_semantic_input("artefact-1", "content-1")],
        Arc::new(semantic::NoopSemanticSummaryProvider),
    )
    .await
    .expect("upsert providerless semantic rows");

    assert_eq!(stats.upserted, 1);

    let feature_rows = relational
        .query_rows(
            "SELECT COUNT(*) AS count
             FROM symbol_features
             WHERE artefact_id = 'artefact-1'",
        )
        .await
        .expect("count historical feature rows");
    assert_eq!(
        feature_rows[0].get("count").and_then(Value::as_i64),
        Some(1)
    );

    let current_feature_rows = relational
        .query_rows(
            "SELECT COUNT(*) AS count
             FROM symbol_features_current
             WHERE artefact_id = 'artefact-1'",
        )
        .await
        .expect("count current feature rows");
    assert_eq!(
        current_feature_rows[0].get("count").and_then(Value::as_i64),
        Some(1)
    );

    let semantic_rows = relational
        .query_rows(
            "SELECT COUNT(*) AS count
             FROM symbol_semantics
             WHERE artefact_id = 'artefact-1'",
        )
        .await
        .expect("count historical semantic rows");
    assert_eq!(
        semantic_rows[0].get("count").and_then(Value::as_i64),
        Some(0)
    );

    let current_semantic_rows = relational
        .query_rows(
            "SELECT COUNT(*) AS count
             FROM symbol_semantics_current
             WHERE artefact_id = 'artefact-1'",
        )
        .await
        .expect("count current semantic rows");
    assert_eq!(
        current_semantic_rows[0]
            .get("count")
            .and_then(Value::as_i64),
        Some(0)
    );

    let search_document_rows = relational
        .query_rows(
            "SELECT COUNT(*) AS count
             FROM symbol_search_documents
             WHERE artefact_id = 'artefact-1'",
        )
        .await
        .expect("count historical search document rows");
    assert_eq!(
        search_document_rows[0].get("count").and_then(Value::as_i64),
        Some(0)
    );

    let current_search_document_rows = relational
        .query_rows(
            "SELECT COUNT(*) AS count
             FROM symbol_search_documents_current
             WHERE artefact_id = 'artefact-1'",
        )
        .await
        .expect("count current search document rows");
    assert_eq!(
        current_search_document_rows[0]
            .get("count")
            .and_then(Value::as_i64),
        Some(0)
    );
}

#[tokio::test]
async fn historical_semantic_upsert_with_summary_mode_off_provider_keeps_docstring_summaries() {
    let relational = sqlite_relational_with_current_projection_schema().await;
    relational
        .exec(
            "INSERT INTO artefacts_current (
                repo_id, path, content_id, symbol_id, artefact_id, language,
                canonical_kind, language_kind, symbol_fqn, start_line, end_line,
                start_byte, end_byte, modifiers, updated_at
            ) VALUES (
                'repo-1', 'src/lib.rs', 'content-1', 'symbol-artefact-1', 'artefact-1', 'rust',
                'function', 'function', 'src/lib.rs::artefact-1', 1, 3, 0, 24, '[]', datetime('now')
            );
            INSERT INTO current_file_state (repo_id, path, analysis_mode, effective_content_id)
            VALUES ('repo-1', 'src/lib.rs', 'code', 'content-1');",
        )
        .await
        .expect("seed current projection rows");

    let stats = upsert_semantic_feature_rows(
        &relational,
        &[sample_semantic_input("artefact-1", "content-1")],
        Arc::new(semantic::DocstringOnlySummaryProvider),
    )
    .await
    .expect("upsert docstring-only semantic rows");

    assert_eq!(stats.upserted, 1);

    let semantic_rows = relational
        .query_rows(
            "SELECT docstring_summary, llm_summary, summary
             FROM symbol_semantics
             WHERE artefact_id = 'artefact-1'",
        )
        .await
        .expect("load historical semantic rows");
    assert_eq!(semantic_rows.len(), 1);
    assert_eq!(
        semantic_rows[0]
            .get("docstring_summary")
            .and_then(Value::as_str),
        Some("Performs work.")
    );
    assert_eq!(
        semantic_rows[0].get("llm_summary").and_then(Value::as_str),
        None
    );
    assert_eq!(
        semantic_rows[0].get("summary").and_then(Value::as_str),
        Some("Function artefact. Performs work.")
    );
}

#[tokio::test]
async fn historical_semantic_upsert_with_summary_mode_off_provider_skips_template_only_rows() {
    let relational = sqlite_relational_with_current_projection_schema().await;
    relational
        .exec(
            "INSERT INTO artefacts_current (
                repo_id, path, content_id, symbol_id, artefact_id, language,
                canonical_kind, language_kind, symbol_fqn, start_line, end_line,
                start_byte, end_byte, modifiers, updated_at
            ) VALUES (
                'repo-1', 'src/lib.rs', 'content-1', 'symbol-artefact-1', 'artefact-1', 'rust',
                'function', 'function', 'src/lib.rs::artefact-1', 1, 3, 0, 24, '[]', datetime('now')
            );
            INSERT INTO current_file_state (repo_id, path, analysis_mode, effective_content_id)
            VALUES ('repo-1', 'src/lib.rs', 'code', 'content-1');
            INSERT INTO symbol_semantics_current (
                artefact_id, repo_id, path, content_id, symbol_id, semantic_features_input_hash,
                docstring_summary, llm_summary, template_summary, summary, confidence, source_model
            ) VALUES (
                'artefact-1', 'repo-1', 'src/lib.rs', 'content-1', 'symbol-artefact-1', 'old-hash',
                NULL, NULL, 'Function artefact 1.', 'Function artefact 1.', 0.35, NULL
            );",
        )
        .await
        .expect("seed current projection rows");

    let mut input = sample_semantic_input("artefact-1", "content-1");
    input.docstring = None;

    let stats = upsert_semantic_feature_rows(
        &relational,
        &[input],
        Arc::new(semantic::DocstringOnlySummaryProvider),
    )
    .await
    .expect("upsert docstring-less semantic rows");

    assert_eq!(stats.upserted, 1);

    let semantic_rows = relational
        .query_rows(
            "SELECT COUNT(*) AS count
             FROM symbol_semantics
             WHERE artefact_id = 'artefact-1'",
        )
        .await
        .expect("count historical semantic rows");
    assert_eq!(
        semantic_rows[0].get("count").and_then(Value::as_i64),
        Some(0)
    );

    let current_semantic_rows = relational
        .query_rows(
            "SELECT COUNT(*) AS count
             FROM symbol_semantics_current
             WHERE artefact_id = 'artefact-1'",
        )
        .await
        .expect("count current semantic rows");
    assert_eq!(
        current_semantic_rows[0]
            .get("count")
            .and_then(Value::as_i64),
        Some(0)
    );
}

#[tokio::test]
async fn historical_semantic_upsert_does_not_overwrite_diverged_current_projection() {
    let relational = sqlite_relational_with_current_projection_schema().await;
    relational
        .exec(
            "INSERT INTO artefacts_current (
                repo_id, path, content_id, symbol_id, artefact_id, language,
                canonical_kind, language_kind, symbol_fqn, start_line, end_line,
                start_byte, end_byte, modifiers, updated_at
            ) VALUES (
                'repo-1', 'src/lib.rs', 'content-new', 'symbol-artefact-1', 'artefact-1', 'rust',
                'function', 'function', 'src/lib.rs::artefact-1', 1, 3, 0, 24, '[]', datetime('now')
            );
            INSERT INTO current_file_state (repo_id, path, analysis_mode, effective_content_id)
            VALUES ('repo-1', 'src/lib.rs', 'code', 'content-new');",
        )
        .await
        .expect("seed diverged current projection rows");

    let stats = upsert_semantic_feature_rows(
        &relational,
        &[sample_semantic_input("artefact-1", "content-old")],
        Arc::new(TestSummaryProvider),
    )
    .await
    .expect("upsert historical semantic rows");

    assert_eq!(stats.upserted, 1);
    let rows = relational
        .query_rows(
            "SELECT summary, llm_summary, source_model, content_id
             FROM symbol_semantics_current
             WHERE artefact_id = 'artefact-1'",
        )
        .await
        .expect("load current summary rows");
    assert!(
        rows.is_empty(),
        "historical summary refresh must not overwrite a newer current projection"
    );
}

#[tokio::test]
async fn repair_current_projection_from_historical_rows_populates_missing_current_rows() {
    let relational = sqlite_relational_with_current_projection_schema().await;
    relational
        .exec(
            "INSERT INTO artefacts_current (
                repo_id, path, content_id, symbol_id, artefact_id, language,
                canonical_kind, language_kind, symbol_fqn, start_line, end_line,
                start_byte, end_byte, modifiers, updated_at
            ) VALUES (
                'repo-1', 'src/lib.rs', 'blob-1', 'symbol-1', 'artefact-1', 'rust',
                NULL, 'impl_item', 'src/lib.rs::impl@12', 12, 20,
                100, 220, '[]', datetime('now')
            );
            INSERT INTO current_file_state (repo_id, path, analysis_mode, effective_content_id)
            VALUES ('repo-1', 'src/lib.rs', 'code', 'blob-1');
            INSERT INTO symbol_features (
                artefact_id, repo_id, blob_sha, semantic_features_input_hash,
                normalized_name, normalized_signature, modifiers, identifier_tokens,
                normalized_body_tokens, parent_kind, context_tokens
            ) VALUES (
                'artefact-1', 'repo-1', 'blob-1', 'hash-1',
                'impl_12', 'impl Service for Handler', '[]', '[\"impl\"]',
                '[\"handler\"]', 'struct_item', '[\"Service\"]'
            );
            INSERT INTO symbol_semantics (
                artefact_id, repo_id, blob_sha, semantic_features_input_hash,
                docstring_summary, llm_summary, template_summary, summary, confidence, source_model
            ) VALUES (
                'artefact-1', 'repo-1', 'blob-1', 'hash-1',
                NULL, 'Implements request handling.', 'Impl item impl.', 'Implements request handling.', 0.95, 'test:model'
            );",
        )
        .await
        .expect("seed stranded current artefact");

    repair_current_semantic_feature_rows_from_historical(
        &relational,
        "repo-1",
        &["artefact-1".to_string()],
    )
    .await
    .expect("repair current projection");

    let rows = relational
        .query_rows(
            "SELECT f.semantic_features_input_hash AS feature_hash,
                    s.semantic_features_input_hash AS semantic_hash,
                    s.summary
             FROM symbol_features_current f
             JOIN symbol_semantics_current s
               ON s.repo_id = f.repo_id
              AND s.artefact_id = f.artefact_id
              AND s.content_id = f.content_id
             WHERE f.repo_id = 'repo-1' AND f.artefact_id = 'artefact-1'",
        )
        .await
        .expect("load repaired current rows");

    assert_eq!(rows.len(), 1);
    assert_eq!(
        rows[0].get("feature_hash").and_then(Value::as_str),
        Some("hash-1")
    );
    assert_eq!(
        rows[0].get("semantic_hash").and_then(Value::as_str),
        Some("hash-1")
    );
    assert_eq!(
        rows[0].get("summary").and_then(Value::as_str),
        Some("Implements request handling.")
    );
}

#[tokio::test]
async fn sqlite_semantic_feature_schema_upgrade_repairs_stranded_current_projection() {
    let temp = tempdir().expect("temp dir");
    let db_path = temp.path().join("semantic.sqlite");
    sqlite_exec_path_allow_create(
        &db_path,
        &format!(
            "{}\nCREATE TABLE artefacts_current (
    repo_id TEXT NOT NULL,
    path TEXT NOT NULL,
    content_id TEXT NOT NULL,
    symbol_id TEXT NOT NULL,
    artefact_id TEXT NOT NULL,
    language TEXT NOT NULL,
    extraction_fingerprint TEXT,
    canonical_kind TEXT,
    language_kind TEXT,
    symbol_fqn TEXT,
    parent_symbol_id TEXT,
    parent_artefact_id TEXT,
    start_line INTEGER NOT NULL,
    end_line INTEGER NOT NULL,
    start_byte INTEGER NOT NULL,
    end_byte INTEGER NOT NULL,
    signature TEXT,
    modifiers TEXT NOT NULL DEFAULT '[]',
    docstring TEXT,
    updated_at TEXT NOT NULL,
    PRIMARY KEY (repo_id, path, symbol_id),
    UNIQUE (repo_id, artefact_id)
);
CREATE TABLE current_file_state (
    repo_id TEXT NOT NULL,
    path TEXT NOT NULL,
    analysis_mode TEXT NOT NULL,
    effective_content_id TEXT NOT NULL,
    PRIMARY KEY (repo_id, path)
);",
            super::super::semantic_features_sqlite_schema_sql(),
        ),
    )
    .await
    .expect("create schema");

    let relational = RelationalStorage::local_only(db_path.clone());
    relational
        .exec(
            "INSERT INTO artefacts_current (
                repo_id, path, content_id, symbol_id, artefact_id, language,
                canonical_kind, language_kind, symbol_fqn, start_line, end_line,
                start_byte, end_byte, modifiers, updated_at
            ) VALUES (
                'repo-1', 'src/lib.rs', 'blob-1', 'symbol-1', 'artefact-1', 'rust',
                NULL, 'impl_item', 'src/lib.rs::impl@12', 12, 20,
                100, 220, '[]', datetime('now')
            );
            INSERT INTO current_file_state (repo_id, path, analysis_mode, effective_content_id)
            VALUES ('repo-1', 'src/lib.rs', 'code', 'blob-1');
            INSERT INTO symbol_features (
                artefact_id, repo_id, blob_sha, semantic_features_input_hash,
                normalized_name, normalized_signature, modifiers, identifier_tokens,
                normalized_body_tokens, parent_kind, context_tokens
            ) VALUES (
                'artefact-1', 'repo-1', 'blob-1', 'hash-1',
                'impl_12', 'impl Service for Handler', '[]', '[\"impl\"]',
                '[\"handler\"]', 'struct_item', '[\"Service\"]'
            );
            INSERT INTO symbol_semantics (
                artefact_id, repo_id, blob_sha, semantic_features_input_hash,
                docstring_summary, llm_summary, template_summary, summary, confidence, source_model
            ) VALUES (
                'artefact-1', 'repo-1', 'blob-1', 'hash-1',
                NULL, 'Implements request handling.', 'Impl item impl.', 'Implements request handling.', 0.95, 'test:model'
            );",
        )
        .await
        .expect("seed stranded rows");

    init_sqlite_semantic_features_schema(&db_path)
        .await
        .expect("run schema upgrade");

    let rows = relational
        .query_rows(
            "SELECT artefact_id
             FROM symbol_semantics_current
             WHERE artefact_id = 'artefact-1'",
        )
        .await
        .expect("query repaired rows");
    assert_eq!(rows.len(), 1);
}

#[tokio::test]
async fn skipped_fresh_historical_semantics_repair_missing_current_projection() {
    let relational = sqlite_relational_with_current_projection_schema().await;
    let input = sample_semantic_input("artefact-1", "content-1");
    let provider = Arc::new(TestSummaryProvider);
    let input_hash = semantic::build_semantic_feature_input_hash(&input, provider.as_ref());

    relational
        .exec(&format!(
            "INSERT INTO artefacts_current (
                repo_id, path, content_id, symbol_id, artefact_id, language,
                canonical_kind, language_kind, symbol_fqn, start_line, end_line,
                start_byte, end_byte, modifiers, updated_at
            ) VALUES (
                'repo-1', 'src/lib.rs', 'content-1', 'symbol-artefact-1', 'artefact-1', 'rust',
                'function', 'function', 'src/lib.rs::artefact-1', 1, 3, 0, 24, '[]', datetime('now')
            );
            INSERT INTO current_file_state (repo_id, path, analysis_mode, effective_content_id)
            VALUES ('repo-1', 'src/lib.rs', 'code', 'content-1');
            INSERT INTO symbol_features (
                artefact_id, repo_id, blob_sha, semantic_features_input_hash,
                normalized_name, normalized_signature, modifiers, identifier_tokens,
                normalized_body_tokens, parent_kind, context_tokens
            ) VALUES (
                'artefact-1', 'repo-1', 'content-1', '{input_hash}',
                'artefact_1', 'fn artefact_1()', '[]', '[]', '[]', NULL, '[]'
            );
            INSERT INTO symbol_semantics (
                artefact_id, repo_id, blob_sha, semantic_features_input_hash,
                docstring_summary, llm_summary, template_summary, summary, confidence, source_model
            ) VALUES (
                'artefact-1', 'repo-1', 'content-1', '{input_hash}',
                NULL, 'Summarises the symbol.', 'Template summary.', 'Summarises the symbol.', 0.95, 'test:model'
            );"
        ))
        .await
        .expect("seed fresh historical rows");

    let stats = upsert_semantic_feature_rows(&relational, &[input], provider)
        .await
        .expect("repair current projection");

    assert_eq!(stats.skipped, 1);
    let current = relational
        .query_rows(
            "SELECT f.semantic_features_input_hash AS feature_hash,
                    s.semantic_features_input_hash AS semantic_hash,
                    s.llm_summary,
                    s.source_model
             FROM symbol_features_current f
             JOIN symbol_semantics_current s
               ON s.repo_id = f.repo_id
              AND s.artefact_id = f.artefact_id
              AND s.content_id = f.content_id
             WHERE f.artefact_id = 'artefact-1'",
        )
        .await
        .expect("load current projection");

    assert_eq!(current.len(), 1);
    assert_eq!(
        current[0].get("feature_hash").and_then(Value::as_str),
        Some(input_hash.as_str())
    );
    assert_eq!(
        current[0].get("semantic_hash").and_then(Value::as_str),
        Some(input_hash.as_str())
    );
    assert_eq!(
        current[0].get("llm_summary").and_then(Value::as_str),
        Some("Summarises the symbol.")
    );
    assert_eq!(
        current[0].get("source_model").and_then(Value::as_str),
        Some("test:model")
    );
}

#[tokio::test]
async fn skipped_fresh_historical_semantics_repair_remapped_current_projection() {
    let relational = sqlite_relational_with_current_projection_schema().await;
    let input = semantic::SemanticFeatureInput {
        artefact_id: "historical-artefact".to_string(),
        symbol_id: Some("historical-symbol".to_string()),
        repo_id: "repo-1".to_string(),
        blob_sha: "content-1".to_string(),
        path: "src/render.ts".to_string(),
        language: "typescript".to_string(),
        canonical_kind: "function".to_string(),
        language_kind: "function".to_string(),
        symbol_fqn: "src/render.ts::renderInvoice".to_string(),
        name: "renderInvoice".to_string(),
        signature: Some("renderInvoice(orderId: string): string".to_string()),
        modifiers: vec!["export".to_string()],
        body: "return orderId;".to_string(),
        docstring: None,
        parent_kind: Some("file".to_string()),
        dependency_signals: Vec::new(),
        content_hash: Some("content-1".to_string()),
    };
    let provider = Arc::new(TestSummaryProvider);
    let input_hash = semantic::build_semantic_feature_input_hash(&input, provider.as_ref());

    relational
        .exec(&format!(
            "INSERT INTO artefacts_current (
                repo_id, path, content_id, symbol_id, artefact_id, language,
                canonical_kind, language_kind, symbol_fqn, start_line, end_line,
                start_byte, end_byte, modifiers, updated_at
            ) VALUES (
                'repo-1', 'src/render.ts', 'content-1', 'current-symbol', 'current-artefact', 'typescript',
                'function', 'function', 'src/render.ts::renderInvoice', 1, 3, 0, 64, '[]', datetime('now')
            );
            INSERT INTO current_file_state (repo_id, path, analysis_mode, effective_content_id)
            VALUES ('repo-1', 'src/render.ts', 'code', 'content-1');
            INSERT INTO symbol_features (
                artefact_id, repo_id, blob_sha, semantic_features_input_hash,
                normalized_name, normalized_signature, modifiers, identifier_tokens,
                normalized_body_tokens, parent_kind, context_tokens
            ) VALUES (
                'historical-artefact', 'repo-1', 'content-1', '{input_hash}',
                'renderinvoice', 'renderInvoice(orderId: string): string', '[\"export\"]',
                '[\"render\", \"invoice\"]', '[\"return\", \"orderId\"]', 'file', '[]'
            );
            INSERT INTO symbol_semantics (
                artefact_id, repo_id, blob_sha, semantic_features_input_hash,
                docstring_summary, llm_summary, template_summary, summary, confidence, source_model
            ) VALUES (
                'historical-artefact', 'repo-1', 'content-1', '{input_hash}',
                NULL, 'Summarises the remapped symbol.', 'Template summary.', 'Summarises the remapped symbol.', 0.95, 'test:model'
            );"
        ))
        .await
        .expect("seed fresh remapped historical rows");

    let stats = upsert_semantic_feature_rows(&relational, &[input], provider)
        .await
        .expect("repair remapped current projection");

    assert_eq!(stats.skipped, 1);
    let current = relational
        .query_rows(
            "SELECT f.artefact_id AS feature_artefact_id,
                    s.artefact_id AS semantic_artefact_id,
                    s.symbol_id,
                    s.llm_summary
             FROM symbol_features_current f
             JOIN symbol_semantics_current s
               ON s.repo_id = f.repo_id
              AND s.artefact_id = f.artefact_id
              AND s.content_id = f.content_id
             WHERE f.repo_id = 'repo-1' AND f.path = 'src/render.ts'",
        )
        .await
        .expect("load remapped current projection");

    assert_eq!(current.len(), 1);
    assert_eq!(
        current[0]
            .get("feature_artefact_id")
            .and_then(Value::as_str),
        Some("current-artefact")
    );
    assert_eq!(
        current[0]
            .get("semantic_artefact_id")
            .and_then(Value::as_str),
        Some("current-artefact")
    );
    assert_eq!(
        current[0].get("symbol_id").and_then(Value::as_str),
        Some("current-symbol")
    );
    assert_eq!(
        current[0].get("llm_summary").and_then(Value::as_str),
        Some("Summarises the remapped symbol.")
    );
}

#[tokio::test]
async fn historical_semantic_upsert_scopes_current_projection_matching_by_repo() {
    let relational = sqlite_relational_with_current_projection_schema().await;
    relational
        .exec(
            "INSERT INTO artefacts_current (
                repo_id, path, content_id, symbol_id, artefact_id, language,
                canonical_kind, language_kind, symbol_fqn, start_line, end_line,
                start_byte, end_byte, modifiers, updated_at
            ) VALUES
            (
                'repo-1', 'src/lib.rs', 'content-1', 'current-symbol-1', 'current-artefact-1', 'rust',
                'function', 'function', 'src/lib.rs::shared', 1, 3, 0, 24, '[]', datetime('now')
            ),
            (
                'repo-2', 'src/lib.rs', 'content-1', 'current-symbol-2', 'current-artefact-2', 'rust',
                'function', 'function', 'src/lib.rs::shared', 1, 3, 0, 24, '[]', datetime('now')
            );
            INSERT INTO current_file_state (repo_id, path, analysis_mode, effective_content_id)
            VALUES
            ('repo-1', 'src/lib.rs', 'code', 'content-1'),
            ('repo-2', 'src/lib.rs', 'code', 'content-1');",
        )
        .await
        .expect("seed repo-scoped current projection rows");

    let mut first = sample_semantic_input("historical-artefact-1", "content-1");
    first.symbol_id = Some("historical-symbol-1".to_string());
    first.symbol_fqn = "src/lib.rs::shared".to_string();
    first.name = "shared".to_string();
    first.signature = Some("fn shared()".to_string());

    let mut second = sample_semantic_input("historical-artefact-2", "content-1");
    second.symbol_id = Some("historical-symbol-2".to_string());
    second.repo_id = "repo-2".to_string();
    second.symbol_fqn = "src/lib.rs::shared".to_string();
    second.name = "shared".to_string();
    second.signature = Some("fn shared()".to_string());

    let stats =
        upsert_semantic_feature_rows(&relational, &[first, second], Arc::new(TestSummaryProvider))
            .await
            .expect("upsert historical semantic rows for multiple repos");

    assert_eq!(stats.upserted, 2);
    let rows = relational
        .query_rows(
            "SELECT artefact_id, repo_id, symbol_id
             FROM symbol_semantics_current
             ORDER BY repo_id",
        )
        .await
        .expect("load repo-scoped mirrored current rows");
    assert_eq!(rows.len(), 2);
    assert_eq!(
        rows[0].get("artefact_id").and_then(Value::as_str),
        Some("current-artefact-1")
    );
    assert_eq!(
        rows[0].get("repo_id").and_then(Value::as_str),
        Some("repo-1")
    );
    assert_eq!(
        rows[0].get("symbol_id").and_then(Value::as_str),
        Some("current-symbol-1")
    );
    assert_eq!(
        rows[1].get("artefact_id").and_then(Value::as_str),
        Some("current-artefact-2")
    );
    assert_eq!(
        rows[1].get("repo_id").and_then(Value::as_str),
        Some("repo-2")
    );
    assert_eq!(
        rows[1].get("symbol_id").and_then(Value::as_str),
        Some("current-symbol-2")
    );
}

#[tokio::test]
async fn historical_semantic_upsert_mirrors_into_current_projection_with_current_sync_ids() {
    let relational = sqlite_relational_with_current_projection_schema().await;
    relational
        .exec(
            "INSERT INTO artefacts_current (
                repo_id, path, content_id, symbol_id, artefact_id, language,
                canonical_kind, language_kind, symbol_fqn, start_line, end_line,
                start_byte, end_byte, modifiers, updated_at
            ) VALUES (
                'repo-1', 'src/render.ts', 'content-1', 'current-symbol', 'current-artefact', 'typescript',
                'function', 'function', 'src/render.ts::renderInvoice', 1, 3, 0, 64, '[]', datetime('now')
            );
            INSERT INTO current_file_state (repo_id, path, analysis_mode, effective_content_id)
            VALUES ('repo-1', 'src/render.ts', 'code', 'content-1');",
        )
        .await
        .expect("seed current projection rows");

    let stats = upsert_semantic_feature_rows(
        &relational,
        &[semantic::SemanticFeatureInput {
            artefact_id: "historical-artefact".to_string(),
            symbol_id: Some("historical-symbol".to_string()),
            repo_id: "repo-1".to_string(),
            blob_sha: "content-1".to_string(),
            path: "src/render.ts".to_string(),
            language: "typescript".to_string(),
            canonical_kind: "function".to_string(),
            language_kind: "function".to_string(),
            symbol_fqn: "src/render.ts::renderInvoice".to_string(),
            name: "renderInvoice".to_string(),
            signature: Some("renderInvoice(orderId: string): string".to_string()),
            modifiers: vec!["export".to_string()],
            body: "return orderId;".to_string(),
            docstring: None,
            parent_kind: Some("file".to_string()),
            dependency_signals: Vec::new(),
            content_hash: Some("content-1".to_string()),
        }],
        Arc::new(TestSummaryProvider),
    )
    .await
    .expect("upsert historical semantic rows");

    assert_eq!(stats.upserted, 1);
    let rows = relational
        .query_rows(
            "SELECT artefact_id, symbol_id, llm_summary, source_model, content_id
             FROM symbol_semantics_current
             WHERE path = 'src/render.ts'",
        )
        .await
        .expect("load mirrored current summary row");
    assert_eq!(rows.len(), 1);
    assert_eq!(
        rows[0].get("artefact_id").and_then(Value::as_str),
        Some("current-artefact")
    );
    assert_eq!(
        rows[0].get("symbol_id").and_then(Value::as_str),
        Some("current-symbol")
    );
    assert_eq!(
        rows[0].get("llm_summary").and_then(Value::as_str),
        Some("Summarises the symbol.")
    );
    assert_eq!(
        rows[0].get("source_model").and_then(Value::as_str),
        Some("ollama:ministral-3:3b")
    );
    assert_eq!(
        rows[0].get("content_id").and_then(Value::as_str),
        Some("content-1")
    );
}

#[tokio::test]
async fn conditional_current_semantic_persist_skips_missing_live_target() {
    let relational = sqlite_relational_with_current_projection_schema().await;
    relational
        .exec(
            "INSERT INTO artefacts_current (
                repo_id, path, content_id, symbol_id, artefact_id, language,
                canonical_kind, language_kind, symbol_fqn, start_line, end_line,
                start_byte, end_byte, modifiers, updated_at
            ) VALUES (
                'repo-1', 'src/lib.rs', 'content-1', 'symbol-artefact-1', 'artefact-1', 'rust',
                'function', 'function', 'src/lib.rs::artefact-1', 1, 3, 0, 24, '[]', datetime('now')
            );
            INSERT INTO current_file_state (repo_id, path, analysis_mode, effective_content_id)
            VALUES ('repo-1', 'src/lib.rs', 'code', 'content-1');
            DELETE FROM artefacts_current WHERE repo_id = 'repo-1' AND path = 'src/lib.rs';",
        )
        .await
        .expect("seed and delete current projection rows");

    let input = sample_semantic_input("artefact-1", "content-1");
    let rows = semantic::build_semantic_feature_rows(&input, &TestSummaryProvider);
    persist_current_semantic_feature_rows_for_matching_input(&relational, &input, &rows)
        .await
        .expect("skip missing current target");

    let rows = relational
        .query_rows("SELECT artefact_id FROM symbol_semantics_current")
        .await
        .expect("load current summary rows");
    assert!(
        rows.is_empty(),
        "current semantic projection should not be recreated after the live target disappears"
    );
}

#[test]
fn remap_semantic_input_to_current_artefact_uses_current_sync_ids() {
    let current = semantic::PreStageArtefactRow {
        artefact_id: "current-artefact".to_string(),
        symbol_id: Some("current-symbol".to_string()),
        repo_id: "repo-1".to_string(),
        blob_sha: "blob-1".to_string(),
        path: "src/render.ts".to_string(),
        language: "typescript".to_string(),
        canonical_kind: "function".to_string(),
        language_kind: "function".to_string(),
        symbol_fqn: "src/render.ts::renderInvoice".to_string(),
        parent_artefact_id: None,
        start_line: Some(1),
        end_line: Some(3),
        start_byte: Some(0),
        end_byte: Some(64),
        signature: Some("renderInvoice(orderId: string): string".to_string()),
        modifiers: vec!["export".to_string()],
        docstring: None,
        content_hash: Some("hash-1".to_string()),
    };
    let historical = semantic::SemanticFeatureInput {
        artefact_id: "historical-artefact".to_string(),
        symbol_id: Some("historical-symbol".to_string()),
        repo_id: "repo-1".to_string(),
        blob_sha: "blob-1".to_string(),
        path: "src/render.ts".to_string(),
        language: "typescript".to_string(),
        canonical_kind: "function".to_string(),
        language_kind: "function".to_string(),
        symbol_fqn: "src/render.ts::renderInvoice".to_string(),
        name: "renderInvoice".to_string(),
        signature: Some("renderInvoice(orderId: string): string".to_string()),
        modifiers: vec!["export".to_string()],
        body: "return orderId;".to_string(),
        docstring: None,
        parent_kind: Some("file".to_string()),
        dependency_signals: Vec::new(),
        content_hash: Some("hash-1".to_string()),
    };
    let current_by_key = HashMap::from([(
        current_semantic_artefact_key_from_row(&current),
        current.clone(),
    )]);

    let remapped = remap_semantic_input_to_current_artefact(historical, &current_by_key)
        .expect("expected current artefact match");

    assert_eq!(remapped.artefact_id, current.artefact_id);
    assert_eq!(remapped.symbol_id, current.symbol_id);
    assert_eq!(remapped.symbol_fqn, current.symbol_fqn);
}
