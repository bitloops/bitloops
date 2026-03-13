use std::path::{Path, PathBuf};

use crate::support::cli::{run_testlens_allow_failure, run_testlens_or_panic};
use crate::support::typescript_journey::{
    copy_real_typescript_fixture, index_real_typescript_fixture,
};
use cucumber::{World as _, given, then, when};
use serde_json::Value;
use tempfile::TempDir;

#[derive(Debug, Default, cucumber::World)]
struct TypeScriptJourneyWorld {
    temp_dir: Option<TempDir>,
    db_path: Option<PathBuf>,
    repo_dir: Option<PathBuf>,
    jest_json_path: Option<PathBuf>,
    commit_sha: String,
    last_failed_query_output: Option<String>,
}

#[given("a temporary sqlite database for the TypeScript full journey")]
fn given_temporary_sqlite_database(world: &mut TypeScriptJourneyWorld) {
    let temp_dir = TempDir::new().expect("failed to create temp dir for TS full journey");
    let db_path = temp_dir.path().join("typescript-full-journey.db");

    world.temp_dir = Some(temp_dir);
    world.db_path = Some(db_path);
    world.commit_sha = "fixture-typescript-full-journey".to_string();
}

#[given("a copied real TypeScript fixture repository for the full journey")]
fn given_copied_real_fixture_repository(world: &mut TypeScriptJourneyWorld) {
    let repo_dir = copy_real_typescript_fixture(world.temp_dir_path());
    let jest_json_path = world.temp_dir_path().join("test-results.json");

    world.repo_dir = Some(repo_dir);
    world.jest_json_path = Some(jest_json_path);
}

#[when("I run the full TypeScript ingestion journey")]
fn when_i_run_the_full_typescript_ingestion_journey(world: &mut TypeScriptJourneyWorld) {
    index_real_typescript_fixture(
        world.db_path(),
        world.repo_dir(),
        &world.commit_sha,
        world.jest_json_path(),
    );
}

#[then(expr = "querying {string} with summary view returns only summary data")]
fn then_summary_view_returns_only_summary_data(
    world: &mut TypeScriptJourneyWorld,
    artefact: String,
) {
    let json = query_json(
        world.db_path(),
        &world.commit_sha,
        &artefact,
        &["--view", "summary"],
    );
    assert!(json.get("artefact").is_some(), "expected artefact payload");
    assert!(json.get("summary").is_some(), "expected summary payload");
    assert!(
        json.get("covering_tests").is_none(),
        "did not expect covering_tests in summary view"
    );
    assert!(
        json.get("coverage").is_none(),
        "did not expect coverage in summary view"
    );
}

#[then(expr = "querying {string} with summary view includes coverage percentages")]
fn then_summary_view_includes_coverage_percentages(
    world: &mut TypeScriptJourneyWorld,
    artefact: String,
) {
    let json = query_json(
        world.db_path(),
        &world.commit_sha,
        &artefact,
        &["--view", "summary"],
    );
    assert!(json["summary"]["line_coverage_pct"].is_number());
    assert!(json["summary"]["branch_coverage_pct"].is_number());
}

#[then(expr = "querying {string} with tests view applies default strength filtering")]
fn then_tests_view_applies_default_strength_filtering(
    world: &mut TypeScriptJourneyWorld,
    artefact: String,
) {
    let default_json = query_json(
        world.db_path(),
        &world.commit_sha,
        &artefact,
        &["--view", "tests"],
    );
    let override_json = query_json(
        world.db_path(),
        &world.commit_sha,
        &artefact,
        &["--view", "tests", "--min-strength", "0.0"],
    );

    let default_count = covering_tests_len(&default_json);
    let override_count = covering_tests_len(&override_json);

    assert!(
        override_count > default_count,
        "expected default strength filter to hide weaker tests (default: {}, override: {})",
        default_count,
        override_count
    );
    assert_eq!(
        default_json["summary"]["total_covering_tests"].as_u64(),
        Some(override_count as u64),
        "expected summary total to keep the full unfiltered match count"
    );
}

#[then(expr = "querying {string} with tests view and min_strength 0.0 returns more tests")]
fn then_tests_view_override_returns_more_tests(
    world: &mut TypeScriptJourneyWorld,
    artefact: String,
) {
    let default_json = query_json(
        world.db_path(),
        &world.commit_sha,
        &artefact,
        &["--view", "tests"],
    );
    let override_json = query_json(
        world.db_path(),
        &world.commit_sha,
        &artefact,
        &["--view", "tests", "--min-strength", "0.0"],
    );

    assert!(
        covering_tests_len(&override_json) > covering_tests_len(&default_json),
        "expected override to return more tests"
    );
}

#[then(expr = "querying {string} with coverage view returns branch coverage")]
fn then_coverage_view_returns_branch_coverage(
    world: &mut TypeScriptJourneyWorld,
    artefact: String,
) {
    let json = query_json(
        world.db_path(),
        &world.commit_sha,
        &artefact,
        &["--view", "coverage"],
    );

    assert!(json.get("artefact").is_some(), "expected artefact payload");
    assert!(json.get("coverage").is_some(), "expected coverage payload");
    assert!(
        json.get("summary").is_none(),
        "did not expect summary payload in coverage view"
    );
    let branches = json["coverage"]["branches"]
        .as_array()
        .expect("expected branches array");
    assert!(!branches.is_empty(), "expected branch entries");
}

