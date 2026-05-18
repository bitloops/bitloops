use std::collections::HashMap;
use std::fs;

use tempfile::TempDir;

use super::*;

fn git_ok(repo_root: &std::path::Path, args: &[&str]) -> String {
    run_git(repo_root, args).unwrap_or_else(|err| panic!("git {:?} failed: {err}", args))
}

fn checkpoint_sqlite_path(repo_root: &std::path::Path) -> std::path::PathBuf {
    let cfg = crate::config::resolve_store_backend_config_for_repo(repo_root)
        .expect("resolve backend config");
    let path = cfg
        .relational
        .sqlite_path
        .as_deref()
        .expect("test daemon config should set sqlite_path");
    crate::config::resolve_sqlite_db_path_for_repo(repo_root, Some(path))
        .expect("resolve configured sqlite path")
}

fn ensure_checkpoint_schema(repo_root: &std::path::Path) {
    let sqlite_path = checkpoint_sqlite_path(repo_root);
    let sqlite =
        crate::storage::SqliteConnectionPool::connect(sqlite_path).expect("open checkpoint sqlite");
    sqlite
        .initialise_checkpoint_schema()
        .expect("initialise checkpoint schema");
}

fn insert_commit_checkpoint_mapping(
    repo_root: &std::path::Path,
    commit_sha: &str,
    checkpoint_id: &str,
) {
    ensure_checkpoint_schema(repo_root);
    let sqlite_path = checkpoint_sqlite_path(repo_root);
    let sqlite =
        crate::storage::SqliteConnectionPool::connect(sqlite_path).expect("open checkpoint sqlite");
    let repo_id = crate::host::devql::resolve_repo_id(repo_root).expect("resolve repo id");
    sqlite
        .with_write_connection(|conn| {
            conn.execute(
                "INSERT INTO commit_checkpoints (commit_sha, checkpoint_id, repo_id)
                 VALUES (?1, ?2, ?3)",
                rusqlite::params![commit_sha, checkpoint_id, repo_id.as_str()],
            )?;
            Ok(())
        })
        .expect("insert commit checkpoint mapping");
}

fn setup_git_repo() -> TempDir {
    let dir = tempfile::tempdir().expect("temp dir");
    git_ok(dir.path(), &["init"]);
    git_ok(dir.path(), &["checkout", "-B", "main"]);
    git_ok(dir.path(), &["config", "user.name", "Explain Test"]);
    git_ok(
        dir.path(),
        &["config", "user.email", "explain-test@example.com"],
    );
    crate::test_support::git_fixtures::write_test_daemon_config(dir.path());
    dir
}

fn commit_file(repo_root: &std::path::Path, path: &str, content: &str, message: &str) -> String {
    fs::write(repo_root.join(path), content).expect("write file");
    git_ok(repo_root, &["add", path]);
    git_ok(repo_root, &["commit", "-m", message]);
    git_ok(repo_root, &["rev-parse", "HEAD"])
}

#[test]
fn short_display_id_handles_empty_short_and_long_values() {
    assert_eq!(short_display_id(""), "");
    assert_eq!(short_display_id("abc123"), "abc123");
    assert_eq!(short_display_id("abcdef123456"), "abcdef1");
}

#[test]
fn build_runtime_session_info_returns_none_for_empty_filter() {
    assert!(build_runtime_session_info(&[], "", &[]).is_none());
}

#[test]
fn build_runtime_session_info_returns_additional_session_when_points_do_not_match() {
    let additional = vec![SessionInfo {
        id: "session-alpha".to_string(),
        description: "Recovered session".to_string(),
        strategy: "manual-commit".to_string(),
        start_time: "2026-03-01T08:00:00Z".to_string(),
        checkpoints: vec![SessionCheckpoint {
            checkpoint_id: "chk-existing".to_string(),
            message: "Existing checkpoint".to_string(),
            timestamp: "2026-03-01T08:00:00Z".to_string(),
        }],
    }];

    let (session, details) = build_runtime_session_info(&[], "session-a", &additional)
        .expect("matching additional session should be returned");

    assert_eq!(session, additional[0]);
    assert!(details.is_empty());
}

