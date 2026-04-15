use rusqlite::Connection;
use std::fs;

use super::fixtures::{
    seed_full_sync_repo, sqlite_relational_store_with_sync_schema, sync_test_cfg_for_repo,
};

#[tokio::test]
async fn cached_parse_error_hit_updates_manifest_and_clears_materialized_rows() {
    let repo = seed_full_sync_repo();
    let cfg = sync_test_cfg_for_repo(repo.path());
    let sqlite_path = repo.path().join("devql.sqlite");
    let relational = sqlite_relational_store_with_sync_schema(&sqlite_path).await;
    let path = "src/lib.rs";
    let broken_content = "fn broken( {\n";

    let baseline = crate::host::devql::execute_sync(
        &cfg,
        &relational,
        crate::host::devql::sync::types::SyncMode::Full,
    )
    .await
    .expect("execute baseline full sync");

    let db = Connection::open(&sqlite_path).expect("open sqlite db");
    let baseline_fingerprint: String = db
        .query_row(
            "SELECT extraction_fingerprint FROM current_file_state WHERE repo_id = ?1 AND path = ?2",
            [cfg.repo.repo_id.as_str(), path],
            |row| row.get(0),
        )
        .expect("read baseline extraction fingerprint");
    let baseline_rows: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM artefacts_current WHERE repo_id = ?1 AND path = ?2",
            [cfg.repo.repo_id.as_str(), path],
            |row| row.get(0),
        )
        .expect("count baseline rows for parse-error path");
    assert!(
        baseline_rows > 0,
        "baseline sync should materialize at least one row for parse-error path"
    );

    fs::write(repo.path().join(path), broken_content).expect("write broken source content");
    let broken_content_id =
        crate::host::devql::sync::content_identity::compute_blob_oid(broken_content.as_bytes());
    let parse_error_payload = crate::host::devql::sync::content_cache::CachedExtraction {
        content_id: broken_content_id.clone(),
        language: "rust".to_string(),
        extraction_fingerprint: baseline_fingerprint,
        parser_version: baseline.parser_version.clone(),
        extractor_version: baseline.extractor_version.clone(),
        parse_status: crate::host::devql::sync::extraction::PARSE_STATUS_PARSE_ERROR.to_string(),
        artefacts: vec![],
        edges: vec![],
    };
    crate::host::devql::sync::content_cache::store_cached_content(
        &relational,
        &parse_error_payload,
        "worktree_only",
    )
    .await
    .expect("store parse-error cache payload");

    let summary = crate::host::devql::execute_sync(
        &cfg,
        &relational,
        crate::host::devql::sync::types::SyncMode::Full,
    )
    .await
    .expect("execute full sync with cached parse-error payload");

    assert_eq!(summary.paths_changed, 1);
    assert_eq!(summary.cache_hits, 1);
    assert_eq!(summary.cache_misses, 0);
    assert_eq!(summary.parse_errors, 1);

    let rows_after: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM artefacts_current WHERE repo_id = ?1 AND path = ?2",
            [cfg.repo.repo_id.as_str(), path],
            |row| row.get(0),
        )
        .expect("count rows after parse-error materialization");
    assert_eq!(
        rows_after, 0,
        "parse-error payload should clear materialized rows for the path"
    );

    let current_state: (String, String) = db
        .query_row(
            "SELECT effective_content_id, effective_source \
             FROM current_file_state \
             WHERE repo_id = ?1 AND path = ?2",
            [cfg.repo.repo_id.as_str(), path],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("read current_file_state after parse-error sync");
    assert_eq!(current_state.0, broken_content_id);
    assert_eq!(current_state.1, "worktree");

    let parse_status: String = db
        .query_row(
            "SELECT parse_status FROM content_cache \
             WHERE content_id = ?1 AND language = ?2 AND parser_version = ?3 AND extractor_version = ?4",
            [
                current_state.0.as_str(),
                "rust",
                summary.parser_version.as_str(),
                summary.extractor_version.as_str(),
            ],
            |row| row.get(0),
        )
        .expect("read parse_status for parse-error cache row");
    assert_eq!(
        parse_status,
        crate::host::devql::sync::extraction::PARSE_STATUS_PARSE_ERROR
    );
}

