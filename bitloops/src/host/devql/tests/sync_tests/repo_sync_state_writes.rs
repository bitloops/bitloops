use tempfile::tempdir;

use super::fixtures::{
    seed_sync_repository_catalog_row, sqlite_relational_store_with_sync_schema, sync_test_cfg,
};

#[tokio::test]
async fn repo_sync_state_write_helpers_track_lifecycle() {
    let temp = tempdir().expect("temp dir");
    let sqlite_path = temp.path().join("devql.sqlite");
    let relational = sqlite_relational_store_with_sync_schema(&sqlite_path).await;
    let cfg = sync_test_cfg();
    let expected_repo_root = cfg.repo_root.to_string_lossy().to_string();
    seed_sync_repository_catalog_row(&relational, &cfg).await;

    crate::host::devql::sync::state::write_sync_started(
        &relational,
        &cfg.repo.repo_id,
        cfg.repo_root.to_string_lossy().as_ref(),
        "full",
        "parser-v1",
        "extractor-v1",
    )
    .await
    .expect("write started state");

    let started_rows = relational
        .query_rows(&format!(
            "SELECT repo_root, active_branch, head_commit_sha, head_tree_sha, parser_version, extractor_version, last_sync_started_at, last_sync_completed_at, last_sync_status, last_sync_reason \
FROM repo_sync_state WHERE repo_id = '{}'",
            crate::host::devql::db_utils::esc_pg(&cfg.repo.repo_id),
        ))
        .await
        .expect("query started sync state");
    let started = started_rows
        .first()
        .and_then(serde_json::Value::as_object)
        .expect("started row");
    assert_eq!(
        started.get("repo_root").and_then(|v| v.as_str()),
        Some(expected_repo_root.as_str())
    );
    assert_eq!(started.get("active_branch").and_then(|v| v.as_str()), None);
    assert_eq!(
        started.get("head_commit_sha").and_then(|v| v.as_str()),
        None
    );
    assert_eq!(started.get("head_tree_sha").and_then(|v| v.as_str()), None);
    assert_eq!(
        started.get("parser_version").and_then(|v| v.as_str()),
        Some("parser-v1")
    );
    assert_eq!(
        started.get("extractor_version").and_then(|v| v.as_str()),
        Some("extractor-v1")
    );
    assert!(
        started
            .get("last_sync_started_at")
            .and_then(|v| v.as_str())
            .is_some(),
        "started timestamp should be written"
    );
    assert_eq!(
        started
            .get("last_sync_completed_at")
            .and_then(|v| v.as_str()),
        None
    );
    assert_eq!(
        started.get("last_sync_status").and_then(|v| v.as_str()),
        Some("running")
    );
    assert_eq!(
        started.get("last_sync_reason").and_then(|v| v.as_str()),
        Some("full")
    );

    crate::host::devql::sync::state::write_sync_completed(
        &relational,
        &cfg.repo.repo_id,
        Some("head-123"),
        Some("tree-456"),
        Some("main"),
        "parser-v1",
        "extractor-v1",
    )
    .await
    .expect("write completed state");

    let completed_rows = relational
        .query_rows(&format!(
            "SELECT repo_root, active_branch, head_commit_sha, head_tree_sha, parser_version, extractor_version, last_sync_started_at, last_sync_completed_at, last_sync_status, last_sync_reason \
FROM repo_sync_state WHERE repo_id = '{}'",
            crate::host::devql::db_utils::esc_pg(&cfg.repo.repo_id),
        ))
        .await
        .expect("query completed sync state");
    let completed = completed_rows
        .first()
        .and_then(serde_json::Value::as_object)
        .expect("completed row");
    assert_eq!(
        completed.get("repo_root").and_then(|v| v.as_str()),
        Some(expected_repo_root.as_str())
    );
    assert_eq!(
        completed.get("active_branch").and_then(|v| v.as_str()),
        Some("main")
    );
    assert_eq!(
        completed.get("head_commit_sha").and_then(|v| v.as_str()),
        Some("head-123")
    );
    assert_eq!(
        completed.get("head_tree_sha").and_then(|v| v.as_str()),
        Some("tree-456")
    );
    assert_eq!(
        completed.get("last_sync_status").and_then(|v| v.as_str()),
        Some("completed")
    );
    assert!(
        completed
            .get("last_sync_completed_at")
            .and_then(|v| v.as_str())
            .is_some(),
        "completed timestamp should be written"
    );
    assert_eq!(
        completed.get("last_sync_reason").and_then(|v| v.as_str()),
        Some("full")
    );
}