#[test]
fn build_runtime_session_info_synthesises_session_details_from_rewind_points() {
    let points = vec![
        RewindPoint {
            id: "bbbbbbbb22222222".to_string(),
            message: "Newest checkpoint".to_string(),
            date: "2026-03-03T12:00:00Z".to_string(),
            checkpoint_id: "chk-newest".to_string(),
            session_id: "session-alpha".to_string(),
            session_prompt: "Second prompt".to_string(),
            is_task_checkpoint: true,
            ..RewindPoint::default()
        },
        RewindPoint {
            id: "aaaaaaaa11111111".to_string(),
            message: "Oldest checkpoint".to_string(),
            date: "2026-03-01T09:00:00Z".to_string(),
            checkpoint_id: String::new(),
            session_id: "session-alpha".to_string(),
            session_prompt: "First prompt".to_string(),
            is_task_checkpoint: false,
            ..RewindPoint::default()
        },
    ];

    let (session, details) = build_runtime_session_info(&points, "session-a", &[])
        .expect("matching rewind points should synthesise runtime session info");

    assert_eq!(session.id, "session-alpha");
    assert_eq!(session.strategy, "manual-commit");
    assert_eq!(session.start_time, "2026-03-01T09:00:00Z");
    assert_eq!(session.checkpoints.len(), 2);
    assert_eq!(session.checkpoints[0].checkpoint_id, "chk-newest");
    assert_eq!(session.checkpoints[1].checkpoint_id, "aaaaaaaa11111111");

    assert_eq!(details.len(), 2);
    assert_eq!(details[0].index, 2);
    assert_eq!(details[0].short_id, "chk-new");
    assert!(details[0].is_task_checkpoint);
    assert_eq!(details[0].interactions[0].prompt, "Second prompt");

    assert_eq!(details[1].index, 1);
    assert_eq!(details[1].short_id, "aaaaaaa");
    assert_eq!(details[1].interactions[0].prompt, "First prompt");
}

#[test]
fn resolve_branch_display_name_handles_empty_and_named_branches() {
    let no_commits = setup_git_repo();
    assert_eq!(
        resolve_branch_display_name(no_commits.path()),
        "HEAD (no commits yet)"
    );

    let named = setup_git_repo();
    commit_file(named.path(), "README.md", "seed", "initial commit");
    git_ok(named.path(), &["checkout", "-b", "feature/branch-tests"]);
    assert_eq!(
        resolve_branch_display_name(named.path()),
        "feature/branch-tests"
    );
}

#[test]
fn compute_reachable_from_default_branch_returns_empty_on_default_branch_and_uses_custom_name() {
    let repo = setup_git_repo();
    assert!(
        compute_reachable_from_default_branch(repo.path(), "main", true).is_empty(),
        "default branch should not compute a reachable-main set"
    );

    git_ok(repo.path(), &["checkout", "-B", "trunk"]);
    let trunk_sha = commit_file(repo.path(), "README.md", "seed", "initial commit");
    git_ok(repo.path(), &["checkout", "-b", "feature/custom-default"]);

    let reachable = compute_reachable_from_default_branch(repo.path(), "trunk", false);
    assert!(
        reachable.contains(&trunk_sha),
        "expected traversal through the resolved default branch to include the default-branch head"
    );
}

#[test]
fn get_associated_commits_from_db_uses_db_mappings_and_search_all() {
    let repo = setup_git_repo();
    commit_file(repo.path(), "README.md", "seed", "initial commit");

    let checkpoint_id = "aabb11223344";
    let feature_sha = "cccc000000000000000000000000000000000000";
    insert_commit_checkpoint_mapping(repo.path(), feature_sha, checkpoint_id);

    let commits = vec![
        CommitNode {
            sha: "aaaa000000000000000000000000000000000000".to_string(),
            message: "Merge feature into main".to_string(),
            parents: vec![
                "bbbb000000000000000000000000000000000000".to_string(),
                feature_sha.to_string(),
            ],
            timestamp: 4,
            author: "Test".to_string(),
            ..CommitNode::default()
        },
        CommitNode {
            sha: "bbbb000000000000000000000000000000000000".to_string(),
            message: "main: parallel work".to_string(),
            parents: vec!["dddd000000000000000000000000000000000000".to_string()],
            timestamp: 3,
            author: "Test".to_string(),
            ..CommitNode::default()
        },
        CommitNode {
            sha: feature_sha.to_string(),
            message: "feat: add feature".to_string(),
            parents: vec!["dddd000000000000000000000000000000000000".to_string()],
            timestamp: 2,
            author: "Feature Dev".to_string(),
            ..CommitNode::default()
        },
    ];

    let first_parent_only =
        get_associated_commits_from_db(repo.path(), &commits, checkpoint_id, false)
            .expect("first-parent DB scan should succeed");
    assert!(first_parent_only.is_empty());

    let search_all = get_associated_commits_from_db(repo.path(), &commits, checkpoint_id, true)
        .expect("search-all DB scan should succeed");
    assert_eq!(search_all.len(), 1);
    assert_eq!(search_all[0].sha, feature_sha);
    assert_eq!(search_all[0].author, "Feature Dev");
}

#[test]
fn walk_first_parent_commits_errors_when_head_is_missing() {
    let commit_map = HashMap::new();
    let err = walk_first_parent_commits("missing", &commit_map, 10)
        .expect_err("missing head commit should fail");
    assert!(format!("{err:#}").contains("failed to get commit missing"));
}
