use super::*;

fn cfg_for_repo(repo_root: &Path) -> DevqlConfig {
    let repo = resolve_repo_identity(repo_root).expect("resolve repo identity");
    DevqlConfig::from_env(repo_root.to_path_buf(), repo).expect("build devql cfg from repo")
}

fn count_rows(conn: &rusqlite::Connection, sql: &str, repo_id: &str) -> i64 {
    conn.query_row(sql, rusqlite::params![repo_id], |row| row.get(0))
        .expect("count rows")
}

#[test]
fn discover_baseline_files_keeps_supported_extensions_only() {
    let repo = seed_git_repo();
    std::fs::create_dir_all(repo.path().join("src")).expect("create src");
    std::fs::write(repo.path().join("src/lib.rs"), "pub fn run() {}\n").expect("write rust file");
    std::fs::write(
        repo.path().join("src/index.ts"),
        "export const value = 1;\n",
    )
    .expect("write ts file");
    std::fs::write(
        repo.path().join("src/view.tsx"),
        "export const View = () => null;\n",
    )
    .expect("write tsx file");
    std::fs::write(repo.path().join("src/node.js"), "module.exports = {};\n")
        .expect("write js file");
    std::fs::write(
        repo.path().join("src/component.jsx"),
        "export const C = () => null;\n",
    )
    .expect("write jsx file");
    std::fs::write(repo.path().join("README.md"), "# docs\n").expect("write markdown file");

    git_ok(repo.path(), &["add", "."]);
    git_ok(repo.path(), &["commit", "-m", "add files"]);

    let files = discover_baseline_files(repo.path()).expect("discover baseline files");
    assert_eq!(
        files,
        vec![
            "src/component.jsx".to_string(),
            "src/index.ts".to_string(),
            "src/lib.rs".to_string(),
            "src/node.js".to_string(),
            "src/view.tsx".to_string(),
        ]
    );
}

#[tokio::test]
async fn baseline_ingestion_populates_current_state_and_sync_state_for_active_branch() {
    let repo = seed_git_repo();
    std::fs::create_dir_all(repo.path().join("src")).expect("create src");
    std::fs::write(
        repo.path().join("src/lib.rs"),
        "pub fn greet(name: &str) -> String { format!(\"hi {name}\") }\n",
    )
    .expect("write rust file");
    std::fs::write(
        repo.path().join("src/index.ts"),
        "export function sum(a: number, b: number) { return a + b; }\n",
    )
    .expect("write ts file");
    git_ok(repo.path(), &["add", "."]);
    git_ok(repo.path(), &["commit", "-m", "add baseline files"]);

    let sqlite_path = repo.path().join("relational.db");
    let relational = sqlite_relational_store_with_schema(&sqlite_path).await;
    let cfg = cfg_for_repo(repo.path());

    run_baseline_ingestion(&cfg, &relational)
        .await
        .expect("run baseline ingestion");

    let head_sha = run_git(repo.path(), &["rev-parse", "HEAD"]).expect("resolve HEAD");
    let conn = rusqlite::Connection::open(sqlite_path).expect("open sqlite");

    let current_file_state_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM current_file_state WHERE repo_id = ?1",
            rusqlite::params![cfg.repo.repo_id.as_str()],
            |row| row.get(0),
        )
        .expect("count current_file_state rows");
    assert_eq!(current_file_state_count, 2);

    let current_count = count_rows(
        &conn,
        "SELECT COUNT(*) FROM artefacts_current WHERE repo_id = ?1",
        cfg.repo.repo_id.as_str(),
    );
    assert!(
        current_count >= 2,
        "expected baseline to persist current-state rows"
    );

    let baseline_sha: String = conn
        .query_row(
            "SELECT state_value FROM sync_state WHERE repo_id = ?1 AND state_key = 'baseline_commit_sha'",
            rusqlite::params![cfg.repo.repo_id.as_str()],
            |row| row.get(0),
        )
        .expect("read baseline watermark");
    assert_eq!(baseline_sha, head_sha);
}

#[tokio::test]
async fn baseline_ingestion_is_idempotent_when_head_is_unchanged() {
    let repo = seed_git_repo();
    std::fs::create_dir_all(repo.path().join("src")).expect("create src");
    std::fs::write(
        repo.path().join("src/main.ts"),
        "export const value = 1;\nexport function read() { return value; }\n",
    )
    .expect("write ts file");
    git_ok(repo.path(), &["add", "."]);
    git_ok(repo.path(), &["commit", "-m", "add file"]);

    let sqlite_path = repo.path().join("relational.db");
    let relational = sqlite_relational_store_with_schema(&sqlite_path).await;
    let cfg = cfg_for_repo(repo.path());

    run_baseline_ingestion(&cfg, &relational)
        .await
        .expect("first baseline run");

    let conn = rusqlite::Connection::open(&sqlite_path).expect("open sqlite");
    let artefacts_current_first = count_rows(
        &conn,
        "SELECT COUNT(*) FROM artefacts_current WHERE repo_id = ?1",
        cfg.repo.repo_id.as_str(),
    );
    let edges_current_first = count_rows(
        &conn,
        "SELECT COUNT(*) FROM artefact_edges_current WHERE repo_id = ?1",
        cfg.repo.repo_id.as_str(),
    );
    let historical_first: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM artefacts WHERE repo_id = ?1",
            rusqlite::params![cfg.repo.repo_id.as_str()],
            |row| row.get(0),
        )
        .expect("count historical rows");
    drop(conn);

    run_baseline_ingestion(&cfg, &relational)
        .await
        .expect("second baseline run");

    let conn = rusqlite::Connection::open(&sqlite_path).expect("open sqlite after rerun");
    let artefacts_current_second = count_rows(
        &conn,
        "SELECT COUNT(*) FROM artefacts_current WHERE repo_id = ?1",
        cfg.repo.repo_id.as_str(),
    );
    let edges_current_second = count_rows(
        &conn,
        "SELECT COUNT(*) FROM artefact_edges_current WHERE repo_id = ?1",
        cfg.repo.repo_id.as_str(),
    );
    let historical_second: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM artefacts WHERE repo_id = ?1",
            rusqlite::params![cfg.repo.repo_id.as_str()],
            |row| row.get(0),
        )
        .expect("count historical rows after rerun");

    assert_eq!(artefacts_current_first, artefacts_current_second);
    assert_eq!(edges_current_first, edges_current_second);
    assert_eq!(historical_first, historical_second);
}
