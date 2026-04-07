use crate::host::devql::sync::content_cache::{CachedArtefact, CachedEdge, CachedExtraction};
use crate::host::devql::sync::types::{DesiredFileState, EffectiveSource};
use crate::host::language_adapter::{GoKind, JavaKind, LanguageKind, TsJsKind};
use rusqlite::Connection;
use serde_json::json;
use tempfile::tempdir;

use super::{materialize_path, parse_cached_language_kind};

fn test_cfg(repo_root: &std::path::Path) -> crate::host::devql::DevqlConfig {
    crate::host::devql::DevqlConfig {
        daemon_config_root: repo_root.to_path_buf(),
        repo_root: repo_root.to_path_buf(),
        repo: crate::host::devql::RepoIdentity {
            provider: "github".to_string(),
            organization: "bitloops".to_string(),
            name: "materializer-test".to_string(),
            identity: "github/bitloops/materializer-test".to_string(),
            repo_id: crate::host::devql::deterministic_uuid(&format!(
                "repo://{}",
                repo_root.display()
            )),
        },
        pg_dsn: None,
        clickhouse_url: "http://localhost:8123".to_string(),
        clickhouse_user: None,
        clickhouse_password: None,
        clickhouse_database: "default".to_string(),
        semantic_provider: None,
        semantic_model: None,
        semantic_api_key: None,
        semantic_base_url: None,
    }
}

async fn create_test_relational() -> crate::host::devql::RelationalStorage {
    let temp = tempdir().expect("temp dir");
    let sqlite_path = temp.path().join("devql.sqlite");
    crate::host::devql::init_sqlite_schema(&sqlite_path)
        .await
        .expect("initialise sqlite relational schema");
    let sqlite_path = temp.keep().join("devql.sqlite");
    crate::host::devql::RelationalStorage::local_only(sqlite_path)
}

async fn seed_test_repository_catalog_row(
    cfg: &crate::host::devql::DevqlConfig,
    relational: &crate::host::devql::RelationalStorage,
) {
    relational
        .exec(&format!(
            "INSERT INTO repositories (repo_id, provider, organization, name, default_branch) \
             VALUES ('{}', '{}', '{}', '{}', 'main') \
             ON CONFLICT(repo_id) DO UPDATE SET \
               provider = excluded.provider, \
               organization = excluded.organization, \
               name = excluded.name, \
               default_branch = excluded.default_branch",
            crate::host::devql::db_utils::esc_pg(&cfg.repo.repo_id),
            crate::host::devql::db_utils::esc_pg(&cfg.repo.provider),
            crate::host::devql::db_utils::esc_pg(&cfg.repo.organization),
            crate::host::devql::db_utils::esc_pg(&cfg.repo.name),
        ))
        .await
        .expect("seed repository catalog row");
}

#[tokio::test]
async fn materialize_path_deduplicates_colliding_artefact_ids() {
    let repo_root = tempdir().expect("temp dir").keep();
    let cfg = test_cfg(&repo_root);
    let relational = create_test_relational().await;
    seed_test_repository_catalog_row(&cfg, &relational).await;
    let path = "src/lib.rs";
    let content = "pub fn greet() {}\n";
    let content_id =
        crate::host::devql::sync::content_identity::compute_blob_oid(content.as_bytes());
    let desired = DesiredFileState {
        path: path.to_string(),
        language: "rust".to_string(),
        head_content_id: Some(content_id.clone()),
        index_content_id: Some(content_id.clone()),
        worktree_content_id: Some(content_id.clone()),
        effective_content_id: content_id.clone(),
        effective_source: EffectiveSource::Head,
        exists_in_head: true,
        exists_in_index: true,
        exists_in_worktree: true,
    };
    let extraction = CachedExtraction {
        content_id: content_id.clone(),
        language: "rust".to_string(),
        parser_version: "parser-v1".to_string(),
        extractor_version: "extractor-v1".to_string(),
        parse_status: "ok".to_string(),
        artefacts: vec![
            CachedArtefact {
                artifact_key: "file::a-old".to_string(),
                canonical_kind: Some("file".to_string()),
                language_kind: "file".to_string(),
                name: path.to_string(),
                parent_artifact_key: None,
                start_line: 1,
                end_line: 1,
                start_byte: 0,
                end_byte: 8,
                signature: "old signature".to_string(),
                modifiers: vec![],
                docstring: Some("old".to_string()),
                metadata: json!({"variant": "old"}),
            },
            CachedArtefact {
                artifact_key: "file::z-new".to_string(),
                canonical_kind: Some("file".to_string()),
                language_kind: "file".to_string(),
                name: path.to_string(),
                parent_artifact_key: None,
                start_line: 2,
                end_line: 2,
                start_byte: 9,
                end_byte: 18,
                signature: "new signature".to_string(),
                modifiers: vec!["pub".to_string()],
                docstring: Some("new".to_string()),
                metadata: json!({"variant": "new"}),
            },
        ],
        edges: vec![],
    };

    materialize_path(
        &cfg,
        &relational,
        &desired,
        &extraction,
        "parser-v1",
        "extractor-v1",
    )
    .await
    .expect("materialize colliding artefacts");

    let db = Connection::open(&relational.local.path).expect("open sqlite db");
    let row_count: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM artefacts_current WHERE repo_id = ?1 AND path = ?2",
            [cfg.repo.repo_id.as_str(), path],
            |row| row.get(0),
        )
        .expect("count deduplicated artefacts_current rows");
    let row: (i32, i32, String, String) = db
        .query_row(
            "SELECT start_line, end_line, signature, docstring \
             FROM artefacts_current \
             WHERE repo_id = ?1 AND path = ?2",
            [cfg.repo.repo_id.as_str(), path],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .expect("read deduplicated artefacts_current row");

    assert_eq!(row_count, 1);
    assert_eq!(row.0, 2);
    assert_eq!(row.1, 2);
    assert_eq!(row.2, "new signature");
    assert_eq!(row.3, "new");
}

