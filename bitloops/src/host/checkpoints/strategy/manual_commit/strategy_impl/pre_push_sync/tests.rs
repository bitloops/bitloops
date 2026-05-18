use super::*;

fn init_local_relational(repo_root: &std::path::Path) -> crate::host::devql::RelationalStorage {
    let sqlite_path = repo_root.join("relational.db");
    let sqlite = crate::storage::SqliteConnectionPool::connect(sqlite_path.clone())
        .expect("create sqlite connection pool");
    sqlite
        .initialise_devql_schema()
        .expect("initialise devql schema");
    crate::host::devql::RelationalStorage::local_only(sqlite_path)
}

#[test]
fn parse_pre_push_update_line_accepts_branch_refs() {
    let line = "refs/heads/main abc123 refs/heads/main def456";
    let parsed = parsing::parse_pre_push_update_line(line).expect("parse branch update");
    assert_eq!(parsed.local_ref, "refs/heads/main");
    assert_eq!(parsed.remote_ref, "refs/heads/main");
    assert_eq!(parsed.local_branch.as_deref(), Some("main"));
    assert_eq!(parsed.remote_branch, "main");
}

#[test]
fn parse_pre_push_update_line_rejects_non_branch_remote_ref() {
    let line = "refs/heads/main abc123 refs/tags/v1 def456";
    assert!(
        parsing::parse_pre_push_update_line(line).is_none(),
        "tag pushes should be ignored by pre-push replication"
    );
}

#[tokio::test]
async fn record_remote_branch_sync_watermarks_marks_remote_branch_complete() {
    let repo = tempfile::tempdir().expect("create temp dir");
    let local = init_local_relational(repo.path());
    let repo_id = "repo-watermark";
    let update = types::PrePushRefUpdate {
        local_ref: "refs/heads/main".to_string(),
        local_sha: "abc123".to_string(),
        remote_ref: "refs/heads/main".to_string(),
        remote_sha: "def456".to_string(),
        local_branch: Some("main".to_string()),
        remote_branch: "main".to_string(),
    };

    runtime::record_remote_branch_sync_watermarks(&local, repo_id, "origin", &[update])
        .await
        .expect("record remote branch sync watermarks");

    let rows = local
        .query_rows(&format!(
            "SELECT state_key, state_value FROM sync_state WHERE repo_id = '{}' ORDER BY state_key",
            crate::host::devql::esc_pg(repo_id),
        ))
        .await
        .expect("read sync_state rows");
    let state: std::collections::BTreeMap<String, String> = rows
        .into_iter()
        .filter_map(|row| {
            Some((
                row.get("state_key")?.as_str()?.to_string(),
                row.get("state_value")?.as_str()?.to_string(),
            ))
        })
        .collect();

    assert_eq!(
        state.get(constants::PRE_PUSH_SYNC_WATERMARK_KEY),
        Some(&"abc123".to_string())
    );
    assert_eq!(
        state.get("last_synced_commit_sha:origin:main"),
        Some(&"abc123".to_string())
    );
    assert!(
        !state.contains_key("pending_remote_sync_sha:origin:main"),
        "remote pre-push bookkeeping should not retain a legacy pending sync row"
    );
}
