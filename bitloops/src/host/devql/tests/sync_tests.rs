use rusqlite::Connection;
use serde_json::json;
use std::fs;
use std::path::Path;
use tempfile::tempdir;

async fn sqlite_relational_store_with_sync_schema(
    path: &Path,
) -> crate::host::devql::RelationalStorage {
    crate::host::devql::init_sqlite_schema(path)
        .await
        .expect("initialise sqlite relational schema");
    crate::host::devql::RelationalStorage::local_only(path.to_path_buf())
}

fn sync_test_cfg() -> crate::host::devql::DevqlConfig {
    crate::host::devql::DevqlConfig {
        config_root: std::path::PathBuf::from("/tmp/repo"),
        repo_root: std::path::PathBuf::from("/tmp/repo"),
        repo: crate::host::devql::RepoIdentity {
            provider: "github".to_string(),
            organization: "bitloops".to_string(),
            name: "temp2".to_string(),
            identity: "github/bitloops/temp2".to_string(),
            repo_id: crate::host::devql::deterministic_uuid("repo://github/bitloops/temp2"),
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
        embedding_provider: None,
        embedding_model: None,
        embedding_api_key: None,
        embedding_cache_dir: None,
    }
}

fn desired_file_state(
    path: &str,
    language: &str,
    content_id: &str,
) -> crate::host::devql::sync::types::DesiredFileState {
    crate::host::devql::sync::types::DesiredFileState {
        path: path.to_string(),
        language: language.to_string(),
        head_content_id: Some(content_id.to_string()),
        index_content_id: Some(content_id.to_string()),
        worktree_content_id: Some(content_id.to_string()),
        effective_content_id: content_id.to_string(),
        effective_source: crate::host::devql::sync::types::EffectiveSource::Head,
        exists_in_head: true,
        exists_in_index: true,
        exists_in_worktree: true,
    }
}

fn expected_symbol_id_by_fqn(
    items: &[crate::host::language_adapter::LanguageArtefact],
    path: &str,
) -> std::collections::HashMap<String, String> {
    let mut symbol_ids = std::collections::HashMap::from([(
        path.to_string(),
        crate::host::devql::file_symbol_id(path),
    )]);

    for item in items {
        let parent_symbol_id = item
            .parent_symbol_fqn
            .as_ref()
            .and_then(|fqn| symbol_ids.get(fqn))
            .map(String::as_str);
        let symbol_id =
            crate::host::devql::structural_symbol_id_for_artefact(item, parent_symbol_id);
        symbol_ids.insert(item.symbol_fqn.clone(), symbol_id);
    }

    symbol_ids
}

#[tokio::test]
async fn sync_schema_creates_all_tables() {
    let temp = tempdir().expect("temp dir");
    let db_path = temp.path().join("devql.sqlite");
    let db = Connection::open(&db_path).expect("open sqlite db");
    db.execute_batch(
        r#"
CREATE TABLE current_file_state (
    repo_id TEXT NOT NULL,
    path TEXT NOT NULL,
    commit_sha TEXT NOT NULL,
    blob_sha TEXT NOT NULL,
    committed_at TEXT NOT NULL,
    updated_at TEXT DEFAULT (datetime('now')),
    PRIMARY KEY (repo_id, path)
);
"#,
    )
    .expect("seed legacy current_file_state");
    drop(db);

    crate::host::devql::init_sqlite_schema(&db_path)
        .await
        .expect("initialise sqlite relational schema");

    let db = Connection::open(&db_path).expect("open sqlite db");

    for table in &[
        "repo_sync_state",
        "current_file_state",
        "content_cache",
        "content_cache_artefacts",
        "content_cache_edges",
    ] {
        let count: i64 = db
            .query_row(
                &format!(
                    "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='{table}'"
                ),
                [],
                |row| row.get(0),
            )
            .expect("read sqlite_master");
        assert_eq!(count, 1, "table {table} should exist");
    }

    let legacy_sync_state_count: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='sync_state'",
            [],
            |row| row.get(0),
        )
        .expect("read sqlite_master for sync_state");
    assert_eq!(
        legacy_sync_state_count, 1,
        "legacy sync_state table should still exist"
    );

    for column in &[
        "head_content_id",
        "index_content_id",
        "worktree_content_id",
        "effective_content_id",
        "effective_source",
        "parser_version",
        "extractor_version",
        "exists_in_head",
        "exists_in_index",
        "exists_in_worktree",
        "last_synced_at",
    ] {
        let column_count: i64 = db
            .query_row(
                "SELECT COUNT(*) \
                 FROM pragma_table_info('current_file_state') \
                 WHERE name = ?1",
                [column],
                |row| row.get(0),
            )
            .expect("read pragma_table_info");
        assert_eq!(
            column_count, 1,
            "column {column} should exist on current_file_state"
        );
    }
}

