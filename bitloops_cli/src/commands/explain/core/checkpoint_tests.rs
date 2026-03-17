use super::*;
use tempfile::TempDir;

fn git_ok(repo_root: &std::path::Path, args: &[&str]) -> String {
    run_git(repo_root, args).unwrap_or_else(|err| panic!("git {:?} failed: {err}", args))
}

fn seed_repo_with_single_commit(message: &str) -> (TempDir, String) {
    let dir = tempfile::tempdir().expect("temp dir");
    git_ok(dir.path(), &["init"]);
    git_ok(dir.path(), &["checkout", "-B", "main"]);
    git_ok(dir.path(), &["config", "user.name", "Explain Test"]);
    git_ok(
        dir.path(),
        &["config", "user.email", "explain-test@example.com"],
    );
    std::fs::write(dir.path().join("README.md"), "seed").expect("write readme");
    git_ok(dir.path(), &["add", "README.md"]);
    git_ok(dir.path(), &["commit", "-m", message]);
    let head_sha = git_ok(dir.path(), &["rev-parse", "HEAD"]);
    (dir, head_sha)
}

fn insert_commit_checkpoint_mapping(
    repo_root: &std::path::Path,
    commit_sha: &str,
    checkpoint_id: &str,
) {
    let sqlite_path = checkpoint_sqlite_path(repo_root);
    let sqlite = crate::engine::db::SqliteConnectionPool::connect(sqlite_path)
        .expect("open checkpoint sqlite");
    sqlite
        .initialise_checkpoint_schema()
        .expect("initialise checkpoint schema");
    let repo_id = crate::engine::devql::resolve_repo_id(repo_root).expect("resolve repo id");
    sqlite
        .with_connection(|conn| {
            conn.execute(
                "INSERT INTO commit_checkpoints (commit_sha, checkpoint_id, repo_id)
                 VALUES (?1, ?2, ?3)",
                rusqlite::params![commit_sha, checkpoint_id, repo_id.as_str()],
            )?;
            Ok(())
        })
        .expect("insert commit checkpoint mapping");
}

fn checkpoint_sqlite_path(repo_root: &std::path::Path) -> std::path::PathBuf {
    let cfg = crate::store_config::resolve_store_backend_config_for_repo(repo_root)
        .expect("resolve backend config");
    if let Some(path) = cfg.relational.sqlite_path.as_deref() {
        crate::store_config::resolve_sqlite_db_path_for_repo(repo_root, Some(path))
            .expect("resolve configured sqlite path")
    } else {
        crate::engine::paths::default_relational_db_path(repo_root)
    }
}

#[test]
fn agent_type_from_str_maps_codex_to_codex() {
    assert_eq!(
        agent_type_from_str(crate::engine::agent::AGENT_TYPE_CODEX),
        AgentType::Codex
    );
}

#[test]
fn metadata_from_json_sets_codex_agent_type() {
    let meta = serde_json::json!({
        "session_id": "session-1",
        "created_at": "2026-03-12T00:00:00Z",
        "files_touched": ["src/main.rs"],
        "checkpoints_count": 1,
        "checkpoint_transcript_start": 0,
        "agent": crate::engine::agent::AGENT_TYPE_CODEX,
    });

    let parsed = metadata_from_json(&meta, "cp-1");

    assert_eq!(parsed.agent_type, AgentType::Codex);
}

#[test]
fn metadata_from_json_unknown_agent_defaults_to_claude() {
    let meta = serde_json::json!({
        "agent": "unknown-agent"
    });

    let parsed = metadata_from_json(&meta, "cp-2");

    assert_eq!(parsed.agent_type, AgentType::ClaudeCode);
}

#[test]
fn build_commit_graph_from_git_reads_checkpoint_from_db_mapping() {
    let checkpoint_id = "aabbccddeeff";
    let (repo, commit_sha) = seed_repo_with_single_commit("checkpoint without trailer");
    insert_commit_checkpoint_mapping(repo.path(), &commit_sha, checkpoint_id);

    let commits = build_commit_graph_from_git(repo.path(), 50).expect("build commit graph");
    let associated = get_associated_commits(&commits, checkpoint_id, true)
        .expect("resolve associated commits");

    assert_eq!(associated.len(), 1, "expected commit mapped from SQLite");
    assert_eq!(associated[0].sha, commit_sha);
    assert_eq!(associated[0].message, "checkpoint without trailer");
}