#[tokio::test]
async fn materialize_path_deduplicates_colliding_edge_ids() {
    let repo_root = tempdir().expect("temp dir").keep();
    let cfg = test_cfg(&repo_root);
    let relational = create_test_relational().await;
    seed_test_repository_catalog_row(&cfg, &relational).await;
    let path = "src/lib.rs";
    let content = "pub fn greet() {}\n";
    let content_id =
        crate::host::devql::sync::content_identity::compute_blob_oid(content.as_bytes());
    let desired = DesiredFileState {
        path: path.to_string(),
        language: "rust".to_string(),
        head_content_id: Some(content_id.clone()),
        index_content_id: Some(content_id.clone()),
        worktree_content_id: Some(content_id.clone()),
        effective_content_id: content_id.clone(),
        effective_source: EffectiveSource::Head,
        exists_in_head: true,
        exists_in_index: true,
        exists_in_worktree: true,
    };
    let extraction = CachedExtraction {
        content_id: content_id.clone(),
        language: "rust".to_string(),
        parser_version: "parser-v1".to_string(),
        extractor_version: "extractor-v1".to_string(),
        parse_status: "ok".to_string(),
        artefacts: vec![CachedArtefact {
            artifact_key: "file::src/lib.rs".to_string(),
            canonical_kind: Some("file".to_string()),
            language_kind: "file".to_string(),
            name: path.to_string(),
            parent_artifact_key: None,
            start_line: 1,
            end_line: 1,
            start_byte: 0,
            end_byte: 8,
            signature: "fn greet()".to_string(),
            modifiers: vec![],
            docstring: Some("greet".to_string()),
            metadata: json!({"variant": "shared"}),
        }],
        edges: vec![
            CachedEdge {
                edge_key: "edge::call::old".to_string(),
                from_artifact_key: "file::src/lib.rs".to_string(),
                to_artifact_key: None,
                to_symbol_ref: Some("target::symbol".to_string()),
                edge_kind: "calls".to_string(),
                start_line: Some(1),
                end_line: Some(1),
                metadata: json!({"variant": "shared"}),
            },
            CachedEdge {
                edge_key: "edge::call::new".to_string(),
                from_artifact_key: "file::src/lib.rs".to_string(),
                to_artifact_key: None,
                to_symbol_ref: Some("target::symbol".to_string()),
                edge_kind: "calls".to_string(),
                start_line: Some(1),
                end_line: Some(1),
                metadata: json!({"variant": "shared"}),
            },
        ],
    };

    materialize_path(
        &cfg,
        &relational,
        &desired,
        &extraction,
        "parser-v1",
        "extractor-v1",
    )
    .await
    .expect("materialize colliding edges");

    let db = Connection::open(&relational.local.path).expect("open sqlite db");
    let row_count: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM artefact_edges_current WHERE repo_id = ?1 AND path = ?2",
            [cfg.repo.repo_id.as_str(), path],
            |row| row.get(0),
        )
        .expect("count deduplicated artefact_edges_current rows");
    let edge_id: String = db
        .query_row(
            "SELECT edge_id FROM artefact_edges_current WHERE repo_id = ?1 AND path = ?2",
            [cfg.repo.repo_id.as_str(), path],
            |row| row.get(0),
        )
        .expect("read deduplicated edge_id");

    assert_eq!(row_count, 1);
    assert!(!edge_id.is_empty());
}

#[test]
fn parse_cached_language_kind_uses_language_specific_resolution_for_ambiguous_kinds() {
    assert_eq!(
        parse_cached_language_kind("typescript", "function_declaration").expect("parse ts kind"),
        LanguageKind::ts_js(TsJsKind::FunctionDeclaration)
    );
    assert_eq!(
        parse_cached_language_kind("go", "function_declaration").expect("parse go kind"),
        LanguageKind::go(GoKind::FunctionDeclaration)
    );
    assert_eq!(
        parse_cached_language_kind("java", "class_declaration").expect("parse java kind"),
        LanguageKind::java(JavaKind::Class)
    );
}