#[tokio::test]
async fn repo_sync_state_write_failed_marks_repo_as_failed() {
    let temp = tempdir().expect("temp dir");
    let sqlite_path = temp.path().join("devql.sqlite");
    let relational = sqlite_relational_store_with_sync_schema(&sqlite_path).await;
    let cfg = sync_test_cfg();
    seed_sync_repository_catalog_row(&relational, &cfg).await;

    crate::host::devql::sync::state::write_sync_started(
        &relational,
        &cfg.repo.repo_id,
        cfg.repo_root.to_string_lossy().as_ref(),
        "repair",
        "parser-v1",
        "extractor-v1",
    )
    .await
    .expect("write started state");

    crate::host::devql::sync::state::write_sync_failed(&relational, &cfg.repo.repo_id)
        .await
        .expect("write failed state");

    let rows = relational
        .query_rows(&format!(
            "SELECT last_sync_status, last_sync_reason, last_sync_started_at FROM repo_sync_state WHERE repo_id = '{}'",
            crate::host::devql::db_utils::esc_pg(&cfg.repo.repo_id),
        ))
        .await
        .expect("query failed sync state");
    let row = rows
        .first()
        .and_then(serde_json::Value::as_object)
        .expect("failed row");
    assert_eq!(
        row.get("last_sync_status").and_then(|v| v.as_str()),
        Some("failed")
    );
    assert_eq!(
        row.get("last_sync_reason").and_then(|v| v.as_str()),
        Some("repair")
    );
    assert!(
        row.get("last_sync_started_at")
            .and_then(|v| v.as_str())
            .is_some(),
        "failed write should preserve started timestamp"
    );
}

#[tokio::test]
async fn repo_sync_state_write_completed_errors_without_started_row() {
    let temp = tempdir().expect("temp dir");
    let sqlite_path = temp.path().join("devql.sqlite");
    let relational = sqlite_relational_store_with_sync_schema(&sqlite_path).await;
    let cfg = sync_test_cfg();
    seed_sync_repository_catalog_row(&relational, &cfg).await;

    let err = crate::host::devql::sync::state::write_sync_completed(
        &relational,
        &cfg.repo.repo_id,
        Some("head-123"),
        Some("tree-456"),
        Some("main"),
        "parser-v1",
        "extractor-v1",
    )
    .await
    .expect_err("missing repo_sync_state row should error");

    assert!(
        err.to_string().contains("repo_sync_state"),
        "error should explain the missing sync state row"
    );
}

#[tokio::test]
async fn repo_sync_state_write_failed_errors_without_started_row() {
    let temp = tempdir().expect("temp dir");
    let sqlite_path = temp.path().join("devql.sqlite");
    let relational = sqlite_relational_store_with_sync_schema(&sqlite_path).await;
    let cfg = sync_test_cfg();
    seed_sync_repository_catalog_row(&relational, &cfg).await;

    let err = crate::host::devql::sync::state::write_sync_failed(&relational, &cfg.repo.repo_id)
        .await
        .expect_err("missing repo_sync_state row should error");

    assert!(
        err.to_string().contains("repo_sync_state"),
        "error should explain the missing sync state row"
    );
}

#[tokio::test]
async fn repo_sync_state_scope_exclusions_fingerprint_round_trips() {
    let temp = tempdir().expect("temp dir");
    let sqlite_path = temp.path().join("devql.sqlite");
    let relational = sqlite_relational_store_with_sync_schema(&sqlite_path).await;
    let cfg = sync_test_cfg();
    seed_sync_repository_catalog_row(&relational, &cfg).await;

    crate::host::devql::sync::state::write_sync_started(
        &relational,
        &cfg.repo.repo_id,
        cfg.repo_root.to_string_lossy().as_ref(),
        "full",
        "parser-v1",
        "extractor-v1",
    )
    .await
    .expect("write started state");

    crate::host::devql::sync::state::write_scope_exclusions_fingerprint(
        &relational,
        &cfg.repo.repo_id,
        "fingerprint-123",
    )
    .await
    .expect("write scope exclusions fingerprint");

    let stored = crate::host::devql::sync::state::read_scope_exclusions_fingerprint(
        &relational,
        &cfg.repo.repo_id,
    )
    .await
    .expect("read scope exclusions fingerprint");
    assert_eq!(stored.as_deref(), Some("fingerprint-123"));
}
