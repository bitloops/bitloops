use super::*;
use serde_json::json;
use std::path::Path;

fn git_ok(repo_root: &Path, args: &[&str]) -> String {
    run_git(repo_root, args).expect("run git command")
}

fn seed_git_repo() -> tempfile::TempDir {
    let repo = tempfile::tempdir().expect("create temp dir");
    git_ok(repo.path(), &["init"]);
    git_ok(repo.path(), &["config", "user.email", "test@example.com"]);
    git_ok(repo.path(), &["config", "user.name", "Test User"]);
    repo
}

fn commit_file(repo_root: &Path, path: &str, content: &str, message: &str) -> String {
    let absolute = repo_root.join(path);
    if let Some(parent) = absolute.parent() {
        std::fs::create_dir_all(parent).expect("create parent dir");
    }
    std::fs::write(&absolute, content).expect("write file");
    git_ok(repo_root, &["add", path]);
    git_ok(repo_root, &["commit", "-m", message]);
    git_ok(repo_root, &["rev-parse", "HEAD"])
}

fn init_local_relational(repo_root: &Path) -> crate::host::devql::RelationalStorage {
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

#[test]
fn build_artefacts_replication_sql_targets_expected_columns() {
    let rows = vec![json!({
        "artefact_id": "a1",
        "symbol_id": "s1",
        "blob_sha": "b1",
        "path": "src/lib.rs",
        "language": "rust",
        "canonical_kind": "function",
        "language_kind": "function_item",
        "symbol_fqn": "src/lib.rs::run",
        "parent_artefact_id": null,
        "start_line": 1,
        "end_line": 3,
        "start_byte": 0,
        "end_byte": 10,
        "signature": "fn run()",
        "modifiers": "[]",
        "docstring": "test",
        "content_hash": "hash-1"
    })];

    let sql = history_replication::build_artefacts_replication_sql("repo-1", &rows).join("\n");
    assert!(sql.contains("INSERT INTO artefacts"));
    assert!(sql.contains("content_hash"));
    assert!(
        !sql.contains("created_at, created_at"),
        "artefacts replication SQL must not duplicate created_at columns"
    );
}

#[tokio::test]
async fn list_commits_to_sync_uses_branch_watermark_range_when_available() {
    let repo = seed_git_repo();
    let sha_a = commit_file(repo.path(), "src/a.ts", "export const a = 1;\n", "commit a");
    let sha_b = commit_file(repo.path(), "src/b.ts", "export const b = 2;\n", "commit b");
    let sha_c = commit_file(repo.path(), "src/c.ts", "export const c = 3;\n", "commit c");

    let local = init_local_relational(repo.path());
    let repo_id = "repo-watermark";
    sync_state::mark_branch_sync_complete(&local, repo_id, "origin", "main", &sha_b)
        .await
        .expect("mark watermark");

    let update = types::PrePushRefUpdate {
        local_ref: "refs/heads/main".to_string(),
        local_sha: sha_c.clone(),
        remote_ref: "refs/heads/main".to_string(),
        remote_sha: sha_a,
        local_branch: Some("main".to_string()),
        remote_branch: "main".to_string(),
    };

    let commits = commit_selection::list_commits_to_sync_for_ref_update(
        repo.path(),
        &local,
        repo_id,
        "origin",
        &update,
    )
    .await
    .expect("list commits to sync");

    assert_eq!(
        commits,
        vec![sha_c],
        "watermark sync should only enqueue commits newer than the branch watermark"
    );
}

#[tokio::test]
async fn list_commits_to_sync_for_new_branch_falls_back_to_local_head_when_no_remote_history() {
    let repo = seed_git_repo();
    let head = commit_file(
        repo.path(),
        "src/new_branch.ts",
        "export const x = 1;\n",
        "head",
    );

    let local = init_local_relational(repo.path());
    let update = types::PrePushRefUpdate {
        local_ref: "refs/heads/feature/new".to_string(),
        local_sha: head.clone(),
        remote_ref: "refs/heads/feature/new".to_string(),
        remote_sha: constants::ZERO_GIT_OID.to_string(),
        local_branch: Some("feature/new".to_string()),
        remote_branch: "feature/new".to_string(),
    };

    let commits = commit_selection::list_commits_to_sync_for_ref_update(
        repo.path(),
        &local,
        "repo-new-branch",
        "origin",
        &update,
    )
    .await
    .expect("list commits to sync for new branch");

    assert!(
        commits.contains(&head),
        "new branch sync should include local head when remote branch does not exist"
    );
}
