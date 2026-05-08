use rusqlite::Connection;
use std::fs;
use std::sync::Mutex;
use tempfile::tempdir;

use super::fixtures::{
    seed_full_sync_repo, sqlite_relational_store_with_sync_schema, sync_test_cfg_for_repo,
};

struct CapturingSyncObserver {
    phases: Mutex<Vec<String>>,
}

impl CapturingSyncObserver {
    fn new() -> Self {
        Self {
            phases: Mutex::new(Vec::new()),
        }
    }

    fn phases(&self) -> Vec<String> {
        self.phases.lock().expect("observer phases lock").clone()
    }
}

impl crate::host::devql::SyncObserver for CapturingSyncObserver {
    fn on_progress(&self, update: crate::host::devql::SyncProgressUpdate) {
        self.phases
            .lock()
            .expect("observer phases lock")
            .push(update.phase.as_str().to_string());
    }
}

#[tokio::test]
async fn full_sync_indexes_all_supported_files() {
    let repo = seed_full_sync_repo();
    let cfg = sync_test_cfg_for_repo(repo.path());
    let sqlite_path = repo.path().join("devql.sqlite");
    let relational = sqlite_relational_store_with_sync_schema(&sqlite_path).await;

    let summary = crate::host::devql::execute_sync(
        &cfg,
        &relational,
        crate::host::devql::sync::types::SyncMode::Full,
    )
    .await
    .expect("execute full sync");

    assert!(summary.success, "full sync should report success");
    assert_eq!(summary.paths_added, 8);
    assert_eq!(summary.paths_changed, 0);
    assert_eq!(summary.paths_removed, 0);
    assert_eq!(summary.paths_unchanged, 0);

    let db = Connection::open(&sqlite_path).expect("open sqlite db");
    let mut stmt = db
        .prepare(
            "SELECT path, language, effective_source \
             FROM current_file_state \
             WHERE repo_id = ?1 \
             ORDER BY path",
        )
        .expect("prepare current_file_state query");
    let rows = stmt
        .query_map([cfg.repo.repo_id.as_str()], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .expect("query current_file_state rows")
        .collect::<Result<Vec<_>, _>>()
        .expect("collect current_file_state rows");

    assert_eq!(
        rows,
        vec![
            (
                "Cargo.toml".to_string(),
                "plain_text".to_string(),
                "head".to_string()
            ),
            (
                "scripts/main.py".to_string(),
                "python".to_string(),
                "head".to_string()
            ),
            (
                "scripts/pyproject.toml".to_string(),
                "plain_text".to_string(),
                "head".to_string()
            ),
            (
                "src/lib.rs".to_string(),
                "rust".to_string(),
                "head".to_string()
            ),
            (
                "web/app.ts".to_string(),
                "typescript".to_string(),
                "head".to_string()
            ),
            (
                "web/package.json".to_string(),
                "plain_text".to_string(),
                "head".to_string()
            ),
            (
                "web/tsconfig.json".to_string(),
                "plain_text".to_string(),
                "head".to_string()
            ),
            (
                "web/util.js".to_string(),
                "javascript".to_string(),
                "head".to_string()
            ),
        ]
    );

    let cache_count: i64 = db
        .query_row("SELECT COUNT(*) FROM content_cache", [], |row| row.get(0))
        .expect("count content_cache rows");
    let artefact_count: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM artefacts_current WHERE repo_id = ?1",
            [cfg.repo.repo_id.as_str()],
            |row| row.get(0),
        )
        .expect("count artefacts_current rows");
    let edge_count: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM artefact_edges_current WHERE repo_id = ?1",
            [cfg.repo.repo_id.as_str()],
            |row| row.get(0),
        )
        .expect("count artefact_edges_current rows");
    let sync_state: (String, String, Option<String>, Option<String>) = db
        .query_row(
            "SELECT last_sync_status, last_sync_reason, active_branch, head_commit_sha \
             FROM repo_sync_state WHERE repo_id = ?1",
            [cfg.repo.repo_id.as_str()],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .expect("read repo_sync_state row");
    let repository_row: (String, String, String) = db
        .query_row(
            "SELECT provider, organization, name FROM repositories WHERE repo_id = ?1",
            [cfg.repo.repo_id.as_str()],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .expect("read repositories row");

    assert_eq!(
        cache_count, 8,
        "code and text files should be cached once each"
    );
    assert!(
        artefact_count >= 8,
        "full sync should materialize file and symbol artefacts"
    );
    assert!(
        edge_count >= 2,
        "full sync should materialize dependency edges for supported files"
    );
    assert_eq!(sync_state.0, "completed");
    assert_eq!(sync_state.1, "full");
    assert_eq!(sync_state.2.as_deref(), Some("main"));
    assert_eq!(
        repository_row,
        (
            cfg.repo.provider.clone(),
            cfg.repo.organization.clone(),
            cfg.repo.name.clone()
        )
    );
    assert!(
        sync_state.3.is_some(),
        "completed sync should persist the resolved HEAD commit"
    );
}

#[tokio::test]
async fn full_sync_persists_scope_exclusions_fingerprint_and_skips_follow_up_reconcile() {
    let repo = seed_full_sync_repo();
    let cfg = sync_test_cfg_for_repo(repo.path());
    let sqlite_path = repo.path().join("devql.sqlite");
    let relational = sqlite_relational_store_with_sync_schema(&sqlite_path).await;

    crate::host::devql::execute_sync(
        &cfg,
        &relational,
        crate::host::devql::sync::types::SyncMode::Full,
    )
    .await
    .expect("execute full sync");

    let db = Connection::open(&sqlite_path).expect("open sqlite db");
    let stored_fingerprint: Option<String> = db
        .query_row(
            "SELECT scope_exclusions_fingerprint FROM repo_sync_state WHERE repo_id = ?1",
            [cfg.repo.repo_id.as_str()],
            |row| row.get(0),
        )
        .expect("read scope exclusions fingerprint");
    let expected_fingerprint =
        crate::host::devql::current_scope_exclusions_fingerprint(repo.path())
            .expect("load current scope exclusions fingerprint");
    assert_eq!(
        stored_fingerprint.as_deref(),
        Some(expected_fingerprint.as_str())
    );

    let reconcile_needed = crate::host::devql::scope_exclusion_reconcile_needed(&cfg, &relational)
        .await
        .expect("check scope exclusion reconcile after completed sync");
    assert_eq!(reconcile_needed, None);
}

#[tokio::test]
async fn sync_validate_reports_clean_state_then_detects_stale_rows() {
    let repo = seed_full_sync_repo();
    let cfg = sync_test_cfg_for_repo(repo.path());
    let sqlite_path = repo.path().join("devql.sqlite");
    let relational = sqlite_relational_store_with_sync_schema(&sqlite_path).await;

    crate::host::devql::execute_sync(
        &cfg,
        &relational,
        crate::host::devql::sync::types::SyncMode::Full,
    )
    .await
    .expect("execute full sync before validation");

    let clean = crate::host::devql::execute_sync_validation(&cfg, &relational)
        .await
        .expect("execute sync validation for clean state");
    let clean_report = clean.validation.expect("validation report");
    assert!(
        clean_report.valid,
        "freshly synced state should validate clean"
    );
    assert!(
        clean_report.files_with_drift.is_empty(),
        "clean validation should not report per-file drift"
    );

    let db = Connection::open(&sqlite_path).expect("open sqlite db");
    db.execute(
        "INSERT INTO artefacts_current (
            repo_id, path, content_id, symbol_id, artefact_id, language, canonical_kind,
            language_kind, symbol_fqn, parent_symbol_id, parent_artefact_id, start_line, end_line,
            start_byte, end_byte, signature, modifiers, docstring, updated_at
         )
         SELECT
            repo_id, path, content_id, 'sync-validation-stale-symbol', 'sync-validation-stale-artefact',
            language, canonical_kind, language_kind, symbol_fqn || '::stale', parent_symbol_id,
            parent_artefact_id, start_line, end_line, start_byte, end_byte, signature,
            modifiers, docstring, datetime('now')
         FROM artefacts_current
         WHERE repo_id = ?1
         LIMIT 1",
        [cfg.repo.repo_id.as_str()],
    )
    .expect("insert stale artefact row");

    let drift = crate::host::devql::execute_sync_validation(&cfg, &relational)
        .await
        .expect("execute sync validation with stale rows");
    let drift_report = drift.validation.expect("validation report");
    assert!(
        !drift_report.valid,
        "validation should fail when stale rows exist"
    );
    assert!(
        drift_report.stale_artefacts >= 1,
        "stale artefacts should be counted in the validation report"
    );
    assert!(
        drift_report
            .files_with_drift
            .iter()
            .any(|file| file.stale_artefacts >= 1),
        "stale artefact drift should be attributed to at least one file"
    );
}

