use serde_json::Value;
use tempfile::tempdir;

use crate::host::devql::RelationalStorage;
use crate::host::devql::db_utils::esc_pg;

use super::{
    CachedArtefact, CachedEdge, CachedExtraction, lookup_cached_content, promote_to_git_backed,
    store_cached_content,
};

async fn create_test_relational() -> RelationalStorage {
    let temp = tempdir().expect("temp dir");
    let sqlite_path = temp.path().join("devql.sqlite");
    crate::host::devql::init_sqlite_schema(&sqlite_path)
        .await
        .expect("initialise sqlite relational schema");
    let sqlite_path = temp.keep().join("devql.sqlite");
    RelationalStorage::local_only(sqlite_path)
}

async fn retention_class_for(relational: &RelationalStorage, content_id: &str) -> Option<String> {
    let sql = format!(
        "SELECT retention_class FROM content_cache \
WHERE content_id = '{}' AND language = 'rust' AND parser_version = 'parser-v1' AND extractor_version = 'extractor-v1'",
        esc_pg(content_id),
    );
    relational
        .query_rows(&sql)
        .await
        .expect("query content_cache")
        .first()
        .and_then(Value::as_object)
        .and_then(|row| row.get("retention_class"))
        .and_then(Value::as_str)
        .map(str::to_string)
}

async fn count_rows(relational: &RelationalStorage, table: &str, content_id: &str) -> usize {
    let sql = format!(
        "SELECT COUNT(*) AS count FROM {} \
WHERE content_id = '{}' AND language = 'rust' AND parser_version = 'parser-v1' AND extractor_version = 'extractor-v1'",
        table,
        esc_pg(content_id),
    );
    relational
        .query_rows(&sql)
        .await
        .expect("query row count")
        .first()
        .and_then(Value::as_object)
        .and_then(|row| row.get("count"))
        .and_then(Value::as_i64)
        .unwrap_or_default() as usize
}

#[tokio::test]
async fn worktree_only_promoted_to_git_backed_when_seen_as_head_blob() {
    let relational = create_test_relational().await;
    let extraction = CachedExtraction {
        content_id: "abc123".to_string(),
        language: "rust".to_string(),
        extraction_fingerprint: "fingerprint-v1".to_string(),
        parser_version: "parser-v1".to_string(),
        extractor_version: "extractor-v1".to_string(),
        parse_status: "ok".to_string(),
        artefacts: vec![],
        edges: vec![],
    };

    store_cached_content(&relational, &extraction, "worktree_only")
        .await
        .expect("store worktree-only cache entry");
    assert_eq!(
        retention_class_for(&relational, &extraction.content_id)
            .await
            .as_deref(),
        Some("worktree_only")
    );

    promote_to_git_backed(
        &relational,
        &extraction.content_id,
        &extraction.language,
        &extraction.extraction_fingerprint,
        &extraction.parser_version,
        &extraction.extractor_version,
    )
    .await
    .expect("promote cache entry");

    assert_eq!(
        retention_class_for(&relational, &extraction.content_id)
            .await
            .as_deref(),
        Some("git_backed")
    );
}

#[tokio::test]
async fn store_cached_content_deduplicates_duplicate_keys() {
    let relational = create_test_relational().await;
    let extraction = CachedExtraction {
        content_id: "abc123".to_string(),
        language: "rust".to_string(),
        extraction_fingerprint: "fingerprint-v1".to_string(),
        parser_version: "parser-v1".to_string(),
        extractor_version: "extractor-v1".to_string(),
        parse_status: "ok".to_string(),
        artefacts: vec![
            CachedArtefact {
                artifact_key: "file::src/lib.rs".to_string(),
                canonical_kind: Some("file".to_string()),
                language_kind: "file".to_string(),
                name: "src/lib.rs".to_string(),
                parent_artifact_key: None,
                start_line: 1,
                end_line: 2,
                start_byte: 0,
                end_byte: 10,
                signature: "fn old()".to_string(),
                modifiers: vec!["pub".to_string()],
                docstring: Some("old".to_string()),
                metadata: Value::String("old".to_string()),
            },
            CachedArtefact {
                artifact_key: "file::src/lib.rs".to_string(),
                canonical_kind: Some("file".to_string()),
                language_kind: "file".to_string(),
                name: "src/lib.rs".to_string(),
                parent_artifact_key: None,
                start_line: 3,
                end_line: 4,
                start_byte: 11,
                end_byte: 20,
                signature: "fn new()".to_string(),
                modifiers: vec!["pub".to_string(), "async".to_string()],
                docstring: Some("new".to_string()),
                metadata: Value::String("new".to_string()),
            },
        ],
        edges: vec![
            CachedEdge {
                edge_key: "edge::call".to_string(),
                from_artifact_key: "file::src/lib.rs".to_string(),
                to_artifact_key: None,
                to_symbol_ref: Some("old::target".to_string()),
                edge_kind: "calls".to_string(),
                start_line: Some(1),
                end_line: Some(1),
                metadata: Value::String("old".to_string()),
            },
            CachedEdge {
                edge_key: "edge::call".to_string(),
                from_artifact_key: "file::src/lib.rs".to_string(),
                to_artifact_key: Some("file::target".to_string()),
                to_symbol_ref: Some("new::target".to_string()),
                edge_kind: "calls".to_string(),
                start_line: Some(2),
                end_line: Some(2),
                metadata: Value::String("new".to_string()),
            },
        ],
    };

    store_cached_content(&relational, &extraction, "git_backed")
        .await
        .expect("store deduplicated cache entry");

    assert_eq!(
        count_rows(
            &relational,
            "content_cache_artefacts",
            &extraction.content_id
        )
        .await,
        1
    );
    assert_eq!(
        count_rows(&relational, "content_cache_edges", &extraction.content_id).await,
        1
    );

    let cached = lookup_cached_content(
        &relational,
        &extraction.content_id,
        &extraction.language,
        &extraction.extraction_fingerprint,
        &extraction.parser_version,
        &extraction.extractor_version,
    )
    .await
    .expect("lookup stored cache entry")
    .expect("cache entry should exist");

    assert_eq!(cached.artefacts.len(), 1);
    assert_eq!(cached.edges.len(), 1);
    assert_eq!(cached.artefacts[0].start_line, 3);
    assert_eq!(cached.artefacts[0].end_line, 4);
    assert_eq!(cached.artefacts[0].signature, "fn new()");
    assert_eq!(cached.artefacts[0].docstring.as_deref(), Some("new"));
    assert_eq!(
        cached.artefacts[0].metadata,
        Value::String("new".to_string())
    );
    assert_eq!(
        cached.edges[0].to_artifact_key.as_deref(),
        Some("file::target")
    );
    assert_eq!(
        cached.edges[0].to_symbol_ref.as_deref(),
        Some("new::target")
    );
    assert_eq!(cached.edges[0].start_line, Some(2));
    assert_eq!(cached.edges[0].metadata, Value::String("new".to_string()));
}
