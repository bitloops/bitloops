use super::*;
use crate::capability_packs::semantic_clones::features::NoopSemanticSummaryProvider;
use crate::cli::inference::{managed_inference_binary_path, platform_summary_gateway_url_override};
use crate::config::resolve_store_backend_config_for_repo;
use crate::host::devql::{
    DevqlConfig, RelationalStorage, execute_ingest_with_observer, execute_init_schema,
    execute_sync, resolve_repo_identity,
};
use crate::host::runtime_store::{
    DaemonSqliteRuntimeStore, RepoSqliteRuntimeStore, SemanticEmbeddingMailboxItemRecord,
    SemanticMailboxItemKind, SemanticMailboxItemStatus, SemanticSummaryMailboxItemRecord,
    WorkplaneJobRecord, WorkplaneJobStatus,
};
use crate::test_support::git_fixtures::{git_ok, init_test_repo};
use crate::test_support::log_capture::capture_logs;
use serde_json::json;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, OnceLock};
use std::time::{Duration as StdDuration, Instant};
use tempfile::TempDir;
use tokio::sync::{Mutex, Notify};
use tokio::task::JoinSet;
use tokio::time::{Duration, advance};

const SUMMARY_REFRESH_200_JOBS_BUDGET_ENV: &str =
    "BITLOOPS_PERF_SUMMARY_REFRESH_200_JOBS_BUDGET_MS";
const SUMMARY_REFRESH_200_JOBS_WORKERS_ENV: &str = "BITLOOPS_PERF_SUMMARY_REFRESH_200_JOBS_WORKERS";
const DEFAULT_SUMMARY_REFRESH_200_JOBS_BUDGET_MS: u64 = 60_000;
const DEFAULT_SUMMARY_REFRESH_PERF_SLOWEST_JOB_COUNT: usize = 5;
const MAX_SUMMARY_REFRESH_PERF_WORKERS: usize = 32;
const SUMMARY_REFRESH_PERF_JOB_COUNT: usize = 200;
const SUMMARY_REFRESH_PERF_RUST_FIXTURE_HEADER: &str = r#"use std::path::{Path, PathBuf};

use crate::config::{
    BITLOOPS_CONFIG_RELATIVE_PATH, resolve_repo_runtime_db_path_for_repo,
    resolve_store_backend_config_for_repo,
};
use crate::test_support::process_state::git_command;
"#;

fn sample_input() -> semantic_features::SemanticFeatureInput {
    semantic_features::SemanticFeatureInput {
        artefact_id: "artefact-1".to_string(),
        symbol_id: Some("symbol-1".to_string()),
        repo_id: "repo-1".to_string(),
        blob_sha: "blob-1".to_string(),
        path: "src/service.rs".to_string(),
        language: "rust".to_string(),
        canonical_kind: "function".to_string(),
        language_kind: "function".to_string(),
        symbol_fqn: "src/service.rs::load_user".to_string(),
        name: "load_user".to_string(),
        signature: Some("fn load_user(id: &str)".to_string()),
        modifiers: vec!["pub".to_string()],
        body: "load_user_impl(id)".to_string(),
        docstring: Some("Loads a user.".to_string()),
        parent_kind: None,
        dependency_signals: vec!["calls:user_store::load".to_string()],
        content_hash: Some("content-hash".to_string()),
    }
}

fn sample_input_with_artefact_id(artefact_id: &str) -> semantic_features::SemanticFeatureInput {
    let mut input = sample_input();
    input.artefact_id = artefact_id.to_string();
    input.symbol_id = Some(format!("symbol-{artefact_id}"));
    input.symbol_fqn = format!("src/service.rs::{artefact_id}");
    input.name = artefact_id.to_string();
    input
}

async fn performance_suite_lock() -> tokio::sync::MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(())).lock().await
}

fn latency_budget_from_env(var: &str, default_ms: u64) -> StdDuration {
    let millis = env::var(var)
        .ok()
        .map(|value| {
            value
                .parse::<u64>()
                .unwrap_or_else(|_| panic!("{var} must be an integer number of milliseconds"))
        })
        .unwrap_or(default_ms);

    StdDuration::from_millis(millis)
}

fn assert_latency_within_budget_with_context(
    label: &str,
    elapsed: StdDuration,
    latency_budget: StdDuration,
    context: &str,
) {
    eprintln!(
        "{label} latency: {elapsed:?} ({:.2} ms/job, budget {latency_budget:?})",
        elapsed.as_secs_f64() * 1_000.0 / SUMMARY_REFRESH_PERF_JOB_COUNT as f64
    );
    assert!(
        elapsed <= latency_budget,
        "{label} took {:?}, budget {:?}\n{}",
        elapsed,
        latency_budget,
        context,
    );
}

#[derive(Debug, Clone)]
struct SummaryRefreshPerfJobMetric {
    job_id: String,
    artefact_id: String,
    symbol_fqn: String,
    queue_wait: StdDuration,
    run: StdDuration,
    end_to_end: StdDuration,
}

#[derive(Debug, Clone, Copy)]
struct DurationDistributionSummary {
    mean_ms: f64,
    p50: StdDuration,
    p95: StdDuration,
    p99: StdDuration,
    max: StdDuration,
}

fn summary_refresh_perf_worker_count(target: &EnrichmentJobTarget) -> usize {
    env::var(SUMMARY_REFRESH_200_JOBS_WORKERS_ENV)
        .ok()
        .and_then(|value| {
            let trimmed = value.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.parse::<usize>().unwrap_or_else(|_| {
                    panic!("{SUMMARY_REFRESH_200_JOBS_WORKERS_ENV} must be an integer worker count")
                }))
            }
        })
        .unwrap_or_else(|| {
            super::worker_count::configured_enrichment_worker_budgets_for_repo(&target.repo_root)
                .summary_refresh
        })
        .clamp(1, MAX_SUMMARY_REFRESH_PERF_WORKERS)
}

fn summary_refresh_perf_symbol_lookup(
    inputs: &[semantic_features::SemanticFeatureInput],
) -> BTreeMap<String, String> {
    inputs
        .iter()
        .map(|input| (input.artefact_id.clone(), input.symbol_fqn.clone()))
        .collect()
}

fn summary_refresh_perf_job_artefact_id(job: &WorkplaneJobRecord) -> String {
    match serde_json::from_value::<
        crate::capability_packs::semantic_clones::workplane::SemanticClonesMailboxPayload,
    >(job.payload.clone())
    {
        Ok(crate::capability_packs::semantic_clones::workplane::SemanticClonesMailboxPayload::Artefact {
            artefact_id,
        }) => artefact_id,
        _ => job.job_id.clone(),
    }
}

fn format_perf_duration(duration: StdDuration) -> String {
    format_perf_millis(duration.as_secs_f64() * 1_000.0)
}

fn format_perf_millis(millis: f64) -> String {
    if millis >= 1_000.0 {
        format!("{:.2}s", millis / 1_000.0)
    } else {
        format!("{millis:.2}ms")
    }
}

fn duration_distribution_summary(samples: &[StdDuration]) -> DurationDistributionSummary {
    assert!(
        !samples.is_empty(),
        "duration distribution summary requires at least one sample"
    );
    let mut sorted = samples.to_vec();
    sorted.sort_unstable();
    DurationDistributionSummary {
        mean_ms: sorted
            .iter()
            .map(|value| value.as_secs_f64() * 1_000.0)
            .sum::<f64>()
            / sorted.len() as f64,
        p50: percentile_duration_from_sorted(&sorted, 50, 100),
        p95: percentile_duration_from_sorted(&sorted, 95, 100),
        p99: percentile_duration_from_sorted(&sorted, 99, 100),
        max: *sorted.last().expect("non-empty sorted durations"),
    }
}

fn percentile_duration_from_sorted(
    sorted: &[StdDuration],
    percentile_numerator: usize,
    percentile_denominator: usize,
) -> StdDuration {
    assert!(
        !sorted.is_empty(),
        "percentile duration requires at least one sample"
    );
    let rank = (sorted.len() * percentile_numerator).div_ceil(percentile_denominator);
    let index = rank.saturating_sub(1).min(sorted.len().saturating_sub(1));
    sorted[index]
}

fn render_summary_refresh_perf_report(
    metrics: &[SummaryRefreshPerfJobMetric],
    worker_count: usize,
    elapsed: StdDuration,
    latency_budget: StdDuration,
) -> String {
    let queue_waits = metrics
        .iter()
        .map(|metric| metric.queue_wait)
        .collect::<Vec<_>>();
    let runs = metrics.iter().map(|metric| metric.run).collect::<Vec<_>>();
    let end_to_end = metrics
        .iter()
        .map(|metric| metric.end_to_end)
        .collect::<Vec<_>>();
    let queue_wait_summary = duration_distribution_summary(&queue_waits);
    let run_summary = duration_distribution_summary(&runs);
    let end_to_end_summary = duration_distribution_summary(&end_to_end);
    let throughput = metrics.len() as f64 / elapsed.as_secs_f64().max(f64::EPSILON);
    let worker_utilisation = 100.0 * runs.iter().map(StdDuration::as_secs_f64).sum::<f64>()
        / (elapsed.as_secs_f64().max(f64::EPSILON) * worker_count as f64);
    let mut lines = vec![format!(
        "summary refresh perf report: jobs={} workers={} total={} throughput={:.2} jobs/s worker_utilisation={:.1}% budget={}",
        metrics.len(),
        worker_count,
        format_perf_duration(elapsed),
        throughput,
        worker_utilisation,
        format_perf_duration(latency_budget),
    )];

    for (label, summary) in [
        ("queue_wait", queue_wait_summary),
        ("run", run_summary),
        ("end_to_end", end_to_end_summary),
    ] {
        lines.push(format!(
            "  {label}: mean={} p50={} p95={} p99={} max={}",
            format_perf_millis(summary.mean_ms),
            format_perf_duration(summary.p50),
            format_perf_duration(summary.p95),
            format_perf_duration(summary.p99),
            format_perf_duration(summary.max),
        ));
    }

    let mut slowest = metrics.to_vec();
    slowest.sort_by(|left, right| {
        right
            .run
            .cmp(&left.run)
            .then_with(|| right.end_to_end.cmp(&left.end_to_end))
    });
    lines.push("  slowest_jobs_by_run:".to_string());
    for metric in slowest
        .iter()
        .take(DEFAULT_SUMMARY_REFRESH_PERF_SLOWEST_JOB_COUNT)
    {
        lines.push(format!(
            "    run={} queue_wait={} end_to_end={} artefact={} symbol={} job_id={}",
            format_perf_duration(metric.run),
            format_perf_duration(metric.queue_wait),
            format_perf_duration(metric.end_to_end),
            metric.artefact_id,
            metric.symbol_fqn,
            metric.job_id,
        ));
    }
    lines.join("\n")
}

fn print_summary_refresh_perf_report(
    metrics: &[SummaryRefreshPerfJobMetric],
    worker_count: usize,
    elapsed: StdDuration,
    latency_budget: StdDuration,
) -> String {
    let report = render_summary_refresh_perf_report(metrics, worker_count, elapsed, latency_budget);
    eprintln!("{report}");
    report
}

async fn run_summary_refresh_perf_jobs(
    coordinator: &EnrichmentCoordinator,
    worker_count: usize,
    symbol_lookup: Arc<BTreeMap<String, String>>,
) -> std::result::Result<(Vec<SummaryRefreshPerfJobMetric>, StdDuration), String> {
    let started = Arc::new(Instant::now());
    let mut workers = JoinSet::new();
    for _ in 0..worker_count {
        let started = Arc::clone(&started);
        let symbol_lookup = Arc::clone(&symbol_lookup);
        let workplane_store = coordinator.workplane_store.clone();
        let runtime_store = coordinator.runtime_store.clone();
        workers.spawn(async move {
            let control_state = default_state();
            let mut metrics = Vec::new();
            while let Some(job) = claim_next_workplane_job(
                    &workplane_store,
                    &runtime_store,
                    &control_state,
                    super::worker_count::EnrichmentWorkerPool::SummaryRefresh,
                )
                .map_err(|err| format!("claim summary perf job: {err:#}"))?
            {

                let queue_wait = started.elapsed();
                let artefact_id = summary_refresh_perf_job_artefact_id(&job);
                let symbol_fqn = symbol_lookup
                    .get(&artefact_id)
                    .cloned()
                    .unwrap_or_else(|| artefact_id.clone());
                let run_started = Instant::now();
                let outcome = super::execution::execute_workplane_job(&job).await;
                if let Some(error) = outcome.error.as_ref() {
                    return Err(format!(
                        "summary perf job `{}` failed: {error}",
                        job.job_id
                    ));
                }
                if !outcome.follow_ups.is_empty() {
                    return Err(format!(
                        "summary refresh perf should not schedule follow-up jobs when summary embeddings are disabled: {} follow-ups for `{}`",
                        outcome.follow_ups.len(),
                        job.job_id
                    ));
                }

                let disposition =
                    persist_workplane_job_completion(&workplane_store, &job, &outcome)
                        .map_err(|err| {
                            format!(
                                "persist summary perf completion for `{}`: {err:#}",
                                job.job_id
                            )
                        })?;
                if disposition != WorkplaneJobCompletionDisposition::Completed {
                    return Err(format!(
                        "summary perf job `{}` ended with unexpected disposition {disposition:?}",
                        job.job_id
                    ));
                }

                metrics.push(SummaryRefreshPerfJobMetric {
                    job_id: job.job_id.clone(),
                    artefact_id,
                    symbol_fqn,
                    queue_wait,
                    run: run_started.elapsed(),
                    end_to_end: started.elapsed(),
                });
            }
            Ok::<Vec<SummaryRefreshPerfJobMetric>, String>(metrics)
        });
    }

    let mut all_metrics = Vec::new();
    while let Some(joined) = workers.join_next().await {
        let worker_metrics =
            joined.map_err(|err| format!("joining summary perf worker task: {err}"))??;
        all_metrics.extend(worker_metrics);
    }
    Ok((all_metrics, started.elapsed()))
}

fn source_slice_between(source: &str, start_marker: &str, end_marker: Option<&str>) -> String {
    let start = source
        .find(start_marker)
        .unwrap_or_else(|| panic!("expected start marker `{start_marker}` in fixture source"));
    let end = match end_marker {
        Some(marker) => {
            let search_start = start + start_marker.len();
            let end_offset = source[search_start..]
                .find(marker)
                .unwrap_or_else(|| panic!("expected end marker `{marker}` in fixture source"));
            search_start + end_offset
        }
        None => source.len(),
    };

    source[start..end].trim().to_string()
}

fn summary_refresh_perf_rust_templates() -> Vec<String> {
    let git_fixtures = include_str!("../test_support/git_fixtures.rs");
    let config_resolve = include_str!("../config/resolve.rs");

    vec![
        format!(
            "{SUMMARY_REFRESH_PERF_RUST_FIXTURE_HEADER}\n{}\n",
            source_slice_between(
                git_fixtures,
                "pub(crate) fn git_ok(",
                Some("pub(crate) fn init_test_repo("),
            )
        ),
        format!(
            "{SUMMARY_REFRESH_PERF_RUST_FIXTURE_HEADER}\n{}\n",
            source_slice_between(
                git_fixtures,
                "pub(crate) fn repo_local_blob_root(",
                Some("pub(crate) fn write_test_daemon_config("),
            )
        ),
        format!(
            "{SUMMARY_REFRESH_PERF_RUST_FIXTURE_HEADER}\n{}\n",
            source_slice_between(
                git_fixtures,
                "pub(crate) fn write_test_daemon_config(",
                Some("#[allow(dead_code)]"),
            )
        ),
        format!(
            "{SUMMARY_REFRESH_PERF_RUST_FIXTURE_HEADER}\n{}\n",
            source_slice_between(
                git_fixtures,
                "#[allow(dead_code)]\npub(crate) fn ensure_test_store_backends(",
                None,
            )
        ),
        format!(
            "{SUMMARY_REFRESH_PERF_RUST_FIXTURE_HEADER}\n{}\n",
            source_slice_between(
                config_resolve,
                "pub fn resolve_store_backend_config_for_repo(",
                Some("pub fn resolve_repo_runtime_db_path_for_repo("),
            )
        ),
        format!(
            "{SUMMARY_REFRESH_PERF_RUST_FIXTURE_HEADER}\n{}\n",
            source_slice_between(
                config_resolve,
                "pub fn resolve_repo_runtime_db_path_for_config_root(",
                Some("pub fn resolve_provider_config("),
            )
        ),
    ]
}