#[tokio::test]
async fn sync_validate_emits_progress_before_final_validation_result() {
    let repo = seed_full_sync_repo();
    let cfg = sync_test_cfg_for_repo(repo.path());
    let sqlite_path = repo.path().join("devql.sqlite");
    let relational = sqlite_relational_store_with_sync_schema(&sqlite_path).await;

    crate::host::devql::execute_sync(
        &cfg,
        &relational,
        crate::host::devql::sync::types::SyncMode::Full,
    )
    .await
    .expect("execute full sync before validation");

    let observer = CapturingSyncObserver::new();
    let summary = crate::host::devql::execute_sync_validation_with_observer(
        &cfg,
        &relational,
        Some(&observer),
    )
    .await
    .expect("execute sync validation with progress observer");

    let validation = summary.validation.expect("validation report");
    assert!(validation.valid, "validation should pass for clean state");

    let phases = observer.phases();
    assert!(
        phases
            .iter()
            .any(|phase| phase == "building_validation_projection"),
        "validate should report expected projection progress, got {phases:?}"
    );
    assert!(
        phases
            .iter()
            .any(|phase| phase == "loading_validation_rows"),
        "validate should report row loading progress, got {phases:?}"
    );
    assert!(
        phases
            .iter()
            .any(|phase| phase == "comparing_validation_rows"),
        "validate should report row comparison progress, got {phases:?}"
    );
}

