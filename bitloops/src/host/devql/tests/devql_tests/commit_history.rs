use super::*;

fn write_local_devql_config(repo_root: &Path) {
    write_repo_daemon_config(
        repo_root,
        r#"[stores.relational]
sqlite_path = ".bitloops/stores/devql.sqlite"

[stores.events]
duckdb_path = ".bitloops/stores/events.duckdb"
"#,
    );
}

fn rewrite_local_events_path(repo_root: &Path, replacement: &Path) {
    let config_path = repo_root.join(crate::config::BITLOOPS_CONFIG_RELATIVE_PATH);
    let content = fs::read_to_string(&config_path).expect("read local devql config");
    let updated = content
        .lines()
        .map(|line| {
            if line.trim_start().starts_with("duckdb_path =") {
                format!("duckdb_path = {:?}", replacement.to_string_lossy())
            } else {
                line.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";
    fs::write(&config_path, updated).expect("rewrite local events path");
}

fn cfg_for_repo(repo_root: &Path) -> DevqlConfig {
    let repo = resolve_repo_identity(repo_root).expect("resolve repo identity");
    DevqlConfig::from_env(repo_root.to_path_buf(), repo).expect("build devql cfg from repo")
}

fn sqlite_path_for_repo(repo_root: &Path) -> std::path::PathBuf {
    crate::config::resolve_store_backend_config_for_repo(repo_root)
        .expect("resolve backend config")
        .relational
        .resolve_sqlite_db_path_for_repo(repo_root)
        .expect("resolve sqlite path")
}

fn duckdb_path_for_repo(repo_root: &Path) -> std::path::PathBuf {
    crate::config::resolve_store_backend_config_for_repo(repo_root)
        .expect("resolve backend config")
        .events
        .resolve_duckdb_db_path_for_repo(repo_root)
}

fn remove_loose_git_object(repo_root: &Path, object_id: &str) {
    assert!(
        object_id.len() > 2,
        "expected non-empty git object id, got `{object_id}`"
    );
    let object_path = repo_root
        .join(".git")
        .join("objects")
        .join(&object_id[..2])
        .join(&object_id[2..]);
    assert!(
        object_path.is_file(),
        "expected loose git object at {}",
        object_path.display()
    );
    fs::remove_file(&object_path)
        .unwrap_or_else(|err| panic!("remove loose git object {}: {err}", object_path.display()));
}

fn sync_state_value(conn: &rusqlite::Connection, repo_id: &str, key: &str) -> Option<String> {
    use rusqlite::OptionalExtension;

    conn.query_row(
        "SELECT state_value FROM sync_state WHERE repo_id = ?1 AND state_key = ?2",
        rusqlite::params![repo_id, key],
        |row| row.get(0),
    )
    .optional()
    .expect("read sync_state value")
}

#[test]
fn select_missing_branch_commit_segment_prefers_branch_watermark_when_it_is_an_ancestor() {
    let repo = seed_git_repo();
    let sqlite_path = repo.path().join("relational.sqlite");
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");
    let relational = runtime.block_on(sqlite_relational_store_with_schema(&sqlite_path));
    let cfg = cfg_for_repo(repo.path());

    std::fs::create_dir_all(repo.path().join("src")).expect("create src");
    std::fs::write(
        repo.path().join("src/lib.rs"),
        "pub fn one() -> i32 { 1 }\n",
    )
    .expect("write lib.rs");
    git_ok(repo.path(), &["add", "."]);
    git_ok(repo.path(), &["commit", "-m", "add lib"]);
    let first_sha = git_ok(repo.path(), &["rev-parse", "HEAD"]);

    std::fs::write(
        repo.path().join("src/lib.rs"),
        "pub fn one() -> i32 { 1 }\npub fn two() -> i32 { 2 }\n",
    )
    .expect("update lib.rs");
    git_ok(repo.path(), &["add", "."]);
    git_ok(repo.path(), &["commit", "-m", "expand lib"]);
    let head_sha = git_ok(repo.path(), &["rev-parse", "HEAD"]);

    runtime
        .block_on(relational.exec(&format!(
            "INSERT INTO sync_state (repo_id, state_key, state_value, updated_at) \
             VALUES ('{}', '{}', '{}', datetime('now'))",
            esc_pg(&cfg.repo.repo_id),
            esc_pg(&historical_branch_watermark_key("main")),
            esc_pg(&first_sha),
        )))
        .expect("seed historical watermark");

    let commits = runtime
        .block_on(select_missing_branch_commit_segment(
            repo.path(),
            &relational,
            &cfg.repo.repo_id,
            Some("main"),
            &head_sha,
        ))
        .expect("select commit range");

    assert_eq!(commits, vec![head_sha]);
}

#[test]
fn select_missing_branch_commit_segment_falls_back_to_nearest_reachable_completed_ledger_commit() {
    let repo = seed_git_repo();
    let sqlite_path = repo.path().join("relational.sqlite");
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");
    let relational = runtime.block_on(sqlite_relational_store_with_schema(&sqlite_path));
    let cfg = cfg_for_repo(repo.path());

    std::fs::create_dir_all(repo.path().join("src")).expect("create src");
    std::fs::write(
        repo.path().join("src/lib.rs"),
        "pub fn one() -> i32 { 1 }\n",
    )
    .expect("write lib.rs");
    git_ok(repo.path(), &["add", "."]);
    git_ok(repo.path(), &["commit", "-m", "add lib"]);
    let first_sha = git_ok(repo.path(), &["rev-parse", "HEAD"]);

    std::fs::write(
        repo.path().join("src/lib.rs"),
        "pub fn one() -> i32 { 1 }\npub fn two() -> i32 { 2 }\n",
    )
    .expect("update lib.rs");
    git_ok(repo.path(), &["add", "."]);
    git_ok(repo.path(), &["commit", "-m", "expand lib"]);
    let second_sha = git_ok(repo.path(), &["rev-parse", "HEAD"]);

    std::fs::write(
        repo.path().join("src/lib.rs"),
        "pub fn one() -> i32 { 1 }\npub fn two() -> i32 { 2 }\npub fn three() -> i32 { 3 }\n",
    )
    .expect("update lib.rs again");
    git_ok(repo.path(), &["add", "."]);
    git_ok(repo.path(), &["commit", "-m", "expand lib again"]);
    let head_sha = git_ok(repo.path(), &["rev-parse", "HEAD"]);

    runtime
        .block_on(relational.exec(&format!(
            "INSERT INTO commit_ingest_ledger (
                repo_id, commit_sha, history_status, checkpoint_status, checkpoint_id, last_error, updated_at
            ) VALUES (
                '{}', '{}', 'completed', 'not_applicable', NULL, NULL, datetime('now')
            )",
            esc_pg(&cfg.repo.repo_id),
            esc_pg(&second_sha),
        )))
        .expect("seed completed ledger row");

    let commits = runtime
        .block_on(select_missing_branch_commit_segment(
            repo.path(),
            &relational,
            &cfg.repo.repo_id,
            Some("main"),
            &head_sha,
        ))
        .expect("select commit range");

    assert_eq!(commits, vec![head_sha]);
    assert_ne!(first_sha, second_sha);
}

#[test]
fn select_missing_branch_commit_segment_caps_history_when_branch_watermark_is_stale() {
    let repo = seed_git_repo();
    let sqlite_path = repo.path().join("relational.sqlite");
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");
    let relational = runtime.block_on(sqlite_relational_store_with_schema(&sqlite_path));
    let cfg = cfg_for_repo(repo.path());

    std::fs::create_dir_all(repo.path().join("src")).expect("create src");
    std::fs::write(
        repo.path().join("src/lib.rs"),
        "pub fn seed() -> i32 { 0 }\n",
    )
    .expect("write initial lib.rs");
    git_ok(repo.path(), &["add", "."]);
    git_ok(repo.path(), &["commit", "-m", "seed history"]);
    let stale_watermark = git_ok(repo.path(), &["rev-parse", "HEAD"]);

    runtime
        .block_on(relational.exec(&format!(
            "INSERT INTO sync_state (repo_id, state_key, state_value, updated_at) \
             VALUES ('{}', '{}', '{}', datetime('now'))",
            esc_pg(&cfg.repo.repo_id),
            esc_pg(&historical_branch_watermark_key("main")),
            esc_pg(&stale_watermark),
        )))
        .expect("seed stale historical watermark");

    git_ok(repo.path(), &["checkout", "--orphan", "rewritten-history"]);
    for idx in 0..205 {
        let body = (0..=idx)
            .map(|n| format!("pub fn value_{n}() -> usize {{ {n} }}"))
            .collect::<Vec<_>>()
            .join("\n");
        std::fs::write(repo.path().join("src/lib.rs"), format!("{body}\n"))
            .expect("write rewritten history lib.rs");
        git_ok(repo.path(), &["add", "."]);
        git_ok(
            repo.path(),
            &["commit", "-m", &format!("rewritten commit {idx}")],
        );
    }
    let head_sha = git_ok(repo.path(), &["rev-parse", "HEAD"]);

    let commits = runtime
        .block_on(select_missing_branch_commit_segment(
            repo.path(),
            &relational,
            &cfg.repo.repo_id,
            Some("main"),
            &head_sha,
        ))
        .expect("select commit range");

    assert_eq!(
        commits.len(),
        200,
        "stale branch watermarks should fall back to a bounded recovery window"
    );
    assert_eq!(commits.last().map(String::as_str), Some(head_sha.as_str()));
    assert!(
        commits.iter().all(|commit| commit != &stale_watermark),
        "stale watermark history should not be returned once the branch has been rewritten"
    );
}

#[tokio::test]
async fn execute_ingest_materialises_unmapped_commit_history_without_current_state_mutation() {
    let repo = seed_git_repo();
    write_local_devql_config(repo.path());
    std::fs::write(
        repo.path().join("Cargo.toml"),
        "[package]\nname = \"artefact-only-history-test\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .expect("write Cargo.toml");
    std::fs::create_dir_all(repo.path().join("src")).expect("create src");
    std::fs::write(
        repo.path().join("src/lib.rs"),
        "pub fn greet(name: &str) -> String { format!(\"hi {name}\") }\n",
    )
    .expect("write lib.rs");
    git_ok(repo.path(), &["add", "."]);
    git_ok(repo.path(), &["commit", "-m", "add lib"]);

    let cfg = cfg_for_repo(repo.path());
    execute_init_schema(&cfg, "commit-history unmapped test")
        .await
        .expect("initialise local devql store for unmapped commit history test");
    let summary = execute_ingest_with_observer(&cfg, false, 500, None, None)
        .await
        .expect("execute ingest for unmapped commits");
    assert!(
        summary.success,
        "ingest summary should report success for unmapped commit history"
    );
    assert_eq!(summary.commits_processed, 2);
    assert_eq!(summary.checkpoint_companions_processed, 0);

    let head_sha = git_ok(repo.path(), &["rev-parse", "HEAD"]);
    let sqlite =
        rusqlite::Connection::open(sqlite_path_for_repo(repo.path())).expect("open sqlite");

    let file_delta_count: i64 = sqlite
        .query_row(
            "SELECT COUNT(*) FROM commit_file_deltas WHERE repo_id = ?1 AND commit_sha = ?2",
            rusqlite::params![cfg.repo.repo_id.as_str(), head_sha.as_str()],
            |row| row.get(0),
        )
        .expect("count commit_file_deltas rows");
    assert_eq!(
        file_delta_count, 0,
        "artefact-only ingest must not write historical commit_file_deltas rows"
    );

    let hunk_count: i64 = sqlite
        .query_row(
            "SELECT COUNT(*) FROM commit_hunks WHERE repo_id = ?1 AND commit_sha = ?2",
            rusqlite::params![cfg.repo.repo_id.as_str(), head_sha.as_str()],
            |row| row.get(0),
        )
        .expect("count commit_hunks rows");
    assert_eq!(
        hunk_count, 0,
        "artefact-only ingest must not write historical commit_hunks rows"
    );

    let commit_artefact_count: i64 = sqlite
        .query_row(
            "SELECT COUNT(*) FROM commit_artefacts WHERE repo_id = ?1 AND commit_sha = ?2",
            rusqlite::params![cfg.repo.repo_id.as_str(), head_sha.as_str()],
            |row| row.get(0),
        )
        .expect("count commit_artefacts rows");
    assert!(
        commit_artefact_count > 0,
        "expected historical commit artefact rows"
    );

    let historical_file_state_count: i64 = sqlite
        .query_row(
            "SELECT COUNT(*) FROM file_state WHERE repo_id = ?1",
            rusqlite::params![cfg.repo.repo_id.as_str()],
            |row| row.get(0),
        )
        .expect("count historical file_state rows");
    assert_eq!(
        historical_file_state_count, 0,
        "artefact-only ingest must not write historical file_state"
    );

    let artefact_count: i64 = sqlite
        .query_row(
            "SELECT COUNT(*) FROM artefacts WHERE repo_id = ?1",
            rusqlite::params![cfg.repo.repo_id.as_str()],
            |row| row.get(0),
        )
        .expect("count historical artefacts");
    assert!(
        artefact_count > 0,
        "artefact-only ingest should write historical artefacts"
    );

    let current_artefact_count: i64 = sqlite
        .query_row(
            "SELECT COUNT(*) FROM artefacts_current WHERE repo_id = ?1",
            rusqlite::params![cfg.repo.repo_id.as_str()],
            |row| row.get(0),
        )
        .expect("count current artefacts");
    assert_eq!(
        current_artefact_count, 0,
        "historical ingest must not mutate artefacts_current"
    );

    let current_file_state_count: i64 = sqlite
        .query_row(
            "SELECT COUNT(*) FROM current_file_state WHERE repo_id = ?1",
            rusqlite::params![cfg.repo.repo_id.as_str()],
            |row| row.get(0),
        )
        .expect("count current_file_state rows");
    assert_eq!(
        current_file_state_count, 0,
        "historical ingest must not mutate current_file_state"
    );

    let repo_sync_state_count: i64 = sqlite
        .query_row(
            "SELECT COUNT(*) FROM repo_sync_state WHERE repo_id = ?1",
            rusqlite::params![cfg.repo.repo_id.as_str()],
            |row| row.get(0),
        )
        .expect("count repo_sync_state rows");
    assert_eq!(
        repo_sync_state_count, 0,
        "historical ingest must not mutate repo_sync_state"
    );

    let semantic_rows: i64 = sqlite
        .query_row(
            "SELECT COUNT(*) FROM symbol_semantics WHERE repo_id = ?1",
            rusqlite::params![cfg.repo.repo_id.as_str()],
            |row| row.get(0),
        )
        .expect("count historical semantic rows");
    let embedding_rows: i64 = sqlite
        .query_row(
            "SELECT COUNT(*) FROM symbol_embeddings WHERE repo_id = ?1",
            rusqlite::params![cfg.repo.repo_id.as_str()],
            |row| row.get(0),
        )
        .expect("count historical embedding rows");
    let clone_rows: i64 = sqlite
        .query_row(
            "SELECT COUNT(*) FROM symbol_clone_edges WHERE repo_id = ?1",
            rusqlite::params![cfg.repo.repo_id.as_str()],
            |row| row.get(0),
        )
        .expect("count historical clone rows");
    assert_eq!(
        semantic_rows, 0,
        "historical ingest must not write semantic clone summaries"
    );
    assert_eq!(
        embedding_rows, 0,
        "historical ingest must not write semantic clone embeddings"
    );
    assert_eq!(
        clone_rows, 0,
        "historical ingest must not rebuild clone edges"
    );

    let ledger_row: (String, String) = sqlite
        .query_row(
            "SELECT history_status, checkpoint_status
             FROM commit_ingest_ledger
             WHERE repo_id = ?1 AND commit_sha = ?2",
            rusqlite::params![cfg.repo.repo_id.as_str(), head_sha.as_str()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("read commit ingest ledger row");
    assert_eq!(ledger_row.0, "completed");
    assert_eq!(ledger_row.1, "not_applicable");

    let watermark = sync_state_value(
        &sqlite,
        cfg.repo.repo_id.as_str(),
        &historical_branch_watermark_key("main"),
    )
    .expect("expected branch historical watermark");
    assert_eq!(watermark, head_sha);

    let checkpoint_projection_rows: i64 = sqlite
        .query_row("SELECT COUNT(*) FROM checkpoint_files", [], |row| {
            row.get(0)
        })
        .expect("count checkpoint projections");
    assert_eq!(
        checkpoint_projection_rows, 0,
        "unmapped commits must not synthesize checkpoint projection rows"
    );

    let duckdb = duckdb::Connection::open(duckdb_path_for_repo(repo.path())).expect("open duckdb");
    let checkpoint_event_rows: i64 = duckdb
        .query_row("SELECT COUNT(*) FROM checkpoint_events", [], |row| {
            row.get(0)
        })
        .expect("count checkpoint events");
    assert_eq!(
        checkpoint_event_rows, 0,
        "unmapped commits must not synthesize checkpoint events"
    );
}

#[tokio::test]
async fn execute_ingest_persists_changed_after_side_artefacts_without_hunks_or_file_deltas() {
    let repo = seed_git_repo();
    write_local_devql_config(repo.path());
    std::fs::write(
        repo.path().join("Cargo.toml"),
        "[package]\nname = \"artefact-only-history-test\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .expect("write Cargo.toml");
    std::fs::create_dir_all(repo.path().join("src")).expect("create src");

    std::fs::write(
        repo.path().join("src/lib.rs"),
        "pub fn one() -> i32 { 1 }\npub fn two() -> i32 { 2 }\n",
    )
    .expect("write lib.rs");
    git_ok(repo.path(), &["add", "."]);
    git_ok(repo.path(), &["commit", "-m", "add lib"]);
    let added_sha = git_ok(repo.path(), &["rev-parse", "HEAD"]);

    std::fs::write(
        repo.path().join("src/lib.rs"),
        "pub fn one() -> i32 { 10 }\npub fn two() -> i32 { 2 }\npub fn three() -> i32 { 3 }\n",
    )
    .expect("modify lib.rs");
    git_ok(repo.path(), &["add", "."]);
    git_ok(repo.path(), &["commit", "-m", "modify lib"]);
    let modified_sha = git_ok(repo.path(), &["rev-parse", "HEAD"]);

    std::fs::remove_file(repo.path().join("src/lib.rs")).expect("delete lib.rs");
    git_ok(repo.path(), &["add", "-A"]);
    git_ok(repo.path(), &["commit", "-m", "delete lib"]);
    let deleted_sha = git_ok(repo.path(), &["rev-parse", "HEAD"]);

    let cfg = cfg_for_repo(repo.path());
    execute_init_schema(&cfg, "commit-history artefact-only test")
        .await
        .expect("initialise local devql store for artefact-only ingest test");
    let summary = execute_ingest_with_observer(&cfg, false, 500, None, None)
        .await
        .expect("execute artefact-only ingest");
    assert!(
        summary.success,
        "ingest summary should report success for artefact-only commit history"
    );
    assert!(
        summary.artefacts_upserted > 0,
        "artefact-only ingest should append artefact metadata for changed files"
    );
    assert_eq!(summary.commits_processed, 4);
    assert_eq!(
        summary.file_deltas_upserted, 0,
        "artefact-only ingest should not report persisted file deltas"
    );
    assert_eq!(
        summary.hunks_upserted, 0,
        "artefact-only ingest should not report persisted hunks"
    );

    let sqlite =
        rusqlite::Connection::open(sqlite_path_for_repo(repo.path())).expect("open sqlite");
    let file_delta_count: i64 = sqlite
        .query_row(
            "SELECT COUNT(*) FROM commit_file_deltas WHERE repo_id = ?1",
            rusqlite::params![cfg.repo.repo_id.as_str()],
            |row| row.get(0),
        )
        .expect("count commit file deltas");
    let hunk_count: i64 = sqlite
        .query_row(
            "SELECT COUNT(*) FROM commit_hunks WHERE repo_id = ?1",
            rusqlite::params![cfg.repo.repo_id.as_str()],
            |row| row.get(0),
        )
        .expect("count commit hunks");
    assert_eq!(file_delta_count, 0);
    assert_eq!(hunk_count, 0);

    let added_commit_artefacts: i64 = sqlite
        .query_row(
            "SELECT COUNT(*) FROM commit_artefacts WHERE repo_id = ?1 AND commit_sha = ?2",
            rusqlite::params![cfg.repo.repo_id.as_str(), added_sha.as_str()],
            |row| row.get(0),
        )
        .expect("count added commit artefacts");
    let modified_commit_artefacts: i64 = sqlite
        .query_row(
            "SELECT COUNT(*) FROM commit_artefacts WHERE repo_id = ?1 AND commit_sha = ?2",
            rusqlite::params![cfg.repo.repo_id.as_str(), modified_sha.as_str()],
            |row| row.get(0),
        )
        .expect("count modified commit artefacts");
    let deleted_commit_artefacts: i64 = sqlite
        .query_row(
            "SELECT COUNT(*) FROM commit_artefacts WHERE repo_id = ?1 AND commit_sha = ?2",
            rusqlite::params![cfg.repo.repo_id.as_str(), deleted_sha.as_str()],
            |row| row.get(0),
        )
        .expect("count deleted commit artefacts");
    let commit_artefact_counts: Vec<(String, i64)> = {
        let mut stmt = sqlite
            .prepare(
                "SELECT commit_sha, COUNT(*)
                 FROM commit_artefacts
                 WHERE repo_id = ?1
                 GROUP BY commit_sha
                 ORDER BY commit_sha",
            )
            .expect("prepare commit artefact counts");
        stmt.query_map(rusqlite::params![cfg.repo.repo_id.as_str()], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        })
        .expect("query commit artefact counts")
        .collect::<Result<Vec<_>, _>>()
        .expect("collect commit artefact counts")
    };
    assert!(
        added_commit_artefacts > 0,
        "added after-side file should produce commit artefact links: {commit_artefact_counts:?}"
    );
    assert!(
        modified_commit_artefacts > 0,
        "modified after-side file should produce commit artefact links: {commit_artefact_counts:?}"
    );
    assert_eq!(
        deleted_commit_artefacts, 0,
        "deleted-only hunks should not produce after-side commit artefacts"
    );

    let modified_symbols: Vec<String> = {
        let mut stmt = sqlite
            .prepare(
                "SELECT DISTINCT a.symbol_fqn
                 FROM commit_artefacts ca
                 JOIN artefacts a
                   ON a.repo_id = ca.repo_id
                  AND a.artefact_id = ca.artefact_id
                 WHERE ca.repo_id = ?1
                   AND ca.commit_sha = ?2
                   AND a.symbol_fqn LIKE 'src/lib.rs::%'
                 ORDER BY a.symbol_fqn",
            )
            .expect("prepare modified commit artefact symbols query");
        stmt.query_map(
            rusqlite::params![cfg.repo.repo_id.as_str(), modified_sha.as_str()],
            |row| row.get::<_, String>(0),
        )
        .expect("query modified commit artefact symbols")
        .collect::<Result<Vec<_>, _>>()
        .expect("collect modified commit artefact symbols")
    };
    assert!(
        modified_symbols
            .iter()
            .any(|symbol| symbol.ends_with("::one")),
        "modified function should be linked to the modifying commit: {modified_symbols:?}"
    );
    assert!(
        modified_symbols
            .iter()
            .any(|symbol| symbol.ends_with("::three")),
        "added function should be linked to the modifying commit: {modified_symbols:?}"
    );
    assert!(
        !modified_symbols
            .iter()
            .any(|symbol| symbol.ends_with("::two")),
        "unchanged non-overlapping function should not be linked to the modifying commit: {modified_symbols:?}"
    );

    let historical_file_state_count: i64 = sqlite
        .query_row(
            "SELECT COUNT(*) FROM file_state WHERE repo_id = ?1",
            rusqlite::params![cfg.repo.repo_id.as_str()],
            |row| row.get(0),
        )
        .expect("count file_state rows");
    let historical_artefact_count: i64 = sqlite
        .query_row(
            "SELECT COUNT(*) FROM artefacts WHERE repo_id = ?1",
            rusqlite::params![cfg.repo.repo_id.as_str()],
            |row| row.get(0),
        )
        .expect("count historical artefacts");
    let historical_snapshot_count: i64 = sqlite
        .query_row(
            "SELECT COUNT(*) FROM artefact_snapshots WHERE repo_id = ?1",
            rusqlite::params![cfg.repo.repo_id.as_str()],
            |row| row.get(0),
        )
        .expect("count historical artefact snapshots");
    assert_eq!(historical_file_state_count, 0);
    assert!(
        historical_artefact_count > 0,
        "changed files should still produce artefact metadata rows"
    );
    assert_eq!(historical_snapshot_count, 0);
}

#[tokio::test]
async fn execute_ingest_mirrors_completed_current_artefacts_and_remains_idempotent() {
    let repo = seed_git_repo();
    write_local_devql_config(repo.path());
    std::fs::write(
        repo.path().join("Cargo.toml"),
        "[package]\nname = \"current-mirror-ingest-test\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .expect("write Cargo.toml");
    std::fs::create_dir_all(repo.path().join("src")).expect("create src");
    std::fs::write(
        repo.path().join("src/lib.rs"),
        "pub fn synced() -> i32 { 7 }\n",
    )
    .expect("write lib.rs");
    git_ok(repo.path(), &["add", "."]);
    git_ok(repo.path(), &["commit", "-m", "add synced lib"]);

    let cfg = cfg_for_repo(repo.path());
    execute_init_schema(&cfg, "commit-history current mirror test")
        .await
        .expect("initialise local devql store for current mirror ingest test");
    let backends = crate::config::resolve_store_backend_config_for_repo(&cfg.daemon_config_root)
        .expect("resolve backend config for current mirror ingest test");
    let relational =
        RelationalStorage::connect(&cfg, &backends.relational, "current mirror ingest test")
            .await
            .expect("connect relational store for current mirror ingest test");
    execute_sync(&cfg, &relational, SyncMode::Full)
        .await
        .expect("sync current artefacts before ingest");

    let sqlite =
        rusqlite::Connection::open(sqlite_path_for_repo(repo.path())).expect("open sqlite");
    let current_rows: i64 = sqlite
        .query_row(
            "SELECT COUNT(*) FROM artefacts_current WHERE repo_id = ?1",
            rusqlite::params![cfg.repo.repo_id.as_str()],
            |row| row.get(0),
        )
        .expect("count current artefacts after sync");
    assert!(
        current_rows > 0,
        "sync should materialise current artefacts before ingest"
    );

    let first = execute_ingest_with_observer(&cfg, false, 500, None, None)
        .await
        .expect("execute ingest after sync");
    assert!(first.success, "ingest should succeed after current sync");
    assert!(
        first.artefacts_upserted >= current_rows as usize,
        "ingest result should include mirrored current artefact rows"
    );

    let missing_current_rows: i64 = sqlite
        .query_row(
            "SELECT COUNT(*) \
             FROM artefacts_current current \
             LEFT JOIN artefacts canonical \
               ON canonical.repo_id = current.repo_id \
              AND canonical.artefact_id = current.artefact_id \
             WHERE current.repo_id = ?1 \
               AND canonical.artefact_id IS NULL",
            rusqlite::params![cfg.repo.repo_id.as_str()],
            |row| row.get(0),
        )
        .expect("count current artefacts missing canonical mirror");
    let mismatched_content_hash_rows: i64 = sqlite
        .query_row(
            "SELECT COUNT(*) \
             FROM artefacts_current current \
             JOIN artefacts canonical \
               ON canonical.repo_id = current.repo_id \
              AND canonical.artefact_id = current.artefact_id \
             WHERE current.repo_id = ?1 \
               AND canonical.content_hash <> current.content_id",
            rusqlite::params![cfg.repo.repo_id.as_str()],
            |row| row.get(0),
        )
        .expect("count mirrored artefacts with mismatched content hash");
    assert_eq!(
        missing_current_rows, 0,
        "every current artefact should be mirrored into canonical artefacts"
    );
    assert_eq!(
        mismatched_content_hash_rows, 0,
        "current content_id should become canonical content_hash"
    );

    let artefacts_after_first: i64 = sqlite
        .query_row(
            "SELECT COUNT(*) FROM artefacts WHERE repo_id = ?1",
            rusqlite::params![cfg.repo.repo_id.as_str()],
            |row| row.get(0),
        )
        .expect("count artefacts after first ingest");
    let file_deltas_after_first: i64 = sqlite
        .query_row(
            "SELECT COUNT(*) FROM commit_file_deltas WHERE repo_id = ?1",
            rusqlite::params![cfg.repo.repo_id.as_str()],
            |row| row.get(0),
        )
        .expect("count file deltas after first ingest");
    let hunks_after_first: i64 = sqlite
        .query_row(
            "SELECT COUNT(*) FROM commit_hunks WHERE repo_id = ?1",
            rusqlite::params![cfg.repo.repo_id.as_str()],
            |row| row.get(0),
        )
        .expect("count hunks after first ingest");
    let commit_artefacts_after_first: i64 = sqlite
        .query_row(
            "SELECT COUNT(*) FROM commit_artefacts WHERE repo_id = ?1",
            rusqlite::params![cfg.repo.repo_id.as_str()],
            |row| row.get(0),
        )
        .expect("count commit artefacts after first ingest");
    assert_eq!(
        file_deltas_after_first, 0,
        "artefact-only ingest should not persist commit file deltas"
    );
    assert_eq!(
        hunks_after_first, 0,
        "artefact-only ingest should not persist textual hunks"
    );
    assert!(
        commit_artefacts_after_first > 0,
        "artefact-only ingest should persist commit artefact links"
    );

    let file_state_rows: i64 = sqlite
        .query_row(
            "SELECT COUNT(*) FROM file_state WHERE repo_id = ?1",
            rusqlite::params![cfg.repo.repo_id.as_str()],
            |row| row.get(0),
        )
        .expect("count file_state rows");
    let snapshot_rows: i64 = sqlite
        .query_row(
            "SELECT COUNT(*) FROM artefact_snapshots WHERE repo_id = ?1",
            rusqlite::params![cfg.repo.repo_id.as_str()],
            |row| row.get(0),
        )
        .expect("count artefact snapshot rows");
    assert_eq!(
        file_state_rows, 0,
        "artefact-only ingest must not write historical file_state rows"
    );
    assert_eq!(
        snapshot_rows, 0,
        "current mirror must not create historical artefact snapshots"
    );

    let replay = execute_ingest_with_observer(&cfg, false, 500, None, None)
        .await
        .expect("re-run ingest after current mirror");
    assert!(replay.success, "replayed ingest should succeed");

    let artefacts_after_replay: i64 = sqlite
        .query_row(
            "SELECT COUNT(*) FROM artefacts WHERE repo_id = ?1",
            rusqlite::params![cfg.repo.repo_id.as_str()],
            |row| row.get(0),
        )
        .expect("count artefacts after replay ingest");
    let file_deltas_after_replay: i64 = sqlite
        .query_row(
            "SELECT COUNT(*) FROM commit_file_deltas WHERE repo_id = ?1",
            rusqlite::params![cfg.repo.repo_id.as_str()],
            |row| row.get(0),
        )
        .expect("count file deltas after replay ingest");
    let hunks_after_replay: i64 = sqlite
        .query_row(
            "SELECT COUNT(*) FROM commit_hunks WHERE repo_id = ?1",
            rusqlite::params![cfg.repo.repo_id.as_str()],
            |row| row.get(0),
        )
        .expect("count hunks after replay ingest");
    let commit_artefacts_after_replay: i64 = sqlite
        .query_row(
            "SELECT COUNT(*) FROM commit_artefacts WHERE repo_id = ?1",
            rusqlite::params![cfg.repo.repo_id.as_str()],
            |row| row.get(0),
        )
        .expect("count commit artefacts after replay ingest");
    assert_eq!(artefacts_after_replay, artefacts_after_first);
    assert_eq!(file_deltas_after_replay, file_deltas_after_first);
    assert_eq!(hunks_after_replay, hunks_after_first);
    assert_eq!(commit_artefacts_after_replay, commit_artefacts_after_first);
}

#[tokio::test]
async fn execute_ingest_skips_current_mirror_when_completed_sync_state_is_not_for_head() {
    let repo = seed_git_repo();
    write_local_devql_config(repo.path());
    std::fs::create_dir_all(repo.path().join("src")).expect("create src");
    std::fs::write(
        repo.path().join("src/lib.rs"),
        "pub fn historical_only() -> i32 { 11 }\n",
    )
    .expect("write lib.rs");
    git_ok(repo.path(), &["add", "."]);
    git_ok(repo.path(), &["commit", "-m", "add historical-only lib"]);

    let cfg = cfg_for_repo(repo.path());
    execute_init_schema(&cfg, "commit-history stale current mirror test")
        .await
        .expect("initialise local devql store for stale current mirror test");
    let head_sha = git_ok(repo.path(), &["rev-parse", "HEAD"]);
    let sqlite =
        rusqlite::Connection::open(sqlite_path_for_repo(repo.path())).expect("open sqlite");
    sqlite
        .execute(
            "INSERT INTO repositories (repo_id, provider, organization, name) \
             VALUES (?1, ?2, ?3, ?4) \
             ON CONFLICT (repo_id) DO UPDATE SET name = excluded.name",
            rusqlite::params![
                cfg.repo.repo_id.as_str(),
                cfg.repo.provider.as_str(),
                cfg.repo.organization.as_str(),
                cfg.repo.name.as_str(),
            ],
        )
        .expect("seed repository row");
    sqlite
        .execute(
            "INSERT INTO repo_sync_state (
                repo_id, repo_root, head_commit_sha, parser_version, extractor_version,
                last_sync_status
             ) VALUES (?1, ?2, ?3, 'parser', 'extractor', 'completed')",
            rusqlite::params![
                cfg.repo.repo_id.as_str(),
                repo.path().display().to_string(),
                format!("stale-{head_sha}"),
            ],
        )
        .expect("seed stale completed sync state");
    sqlite
        .execute(
            "INSERT INTO artefacts_current (
                repo_id, path, content_id, symbol_id, artefact_id, language,
                extraction_fingerprint, canonical_kind, language_kind, symbol_fqn,
                start_line, end_line, start_byte, end_byte, modifiers, updated_at
             ) VALUES (
                ?1, 'src/current_only.rs', 'content-current-only', 'current-only-symbol',
                'current-only-artefact', 'rust', 'fingerprint', 'function', 'function',
                'current_only::symbol', 1, 1, 0, 10, '[]', datetime('now')
             )",
            rusqlite::params![cfg.repo.repo_id.as_str()],
        )
        .expect("seed current-only artefact row");

    let summary = execute_ingest_with_observer(&cfg, false, 500, None, None)
        .await
        .expect("execute ingest with stale sync state");
    assert!(
        summary.success,
        "ingest should still process historical artefacts when current mirror is skipped"
    );

    let current_only_mirrored: i64 = sqlite
        .query_row(
            "SELECT COUNT(*) FROM artefacts \
             WHERE repo_id = ?1 AND artefact_id = 'current-only-artefact'",
            rusqlite::params![cfg.repo.repo_id.as_str()],
            |row| row.get(0),
        )
        .expect("count stale current-only canonical rows");
    let hunk_rows: i64 = sqlite
        .query_row(
            "SELECT COUNT(*) FROM commit_hunks WHERE repo_id = ?1",
            rusqlite::params![cfg.repo.repo_id.as_str()],
            |row| row.get(0),
        )
        .expect("count hunk rows after stale mirror skip");
    let commit_artefact_rows: i64 = sqlite
        .query_row(
            "SELECT COUNT(*) FROM commit_artefacts WHERE repo_id = ?1",
            rusqlite::params![cfg.repo.repo_id.as_str()],
            |row| row.get(0),
        )
        .expect("count commit artefact rows after stale mirror skip");

    assert_eq!(
        current_only_mirrored, 0,
        "stale completed sync state must not mirror current-only artefacts"
    );
    assert_eq!(
        hunk_rows, 0,
        "artefact-only ingest should not persist hunks"
    );
    assert!(
        commit_artefact_rows > 0,
        "artefact-only ingest should still persist historical commit artefacts"
    );
}

#[tokio::test]
async fn execute_ingest_skips_binary_file_delta_without_textual_hunks() {
    let repo = seed_git_repo();
    write_local_devql_config(repo.path());
    std::fs::create_dir_all(repo.path().join("assets")).expect("create assets");
    std::fs::write(
        repo.path().join("assets/blob.bin"),
        [0, 159, 146, 150, 0, 1, 2, 3, 255, 0, 4, 5],
    )
    .expect("write binary fixture");
    git_ok(repo.path(), &["add", "."]);
    git_ok(repo.path(), &["commit", "-m", "add binary blob"]);
    let binary_sha = git_ok(repo.path(), &["rev-parse", "HEAD"]);

    let cfg = cfg_for_repo(repo.path());
    execute_init_schema(&cfg, "commit-history binary artefact-only test")
        .await
        .expect("initialise local devql store for binary artefact-only ingest test");
    let summary = execute_ingest_with_observer(&cfg, false, 500, None, None)
        .await
        .expect("execute ingest for binary artefact-only test");
    assert!(
        summary.success,
        "binary artefact-only ingest should succeed"
    );

    let sqlite =
        rusqlite::Connection::open(sqlite_path_for_repo(repo.path())).expect("open sqlite");
    let binary_file_delta_rows: i64 = sqlite
        .query_row(
            "SELECT COUNT(*) FROM commit_file_deltas \
             WHERE repo_id = ?1 \
               AND commit_sha = ?2 \
               AND path_after = 'assets/blob.bin' \
               AND is_binary = 1",
            rusqlite::params![cfg.repo.repo_id.as_str(), binary_sha.as_str()],
            |row| row.get(0),
        )
        .expect("count binary file deltas");
    let binary_hunk_rows: i64 = sqlite
        .query_row(
            "SELECT COUNT(*) FROM commit_hunks \
             WHERE repo_id = ?1 \
               AND commit_sha = ?2 \
               AND path_after = 'assets/blob.bin'",
            rusqlite::params![cfg.repo.repo_id.as_str(), binary_sha.as_str()],
            |row| row.get(0),
        )
        .expect("count binary hunk rows");
    let file_state_rows: i64 = sqlite
        .query_row(
            "SELECT COUNT(*) FROM file_state WHERE repo_id = ?1",
            rusqlite::params![cfg.repo.repo_id.as_str()],
            |row| row.get(0),
        )
        .expect("count file_state rows for binary ingest");
    let snapshot_rows: i64 = sqlite
        .query_row(
            "SELECT COUNT(*) FROM artefact_snapshots WHERE repo_id = ?1",
            rusqlite::params![cfg.repo.repo_id.as_str()],
            |row| row.get(0),
        )
        .expect("count artefact snapshot rows for binary ingest");
    let binary_commit_artefact_rows: i64 = sqlite
        .query_row(
            "SELECT COUNT(*) FROM commit_artefacts \
             WHERE repo_id = ?1 \
               AND commit_sha = ?2 \
               AND path = 'assets/blob.bin'",
            rusqlite::params![cfg.repo.repo_id.as_str(), binary_sha.as_str()],
            |row| row.get(0),
        )
        .expect("count binary commit artefact rows");

    assert_eq!(
        binary_file_delta_rows, 0,
        "binary files should not produce commit_file_deltas rows"
    );
    assert_eq!(
        binary_hunk_rows, 0,
        "binary file deltas should not produce textual hunk rows"
    );
    assert_eq!(
        binary_commit_artefact_rows, 0,
        "binary files without extractable after-side artefacts should not produce commit artefact rows"
    );
    assert_eq!(file_state_rows, 0);
    assert_eq!(snapshot_rows, 0);
}

#[tokio::test]
async fn execute_ingest_materialises_invalid_utf8_commit_as_file_artefact_only() {
    let repo = seed_git_repo();
    write_local_devql_config(repo.path());
    std::fs::create_dir_all(repo.path().join("src")).expect("create src");
    std::fs::write(
        repo.path().join("Cargo.toml"),
        "[package]\nname = \"invalid-utf8-ingest-test\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .expect("write Cargo.toml");
    std::fs::write(
        repo.path().join("src/bad.rs"),
        [
            0x2f, 0x2f, 0x20, 0x62, 0x61, 0x64, 0xff, 0x0a, 0x70, 0x75, 0x62, 0x20, 0x66, 0x6e,
            0x20, 0x62, 0x61, 0x64, 0x28, 0x29, 0x20, 0x2d, 0x3e, 0x20, 0x69, 0x33, 0x32, 0x20,
            0x7b, 0x0a, 0x20, 0x20, 0x20, 0x20, 0x32, 0x0a, 0x7d, 0x0a,
        ],
    )
    .expect("write invalid UTF-8 rust file");
    git_ok(repo.path(), &["add", "."]);
    git_ok(repo.path(), &["commit", "-m", "add invalid utf8 file"]);

    let cfg = cfg_for_repo(repo.path());
    execute_init_schema(&cfg, "commit-history decode-degraded file-only test")
        .await
        .expect("initialise local devql store for decode-degraded ingest test");
    let summary = execute_ingest_with_observer(&cfg, false, 500, None, None)
        .await
        .expect("execute ingest for decode-degraded commits");
    assert!(summary.success, "ingest summary should report success");

    let head_sha = git_ok(repo.path(), &["rev-parse", "HEAD"]);
    let sqlite =
        rusqlite::Connection::open(sqlite_path_for_repo(repo.path())).expect("open sqlite");

    let file_delta_rows: i64 = sqlite
        .query_row(
            "SELECT COUNT(*) \
             FROM commit_file_deltas \
             WHERE repo_id = ?1 \
               AND commit_sha = ?2 \
               AND path_after = ?3",
            rusqlite::params![cfg.repo.repo_id.as_str(), head_sha.as_str(), "src/bad.rs"],
            |row| row.get(0),
        )
        .expect("count commit_file_deltas rows for src/bad.rs");
    let bad_file_commit_artefact_rows: i64 = sqlite
        .query_row(
            "SELECT COUNT(*) \
             FROM commit_artefacts ca \
             JOIN artefacts a \
               ON a.repo_id = ca.repo_id \
              AND a.artefact_id = ca.artefact_id \
             WHERE ca.repo_id = ?1 \
               AND ca.commit_sha = ?2 \
               AND ca.path = 'src/bad.rs' \
               AND a.symbol_fqn = 'src/bad.rs' \
               AND a.canonical_kind = 'file'",
            rusqlite::params![cfg.repo.repo_id.as_str(), head_sha.as_str()],
            |row| row.get(0),
        )
        .expect("count commit_artefacts rows for src/bad.rs");
    let file_state_rows: i64 = sqlite
        .query_row(
            "SELECT COUNT(*) FROM file_state WHERE repo_id = ?1",
            rusqlite::params![cfg.repo.repo_id.as_str()],
            |row| row.get(0),
        )
        .expect("count historical file_state rows");
    let file_artefact_rows: i64 = sqlite
        .query_row(
            "SELECT COUNT(*) FROM artefacts
             WHERE repo_id = ?1 AND symbol_fqn = ?2 AND canonical_kind = 'file'",
            rusqlite::params![cfg.repo.repo_id.as_str(), "src/bad.rs"],
            |row| row.get(0),
        )
        .expect("count file artefact rows for src/bad.rs");
    let nested_artefact_rows: i64 = sqlite
        .query_row(
            "SELECT COUNT(*) FROM artefacts
             WHERE repo_id = ?1 AND symbol_fqn LIKE ?2",
            rusqlite::params![cfg.repo.repo_id.as_str(), "src/bad.rs::%"],
            |row| row.get(0),
        )
        .expect("count nested artefact rows for src/bad.rs");
    let snapshot_rows: i64 = sqlite
        .query_row(
            "SELECT COUNT(*) FROM artefact_snapshots
             WHERE repo_id = ?1",
            rusqlite::params![cfg.repo.repo_id.as_str()],
            |row| row.get(0),
        )
        .expect("count artefact snapshots");
    let edge_rows: i64 = sqlite
        .query_row(
            "SELECT COUNT(*) FROM artefact_edges WHERE repo_id = ?1",
            rusqlite::params![cfg.repo.repo_id.as_str()],
            |row| row.get(0),
        )
        .expect("count historical edge rows");

    assert_eq!(
        file_delta_rows, 0,
        "artefact-only ingest should not persist a file delta for src/bad.rs"
    );
    assert_eq!(
        bad_file_commit_artefact_rows, 1,
        "artefact-only ingest should link the file artefact for decode-degraded files"
    );
    assert_eq!(
        file_state_rows, 0,
        "artefact-only ingest must not persist file_state for src/bad.rs"
    );
    assert_eq!(
        file_artefact_rows, 1,
        "artefact-only ingest should persist file artefact metadata for decode-degraded files"
    );
    assert_eq!(
        nested_artefact_rows, 0,
        "ingest should not persist nested artefacts for src/bad.rs"
    );
    assert_eq!(
        snapshot_rows, 0,
        "artefact-only ingest must not persist snapshot rows for src/bad.rs"
    );
    assert_eq!(
        edge_rows, 0,
        "ingest should not persist dependency edges for src/bad.rs"
    );
}

#[tokio::test]
async fn execute_ingest_errors_when_hunk_diff_cannot_be_loaded() {
    let repo = seed_git_repo();
    write_local_devql_config(repo.path());
    std::fs::create_dir_all(repo.path().join("src")).expect("create src");
    std::fs::write(
        repo.path().join("src/lib.rs"),
        "pub fn answer() -> i32 { 42 }\n",
    )
    .expect("write lib.rs");
    git_ok(repo.path(), &["add", "."]);
    git_ok(repo.path(), &["commit", "-m", "add lib"]);

    let cfg = cfg_for_repo(repo.path());
    execute_init_schema(&cfg, "commit-history missing-blob test")
        .await
        .expect("initialise local devql store for missing blob ingest test");

    let commit_sha = git_ok(repo.path(), &["rev-parse", "HEAD"]);
    let blob_sha = git_ok(repo.path(), &["rev-parse", "HEAD:src/lib.rs"]);
    remove_loose_git_object(repo.path(), &blob_sha);

    let summary = execute_ingest_with_observer(&cfg, false, 500, None, None)
        .await
        .expect("execute ingest with one missing blob object");
    assert!(
        !summary.success,
        "artefact-only ingest should report partial failure when git cannot render the commit diff"
    );

    let sqlite =
        rusqlite::Connection::open(sqlite_path_for_repo(repo.path())).expect("open sqlite");
    let ledger_row: (String, String, Option<String>) = sqlite
        .query_row(
            "SELECT history_status, checkpoint_status, last_error
             FROM commit_ingest_ledger
             WHERE repo_id = ?1 AND commit_sha = ?2",
            rusqlite::params![cfg.repo.repo_id.as_str(), commit_sha.as_str()],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .expect("read failed commit ledger row");
    let message = ledger_row.2.as_deref().unwrap_or_default();

    assert_eq!(
        ledger_row.0, "failed",
        "artefact-only ingest should mark the commit as failed when git cannot render the diff"
    );
    assert_eq!(
        ledger_row.1, "failed",
        "historical ingest should record the commit as failed before checkpoint completion"
    );

    assert!(
        message.contains("reading hunk diff for commit"),
        "unexpected artefact-only ingest error: {message}"
    );
    assert!(
        message.contains(commit_sha.as_str()),
        "historical ingest error should include commit context: {message}"
    );
    assert!(
        message.contains(blob_sha.as_str()),
        "artefact-only ingest error should include blob context: {message}"
    );
}

#[tokio::test]
async fn execute_ingest_skips_events_backend_for_unmapped_commits_when_events_store_is_unavailable()
{
    let repo = seed_git_repo();
    write_local_devql_config(repo.path());
    std::fs::create_dir_all(repo.path().join("src")).expect("create src");
    std::fs::write(
        repo.path().join("src/lib.rs"),
        "pub fn greet(name: &str) -> String { format!(\"hi {name}\") }\n",
    )
    .expect("write lib.rs");
    git_ok(repo.path(), &["add", "."]);
    git_ok(repo.path(), &["commit", "-m", "add lib"]);

    let cfg = cfg_for_repo(repo.path());
    execute_init_schema(&cfg, "commit-history unmapped events-unavailable test")
        .await
        .expect("initialise local devql store for unmapped events-unavailable test");

    let blocked_parent = repo.path().join("blocked-events-parent");
    std::fs::write(&blocked_parent, "not a directory").expect("write blocking file");
    rewrite_local_events_path(repo.path(), &blocked_parent.join("events.duckdb"));

    let summary = execute_ingest_with_observer(&cfg, false, 500, None, None)
        .await
        .expect("execute ingest for unmapped commits without events backend");
    assert!(
        summary.success,
        "ingest summary should report success when unmapped commits do not require the events backend"
    );
    assert_eq!(summary.commits_processed, 2);
    assert_eq!(summary.checkpoint_companions_processed, 0);
    assert_eq!(summary.events_inserted, 0);

    let sqlite =
        rusqlite::Connection::open(sqlite_path_for_repo(repo.path())).expect("open sqlite");
    let file_delta_count: i64 = sqlite
        .query_row(
            "SELECT COUNT(*) FROM commit_file_deltas WHERE repo_id = ?1",
            rusqlite::params![cfg.repo.repo_id.as_str()],
            |row| row.get(0),
        )
        .expect("count commit_file_deltas rows");
    let commit_artefact_count: i64 = sqlite
        .query_row(
            "SELECT COUNT(*) FROM commit_artefacts WHERE repo_id = ?1",
            rusqlite::params![cfg.repo.repo_id.as_str()],
            |row| row.get(0),
        )
        .expect("count commit_artefacts rows");
    assert_eq!(
        file_delta_count, 0,
        "artefact-only ingest should not persist hunk-delta rows when no checkpoint companions are present"
    );
    assert!(
        commit_artefact_count > 0,
        "historical ingest should still persist commit artefacts when no checkpoint companions are present"
    );
}

#[tokio::test]
async fn execute_ingest_runs_checkpoint_companion_work_once_for_mapped_commits() {
    use crate::host::checkpoints::strategy::manual_commit::{
        WriteCommittedOptions, write_committed,
    };

    let repo = seed_git_repo();
    write_local_devql_config(repo.path());
    std::fs::create_dir_all(repo.path().join("src")).expect("create src");
    std::fs::write(
        repo.path().join("src/lib.rs"),
        "export const answer = () => 42;\n",
    )
    .expect("write lib.rs");
    git_ok(repo.path(), &["add", "."]);
    git_ok(repo.path(), &["commit", "-m", "add mapped file"]);
    let head_sha = git_ok(repo.path(), &["rev-parse", "HEAD"]);
    let cfg = cfg_for_repo(repo.path());
    execute_init_schema(&cfg, "commit-history mapped test")
        .await
        .expect("initialise local devql store for mapped checkpoint test");

    let checkpoint_id = "aabbccddeeff";
    write_committed(
        repo.path(),
        WriteCommittedOptions {
            checkpoint_id: checkpoint_id.to_string(),
            session_id: "session-1".to_string(),
            strategy: "manual-commit".to_string(),
            agent: "codex".to_string(),
            transcript: br#"{"checkpoint":true}"#.to_vec(),
            prompts: None,
            context: None,
            checkpoints_count: 1,
            files_touched: vec!["src/lib.rs".to_string()],
            token_usage_input: None,
            token_usage_output: None,
            token_usage_api_call_count: None,
            turn_id: String::new(),
            transcript_identifier_at_start: String::new(),
            checkpoint_transcript_start: 0,
            token_usage: None,
            initial_attribution: None,
            author_name: "Bitloops Test".to_string(),
            author_email: "bitloops-test@example.com".to_string(),
            summary: None,
            is_task: false,
            tool_use_id: String::new(),
            agent_id: String::new(),
            transcript_path: String::new(),
            subagent_transcript_path: String::new(),
        },
    )
    .expect("write committed checkpoint");
    insert_commit_checkpoint_mapping(repo.path(), &head_sha, checkpoint_id);

    let first_summary = execute_ingest_with_observer(&cfg, false, 500, None, None)
        .await
        .expect("execute ingest for mapped commit");
    assert!(
        first_summary.commits_processed >= 1,
        "initial catch-up may include earlier reachable commits"
    );
    assert_eq!(first_summary.checkpoint_companions_processed, 1);
    let replay_summary = execute_ingest_with_observer(&cfg, false, 500, None, None)
        .await
        .expect("replay ingest for mapped commit");
    assert_eq!(replay_summary.commits_processed, 0);
    assert_eq!(replay_summary.checkpoint_companions_processed, 0);

    let sqlite =
        rusqlite::Connection::open(sqlite_path_for_repo(repo.path())).expect("open sqlite");
    let checkpoint_projection_rows: i64 = sqlite
        .query_row(
            "SELECT COUNT(*) FROM checkpoint_files WHERE checkpoint_id = ?1 AND commit_sha = ?2",
            rusqlite::params![checkpoint_id, head_sha.as_str()],
            |row| row.get(0),
        )
        .expect("count checkpoint projections");
    assert!(checkpoint_projection_rows >= 1);

    let ledger_row: (String, String) = sqlite
        .query_row(
            "SELECT history_status, checkpoint_status
             FROM commit_ingest_ledger
             WHERE repo_id = ?1 AND commit_sha = ?2",
            rusqlite::params![cfg.repo.repo_id.as_str(), head_sha.as_str()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("read commit ingest ledger row");
    assert_eq!(ledger_row.0, "completed");
    assert_eq!(ledger_row.1, "completed");

    let duckdb = duckdb::Connection::open(duckdb_path_for_repo(repo.path())).expect("open duckdb");
    let checkpoint_event_rows: i64 = duckdb
        .query_row(
            "SELECT COUNT(*) FROM checkpoint_events WHERE checkpoint_id = ?",
            [checkpoint_id],
            |row| row.get(0),
        )
        .expect("count checkpoint events");
    assert_eq!(checkpoint_event_rows, 1);
}

#[tokio::test]
async fn execute_ingest_continues_after_failed_checkpoint_companion_commit() {
    let repo = seed_git_repo();
    write_local_devql_config(repo.path());
    std::fs::create_dir_all(repo.path().join("src")).expect("create src");

    std::fs::write(
        repo.path().join("src/lib.rs"),
        "pub fn one() -> i32 { 1 }\n",
    )
    .expect("write first revision");
    git_ok(repo.path(), &["add", "."]);
    git_ok(repo.path(), &["commit", "-m", "add one"]);

    std::fs::write(
        repo.path().join("src/lib.rs"),
        "pub fn one() -> i32 { 1 }\npub fn two() -> i32 { 2 }\n",
    )
    .expect("write second revision");
    git_ok(repo.path(), &["add", "."]);
    git_ok(repo.path(), &["commit", "-m", "add two"]);
    let mapped_commit_sha = git_ok(repo.path(), &["rev-parse", "HEAD"]);

    std::fs::write(
        repo.path().join("src/lib.rs"),
        "pub fn one() -> i32 { 1 }\npub fn two() -> i32 { 2 }\npub fn three() -> i32 { 3 }\n",
    )
    .expect("write third revision");
    git_ok(repo.path(), &["add", "."]);
    git_ok(repo.path(), &["commit", "-m", "add three"]);
    let head_sha = git_ok(repo.path(), &["rev-parse", "HEAD"]);

    let cfg = cfg_for_repo(repo.path());
    execute_init_schema(&cfg, "commit-history continue after failure test")
        .await
        .expect("initialise local devql store");
    insert_commit_checkpoint_mapping(repo.path(), &mapped_commit_sha, "abcdef123456");

    let summary = execute_ingest_with_observer(&cfg, false, 500, None, None)
        .await
        .expect("execute ingest with one failing mapped commit");

    assert!(
        !summary.success,
        "ingest summary should report partial failure when at least one commit fails"
    );

    let sqlite =
        rusqlite::Connection::open(sqlite_path_for_repo(repo.path())).expect("open sqlite");
    let mapped_ledger_row: (String, String, Option<String>) = sqlite
        .query_row(
            "SELECT history_status, checkpoint_status, last_error
             FROM commit_ingest_ledger
             WHERE repo_id = ?1 AND commit_sha = ?2",
            rusqlite::params![cfg.repo.repo_id.as_str(), mapped_commit_sha.as_str()],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .expect("read mapped commit ledger row");
    let head_ledger_row: (String, String) = sqlite
        .query_row(
            "SELECT history_status, checkpoint_status
             FROM commit_ingest_ledger
             WHERE repo_id = ?1 AND commit_sha = ?2",
            rusqlite::params![cfg.repo.repo_id.as_str(), head_sha.as_str()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("read head commit ledger row");

    assert_eq!(
        mapped_ledger_row.0, "completed",
        "history ingestion can complete before checkpoint companion fails"
    );
    assert_eq!(
        mapped_ledger_row.1, "failed",
        "mapped commit checkpoint status should record failure"
    );
    assert!(
        mapped_ledger_row
            .2
            .as_deref()
            .unwrap_or_default()
            .contains("checkpoint mapping exists but metadata is missing"),
        "expected checkpoint metadata failure in ledger error message"
    );
    assert_eq!(
        head_ledger_row.0, "completed",
        "ingest should continue processing commits after a failure"
    );
    assert_eq!(head_ledger_row.1, "not_applicable");
}

#[tokio::test]
async fn execute_ingest_records_after_side_file_artefact_for_blob_only_changes_without_full_state()
{
    let repo = seed_git_repo();
    write_local_devql_config(repo.path());
    std::fs::write(
        repo.path().join("Cargo.toml"),
        "[package]\nname = \"commit-history-test\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .expect("write Cargo.toml");
    std::fs::create_dir_all(repo.path().join("src")).expect("create src");
    std::fs::write(
        repo.path().join("src/lib.rs"),
        "pub fn stable() -> i32 { 1 }\n",
    )
    .expect("write first revision");
    git_ok(repo.path(), &["add", "."]);
    git_ok(repo.path(), &["commit", "-m", "add stable"]);

    std::fs::write(
        repo.path().join("src/lib.rs"),
        "// comment that changes the file blob only\npub fn stable() -> i32 { 1 }\n",
    )
    .expect("write second revision");
    git_ok(repo.path(), &["add", "."]);
    git_ok(repo.path(), &["commit", "-m", "add comment"]);
    let comment_sha = git_ok(repo.path(), &["rev-parse", "HEAD"]);

    let cfg = cfg_for_repo(repo.path());
    execute_init_schema(&cfg, "commit-history symbol content dedupe test")
        .await
        .expect("initialise local devql store for symbol content dedupe test");
    let summary = execute_ingest_with_observer(&cfg, false, 500, None, None)
        .await
        .expect("execute ingest for symbol content dedupe test");
    assert!(summary.success, "ingest should succeed");
    assert_eq!(summary.commits_processed, 3);

    let sqlite =
        rusqlite::Connection::open(sqlite_path_for_repo(repo.path())).expect("open sqlite");
    let file_delta_count: i64 = sqlite
        .query_row(
            "SELECT COUNT(*) \
             FROM commit_file_deltas \
             WHERE repo_id = ?1 AND commit_sha = ?2 AND path_after = 'src/lib.rs'",
            rusqlite::params![cfg.repo.repo_id.as_str(), comment_sha.as_str()],
            |row| row.get(0),
        )
        .expect("count artefact-only file deltas");
    let hunk_count: i64 = sqlite
        .query_row(
            "SELECT COUNT(*) \
             FROM commit_hunks \
             WHERE repo_id = ?1 AND commit_sha = ?2",
            rusqlite::params![cfg.repo.repo_id.as_str(), comment_sha.as_str()],
            |row| row.get(0),
        )
        .expect("count artefact-only hunks");
    let comment_commit_file_links: i64 = sqlite
        .query_row(
            "SELECT COUNT(*) \
             FROM commit_artefacts ca \
             JOIN artefacts a \
               ON a.repo_id = ca.repo_id \
              AND a.artefact_id = ca.artefact_id \
             WHERE ca.repo_id = ?1 \
               AND ca.commit_sha = ?2 \
               AND a.symbol_fqn = 'src/lib.rs' \
               AND a.canonical_kind = 'file'",
            rusqlite::params![cfg.repo.repo_id.as_str(), comment_sha.as_str()],
            |row| row.get(0),
        )
        .expect("count comment commit file artefact links");
    let artefact_count: i64 = sqlite
        .query_row(
            "SELECT COUNT(*) FROM artefacts WHERE repo_id = ?1",
            rusqlite::params![cfg.repo.repo_id.as_str()],
            |row| row.get(0),
        )
        .expect("count historical artefacts");
    let snapshot_count: i64 = sqlite
        .query_row(
            "SELECT COUNT(*) FROM artefact_snapshots WHERE repo_id = ?1",
            rusqlite::params![cfg.repo.repo_id.as_str()],
            |row| row.get(0),
        )
        .expect("count historical snapshots");
    assert_eq!(
        file_delta_count, 0,
        "artefact-only ingest should not persist file deltas for the blob-only change"
    );
    assert_eq!(
        hunk_count, 0,
        "artefact-only ingest should not persist hunks for the blob-only change"
    );
    assert_eq!(
        comment_commit_file_links, 1,
        "comment-only changes should still link the after-side file artefact"
    );
    assert!(
        artefact_count > 0,
        "changed blob revisions should still upsert artefact metadata"
    );
    assert_eq!(snapshot_count, 0);
}

#[tokio::test]
async fn execute_ingest_repairs_completed_commit_history_missing_artefact_metadata() {
    let repo = seed_git_repo();
    write_local_devql_config(repo.path());
    std::fs::write(
        repo.path().join("Cargo.toml"),
        "[package]\nname = \"commit-history-repair-test\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .expect("write Cargo.toml");
    std::fs::create_dir_all(repo.path().join("src")).expect("create src");
    std::fs::write(
        repo.path().join("src/lib.rs"),
        "pub fn repaired() -> i32 { 1 }\n",
    )
    .expect("write revision");
    git_ok(repo.path(), &["add", "."]);
    git_ok(repo.path(), &["commit", "-m", "add repaired"]);
    let head_sha = git_ok(repo.path(), &["rev-parse", "HEAD"]);

    let cfg = cfg_for_repo(repo.path());
    execute_init_schema(&cfg, "commit-history artefact metadata repair test")
        .await
        .expect("initialise local devql store for artefact metadata repair test");

    let first_summary = execute_ingest_with_backfill_window(&cfg, false, 1, None, None)
        .await
        .expect("execute initial bounded ingest");
    assert!(first_summary.artefacts_upserted > 0);

    let sqlite =
        rusqlite::Connection::open(sqlite_path_for_repo(repo.path())).expect("open sqlite");
    sqlite
        .execute(
            "DELETE FROM artefacts WHERE repo_id = ?1",
            rusqlite::params![cfg.repo.repo_id.as_str()],
        )
        .expect("remove artefact metadata to simulate pre-repair artefact-only ingest");

    let repair_summary = execute_ingest_with_backfill_window(&cfg, false, 1, None, None)
        .await
        .expect("execute bounded repair ingest");
    assert_eq!(
        repair_summary.commits_processed, 1,
        "bounded ingest should revisit completed commits whose commit artefacts lack artefact metadata"
    );
    assert!(
        repair_summary.artefacts_upserted > 0,
        "repair ingest should restore artefact metadata"
    );

    let file_delta_rows: i64 = sqlite
        .query_row(
            "SELECT COUNT(*) FROM commit_file_deltas WHERE repo_id = ?1 AND commit_sha = ?2",
            rusqlite::params![cfg.repo.repo_id.as_str(), head_sha.as_str()],
            |row| row.get(0),
        )
        .expect("count repaired commit file deltas");
    let commit_artefact_rows: i64 = sqlite
        .query_row(
            "SELECT COUNT(*) FROM commit_artefacts WHERE repo_id = ?1 AND commit_sha = ?2",
            rusqlite::params![cfg.repo.repo_id.as_str(), head_sha.as_str()],
            |row| row.get(0),
        )
        .expect("count repaired commit artefact links");
    let artefact_rows: i64 = sqlite
        .query_row(
            "SELECT COUNT(*) FROM artefacts WHERE repo_id = ?1",
            rusqlite::params![cfg.repo.repo_id.as_str()],
            |row| row.get(0),
        )
        .expect("count repaired artefact metadata");
    let file_state_rows: i64 = sqlite
        .query_row(
            "SELECT COUNT(*) FROM file_state WHERE repo_id = ?1",
            rusqlite::params![cfg.repo.repo_id.as_str()],
            |row| row.get(0),
        )
        .expect("count repaired file_state rows");
    let snapshot_rows: i64 = sqlite
        .query_row(
            "SELECT COUNT(*) FROM artefact_snapshots WHERE repo_id = ?1",
            rusqlite::params![cfg.repo.repo_id.as_str()],
            |row| row.get(0),
        )
        .expect("count repaired snapshots");

    assert_eq!(file_delta_rows, 0);
    assert!(commit_artefact_rows > 0);
    assert!(artefact_rows > 0);
    assert_eq!(file_state_rows, 0);
    assert_eq!(snapshot_rows, 0);
}

#[tokio::test]
async fn execute_ingest_hidden_max_commits_cap_limits_commit_replay_without_public_api_changes() {
    use rusqlite::OptionalExtension;

    let repo = seed_git_repo();
    write_local_devql_config(repo.path());
    std::fs::create_dir_all(repo.path().join("src")).expect("create src");
    std::fs::write(
        repo.path().join("src/lib.rs"),
        "pub fn one() -> i32 { 1 }\n",
    )
    .expect("write first revision");
    git_ok(repo.path(), &["add", "."]);
    git_ok(repo.path(), &["commit", "-m", "add one"]);

    std::fs::write(
        repo.path().join("src/lib.rs"),
        "pub fn one() -> i32 { 1 }\npub fn two() -> i32 { 2 }\n",
    )
    .expect("write second revision");
    git_ok(repo.path(), &["add", "."]);
    git_ok(repo.path(), &["commit", "-m", "add two"]);
    let head_sha = git_ok(repo.path(), &["rev-parse", "HEAD"]);

    let cfg = cfg_for_repo(repo.path());
    execute_init_schema(&cfg, "commit-history hidden max commits test")
        .await
        .expect("initialise local devql store for hidden max commits test");

    let summary = execute_ingest_with_observer(&cfg, false, 1, None, None)
        .await
        .expect("execute ingest with hidden max commit cap");
    assert_eq!(summary.commits_processed, 1);

    let sqlite =
        rusqlite::Connection::open(sqlite_path_for_repo(repo.path())).expect("open sqlite");
    let ingested_commit_count: i64 = sqlite
        .query_row(
            "SELECT COUNT(*) FROM commit_ingest_ledger WHERE repo_id = ?1 AND history_status = 'completed'",
            rusqlite::params![cfg.repo.repo_id.as_str()],
            |row| row.get(0),
        )
        .expect("count completed ledger rows");
    let head_ledger: Option<String> = sqlite
        .query_row(
            "SELECT history_status FROM commit_ingest_ledger WHERE repo_id = ?1 AND commit_sha = ?2",
            rusqlite::params![cfg.repo.repo_id.as_str(), head_sha.as_str()],
            |row| row.get(0),
        )
        .optional()
        .expect("read head ledger row");
    assert_eq!(ingested_commit_count, 1);
    assert!(head_ledger.is_none(), "head commit should remain pending");
}

#[tokio::test]
async fn execute_ingest_with_backfill_window_targets_latest_commits_and_can_reach_older_skipped_history_later()
 {
    use rusqlite::OptionalExtension;

    let repo = seed_git_repo();
    write_local_devql_config(repo.path());
    std::fs::create_dir_all(repo.path().join("src")).expect("create src");

    std::fs::write(
        repo.path().join("src/lib.rs"),
        "pub fn one() -> i32 { 1 }\n",
    )
    .expect("write first revision");
    git_ok(repo.path(), &["add", "."]);
    git_ok(repo.path(), &["commit", "-m", "add one"]);
    let first_sha = git_ok(repo.path(), &["rev-parse", "HEAD"]);

    std::fs::write(
        repo.path().join("src/lib.rs"),
        "pub fn one() -> i32 { 1 }\npub fn two() -> i32 { 2 }\n",
    )
    .expect("write second revision");
    git_ok(repo.path(), &["add", "."]);
    git_ok(repo.path(), &["commit", "-m", "add two"]);
    let second_sha = git_ok(repo.path(), &["rev-parse", "HEAD"]);

    std::fs::write(
        repo.path().join("src/lib.rs"),
        "pub fn one() -> i32 { 1 }\npub fn two() -> i32 { 2 }\npub fn three() -> i32 { 3 }\n",
    )
    .expect("write third revision");
    git_ok(repo.path(), &["add", "."]);
    git_ok(repo.path(), &["commit", "-m", "add three"]);
    let third_sha = git_ok(repo.path(), &["rev-parse", "HEAD"]);

    let cfg = cfg_for_repo(repo.path());
    execute_init_schema(&cfg, "commit-history bounded backfill test")
        .await
        .expect("initialise local devql store");

    let first_summary = execute_ingest_with_backfill_window(&cfg, false, 1, None, None)
        .await
        .expect("execute bounded ingest");
    assert_eq!(first_summary.commits_processed, 1);

    let sqlite =
        rusqlite::Connection::open(sqlite_path_for_repo(repo.path())).expect("open sqlite");
    let first_ledger: Option<String> = sqlite
        .query_row(
            "SELECT history_status FROM commit_ingest_ledger WHERE repo_id = ?1 AND commit_sha = ?2",
            rusqlite::params![cfg.repo.repo_id.as_str(), first_sha.as_str()],
            |row| row.get(0),
        )
        .optional()
        .expect("read first commit ledger row");
    let second_ledger: Option<String> = sqlite
        .query_row(
            "SELECT history_status FROM commit_ingest_ledger WHERE repo_id = ?1 AND commit_sha = ?2",
            rusqlite::params![cfg.repo.repo_id.as_str(), second_sha.as_str()],
            |row| row.get(0),
        )
        .optional()
        .expect("read second commit ledger row");
    let third_ledger: Option<String> = sqlite
        .query_row(
            "SELECT history_status FROM commit_ingest_ledger WHERE repo_id = ?1 AND commit_sha = ?2",
            rusqlite::params![cfg.repo.repo_id.as_str(), third_sha.as_str()],
            |row| row.get(0),
        )
        .optional()
        .expect("read third commit ledger row");

    assert!(
        first_ledger.is_none(),
        "oldest commit should stay skipped by backfill=1"
    );
    assert!(
        second_ledger.is_none(),
        "middle commit should stay skipped by backfill=1"
    );
    assert_eq!(third_ledger.as_deref(), Some("completed"));

    let replay_summary = execute_ingest_with_backfill_window(&cfg, false, 3, None, None)
        .await
        .expect("execute larger bounded ingest");
    assert_eq!(replay_summary.commits_processed, 2);

    let first_ledger: Option<String> = sqlite
        .query_row(
            "SELECT history_status FROM commit_ingest_ledger WHERE repo_id = ?1 AND commit_sha = ?2",
            rusqlite::params![cfg.repo.repo_id.as_str(), first_sha.as_str()],
            |row| row.get(0),
        )
        .optional()
        .expect("read first commit ledger row");
    let second_ledger: Option<String> = sqlite
        .query_row(
            "SELECT history_status FROM commit_ingest_ledger WHERE repo_id = ?1 AND commit_sha = ?2",
            rusqlite::params![cfg.repo.repo_id.as_str(), second_sha.as_str()],
            |row| row.get(0),
        )
        .optional()
        .expect("read second commit ledger row");

    assert_eq!(first_ledger.as_deref(), Some("completed"));
    assert_eq!(second_ledger.as_deref(), Some("completed"));
}

#[tokio::test]
async fn execute_ingest_recovers_older_skipped_history_after_bounded_backfill() {
    use rusqlite::OptionalExtension;

    let repo = seed_git_repo();
    write_local_devql_config(repo.path());
    std::fs::create_dir_all(repo.path().join("src")).expect("create src");

    std::fs::write(
        repo.path().join("src/lib.rs"),
        "pub fn one() -> i32 { 1 }\n",
    )
    .expect("write first revision");
    git_ok(repo.path(), &["add", "."]);
    git_ok(repo.path(), &["commit", "-m", "add one"]);
    let first_sha = git_ok(repo.path(), &["rev-parse", "HEAD"]);

    std::fs::write(
        repo.path().join("src/lib.rs"),
        "pub fn one() -> i32 { 1 }\npub fn two() -> i32 { 2 }\n",
    )
    .expect("write second revision");
    git_ok(repo.path(), &["add", "."]);
    git_ok(repo.path(), &["commit", "-m", "add two"]);
    let second_sha = git_ok(repo.path(), &["rev-parse", "HEAD"]);

    std::fs::write(
        repo.path().join("src/lib.rs"),
        "pub fn one() -> i32 { 1 }\npub fn two() -> i32 { 2 }\npub fn three() -> i32 { 3 }\n",
    )
    .expect("write third revision");
    git_ok(repo.path(), &["add", "."]);
    git_ok(repo.path(), &["commit", "-m", "add three"]);
    let third_sha = git_ok(repo.path(), &["rev-parse", "HEAD"]);

    let cfg = cfg_for_repo(repo.path());
    execute_init_schema(
        &cfg,
        "commit-history bounded backfill full-ingest catchup test",
    )
    .await
    .expect("initialise local devql store");

    let first_summary = execute_ingest_with_backfill_window(&cfg, false, 1, None, None)
        .await
        .expect("execute bounded ingest");
    assert_eq!(first_summary.commits_processed, 1);

    let sqlite =
        rusqlite::Connection::open(sqlite_path_for_repo(repo.path())).expect("open sqlite");
    let first_ledger: Option<String> = sqlite
        .query_row(
            "SELECT history_status FROM commit_ingest_ledger WHERE repo_id = ?1 AND commit_sha = ?2",
            rusqlite::params![cfg.repo.repo_id.as_str(), first_sha.as_str()],
            |row| row.get(0),
        )
        .optional()
        .expect("read first commit ledger row");
    let second_ledger: Option<String> = sqlite
        .query_row(
            "SELECT history_status FROM commit_ingest_ledger WHERE repo_id = ?1 AND commit_sha = ?2",
            rusqlite::params![cfg.repo.repo_id.as_str(), second_sha.as_str()],
            |row| row.get(0),
        )
        .optional()
        .expect("read second commit ledger row");
    let third_ledger: Option<String> = sqlite
        .query_row(
            "SELECT history_status FROM commit_ingest_ledger WHERE repo_id = ?1 AND commit_sha = ?2",
            rusqlite::params![cfg.repo.repo_id.as_str(), third_sha.as_str()],
            |row| row.get(0),
        )
        .optional()
        .expect("read third commit ledger row");

    assert!(
        first_ledger.is_none(),
        "oldest commit should stay skipped by backfill=1"
    );
    assert!(
        second_ledger.is_none(),
        "middle commit should stay skipped by backfill=1"
    );
    assert_eq!(third_ledger.as_deref(), Some("completed"));

    let replay_summary = execute_ingest_with_observer(&cfg, false, 0, None, None)
        .await
        .expect("execute full ingest catch-up");
    assert_eq!(replay_summary.commits_processed, 3);

    let first_ledger: Option<String> = sqlite
        .query_row(
            "SELECT history_status FROM commit_ingest_ledger WHERE repo_id = ?1 AND commit_sha = ?2",
            rusqlite::params![cfg.repo.repo_id.as_str(), first_sha.as_str()],
            |row| row.get(0),
        )
        .optional()
        .expect("read first commit ledger row");
    let second_ledger: Option<String> = sqlite
        .query_row(
            "SELECT history_status FROM commit_ingest_ledger WHERE repo_id = ?1 AND commit_sha = ?2",
            rusqlite::params![cfg.repo.repo_id.as_str(), second_sha.as_str()],
            |row| row.get(0),
        )
        .optional()
        .expect("read second commit ledger row");

    assert_eq!(first_ledger.as_deref(), Some("completed"));
    assert_eq!(second_ledger.as_deref(), Some("completed"));
}
