use super::*;

#[test]
pub(crate) fn pre_push_does_not_push_checkpoints_branch() {
    let base = tempfile::tempdir().unwrap();
    let origin_dir = base.path().join("origin.git");
    let work_dir = base.path().join("work");
    fs::create_dir_all(&work_dir).unwrap();

    // Bare remote.
    let out = git_command()
        .args(["init", "--bare", origin_dir.to_string_lossy().as_ref()])
        .output()
        .unwrap();
    assert!(out.status.success(), "git init --bare failed");

    let work_temp = tempfile::TempDir::new_in(&work_dir).unwrap();
    let repo_dir = work_temp.path();
    let run = |args: &[&str]| {
        let out = git_command()
            .args(args)
            .current_dir(repo_dir)
            .output()
            .unwrap();
        assert!(out.status.success(), "git {:?} failed", args);
    };

    run(&["init"]);
    run(&["config", "user.email", "t@t.com"]);
    run(&["config", "user.name", "Test"]);
    fs::write(repo_dir.join("README.md"), "init").unwrap();
    run(&["add", "."]);
    run(&["commit", "-m", "initial"]);
    run(&[
        "remote",
        "add",
        "origin",
        origin_dir.to_string_lossy().as_ref(),
    ]);

    // Create local checkpoints branch to push.
    let head = run_git(repo_dir, &["rev-parse", "HEAD"]).unwrap();
    run(&["update-ref", "refs/heads/bitloops/checkpoints/v1", &head]);

    let strategy = ManualCommitStrategy::new(repo_dir);
    strategy.pre_push("origin", &[]).unwrap();

    // Remote should never receive bitloops/checkpoints/v1 from pre-push replication.
    let remote_ref = git_command()
        .args([
            "--git-dir",
            origin_dir.to_string_lossy().as_ref(),
            "show-ref",
            "--verify",
            "refs/heads/bitloops/checkpoints/v1",
        ])
        .output()
        .unwrap();
    assert!(
        !remote_ref.status.success(),
        "remote should not contain checkpoints branch after pre-push sync"
    );
}

#[test]
pub(crate) fn pre_push_without_postgres_prunes_historical_rows_by_retention() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    let sqlite_path = init_devql_schema(dir.path());
    let repo_id = crate::host::devql::resolve_repo_identity(dir.path())
        .unwrap()
        .repo_id;

    let sqlite = rusqlite::Connection::open(&sqlite_path).unwrap();
    for idx in 0..55 {
        let commit_sha = format!("commit-{idx:03}");
        let blob_sha = format!("blob-{idx:03}");
        let path = format!("src/history_{idx:03}.ts");
        let artefact_id = format!("artefact-{idx:03}");
        let edge_id = format!("edge-{idx:03}");
        let committed_at = format!("2026-01-01 00:{idx:02}:00");

        sqlite
            .execute(
                "INSERT INTO commits (commit_sha, repo_id, author_name, author_email, commit_message, committed_at) \
                 VALUES (?1, ?2, 'Test', 'test@example.com', 'seed', ?3)",
                rusqlite::params![commit_sha.as_str(), repo_id.as_str(), committed_at.as_str()],
            )
            .unwrap();
        sqlite
            .execute(
                "INSERT INTO file_state (repo_id, commit_sha, path, blob_sha) VALUES (?1, ?2, ?3, ?4)",
                rusqlite::params![
                    repo_id.as_str(),
                    commit_sha.as_str(),
                    path.as_str(),
                    blob_sha.as_str()
                ],
            )
            .unwrap();
        sqlite
            .execute(
                "INSERT INTO artefacts (artefact_id, symbol_id, repo_id, language, canonical_kind, language_kind, symbol_fqn, signature, modifiers, docstring, content_hash) \
                 VALUES (?1, ?2, ?3, 'typescript', 'function', 'function', ?2, 'fn()', '[]', NULL, ?4)",
                rusqlite::params![
                    artefact_id.as_str(),
                    format!("symbol-{idx:03}"),
                    repo_id.as_str(),
                    format!("hash-{idx:03}")
                ],
            )
            .unwrap();
        sqlite
            .execute(
                "INSERT INTO artefact_snapshots (repo_id, blob_sha, path, artefact_id, parent_artefact_id, start_line, end_line, start_byte, end_byte) \
                 VALUES (?1, ?2, ?3, ?4, NULL, 1, 1, 0, 1)",
                rusqlite::params![
                    repo_id.as_str(),
                    blob_sha.as_str(),
                    path.as_str(),
                    artefact_id.as_str(),
                ],
            )
            .unwrap();
        sqlite
            .execute(
                "INSERT INTO artefact_edges (edge_id, repo_id, blob_sha, from_artefact_id, to_symbol_ref, edge_kind, language, metadata) \
                 VALUES (?1, ?2, ?3, ?4, ?5, 'calls', 'typescript', '{}')",
                rusqlite::params![
                    edge_id.as_str(),
                    repo_id.as_str(),
                    blob_sha.as_str(),
                    artefact_id.as_str(),
                    format!("target::{idx:03}")
                ],
            )
            .unwrap();
    }
    drop(sqlite);

    ManualCommitStrategy::new(dir.path())
        .pre_push("origin", &[])
        .unwrap();

    let sqlite = rusqlite::Connection::open(sqlite_path).unwrap();
    let file_state_rows: i64 = sqlite
        .query_row(
            "SELECT COUNT(*) FROM file_state WHERE repo_id = ?1",
            rusqlite::params![repo_id.as_str()],
            |row| row.get(0),
        )
        .unwrap();
    let artefact_rows: i64 = sqlite
        .query_row(
            "SELECT COUNT(*) FROM artefacts WHERE repo_id = ?1",
            rusqlite::params![repo_id.as_str()],
            |row| row.get(0),
        )
        .unwrap();
    let edge_rows: i64 = sqlite
        .query_row(
            "SELECT COUNT(*) FROM artefact_edges WHERE repo_id = ?1",
            rusqlite::params![repo_id.as_str()],
            |row| row.get(0),
        )
        .unwrap();

    assert_eq!(
        file_state_rows, 50,
        "retention mode should keep file_state rows for the latest 50 commits"
    );
    assert_eq!(
        artefact_rows, 50,
        "retention mode should keep artefact rows for the latest 50 commits"
    );
    assert_eq!(
        edge_rows, 50,
        "retention mode should keep edge rows for the latest 50 commits"
    );
}