#[test]
fn sync_artefacts_current_migration_sql_recreates_current_state_tables() {
    let sql = crate::host::devql::sync::schema::sync_artefacts_current_migration_sql();

    assert!(sql.contains("DROP TABLE IF EXISTS artefacts_current;"));
    assert!(sql.contains("DROP TABLE IF EXISTS artefact_edges_current;"));

    assert!(sql.contains("CREATE TABLE IF NOT EXISTS artefacts_current ("));
    assert!(sql.contains("content_id TEXT NOT NULL"));
    assert!(sql.contains("modifiers TEXT NOT NULL DEFAULT '[]'"));
    assert!(sql.contains("PRIMARY KEY (repo_id, path, symbol_id)"));
    assert!(sql.contains("UNIQUE (repo_id, artefact_id)"));
    assert!(sql.contains("CREATE INDEX IF NOT EXISTS artefacts_current_path_idx"));
    assert!(sql.contains("CREATE INDEX IF NOT EXISTS artefacts_current_kind_idx"));
    assert!(sql.contains("CREATE INDEX IF NOT EXISTS artefacts_current_fqn_idx"));
    assert!(!sql.contains("branch TEXT"));
    assert!(!sql.contains("commit_sha"));
    assert!(!sql.contains("revision_kind"));
    assert!(!sql.contains("revision_id"));
    assert!(!sql.contains("temp_checkpoint_id"));
    assert!(!sql.contains("blob_sha"));

    assert!(sql.contains("CREATE TABLE IF NOT EXISTS artefact_edges_current ("));
    assert!(sql.contains("metadata TEXT NOT NULL DEFAULT '{}'"));
    assert!(sql.contains("PRIMARY KEY (repo_id, edge_id)"));
    assert!(sql.contains("CREATE INDEX IF NOT EXISTS artefact_edges_current_path_idx"));
    assert!(sql.contains("CREATE INDEX IF NOT EXISTS artefact_edges_current_from_idx"));
    assert!(!sql.contains("artefact_edges_current_branch_from_idx"));
    assert!(!sql.contains("artefact_edges_current_branch_to_idx"));
    assert!(!sql.contains("JSONB"));
}

fn seed_workspace_repo() -> tempfile::TempDir {
    let dir = tempdir().expect("temp dir");
    crate::test_support::git_fixtures::init_test_repo(
        dir.path(),
        "main",
        "Bitloops Test",
        "bitloops-test@example.com",
    );

    fs::create_dir_all(dir.path().join("src")).expect("create src dir");
    fs::write(
        dir.path().join("src/lib.rs"),
        "pub fn greet(name: &str) -> String {\n    format!(\"hi {name}\")\n}\n",
    )
    .expect("write rust source");
    fs::write(dir.path().join("README.md"), "# ignored\n").expect("write readme");

    crate::test_support::git_fixtures::git_ok(dir.path(), &["add", "."]);
    crate::test_support::git_fixtures::git_ok(dir.path(), &["commit", "-m", "initial"]);
    dir
}

#[test]
fn workspace_state_inspect_workspace_reads_head_tree() {
    let repo = seed_workspace_repo();

    let state = crate::host::devql::sync::workspace_state::inspect_workspace(repo.path())
        .expect("inspect clean workspace");

    let head_sha = crate::host::checkpoints::strategy::manual_commit::run_git(
        repo.path(),
        &["rev-parse", "HEAD"],
    )
    .expect("resolve HEAD");
    let head_blob = crate::host::checkpoints::strategy::manual_commit::run_git(
        repo.path(),
        &["rev-parse", "HEAD:src/lib.rs"],
    )
    .expect("resolve HEAD blob");
    let head_tree_sha = crate::host::checkpoints::strategy::manual_commit::run_git(
        repo.path(),
        &["rev-parse", "HEAD^{tree}"],
    )
    .expect("resolve HEAD tree");
    let active_branch = crate::host::checkpoints::strategy::manual_commit::run_git(
        repo.path(),
        &["branch", "--show-current"],
    )
    .expect("resolve active branch");

    assert_eq!(state.head_commit_sha.as_deref(), Some(head_sha.as_str()));
    assert_eq!(state.head_tree_sha.as_deref(), Some(head_tree_sha.as_str()));
    assert_eq!(state.active_branch.as_deref(), Some(active_branch.as_str()));
    assert_eq!(state.head_tree.len(), 2);
    assert_eq!(state.head_tree.get("src/lib.rs"), Some(&head_blob));
    assert!(state.head_tree.contains_key("README.md"));
    assert!(state.staged_changes.is_empty());
    assert!(state.dirty_files.is_empty());
    assert!(state.untracked_files.is_empty());
}

