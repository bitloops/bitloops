use crate::fixtures::{run_query_json, seeded_rust_graphql_workspace};
use std::env;
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};

const FIRST_QUERY_BUDGET_ENV: &str = "BITLOOPS_PERF_FIRST_QUERY_BUDGET_MS";
const WARM_QUERY_BUDGET_ENV: &str = "BITLOOPS_PERF_WARM_QUERY_BUDGET_MS";
const DEFAULT_FIRST_QUERY_BUDGET_MS: u64 = 1_000;
const DEFAULT_WARM_QUERY_BUDGET_MS: u64 = 750;

fn performance_suite_lock() -> MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|err| err.into_inner())
}

fn latency_budget_from_env(var: &str, default_ms: u64) -> Duration {
    let millis = env::var(var)
        .ok()
        .map(|value| {
            value
                .parse::<u64>()
                .unwrap_or_else(|_| panic!("{var} must be an integer number of milliseconds"))
        })
        .unwrap_or(default_ms);

    Duration::from_millis(millis)
}

fn sample_query() -> &'static str {
    r#"query ListCheckpoints {
  file(path: "./src/repositories/user_repository.rs") {
    language
  }
}"#
}

fn assert_latency_within_budget(label: &str, elapsed: Duration, latency_budget: Duration) {
    eprintln!("{label} latency: {elapsed:?} (budget {latency_budget:?})");
    assert!(
        elapsed <= latency_budget,
        "{label} daemon-backed `bitloops devql query` took {:?}, budget {:?}",
        elapsed,
        latency_budget
    );
}

#[test]
#[ignore = "performance smoke test; run locally with `cargo test --test performance -- --ignored`"]
fn bitloops_devql_first_query_stays_within_latency_budget_end_to_end() {
    let _guard = performance_suite_lock();
    let seeded = seeded_rust_graphql_workspace("perf-devql-first-query");
    let query = sample_query();
    let latency_budget =
        latency_budget_from_env(FIRST_QUERY_BUDGET_ENV, DEFAULT_FIRST_QUERY_BUDGET_MS);

    let started = Instant::now();
    let output = run_query_json(
        &seeded,
        &["devql", "query", "--compact", query],
    );
    let elapsed = started.elapsed();

    assert!(
        output["file"]["language"].as_str().is_some(),
        "expected seeded first-query latency check to resolve file language"
    );
    assert_latency_within_budget("first query", elapsed, latency_budget);
}

#[test]
#[ignore = "performance smoke test; run locally with `cargo test --test performance -- --ignored`"]
fn bitloops_devql_warm_query_stays_within_latency_budget_end_to_end() {
    let _guard = performance_suite_lock();
    let seeded = seeded_rust_graphql_workspace("perf-devql-warm-query");
    let query = sample_query();
    let latency_budget =
        latency_budget_from_env(WARM_QUERY_BUDGET_ENV, DEFAULT_WARM_QUERY_BUDGET_MS);

    let first_output = run_query_json(
        &seeded,
        &["devql", "query", "--compact", query],
    );
    assert!(
        first_output["file"]["language"].as_str().is_some(),
        "expected seeded warmup query to resolve file language"
    );

    let started = Instant::now();
    let warm_output = run_query_json(
        &seeded,
        &["devql", "query", "--compact", query],
    );
    let elapsed = started.elapsed();

    assert!(
        warm_output["file"]["language"].as_str().is_some(),
        "expected seeded warm-query latency check to resolve file language"
    );
    assert_latency_within_budget("warm query", elapsed, latency_budget);
}