#[tokio::test]
async fn auto_sync_uses_full_reason_and_git_backed_retention_for_clean_head_files() {
    let repo = seed_full_sync_repo();
    let cfg = sync_test_cfg_for_repo(repo.path());
    let sqlite_path = repo.path().join("devql.sqlite");
    let relational = sqlite_relational_store_with_sync_schema(&sqlite_path).await;

    let summary = crate::host::devql::execute_sync(
        &cfg,
        &relational,
        crate::host::devql::sync::types::SyncMode::Auto,
    )
    .await
    .expect("execute auto sync");

    assert_eq!(summary.mode, "full");

    let db = Connection::open(&sqlite_path).expect("open sqlite db");
    let sync_reason: String = db
        .query_row(
            "SELECT last_sync_reason FROM repo_sync_state WHERE repo_id = ?1",
            [cfg.repo.repo_id.as_str()],
            |row| row.get(0),
        )
        .expect("read repo_sync_state reason");
    let retention_classes = {
        let mut stmt = db
            .prepare(
                "SELECT DISTINCT retention_class \
                 FROM content_cache \
                 ORDER BY retention_class",
            )
            .expect("prepare retention class query");
        stmt.query_map([], |row| row.get::<_, String>(0))
            .expect("query retention classes")
            .collect::<Result<Vec<_>, _>>()
            .expect("collect retention classes")
    };

    assert_eq!(sync_reason, "full");
    assert_eq!(retention_classes, vec!["git_backed".to_string()]);
}