fn seed_summary_refresh_perf_repo(repo_root: &Path, file_count: usize) {
    let src_root = repo_root.join("src");
    fs::create_dir_all(&src_root).expect("create summary perf src dir");
    fs::write(
        repo_root.join("Cargo.toml"),
        r#"[package]
name = "summary-refresh-perf"
version = "0.1.0"
edition = "2021"

[lib]
path = "src/lib.rs"
"#,
    )
    .expect("write summary perf Cargo.toml");
    fs::write(
        src_root.join("lib.rs"),
        "//! Summary refresh performance fixtures.\n",
    )
    .expect("write summary perf lib.rs");

    let templates = summary_refresh_perf_rust_templates();
    for index in 0..file_count {
        let template = &templates[index % templates.len()];
        fs::write(
            src_root.join(format!("perf_summary_{index:03}.rs")),
            template,
        )
        .expect("write summary perf rust fixture");
    }

    git_ok(repo_root, &["add", "."]);
    git_ok(
        repo_root,
        &["commit", "-m", "Seed summary refresh performance fixtures"],
    );
}

fn summary_refresh_perf_cfg(target: &EnrichmentJobTarget) -> DevqlConfig {
    let repo =
        resolve_repo_identity(&target.repo_root).expect("resolve summary perf repo identity");
    DevqlConfig::from_roots(target.config_root.clone(), target.repo_root.clone(), repo)
        .expect("build summary perf daemon config")
}

fn resolve_summary_refresh_perf_runtime_command() -> Option<String> {
    let sibling_workspace_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .map(Path::to_path_buf);
    let sibling_repo_binary = sibling_workspace_root
        .as_ref()
        .map(|root| {
            root.join("bitloops-inference")
                .join("target")
                .join("debug")
                .join(if cfg!(windows) {
                    "bitloops-inference.exe"
                } else {
                    "bitloops-inference"
                })
        })
        .filter(|path| path.is_file());
    if let Some(path) = sibling_repo_binary {
        return Some(path.display().to_string());
    }

    if let Ok(path) = managed_inference_binary_path()
        && path.is_file()
    {
        return Some(path.display().to_string());
    }

    let command = if cfg!(windows) {
        "bitloops-inference.exe"
    } else {
        "bitloops-inference"
    };
    match std::process::Command::new(command)
        .arg("--version")
        .output()
    {
        Ok(output) if output.status.success() => Some(command.to_string()),
        _ => None,
    }
}

fn configure_platform_summary_refresh_for_repo(
    target: &EnrichmentJobTarget,
    runtime_command: &str,
) {
    let config_path =
        crate::test_support::git_fixtures::write_test_daemon_config(&target.config_root);
    crate::config::settings::write_repo_daemon_binding(
        &target
            .repo_root
            .join(crate::config::REPO_POLICY_LOCAL_FILE_NAME),
        &config_path,
    )
    .expect("bind repo root to daemon config");

    let mut config = fs::read_to_string(&config_path).expect("read platform summary perf config");
    config.push_str(&format!(
        r#"
[semantic_clones.inference]
summary_generation = "summary_llm"

[inference.runtimes.bitloops_inference]
command = {runtime_command:?}
args = []
startup_timeout_secs = 60
request_timeout_secs = 300

[inference.profiles.summary_llm]
task = "text_generation"
runtime = "bitloops_inference"
driver = "bitloops_platform_chat"
model = "ministral-3-3b-instruct"
api_key = "${{BITLOOPS_PLATFORM_GATEWAY_TOKEN}}"
temperature = "0.1"
max_output_tokens = 200
"#,
    ));

    if let Some(base_url) = platform_summary_gateway_url_override() {
        config.push_str(&format!("base_url = {base_url:?}\n"));
    }

    fs::write(&config_path, config).expect("write platform summary perf config");
}

fn summary_refresh_perf_platform_prerequisites() -> Result<String, String> {
    if std::env::var(crate::daemon::PLATFORM_GATEWAY_TOKEN_ENV)
        .ok()
        .as_deref()
        .map(str::trim)
        .is_some_and(|token| !token.is_empty())
    {
        return resolve_summary_refresh_perf_runtime_command().ok_or_else(|| {
            "install `bitloops-inference` or make it available on PATH before running this perf test"
                .to_string()
        });
    }

    if crate::daemon::platform_gateway_bearer_token()
        .expect("read platform gateway auth state")
        .is_none()
    {
        return Err(format!(
            "run `bitloops login` first or export `{}` so the runtime can inject a platform JWT",
            crate::daemon::PLATFORM_GATEWAY_TOKEN_ENV
        ));
    }

    resolve_summary_refresh_perf_runtime_command().ok_or_else(|| {
        "install `bitloops-inference` or make it available on PATH before running this perf test"
            .to_string()
    })
}

async fn load_summary_refresh_perf_inputs(
    target: &EnrichmentJobTarget,
) -> Vec<semantic_features::SemanticFeatureInput> {
    let cfg = summary_refresh_perf_cfg(target);
    execute_init_schema(&cfg, "summary refresh performance test")
        .await
        .expect("initialise summary perf devql schema");
    let backends = resolve_store_backend_config_for_repo(&cfg.daemon_config_root)
        .expect("resolve summary perf store backends");
    let relational = RelationalStorage::connect(&cfg, &backends.relational, "summary perf")
        .await
        .expect("connect summary perf relational storage");
    execute_ingest_with_observer(&cfg, false, 10, None, None)
        .await
        .expect("ingest summary perf fixtures");
    execute_sync(
        &cfg,
        &relational,
        crate::host::devql::sync::types::SyncMode::Full,
    )
    .await
    .expect("sync summary perf fixtures");

    let mut inputs =
        crate::capability_packs::semantic_clones::load_semantic_feature_inputs_for_current_repo(
            &relational,
            &target.repo_root,
            &cfg.repo.repo_id,
        )
        .await
        .expect("load summary perf semantic inputs");
    inputs.retain(|input| input.language == "rust" && input.canonical_kind == "function");
    inputs.sort_by(|left, right| {
        left.path
            .cmp(&right.path)
            .then_with(|| left.symbol_fqn.cmp(&right.symbol_fqn))
    });
    inputs
}

fn summary_refresh_perf_input_hashes(
    inputs: &[semantic_features::SemanticFeatureInput],
) -> BTreeMap<String, String> {
    inputs
        .iter()
        .map(|input| {
            (
                input.artefact_id.clone(),
                semantic_features::build_semantic_feature_input_hash(
                    input,
                    &NoopSemanticSummaryProvider,
                ),
            )
        })
        .collect()
}

#[test]
fn enrichment_job_kind_serializes_lightweight_artefact_ids() {
    let job = EnrichmentJobKind::SemanticSummaries {
        artefact_ids: vec!["artefact-1".to_string()],
        input_hashes: BTreeMap::from([("artefact-1".to_string(), "hash-1".to_string())]),
        batch_key: "artefact-1".to_string(),
    };

    let value = serde_json::to_value(job).expect("serialize job kind");
    assert_eq!(
        value.get("kind").and_then(|value| value.as_str()),
        Some("semantic_summaries")
    );
    assert_eq!(
        value
            .get("artefact_ids")
            .and_then(|value| value.as_array())
            .map(|values| values.len()),
        Some(1)
    );
    assert!(value.get("inputs").is_none());
}

#[test]
fn enrichment_job_kind_deserializes_legacy_inputs_into_artefact_ids() {
    let input = sample_input();
    let job = serde_json::from_value::<EnrichmentJobKind>(json!({
        "kind": "semantic_summaries",
        "inputs": [input],
        "input_hashes": { "artefact-1": "hash-1" },
        "batch_key": "artefact-1",
        "embedding_mode": "semantic_aware_once"
    }))
    .expect("deserialize legacy job kind");

    match job {
        EnrichmentJobKind::SemanticSummaries { artefact_ids, .. } => {
            assert_eq!(artefact_ids, vec!["artefact-1".to_string()]);
        }
        other => panic!("expected semantic summaries job, got {other:?}"),
    }
}

#[test]
fn load_workplane_jobs_prioritises_embedding_mailboxes_before_summary_refresh() {
    let temp = TempDir::new().expect("temp dir");
    let (coordinator, target, repo_id) = new_test_coordinator(&temp);

    insert_workplane_job(
        &coordinator,
        &target,
        WorkplaneJobFixture {
            repo_id: &repo_id,
            mailbox_name: SEMANTIC_CLONES_SUMMARY_REFRESH_MAILBOX,
            status: WorkplaneJobStatus::Pending,
            artefact_id: Some("summary-a"),
            job_id: "summary-a",
            updated_at_unix: 1,
            attempts: 0,
            last_error: None,
        },
    );
    insert_workplane_job(
        &coordinator,
        &target,
        WorkplaneJobFixture {
            repo_id: &repo_id,
            mailbox_name: SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX,
            status: WorkplaneJobStatus::Pending,
            artefact_id: Some("code-a"),
            job_id: "code-a",
            updated_at_unix: 2,
            attempts: 0,
            last_error: None,
        },
    );
    insert_workplane_job(
        &coordinator,
        &target,
        WorkplaneJobFixture {
            repo_id: &repo_id,
            mailbox_name: SEMANTIC_CLONES_SUMMARY_EMBEDDING_MAILBOX,
            status: WorkplaneJobStatus::Pending,
            artefact_id: Some("summary-embed-a"),
            job_id: "summary-embed-a",
            updated_at_unix: 3,
            attempts: 0,
            last_error: None,
        },
    );
    insert_workplane_job(
        &coordinator,
        &target,
        WorkplaneJobFixture {
            repo_id: &repo_id,
            mailbox_name: SEMANTIC_CLONES_CLONE_REBUILD_MAILBOX,
            status: WorkplaneJobStatus::Pending,
            artefact_id: None,
            job_id: "clone-a",
            updated_at_unix: 4,
            attempts: 0,
            last_error: None,
        },
    );

    let pending_jobs = load_workplane_jobs(&coordinator, WorkplaneJobStatus::Pending);
    let mailboxes = pending_jobs
        .iter()
        .map(|job| job.mailbox_name.as_str())
        .collect::<Vec<_>>();

    assert_eq!(
        mailboxes,
        vec![
            SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX,
            SEMANTIC_CLONES_SUMMARY_EMBEDDING_MAILBOX,
            SEMANTIC_CLONES_SUMMARY_REFRESH_MAILBOX,
            SEMANTIC_CLONES_CLONE_REBUILD_MAILBOX,
        ]
    );
}

fn sample_target(config_root: PathBuf, repo_root: PathBuf) -> EnrichmentJobTarget {
    EnrichmentJobTarget::new(config_root, repo_root)
}

fn new_test_coordinator(temp: &TempDir) -> (EnrichmentCoordinator, EnrichmentJobTarget, String) {
    let config_root = temp.path().join("config");
    let repo_root = temp.path().join("repo");
    fs::create_dir_all(&config_root).expect("create test config root");
    fs::create_dir_all(&repo_root).expect("create test repo root");
    init_test_repo(&repo_root, "main", "Bitloops Test", "bitloops@example.com");
    let repo_store = RepoSqliteRuntimeStore::open_for_roots(&config_root, &repo_root)
        .expect("open repo workplane store");
    let runtime_db_path = repo_store.db_path().to_path_buf();
    let repo_id = repo_store.repo_id().to_string();
    (
        EnrichmentCoordinator {
            runtime_store: DaemonSqliteRuntimeStore::open_at(runtime_db_path.clone())
                .expect("open test daemon runtime store"),
            workplane_store: DaemonSqliteRuntimeStore::open_at(runtime_db_path)
                .expect("open test workplane store"),
            daemon_config_root: config_root.clone(),
            subscription_hub: std::sync::Mutex::new(None),
            lock: Mutex::new(()),
            notify: Notify::new(),
            state_initialised: AtomicBool::new(false),
            maintenance_started: AtomicBool::new(false),
            started_worker_counts: std::sync::Mutex::new(
                super::worker_count::EnrichmentWorkerBudgets::default(),
            ),
        },
        sample_target(config_root, repo_root),
        repo_id,
    )
}

fn configure_summary_refresh_for_repo(target: &EnrichmentJobTarget) {
    let config_path =
        crate::test_support::git_fixtures::write_test_daemon_config(&target.config_root);
    crate::config::settings::write_repo_daemon_binding(
        &target
            .repo_root
            .join(crate::config::REPO_POLICY_LOCAL_FILE_NAME),
        &config_path,
    )
    .expect("bind repo root to daemon config");

    #[cfg(unix)]
    let (command, args) = fake_text_generation_runtime_command_and_args(&target.repo_root);
    #[cfg(windows)]
    let (command, args) = fake_text_generation_runtime_command_and_args(&target.repo_root);
    let runtime_args = args
        .iter()
        .map(|arg| format!("{arg:?}"))
        .collect::<Vec<_>>()
        .join(", ");
    let mut config = fs::read_to_string(&config_path).expect("read test daemon config");
    config.push_str(&format!(
        r#"
[semantic_clones.inference]
summary_generation = "summary_local"

[inference.runtimes.bitloops_inference]
command = {command:?}
args = [{runtime_args}]
startup_timeout_secs = 1
request_timeout_secs = 1

[inference.profiles.summary_local]
task = "text_generation"
driver = "ollama_chat"
runtime = "bitloops_inference"
model = "ministral-3:3b"
base_url = "http://127.0.0.1:11434/api/chat"
temperature = "0.1"
max_output_tokens = 200
"#,
    ));
    fs::write(&config_path, config).expect("write test daemon config with summary profile");
}

fn configure_embeddings_for_repo(target: &EnrichmentJobTarget, profile_name: &str) -> PathBuf {
    let config_path =
        crate::test_support::git_fixtures::write_test_daemon_config(&target.config_root);
    crate::config::settings::write_repo_daemon_binding(
        &target
            .repo_root
            .join(crate::config::REPO_POLICY_LOCAL_FILE_NAME),
        &config_path,
    )
    .expect("bind repo root to daemon config");

    #[cfg(unix)]
    let (command, args) = fake_embeddings_runtime_command_and_args(&target.repo_root);
    #[cfg(windows)]
    let (command, args) = fake_embeddings_runtime_command_and_args(&target.repo_root);
    let runtime_args = args
        .iter()
        .map(|arg| format!("{arg:?}"))
        .collect::<Vec<_>>()
        .join(", ");
    let mut config = fs::read_to_string(&config_path).expect("read test daemon config");
    config.push_str(&format!(
        r#"
[semantic_clones]
embedding_mode = "semantic_aware_once"

[semantic_clones.inference]
code_embeddings = "{profile_name}"
summary_embeddings = "{profile_name}"

[inference.runtimes.bitloops_local_embeddings]
command = {command:?}
args = [{runtime_args}]
startup_timeout_secs = 1
request_timeout_secs = 1

[inference.profiles.{profile_name}]
task = "embeddings"
driver = "bitloops_embeddings_ipc"
runtime = "bitloops_local_embeddings"
model = "local-code"
"#
    ));
    fs::write(&config_path, config).expect("write test daemon config with embeddings profile");
    config_path
}

fn configure_summary_embeddings_only_for_repo(
    target: &EnrichmentJobTarget,
    profile_name: &str,
) -> PathBuf {
    let config_path =
        crate::test_support::git_fixtures::write_test_daemon_config(&target.config_root);
    crate::config::settings::write_repo_daemon_binding(
        &target
            .repo_root
            .join(crate::config::REPO_POLICY_LOCAL_FILE_NAME),
        &config_path,
    )
    .expect("bind repo root to daemon config");

    #[cfg(unix)]
    let (command, args) = fake_embeddings_runtime_command_and_args(&target.repo_root);
    #[cfg(windows)]
    let (command, args) = fake_embeddings_runtime_command_and_args(&target.repo_root);
    let runtime_args = args
        .iter()
        .map(|arg| format!("{arg:?}"))
        .collect::<Vec<_>>()
        .join(", ");
    let mut config = fs::read_to_string(&config_path).expect("read test daemon config");
    config.push_str(&format!(
        r#"
[semantic_clones]
embedding_mode = "semantic_aware_once"

[semantic_clones.inference]
summary_embeddings = "{profile_name}"

[inference.runtimes.bitloops_local_embeddings]
command = {command:?}
args = [{runtime_args}]
startup_timeout_secs = 1
request_timeout_secs = 1

[inference.profiles.{profile_name}]
task = "embeddings"
driver = "bitloops_embeddings_ipc"
runtime = "bitloops_local_embeddings"
model = "local-summary"
"#
    ));
    fs::write(&config_path, config)
        .expect("write test daemon config with summary-only embeddings profile");
    config_path
}