#[test]
fn workspace_state_reports_dirty_files() {
    let repo = seed_workspace_repo();
    fs::write(
        repo.path().join("src/lib.rs"),
        "pub fn greet(name: &str) -> String {\n    format!(\"hello {name}\")\n}\n",
    )
    .expect("rewrite rust source");

    let state = crate::host::devql::sync::workspace_state::inspect_workspace(repo.path())
        .expect("inspect dirty workspace");

    assert!(state.staged_changes.is_empty());
    assert_eq!(state.dirty_files, vec!["src/lib.rs".to_string()]);
    assert!(state.untracked_files.is_empty());
    assert!(state.head_tree.contains_key("src/lib.rs"));
}

#[test]
fn workspace_state_staged_changes_report_index_diffs() {
    let repo = seed_workspace_repo();
    fs::write(
        repo.path().join("src/lib.rs"),
        "pub fn greet(name: &str) -> String {\n    format!(\"hey {name}\")\n}\n",
    )
    .expect("rewrite rust source");
    crate::test_support::git_fixtures::git_ok(repo.path(), &["add", "src/lib.rs"]);

    let state = crate::host::devql::sync::workspace_state::inspect_workspace(repo.path())
        .expect("inspect staged workspace");

    let index_blob = crate::host::checkpoints::strategy::manual_commit::run_git(
        repo.path(),
        &["rev-parse", ":src/lib.rs"],
    )
    .expect("resolve index blob");
    let staged = state
        .staged_changes
        .get("src/lib.rs")
        .expect("expected staged rust file");
    assert_eq!(
        staged,
        &crate::host::devql::sync::workspace_state::StagedChange::Modified(index_blob)
    );
    assert_eq!(state.staged_changes.len(), 1);
    assert!(state.dirty_files.is_empty());
    assert!(state.untracked_files.is_empty());
}

#[test]
fn workspace_state_reports_staged_deletes() {
    let repo = seed_workspace_repo();
    crate::test_support::git_fixtures::git_ok(repo.path(), &["rm", "src/lib.rs"]);

    let state = crate::host::devql::sync::workspace_state::inspect_workspace(repo.path())
        .expect("inspect staged delete workspace");

    let staged = state
        .staged_changes
        .get("src/lib.rs")
        .expect("expected staged delete");
    assert_eq!(
        staged,
        &crate::host::devql::sync::workspace_state::StagedChange::Deleted
    );
    assert_eq!(state.staged_changes.len(), 1);
    assert!(state.dirty_files.is_empty());
    assert!(state.untracked_files.is_empty());
}

#[test]
fn workspace_state_reports_untracked_files() {
    let repo = seed_workspace_repo();
    fs::write(
        repo.path().join("src/new_file.rs"),
        "pub fn created() -> i32 {\n    7\n}\n",
    )
    .expect("write untracked rust source");

    let state = crate::host::devql::sync::workspace_state::inspect_workspace(repo.path())
        .expect("inspect workspace with untracked file");

    assert!(state.staged_changes.is_empty());
    assert!(state.dirty_files.is_empty());
    assert_eq!(state.untracked_files, vec!["src/new_file.rs".to_string()]);
    assert!(!state.head_tree.contains_key("src/new_file.rs"));
}

#[test]
fn workspace_state_unborn_head_reports_raw_workspace_state() {
    let repo = tempdir().expect("temp dir");
    crate::test_support::git_fixtures::init_test_repo(
        repo.path(),
        "main",
        "Bitloops Test",
        "bitloops-test@example.com",
    );
    fs::create_dir_all(repo.path().join("src")).expect("create src dir");
    fs::write(
        repo.path().join("src/lib.rs"),
        "pub fn draft() -> bool {\n    true\n}\n",
    )
    .expect("write rust source");
    crate::test_support::git_fixtures::git_ok(repo.path(), &["add", "src/lib.rs"]);

    let state = crate::host::devql::sync::workspace_state::inspect_workspace(repo.path())
        .expect("inspect unborn HEAD");

    let active_branch = crate::host::checkpoints::strategy::manual_commit::run_git(
        repo.path(),
        &["branch", "--show-current"],
    )
    .expect("resolve active branch");

    assert_eq!(state.head_commit_sha, None);
    assert_eq!(state.head_tree_sha, None);
    assert_eq!(state.active_branch.as_deref(), Some(active_branch.as_str()));
    assert!(state.head_tree.is_empty());
    assert_eq!(state.staged_changes.len(), 1);
    assert_eq!(
        state
            .staged_changes
            .get("src/lib.rs")
            .expect("expected staged rust file"),
        &crate::host::devql::sync::workspace_state::StagedChange::Added(
            crate::host::checkpoints::strategy::manual_commit::run_git(
                repo.path(),
                &["rev-parse", ":src/lib.rs"],
            )
            .expect("resolve staged blob"),
        )
    );
    assert!(state.dirty_files.is_empty());
    assert!(state.untracked_files.is_empty());
}

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
        parse_status: "parsed".to_string(),
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
        parse_status: "parsed".to_string(),
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