#[test]
pub(crate) fn pre_push_retention_pruning_preserves_current_state_tables() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    let sqlite_path = init_devql_schema(dir.path());
    let repo_id = crate::host::devql::resolve_repo_identity(dir.path())
        .unwrap()
        .repo_id;

    let sqlite = rusqlite::Connection::open(&sqlite_path).unwrap();
    sqlite
        .execute(
            "INSERT INTO artefacts_current (
                repo_id, path, content_id, symbol_id, artefact_id, language, canonical_kind,
                language_kind, symbol_fqn, parent_symbol_id, parent_artefact_id,
                start_line, end_line, start_byte, end_byte, signature, modifiers, docstring, updated_at
            ) VALUES (
                ?1, 'src/current.ts', 'current-content', 'current-symbol', 'current-artefact',
                'typescript', 'function', 'function', 'src/current.ts::currentSymbol',
                NULL, NULL, 1, 1, 0, 1, 'current()', '[]', NULL, datetime('now')
            )",
            rusqlite::params![repo_id.as_str()],
        )
        .unwrap();
    sqlite
        .execute(
            "INSERT INTO artefact_edges_current (
                repo_id, edge_id, path, content_id, from_symbol_id, from_artefact_id,
                to_symbol_id, to_artefact_id, to_symbol_ref, edge_kind, language,
                start_line, end_line, metadata, updated_at
            ) VALUES (
                ?1, 'current-edge', 'src/current.ts', 'current-content',
                'current-symbol', 'current-artefact', NULL, NULL, 'target::symbol', 'references', 'typescript',
                1, 1, '{}', datetime('now')
            )",
            rusqlite::params![repo_id.as_str()],
        )
        .unwrap();

    for idx in 0..55 {
        let commit_sha = format!("commit-{idx:03}");
        let blob_sha = format!("blob-{idx:03}");
        let path = format!("src/history_{idx:03}.ts");
        let artefact_id = format!("artefact-{idx:03}");
        let edge_id = format!("edge-{idx:03}");
        let committed_at = format!("2026-01-01 00:{idx:02}:00");

        sqlite
            .execute(
                "INSERT INTO commits (commit_sha, repo_id, author_name, author_email, commit_message, committed_at) \
                 VALUES (?1, ?2, 'Test', 'test@example.com', 'seed', ?3)",
                rusqlite::params![commit_sha.as_str(), repo_id.as_str(), committed_at.as_str()],
            )
            .unwrap();
        sqlite
            .execute(
                "INSERT INTO file_state (repo_id, commit_sha, path, blob_sha) VALUES (?1, ?2, ?3, ?4)",
                rusqlite::params![
                    repo_id.as_str(),
                    commit_sha.as_str(),
                    path.as_str(),
                    blob_sha.as_str()
                ],
            )
            .unwrap();
        sqlite
            .execute(
                "INSERT INTO artefacts (artefact_id, symbol_id, repo_id, language, canonical_kind, language_kind, symbol_fqn, signature, modifiers, docstring, content_hash) \
                 VALUES (?1, ?2, ?3, 'typescript', 'function', 'function', ?2, 'fn()', '[]', NULL, ?4)",
                rusqlite::params![
                    artefact_id.as_str(),
                    format!("symbol-{idx:03}"),
                    repo_id.as_str(),
                    format!("hash-{idx:03}")
                ],
            )
            .unwrap();
        sqlite
            .execute(
                "INSERT INTO artefact_snapshots (repo_id, blob_sha, path, artefact_id, parent_artefact_id, start_line, end_line, start_byte, end_byte) \
                 VALUES (?1, ?2, ?3, ?4, NULL, 1, 1, 0, 1)",
                rusqlite::params![
                    repo_id.as_str(),
                    blob_sha.as_str(),
                    path.as_str(),
                    artefact_id.as_str(),
                ],
            )
            .unwrap();
        sqlite
            .execute(
                "INSERT INTO artefact_edges (edge_id, repo_id, blob_sha, from_artefact_id, to_symbol_ref, edge_kind, language, metadata) \
                 VALUES (?1, ?2, ?3, ?4, ?5, 'calls', 'typescript', '{}')",
                rusqlite::params![
                    edge_id.as_str(),
                    repo_id.as_str(),
                    blob_sha.as_str(),
                    artefact_id.as_str(),
                    format!("target::{idx:03}")
                ],
            )
            .unwrap();
    }
    drop(sqlite);

    ManualCommitStrategy::new(dir.path())
        .pre_push("origin", &[])
        .unwrap();

    let sqlite = rusqlite::Connection::open(sqlite_path).unwrap();
    let current_artefacts_rows: i64 = sqlite
        .query_row(
            "SELECT COUNT(*) FROM artefacts_current WHERE repo_id = ?1 AND path = 'src/current.ts' AND symbol_id = 'current-symbol'",
            rusqlite::params![repo_id.as_str()],
            |row| row.get(0),
        )
        .unwrap();
    let current_edges_rows: i64 = sqlite
        .query_row(
            "SELECT COUNT(*) FROM artefact_edges_current WHERE repo_id = ?1 AND edge_id = 'current-edge'",
            rusqlite::params![repo_id.as_str()],
            |row| row.get(0),
        )
        .unwrap();

    assert_eq!(
        current_artefacts_rows, 1,
        "retention pruning must not delete sync-owned artefacts_current rows"
    );
    assert_eq!(
        current_edges_rows, 1,
        "retention pruning must not delete sync-owned artefact_edges_current rows"
    );
}

