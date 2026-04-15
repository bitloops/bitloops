mod test_harness_support;

use std::fs;

use bitloops::storage::init::init_database;
use rusqlite::{Connection, params};
use tempfile::tempdir;
use test_harness_support::production_seed::seed_production_artefacts_for_repo;

#[test]
fn discovers_supported_production_artefacts_and_ignores_tests() {
    let temp = tempdir().expect("failed to create temp dir");
    let repo_dir = temp.path().join("repo");
    let db_path = temp.path().join("testlens.db");

    fs::create_dir_all(repo_dir.join("src")).expect("failed creating src dir");
    fs::create_dir_all(repo_dir.join("tests")).expect("failed creating tests dir");

    fs::write(
        repo_dir.join("src/service.ts"),
        r#"
export interface User {
  id: string;
}

export type UserId = string;
export const MAX_RETRIES = 3;

export function validateEmail(email: string): boolean {
  return email.includes("@");
}

export class UserService {
  createUser(name: string): string {
    return name.trim();
  }
}
"#,
    )
    .expect("failed writing service.ts");

    fs::write(
        repo_dir.join("src/lib.rs"),
        r#"
pub fn normalize(input: &str) -> String {
    input.trim().to_lowercase()
}
"#,
    )
    .expect("failed writing lib.rs");

    fs::write(
        repo_dir.join("src/service.go"),
        r#"
package service

func Run() string {
    return "ok"
}
"#,
    )
    .expect("failed writing service.go");

    fs::write(
        repo_dir.join("tests/service.test.ts"),
        r#"
describe("service", () => {
  it("works", () => {
    expect(true).toBe(true);
  });
});
"#,
    )
    .expect("failed writing test file");

    init_database(&db_path, false, "ignored").expect("failed to init db");
    seed_production_artefacts_for_repo(&db_path, &repo_dir, "c1").expect("ingest should succeed");

    let conn = Connection::open(db_path).expect("failed to open db");

    let file_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM file_state WHERE commit_sha = 'c1'",
            [],
            |row| row.get(0),
        )
        .expect("failed querying file count");
    assert_eq!(file_count, 3, "expected only src files, not test files");

    assert_has_symbol(&conn, "c1", "function", "src/service.ts::validateEmail");
    assert_has_symbol(&conn, "c1", "<null>", "src/service.ts::UserService");
    assert_has_symbol(
        &conn,
        "c1",
        "method",
        "src/service.ts::UserService::createUser",
    );
    assert_has_symbol(&conn, "c1", "interface", "src/service.ts::User");
    assert_has_symbol(&conn, "c1", "type", "src/service.ts::UserId");
    assert_has_symbol(&conn, "c1", "variable", "src/service.ts::MAX_RETRIES");
    assert_has_symbol(&conn, "c1", "function", "src/lib.rs::normalize");
    assert_has_symbol(&conn, "c1", "function", "src/service.go::Run");

    let test_path_rows: i64 = conn
        .query_row(
            r#"
SELECT COUNT(*)
FROM file_state
WHERE commit_sha = 'c1'
  AND path LIKE 'tests/%'
"#,
            [],
            |row| row.get(0),
        )
        .expect("failed querying test path rows");
    assert_eq!(
        test_path_rows, 0,
        "production ingestion should not ingest tests/* files"
    );
}

#[test]
fn rerun_replaces_stale_production_artefacts_for_same_commit() {
    let temp = tempdir().expect("failed to create temp dir");
    let repo_dir = temp.path().join("repo");
    let db_path = temp.path().join("testlens.db");

    fs::create_dir_all(repo_dir.join("src")).expect("failed creating src dir");
    fs::write(
        repo_dir.join("src/service.ts"),
        r#"
export function oldName(): string {
  return "old";
}
"#,
    )
    .expect("failed writing initial service.ts");

    init_database(&db_path, false, "ignored").expect("failed to init db");
    seed_production_artefacts_for_repo(&db_path, &repo_dir, "c1")
        .expect("first ingest should succeed");

    fs::write(
        repo_dir.join("src/service.ts"),
        r#"
export function newName(): string {
  return "new";
}
"#,
    )
    .expect("failed updating service.ts");

    seed_production_artefacts_for_repo(&db_path, &repo_dir, "c1")
        .expect("second ingest should succeed");

    let conn = Connection::open(db_path).expect("failed to open db");
    assert_has_symbol(&conn, "c1", "function", "src/service.ts::newName");

    let old_count: i64 = conn
        .query_row(
            r#"
SELECT COUNT(*)
FROM file_state fs
JOIN artefacts_historical a
  ON a.repo_id = fs.repo_id
 AND a.blob_sha = fs.blob_sha
 AND a.path = fs.path
WHERE fs.commit_sha = 'c1'
  AND a.symbol_fqn = 'src/service.ts::oldName'
"#,
            [],
            |row| row.get(0),
        )
        .expect("failed querying old symbol count");
    assert_eq!(old_count, 0, "stale symbol oldName should be removed");
}

#[test]
fn keeps_distinct_trait_impl_methods_for_same_type() {
    let temp = tempdir().expect("failed to create temp dir");
    let repo_dir = temp.path().join("repo");
    let db_path = temp.path().join("testlens.db");

    fs::create_dir_all(repo_dir.join("src")).expect("failed creating src dir");
    fs::write(
        repo_dir.join("src/lib.rs"),
        r#"
struct Widget;

trait RenderHtml {
    fn render(&self) -> String;
}

trait RenderText {
    fn render(&self) -> String;
}

impl RenderHtml for Widget {
    fn render(&self) -> String {
        "<div/>".to_string()
    }
}

impl RenderText for Widget {
    fn render(&self) -> String {
        "widget".to_string()
    }
}
"#,
    )
    .expect("failed writing lib.rs");

    init_database(&db_path, false, "ignored").expect("failed to init db");
    seed_production_artefacts_for_repo(&db_path, &repo_dir, "c1").expect("ingest should succeed");

    let conn = Connection::open(db_path).expect("failed to open db");
    let method_count: i64 = conn
        .query_row(
            r#"
SELECT COUNT(*)
FROM file_state fs
JOIN artefacts_historical a
  ON a.repo_id = fs.repo_id
 AND a.blob_sha = fs.blob_sha
 AND a.path = fs.path
WHERE fs.commit_sha = 'c1'
  AND a.canonical_kind = 'method'
  AND a.path = 'src/lib.rs'
"#,
            [],
            |row| row.get(0),
        )
        .expect("failed querying method count");
    assert_eq!(
        method_count, 2,
        "trait impl methods with the same name should not collide"
    );
}

fn assert_has_symbol(conn: &Connection, commit: &str, kind: &str, symbol: &str) {
    let count: i64 = conn
        .query_row(
            r#"
SELECT COUNT(*)
FROM file_state fs
JOIN artefacts_historical a
  ON a.repo_id = fs.repo_id
 AND a.blob_sha = fs.blob_sha
 AND a.path = fs.path
WHERE fs.commit_sha = ?1
  AND a.canonical_kind = ?2
  AND a.symbol_fqn = ?3
"#,
            params![commit, kind, symbol],
            |row| row.get(0),
        )
        .expect("failed querying symbol presence");
    assert!(count > 0, "expected symbol `{symbol}` of kind `{kind}`");
}