#[test]
fn sync_extraction_converts_typescript_content_to_cache_format() {
    let cfg = sync_test_cfg();
    let path = "src/sample.ts";
    let content = r#"import { remoteFoo } from "./remote";

class Service {
  run(): number {
    return localHelper() + remoteFoo();
  }
}

function localHelper(): number {
  return 1;
}
"#;
    let content_id =
        crate::host::devql::sync::content_identity::compute_blob_oid(content.as_bytes());

    let extraction = crate::host::devql::sync::extraction::extract_to_cache_format(
        &cfg,
        path,
        &content_id,
        "tree-sitter-ts@1",
        "ts-language-pack@1",
        content,
    )
    .expect("extract TypeScript content into cache format")
    .expect("TypeScript cache extraction should be supported");

    assert_eq!(extraction.content_id, content_id);
    assert_eq!(extraction.language, "typescript");
    assert_eq!(extraction.parser_version, "tree-sitter-ts@1");
    assert_eq!(extraction.extractor_version, "ts-language-pack@1");
    assert_eq!(extraction.parse_status, "parsed");

    let repeated = crate::host::devql::sync::extraction::extract_to_cache_format(
        &cfg,
        path,
        &content_id,
        "tree-sitter-ts@1",
        "ts-language-pack@1",
        content,
    )
    .expect("repeat extract TypeScript content into cache format")
    .expect("repeated TypeScript cache extraction should be supported");
    assert_eq!(
        extraction, repeated,
        "cache extraction should be deterministic"
    );

    let file = extraction
        .artefacts
        .iter()
        .find(|artefact| {
            artefact.canonical_kind.as_deref() == Some("file") && artefact.name == path
        })
        .expect("expected file artefact");
    let class = extraction
        .artefacts
        .iter()
        .find(|artefact| {
            artefact.language_kind == "class_declaration" && artefact.name == "Service"
        })
        .expect("expected class artefact");
    let method = extraction
        .artefacts
        .iter()
        .find(|artefact| {
            artefact.canonical_kind.as_deref() == Some("method") && artefact.name == "run"
        })
        .expect("expected method artefact");
    let helper = extraction
        .artefacts
        .iter()
        .find(|artefact| {
            artefact.canonical_kind.as_deref() == Some("function") && artefact.name == "localHelper"
        })
        .expect("expected local helper artefact");

    assert!(
        !file.artifact_key.is_empty(),
        "file artefact key should be deterministic and non-empty"
    );
    assert!(
        !class.artifact_key.is_empty(),
        "class artefact key should be deterministic and non-empty"
    );
    assert!(
        !method.artifact_key.is_empty(),
        "method artefact key should be deterministic and non-empty"
    );
    assert!(
        !helper.artifact_key.is_empty(),
        "helper artefact key should be deterministic and non-empty"
    );
    assert_eq!(
        class.parent_artifact_key.as_deref(),
        Some(file.artifact_key.as_str())
    );
    assert_eq!(
        method.parent_artifact_key.as_deref(),
        Some(class.artifact_key.as_str())
    );
    assert_eq!(
        helper.parent_artifact_key.as_deref(),
        Some(file.artifact_key.as_str())
    );

    let same_file_call = extraction
        .edges
        .iter()
        .find(|edge| {
            edge.edge_kind == "calls"
                && edge.from_artifact_key == method.artifact_key
                && edge.to_artifact_key.as_deref() == Some(helper.artifact_key.as_str())
        })
        .expect("expected same-file call edge");
    assert!(
        !same_file_call.edge_key.is_empty(),
        "same-file edge key should be deterministic and non-empty"
    );
    assert_eq!(same_file_call.to_symbol_ref, None);

    let cross_file_call = extraction
        .edges
        .iter()
        .find(|edge| {
            edge.edge_kind == "calls"
                && edge.from_artifact_key == method.artifact_key
                && edge.to_symbol_ref.as_deref() == Some("./remote::remoteFoo")
        })
        .expect("expected cross-file call edge");
    assert!(
        !cross_file_call.edge_key.is_empty(),
        "cross-file edge key should be deterministic and non-empty"
    );
    assert_eq!(cross_file_call.to_artifact_key, None);

    let import_edge = extraction
        .edges
        .iter()
        .find(|edge| {
            edge.edge_kind == "imports"
                && edge.from_artifact_key == file.artifact_key
                && edge.to_symbol_ref.as_deref() == Some("./remote")
        })
        .expect("expected file-level import edge");
    assert!(
        !import_edge.edge_key.is_empty(),
        "import edge key should be deterministic and non-empty"
    );
    assert_eq!(import_edge.to_artifact_key, None);
}

