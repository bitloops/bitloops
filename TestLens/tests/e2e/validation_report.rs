use std::fs;

use crate::support::cli::project_root;

struct JiraRow {
    jira: &'static str,
    scope: &'static str,
    rust_first: &'static str,
    typescript_parity: &'static str,
    state: &'static str,
    notes: &'static str,
}

struct ScenarioRow {
    scenario: &'static str,
    coverage: &'static str,
    notes: &'static str,
}

#[test]
fn emits_validation_status_report() {
    let jira_rows = [
        JiraRow {
            jira: "CLI-1345",
            scope: "Test artefact discovery",
            rust_first: "yes",
            typescript_parity: "yes",
            state: "covered",
            notes: "Executable BDD coverage exists on the synthetic mixed-language fixture.",
        },
        JiraRow {
            jira: "CLI-1346",
            scope: "Static linkage",
            rust_first: "yes",
            typescript_parity: "yes",
            state: "covered",
            notes: "Executable BDD coverage exists on the synthetic mixed-language fixture.",
        },
        JiraRow {
            jira: "CLI-1347",
            scope: "Query levels, noise control, and query errors",
            rust_first: "partial",
            typescript_parity: "yes",
            state: "covered",
            notes: "TypeScript fixture covers summary/tests/coverage views, untested artefacts, and ERR1/ERR2; Rust quickstart still covers the static query path rather than every query view.",
        },
        JiraRow {
            jira: "CLI-1348",
            scope: "LCOV ingestion",
            rust_first: "yes",
            typescript_parity: "yes",
            state: "covered",
            notes: "Rust fixture validates commit-addressed ingestion; TypeScript full journey validates parity.",
        },
        JiraRow {
            jira: "CLI-1349",
            scope: "Coverage view",
            rust_first: "partial",
            typescript_parity: "yes",
            state: "covered",
            notes: "Rust stable tooling validates line coverage and commit scoping; explicit uncovered-branch assertions are currently TypeScript-led.",
        },
        JiraRow {
            jira: "CLI-1350",
            scope: "Run outcomes and scoring parity",
            rust_first: "deferred",
            typescript_parity: "deferred",
            state: "deferred",
            notes: "Explicitly deferred for now. TypeScript still has a spot-check for existing Jest result ingestion.",
        },
        JiraRow {
            jira: "CLI-1351",
            scope: "Validation matrix and reporting",
            rust_first: "yes",
            typescript_parity: "yes",
            state: "covered",
            notes: "This report plus the traceability matrix make the current covered/partial/deferred state explicit on every cargo test run.",
        },
        JiraRow {
            jira: "CLI-1352",
            scope: "Prototype decisions and threshold boundaries",
            rust_first: "yes",
            typescript_parity: "yes",
            state: "covered",
            notes: "Prototype defaults are documented and now backed by named constants and boundary tests.",
        },
    ];

    let scenario_rows = [
        ScenarioRow {
            scenario: "Scenario 1: Pre-change safety assessment",
            coverage: "covered",
            notes: "Summary view is exercised with coverage percentages on the real TypeScript fixture.",
        },
        ScenarioRow {
            scenario: "Scenario 2: Understanding what tests exist before making a change",
            coverage: "covered",
            notes: "Tests view, strength filtering, and explicit override are exercised on the real TypeScript fixture.",
        },
        ScenarioRow {
            scenario: "Scenario 3: Finding untested code paths before fixing a bug",
            coverage: "covered",
            notes: "Coverage view and uncovered branch reporting are exercised end to end.",
        },
        ScenarioRow {
            scenario: "Scenario 4: Identifying pre-existing test failures",
            coverage: "deferred",
            notes: "Cross-framework run-outcome parity is deferred with CLI-1350, although the TypeScript journey still spot-checks the existing Jest ingestion path.",
        },
        ScenarioRow {
            scenario: "Scenario 6: Assessing an unfamiliar artefact",
            coverage: "covered",
            notes: "The untested `hashPassword` artefact is now asserted through summary view on the real TypeScript fixture.",
        },
        ScenarioRow {
            scenario: "Scenario 7: PR review adequacy signal",
            coverage: "covered",
            notes: "Summary view exposes verification_level, counts, and coverage percentages needed for review-time triage.",
        },
        ScenarioRow {
            scenario: "Scenario 8: Red-phase TDD pattern discovery",
            coverage: "covered",
            notes: "Tests view exposes suite/test names and classifications for existing patterns.",
        },
        ScenarioRow {
            scenario: "Scenario 9: Cross-cutting artefact noise management",
            coverage: "covered",
            notes: "The default min_strength filter plus override are exercised on the real TypeScript fixture.",
        },
        ScenarioRow {
            scenario: "Scenario 10: Deciding whether to write tests",
            coverage: "covered",
            notes: "Verification level is asserted for both tested and untested artefacts.",
        },
    ];

    let covered_jira = jira_rows.iter().filter(|row| row.state == "covered").count();
    let deferred_jira = jira_rows.iter().filter(|row| row.state == "deferred").count();
    let covered_scenarios = scenario_rows
        .iter()
        .filter(|row| row.coverage == "covered")
        .count();
    let deferred_scenarios = scenario_rows
        .iter()
        .filter(|row| row.coverage == "deferred")
        .count();

    let mut report = String::from("# Test Harness Validation Status\n\n");
    report.push_str("Generated by `cargo test`.\n\n");
    report.push_str("## Summary\n\n");
    report.push_str(&format!(
        "- Jira slices covered now: {covered_jira}\n- Jira slices deferred: {deferred_jira}\n- Design-spec scenarios represented now: {covered_scenarios}\n- Design-spec scenarios deferred: {deferred_scenarios}\n\n"
    ));
    report.push_str("## Jira Slice Status\n\n");
    report.push_str("| Jira | Scope | Rust-first | TypeScript parity | State | Notes |\n");
    report.push_str("| --- | --- | --- | --- | --- | --- |\n");
    for row in jira_rows {
        report.push_str(&format!(
            "| `{}` | {} | {} | {} | {} | {} |\n",
            row.jira, row.scope, row.rust_first, row.typescript_parity, row.state, row.notes
        ));
    }

    report.push_str("\n## Design Spec Scenario Status\n\n");
    report.push_str("| Scenario | Status | Notes |\n");
    report.push_str("| --- | --- | --- |\n");
    for row in scenario_rows {
        report.push_str(&format!(
            "| {} | {} | {} |\n",
            row.scenario, row.coverage, row.notes
        ));
    }

    let report_dir = project_root().join("target/validation");
    fs::create_dir_all(&report_dir).expect("failed to create validation report directory");
    let report_path = report_dir.join("current_status.md");
    fs::write(&report_path, report).expect("failed to write validation status report");

    assert!(
        report_path.exists(),
        "expected validation report at {}",
        report_path.display()
    );
}
