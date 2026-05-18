use rusqlite::Connection;
use std::fs;

use super::fixtures::{
    seed_full_sync_repo, sqlite_relational_store_with_sync_schema, sync_test_cfg_for_repo,
};

#[tokio::test]
async fn sync_detects_file_edit() {
    let repo = seed_full_sync_repo();
    let cfg = sync_test_cfg_for_repo(repo.path());
    let sqlite_path = repo.path().join("devql.sqlite");
    let relational = sqlite_relational_store_with_sync_schema(&sqlite_path).await;
    let original_content = "pub fn greet(name: &str) -> String {\n    format!(\"hi {name}\")\n}\n";
    let edited_content = "pub fn greet(name: &str) -> String {\n    format!(\"hello {name}\")\n}\n";
    let original_blob =
        crate::host::devql::sync::content_identity::compute_blob_oid(original_content.as_bytes());

    crate::host::devql::execute_sync(
        &cfg,
        &relational,
        crate::host::devql::sync::types::SyncMode::Full,
    )
    .await
    .expect("execute initial full sync");

    fs::write(repo.path().join("src/lib.rs"), edited_content).expect("edit tracked source file");

    let result = crate::host::devql::execute_sync(
        &cfg,
        &relational,
        crate::host::devql::sync::types::SyncMode::Full,
    )
    .await
    .expect("execute sync after edit");

    let db = Connection::open(&sqlite_path).expect("open sqlite db");
    let current_state: (String, String) = db
        .query_row(
            "SELECT effective_content_id, effective_source \
             FROM current_file_state \
             WHERE repo_id = ?1 AND path = ?2",
            [cfg.repo.repo_id.as_str(), "src/lib.rs"],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("read current_file_state for edited path");
    let artefact_content_ids = {
        let mut stmt = db
            .prepare(
                "SELECT content_id FROM artefacts_current \
                 WHERE repo_id = ?1 AND path = ?2 \
                 ORDER BY symbol_id",
            )
            .expect("prepare artefacts_current content query");
        stmt.query_map([cfg.repo.repo_id.as_str(), "src/lib.rs"], |row| {
            row.get::<_, String>(0)
        })
        .expect("query artefacts_current content ids")
        .collect::<Result<Vec<_>, _>>()
        .expect("collect artefacts_current content ids")
    };
    let old_content_count: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM artefacts_current WHERE repo_id = ?1 AND path = ?2 AND content_id = ?3",
            [cfg.repo.repo_id.as_str(), "src/lib.rs", original_blob.as_str()],
            |row| row.get(0),
        )
        .expect("count artefacts with previous content id");

    let edited_blob = crate::host::devql::sync::content_identity::compute_blob_oid(
        &fs::read(repo.path().join("src/lib.rs")).expect("read edited worktree file"),
    );

    assert_eq!(result.paths_changed, 1);
    assert_eq!(result.paths_added, 0);
    assert_eq!(result.paths_removed, 0);
    assert_eq!(result.paths_unchanged, 7);
    assert_eq!(current_state.0, edited_blob);
    assert_eq!(current_state.1, "worktree");
    assert!(!artefact_content_ids.is_empty());
    assert!(
        artefact_content_ids
            .iter()
            .all(|content_id| content_id == &edited_blob),
        "all materialized rows for src/lib.rs should reflect the edited content"
    );
    assert_eq!(
        old_content_count, 0,
        "no artefacts_current rows should remain for the previous content id"
    );
}