#[test]
fn sync_extraction_uses_path_agnostic_artifact_keys_for_same_content() {
    let cfg = sync_test_cfg();
    let content = r#"class Service {
  run(): number {
    return localHelper();
  }
}

function localHelper(): number {
  return 1;
}
"#;
    let content_id =
        crate::host::devql::sync::content_identity::compute_blob_oid(content.as_bytes());

    let first = crate::host::devql::sync::extraction::extract_to_cache_format(
        &cfg,
        "src/sample.ts",
        &content_id,
        "tree-sitter-ts@1",
        "ts-language-pack@1",
        content,
    )
    .expect("extract first TypeScript path")
    .expect("first TypeScript cache extraction should be supported");
    let second = crate::host::devql::sync::extraction::extract_to_cache_format(
        &cfg,
        "nested/other.ts",
        &content_id,
        "tree-sitter-ts@1",
        "ts-language-pack@1",
        content,
    )
    .expect("extract second TypeScript path")
    .expect("second TypeScript cache extraction should be supported");

    let key_for = |extraction: &crate::host::devql::sync::content_cache::CachedExtraction,
                   name: &str,
                   language_kind: &str| {
        extraction
            .artefacts
            .iter()
            .find(|artefact| artefact.name == name && artefact.language_kind == language_kind)
            .map(|artefact| artefact.artifact_key.clone())
            .expect("expected artefact key")
    };

    assert_eq!(
        first
            .artefacts
            .iter()
            .find(|artefact| artefact.canonical_kind.as_deref() == Some("file"))
            .map(|artefact| artefact.artifact_key.clone()),
        second
            .artefacts
            .iter()
            .find(|artefact| artefact.canonical_kind.as_deref() == Some("file"))
            .map(|artefact| artefact.artifact_key.clone())
    );
    assert_eq!(
        key_for(&first, "Service", "class_declaration"),
        key_for(&second, "Service", "class_declaration")
    );
    assert_eq!(
        key_for(&first, "run", "method_definition"),
        key_for(&second, "run", "method_definition")
    );
    assert_eq!(
        key_for(&first, "localHelper", "function_declaration"),
        key_for(&second, "localHelper", "function_declaration")
    );

    let same_file_edge_key =
        |extraction: &crate::host::devql::sync::content_cache::CachedExtraction| {
            extraction
                .edges
                .iter()
                .find(|edge| edge.edge_kind == "calls" && edge.to_artifact_key.is_some())
                .map(|edge| edge.edge_key.clone())
                .expect("expected same-file edge key")
        };

    assert_eq!(same_file_edge_key(&first), same_file_edge_key(&second));
}

