use std::fs;
mod test_harness_support;

use bitloops::capability_packs::test_harness::storage::TestHarnessQueryRepository;
use bitloops::models::{CoverageFormat, ScopeKind};
use rusqlite::{Connection, params};
use test_harness_support::{
    Workspace, bootstrap_minimal_workspace, ingest_test_harness_coverage,
    ingest_test_harness_tests, open_test_harness_repository, run_devql_init,
    seed_production_artefacts, write_rust_coverage_fixture,
};

#[test]
fn bitloops_devql_init_initialises_sqlite_test_harness_tables() {
    let workspace = Workspace::new("sqlite-init");

    bootstrap_minimal_workspace(&workspace);

    assert!(
        workspace.db_path().is_file(),
        "expected sqlite db at {}",
        workspace.db_path().display()
    );

    let conn = Connection::open(workspace.db_path()).expect("open sqlite db");
    let sessions_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'sessions'",
            [],
            |row| row.get(0),
        )
        .expect("query sessions table");
    assert_eq!(
        sessions_count, 1,
        "expected checkpoint/session schema after init"
    );

    let pre_devql_test_artefacts_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'test_artefacts_current'",
            [],
            |row| row.get(0),
        )
        .expect("query pre-devql test_artefacts_current table");
    assert_eq!(
        pre_devql_test_artefacts_count, 0,
        "did not expect test-harness tables before devql init"
    );

    run_devql_init(&workspace);

    for table in [
        "test_artefacts_current",
        "test_artefact_edges_current",
        "coverage_captures",
        "coverage_hits",
    ] {
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
                [table],
                |row| row.get(0),
            )
            .expect("query sqlite schema");
        assert_eq!(count, 1, "expected sqlite table `{table}`");
    }
}

#[test]
fn bitloops_testlens_ingest_coverage_records_sqlite_hits_without_cli_spawn() {
    let workspace = Workspace::new("sqlite-coverage");
    write_rust_coverage_fixture(&workspace);

    bootstrap_minimal_workspace(&workspace);
    run_devql_init(&workspace);

    for commit in ["C0", "C1"] {
        seed_production_artefacts(&workspace, commit);
        ingest_test_harness_tests(&workspace, commit);
    }

    let lcov_path = workspace.repo_dir().join("rust-coverage.lcov");
    fs::write(
        &lcov_path,
        r#"
TN:
SF:src/lib.rs
DA:4,1
DA:5,1
DA:8,0
DA:9,0
end_of_record
"#
        .trim_start(),
    )
    .expect("write lcov");

    ingest_test_harness_coverage(
        &workspace,
        "C1",
        &lcov_path,
        ScopeKind::Workspace,
        "cargo-llvm-cov",
        CoverageFormat::Lcov,
    );

    let conn = Connection::open(workspace.db_path()).expect("open sqlite db");
    let repository = open_test_harness_repository(&workspace);
    assert!(
        !repository
            .coverage_exists_for_commit("C0")
            .expect("check C0 coverage state"),
        "expected no coverage payload for commit C0"
    );
    assert!(
        repository
            .coverage_exists_for_commit("C1")
            .expect("check C1 coverage state"),
        "expected coverage payload for commit C1"
    );
    let covered_symbol_id: String = conn
        .query_row(
            r#"
SELECT ch.production_symbol_id
FROM coverage_hits ch
JOIN coverage_captures cc ON cc.capture_id = ch.capture_id
WHERE cc.commit_sha = ?1
  AND ch.branch_id = -1
  AND ch.covered = 1
ORDER BY ch.production_symbol_id, ch.line
LIMIT 1
"#,
            params!["C1"],
            |row| row.get(0),
        )
        .expect("load a covered production symbol for C1");
    let c1_summary = repository
        .load_coverage_summary("C1", &covered_symbol_id)
        .expect("load C1 coverage summary")
        .expect("expected coverage summary for commit C1");
    assert!(
        c1_summary.line_total > 0,
        "expected line coverage rows for C1"
    );
    assert!(
        c1_summary.line_covered > 0,
        "expected at least one covered line for C1"
    );

    let c1_hit_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM coverage_hits WHERE capture_id IN (SELECT capture_id FROM coverage_captures WHERE commit_sha = ?1)",
            params!["C1"],
            |row| row.get(0),
        )
        .expect("count C1 coverage hits");
    let c0_hit_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM coverage_hits WHERE capture_id IN (SELECT capture_id FROM coverage_captures WHERE commit_sha = ?1)",
            params!["C0"],
            |row| row.get(0),
        )
        .expect("count C0 coverage hits");
    assert_eq!(c0_hit_count, 0, "expected no coverage hits for C0");
    assert!(c1_hit_count > 0, "expected coverage hits for C1");
}