#[tokio::test]
async fn dirty_then_commit_unchanged_is_cache_hit() {
    let repo = seed_full_sync_repo();
    let cfg = sync_test_cfg_for_repo(repo.path());
    let sqlite_path = repo.path().join("devql.sqlite");
    let relational = sqlite_relational_store_with_sync_schema(&sqlite_path).await;
    let path = "src/lib.rs";
    let edited_content = "pub fn greet(name: &str) -> String {\n    format!(\"hello {name}\")\n}\n";

    crate::host::devql::execute_sync(
        &cfg,
        &relational,
        crate::host::devql::sync::types::SyncMode::Full,
    )
    .await
    .expect("execute initial full sync");

    fs::write(repo.path().join(path), edited_content).expect("edit tracked source file");

    let dirty_sync = crate::host::devql::execute_sync(
        &cfg,
        &relational,
        crate::host::devql::sync::types::SyncMode::Full,
    )
    .await
    .expect("execute sync after dirty edit");

    assert_eq!(dirty_sync.cache_hits, 0);
    assert_eq!(dirty_sync.cache_misses, 1);

    crate::host::checkpoints::strategy::manual_commit::run_git(repo.path(), &["add", path])
        .expect("stage edited source file");
    crate::host::checkpoints::strategy::manual_commit::run_git(
        repo.path(),
        &["commit", "-m", "commit edited source"],
    )
    .expect("commit edited source file");

    let committed_sync = crate::host::devql::execute_sync(
        &cfg,
        &relational,
        crate::host::devql::sync::types::SyncMode::Full,
    )
    .await
    .expect("execute sync after committing unchanged content");

    assert_eq!(committed_sync.cache_hits, 1);
    assert_eq!(committed_sync.cache_misses, 0);

    let db = Connection::open(&sqlite_path).expect("open sqlite db");
    let current_state: (String, String) = db
        .query_row(
            "SELECT effective_content_id, effective_source \
             FROM current_file_state \
             WHERE repo_id = ?1 AND path = ?2",
            [cfg.repo.repo_id.as_str(), path],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("read current_file_state after commit");

    assert_eq!(
        current_state.0,
        crate::host::devql::sync::content_identity::compute_blob_oid(edited_content.as_bytes())
    );
    assert_eq!(current_state.1, "head");

    let retention_class: String = db
        .query_row(
            "SELECT retention_class \
             FROM content_cache \
             WHERE content_id = ?1 AND language = ?2 AND parser_version = ?3 AND extractor_version = ?4",
            [
                current_state.0.as_str(),
                "rust",
                committed_sync.parser_version.as_str(),
                committed_sync.extractor_version.as_str(),
            ],
            |row| row.get(0),
    )
        .expect("read content_cache retention class");
    assert_eq!(retention_class, "git_backed");
}

#[tokio::test]
async fn linked_worktrees_keep_local_current_state_isolated_per_workspace() {
    let main_repo = seed_full_sync_repo();
    let feature_parent = tempfile::tempdir().expect("temp dir");
    let feature_root = feature_parent.path().join("feature-worktree");
    let feature_root_str = feature_root.to_string_lossy().to_string();
    crate::test_support::git_fixtures::git_ok(
        main_repo.path(),
        &[
            "worktree",
            "add",
            "-b",
            "feature/isolation",
            &feature_root_str,
        ],
    );

    let mut main_cfg = sync_test_cfg_for_repo(main_repo.path());
    let mut feature_cfg = sync_test_cfg_for_repo(&feature_root);
    let shared_repo_id = crate::host::devql::deterministic_uuid("repo://linked-worktree-isolation");
    main_cfg.repo.repo_id = shared_repo_id.clone();
    feature_cfg.repo.repo_id = shared_repo_id;

    let main_sqlite_path = main_repo.path().join("devql-main.sqlite");
    let feature_sqlite_path = feature_root.join("devql-feature.sqlite");
    let main_relational = sqlite_relational_store_with_sync_schema(&main_sqlite_path).await;
    let feature_relational = sqlite_relational_store_with_sync_schema(&feature_sqlite_path).await;

    crate::host::devql::execute_sync(
        &main_cfg,
        &main_relational,
        crate::host::devql::sync::types::SyncMode::Full,
    )
    .await
    .expect("sync main worktree");

    let feature_content =
        "pub fn greet(name: &str) -> String {\n    format!(\"feature {name}\")\n}\n";
    fs::write(feature_root.join("src/lib.rs"), feature_content)
        .expect("edit tracked file in feature worktree");
    crate::host::devql::execute_sync(
        &feature_cfg,
        &feature_relational,
        crate::host::devql::sync::types::SyncMode::Full,
    )
    .await
    .expect("sync feature worktree");

    let main_db = Connection::open(&main_sqlite_path).expect("open main sqlite db");
    let feature_db = Connection::open(&feature_sqlite_path).expect("open feature sqlite db");
    let main_content_id: String = main_db
        .query_row(
            "SELECT effective_content_id FROM current_file_state WHERE repo_id = ?1 AND path = ?2",
            [main_cfg.repo.repo_id.as_str(), "src/lib.rs"],
            |row| row.get(0),
        )
        .expect("read main worktree current state");
    let feature_content_id: String = feature_db
        .query_row(
            "SELECT effective_content_id FROM current_file_state WHERE repo_id = ?1 AND path = ?2",
            [feature_cfg.repo.repo_id.as_str(), "src/lib.rs"],
            |row| row.get(0),
        )
        .expect("read feature worktree current state");

    assert_eq!(
        main_content_id,
        crate::host::devql::sync::content_identity::compute_blob_oid(
            "pub fn greet(name: &str) -> String {\n    format!(\"hi {name}\")\n}\n".as_bytes()
        )
    );
    assert_eq!(
        feature_content_id,
        crate::host::devql::sync::content_identity::compute_blob_oid(feature_content.as_bytes())
    );
    assert_ne!(
        main_content_id, feature_content_id,
        "linked worktrees should preserve independent local current state"
    );
}