#[tokio::test]
async fn materialize_writes_artefacts_current_with_correct_symbol_id() {
    let cfg = sync_test_cfg();
    let temp = tempdir().expect("temp dir");
    let sqlite_path = temp.path().join("devql.sqlite");
    let relational = sqlite_relational_store_with_sync_schema(&sqlite_path).await;
    let path = "src/sample.ts";
    let content = r#"import { remoteFoo } from "./remote";

class Service {
  run(): number {
    return localHelper() + remoteFoo();
  }
}

function localHelper(): number {
  return 1;
}
"#;
    let content_id =
        crate::host::devql::sync::content_identity::compute_blob_oid(content.as_bytes());
    let desired = desired_file_state(path, "typescript", &content_id);
    let extraction = crate::host::devql::sync::extraction::extract_to_cache_format(
        &cfg,
        path,
        &content_id,
        "tree-sitter-ts@1",
        "ts-language-pack@1",
        content,
    )
    .expect("extract TypeScript content into cache format")
    .expect("TypeScript cache extraction should be supported");
    let rev = crate::host::devql::FileRevision {
        commit_sha: &content_id,
        revision: crate::host::devql::TemporalRevisionRef {
            kind: crate::host::devql::TemporalRevisionKind::Temporary,
            id: &content_id,
            temp_checkpoint_id: None,
        },
        commit_unix: 0,
        path,
        blob_sha: &content_id,
    };
    let (items, _, _) = crate::host::devql::extract_language_pack_artefacts_and_edges(
        &cfg,
        &rev,
        "typescript",
        content,
    )
    .expect("extract expected TypeScript artefacts")
    .expect("TypeScript artefacts should be supported");
    let expected_symbol_ids = expected_symbol_id_by_fqn(&items, path);

    crate::host::devql::sync::materializer::materialize_path(
        &cfg,
        &relational,
        &desired,
        &extraction,
        "tree-sitter-ts@1",
        "ts-language-pack@1",
    )
    .await
    .expect("materialize cached path");

    let db = Connection::open(&sqlite_path).expect("open sqlite db");
    let mut stmt = db
        .prepare(
            "SELECT symbol_fqn, symbol_id, artefact_id \
             FROM artefacts_current \
             WHERE repo_id = ?1 AND path = ?2 \
             ORDER BY symbol_fqn",
        )
        .expect("prepare current artefacts query");
    let rows = stmt
        .query_map([cfg.repo.repo_id.as_str(), path], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .expect("query current artefacts")
        .collect::<Result<Vec<_>, _>>()
        .expect("collect current artefacts");

    let mut expected = expected_symbol_ids
        .into_iter()
        .map(|(symbol_fqn, symbol_id)| {
            let artefact_id = crate::host::devql::revision_artefact_id(
                &cfg.repo.repo_id,
                &content_id,
                &symbol_id,
            );
            (symbol_fqn, symbol_id, artefact_id)
        })
        .collect::<Vec<_>>();
    expected.sort_by(|lhs, rhs| lhs.0.cmp(&rhs.0));

    assert_eq!(rows, expected);
}

#[tokio::test]
async fn materialize_then_re_materialize_is_idempotent() {
    let cfg = sync_test_cfg();
    let temp = tempdir().expect("temp dir");
    let sqlite_path = temp.path().join("devql.sqlite");
    let relational = sqlite_relational_store_with_sync_schema(&sqlite_path).await;
    let path = "src/sample.ts";
    let content = r#"import { remoteFoo } from "./remote";

class Service {
  run(): number {
    return localHelper() + remoteFoo();
  }
}

function localHelper(): number {
  return 1;
}
"#;
    let content_id =
        crate::host::devql::sync::content_identity::compute_blob_oid(content.as_bytes());
    let desired = desired_file_state(path, "typescript", &content_id);
    let extraction = crate::host::devql::sync::extraction::extract_to_cache_format(
        &cfg,
        path,
        &content_id,
        "tree-sitter-ts@1",
        "ts-language-pack@1",
        content,
    )
    .expect("extract TypeScript content into cache format")
    .expect("TypeScript cache extraction should be supported");
    let rev = crate::host::devql::FileRevision {
        commit_sha: &content_id,
        revision: crate::host::devql::TemporalRevisionRef {
            kind: crate::host::devql::TemporalRevisionKind::Temporary,
            id: &content_id,
            temp_checkpoint_id: None,
        },
        commit_unix: 0,
        path,
        blob_sha: &content_id,
    };
    let (items, _, _) = crate::host::devql::extract_language_pack_artefacts_and_edges(
        &cfg,
        &rev,
        "typescript",
        content,
    )
    .expect("extract expected TypeScript artefacts")
    .expect("TypeScript artefacts should be supported");
    let expected_symbol_ids = expected_symbol_id_by_fqn(&items, path);
    let helper_symbol_id = expected_symbol_ids
        .get(&format!("{path}::localHelper"))
        .cloned()
        .expect("expected localHelper symbol id");
    let helper_artefact_id =
        crate::host::devql::revision_artefact_id(&cfg.repo.repo_id, &content_id, &helper_symbol_id);

    let load_artefacts = |db: &Connection| {
        let mut stmt = db
            .prepare(
                "SELECT symbol_fqn, symbol_id, artefact_id, parent_symbol_id, parent_artefact_id \
                 FROM artefacts_current \
                 WHERE repo_id = ?1 AND path = ?2 \
                 ORDER BY symbol_fqn",
            )
            .expect("prepare artefacts_current query");
        stmt.query_map([cfg.repo.repo_id.as_str(), path], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, Option<String>>(4)?,
            ))
        })
        .expect("query artefacts_current rows")
        .collect::<Result<Vec<_>, _>>()
        .expect("collect artefacts_current rows")
    };
    let load_edges = |db: &Connection| {
        let mut stmt = db
            .prepare(
                "SELECT edge_id, from_symbol_id, to_symbol_id, to_artefact_id, to_symbol_ref, edge_kind \
                 FROM artefact_edges_current \
                 WHERE repo_id = ?1 AND path = ?2 \
                 ORDER BY edge_id",
            )
            .expect("prepare artefact_edges_current query");
        stmt.query_map([cfg.repo.repo_id.as_str(), path], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, Option<String>>(4)?,
                row.get::<_, String>(5)?,
            ))
        })
        .expect("query artefact_edges_current rows")
        .collect::<Result<Vec<_>, _>>()
        .expect("collect artefact_edges_current rows")
    };
    let load_current_state = |db: &Connection| {
        db.query_row(
            "SELECT language, head_content_id, index_content_id, worktree_content_id, effective_content_id, effective_source, parser_version, extractor_version, exists_in_head, exists_in_index, exists_in_worktree \
             FROM current_file_state \
             WHERE repo_id = ?1 AND path = ?2",
            [cfg.repo.repo_id.as_str(), path],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, String>(7)?,
                    row.get::<_, i64>(8)?,
                    row.get::<_, i64>(9)?,
                    row.get::<_, i64>(10)?,
                ))
            },
        )
        .expect("load current_file_state row")
    };

    crate::host::devql::sync::materializer::materialize_path(
        &cfg,
        &relational,
        &desired,
        &extraction,
        "tree-sitter-ts@1",
        "ts-language-pack@1",
    )
    .await
    .expect("materialize cached path first time");

    let db = Connection::open(&sqlite_path).expect("open sqlite db");
    let first_artefacts = load_artefacts(&db);
    let first_edges = load_edges(&db);
    let first_state = load_current_state(&db);

    crate::host::devql::sync::materializer::materialize_path(
        &cfg,
        &relational,
        &desired,
        &extraction,
        "tree-sitter-ts@1",
        "ts-language-pack@1",
    )
    .await
    .expect("materialize cached path second time");

    let second_artefacts = load_artefacts(&db);
    let second_edges = load_edges(&db);
    let second_state = load_current_state(&db);

    assert_eq!(first_artefacts, second_artefacts);
    assert_eq!(first_edges, second_edges);
    assert_eq!(first_state, second_state);
    assert_eq!(first_artefacts.len(), extraction.artefacts.len());
    assert_eq!(first_edges.len(), extraction.edges.len());
    assert_eq!(
        first_state,
        (
            "typescript".to_string(),
            Some(content_id.clone()),
            Some(content_id.clone()),
            Some(content_id.clone()),
            content_id.clone(),
            "head".to_string(),
            "tree-sitter-ts@1".to_string(),
            "ts-language-pack@1".to_string(),
            1,
            1,
            1,
        )
    );
    assert!(
        first_edges.iter().any(|edge| {
            edge.2.as_deref() == Some(helper_symbol_id.as_str())
                && edge.3.as_deref() == Some(helper_artefact_id.as_str())
                && edge.5 == "calls"
        }),
        "same-file call edge should resolve through cached artifact_key mapping"
    );
}