fn configure_remote_embeddings_for_repo(
    target: &EnrichmentJobTarget,
    profile_name: &str,
) -> PathBuf {
    let config_path =
        crate::test_support::git_fixtures::write_test_daemon_config(&target.config_root);
    crate::config::settings::write_repo_daemon_binding(
        &target
            .repo_root
            .join(crate::config::REPO_POLICY_LOCAL_FILE_NAME),
        &config_path,
    )
    .expect("bind repo root to daemon config");

    let mut config = fs::read_to_string(&config_path).expect("read test daemon config");
    config.push_str(&format!(
        r#"
[semantic_clones]
embedding_mode = "semantic_aware_once"

[semantic_clones.inference]
code_embeddings = "{profile_name}"
summary_embeddings = "{profile_name}"

[inference.runtimes.bitloops_platform_embeddings]
command = "platform-embeddings"
args = []
startup_timeout_secs = 60
request_timeout_secs = 300

[inference.profiles.{profile_name}]
task = "embeddings"
driver = "bitloops_embeddings_ipc"
runtime = "bitloops_platform_embeddings"
model = "bge-m3"
"#
    ));
    fs::write(&config_path, config)
        .expect("write test daemon config with remote embeddings profile");
    config_path
}

#[cfg(unix)]
fn fake_text_generation_runtime_command_and_args(repo_root: &Path) -> (String, Vec<String>) {
    use std::os::unix::fs::PermissionsExt;

    let script_path = repo_root.join(".bitloops/test-bin/fake-text-generation-runtime.sh");
    if let Some(parent) = script_path.parent() {
        fs::create_dir_all(parent).expect("create fake text-generation runtime dir");
    }
    fs::write(
        &script_path,
        r#"#!/bin/sh
while IFS= read -r line; do
  request_id=$(printf '%s' "$line" | sed -n 's/.*"request_id":"\([^"]*\)".*/\1/p')
  case "$line" in
    *'"type":"describe"'*)
      printf '{"type":"describe","request_id":"%s","protocol_version":1,"runtime_name":"bitloops-inference","runtime_version":"0.1.0","profile_name":"summary_local","provider":{"kind":"ollama_chat","provider_name":"ollama","model_name":"ministral-3:3b","endpoint":"http://127.0.0.1:11434","capabilities":["text","json_object"]}}\n' "$request_id"
      ;;
    *'"type":"shutdown"'*)
      printf '{"type":"shutdown","request_id":"%s"}\n' "$request_id"
      exit 0
      ;;
    *'"type":"infer"'*)
      printf '{"type":"infer","request_id":"%s","text":"","parsed_json":{"summary":"Summarises the symbol.","confidence":0.91},"provider_name":"ollama","model_name":"ministral-3:3b"}\n' "$request_id"
      ;;
  esac
done
"#,
    )
    .expect("write fake text-generation runtime script");
    let mut permissions = fs::metadata(&script_path)
        .expect("stat fake text-generation runtime script")
        .permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&script_path, permissions)
        .expect("chmod fake text-generation runtime script");
    (
        "/bin/sh".to_string(),
        vec![script_path.to_string_lossy().into_owned()],
    )
}

#[cfg(windows)]
fn fake_text_generation_runtime_command_and_args(repo_root: &Path) -> (String, Vec<String>) {
    let script_path = repo_root.join(".bitloops/test-bin/fake-text-generation-runtime.ps1");
    if let Some(parent) = script_path.parent() {
        fs::create_dir_all(parent).expect("create fake text-generation runtime dir");
    }
    fs::write(
        &script_path,
        r#"
while (($line = [Console]::In.ReadLine()) -ne $null) {
  if ([string]::IsNullOrWhiteSpace($line)) { continue }
  $requestId = [regex]::Match($line, '"request_id":"([^"]+)"').Groups[1].Value
  if ($line -like '*"type":"describe"*') {
    Write-Output '{"type":"describe","request_id":"'"$requestId"'","protocol_version":1,"runtime_name":"bitloops-inference","runtime_version":"0.1.0","profile_name":"summary_local","provider":{"kind":"ollama_chat","provider_name":"ollama","model_name":"ministral-3:3b","endpoint":"http://127.0.0.1:11434","capabilities":["text","json_object"]}}'
  } elseif ($line -like '*"type":"shutdown"*') {
    Write-Output '{"type":"shutdown","request_id":"'"$requestId"'"}'
    exit 0
  } elseif ($line -like '*"type":"infer"*') {
    Write-Output '{"type":"infer","request_id":"'"$requestId"'","text":"","parsed_json":{"summary":"Summarises the symbol.","confidence":0.91},"provider_name":"ollama","model_name":"ministral-3:3b"}'
  }
}
"#,
    )
    .expect("write fake text-generation runtime script");
    (
        "powershell".to_string(),
        vec![
            "-NoProfile".to_string(),
            "-ExecutionPolicy".to_string(),
            "Bypass".to_string(),
            "-File".to_string(),
            script_path.to_string_lossy().into_owned(),
        ],
    )
}

#[cfg(unix)]
fn fake_embeddings_runtime_command_and_args(repo_root: &Path) -> (String, Vec<String>) {
    use std::os::unix::fs::PermissionsExt;

    let script_path = repo_root.join(".bitloops/test-bin/fake-embeddings-runtime.sh");
    if let Some(parent) = script_path.parent() {
        fs::create_dir_all(parent).expect("create fake embeddings runtime dir");
    }
    fs::write(
        &script_path,
        r#"#!/bin/sh
printf '{"event":"ready","protocol":1,"capabilities":["embed","shutdown"]}\n'
while IFS= read -r line; do
  req_id=$(printf '%s\n' "$line" | sed -n 's/.*"id":"\([^"]*\)".*/\1/p')
  case "$line" in
    *'"cmd":"embed"'*)
      printf '{"id":"%s","ok":true,"vectors":[[0.1,0.2,0.3]],"model":"local-code"}\n' "$req_id"
      ;;
    *'"cmd":"shutdown"'*)
      printf '{"id":"%s","ok":true,"model":"local-code"}\n' "$req_id"
      exit 0
      ;;
    *)
      printf '{"id":"%s","ok":false,"error":{"message":"unexpected request"}}\n' "$req_id"
      ;;
  esac
done
"#,
    )
    .expect("write fake embeddings runtime script");
    let mut permissions = fs::metadata(&script_path)
        .expect("stat fake embeddings runtime script")
        .permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&script_path, permissions).expect("chmod fake embeddings runtime script");
    (
        "/bin/sh".to_string(),
        vec![script_path.to_string_lossy().into_owned()],
    )
}

#[cfg(windows)]
fn fake_embeddings_runtime_command_and_args(repo_root: &Path) -> (String, Vec<String>) {
    let script_path = repo_root.join(".bitloops/test-bin/fake-embeddings-runtime.ps1");
    if let Some(parent) = script_path.parent() {
        fs::create_dir_all(parent).expect("create fake embeddings runtime dir");
    }
    fs::write(
        &script_path,
        r#"
$ready = @{ event = "ready"; protocol = 1; capabilities = @("embed", "shutdown") }
$ready | ConvertTo-Json -Compress
while (($line = [Console]::In.ReadLine()) -ne $null) {
  if ([string]::IsNullOrWhiteSpace($line)) { continue }
  $request = $line | ConvertFrom-Json
  switch ($request.cmd) {
    "embed" {
      @{ id = $request.id; ok = $true; vectors = @(@(0.1, 0.2, 0.3)); model = "local-code" } | ConvertTo-Json -Compress
    }
    "shutdown" {
      @{ id = $request.id; ok = $true; model = "local-code" } | ConvertTo-Json -Compress
      exit 0
    }
    default {
      @{ id = $request.id; ok = $false; error = @{ message = "unexpected request" } } | ConvertTo-Json -Compress
    }
  }
}
"#,
    )
    .expect("write fake embeddings runtime script");
    (
        "powershell".to_string(),
        vec![
            "-NoProfile".to_string(),
            "-ExecutionPolicy".to_string(),
            "Bypass".to_string(),
            "-File".to_string(),
            script_path.to_string_lossy().into_owned(),
        ],
    )
}

fn load_workplane_jobs(
    coordinator: &EnrichmentCoordinator,
    status: WorkplaneJobStatus,
) -> Vec<WorkplaneJobRecord> {
    coordinator
        .workplane_store
        .with_connection(|conn| super::load_workplane_jobs_by_status(conn, status))
        .expect("load workplane jobs")
}

struct SummaryMailboxItemFixture<'a> {
    repo_id: &'a str,
    item_id: &'a str,
    status: SemanticMailboxItemStatus,
    item_kind: SemanticMailboxItemKind,
    artefact_id: Option<&'a str>,
    payload_json: Option<serde_json::Value>,
    submitted_at_unix: u64,
    updated_at_unix: u64,
    attempts: u32,
    lease_token: Option<&'a str>,
    lease_expires_at_unix: Option<u64>,
    last_error: Option<&'a str>,
}

struct EmbeddingMailboxItemFixture<'a> {
    repo_id: &'a str,
    item_id: &'a str,
    representation_kind: &'a str,
    status: SemanticMailboxItemStatus,
    item_kind: SemanticMailboxItemKind,
    artefact_id: Option<&'a str>,
    payload_json: Option<serde_json::Value>,
    submitted_at_unix: u64,
    updated_at_unix: u64,
    attempts: u32,
    lease_token: Option<&'a str>,
    lease_expires_at_unix: Option<u64>,
    last_error: Option<&'a str>,
}

fn insert_summary_mailbox_item(
    coordinator: &EnrichmentCoordinator,
    target: &EnrichmentJobTarget,
    fixture: SummaryMailboxItemFixture<'_>,
) {
    let dedupe_key = match (fixture.item_kind, fixture.artefact_id) {
        (SemanticMailboxItemKind::Artefact, Some(artefact_id)) => Some(format!(
            "{SEMANTIC_CLONES_SUMMARY_REFRESH_MAILBOX}:{artefact_id}"
        )),
        (SemanticMailboxItemKind::RepoBackfill, _) => Some(
            crate::capability_packs::semantic_clones::workplane::repo_backfill_dedupe_key(
                SEMANTIC_CLONES_SUMMARY_REFRESH_MAILBOX,
            ),
        ),
        _ => None,
    };
    let leased_at_unix =
        (fixture.status == SemanticMailboxItemStatus::Leased).then_some(fixture.updated_at_unix);
    coordinator
        .workplane_store
        .with_connection(|conn| {
            conn.execute(
                "INSERT INTO semantic_summary_mailbox_items (
                     item_id, repo_id, repo_root, config_root, init_session_id, item_kind,
                     artefact_id, payload_json, dedupe_key, status, attempts, available_at_unix,
                     submitted_at_unix, leased_at_unix, lease_expires_at_unix, lease_token,
                     updated_at_unix, last_error
                 ) VALUES (?1, ?2, ?3, ?4, NULL, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)",
                rusqlite::params![
                    fixture.item_id,
                    fixture.repo_id,
                    target.repo_root.to_string_lossy().to_string(),
                    target.config_root.to_string_lossy().to_string(),
                    fixture.item_kind.as_str(),
                    fixture.artefact_id,
                    fixture.payload_json.as_ref().map(serde_json::Value::to_string),
                    dedupe_key,
                    fixture.status.as_str(),
                    fixture.attempts,
                    sql_i64(fixture.submitted_at_unix)?,
                    sql_i64(fixture.submitted_at_unix)?,
                    leased_at_unix.map(sql_i64).transpose()?,
                    fixture.lease_expires_at_unix.map(sql_i64).transpose()?,
                    fixture.lease_token,
                    sql_i64(fixture.updated_at_unix)?,
                    fixture.last_error,
                ],
            )
            .map(|_| ())
            .map_err(anyhow::Error::from)
        })
        .expect("insert summary mailbox item");
}

fn insert_embedding_mailbox_item(
    coordinator: &EnrichmentCoordinator,
    target: &EnrichmentJobTarget,
    fixture: EmbeddingMailboxItemFixture<'_>,
) {
    let mailbox_name = if fixture.representation_kind == "summary" {
        SEMANTIC_CLONES_SUMMARY_EMBEDDING_MAILBOX
    } else {
        SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX
    };
    let dedupe_key = match (fixture.item_kind, fixture.artefact_id) {
        (SemanticMailboxItemKind::Artefact, Some(artefact_id)) => {
            Some(format!("{mailbox_name}:{artefact_id}"))
        }
        (SemanticMailboxItemKind::RepoBackfill, _) => Some(
            crate::capability_packs::semantic_clones::workplane::repo_backfill_dedupe_key(
                mailbox_name,
            ),
        ),
        _ => None,
    };
    let leased_at_unix =
        (fixture.status == SemanticMailboxItemStatus::Leased).then_some(fixture.updated_at_unix);
    coordinator
        .workplane_store
        .with_connection(|conn| {
            conn.execute(
                "INSERT INTO semantic_embedding_mailbox_items (
                     item_id, repo_id, repo_root, config_root, init_session_id,
                     representation_kind, item_kind, artefact_id, payload_json, dedupe_key,
                     status, attempts, available_at_unix, submitted_at_unix, leased_at_unix,
                     lease_expires_at_unix, lease_token, updated_at_unix, last_error
                 ) VALUES (?1, ?2, ?3, ?4, NULL, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18)",
                rusqlite::params![
                    fixture.item_id,
                    fixture.repo_id,
                    target.repo_root.to_string_lossy().to_string(),
                    target.config_root.to_string_lossy().to_string(),
                    fixture.representation_kind,
                    fixture.item_kind.as_str(),
                    fixture.artefact_id,
                    fixture.payload_json.as_ref().map(serde_json::Value::to_string),
                    dedupe_key,
                    fixture.status.as_str(),
                    fixture.attempts,
                    sql_i64(fixture.submitted_at_unix)?,
                    sql_i64(fixture.submitted_at_unix)?,
                    leased_at_unix.map(sql_i64).transpose()?,
                    fixture.lease_expires_at_unix.map(sql_i64).transpose()?,
                    fixture.lease_token,
                    sql_i64(fixture.updated_at_unix)?,
                    fixture.last_error,
                ],
            )
            .map(|_| ())
            .map_err(anyhow::Error::from)
        })
        .expect("insert embedding mailbox item");
}

fn load_summary_mailbox_items(
    coordinator: &EnrichmentCoordinator,
    status: SemanticMailboxItemStatus,
) -> Vec<SemanticSummaryMailboxItemRecord> {
    coordinator
        .workplane_store
        .with_connection(|conn| {
            let mut stmt = conn.prepare(
                "SELECT item_id, repo_id, repo_root, config_root, init_session_id, item_kind,
                        artefact_id, payload_json, dedupe_key, status, attempts, available_at_unix,
                        submitted_at_unix, leased_at_unix, lease_expires_at_unix, lease_token,
                        updated_at_unix, last_error
                 FROM semantic_summary_mailbox_items
                 WHERE status = ?1
                 ORDER BY submitted_at_unix ASC, item_id ASC",
            )?;
            let rows = stmt.query_map(rusqlite::params![status.as_str()], |row| {
                Ok(SemanticSummaryMailboxItemRecord {
                    item_id: row.get(0)?,
                    repo_id: row.get(1)?,
                    repo_root: PathBuf::from(row.get::<_, String>(2)?),
                    config_root: PathBuf::from(row.get::<_, String>(3)?),
                    init_session_id: row.get(4)?,
                    item_kind: SemanticMailboxItemKind::parse(&row.get::<_, String>(5)?),
                    artefact_id: row.get(6)?,
                    payload_json: row
                        .get::<_, Option<String>>(7)?
                        .and_then(|raw| serde_json::from_str(&raw).ok()),
                    dedupe_key: row.get(8)?,
                    status: SemanticMailboxItemStatus::parse(&row.get::<_, String>(9)?),
                    attempts: row.get(10)?,
                    available_at_unix: u64::try_from(row.get::<_, i64>(11)?).unwrap_or_default(),
                    submitted_at_unix: u64::try_from(row.get::<_, i64>(12)?).unwrap_or_default(),
                    leased_at_unix: row
                        .get::<_, Option<i64>>(13)?
                        .and_then(|value| u64::try_from(value).ok()),
                    lease_expires_at_unix: row
                        .get::<_, Option<i64>>(14)?
                        .and_then(|value| u64::try_from(value).ok()),
                    lease_token: row.get(15)?,
                    updated_at_unix: u64::try_from(row.get::<_, i64>(16)?).unwrap_or_default(),
                    last_error: row.get(17)?,
                })
            })?;
            let mut values = Vec::new();
            for row in rows {
                values.push(row?);
            }
            Ok::<_, anyhow::Error>(values)
        })
        .expect("load summary mailbox items")
}