#[then(expr = "querying {string} with summary view reports the artefact as untested")]
fn then_summary_view_reports_artefact_as_untested(
    world: &mut TypeScriptJourneyWorld,
    artefact: String,
) {
    let json = query_json(
        world.db_path(),
        &world.commit_sha,
        &artefact,
        &["--view", "summary"],
    );
    assert_eq!(
        json["summary"]["verification_level"].as_str(),
        Some("untested")
    );
    assert_eq!(json["summary"]["total_covering_tests"].as_u64(), Some(0));
}

#[then(expr = "querying {string} with tests view surfaces a failing last run")]
fn then_tests_view_surfaces_a_failing_last_run(
    world: &mut TypeScriptJourneyWorld,
    artefact: String,
) {
    let json = query_json(
        world.db_path(),
        &world.commit_sha,
        &artefact,
        &["--view", "tests", "--min-strength", "0.0"],
    );
    let covering_tests = json["covering_tests"]
        .as_array()
        .expect("expected covering_tests array");

    let has_failing_run = covering_tests.iter().any(|row| {
        row["last_run"]["status"]
            .as_str()
            .is_some_and(|status| status == "fail")
    });
    assert!(
        has_failing_run,
        "expected at least one failing last_run for {}",
        artefact
    );
}

#[when(expr = "I query {string} before indexing the TypeScript journey")]
fn when_i_query_before_indexing(
    world: &mut TypeScriptJourneyWorld,
    artefact: String,
) {
    initialize_db(world.db_path());
    world.last_failed_query_output = Some(query_failure(
        world.db_path(),
        &world.commit_sha,
        &artefact,
        &["--view", "summary"],
    ));
}

#[when(expr = "I query {string} after indexing the TypeScript journey")]
fn when_i_query_after_indexing(
    world: &mut TypeScriptJourneyWorld,
    artefact: String,
) {
    world.last_failed_query_output = Some(query_failure(
        world.db_path(),
        &world.commit_sha,
        &artefact,
        &["--view", "summary"],
    ));
}

#[then(expr = "the query fails with {string}")]
fn then_query_fails_with(world: &mut TypeScriptJourneyWorld, message: String) {
    let output = world
        .last_failed_query_output
        .as_ref()
        .expect("expected failed query output");
    assert!(
        output.contains(&message),
        "expected query failure output to contain {:?}, got:\n{}",
        message,
        output
    );
}

#[tokio::test]
async fn typescript_full_journey_e2e_gherkin() {
    TypeScriptJourneyWorld::run("features/typescript_full_journey_e2e.feature").await;
}

fn query_json(db_path: &Path, commit_sha: &str, artefact: &str, extra_args: &[&str]) -> Value {
    let db = db_path.to_string_lossy().to_string();

    let mut args = vec![
        "query".to_string(),
        "--db".to_string(),
        db,
        "--artefact".to_string(),
        artefact.to_string(),
        "--commit".to_string(),
        commit_sha.to_string(),
    ];
    args.extend(extra_args.iter().map(|item| item.to_string()));
    let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();

    let output = run_testlens_or_panic(&arg_refs);
    serde_json::from_str(&output).expect("failed to parse query JSON")
}

fn covering_tests_len(json: &Value) -> usize {
    json["covering_tests"]
        .as_array()
        .expect("expected covering_tests array")
        .len()
}

fn initialize_db(db_path: &Path) {
    let db = db_path.to_string_lossy().to_string();
    run_testlens_or_panic(&["init", "--db", &db]);
}

fn query_failure(db_path: &Path, commit_sha: &str, artefact: &str, extra_args: &[&str]) -> String {
    let db = db_path.to_string_lossy().to_string();

    let mut args = vec![
        "query".to_string(),
        "--db".to_string(),
        db,
        "--artefact".to_string(),
        artefact.to_string(),
        "--commit".to_string(),
        commit_sha.to_string(),
    ];
    args.extend(extra_args.iter().map(|item| item.to_string()));
    let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();

    let output = run_testlens_allow_failure(&arg_refs);
    assert!(
        !output.status.success(),
        "expected query to fail, stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

impl TypeScriptJourneyWorld {
    fn temp_dir_path(&self) -> &Path {
        self.temp_dir
            .as_ref()
            .expect("temp dir should be initialized")
            .path()
    }

    fn db_path(&self) -> &Path {
        self.db_path
            .as_deref()
            .expect("db path should be initialized")
    }

    fn repo_dir(&self) -> &Path {
        self.repo_dir
            .as_deref()
            .expect("repo dir should be initialized")
    }

    fn jest_json_path(&self) -> &Path {
        self.jest_json_path
            .as_deref()
            .expect("jest json path should be initialized")
    }
}