#[tokio::test]
async fn branch_switch_reuses_cache() {
    let repo = seed_full_sync_repo();
    let cfg = sync_test_cfg_for_repo(repo.path());
    let sqlite_path = repo.path().join("devql.sqlite");
    let relational = sqlite_relational_store_with_sync_schema(&sqlite_path).await;
    let path = "src/lib.rs";
    let feature_content =
        "pub fn greet(name: &str) -> String {\n    format!(\"feature {name}\")\n}\n";

    let initial_sync = crate::host::devql::execute_sync(
        &cfg,
        &relational,
        crate::host::devql::sync::types::SyncMode::Full,
    )
    .await
    .expect("execute initial full sync on main");

    assert_eq!(initial_sync.cache_hits, 0);
    assert_eq!(initial_sync.cache_misses, initial_sync.paths_added);
    assert!(initial_sync.cache_misses > 0);

    crate::host::checkpoints::strategy::manual_commit::run_git(
        repo.path(),
        &["checkout", "-b", "feature/cache-reuse"],
    )
    .expect("create feature branch");
    fs::write(repo.path().join(path), feature_content).expect("edit tracked source file");
    crate::host::checkpoints::strategy::manual_commit::run_git(repo.path(), &["add", path])
        .expect("stage feature change");
    crate::host::checkpoints::strategy::manual_commit::run_git(
        repo.path(),
        &["commit", "-m", "feature branch change"],
    )
    .expect("commit feature branch change");

    let feature_sync = crate::host::devql::execute_sync(
        &cfg,
        &relational,
        crate::host::devql::sync::types::SyncMode::Full,
    )
    .await
    .expect("execute sync on feature branch");

    assert_eq!(feature_sync.cache_hits, 0);
    assert_eq!(feature_sync.cache_misses, 1);

    crate::host::checkpoints::strategy::manual_commit::run_git(repo.path(), &["checkout", "main"])
        .expect("checkout main branch");

    let main_sync = crate::host::devql::execute_sync(
        &cfg,
        &relational,
        crate::host::devql::sync::types::SyncMode::Full,
    )
    .await
    .expect("execute sync after returning to main");

    assert_eq!(main_sync.cache_hits, 1);
    assert_eq!(main_sync.cache_misses, 0);

    let db = Connection::open(&sqlite_path).expect("open sqlite db");
    let current_state: (String, String) = db
        .query_row(
            "SELECT effective_content_id, effective_source \
             FROM current_file_state \
             WHERE repo_id = ?1 AND path = ?2",
            [cfg.repo.repo_id.as_str(), path],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("read current_file_state after branch switch");

    assert_eq!(
        current_state.0,
        crate::host::devql::sync::content_identity::compute_blob_oid(
            "pub fn greet(name: &str) -> String {\n    format!(\"hi {name}\")\n}\n".as_bytes()
        )
    );
    assert_eq!(current_state.1, "head");
}

#[tokio::test]
async fn staged_content_takes_precedence_over_head() {
    let repo = seed_full_sync_repo();
    let cfg = sync_test_cfg_for_repo(repo.path());
    let sqlite_path = repo.path().join("devql.sqlite");
    let relational = sqlite_relational_store_with_sync_schema(&sqlite_path).await;
    let path = "src/lib.rs";
    let staged_content =
        "pub fn greet(name: &str) -> String {\n    format!(\"staged {name}\")\n}\n";

    crate::host::devql::execute_sync(
        &cfg,
        &relational,
        crate::host::devql::sync::types::SyncMode::Full,
    )
    .await
    .expect("execute baseline full sync");

    let baseline_supported_paths: i64 = Connection::open(&sqlite_path)
        .expect("open sqlite db")
        .query_row(
            "SELECT COUNT(*) FROM current_file_state WHERE repo_id = ?1",
            [cfg.repo.repo_id.as_str()],
            |row| row.get(0),
        )
        .expect("count baseline supported paths");

    fs::write(repo.path().join(path), staged_content).expect("edit tracked source file");
    crate::test_support::git_fixtures::git_ok(repo.path(), &["add", path]);

    let result = crate::host::devql::execute_sync(
        &cfg,
        &relational,
        crate::host::devql::sync::types::SyncMode::Full,
    )
    .await
    .expect("execute sync with staged content");

    let db = Connection::open(&sqlite_path).expect("open sqlite db");
    let current_state: (String, Option<String>, String, Option<String>) = db
        .query_row(
            "SELECT effective_content_id, index_content_id, effective_source, head_content_id \
             FROM current_file_state \
             WHERE repo_id = ?1 AND path = ?2",
            [cfg.repo.repo_id.as_str(), path],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .expect("read current_file_state for staged path");
    let staged_blob =
        crate::host::devql::sync::content_identity::compute_blob_oid(staged_content.as_bytes());

    assert_eq!(result.paths_changed, 1);
    assert_eq!(result.paths_added, 0);
    assert_eq!(result.paths_removed, 0);
    assert_eq!(
        result.paths_changed + result.paths_unchanged,
        baseline_supported_paths as usize
    );
    assert_eq!(current_state.0, staged_blob);
    assert_eq!(current_state.1.as_deref(), Some(staged_blob.as_str()));
    assert_eq!(current_state.2, "index");
    assert_ne!(current_state.3.as_deref(), Some(staged_blob.as_str()));
}