fn load_embedding_mailbox_items(
    coordinator: &EnrichmentCoordinator,
    status: SemanticMailboxItemStatus,
) -> Vec<SemanticEmbeddingMailboxItemRecord> {
    coordinator
        .workplane_store
        .with_connection(|conn| {
            let mut stmt = conn.prepare(
                "SELECT item_id, repo_id, repo_root, config_root, init_session_id,
                        representation_kind, item_kind, artefact_id, payload_json, dedupe_key,
                        status, attempts, available_at_unix, submitted_at_unix, leased_at_unix,
                        lease_expires_at_unix, lease_token, updated_at_unix, last_error
                 FROM semantic_embedding_mailbox_items
                 WHERE status = ?1
                 ORDER BY submitted_at_unix ASC, item_id ASC",
            )?;
            let rows = stmt.query_map(rusqlite::params![status.as_str()], |row| {
                Ok(SemanticEmbeddingMailboxItemRecord {
                    item_id: row.get(0)?,
                    repo_id: row.get(1)?,
                    repo_root: PathBuf::from(row.get::<_, String>(2)?),
                    config_root: PathBuf::from(row.get::<_, String>(3)?),
                    init_session_id: row.get(4)?,
                    representation_kind: row.get(5)?,
                    item_kind: SemanticMailboxItemKind::parse(&row.get::<_, String>(6)?),
                    artefact_id: row.get(7)?,
                    payload_json: row
                        .get::<_, Option<String>>(8)?
                        .and_then(|raw| serde_json::from_str(&raw).ok()),
                    dedupe_key: row.get(9)?,
                    status: SemanticMailboxItemStatus::parse(&row.get::<_, String>(10)?),
                    attempts: row.get(11)?,
                    available_at_unix: u64::try_from(row.get::<_, i64>(12)?).unwrap_or_default(),
                    submitted_at_unix: u64::try_from(row.get::<_, i64>(13)?).unwrap_or_default(),
                    leased_at_unix: row
                        .get::<_, Option<i64>>(14)?
                        .and_then(|value| u64::try_from(value).ok()),
                    lease_expires_at_unix: row
                        .get::<_, Option<i64>>(15)?
                        .and_then(|value| u64::try_from(value).ok()),
                    lease_token: row.get(16)?,
                    updated_at_unix: u64::try_from(row.get::<_, i64>(17)?).unwrap_or_default(),
                    last_error: row.get(18)?,
                })
            })?;
            let mut values = Vec::new();
            for row in rows {
                values.push(row?);
            }
            Ok::<_, anyhow::Error>(values)
        })
        .expect("load embedding mailbox items")
}

#[test]
fn summary_mailbox_batch_claim_leases_up_to_ten_items_without_touching_embedding_rows() {
    let temp = TempDir::new().expect("temp dir");
    let (coordinator, target, repo_id) = new_test_coordinator(&temp);
    configure_summary_refresh_for_repo(&target);

    for index in 0..55 {
        insert_summary_mailbox_item(
            &coordinator,
            &target,
            SummaryMailboxItemFixture {
                repo_id: &repo_id,
                item_id: &format!("summary-item-{index}"),
                status: SemanticMailboxItemStatus::Pending,
                item_kind: SemanticMailboxItemKind::Artefact,
                artefact_id: Some(&format!("summary-{index}")),
                payload_json: None,
                submitted_at_unix: (index + 1) as u64,
                updated_at_unix: (index + 1) as u64,
                attempts: 0,
                lease_token: None,
                lease_expires_at_unix: None,
                last_error: None,
            },
        );
    }
    insert_embedding_mailbox_item(
        &coordinator,
        &target,
        EmbeddingMailboxItemFixture {
            repo_id: &repo_id,
            item_id: "embedding-pending",
            representation_kind: "code",
            status: SemanticMailboxItemStatus::Pending,
            item_kind: SemanticMailboxItemKind::Artefact,
            artefact_id: Some("embedding-a"),
            payload_json: None,
            submitted_at_unix: 1,
            updated_at_unix: 1,
            attempts: 0,
            lease_token: None,
            lease_expires_at_unix: None,
            last_error: None,
        },
    );

    let claimed = super::claim_summary_mailbox_batch(
        &coordinator.workplane_store,
        &coordinator.runtime_store,
        &default_state(),
    )
    .expect("claim summary mailbox batch")
    .expect("summary mailbox batch should be claimable");

    assert_eq!(claimed.items.len(), 10);
    assert!(claimed.items.iter().all(|item| item.repo_id == repo_id));
    assert!(
        !claimed.lease_token.is_empty(),
        "summary claim should assign one shared lease token",
    );
    assert_eq!(
        load_summary_mailbox_items(&coordinator, SemanticMailboxItemStatus::Leased).len(),
        10,
    );
    assert_eq!(
        load_summary_mailbox_items(&coordinator, SemanticMailboxItemStatus::Pending).len(),
        45,
    );
    assert_eq!(
        load_embedding_mailbox_items(&coordinator, SemanticMailboxItemStatus::Pending).len(),
        1,
    );
}

#[test]
fn embedding_mailbox_batch_claim_leases_up_to_fifty_items_from_embedding_inbox_only() {
    let temp = TempDir::new().expect("temp dir");
    let (coordinator, target, repo_id) = new_test_coordinator(&temp);
    configure_summary_refresh_for_repo(&target);
    let _config_path = configure_embeddings_for_repo(&target, "local_code");

    insert_summary_mailbox_item(
        &coordinator,
        &target,
        SummaryMailboxItemFixture {
            repo_id: &repo_id,
            item_id: "summary-pending",
            status: SemanticMailboxItemStatus::Pending,
            item_kind: SemanticMailboxItemKind::Artefact,
            artefact_id: Some("summary-a"),
            payload_json: None,
            submitted_at_unix: 1,
            updated_at_unix: 1,
            attempts: 0,
            lease_token: None,
            lease_expires_at_unix: None,
            last_error: None,
        },
    );
    for index in 0..55 {
        insert_embedding_mailbox_item(
            &coordinator,
            &target,
            EmbeddingMailboxItemFixture {
                repo_id: &repo_id,
                item_id: &format!("embedding-item-{index}"),
                representation_kind: "code",
                status: SemanticMailboxItemStatus::Pending,
                item_kind: SemanticMailboxItemKind::Artefact,
                artefact_id: Some(&format!("code-{index}")),
                payload_json: None,
                submitted_at_unix: (index + 1) as u64,
                updated_at_unix: (index + 1) as u64,
                attempts: 0,
                lease_token: None,
                lease_expires_at_unix: None,
                last_error: None,
            },
        );
    }

    let claimed = super::claim_embedding_mailbox_batch(
        &coordinator.workplane_store,
        &coordinator.runtime_store,
        &default_state(),
    )
    .expect("claim embedding mailbox batch")
    .expect("embedding mailbox batch should be claimable");

    assert_eq!(claimed.items.len(), 50);
    assert_eq!(claimed.representation_kind.to_string(), "code");
    assert!(
        claimed
            .items
            .iter()
            .all(|item| item.representation_kind == "code"),
    );
    assert_eq!(
        load_embedding_mailbox_items(&coordinator, SemanticMailboxItemStatus::Leased).len(),
        50,
    );
    assert_eq!(
        load_embedding_mailbox_items(&coordinator, SemanticMailboxItemStatus::Pending).len(),
        5,
    );
    assert_eq!(
        load_summary_mailbox_items(&coordinator, SemanticMailboxItemStatus::Pending).len(),
        1,
    );
}

#[test]
fn embedding_mailbox_batch_claim_leases_identity_items_when_embeddings_are_configured() {
    let temp = TempDir::new().expect("temp dir");
    let (coordinator, target, repo_id) = new_test_coordinator(&temp);
    let _config_path = configure_embeddings_for_repo(&target, "local_code");

    for index in 0..2 {
        insert_embedding_mailbox_item(
            &coordinator,
            &target,
            EmbeddingMailboxItemFixture {
                repo_id: &repo_id,
                item_id: &format!("identity-item-{index}"),
                representation_kind: "identity",
                status: SemanticMailboxItemStatus::Pending,
                item_kind: SemanticMailboxItemKind::Artefact,
                artefact_id: Some(&format!("identity-{index}")),
                payload_json: None,
                submitted_at_unix: (index + 1) as u64,
                updated_at_unix: (index + 1) as u64,
                attempts: 0,
                lease_token: None,
                lease_expires_at_unix: None,
                last_error: None,
            },
        );
    }

    let claimed = super::claim_embedding_mailbox_batch(
        &coordinator.workplane_store,
        &coordinator.runtime_store,
        &default_state(),
    )
    .expect("claim embedding mailbox batch")
    .expect("identity embedding mailbox batch should be claimable");

    assert_eq!(claimed.items.len(), 2);
    assert_eq!(claimed.representation_kind.to_string(), "identity");
    assert!(
        claimed
            .items
            .iter()
            .all(|item| item.representation_kind == "identity"),
    );
    assert_eq!(
        load_embedding_mailbox_items(&coordinator, SemanticMailboxItemStatus::Leased).len(),
        2,
    );
    assert_eq!(
        load_embedding_mailbox_items(&coordinator, SemanticMailboxItemStatus::Pending).len(),
        0,
    );
}

#[test]
fn projected_workplane_status_counts_inbox_backed_batches() {
    let temp = TempDir::new().expect("temp dir");
    let (coordinator, target, repo_id) = new_test_coordinator(&temp);
    configure_summary_refresh_for_repo(&target);
    let _config_path = configure_embeddings_for_repo(&target, "local_code");

    insert_summary_mailbox_item(
        &coordinator,
        &target,
        SummaryMailboxItemFixture {
            repo_id: &repo_id,
            item_id: "summary-pending",
            status: SemanticMailboxItemStatus::Pending,
            item_kind: SemanticMailboxItemKind::Artefact,
            artefact_id: Some("summary-a"),
            payload_json: None,
            submitted_at_unix: 1,
            updated_at_unix: 1,
            attempts: 0,
            lease_token: None,
            lease_expires_at_unix: None,
            last_error: None,
        },
    );
    insert_embedding_mailbox_item(
        &coordinator,
        &target,
        EmbeddingMailboxItemFixture {
            repo_id: &repo_id,
            item_id: "embedding-lease-a",
            representation_kind: "code",
            status: SemanticMailboxItemStatus::Leased,
            item_kind: SemanticMailboxItemKind::Artefact,
            artefact_id: Some("code-a"),
            payload_json: None,
            submitted_at_unix: 2,
            updated_at_unix: 2,
            attempts: 1,
            lease_token: Some("embedding-lease"),
            lease_expires_at_unix: Some(unix_timestamp_now() + 300),
            last_error: None,
        },
    );
    insert_embedding_mailbox_item(
        &coordinator,
        &target,
        EmbeddingMailboxItemFixture {
            repo_id: &repo_id,
            item_id: "embedding-lease-b",
            representation_kind: "code",
            status: SemanticMailboxItemStatus::Leased,
            item_kind: SemanticMailboxItemKind::Artefact,
            artefact_id: Some("code-b"),
            payload_json: None,
            submitted_at_unix: 3,
            updated_at_unix: 3,
            attempts: 1,
            lease_token: Some("embedding-lease"),
            lease_expires_at_unix: Some(unix_timestamp_now() + 300),
            last_error: None,
        },
    );
    insert_workplane_job(
        &coordinator,
        &target,
        WorkplaneJobFixture {
            repo_id: &repo_id,
            mailbox_name: SEMANTIC_CLONES_CLONE_REBUILD_MAILBOX,
            status: WorkplaneJobStatus::Failed,
            artefact_id: None,
            job_id: "clone-failed",
            updated_at_unix: 4,
            attempts: 2,
            last_error: Some("failed"),
        },
    );

    let projected = project_workplane_status(
        &coordinator.workplane_store,
        &default_state(),
        super::worker_count::EnrichmentWorkerBudgets {
            summary_refresh: 1,
            embeddings: 1,
            clone_rebuild: 1,
        },
    )
    .expect("project workplane status");

    assert_eq!(projected.pending_semantic_jobs, 1);
    assert_eq!(projected.pending_semantic_work_items, 1);
    assert_eq!(projected.running_embedding_jobs, 1);
    assert_eq!(projected.running_embedding_work_items, 2);
    assert_eq!(projected.failed_clone_edges_rebuild_jobs, 1);
    assert_eq!(projected.completed_recent_jobs, 0);
}

#[test]
fn ensure_started_migrates_legacy_semantic_rows_into_the_new_inboxes() {
    let temp = TempDir::new().expect("temp dir");
    let (coordinator, target, repo_id) = new_test_coordinator(&temp);
    let coordinator = Arc::new(coordinator);
    configure_summary_refresh_for_repo(&target);
    let _config_path = configure_embeddings_for_repo(&target, "local_code");

    insert_workplane_job(
        &coordinator,
        &target,
        WorkplaneJobFixture {
            repo_id: &repo_id,
            mailbox_name: SEMANTIC_CLONES_SUMMARY_REFRESH_MAILBOX,
            status: WorkplaneJobStatus::Pending,
            artefact_id: Some("summary-a"),
            job_id: "legacy-summary-pending",
            updated_at_unix: 1,
            attempts: 0,
            last_error: None,
        },
    );
    insert_workplane_job(
        &coordinator,
        &target,
        WorkplaneJobFixture {
            repo_id: &repo_id,
            mailbox_name: SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX,
            status: WorkplaneJobStatus::Running,
            artefact_id: Some("code-a"),
            job_id: "legacy-embedding-running",
            updated_at_unix: 2,
            attempts: 1,
            last_error: None,
        },
    );

    coordinator.ensure_started();

    let legacy_semantic_jobs = load_workplane_jobs(&coordinator, WorkplaneJobStatus::Pending)
        .into_iter()
        .chain(load_workplane_jobs(
            &coordinator,
            WorkplaneJobStatus::Running,
        ))
        .filter(|job| {
            matches!(
                job.mailbox_name.as_str(),
                SEMANTIC_CLONES_SUMMARY_REFRESH_MAILBOX
                    | SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX
                    | SEMANTIC_CLONES_SUMMARY_EMBEDDING_MAILBOX
            )
        })
        .collect::<Vec<_>>();
    assert!(
        legacy_semantic_jobs.is_empty(),
        "startup should migrate pending and running semantic rows out of the legacy workplane table",
    );

    let pending_summary =
        load_summary_mailbox_items(&coordinator, SemanticMailboxItemStatus::Pending);
    let pending_embeddings =
        load_embedding_mailbox_items(&coordinator, SemanticMailboxItemStatus::Pending);
    assert_eq!(pending_summary.len(), 1);
    assert_eq!(pending_embeddings.len(), 1);
    assert_eq!(pending_summary[0].artefact_id.as_deref(), Some("summary-a"));
    assert_eq!(pending_embeddings[0].artefact_id.as_deref(), Some("code-a"));
}

#[test]
fn retry_failed_jobs_migrates_legacy_embedding_repo_backfill_rows_into_the_embedding_inbox() {
    let temp = TempDir::new().expect("temp dir");
    let (coordinator, target, repo_id) = new_test_coordinator(&temp);
    let artefact_ids = (0..40)
        .map(|index| format!("artefact-{index}"))
        .collect::<Vec<_>>();
    let payload = serde_json::to_string(
        &crate::capability_packs::semantic_clones::workplane::SemanticClonesMailboxPayload::RepoBackfill {
            work_item_count: Some(artefact_ids.len() as u64),
            artefact_ids: Some(artefact_ids.clone()),
        },
    )
    .expect("serialize repo backfill payload");
    let dedupe_key = crate::capability_packs::semantic_clones::workplane::repo_backfill_dedupe_key(
        SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX,
    );
    coordinator
        .workplane_store
        .with_connection(|conn| {
            conn.execute(
                "INSERT INTO capability_workplane_jobs (
                     job_id, repo_id, repo_root, config_root, capability_id, mailbox_name,
                     dedupe_key, payload, status, attempts, available_at_unix, submitted_at_unix,
                     started_at_unix, updated_at_unix, completed_at_unix, lease_owner,
                     lease_expires_at_unix, last_error
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, NULL, NULL, ?16)",
                rusqlite::params![
                    "failed-backfill",
                    repo_id,
                    target.repo_root.to_string_lossy().to_string(),
                    target.config_root.to_string_lossy().to_string(),
                    SEMANTIC_CLONES_CAPABILITY_ID,
                    SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX,
                    dedupe_key,
                    payload,
                    WorkplaneJobStatus::Failed.as_str(),
                    2u32,
                    sql_i64(1)?,
                    sql_i64(1)?,
                    sql_i64(1)?,
                    sql_i64(1)?,
                    sql_i64(1)?,
                    "timeout",
                ],
            )
            .map(|_| ())
            .map_err(anyhow::Error::from)
        })
        .expect("insert failed repo backfill job");

    let retried =
        super::retry_failed_jobs_in_store(&coordinator.workplane_store).expect("retry failed jobs");

    assert_eq!(retried, 1);
    assert!(
        load_workplane_jobs(&coordinator, WorkplaneJobStatus::Pending)
            .into_iter()
            .all(|job| job.mailbox_name != SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX),
        "legacy embedding rows should be migrated out of the workplane table after retry",
    );
    let pending_items =
        load_embedding_mailbox_items(&coordinator, SemanticMailboxItemStatus::Pending);
    assert_eq!(pending_items.len(), 1);
    assert_eq!(
        pending_items[0].item_kind,
        SemanticMailboxItemKind::RepoBackfill
    );
    let requeued_artefact_ids = pending_items[0]
        .payload_json
        .as_ref()
        .and_then(serde_json::Value::as_array)
        .expect("explicit artefact ids should be preserved");
    assert_eq!(requeued_artefact_ids.len(), 40);
    assert_eq!(
        requeued_artefact_ids
            .first()
            .and_then(serde_json::Value::as_str),
        Some("artefact-0")
    );
    assert_eq!(
        requeued_artefact_ids
            .last()
            .and_then(serde_json::Value::as_str),
        Some("artefact-39")
    );
}

