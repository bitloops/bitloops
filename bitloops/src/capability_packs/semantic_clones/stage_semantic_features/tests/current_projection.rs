use std::collections::HashMap;
use std::sync::Arc;

use serde_json::Value;

use super::super::hydration::{
    current_semantic_artefact_key_from_row, remap_semantic_input_to_current_artefact,
};
use super::super::persistence::persist_current_semantic_feature_rows_for_matching_input;
use super::super::upsert_semantic_feature_rows;
use super::support::{
    TestSummaryProvider, sample_semantic_input, sqlite_relational_with_current_projection_schema,
};
use crate::capability_packs::semantic_clones::features as semantic;

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
            INSERT INTO current_file_state (repo_id, path, analysis_mode)
            VALUES ('repo-1', 'src/lib.rs', 'code');",
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
            INSERT INTO current_file_state (repo_id, path, analysis_mode)
            VALUES ('repo-1', 'src/lib.rs', 'code');",
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
            INSERT INTO current_file_state (repo_id, path, analysis_mode)
            VALUES
            ('repo-1', 'src/lib.rs', 'code'),
            ('repo-2', 'src/lib.rs', 'code');",
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
            INSERT INTO current_file_state (repo_id, path, analysis_mode)
            VALUES ('repo-1', 'src/render.ts', 'code');",
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
            INSERT INTO current_file_state (repo_id, path, analysis_mode)
            VALUES ('repo-1', 'src/lib.rs', 'code');
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
