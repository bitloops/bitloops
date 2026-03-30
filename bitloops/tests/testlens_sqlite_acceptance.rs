use std::fs;
mod test_harness_support;

use rusqlite::{Connection, params};
use serde_json::Value;
use test_harness_support::{
    Workspace, bootstrap_codex_workspace, load_symbol_fqn, run_bitloops_or_panic,
    seed_production_artefacts,
    write_rust_coverage_fixture,
};

#[test]
#[ignore = "slow E2E: spawns bitloops binary; run with `cargo test -- --ignored`"]
fn bitloops_devql_init_initialises_sqlite_test_harness_tables() {
    let workspace = Workspace::new("sqlite-init");

    bootstrap_codex_workspace(&workspace);

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

    let pre_devql_test_scenarios_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'test_scenarios'",
            [],
            |row| row.get(0),
        )
        .expect("query pre-devql test_scenarios table");
    assert_eq!(
        pre_devql_test_scenarios_count, 0,
        "did not expect test-harness tables before devql init"
    );

    run_bitloops_or_panic(workspace.repo_dir(), &["devql", "init"]);

    for table in [
        "test_suites",
        "test_scenarios",
        "test_links",
        "coverage_captures",
        "coverage_hits",
        "test_discovery_runs",
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
#[ignore = "slow E2E: spawns bitloops binary; run with `cargo test -- --ignored`"]
fn bitloops_testlens_ingest_coverage_reports_artefact_only_mode_on_sqlite() {
    let workspace = Workspace::new("sqlite-coverage");
    write_rust_coverage_fixture(&workspace);

    bootstrap_codex_workspace(&workspace);
    run_bitloops_or_panic(workspace.repo_dir(), &["devql", "init"]);

    for commit in ["C0", "C1"] {
        seed_production_artefacts(&workspace, commit);
        run_bitloops_or_panic(
            workspace.repo_dir(),
            &["testlens", "ingest-tests", "--commit", commit],
        );
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

    run_bitloops_or_panic(
        workspace.repo_dir(),
        &[
            "testlens",
            "ingest-coverage",
            "--commit",
            "C1",
            "--scope",
            "workspace",
            "--tool",
            "cargo-llvm-cov",
            "--lcov",
            lcov_path.to_str().expect("lcov path should be utf-8"),
        ],
    );

    let conn = Connection::open(workspace.db_path()).expect("open sqlite db");
    let rust_find_by_id = load_symbol_fqn(&conn, "C1", "%find_by_id");

    let c0_query: Value = serde_json::from_str(&run_bitloops_or_panic(
        workspace.repo_dir(),
        &[
            "testlens",
            "query",
            "--artefact",
            &rust_find_by_id,
            "--commit",
            "C0",
            "--view",
            "coverage",
        ],
    ))
    .expect("parse C0 query json");
    assert!(
        c0_query["coverage"].is_null(),
        "expected null coverage payload for commit C0"
    );

    let c1_summary: Value = serde_json::from_str(&run_bitloops_or_panic(
        workspace.repo_dir(),
        &[
            "testlens",
            "query",
            "--artefact",
            &rust_find_by_id,
            "--commit",
            "C1",
            "--view",
            "summary",
        ],
    ))
    .expect("parse C1 summary json");
    assert_eq!(
        c1_summary["summary"]["coverage_mode"].as_str(),
        Some("artefact_only"),
        "expected workspace coverage to remain artefact_only"
    );
    assert!(
        c1_summary["summary"]["line_coverage_pct"].is_number(),
        "expected line coverage percentage in summary"
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