#[tokio::test(start_paused = true, flavor = "current_thread")]
async fn periodic_maintenance_requeues_expired_semantic_leases_on_the_sixty_second_tick() {
    let temp = TempDir::new().expect("temp dir");
    let (coordinator, target, repo_id) = new_test_coordinator(&temp);
    let coordinator = Arc::new(coordinator);

    coordinator.ensure_started();
    tokio::task::yield_now().await;

    insert_summary_mailbox_item(
        &coordinator,
        &target,
        SummaryMailboxItemFixture {
            repo_id: &repo_id,
            item_id: "expired-summary-lease",
            status: SemanticMailboxItemStatus::Leased,
            item_kind: SemanticMailboxItemKind::Artefact,
            artefact_id: Some("summary-a"),
            payload_json: None,
            submitted_at_unix: 1,
            updated_at_unix: 1,
            attempts: 1,
            lease_token: Some("expired-summary-lease"),
            lease_expires_at_unix: Some(1),
            last_error: None,
        },
    );

    assert_eq!(
        load_summary_mailbox_items(&coordinator, SemanticMailboxItemStatus::Leased).len(),
        1,
    );

    advance(Duration::from_secs(59)).await;
    tokio::task::yield_now().await;
    assert_eq!(
        load_summary_mailbox_items(&coordinator, SemanticMailboxItemStatus::Leased).len(),
        1,
    );

    advance(Duration::from_secs(1)).await;
    tokio::task::yield_now().await;
    tokio::task::yield_now().await;

    let pending_items =
        load_summary_mailbox_items(&coordinator, SemanticMailboxItemStatus::Pending);
    assert_eq!(pending_items.len(), 1);
    assert!(pending_items[0].lease_token.is_none());
    assert!(pending_items[0].lease_expires_at_unix.is_none());
    assert_eq!(
        load_summary_mailbox_items(&coordinator, SemanticMailboxItemStatus::Leased).len(),
        0,
    );
}

#[test]
fn summary_refresh_pool_only_claims_summary_jobs() {
    let temp = TempDir::new().expect("temp dir");
    let (coordinator, target, repo_id) = new_test_coordinator(&temp);
    configure_summary_refresh_for_repo(&target);

    insert_workplane_job(
        &coordinator,
        &target,
        WorkplaneJobFixture {
            repo_id: &repo_id,
            mailbox_name: SEMANTIC_CLONES_SUMMARY_REFRESH_MAILBOX,
            status: WorkplaneJobStatus::Pending,
            artefact_id: Some("summary-a"),
            job_id: "summary-a",
            updated_at_unix: 1,
            attempts: 0,
            last_error: None,
        },
    );
    insert_workplane_job(
        &coordinator,
        &target,
        WorkplaneJobFixture {
            repo_id: &repo_id,
            mailbox_name: SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX,
            status: WorkplaneJobStatus::Pending,
            artefact_id: Some("code-a"),
            job_id: "code-a",
            updated_at_unix: 2,
            attempts: 0,
            last_error: None,
        },
    );
    insert_workplane_job(
        &coordinator,
        &target,
        WorkplaneJobFixture {
            repo_id: &repo_id,
            mailbox_name: SEMANTIC_CLONES_CLONE_REBUILD_MAILBOX,
            status: WorkplaneJobStatus::Pending,
            artefact_id: None,
            job_id: "clone-a",
            updated_at_unix: 3,
            attempts: 0,
            last_error: None,
        },
    );

    let claimed = claim_next_workplane_job(
        &coordinator.workplane_store,
        &coordinator.runtime_store,
        &default_state(),
        super::worker_count::EnrichmentWorkerPool::SummaryRefresh,
    )
    .expect("claim summary refresh job")
    .expect("summary refresh job should be claimable");

    assert_eq!(
        claimed.mailbox_name,
        SEMANTIC_CLONES_SUMMARY_REFRESH_MAILBOX
    );
}

#[test]
fn summary_refresh_pool_skips_paused_jobs() {
    let temp = TempDir::new().expect("temp dir");
    let (coordinator, target, repo_id) = new_test_coordinator(&temp);
    configure_summary_refresh_for_repo(&target);

    insert_workplane_job(
        &coordinator,
        &target,
        WorkplaneJobFixture {
            repo_id: &repo_id,
            mailbox_name: SEMANTIC_CLONES_SUMMARY_REFRESH_MAILBOX,
            status: WorkplaneJobStatus::Pending,
            artefact_id: Some("summary-a"),
            job_id: "summary-a",
            updated_at_unix: 1,
            attempts: 0,
            last_error: None,
        },
    );

    let mut state = default_state();
    state.paused_semantic = true;
    let claimed = claim_next_workplane_job(
        &coordinator.workplane_store,
        &coordinator.runtime_store,
        &state,
        super::worker_count::EnrichmentWorkerPool::SummaryRefresh,
    )
    .expect("attempt paused summary refresh claim");

    assert!(claimed.is_none());
}

#[test]
fn embeddings_pool_only_claims_embedding_jobs() {
    let temp = TempDir::new().expect("temp dir");
    let (coordinator, target, repo_id) = new_test_coordinator(&temp);
    let _config_path = configure_embeddings_for_repo(&target, "local_code");

    insert_workplane_job(
        &coordinator,
        &target,
        WorkplaneJobFixture {
            repo_id: &repo_id,
            mailbox_name: SEMANTIC_CLONES_SUMMARY_REFRESH_MAILBOX,
            status: WorkplaneJobStatus::Pending,
            artefact_id: Some("summary-a"),
            job_id: "summary-a",
            updated_at_unix: 1,
            attempts: 0,
            last_error: None,
        },
    );
    insert_workplane_job(
        &coordinator,
        &target,
        WorkplaneJobFixture {
            repo_id: &repo_id,
            mailbox_name: SEMANTIC_CLONES_SUMMARY_EMBEDDING_MAILBOX,
            status: WorkplaneJobStatus::Pending,
            artefact_id: Some("summary-embed-a"),
            job_id: "summary-embed-a",
            updated_at_unix: 2,
            attempts: 0,
            last_error: None,
        },
    );
    insert_workplane_job(
        &coordinator,
        &target,
        WorkplaneJobFixture {
            repo_id: &repo_id,
            mailbox_name: SEMANTIC_CLONES_CLONE_REBUILD_MAILBOX,
            status: WorkplaneJobStatus::Pending,
            artefact_id: None,
            job_id: "clone-a",
            updated_at_unix: 3,
            attempts: 0,
            last_error: None,
        },
    );

    let claimed = claim_next_workplane_job(
        &coordinator.workplane_store,
        &coordinator.runtime_store,
        &default_state(),
        super::worker_count::EnrichmentWorkerPool::Embeddings,
    )
    .expect("claim embeddings job")
    .expect("embedding job should be claimable");

    assert_eq!(
        claimed.mailbox_name,
        SEMANTIC_CLONES_SUMMARY_EMBEDDING_MAILBOX
    );
}

#[test]
fn embeddings_pool_skips_unready_candidates_with_bounded_query() {
    let temp = TempDir::new().expect("temp dir");
    let (coordinator, target, repo_id) = new_test_coordinator(&temp);
    let _config_path = configure_summary_embeddings_only_for_repo(&target, "summary_only");

    for index in 0..31 {
        let job_id = format!("blocked-code-{index}");
        let artefact_id = format!("code-{index}");
        insert_workplane_job(
            &coordinator,
            &target,
            WorkplaneJobFixture {
                repo_id: &repo_id,
                mailbox_name: SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX,
                status: WorkplaneJobStatus::Pending,
                artefact_id: Some(&artefact_id),
                job_id: &job_id,
                updated_at_unix: (index + 1) as u64,
                attempts: 0,
                last_error: None,
            },
        );
    }
    insert_workplane_job(
        &coordinator,
        &target,
        WorkplaneJobFixture {
            repo_id: &repo_id,
            mailbox_name: SEMANTIC_CLONES_SUMMARY_EMBEDDING_MAILBOX,
            status: WorkplaneJobStatus::Pending,
            artefact_id: Some("summary-ready"),
            job_id: "summary-ready",
            updated_at_unix: 32,
            attempts: 0,
            last_error: None,
        },
    );
    insert_workplane_job(
        &coordinator,
        &target,
        WorkplaneJobFixture {
            repo_id: &repo_id,
            mailbox_name: SEMANTIC_CLONES_SUMMARY_EMBEDDING_MAILBOX,
            status: WorkplaneJobStatus::Pending,
            artefact_id: Some("summary-later"),
            job_id: "summary-later",
            updated_at_unix: 33,
            attempts: 0,
            last_error: None,
        },
    );

    let claimed = claim_next_workplane_job(
        &coordinator.workplane_store,
        &coordinator.runtime_store,
        &default_state(),
        super::worker_count::EnrichmentWorkerPool::Embeddings,
    )
    .expect("claim embeddings job with blocked candidates")
    .expect("summary embedding job should be claimable");

    assert_eq!(
        claimed.mailbox_name,
        SEMANTIC_CLONES_SUMMARY_EMBEDDING_MAILBOX
    );
    assert_eq!(claimed.job_id, "summary-ready");
}

#[test]
fn clone_rebuild_pool_only_claims_clone_rebuild_jobs() {
    let temp = TempDir::new().expect("temp dir");
    let (coordinator, target, repo_id) = new_test_coordinator(&temp);
    configure_summary_refresh_for_repo(&target);

    insert_workplane_job(
        &coordinator,
        &target,
        WorkplaneJobFixture {
            repo_id: &repo_id,
            mailbox_name: SEMANTIC_CLONES_SUMMARY_REFRESH_MAILBOX,
            status: WorkplaneJobStatus::Pending,
            artefact_id: Some("summary-a"),
            job_id: "summary-a",
            updated_at_unix: 1,
            attempts: 0,
            last_error: None,
        },
    );
    insert_workplane_job(
        &coordinator,
        &target,
        WorkplaneJobFixture {
            repo_id: &repo_id,
            mailbox_name: SEMANTIC_CLONES_CLONE_REBUILD_MAILBOX,
            status: WorkplaneJobStatus::Pending,
            artefact_id: None,
            job_id: "clone-a",
            updated_at_unix: 2,
            attempts: 0,
            last_error: None,
        },
    );

    let claimed = claim_next_workplane_job(
        &coordinator.workplane_store,
        &coordinator.runtime_store,
        &default_state(),
        super::worker_count::EnrichmentWorkerPool::CloneRebuild,
    )
    .expect("claim clone rebuild job")
    .expect("clone rebuild job should be claimable");

    assert_eq!(claimed.mailbox_name, SEMANTIC_CLONES_CLONE_REBUILD_MAILBOX);
}

#[test]
fn embeddings_pool_does_not_borrow_summary_or_clone_rebuild_work() {
    let temp = TempDir::new().expect("temp dir");
    let (coordinator, target, repo_id) = new_test_coordinator(&temp);
    configure_summary_refresh_for_repo(&target);

    insert_workplane_job(
        &coordinator,
        &target,
        WorkplaneJobFixture {
            repo_id: &repo_id,
            mailbox_name: SEMANTIC_CLONES_SUMMARY_REFRESH_MAILBOX,
            status: WorkplaneJobStatus::Pending,
            artefact_id: Some("summary-a"),
            job_id: "summary-a",
            updated_at_unix: 1,
            attempts: 0,
            last_error: None,
        },
    );
    insert_workplane_job(
        &coordinator,
        &target,
        WorkplaneJobFixture {
            repo_id: &repo_id,
            mailbox_name: SEMANTIC_CLONES_CLONE_REBUILD_MAILBOX,
            status: WorkplaneJobStatus::Pending,
            artefact_id: None,
            job_id: "clone-a",
            updated_at_unix: 2,
            attempts: 0,
            last_error: None,
        },
    );

    let claimed = claim_next_workplane_job(
        &coordinator.workplane_store,
        &coordinator.runtime_store,
        &default_state(),
        super::worker_count::EnrichmentWorkerPool::Embeddings,
    )
    .expect("attempt embeddings pool claim");

    assert!(claimed.is_none());
}

#[test]
fn summary_refresh_pool_skips_future_available_jobs() {
    let temp = TempDir::new().expect("temp dir");
    let (coordinator, target, repo_id) = new_test_coordinator(&temp);
    configure_summary_refresh_for_repo(&target);
    let now = unix_timestamp_now();

    insert_workplane_job(
        &coordinator,
        &target,
        WorkplaneJobFixture {
            repo_id: &repo_id,
            mailbox_name: SEMANTIC_CLONES_SUMMARY_REFRESH_MAILBOX,
            status: WorkplaneJobStatus::Pending,
            artefact_id: Some("summary-future"),
            job_id: "summary-future",
            updated_at_unix: 1,
            attempts: 0,
            last_error: None,
        },
    );
    set_workplane_job_schedule(&coordinator, "summary-future", now + 300, 1);
    insert_workplane_job(
        &coordinator,
        &target,
        WorkplaneJobFixture {
            repo_id: &repo_id,
            mailbox_name: SEMANTIC_CLONES_SUMMARY_REFRESH_MAILBOX,
            status: WorkplaneJobStatus::Pending,
            artefact_id: Some("summary-ready"),
            job_id: "summary-ready",
            updated_at_unix: 2,
            attempts: 0,
            last_error: None,
        },
    );

    let claimed = claim_next_workplane_job(
        &coordinator.workplane_store,
        &coordinator.runtime_store,
        &default_state(),
        super::worker_count::EnrichmentWorkerPool::SummaryRefresh,
    )
    .expect("claim summary refresh job with future candidate")
    .expect("ready summary refresh job should be claimable");

    assert_eq!(claimed.job_id, "summary-ready");
}

#[test]
fn projected_workplane_status_reports_per_pool_counts() {
    let temp = TempDir::new().expect("temp dir");
    let (coordinator, target, repo_id) = new_test_coordinator(&temp);
    configure_summary_refresh_for_repo(&target);
    let _config_path = configure_embeddings_for_repo(&target, "local_code");

    insert_workplane_job(
        &coordinator,
        &target,
        WorkplaneJobFixture {
            repo_id: &repo_id,
            mailbox_name: SEMANTIC_CLONES_SUMMARY_REFRESH_MAILBOX,
            status: WorkplaneJobStatus::Completed,
            artefact_id: Some("summary-complete"),
            job_id: "summary-complete",
            updated_at_unix: 1,
            attempts: 1,
            last_error: None,
        },
    );
    insert_workplane_job(
        &coordinator,
        &target,
        WorkplaneJobFixture {
            repo_id: &repo_id,
            mailbox_name: SEMANTIC_CLONES_SUMMARY_REFRESH_MAILBOX,
            status: WorkplaneJobStatus::Pending,
            artefact_id: Some("summary-pending"),
            job_id: "summary-pending",
            updated_at_unix: 2,
            attempts: 0,
            last_error: None,
        },
    );
    insert_workplane_job(
        &coordinator,
        &target,
        WorkplaneJobFixture {
            repo_id: &repo_id,
            mailbox_name: SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX,
            status: WorkplaneJobStatus::Running,
            artefact_id: Some("code-running"),
            job_id: "code-running",
            updated_at_unix: 3,
            attempts: 1,
            last_error: None,
        },
    );
    insert_workplane_job(
        &coordinator,
        &target,
        WorkplaneJobFixture {
            repo_id: &repo_id,
            mailbox_name: SEMANTIC_CLONES_CLONE_REBUILD_MAILBOX,
            status: WorkplaneJobStatus::Failed,
            artefact_id: None,
            job_id: "clone-failed",
            updated_at_unix: 4,
            attempts: 2,
            last_error: Some("failed"),
        },
    );

    let projected = project_workplane_status(
        &coordinator.workplane_store,
        &default_state(),
        super::worker_count::EnrichmentWorkerBudgets {
            summary_refresh: 1,
            embeddings: 1,
            clone_rebuild: 1,
        },
    )
    .expect("project workplane status");

    assert_eq!(projected.completed_recent_jobs, 1);
    assert_eq!(projected.worker_pools.len(), 3);
    assert_eq!(
        projected
            .worker_pools
            .iter()
            .find(|pool| pool.kind == crate::daemon::EnrichmentWorkerPoolKind::SummaryRefresh)
            .map(|pool| {
                (
                    pool.pending_jobs,
                    pool.running_jobs,
                    pool.failed_jobs,
                    pool.completed_recent_jobs,
                )
            }),
        Some((1, 0, 0, 1))
    );
    assert_eq!(
        projected
            .worker_pools
            .iter()
            .find(|pool| pool.kind == crate::daemon::EnrichmentWorkerPoolKind::Embeddings)
            .map(|pool| {
                (
                    pool.pending_jobs,
                    pool.running_jobs,
                    pool.failed_jobs,
                    pool.completed_recent_jobs,
                )
            }),
        Some((0, 1, 0, 0))
    );
    assert_eq!(
        projected
            .worker_pools
            .iter()
            .find(|pool| pool.kind == crate::daemon::EnrichmentWorkerPoolKind::CloneRebuild)
            .map(|pool| {
                (
                    pool.pending_jobs,
                    pool.running_jobs,
                    pool.failed_jobs,
                    pool.completed_recent_jobs,
                )
            }),
        Some((0, 0, 1, 0))
    );
}