#[tokio::test]
async fn materialize_reuses_cached_extraction_at_new_path_with_path_sensitive_identity() {
    let cfg = sync_test_cfg();
    let temp = tempdir().expect("temp dir");
    let sqlite_path = temp.path().join("devql.sqlite");
    let relational = sqlite_relational_store_with_sync_schema(&sqlite_path).await;
    let original_path = "src/sample.ts";
    let materialized_path = "nested/other.ts";
    let content = r#"import { remoteFoo } from "./remote";

class Service {
  run(): number {
    return localHelper() + remoteFoo();
  }
}

function localHelper(): number {
  return 1;
}
"#;
    let content_id =
        crate::host::devql::sync::content_identity::compute_blob_oid(content.as_bytes());
    let extraction = crate::host::devql::sync::extraction::extract_to_cache_format(
        &cfg,
        original_path,
        &content_id,
        "tree-sitter-ts@1",
        "ts-language-pack@1",
        content,
    )
    .expect("extract original TypeScript content into cache format")
    .expect("original TypeScript cache extraction should be supported");
    let desired = desired_file_state(materialized_path, "typescript", &content_id);
    let rev = crate::host::devql::FileRevision {
        commit_sha: &content_id,
        revision: crate::host::devql::TemporalRevisionRef {
            kind: crate::host::devql::TemporalRevisionKind::Temporary,
            id: &content_id,
            temp_checkpoint_id: None,
        },
        commit_unix: 0,
        path: materialized_path,
        blob_sha: &content_id,
    };
    let (items, _, _) = crate::host::devql::extract_language_pack_artefacts_and_edges(
        &cfg,
        &rev,
        "typescript",
        content,
    )
    .expect("extract expected TypeScript artefacts for new path")
    .expect("TypeScript artefacts for new path should be supported");
    let expected_symbol_ids = expected_symbol_id_by_fqn(&items, materialized_path);

    crate::host::devql::sync::materializer::materialize_path(
        &cfg,
        &relational,
        &desired,
        &extraction,
        "tree-sitter-ts@1",
        "ts-language-pack@1",
    )
    .await
    .expect("materialize cached extraction at new path");

    let db = Connection::open(&sqlite_path).expect("open sqlite db");
    let mut stmt = db
        .prepare(
            "SELECT symbol_fqn, symbol_id, artefact_id \
             FROM artefacts_current \
             WHERE repo_id = ?1 AND path = ?2 \
             ORDER BY symbol_fqn",
        )
        .expect("prepare current artefacts query");
    let rows = stmt
        .query_map([cfg.repo.repo_id.as_str(), materialized_path], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .expect("query current artefacts")
        .collect::<Result<Vec<_>, _>>()
        .expect("collect current artefacts");

    let mut expected = expected_symbol_ids
        .into_iter()
        .map(|(symbol_fqn, symbol_id)| {
            let artefact_id = crate::host::devql::revision_artefact_id(
                &cfg.repo.repo_id,
                &content_id,
                &symbol_id,
            );
            (symbol_fqn, symbol_id, artefact_id)
        })
        .collect::<Vec<_>>();
    expected.sort_by(|lhs, rhs| lhs.0.cmp(&rhs.0));

    assert!(
        rows.iter()
            .all(|(symbol_fqn, _, _)| symbol_fqn.starts_with(materialized_path)),
        "all symbol_fqn values should be re-derived from the materialization path"
    );
    assert!(
        rows.iter()
            .all(|(symbol_fqn, _, _)| !symbol_fqn.starts_with(original_path)),
        "stored symbol_fqn values should not retain the cached source path"
    );
    assert_eq!(rows, expected);
}

#[tokio::test]
async fn remove_path_deletes_all_rows() {
    let cfg = sync_test_cfg();
    let temp = tempdir().expect("temp dir");
    let sqlite_path = temp.path().join("devql.sqlite");
    let relational = sqlite_relational_store_with_sync_schema(&sqlite_path).await;
    let path = "src/sample.ts";
    let content = r#"import { remoteFoo } from "./remote";

class Service {
  run(): number {
    return localHelper() + remoteFoo();
  }
}

function localHelper(): number {
  return 1;
}
"#;
    let content_id =
        crate::host::devql::sync::content_identity::compute_blob_oid(content.as_bytes());
    let desired = desired_file_state(path, "typescript", &content_id);
    let extraction = crate::host::devql::sync::extraction::extract_to_cache_format(
        &cfg,
        path,
        &content_id,
        "tree-sitter-ts@1",
        "ts-language-pack@1",
        content,
    )
    .expect("extract TypeScript content into cache format")
    .expect("TypeScript cache extraction should be supported");

    crate::host::devql::sync::materializer::materialize_path(
        &cfg,
        &relational,
        &desired,
        &extraction,
        "tree-sitter-ts@1",
        "ts-language-pack@1",
    )
    .await
    .expect("materialize cached path");
    crate::host::devql::sync::materializer::remove_path(&cfg, &relational, path)
        .await
        .expect("remove materialized path");

    let db = Connection::open(&sqlite_path).expect("open sqlite db");
    let artefact_count: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM artefacts_current WHERE repo_id = ?1 AND path = ?2",
            [cfg.repo.repo_id.as_str(), path],
            |row| row.get(0),
        )
        .expect("count artefacts_current rows");
    let edge_count: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM artefact_edges_current WHERE repo_id = ?1 AND path = ?2",
            [cfg.repo.repo_id.as_str(), path],
            |row| row.get(0),
        )
        .expect("count artefact_edges_current rows");
    let current_file_state_count: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM current_file_state WHERE repo_id = ?1 AND path = ?2",
            [cfg.repo.repo_id.as_str(), path],
            |row| row.get(0),
        )
        .expect("count current_file_state rows");

    assert_eq!(artefact_count, 0);
    assert_eq!(edge_count, 0);
    assert_eq!(current_file_state_count, 0);
}