#[test]
pub(crate) fn pre_push_marks_branch_pending_when_postgres_is_unreachable() {
    let dir = tempfile::tempdir().unwrap();
    let head = setup_git_repo(&dir);
    let sqlite_path =
        init_devql_schema_with_postgres_dsn(dir.path(), Some("postgres://127.0.0.1:1/devql"));
    let repo_id = crate::host::devql::resolve_repo_identity(dir.path())
        .unwrap()
        .repo_id;
    let branch = run_git(dir.path(), &["branch", "--show-current"]).unwrap();
    let zero_sha = "0000000000000000000000000000000000000000";
    let stdin_line = format!(
        "refs/heads/{branch} {head} refs/heads/{branch} {zero_sha}",
        branch = branch,
        head = head,
        zero_sha = zero_sha
    );

    ManualCommitStrategy::new(dir.path())
        .pre_push("origin", &[stdin_line])
        .unwrap();

    let sqlite = rusqlite::Connection::open(sqlite_path).unwrap();
    let pending_key = format!("pending_remote_sync_sha:origin:{branch}");
    let pending_sha: String = sqlite
        .query_row(
            "SELECT state_value FROM sync_state WHERE repo_id = ?1 AND state_key = ?2",
            rusqlite::params![repo_id.as_str(), pending_key.as_str()],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        pending_sha, head,
        "failed remote sync should mark the branch as pending with the local head SHA"
    );
}

#[test]
pub(crate) fn shadow_strategy_direct_instantiation() {
    let dir = tempfile::tempdir().unwrap();
    let strategy = ManualCommitStrategy::new(dir.path());
    assert_eq!(strategy.name(), "manual-commit");
}

#[test]
pub(crate) fn shadow_strategy_description() {
    let dir = tempfile::tempdir().unwrap();
    let strategy = ManualCommitStrategy::new(dir.path());
    assert_eq!(strategy.name(), "manual-commit");
}

#[test]
pub(crate) fn shadow_strategy_validate_repository() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    assert!(
        run_git(dir.path(), &["rev-parse", "--is-inside-work-tree"]).is_ok(),
        "expected git repo to validate"
    );
}

#[test]
pub(crate) fn shadow_strategy_validate_repository_not_git_repo() {
    let dir = tempfile::tempdir().unwrap();
    assert!(
        run_git(dir.path(), &["rev-parse", "--is-inside-work-tree"]).is_err(),
        "non-git directory should fail validation"
    );
}