#[test]
fn effective_worker_budgets_use_remote_embedding_defaults_for_active_config_roots() {
    let temp = TempDir::new().expect("temp dir");
    let (coordinator, target, repo_id) = new_test_coordinator(&temp);
    let _config_path = configure_remote_embeddings_for_repo(&target, "platform_code");

    insert_workplane_job(
        &coordinator,
        &target,
        WorkplaneJobFixture {
            repo_id: &repo_id,
            mailbox_name: SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX,
            status: WorkplaneJobStatus::Pending,
            artefact_id: Some("code-pending"),
            job_id: "code-pending",
            updated_at_unix: 1,
            attempts: 0,
            last_error: None,
        },
    );

    let budgets = effective_worker_budgets(
        &coordinator.workplane_store,
        &coordinator.daemon_config_root,
    )
    .expect("resolve effective worker budgets");

    assert_eq!(budgets.embeddings, 4);
}

struct WorkplaneJobFixture<'a> {
    repo_id: &'a str,
    mailbox_name: &'a str,
    status: WorkplaneJobStatus,
    artefact_id: Option<&'a str>,
    job_id: &'a str,
    updated_at_unix: u64,
    attempts: u32,
    last_error: Option<&'a str>,
}

fn insert_workplane_job(
    coordinator: &EnrichmentCoordinator,
    target: &EnrichmentJobTarget,
    fixture: WorkplaneJobFixture<'_>,
) {
    let dedupe_key = match (fixture.mailbox_name, fixture.artefact_id) {
        (SEMANTIC_CLONES_CLONE_REBUILD_MAILBOX, _) => {
            Some(SEMANTIC_CLONES_CLONE_REBUILD_MAILBOX.to_string())
        }
        (_, Some(artefact_id)) => Some(format!("{}:{artefact_id}", fixture.mailbox_name)),
        _ => None,
    };
    let payload = fixture
        .artefact_id
        .map(|artefact_id| serde_json::json!({ "artefact_id": artefact_id }))
        .unwrap_or_else(|| {
            serde_json::to_value(
                crate::capability_packs::semantic_clones::workplane::SemanticClonesMailboxPayload::RepoBackfill {
                    work_item_count: None,
                    artefact_ids: None,
                },
            )
            .expect("serialize repo backfill test payload")
        });
    let started_at_unix =
        (fixture.status == WorkplaneJobStatus::Running).then_some(fixture.updated_at_unix);
    let completed_at_unix = matches!(
        fixture.status,
        WorkplaneJobStatus::Completed | WorkplaneJobStatus::Failed
    )
    .then_some(fixture.updated_at_unix);
    coordinator
        .workplane_store
        .with_connection(|conn| {
            conn.execute(
                "INSERT INTO capability_workplane_jobs (
                     job_id, repo_id, repo_root, config_root, capability_id, mailbox_name,
                     dedupe_key, payload, status, attempts, available_at_unix, submitted_at_unix,
                     started_at_unix, updated_at_unix, completed_at_unix, lease_owner,
                     lease_expires_at_unix, last_error
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, NULL, NULL, ?16)",
                rusqlite::params![
                    fixture.job_id,
                    fixture.repo_id,
                    target.repo_root.to_string_lossy().to_string(),
                    target.config_root.to_string_lossy().to_string(),
                    SEMANTIC_CLONES_CAPABILITY_ID,
                    fixture.mailbox_name,
                    dedupe_key,
                    payload.to_string(),
                    fixture.status.as_str(),
                    fixture.attempts,
                    sql_i64(fixture.updated_at_unix)?,
                    sql_i64(fixture.updated_at_unix)?,
                    started_at_unix.map(sql_i64).transpose()?,
                    sql_i64(fixture.updated_at_unix)?,
                    completed_at_unix.map(sql_i64).transpose()?,
                    fixture.last_error,
                ],
            )
            .map(|_| ())
            .map_err(anyhow::Error::from)
        })
        .expect("insert workplane job");
}

fn insert_pending_artefact_jobs_bulk(
    coordinator: &EnrichmentCoordinator,
    target: &EnrichmentJobTarget,
    repo_id: &str,
    mailbox_name: &str,
    count: usize,
    submitted_at_unix: u64,
) {
    coordinator
        .workplane_store
        .with_connection(|conn| {
            let tx = conn.unchecked_transaction()?;
            {
                let mut stmt = tx.prepare(
                    "INSERT INTO capability_workplane_jobs (
                         job_id, repo_id, repo_root, config_root, capability_id, mailbox_name,
                         dedupe_key, payload, status, attempts, available_at_unix, submitted_at_unix,
                         started_at_unix, updated_at_unix, completed_at_unix, lease_owner,
                         lease_expires_at_unix, last_error
                    ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 0, ?10, ?11, NULL, ?12, NULL, NULL, NULL, NULL)",
                )?;
                for index in 0..count {
                    let artefact_id = format!("artefact-{index}");
                    stmt.execute(rusqlite::params![
                        format!("bulk-job-{mailbox_name}-{index}"),
                        repo_id,
                        target.repo_root.to_string_lossy().to_string(),
                        target.config_root.to_string_lossy().to_string(),
                        SEMANTIC_CLONES_CAPABILITY_ID,
                        mailbox_name,
                        format!("{mailbox_name}:{artefact_id}"),
                        serde_json::to_string(
                            &crate::capability_packs::semantic_clones::workplane::SemanticClonesMailboxPayload::Artefact { artefact_id }
                        )
                        .expect("serialize bulk artefact payload"),
                        WorkplaneJobStatus::Pending.as_str(),
                        sql_i64(submitted_at_unix)?,
                        sql_i64(submitted_at_unix)?,
                        sql_i64(submitted_at_unix)?,
                    ])?;
                }
            }
            tx.commit()?;
            Ok::<_, anyhow::Error>(())
        })
        .expect("insert bulk workplane jobs");
}

fn set_workplane_job_schedule(
    coordinator: &EnrichmentCoordinator,
    job_id: &str,
    available_at_unix: u64,
    submitted_at_unix: u64,
) {
    coordinator
        .workplane_store
        .with_connection(|conn| {
            conn.execute(
                "UPDATE capability_workplane_jobs
                 SET available_at_unix = ?1,
                     submitted_at_unix = ?2,
                     updated_at_unix = ?3
                 WHERE job_id = ?4",
                rusqlite::params![
                    sql_i64(available_at_unix)?,
                    sql_i64(submitted_at_unix)?,
                    sql_i64(submitted_at_unix)?,
                    job_id,
                ],
            )
            .map(|_| ())
            .map_err(anyhow::Error::from)
        })
        .expect("update workplane job schedule");
}

#[tokio::test]
async fn daemon_enrichment_worker_logs_terminal_failure() {
    let temp = TempDir::new().expect("temp dir");
    let (coordinator, target, repo_id) = new_test_coordinator(&temp);
    let job = WorkplaneJobRecord {
        job_id: "job-terminal-failure".to_string(),
        repo_id,
        repo_root: target.repo_root.clone(),
        config_root: target.config_root.clone(),
        capability_id: SEMANTIC_CLONES_CAPABILITY_ID.to_string(),
        mailbox_name: SEMANTIC_CLONES_SUMMARY_REFRESH_MAILBOX.to_string(),
        init_session_id: None,
        dedupe_key: None,
        payload: serde_json::json!({}),
        status: WorkplaneJobStatus::Running,
        attempts: 1,
        available_at_unix: 1,
        submitted_at_unix: 1,
        started_at_unix: Some(1),
        updated_at_unix: 1,
        completed_at_unix: None,
        lease_owner: None,
        lease_expires_at_unix: None,
        last_error: None,
    };
    let outcome = JobExecutionOutcome::failed(anyhow::anyhow!("simulated enrichment failure"));

    let (result, logs) = capture_logs(|| {
        persist_workplane_job_completion(&coordinator.workplane_store, &job, &outcome)
    });

    result.expect("persist workplane completion");
    assert!(
        logs.iter().any(|entry| entry.level == log::Level::Error
            && entry.message.contains("daemon enrichment job failed")),
        "expected terminal enrichment failure log, got logs: {logs:?}"
    );
}

#[tokio::test]
async fn enqueue_clone_edges_rebuild_waits_for_embedding_and_semantic_jobs_to_drain() {
    let temp = TempDir::new().expect("temp dir");
    let (coordinator, target, repo_id) = new_test_coordinator(&temp);

    insert_workplane_job(
        &coordinator,
        &target,
        WorkplaneJobFixture {
            repo_id: &repo_id,
            mailbox_name: SEMANTIC_CLONES_SUMMARY_REFRESH_MAILBOX,
            status: WorkplaneJobStatus::Pending,
            artefact_id: Some("artefact-semantic-a"),
            job_id: "semantic-a",
            updated_at_unix: 1,
            attempts: 0,
            last_error: None,
        },
    );
    insert_workplane_job(
        &coordinator,
        &target,
        WorkplaneJobFixture {
            repo_id: &repo_id,
            mailbox_name: SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX,
            status: WorkplaneJobStatus::Pending,
            artefact_id: Some("artefact-embedding-a"),
            job_id: "embedding-a",
            updated_at_unix: 1,
            attempts: 0,
            last_error: None,
        },
    );
    insert_workplane_job(
        &coordinator,
        &target,
        WorkplaneJobFixture {
            repo_id: &repo_id,
            mailbox_name: SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX,
            status: WorkplaneJobStatus::Running,
            artefact_id: Some("artefact-embedding-b"),
            job_id: "embedding-b",
            updated_at_unix: 1,
            attempts: 1,
            last_error: None,
        },
    );

    coordinator
        .enqueue_clone_edges_rebuild(target.clone())
        .await
        .expect("enqueue coalesced clone rebuild request");

    let enqueued_state = load_workplane_jobs(&coordinator, WorkplaneJobStatus::Pending);
    assert_eq!(
        enqueued_state
            .iter()
            .filter(|job| job.mailbox_name == SEMANTIC_CLONES_CLONE_REBUILD_MAILBOX)
            .count(),
        1
    );

    coordinator
        .enqueue_clone_edges_rebuild(target)
        .await
        .expect("dedupe clone rebuild jobs");

    let deduped_state = load_workplane_jobs(&coordinator, WorkplaneJobStatus::Pending);
    assert_eq!(
        deduped_state
            .iter()
            .filter(|job| job.mailbox_name == SEMANTIC_CLONES_CLONE_REBUILD_MAILBOX)
            .count(),
        1
    );
}

#[tokio::test]
async fn enqueue_symbol_embeddings_splits_large_batches_into_smaller_jobs() {
    let temp = TempDir::new().expect("temp dir");
    let (coordinator, target, _repo_id) = new_test_coordinator(&temp);
    let inputs = (0..(MAX_SEMANTIC_ENRICHMENT_JOB_ARTEFACTS + 1))
        .map(|index| sample_input_with_artefact_id(&format!("artefact-{index}")))
        .collect::<Vec<_>>();
    let input_count = inputs.len();
    let input_hashes = inputs
        .iter()
        .map(|input| {
            (
                input.artefact_id.clone(),
                format!("hash-{}", input.artefact_id),
            )
        })
        .collect::<BTreeMap<_, _>>();

    coordinator
        .enqueue_symbol_embeddings(
            target,
            inputs,
            input_hashes,
            EmbeddingRepresentationKind::Code,
        )
        .await
        .expect("enqueue embedding jobs");

    let embedding_jobs =
        load_embedding_mailbox_items(&coordinator, SemanticMailboxItemStatus::Pending)
            .iter()
            .filter(|item| {
                item.representation_kind == EmbeddingRepresentationKind::Code.to_string()
            })
            .filter_map(|item| item.artefact_id.clone())
            .collect::<Vec<_>>();

    assert_eq!(embedding_jobs.len(), input_count);
    assert!(
        embedding_jobs
            .iter()
            .all(|artefact_id| !artefact_id.is_empty())
    );
}

#[tokio::test]
async fn enqueue_semantic_summaries_keeps_larger_semantic_batches() {
    let temp = TempDir::new().expect("temp dir");
    let (coordinator, target, _repo_id) = new_test_coordinator(&temp);
    configure_summary_refresh_for_repo(&target);
    let inputs = (0..(MAX_SEMANTIC_ENRICHMENT_JOB_ARTEFACTS + 1))
        .map(|index| sample_input_with_artefact_id(&format!("artefact-{index}")))
        .collect::<Vec<_>>();
    let input_hashes = inputs
        .iter()
        .map(|input| {
            (
                input.artefact_id.clone(),
                format!("hash-{}", input.artefact_id),
            )
        })
        .collect::<BTreeMap<_, _>>();

    coordinator
        .enqueue_semantic_summaries(target, inputs, input_hashes)
        .await
        .expect("enqueue semantic jobs");

    let semantic_jobs =
        load_summary_mailbox_items(&coordinator, SemanticMailboxItemStatus::Pending)
            .iter()
            .filter_map(|item| item.artefact_id.clone())
            .collect::<Vec<_>>();

    assert_eq!(
        semantic_jobs.len(),
        MAX_SEMANTIC_ENRICHMENT_JOB_ARTEFACTS + 1
    );
    assert!(
        semantic_jobs
            .iter()
            .all(|artefact_id| !artefact_id.is_empty())
    );
}

#[tokio::test]
async fn enqueue_repo_backfill_embedding_jobs_chunks_large_payloads_for_parallel_workers() {
    let temp = TempDir::new().expect("temp dir");
    let (coordinator, target, _repo_id) = new_test_coordinator(&temp);
    let artefact_ids = (0..55)
        .map(|index| format!("artefact-{index:03}"))
        .collect::<Vec<_>>();

    coordinator
        .enqueue_follow_up(FollowUpJob::RepoBackfillEmbeddings {
            target,
            artefact_ids,
            representation_kind: EmbeddingRepresentationKind::Summary,
        })
        .await
        .expect("enqueue chunked repo backfill embedding jobs");

    let pending_items =
        load_embedding_mailbox_items(&coordinator, SemanticMailboxItemStatus::Pending)
            .into_iter()
            .filter(|item| {
                item.representation_kind == EmbeddingRepresentationKind::Summary.to_string()
            })
            .collect::<Vec<_>>();

    assert_eq!(pending_items.len(), 2);
    assert!(pending_items.iter().all(|item| {
        item.item_kind == SemanticMailboxItemKind::RepoBackfill
            && item
                .payload_json
                .as_ref()
                .and_then(serde_json::Value::as_array)
                .is_some_and(|artefact_ids| artefact_ids.len() <= 50)
    }));
    assert_eq!(
        pending_items
            .iter()
            .map(|item| {
                item.payload_json
                    .as_ref()
                    .and_then(serde_json::Value::as_array)
                    .map(|artefact_ids| artefact_ids.len() as u64)
                    .unwrap_or_default()
            })
            .sum::<u64>(),
        55
    );
}

