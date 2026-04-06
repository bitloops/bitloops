use serde_json::json;
use tempfile::tempdir;

use super::fixtures::sqlite_relational_store_with_sync_schema;

#[tokio::test]
async fn content_cache_lookup_returns_none_on_cache_miss() {
    let temp = tempdir().expect("temp dir");
    let sqlite_path = temp.path().join("devql.sqlite");
    let relational = sqlite_relational_store_with_sync_schema(&sqlite_path).await;

    let cached = crate::host::devql::sync::content_cache::lookup_cached_content(
        &relational,
        "content-1",
        "rust",
        "parser-v1",
        "extractor-v1",
    )
    .await
    .expect("lookup cache entry");

    assert_eq!(cached, None);
}

#[tokio::test]
async fn content_cache_store_then_lookup_roundtrips_payload() {
    let temp = tempdir().expect("temp dir");
    let sqlite_path = temp.path().join("devql.sqlite");
    let relational = sqlite_relational_store_with_sync_schema(&sqlite_path).await;
    let expected = crate::host::devql::sync::content_cache::CachedExtraction {
        content_id: "content-1".to_string(),
        language: "rust".to_string(),
        parser_version: "parser-v1".to_string(),
        extractor_version: "extractor-v1".to_string(),
        parse_status: "ok".to_string(),
        artefacts: vec![crate::host::devql::sync::content_cache::CachedArtefact {
            artifact_key: "file::src/lib.rs".to_string(),
            canonical_kind: Some("file".to_string()),
            language_kind: "file".to_string(),
            name: "src/lib.rs".to_string(),
            parent_artifact_key: None,
            start_line: 1,
            end_line: 3,
            start_byte: 0,
            end_byte: 48,
            signature: "pub fn greet(name: &str) -> String".to_string(),
            modifiers: vec!["pub".to_string()],
            docstring: Some("Greets a caller.".to_string()),
            metadata: json!({ "symbol_fqn": "src/lib.rs" }),
        }],
        edges: vec![crate::host::devql::sync::content_cache::CachedEdge {
            edge_key: "edge::call".to_string(),
            from_artifact_key: "file::src/lib.rs".to_string(),
            to_artifact_key: None,
            to_symbol_ref: Some("std::fmt::format".to_string()),
            edge_kind: "calls".to_string(),
            start_line: Some(2),
            end_line: Some(2),
            metadata: json!({ "call_form": "macro" }),
        }],
    };

    crate::host::devql::sync::content_cache::store_cached_content(&relational, &expected, "hot")
        .await
        .expect("store cache entry");

    let cached = crate::host::devql::sync::content_cache::lookup_cached_content(
        &relational,
        &expected.content_id,
        &expected.language,
        &expected.parser_version,
        &expected.extractor_version,
    )
    .await
    .expect("lookup stored cache entry")
    .expect("cache entry should exist");

    assert_eq!(cached.content_id, expected.content_id);
    assert_eq!(cached.language, expected.language);
    assert_eq!(cached.parse_status, expected.parse_status);
    assert_eq!(cached.artefacts, expected.artefacts);
    assert_eq!(cached.edges, expected.edges);
}

#[tokio::test]
async fn content_cache_lookup_respects_parser_and_extractor_versions() {
    let temp = tempdir().expect("temp dir");
    let sqlite_path = temp.path().join("devql.sqlite");
    let relational = sqlite_relational_store_with_sync_schema(&sqlite_path).await;
    let extraction = crate::host::devql::sync::content_cache::CachedExtraction {
        content_id: "content-versions".to_string(),
        language: "rust".to_string(),
        parser_version: "parser-a".to_string(),
        extractor_version: "extractor-a".to_string(),
        parse_status: "ok".to_string(),
        artefacts: vec![crate::host::devql::sync::content_cache::CachedArtefact {
            artifact_key: "fn::demo".to_string(),
            canonical_kind: Some("function".to_string()),
            language_kind: "function_item".to_string(),
            name: "demo".to_string(),
            parent_artifact_key: None,
            start_line: 1,
            end_line: 1,
            start_byte: 0,
            end_byte: 12,
            signature: "fn demo()".to_string(),
            modifiers: vec![],
            docstring: None,
            metadata: json!({}),
        }],
        edges: vec![],
    };

    crate::host::devql::sync::content_cache::store_cached_content(&relational, &extraction, "hot")
        .await
        .expect("store versioned cache entry");

    let version_a = crate::host::devql::sync::content_cache::lookup_cached_content(
        &relational,
        &extraction.content_id,
        &extraction.language,
        &extraction.parser_version,
        &extraction.extractor_version,
    )
    .await
    .expect("lookup version a");

    let version_b = crate::host::devql::sync::content_cache::lookup_cached_content(
        &relational,
        &extraction.content_id,
        &extraction.language,
        "parser-b",
        "extractor-b",
    )
    .await
    .expect("lookup version b");

    assert_eq!(version_a, Some(extraction));
    assert_eq!(version_b, None);
}