#[tokio::test]
async fn auto_sync_marks_dirty_worktree_content_as_worktree_only_retention() {
    let repo = seed_full_sync_repo();
    fs::write(
        repo.path().join("src/lib.rs"),
        "pub fn greet(name: &str) -> String {\n    format!(\"hello {name}\")\n}\n",
    )
    .expect("rewrite rust source in worktree");

    let cfg = sync_test_cfg_for_repo(repo.path());
    let sqlite_path = repo.path().join("devql.sqlite");
    let relational = sqlite_relational_store_with_sync_schema(&sqlite_path).await;

    let summary = crate::host::devql::execute_sync(
        &cfg,
        &relational,
        crate::host::devql::sync::types::SyncMode::Auto,
    )
    .await
    .expect("execute auto sync with dirty worktree");

    assert_eq!(summary.mode, "full");

    let db = Connection::open(&sqlite_path).expect("open sqlite db");
    let effective_source: String = db
        .query_row(
            "SELECT effective_source FROM current_file_state WHERE repo_id = ?1 AND path = ?2",
            [cfg.repo.repo_id.as_str(), "src/lib.rs"],
            |row| row.get(0),
        )
        .expect("read current_file_state effective source");
    let retention_class: String = db
        .query_row(
            "SELECT retention_class \
             FROM content_cache c \
             JOIN current_file_state s \
               ON s.effective_content_id = c.content_id \
              AND s.language = c.language \
             WHERE s.repo_id = ?1 AND s.path = ?2",
            [cfg.repo.repo_id.as_str(), "src/lib.rs"],
            |row| row.get(0),
        )
        .expect("read content_cache retention class");

    assert_eq!(effective_source, "worktree");
    assert_eq!(retention_class, "worktree_only");
}

#[tokio::test]
async fn execute_sync_preserves_original_error_when_failed_status_write_fails() {
    let temp = tempdir().expect("temp dir");
    let sqlite_path = temp.path().join("devql.sqlite");
    let relational = sqlite_relational_store_with_sync_schema(&sqlite_path).await;
    let mut cfg = sync_test_cfg_for_repo(temp.path());
    cfg.repo_root = temp.path().join("missing-repo");

    let db = Connection::open(&sqlite_path).expect("open sqlite db");
    db.execute_batch(
        r#"
CREATE TRIGGER fail_sync_failed_status
BEFORE UPDATE OF last_sync_status ON repo_sync_state
WHEN NEW.last_sync_status = 'failed'
BEGIN
    SELECT RAISE(FAIL, 'forced write_sync_failed failure');
END;
"#,
    )
    .expect("create failing repo_sync_state trigger");

    let err = crate::host::devql::execute_sync(
        &cfg,
        &relational,
        crate::host::devql::sync::types::SyncMode::Full,
    )
    .await
    .expect_err("sync should fail when repo_root is not a git workspace");
    let message = format!("{err:#}");

    assert!(
        message.contains("inspecting workspace for DevQL sync"),
        "returned error should preserve the original inner sync failure: {message}"
    );
    assert!(
        !message.contains("forced write_sync_failed failure"),
        "write_sync_failed failure must not mask the original sync failure: {message}"
    );
}

#[tokio::test]
async fn sync_twice_with_no_changes_is_noop() {
    let repo = seed_full_sync_repo();
    let cfg = sync_test_cfg_for_repo(repo.path());
    let sqlite_path = repo.path().join("devql.sqlite");
    let relational = sqlite_relational_store_with_sync_schema(&sqlite_path).await;

    let first = crate::host::devql::execute_sync(
        &cfg,
        &relational,
        crate::host::devql::sync::types::SyncMode::Full,
    )
    .await
    .expect("execute initial full sync");
    assert_eq!(first.paths_added, 8);
    assert_eq!(first.paths_changed, 0);
    assert_eq!(first.paths_removed, 0);
    assert_eq!(first.paths_unchanged, 0);

    let db = Connection::open(&sqlite_path).expect("open sqlite db");
    let artefacts_before: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM artefacts_current WHERE repo_id = ?1",
            [cfg.repo.repo_id.as_str()],
            |row| row.get(0),
        )
        .expect("count artefacts before second sync");

    let second = crate::host::devql::execute_sync(
        &cfg,
        &relational,
        crate::host::devql::sync::types::SyncMode::Full,
    )
    .await
    .expect("execute second full sync");

    let artefacts_after: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM artefacts_current WHERE repo_id = ?1",
            [cfg.repo.repo_id.as_str()],
            |row| row.get(0),
        )
        .expect("count artefacts after second sync");

    assert_eq!(second.paths_unchanged, 8);
    assert_eq!(second.paths_added, 0);
    assert_eq!(second.paths_changed, 0);
    assert_eq!(second.paths_removed, 0);
    assert_eq!(second.cache_hits, 0);
    assert_eq!(second.cache_misses, 0);
    assert_eq!(artefacts_before, artefacts_after);
}