#[test]
fn requeue_running_jobs_moves_stale_running_jobs_back_to_pending() {
    let temp = TempDir::new().expect("temp dir");
    let (coordinator, target, repo_id) = new_test_coordinator(&temp);

    coordinator
        .runtime_store
        .save_enrichment_queue_state(&default_state())
        .expect("write initial control state");
    insert_summary_mailbox_item(
        &coordinator,
        &target,
        SummaryMailboxItemFixture {
            repo_id: &repo_id,
            item_id: "semantic-a",
            status: SemanticMailboxItemStatus::Leased,
            item_kind: SemanticMailboxItemKind::Artefact,
            artefact_id: Some("artefact-semantic-a"),
            payload_json: None,
            submitted_at_unix: 1,
            updated_at_unix: 1,
            attempts: 1,
            lease_token: Some("lease-summary-a"),
            lease_expires_at_unix: Some(60),
            last_error: None,
        },
    );
    insert_embedding_mailbox_item(
        &coordinator,
        &target,
        EmbeddingMailboxItemFixture {
            repo_id: &repo_id,
            item_id: "embedding-a",
            representation_kind: "code",
            status: SemanticMailboxItemStatus::Leased,
            item_kind: SemanticMailboxItemKind::Artefact,
            artefact_id: Some("artefact-embedding-a"),
            payload_json: None,
            submitted_at_unix: 1,
            updated_at_unix: 1,
            attempts: 1,
            lease_token: Some("lease-embedding-a"),
            lease_expires_at_unix: Some(60),
            last_error: None,
        },
    );
    insert_embedding_mailbox_item(
        &coordinator,
        &target,
        EmbeddingMailboxItemFixture {
            repo_id: &repo_id,
            item_id: "embedding-b",
            representation_kind: "code",
            status: SemanticMailboxItemStatus::Pending,
            item_kind: SemanticMailboxItemKind::Artefact,
            artefact_id: Some("artefact-embedding-b"),
            payload_json: None,
            submitted_at_unix: 1,
            updated_at_unix: 1,
            attempts: 0,
            lease_token: None,
            lease_expires_at_unix: None,
            last_error: None,
        },
    );

    coordinator.requeue_running_jobs();

    let recovered_summary_leased =
        load_summary_mailbox_items(&coordinator, SemanticMailboxItemStatus::Leased);
    let recovered_summary_pending =
        load_summary_mailbox_items(&coordinator, SemanticMailboxItemStatus::Pending);
    let recovered_embedding_leased =
        load_embedding_mailbox_items(&coordinator, SemanticMailboxItemStatus::Leased);
    let recovered_embedding_pending =
        load_embedding_mailbox_items(&coordinator, SemanticMailboxItemStatus::Pending);
    assert_eq!(recovered_summary_leased.len(), 0);
    assert_eq!(recovered_embedding_leased.len(), 0);
    assert_eq!(recovered_summary_pending.len(), 1);
    assert_eq!(recovered_embedding_pending.len(), 2);
    assert_eq!(
        coordinator
            .runtime_store
            .load_enrichment_queue_state()
            .expect("read recovered control state")
            .expect("state exists")
            .last_action
            .as_deref(),
        Some("requeue_running")
    );
}

#[test]
fn ensure_started_recovers_stale_running_jobs_on_startup() {
    let temp = TempDir::new().expect("temp dir");
    let (coordinator, target, repo_id) = new_test_coordinator(&temp);
    let coordinator = Arc::new(coordinator);
    configure_summary_refresh_for_repo(&target);

    coordinator
        .runtime_store
        .save_enrichment_queue_state(&default_state())
        .expect("write initial control state");
    insert_summary_mailbox_item(
        &coordinator,
        &target,
        SummaryMailboxItemFixture {
            repo_id: &repo_id,
            item_id: "semantic-a",
            status: SemanticMailboxItemStatus::Leased,
            item_kind: SemanticMailboxItemKind::Artefact,
            artefact_id: Some("artefact-semantic-a"),
            payload_json: None,
            submitted_at_unix: 1,
            updated_at_unix: 1,
            attempts: 1,
            lease_token: Some("lease-summary-a"),
            lease_expires_at_unix: Some(60),
            last_error: None,
        },
    );
    insert_embedding_mailbox_item(
        &coordinator,
        &target,
        EmbeddingMailboxItemFixture {
            repo_id: &repo_id,
            item_id: "embedding-a",
            representation_kind: "code",
            status: SemanticMailboxItemStatus::Leased,
            item_kind: SemanticMailboxItemKind::Artefact,
            artefact_id: Some("artefact-embedding-a"),
            payload_json: None,
            submitted_at_unix: 1,
            updated_at_unix: 1,
            attempts: 1,
            lease_token: Some("lease-embedding-a"),
            lease_expires_at_unix: Some(60),
            last_error: None,
        },
    );
    insert_embedding_mailbox_item(
        &coordinator,
        &target,
        EmbeddingMailboxItemFixture {
            repo_id: &repo_id,
            item_id: "embedding-b",
            representation_kind: "code",
            status: SemanticMailboxItemStatus::Pending,
            item_kind: SemanticMailboxItemKind::Artefact,
            artefact_id: Some("artefact-embedding-b"),
            payload_json: None,
            submitted_at_unix: 1,
            updated_at_unix: 1,
            attempts: 0,
            lease_token: None,
            lease_expires_at_unix: None,
            last_error: None,
        },
    );

    coordinator.ensure_started();

    let recovered_summary_leased =
        load_summary_mailbox_items(&coordinator, SemanticMailboxItemStatus::Leased);
    let recovered_summary_pending =
        load_summary_mailbox_items(&coordinator, SemanticMailboxItemStatus::Pending);
    let recovered_embedding_leased =
        load_embedding_mailbox_items(&coordinator, SemanticMailboxItemStatus::Leased);
    let recovered_embedding_pending =
        load_embedding_mailbox_items(&coordinator, SemanticMailboxItemStatus::Pending);
    assert_eq!(recovered_summary_leased.len(), 0);
    assert_eq!(recovered_embedding_leased.len(), 0);
    assert_eq!(recovered_summary_pending.len(), 1);
    assert_eq!(recovered_embedding_pending.len(), 2);
    assert_eq!(
        coordinator
            .runtime_store
            .load_enrichment_queue_state()
            .expect("read recovered control state")
            .expect("state exists")
            .last_action
            .as_deref(),
        Some("requeue_running")
    );
}

#[test]
fn ensure_started_logs_missing_runtime_activation_error() {
    let temp = TempDir::new().expect("temp dir");
    let (coordinator, target, _repo_id) = new_test_coordinator(&temp);
    let coordinator = Arc::new(coordinator);
    configure_summary_refresh_for_repo(&target);

    let (_, logs) = capture_logs(|| coordinator.ensure_started());

    assert!(logs.iter().any(|entry| {
        entry.level == log::Level::Error
            && entry
                .message
                .contains("enrichment worker activation requested without an active tokio runtime")
    }));
}

#[test]
fn snapshot_projects_last_failed_embedding_job_details() {
    let temp = TempDir::new().expect("temp dir");
    let (coordinator, target, repo_id) = new_test_coordinator(&temp);
    insert_workplane_job(
        &coordinator,
        &target,
        WorkplaneJobFixture {
            repo_id: &repo_id,
            mailbox_name: SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX,
            status: WorkplaneJobStatus::Failed,
            artefact_id: Some("artefact-older"),
            job_id: "embedding-older",
            updated_at_unix: 10,
            attempts: 1,
            last_error: Some("older failure"),
        },
    );
    insert_workplane_job(
        &coordinator,
        &target,
        WorkplaneJobFixture {
            repo_id: &repo_id,
            mailbox_name: SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX,
            status: WorkplaneJobStatus::Failed,
            artefact_id: Some("artefact-newer"),
            job_id: "embedding-newer",
            updated_at_unix: 20,
            attempts: 3,
            last_error: Some("[capability_host:timeout] capability ingester timed out after 300s"),
        },
    );

    let summary = super::last_failed_embedding_job_from_workplane(&coordinator.workplane_store)
        .expect("read failed embedding summary")
        .expect("failed embedding summary");
    assert_eq!(summary.job_id, "embedding-newer");
    assert_eq!(summary.repo_id, repo_id);
    assert_eq!(summary.branch, "unknown");
    assert_eq!(summary.representation_kind, "code");
    assert_eq!(summary.artefact_count, 1);
    assert_eq!(summary.attempts, 3);
    assert_eq!(
        summary.error.as_deref(),
        Some("[capability_host:timeout] capability ingester timed out after 300s")
    );
    assert_eq!(summary.updated_at_unix, 20);
}

#[test]
fn compaction_replaces_large_old_pending_embedding_backlog_with_repo_backfill_job() {
    let temp = TempDir::new().expect("temp dir");
    let (coordinator, target, repo_id) = new_test_coordinator(&temp);
    let pending_count = usize::try_from(WORKPLANE_PENDING_COMPACTION_MIN_COUNT)
        .expect("pending compaction threshold fits usize");
    insert_pending_artefact_jobs_bulk(
        &coordinator,
        &target,
        &repo_id,
        SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX,
        pending_count,
        1,
    );

    super::compact_and_prune_workplane_jobs(&coordinator.workplane_store)
        .expect("compact pending workplane backlog");

    let pending_jobs = load_workplane_jobs(&coordinator, WorkplaneJobStatus::Pending)
        .into_iter()
        .filter(|job| job.mailbox_name == SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX)
        .collect::<Vec<_>>();

    assert_eq!(
        pending_jobs.len(),
        pending_count,
        "semantic backlog compaction is removed from the hot path",
    );
    assert!(
        pending_jobs.iter().all(|job| {
            !crate::capability_packs::semantic_clones::workplane::payload_is_repo_backfill(
                &job.payload,
            )
        }),
        "legacy semantic maintenance should not rewrite pending embedding jobs into repo-backfill rows",
    );
}

#[test]
fn compaction_prunes_pending_summary_refresh_jobs_when_summary_provider_is_unconfigured() {
    let temp = TempDir::new().expect("temp dir");
    let (coordinator, target, repo_id) = new_test_coordinator(&temp);
    let config_path =
        crate::test_support::git_fixtures::write_test_daemon_config(&target.config_root);
    crate::config::settings::write_repo_daemon_binding(
        &target
            .repo_root
            .join(crate::config::REPO_POLICY_LOCAL_FILE_NAME),
        &config_path,
    )
    .expect("bind repo root to config");
    crate::capability_packs::semantic_clones::workplane::activate_deferred_pipeline_mailboxes(
        &target.repo_root,
        "init",
    )
    .expect("activate deferred mailboxes");

    insert_workplane_job(
        &coordinator,
        &target,
        WorkplaneJobFixture {
            repo_id: &repo_id,
            mailbox_name: SEMANTIC_CLONES_SUMMARY_REFRESH_MAILBOX,
            status: WorkplaneJobStatus::Pending,
            artefact_id: Some("artefact-semantic-a"),
            job_id: "semantic-a",
            updated_at_unix: 1,
            attempts: 0,
            last_error: None,
        },
    );
    insert_workplane_job(
        &coordinator,
        &target,
        WorkplaneJobFixture {
            repo_id: &repo_id,
            mailbox_name: SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX,
            status: WorkplaneJobStatus::Pending,
            artefact_id: Some("artefact-embedding-a"),
            job_id: "embedding-a",
            updated_at_unix: 1,
            attempts: 0,
            last_error: None,
        },
    );

    super::compact_and_prune_workplane_jobs(&coordinator.workplane_store)
        .expect("prune inactive summary refresh jobs");

    let pending_jobs = load_workplane_jobs(&coordinator, WorkplaneJobStatus::Pending);
    assert_eq!(
        pending_jobs
            .iter()
            .filter(|job| job.mailbox_name == SEMANTIC_CLONES_SUMMARY_REFRESH_MAILBOX)
            .count(),
        1,
        "semantic maintenance no longer prunes pending summary refresh work",
    );
    assert_eq!(
        pending_jobs
            .iter()
            .filter(|job| job.mailbox_name == SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX)
            .count(),
        1,
        "other pending work should be preserved"
    );
}

#[tokio::test]
async fn enqueue_does_not_compact_large_pending_backlog() {
    let temp = TempDir::new().expect("temp dir");
    let (coordinator, target, repo_id) = new_test_coordinator(&temp);
    let pending_count = usize::try_from(WORKPLANE_PENDING_COMPACTION_MIN_COUNT)
        .expect("pending compaction threshold fits usize");
    insert_pending_artefact_jobs_bulk(
        &coordinator,
        &target,
        &repo_id,
        SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX,
        pending_count,
        1,
    );

    coordinator
        .enqueue_clone_edges_rebuild(target.clone())
        .await
        .expect("enqueue clone rebuild job");

    let pending_jobs = load_workplane_jobs(&coordinator, WorkplaneJobStatus::Pending);
    let embedding_jobs = pending_jobs
        .iter()
        .filter(|job| job.mailbox_name == SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX)
        .collect::<Vec<_>>();

    assert_eq!(embedding_jobs.len(), pending_count);
    assert!(
        embedding_jobs.iter().all(|job| {
            !crate::capability_packs::semantic_clones::workplane::payload_is_repo_backfill(
                &job.payload,
            )
        }),
        "enqueue should not compact a pending artefact backlog on the hot path",
    );
    assert_eq!(
        pending_jobs
            .iter()
            .filter(|job| job.mailbox_name == SEMANTIC_CLONES_CLONE_REBUILD_MAILBOX)
            .count(),
        1,
    );
}

#[test]
fn claim_does_not_compact_large_pending_backlog() {
    let temp = TempDir::new().expect("temp dir");
    let (coordinator, target, repo_id) = new_test_coordinator(&temp);
    configure_summary_refresh_for_repo(&target);
    let pending_count = usize::try_from(WORKPLANE_PENDING_COMPACTION_MIN_COUNT)
        .expect("pending compaction threshold fits usize");
    insert_pending_artefact_jobs_bulk(
        &coordinator,
        &target,
        &repo_id,
        SEMANTIC_CLONES_SUMMARY_REFRESH_MAILBOX,
        pending_count,
        1,
    );

    let claimed = claim_next_workplane_job(
        &coordinator.workplane_store,
        &coordinator.runtime_store,
        &default_state(),
        super::worker_count::EnrichmentWorkerPool::SummaryRefresh,
    )
    .expect("claim summary refresh job")
    .expect("summary refresh job should be claimable");

    assert_eq!(
        claimed.mailbox_name,
        SEMANTIC_CLONES_SUMMARY_REFRESH_MAILBOX
    );
    let pending_jobs = load_workplane_jobs(&coordinator, WorkplaneJobStatus::Pending)
        .into_iter()
        .filter(|job| job.mailbox_name == SEMANTIC_CLONES_SUMMARY_REFRESH_MAILBOX)
        .collect::<Vec<_>>();
    assert_eq!(pending_jobs.len(), pending_count - 1);
    assert!(
        pending_jobs.iter().all(|job| {
            !crate::capability_packs::semantic_clones::workplane::payload_is_repo_backfill(
                &job.payload,
            )
        }),
        "claiming should not compact a pending artefact backlog on the hot path",
    );
}

#[test]
fn ensure_started_compacts_large_pending_backlog_on_startup() {
    let temp = TempDir::new().expect("temp dir");
    let (coordinator, target, repo_id) = new_test_coordinator(&temp);
    let coordinator = Arc::new(coordinator);
    let pending_count = usize::try_from(WORKPLANE_PENDING_COMPACTION_MIN_COUNT)
        .expect("pending compaction threshold fits usize");
    insert_pending_artefact_jobs_bulk(
        &coordinator,
        &target,
        &repo_id,
        SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX,
        pending_count,
        1,
    );

    coordinator.ensure_started();

    let pending_jobs = load_workplane_jobs(&coordinator, WorkplaneJobStatus::Pending)
        .into_iter()
        .filter(|job| job.mailbox_name == SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX)
        .collect::<Vec<_>>();
    assert_eq!(
        pending_jobs.len(),
        0,
        "startup should migrate legacy semantic rows out of the workplane table",
    );
    assert_eq!(
        load_embedding_mailbox_items(&coordinator, SemanticMailboxItemStatus::Pending).len(),
        pending_count,
        "startup should seed the embedding inbox with the legacy pending workload",
    );
}

#[test]
fn retry_failed_jobs_runs_maintenance() {
    let temp = TempDir::new().expect("temp dir");
    let (coordinator, target, repo_id) = new_test_coordinator(&temp);
    let pending_count = usize::try_from(WORKPLANE_PENDING_COMPACTION_MIN_COUNT)
        .expect("pending compaction threshold fits usize");
    insert_pending_artefact_jobs_bulk(
        &coordinator,
        &target,
        &repo_id,
        SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX,
        pending_count,
        1,
    );
    insert_workplane_job(
        &coordinator,
        &target,
        WorkplaneJobFixture {
            repo_id: &repo_id,
            mailbox_name: SEMANTIC_CLONES_CLONE_REBUILD_MAILBOX,
            status: WorkplaneJobStatus::Failed,
            artefact_id: None,
            job_id: "clone-failed",
            updated_at_unix: 10,
            attempts: 2,
            last_error: Some("boom"),
        },
    );

    let retried =
        super::retry_failed_jobs_in_store(&coordinator.workplane_store).expect("retry failed jobs");

    assert_eq!(retried, 1);
    let pending_jobs = load_workplane_jobs(&coordinator, WorkplaneJobStatus::Pending);
    assert_eq!(
        pending_jobs
            .iter()
            .filter(|job| job.mailbox_name == SEMANTIC_CLONES_CLONE_REBUILD_MAILBOX)
            .count(),
        1,
    );
    let embedding_jobs = pending_jobs
        .iter()
        .filter(|job| job.mailbox_name == SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX)
        .collect::<Vec<_>>();
    assert_eq!(
        embedding_jobs.len(),
        0,
        "retry should migrate legacy semantic rows out of the workplane table",
    );
    assert_eq!(
        load_embedding_mailbox_items(&coordinator, SemanticMailboxItemStatus::Pending).len(),
        pending_count,
        "retry maintenance should seed the embedding inbox instead of compacting the legacy backlog",
    );
}

#[tokio::test(start_paused = true, flavor = "current_thread")]
async fn periodic_maintenance_runs_on_the_sixty_second_tick() {
    let temp = TempDir::new().expect("temp dir");
    let (coordinator, target, repo_id) = new_test_coordinator(&temp);
    if let Ok(mut counts) = coordinator.started_worker_counts.lock() {
        *counts = super::worker_count::EnrichmentWorkerBudgets {
            summary_refresh: 32,
            embeddings: 32,
            clone_rebuild: 32,
        };
    }
    let coordinator = Arc::new(coordinator);
    coordinator.ensure_started();
    tokio::task::yield_now().await;

    insert_embedding_mailbox_item(
        &coordinator,
        &target,
        EmbeddingMailboxItemFixture {
            repo_id: &repo_id,
            item_id: "expired-embedding-lease",
            representation_kind: "code",
            status: SemanticMailboxItemStatus::Leased,
            item_kind: SemanticMailboxItemKind::Artefact,
            artefact_id: Some("code-a"),
            payload_json: None,
            submitted_at_unix: 1,
            updated_at_unix: 1,
            attempts: 1,
            lease_token: Some("expired-embedding-lease"),
            lease_expires_at_unix: Some(1),
            last_error: None,
        },
    );

    let load_embedding_jobs =
        || load_embedding_mailbox_items(&coordinator, SemanticMailboxItemStatus::Pending);

    assert_eq!(
        load_embedding_mailbox_items(&coordinator, SemanticMailboxItemStatus::Leased).len(),
        1,
    );
    assert_eq!(load_embedding_jobs().len(), 0);

    advance(Duration::from_secs(59)).await;
    tokio::task::yield_now().await;
    assert_eq!(load_embedding_jobs().len(), 0);

    advance(Duration::from_secs(1)).await;
    tokio::task::yield_now().await;
    tokio::task::yield_now().await;

    let embedding_jobs = load_embedding_jobs();
    assert_eq!(embedding_jobs.len(), 1);
    assert_eq!(embedding_jobs[0].artefact_id.as_deref(), Some("code-a"));
    assert!(embedding_jobs[0].lease_token.is_none());
}

#[test]
fn retry_failed_jobs_requeues_historical_repo_backfill_payloads() {
    let temp = TempDir::new().expect("temp dir");
    let (coordinator, target, repo_id) = new_test_coordinator(&temp);
    let artefact_ids = (0..40)
        .map(|index| format!("artefact-{index}"))
        .collect::<Vec<_>>();
    let payload = serde_json::to_string(
        &crate::capability_packs::semantic_clones::workplane::SemanticClonesMailboxPayload::RepoBackfill {
            work_item_count: Some(artefact_ids.len() as u64),
            artefact_ids: Some(artefact_ids.clone()),
        },
    )
    .expect("serialize repo backfill payload");
    let dedupe_key = crate::capability_packs::semantic_clones::workplane::repo_backfill_dedupe_key(
        SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX,
    );
    coordinator
        .workplane_store
        .with_connection(|conn| {
            conn.execute(
                "INSERT INTO capability_workplane_jobs (
                     job_id, repo_id, repo_root, config_root, capability_id, mailbox_name,
                     dedupe_key, payload, status, attempts, available_at_unix, submitted_at_unix,
                     started_at_unix, updated_at_unix, completed_at_unix, lease_owner,
                     lease_expires_at_unix, last_error
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, NULL, NULL, ?16)",
                rusqlite::params![
                    "failed-backfill",
                    repo_id,
                    target.repo_root.to_string_lossy().to_string(),
                    target.config_root.to_string_lossy().to_string(),
                    SEMANTIC_CLONES_CAPABILITY_ID,
                    SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX,
                    dedupe_key,
                    payload,
                    WorkplaneJobStatus::Failed.as_str(),
                    2u32,
                    sql_i64(1)?,
                    sql_i64(1)?,
                    sql_i64(1)?,
                    sql_i64(1)?,
                    sql_i64(1)?,
                    "timeout",
                ],
            )
            .map(|_| ())
            .map_err(anyhow::Error::from)
        })
        .expect("insert failed repo backfill job");

    let retried =
        super::retry_failed_jobs_in_store(&coordinator.workplane_store).expect("retry failed jobs");

    assert_eq!(retried, 1);
    assert!(
        load_workplane_jobs(&coordinator, WorkplaneJobStatus::Pending)
            .into_iter()
            .all(|job| job.mailbox_name != SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX),
        "retried legacy embedding rows should be migrated into the embedding inbox",
    );
    let pending_items =
        load_embedding_mailbox_items(&coordinator, SemanticMailboxItemStatus::Pending);
    assert_eq!(pending_items.len(), 1);
    let requeued_artefact_ids = pending_items[0]
        .payload_json
        .as_ref()
        .and_then(serde_json::Value::as_array)
        .expect("explicit artefact ids should be preserved for retried repo backfill jobs");
    assert_eq!(requeued_artefact_ids.len(), 40);
    assert_eq!(
        requeued_artefact_ids
            .first()
            .and_then(serde_json::Value::as_str),
        Some("artefact-0")
    );
    assert_eq!(
        requeued_artefact_ids
            .last()
            .and_then(serde_json::Value::as_str),
        Some("artefact-39")
    );
}

#[test]
fn transient_embedding_timeout_is_requeued_with_backoff() {
    let temp = TempDir::new().expect("temp dir");
    let (coordinator, target, repo_id) = new_test_coordinator(&temp);

    insert_workplane_job(
        &coordinator,
        &target,
        WorkplaneJobFixture {
            repo_id: &repo_id,
            mailbox_name: SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX,
            status: WorkplaneJobStatus::Running,
            artefact_id: Some("code-timeout"),
            job_id: "code-timeout",
            updated_at_unix: 10,
            attempts: 1,
            last_error: None,
        },
    );
    let running_job = load_workplane_jobs(&coordinator, WorkplaneJobStatus::Running)
        .into_iter()
        .find(|job| job.job_id == "code-timeout")
        .expect("running embedding job");

    let before = unix_timestamp_now();
    let outcome = JobExecutionOutcome::failed(anyhow::anyhow!(
        "[capability_host:timeout] capability ingester timed out after 300s"
    ));
    let disposition = super::workplane::persist_workplane_job_completion(
        &coordinator.workplane_store,
        &running_job,
        &outcome,
    )
    .expect("persist retryable timeout");

    match disposition {
        super::workplane::WorkplaneJobCompletionDisposition::RetryScheduled {
            available_at_unix,
            retry_in_secs,
        } => {
            assert_eq!(retry_in_secs, 5);
            assert!(
                available_at_unix >= before + retry_in_secs,
                "retry should be scheduled in the future"
            );
        }
        other => panic!("expected retry disposition, got {other:?}"),
    }

    let pending_job = load_workplane_jobs(&coordinator, WorkplaneJobStatus::Pending)
        .into_iter()
        .find(|job| job.job_id == "code-timeout")
        .expect("requeued embedding job");
    assert_eq!(pending_job.last_error.as_deref(), outcome.error.as_deref());
    assert!(pending_job.started_at_unix.is_none());
    assert!(pending_job.completed_at_unix.is_none());
}

#[test]
fn embedding_timeout_at_retry_limit_stays_failed() {
    let temp = TempDir::new().expect("temp dir");
    let (coordinator, target, repo_id) = new_test_coordinator(&temp);

    insert_workplane_job(
        &coordinator,
        &target,
        WorkplaneJobFixture {
            repo_id: &repo_id,
            mailbox_name: SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX,
            status: WorkplaneJobStatus::Running,
            artefact_id: Some("code-timeout-terminal"),
            job_id: "code-timeout-terminal",
            updated_at_unix: 10,
            attempts: 3,
            last_error: None,
        },
    );
    let running_job = load_workplane_jobs(&coordinator, WorkplaneJobStatus::Running)
        .into_iter()
        .find(|job| job.job_id == "code-timeout-terminal")
        .expect("running embedding job");

    let outcome = JobExecutionOutcome::failed(anyhow::anyhow!(
        "[capability_host:timeout] capability ingester timed out after 300s"
    ));
    let disposition = super::workplane::persist_workplane_job_completion(
        &coordinator.workplane_store,
        &running_job,
        &outcome,
    )
    .expect("persist terminal timeout");

    assert_eq!(
        disposition,
        super::workplane::WorkplaneJobCompletionDisposition::Failed
    );
    let failed_job = load_workplane_jobs(&coordinator, WorkplaneJobStatus::Failed)
        .into_iter()
        .find(|job| job.job_id == "code-timeout-terminal")
        .expect("failed embedding job");
    assert_eq!(failed_job.last_error.as_deref(), outcome.error.as_deref());
    assert!(failed_job.completed_at_unix.is_some());
}

#[test]
fn workplane_completion_log_emits_queue_wait_and_run_durations() {
    let job = WorkplaneJobRecord {
        job_id: "job-1".to_string(),
        repo_id: "repo-1".to_string(),
        repo_root: PathBuf::from("/tmp/repo-1"),
        config_root: PathBuf::from("/tmp/config-1"),
        capability_id: SEMANTIC_CLONES_CAPABILITY_ID.to_string(),
        mailbox_name: SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX.to_string(),
        init_session_id: None,
        dedupe_key: Some("semantic_clones.embedding.code:artefact-1".to_string()),
        payload: serde_json::to_value(
            crate::capability_packs::semantic_clones::workplane::SemanticClonesMailboxPayload::RepoBackfill {
                work_item_count: Some(4),
                artefact_ids: Some(vec![
                    "artefact-1".to_string(),
                    "artefact-2".to_string(),
                    "artefact-3".to_string(),
                    "artefact-4".to_string(),
                ]),
            },
        )
        .expect("serialize repo backfill payload"),
        status: WorkplaneJobStatus::Running,
        attempts: 3,
        available_at_unix: 5,
        submitted_at_unix: 10,
        started_at_unix: Some(25),
        updated_at_unix: 25,
        completed_at_unix: None,
        lease_owner: None,
        lease_expires_at_unix: None,
        last_error: None,
    };

    let log_line =
        super::workplane::format_workplane_job_completion_log(&job, 40, &JobExecutionOutcome::ok());

    assert!(log_line.contains("mailbox_name=semantic_clones.embedding.code"));
    assert!(log_line.contains("payload_work_item_count=4"));
    assert!(log_line.contains("queue_wait_secs=15"));
    assert!(log_line.contains("run_secs=15"));
    assert!(log_line.contains("attempts=3"));
    assert!(log_line.contains("outcome=completed"));
}

#[test]
fn summary_refresh_perf_distribution_uses_nearest_rank_percentiles() {
    let samples = [
        StdDuration::from_millis(10),
        StdDuration::from_millis(20),
        StdDuration::from_millis(30),
        StdDuration::from_millis(40),
    ];

    let summary = duration_distribution_summary(&samples);

    assert!((summary.mean_ms - 25.0).abs() < f64::EPSILON);
    assert_eq!(summary.p50, StdDuration::from_millis(20));
    assert_eq!(summary.p95, StdDuration::from_millis(40));
    assert_eq!(summary.p99, StdDuration::from_millis(40));
    assert_eq!(summary.max, StdDuration::from_millis(40));
}

#[test]
fn summary_refresh_perf_report_includes_percentiles_and_slowest_jobs() {
    let metrics = vec![
        SummaryRefreshPerfJobMetric {
            job_id: "job-a".to_string(),
            artefact_id: "artefact-a".to_string(),
            symbol_fqn: "src/a.rs::alpha".to_string(),
            queue_wait: StdDuration::from_millis(100),
            run: StdDuration::from_millis(300),
            end_to_end: StdDuration::from_millis(400),
        },
        SummaryRefreshPerfJobMetric {
            job_id: "job-b".to_string(),
            artefact_id: "artefact-b".to_string(),
            symbol_fqn: "src/b.rs::beta".to_string(),
            queue_wait: StdDuration::from_millis(200),
            run: StdDuration::from_millis(500),
            end_to_end: StdDuration::from_millis(700),
        },
    ];

    let report = render_summary_refresh_perf_report(
        &metrics,
        2,
        StdDuration::from_secs(1),
        StdDuration::from_secs(60),
    );

    assert!(report.contains("summary refresh perf report: jobs=2 workers=2"));
    assert!(report.contains("queue_wait: mean="));
    assert!(report.contains("run: mean="));
    assert!(report.contains("end_to_end: mean="));
    assert!(report.contains("slowest_jobs_by_run:"));
    assert!(report.contains("artefact=artefact-b"));
    assert!(report.contains("symbol=src/b.rs::beta"));
}

#[tokio::test]
#[ignore = "performance smoke test; run locally with `cargo nextest run -p bitloops --lib summary_refresh_perf_200_jobs_calls_actual_gateway --run-ignored ignored-only`"]
async fn summary_refresh_perf_200_jobs_calls_actual_gateway() {
    let _guard = performance_suite_lock().await;
    let temp = TempDir::new().expect("temp dir");
    let (coordinator, target, _repo_id) = new_test_coordinator(&temp);
    let runtime_command = match summary_refresh_perf_platform_prerequisites() {
        Ok(command) => command,
        Err(reason) => {
            eprintln!("skipping summary refresh actual-gateway perf run: {reason}");
            return;
        }
    };
    configure_platform_summary_refresh_for_repo(&target, &runtime_command);
    seed_summary_refresh_perf_repo(&target.repo_root, SUMMARY_REFRESH_PERF_JOB_COUNT);
    let latency_budget = latency_budget_from_env(
        SUMMARY_REFRESH_200_JOBS_BUDGET_ENV,
        DEFAULT_SUMMARY_REFRESH_200_JOBS_BUDGET_MS,
    );

    let inputs = load_summary_refresh_perf_inputs(&target).await;
    assert_eq!(
        inputs.len(),
        SUMMARY_REFRESH_PERF_JOB_COUNT,
        "expected exactly one summary candidate per generated Rust fixture file"
    );
    let input_hashes = summary_refresh_perf_input_hashes(&inputs);
    let symbol_lookup = Arc::new(summary_refresh_perf_symbol_lookup(&inputs));
    let worker_count = summary_refresh_perf_worker_count(&target);

    coordinator
        .enqueue_semantic_summaries(
            EnrichmentJobTarget::new(target.config_root.clone(), target.repo_root.clone()),
            inputs,
            input_hashes,
        )
        .await
        .expect("enqueue summary perf jobs");

    let enqueued_jobs = load_workplane_jobs(&coordinator, WorkplaneJobStatus::Pending)
        .into_iter()
        .filter(|job| job.mailbox_name == SEMANTIC_CLONES_SUMMARY_REFRESH_MAILBOX)
        .count();
    assert_eq!(
        enqueued_jobs, SUMMARY_REFRESH_PERF_JOB_COUNT,
        "expected one queued summary_refresh job per Rust fixture function"
    );

    let (metrics, elapsed) =
        run_summary_refresh_perf_jobs(&coordinator, worker_count, symbol_lookup)
            .await
            .expect("run summary refresh perf workers");
    let processed = metrics.len();

    assert_eq!(
        processed, SUMMARY_REFRESH_PERF_JOB_COUNT,
        "expected to process every queued summary_refresh job"
    );
    let report = print_summary_refresh_perf_report(&metrics, worker_count, elapsed, latency_budget);
    assert_latency_within_budget_with_context(
        "summary refresh 200 jobs",
        elapsed,
        latency_budget,
        &report,
    );

    let completed_jobs = load_workplane_jobs(&coordinator, WorkplaneJobStatus::Completed)
        .into_iter()
        .filter(|job| job.mailbox_name == SEMANTIC_CLONES_SUMMARY_REFRESH_MAILBOX)
        .count();
    assert_eq!(
        completed_jobs, SUMMARY_REFRESH_PERF_JOB_COUNT,
        "expected every summary_refresh job to complete"
    );

    let failed_jobs = load_workplane_jobs(&coordinator, WorkplaneJobStatus::Failed)
        .into_iter()
        .filter(|job| job.mailbox_name == SEMANTIC_CLONES_SUMMARY_REFRESH_MAILBOX)
        .count();
    assert_eq!(failed_jobs, 0, "summary perf jobs should not fail");
}
