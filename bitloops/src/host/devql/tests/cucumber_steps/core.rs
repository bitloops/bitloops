use crate::capability_packs::test_harness::mapping::languages::rust::scenarios::collect_rust_suites;
use crate::capability_packs::test_harness::mapping::linker::build_production_index;
use crate::capability_packs::test_harness::mapping::materialize::{
    MaterializationContext, materialize_source_discovery,
};
use crate::capability_packs::test_harness::mapping::model::{
    DiscoveredTestFile, ReferenceCandidate, StructuralMappingStats,
};
use crate::capability_packs::test_harness::storage::TestHarnessRepository;
use crate::host::devql::cucumber_world::{DevqlBddWorld, EdgeExpectation};
use crate::host::devql::*;
use crate::models::{
    CoverageCaptureRecord, CoverageFormat, CoverageHitRecord, ProductionArtefact, ScopeKind,
    TestArtefactCurrentRecord, TestArtefactEdgeCurrentRecord, TestDiscoveryRunRecord,
};
use crate::telemetry::logging;
use crate::test_support::git_fixtures::{git_ok, init_test_repo};
use crate::test_support::logger_lock::with_logger_test_lock;
use crate::test_support::process_state::{enter_process_state, with_cwd};
use anyhow::Context;
use cucumber::{codegen::LocalBoxFuture, step::Collection};
use regex::Regex;
use serde_json::Value;
use std::collections::HashSet;
use std::future::Future;
use std::path::Path;
use std::task::{Context as TaskContext, Poll, RawWaker, RawWakerVTable, Waker};
use tempfile::TempDir;
use tree_sitter::Parser;
use tree_sitter_rust::LANGUAGE as LANGUAGE_RUST;

fn doc_string(ctx: &cucumber::step::Context) -> String {
    ctx.step
        .docstring
        .as_ref()
        .map(ToString::to_string)
        .expect("step docstring should be present")
}

fn table_rows(ctx: &cucumber::step::Context) -> Vec<Vec<String>> {
    ctx.step
        .table
        .as_ref()
        .map(|table| table.rows.clone())
        .expect("step table should be present")
}

fn table_row_maps(ctx: &cucumber::step::Context) -> Vec<std::collections::HashMap<String, String>> {
    let rows = table_rows(ctx);
    let (header, values) = rows
        .split_first()
        .expect("table should include a header row");
    values
        .iter()
        .map(|row| {
            header
                .iter()
                .cloned()
                .zip(row.iter().cloned())
                .collect::<std::collections::HashMap<_, _>>()
        })
        .collect()
}

fn cell_to_opt(cell: &str) -> Option<&str> {
    match cell.trim() {
        "" | "-" => None,
        other => Some(other),
    }
}

fn regex(pattern: &str) -> Regex {
    Regex::new(pattern).unwrap_or_else(|err| panic!("invalid step regex `{pattern}`: {err}"))
}

fn link_metadata(link: &TestArtefactEdgeCurrentRecord) -> Value {
    serde_json::from_str(&link.metadata)
        .unwrap_or_else(|err| panic!("invalid link metadata `{}`: {err}", link.metadata))
}

fn link_confidence(link: &TestArtefactEdgeCurrentRecord) -> f64 {
    link_metadata(link)
        .get("confidence")
        .and_then(Value::as_f64)
        .expect("link metadata should include confidence")
}

fn link_status(link: &TestArtefactEdgeCurrentRecord) -> String {
    link_metadata(link)
        .get("linkage_status")
        .and_then(Value::as_str)
        .expect("link metadata should include linkage_status")
        .to_string()
}

fn noop_waker() -> Waker {
    fn clone(_: *const ()) -> RawWaker {
        RawWaker::new(std::ptr::null(), &VTABLE)
    }
    fn wake(_: *const ()) {}
    fn wake_by_ref(_: *const ()) {}
    fn drop(_: *const ()) {}

    static VTABLE: RawWakerVTable = RawWakerVTable::new(clone, wake, wake_by_ref, drop);

    unsafe { Waker::from_raw(RawWaker::new(std::ptr::null(), &VTABLE)) }
}

fn run_ready_future<F: Future>(future: F) -> F::Output {
    let waker = noop_waker();
    let mut context = TaskContext::from_waker(&waker);
    let mut future = std::pin::pin!(future);
    match Future::poll(future.as_mut(), &mut context) {
        Poll::Ready(value) => value,
        Poll::Pending => panic!("expected future to complete without awaiting external IO"),
    }
}

fn step_fn(
    f: for<'a> fn(&'a mut DevqlBddWorld, cucumber::step::Context) -> LocalBoxFuture<'a, ()>,
) -> for<'a> fn(&'a mut DevqlBddWorld, cucumber::step::Context) -> LocalBoxFuture<'a, ()> {
    f
}

fn extract_artefacts(world: &mut DevqlBddWorld) {
    world.artefacts.clear();

    if let Some(content) = &world.source_content {
        let path = world
            .source_path
            .as_deref()
            .expect("source path should be set");
        let artefacts = extract_js_ts_artefacts(content, path).expect("extract JS/TS artefacts");
        world.artefacts.extend(artefacts);
    }

    if let Some(content) = &world.rust_source_content {
        let path = world
            .rust_source_path
            .as_deref()
            .or(world.source_path.as_deref())
            .expect("rust source path should be set");
        let artefacts = extract_rust_artefacts(content, path).expect("extract Rust artefacts");
        world.artefacts.extend(artefacts);
    }
}

fn extract_edges(world: &mut DevqlBddWorld) {
    if world.artefacts.is_empty() {
        extract_artefacts(world);
    }
    world.edges.clear();

    if let Some(content) = &world.source_content {
        let path = world
            .source_path
            .as_deref()
            .expect("source path should be set");
        let ts_artefacts = world
            .artefacts
            .iter()
            .filter(|artefact| artefact.symbol_fqn.starts_with(path))
            .cloned()
            .collect::<Vec<_>>();
        let edges = extract_js_ts_dependency_edges(content, path, &ts_artefacts)
            .expect("extract JS/TS dependency edges");
        world.edges.extend(edges);
    }

    if let Some(content) = &world.rust_source_content {
        let path = world
            .rust_source_path
            .as_deref()
            .or(world.source_path.as_deref())
            .expect("rust source path should be set");
        let rust_artefacts = world
            .artefacts
            .iter()
            .filter(|artefact| artefact.symbol_fqn.starts_with(path))
            .cloned()
            .collect::<Vec<_>>();
        let edges = extract_rust_dependency_edges(content, path, &rust_artefacts)
            .expect("extract Rust dependency edges");
        world.edges.extend(edges);
    }
}

fn given_typescript_source(
    world: &mut DevqlBddWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let path = ctx.matches[1].1.clone();
        world.source_path = Some(path);
        world.source_language = Some("typescript".to_string());
        world.source_content = Some(doc_string(&ctx));
    })
}

fn given_rust_source(
    world: &mut DevqlBddWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let path = ctx.matches[1].1.clone();
        if world.source_path.is_none() {
            world.source_path = Some(path.clone());
            world.source_language = Some("rust".to_string());
        }
        world.rust_source_path = Some(path);
        world.rust_source_content = Some(doc_string(&ctx));
    })
}

fn when_extract_artefacts(
    world: &mut DevqlBddWorld,
    _ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        extract_artefacts(world);
    })
}

fn when_extract_dependency_edges(
    world: &mut DevqlBddWorld,
    _ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        extract_edges(world);
    })
}

fn when_parse_query(
    world: &mut DevqlBddWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        world.parsed_query = Some(
            parse_devql_query(&doc_string(&ctx)).expect("query should parse for this scenario"),
        );
    })
}

fn when_build_deps_sql(
    world: &mut DevqlBddWorld,
    _ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let parsed = world
            .parsed_query
            .as_ref()
            .expect("query should be parsed before building SQL");
        world.query_sql = Some(
            build_postgres_deps_query(&world.cfg, parsed, &world.cfg.repo.repo_id)
                .expect("deps SQL should build"),
        );
    })
}

fn when_execute_query_without_pg_client(
    world: &mut DevqlBddWorld,
    _ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        world.init_test_logger();
        let workspace = world.logger_workspace_path().to_path_buf();
        let parsed = world
            .parsed_query
            .clone()
            .expect("query should be parsed before execution");
        let cfg = world.cfg.clone();

        let error = with_cwd(&workspace, || {
            with_logger_test_lock(|| {
                logging::reset_logger_for_tests();
                logging::init("bdd-devql-session").expect("initialize test logger");
                let events_cfg = EventsBackendConfig {
                    duckdb_path: None,
                    clickhouse_url: None,
                    clickhouse_user: None,
                    clickhouse_password: None,
                    clickhouse_database: None,
                };
                let result =
                    run_ready_future(execute_devql_query(&cfg, &parsed, &events_cfg, None));
                logging::close();
                result.expect_err("query should fail without a Postgres client")
            })
        });
        world.query_error = Some(error);
    })
}

fn when_extract_artefacts_and_edges_with_logger(
    world: &mut DevqlBddWorld,
    _ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        world.init_test_logger();
        let workspace = world.logger_workspace_path().to_path_buf();
        let source_content = world
            .source_content
            .clone()
            .expect("typescript source should be set");
        let source_path = world
            .source_path
            .clone()
            .expect("typescript source path should be set");

        let (artefacts, edges) = with_cwd(&workspace, || {
            with_logger_test_lock(|| {
                logging::reset_logger_for_tests();
                logging::init("bdd-devql-session").expect("initialize test logger");
                let artefacts = extract_js_ts_artefacts(&source_content, &source_path)
                    .expect("extract artefacts");
                let edges =
                    extract_js_ts_dependency_edges(&source_content, &source_path, &artefacts)
                        .expect("extract edges");
                logging::close();
                (artefacts, edges)
            })
        });

        world.artefacts = artefacts;
        world.edges = edges;
    })
}

fn then_artefacts_include(
    world: &mut DevqlBddWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        for row in table_row_maps(&ctx) {
            world.assert_artefact(
                row.get("language_kind")
                    .map(String::as_str)
                    .expect("language_kind column should exist"),
                cell_to_opt(
                    row.get("canonical_kind")
                        .map(String::as_str)
                        .expect("canonical_kind column should exist"),
                ),
                row.get("name")
                    .map(String::as_str)
                    .expect("name column should exist"),
            );
        }
    })
}

fn then_edges_include(
    world: &mut DevqlBddWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        for row in table_row_maps(&ctx) {
            world.assert_edge(EdgeExpectation {
                edge_kind: row
                    .get("edge_kind")
                    .map(String::as_str)
                    .expect("edge_kind column should exist"),
                from_symbol_fqn: row
                    .get("from")
                    .map(String::as_str)
                    .expect("from column should exist"),
                to_target_symbol_fqn: cell_to_opt(
                    row.get("to_target")
                        .map(String::as_str)
                        .expect("to_target column should exist"),
                ),
                to_symbol_ref: cell_to_opt(
                    row.get("to_ref")
                        .map(String::as_str)
                        .expect("to_ref column should exist"),
                ),
                metadata_key: cell_to_opt(
                    row.get("metadata_key")
                        .map(String::as_str)
                        .expect("metadata_key column should exist"),
                ),
                metadata_value: cell_to_opt(
                    row.get("metadata_value")
                        .map(String::as_str)
                        .expect("metadata_value column should exist"),
                ),
            });
        }
    })
}

fn then_no_artefacts_are_emitted(
    world: &mut DevqlBddWorld,
    _ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        assert!(
            world.artefacts.is_empty(),
            "expected no artefacts, got {:#?}",
            world.artefacts
        );
    })
}

fn then_no_edges_are_emitted(
    world: &mut DevqlBddWorld,
    _ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        assert!(
            world.edges.is_empty(),
            "expected no edges, got {:#?}",
            world.edges
        );
    })
}

fn then_generated_sql_contains(
    world: &mut DevqlBddWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        for row in table_row_maps(&ctx) {
            let fragment = row
                .get("fragment")
                .map(String::as_str)
                .expect("fragment column should exist");
            world.assert_sql_contains(fragment);
        }
    })
}

fn then_query_fails_with_message(
    world: &mut DevqlBddWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let fragment = &ctx.matches[1].1;
        let err = world
            .query_error
            .as_ref()
            .expect("query error should be set before assertion");
        assert!(
            err.to_string().contains(fragment),
            "expected error containing `{fragment}`, got `{err}`"
        );
    })
}

fn then_export_edge_named_appears_count(
    world: &mut DevqlBddWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let export_name = &ctx.matches[1].1;
        let expected_count: usize = ctx.matches[2]
            .1
            .parse()
            .expect("export count should be numeric");
        let actual_count = world
            .edges
            .iter()
            .filter(|edge| {
                edge.edge_kind == "exports"
                    && edge
                        .metadata
                        .get("export_name")
                        .and_then(|value| value.as_str())
                        == Some(export_name.as_str())
            })
            .count();
        assert_eq!(
            actual_count, expected_count,
            "unexpected export edge count for `{export_name}`"
        );
    })
}

fn then_logs_parse_failure(
    world: &mut DevqlBddWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let path = &ctx.matches[1].1;
        let entries = world.read_log_entries();
        assert!(
            entries.iter().any(|entry| {
                entry.get("msg").and_then(Value::as_str) == Some("devql parse failure fallback")
                    && entry.get("path").and_then(Value::as_str) == Some(path.as_str())
                    && entry.get("component").and_then(Value::as_str) == Some("devql")
                    && entry.get("failure_kind").and_then(Value::as_str) == Some("parse_error")
            }),
            "expected parse-failure log entry for `{path}`, got {entries:#?}"
        );
    })
}

fn given_rust_production_file(
    world: &mut DevqlBddWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let path = ctx.matches[1].1.clone();
        let source = doc_string(&ctx);
        world.production_sources.push((path, source));
    })
}

fn given_rust_test_file(
    world: &mut DevqlBddWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let path = ctx.matches[1].1.clone();
        let source = doc_string(&ctx);
        world.test_sources.push((path, source));
    })
}

fn run_test_discovery(world: &mut DevqlBddWorld) {
    let lang: tree_sitter::Language = LANGUAGE_RUST.into();
    let mut parser = Parser::new();
    parser.set_language(&lang).expect("set rust language");

    world.discovered_suites.clear();
    world.discovered_scenarios.clear();

    for (path, source) in &world.test_sources {
        let tree = match parser.parse(source, None) {
            Some(tree) => tree,
            None => {
                world.discovery_issues.push(
                    crate::capability_packs::test_harness::mapping::model::DiscoveryIssue {
                        path: path.clone(),
                        message: "failed to parse source".to_string(),
                    },
                );
                continue;
            }
        };

        let suites = collect_rust_suites(tree.root_node(), source, path);

        let repo_id = "test-repo";
        let commit_sha = "test-commit";

        for suite in &suites {
            let suite_symbol_id = format!("test_suite:{commit_sha}:{path}:{}", suite.start_line);
            let suite_artefact_id = format!("test_artefact:{suite_symbol_id}");
            world.discovered_suites.push(TestArtefactCurrentRecord {
                artefact_id: suite_artefact_id.clone(),
                symbol_id: suite_symbol_id.clone(),
                repo_id: repo_id.to_string(),
                commit_sha: commit_sha.to_string(),
                blob_sha: format!("blob:{commit_sha}:{path}"),
                path: path.clone(),
                language: "rust".to_string(),
                canonical_kind: "test_suite".to_string(),
                language_kind: None,
                symbol_fqn: Some(suite.name.clone()),
                name: suite.name.clone(),
                parent_artefact_id: None,
                parent_symbol_id: None,
                start_line: suite.start_line,
                end_line: suite.end_line,
                start_byte: None,
                end_byte: None,
                signature: None,
                modifiers: "[]".to_string(),
                docstring: None,
                content_hash: None,
                discovery_source: "source".to_string(),
                revision_kind: "commit".to_string(),
                revision_id: commit_sha.to_string(),
            });

            for scenario in &suite.scenarios {
                let scenario_symbol_id = format!(
                    "test_case:{commit_sha}:{path}:{}:{}",
                    scenario.start_line, scenario.name
                );
                world.discovered_scenarios.push(TestArtefactCurrentRecord {
                    artefact_id: format!("test_artefact:{scenario_symbol_id}"),
                    symbol_id: scenario_symbol_id,
                    repo_id: repo_id.to_string(),
                    commit_sha: commit_sha.to_string(),
                    blob_sha: format!("blob:{commit_sha}:{path}"),
                    path: path.clone(),
                    language: "rust".to_string(),
                    canonical_kind: "test_scenario".to_string(),
                    language_kind: None,
                    symbol_fqn: Some(format!("{}.{}", suite.name, scenario.name)),
                    name: scenario.name.clone(),
                    parent_artefact_id: Some(suite_artefact_id.clone()),
                    parent_symbol_id: Some(suite_symbol_id.clone()),
                    start_line: scenario.start_line,
                    end_line: scenario.end_line,
                    start_byte: None,
                    end_byte: None,
                    signature: None,
                    modifiers: "[]".to_string(),
                    docstring: None,
                    content_hash: None,
                    discovery_source: scenario.discovery_source.as_str().to_string(),
                    revision_kind: "commit".to_string(),
                    revision_id: commit_sha.to_string(),
                });
            }
        }
    }
}

fn run_linkage_resolution(world: &mut DevqlBddWorld) {
    let lang: tree_sitter::Language = LANGUAGE_RUST.into();
    let mut parser = Parser::new();
    parser.set_language(&lang).expect("set rust language");

    // Build production artefacts from production sources
    let mut production_artefacts: Vec<ProductionArtefact> = Vec::new();
    for (path, source) in &world.production_sources {
        let _tree = match parser.parse(source, None) {
            Some(tree) => tree,
            None => continue,
        };
        let artefacts = extract_rust_artefacts(source, path).unwrap_or_default();
        for artefact in artefacts {
            production_artefacts.push(ProductionArtefact {
                artefact_id: format!("artefact:{}:{}", path, artefact.name),
                symbol_id: format!("sym:{}:{}", path, artefact.name),
                symbol_fqn: artefact.symbol_fqn.clone(),
                path: path.clone(),
                start_line: artefact.start_line as i64,
            });
        }
    }

    let production_index = build_production_index(&production_artefacts);

    // Discover test files and materialize links
    let mut test_artefacts = Vec::new();
    let mut test_edges = Vec::new();
    let mut link_keys = HashSet::new();
    let mut stats = StructuralMappingStats::default();

    let mut discovered_files: Vec<DiscoveredTestFile> = Vec::new();
    for (path, source) in &world.test_sources {
        let tree = match parser.parse(source, None) {
            Some(tree) => tree,
            None => continue,
        };
        let test_suites = collect_rust_suites(tree.root_node(), source, path);

        // Collect reference candidates from the test file's import paths
        let reference_candidates = vec![ReferenceCandidate::SourcePath(path.clone())];
        // Also add source paths from production sources
        let mut file_references = reference_candidates;
        for (prod_path, _) in &world.production_sources {
            file_references.push(ReferenceCandidate::SourcePath(prod_path.clone()));
        }

        discovered_files.push(DiscoveredTestFile {
            relative_path: path.clone(),
            language: "rust".to_string(),
            reference_candidates: file_references,
            suites: test_suites,
        });
    }

    let mut materialization = MaterializationContext {
        repo_id: "test-repo",
        commit_sha: "test-commit",
        production: &production_artefacts,
        production_index: &production_index,
        test_artefacts: &mut test_artefacts,
        test_edges: &mut test_edges,
        link_keys: &mut link_keys,
        stats: &mut stats,
    };

    materialize_source_discovery(&mut materialization, &discovered_files);

    world.discovered_suites = test_artefacts
        .iter()
        .filter(|artefact| artefact.canonical_kind == "test_suite")
        .cloned()
        .collect();
    world.discovered_scenarios = test_artefacts
        .iter()
        .filter(|artefact| artefact.canonical_kind == "test_scenario")
        .cloned()
        .collect();
    world.materialized_links = test_edges;
}

#[derive(Copy, Clone)]
enum RegisteredStageQueryMode {
    Current,
    AsOfCommit,
    AsOfRef,
}

struct SeededArtefact {
    path: String,
    symbol_id: String,
    current_artefact_id: String,
    historical_artefact_id: String,
}

fn write_repo_sources(repo_root: &Path, world: &DevqlBddWorld) {
    for (path, source) in world
        .production_sources
        .iter()
        .chain(world.test_sources.iter())
    {
        let full_path = repo_root.join(path);
        if let Some(parent) = full_path.parent() {
            std::fs::create_dir_all(parent).expect("create source parent directory");
        }
        std::fs::write(&full_path, source).expect("write fixture source file");
    }
}

fn write_repo_config(repo_root: &Path, sqlite_path: &Path) {
    let config_dir = repo_root.join(".bitloops");
    std::fs::create_dir_all(&config_dir).expect("create .bitloops config dir");
    std::fs::write(
        config_dir.join("config.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "version": "1.0",
            "scope": "project",
            "settings": {
                "stores": {
                    "relational": {
                        "sqlite_path": sqlite_path.to_string_lossy()
                    }
                }
            }
        }))
        .expect("serialise config"),
    )
    .expect("write config");
}

fn rewrite_test_artefact(
    artefact: &TestArtefactCurrentRecord,
    repo_id: &str,
    commit_sha: &str,
) -> TestArtefactCurrentRecord {
    let mut rewritten = artefact.clone();
    rewritten.repo_id = repo_id.to_string();
    rewritten.commit_sha = commit_sha.to_string();
    rewritten.blob_sha = format!("blob:{commit_sha}:{}", rewritten.path);
    rewritten.revision_id = commit_sha.to_string();
    rewritten
}

fn rewrite_test_edge(
    edge: &TestArtefactEdgeCurrentRecord,
    repo_id: &str,
    commit_sha: &str,
    artefact_name: &str,
    symbol_id: &str,
    current_artefact_id: &str,
) -> TestArtefactEdgeCurrentRecord {
    let mut rewritten = edge.clone();
    rewritten.repo_id = repo_id.to_string();
    rewritten.commit_sha = commit_sha.to_string();
    rewritten.blob_sha = format!("blob:{commit_sha}:{}", rewritten.path);
    rewritten.revision_id = commit_sha.to_string();
    if edge_targets_artefact(edge, artefact_name) {
        rewritten.to_symbol_id = Some(symbol_id.to_string());
        rewritten.to_artefact_id = Some(current_artefact_id.to_string());
    }
    rewritten
}

fn edge_targets_artefact(edge: &TestArtefactEdgeCurrentRecord, artefact_name: &str) -> bool {
    edge.to_artefact_id
        .as_deref()
        .is_some_and(|artefact_id| artefact_id.contains(artefact_name))
        || edge
            .to_symbol_id
            .as_deref()
            .is_some_and(|symbol_id| symbol_id.contains(artefact_name))
}

fn discovery_run_record(repo_id: &str, commit_sha: &str) -> TestDiscoveryRunRecord {
    TestDiscoveryRunRecord {
        discovery_run_id: format!("discovery:{commit_sha}:bdd"),
        repo_id: repo_id.to_string(),
        commit_sha: commit_sha.to_string(),
        language: Some("rust".to_string()),
        started_at: "2026-03-24T00:00:00Z".to_string(),
        finished_at: Some("2026-03-24T00:00:01Z".to_string()),
        status: "complete".to_string(),
        enumeration_status: Some("hybrid_full".to_string()),
        notes_json: None,
        stats_json: None,
    }
}

fn coverage_capture_record(
    repo_id: &str,
    commit_sha: &str,
    scenario_id: &str,
) -> CoverageCaptureRecord {
    CoverageCaptureRecord {
        capture_id: format!("capture:{commit_sha}:bdd"),
        repo_id: repo_id.to_string(),
        commit_sha: commit_sha.to_string(),
        tool: "llvm-cov".to_string(),
        format: CoverageFormat::Lcov,
        scope_kind: ScopeKind::TestScenario,
        subject_test_symbol_id: Some(scenario_id.to_string()),
        line_truth: true,
        branch_truth: true,
        captured_at: "2026-03-24T00:00:02Z".to_string(),
        status: "complete".to_string(),
        metadata_json: Some("{\"runner\":\"cargo test\"}".to_string()),
    }
}

fn coverage_hits(symbol_id: &str, path: &str, capture_id: &str) -> Vec<CoverageHitRecord> {
    vec![
        CoverageHitRecord {
            capture_id: capture_id.to_string(),
            production_symbol_id: symbol_id.to_string(),
            file_path: path.to_string(),
            line: 1,
            branch_id: -1,
            covered: true,
            hit_count: 3,
        },
        CoverageHitRecord {
            capture_id: capture_id.to_string(),
            production_symbol_id: symbol_id.to_string(),
            file_path: path.to_string(),
            line: 2,
            branch_id: -1,
            covered: false,
            hit_count: 0,
        },
        CoverageHitRecord {
            capture_id: capture_id.to_string(),
            production_symbol_id: symbol_id.to_string(),
            file_path: path.to_string(),
            line: 3,
            branch_id: 0,
            covered: true,
            hit_count: 1,
        },
        CoverageHitRecord {
            capture_id: capture_id.to_string(),
            production_symbol_id: symbol_id.to_string(),
            file_path: path.to_string(),
            line: 3,
            branch_id: 1,
            covered: false,
            hit_count: 0,
        },
    ]
}

fn seed_target_production_artefact(
    conn: &rusqlite::Connection,
    repo_root: &Path,
    repo_id: &str,
    commit_sha: &str,
    world: &DevqlBddWorld,
    artefact_name: &str,
) -> anyhow::Result<SeededArtefact> {
    for (path, source) in &world.production_sources {
        let artefacts = extract_rust_artefacts(source, path).context("extract rust artefacts")?;
        if artefacts
            .iter()
            .any(|artefact| artefact.name == artefact_name)
        {
            let blob_sha = git_ok(repo_root, &["rev-parse", &format!("{commit_sha}:{path}")]);
            let symbol_id = format!("sym:{path}:{artefact_name}");
            let current_artefact_id = format!("current:{path}:{artefact_name}");
            let historical_artefact_id = format!("historical:{path}:{artefact_name}");
            let symbol_fqn = format!("{path}::{artefact_name}");

            conn.execute(
                "INSERT INTO artefacts_current (
                    repo_id, symbol_id, artefact_id, commit_sha, blob_sha, path, language,
                    canonical_kind, language_kind, symbol_fqn, start_line, end_line, start_byte,
                    end_byte, modifiers, content_hash
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)",
                rusqlite::params![
                    repo_id,
                    symbol_id.as_str(),
                    current_artefact_id.as_str(),
                    commit_sha,
                    blob_sha.as_str(),
                    path,
                    "rust",
                    "function",
                    "function_item",
                    symbol_fqn.as_str(),
                    1i64,
                    3i64,
                    0i64,
                    64i64,
                    "[]",
                    "hash-current",
                ],
            )
            .context("insert current artefact row")?;

            conn.execute(
                "INSERT INTO artefacts (
                    artefact_id, symbol_id, repo_id, blob_sha, path, language, canonical_kind,
                    language_kind, symbol_fqn, start_line, end_line, start_byte, end_byte,
                    modifiers, content_hash
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
                rusqlite::params![
                    historical_artefact_id.as_str(),
                    symbol_id.as_str(),
                    repo_id,
                    blob_sha.as_str(),
                    path,
                    "rust",
                    "function",
                    "function_item",
                    symbol_fqn.as_str(),
                    1i64,
                    3i64,
                    0i64,
                    64i64,
                    "[]",
                    "hash-historical",
                ],
            )
            .context("insert historical artefact row")?;

            return Ok(SeededArtefact {
                path: path.clone(),
                symbol_id,
                current_artefact_id,
                historical_artefact_id,
            });
        }
    }

    anyhow::bail!("target production artefact `{artefact_name}` not found in fixture sources")
}

async fn execute_registered_stage_query(
    world: &mut DevqlBddWorld,
    stage_name: &str,
    artefact_name: &str,
    mode: RegisteredStageQueryMode,
) -> anyhow::Result<Value> {
    run_linkage_resolution(world);

    let temp = TempDir::new().context("create temp dir")?;
    let home = TempDir::new().context("create temp home")?;
    let home_path = home.path().to_string_lossy().to_string();
    let _guard = enter_process_state(
        None,
        &[
            ("HOME", Some(home_path.as_str())),
            ("USERPROFILE", Some(home_path.as_str())),
            ("BITLOOPS_DEVQL_PG_DSN", None),
            ("BITLOOPS_DEVQL_CH_URL", None),
            ("BITLOOPS_DEVQL_CH_USER", None),
            ("BITLOOPS_DEVQL_CH_PASSWORD", None),
            ("BITLOOPS_DEVQL_CH_DATABASE", None),
        ],
    );

    let repo_root = temp.path().join("repo");
    std::fs::create_dir_all(&repo_root).context("create repo root")?;
    init_test_repo(
        &repo_root,
        "main",
        "Bitloops Test",
        "bitloops-test@example.com",
    );
    write_repo_sources(&repo_root, world);
    git_ok(&repo_root, &["add", "."]);
    git_ok(&repo_root, &["commit", "-m", "seed devql bdd fixture"]);
    let commit_sha = git_ok(&repo_root, &["rev-parse", "HEAD"]);

    let mut cfg = DevqlBddWorld::test_cfg();
    cfg.repo_root = repo_root.clone();
    let sqlite_path = temp.path().join("relational.sqlite");
    write_repo_config(&repo_root, &sqlite_path);
    init_sqlite_schema(&sqlite_path)
        .await
        .context("initialise sqlite relational schema")?;
    crate::capability_packs::test_harness::storage::init_schema_for_repo(&repo_root)
        .context("initialise test harness schema")?;

    let conn = rusqlite::Connection::open(&sqlite_path).context("open sqlite db")?;
    let seeded = seed_target_production_artefact(
        &conn,
        &repo_root,
        &cfg.repo.repo_id,
        commit_sha.trim(),
        world,
        artefact_name,
    )?;

    let mut repository =
        crate::capability_packs::test_harness::storage::open_repository_for_repo(&repo_root)
            .context("open test harness repository")?;
    let rewritten_suites = world
        .discovered_suites
        .iter()
        .map(|artefact| rewrite_test_artefact(artefact, &cfg.repo.repo_id, commit_sha.trim()))
        .collect::<Vec<_>>();
    let rewritten_scenarios = world
        .discovered_scenarios
        .iter()
        .map(|artefact| rewrite_test_artefact(artefact, &cfg.repo.repo_id, commit_sha.trim()))
        .collect::<Vec<_>>();
    let rewritten_edges = world
        .materialized_links
        .iter()
        .map(|edge| {
            rewrite_test_edge(
                edge,
                &cfg.repo.repo_id,
                commit_sha.trim(),
                artefact_name,
                &seeded.symbol_id,
                &seeded.current_artefact_id,
            )
        })
        .collect::<Vec<_>>();
    let mut test_artefacts = rewritten_suites;
    test_artefacts.extend(rewritten_scenarios.clone());
    repository
        .replace_test_discovery(
            commit_sha.trim(),
            &test_artefacts,
            &rewritten_edges,
            &discovery_run_record(&cfg.repo.repo_id, commit_sha.trim()),
            &[],
        )
        .context("seed test discovery rows")?;

    if stage_name == "coverage"
        && world
            .materialized_links
            .iter()
            .any(|edge| edge_targets_artefact(edge, artefact_name))
    {
        let scenario_id = rewritten_scenarios
            .first()
            .map(|scenario| scenario.symbol_id.as_str())
            .expect("expected discovered scenario");
        let capture = coverage_capture_record(&cfg.repo.repo_id, commit_sha.trim(), scenario_id);
        repository
            .insert_coverage_capture(&capture)
            .context("seed coverage capture")?;
        repository
            .insert_coverage_hits(&coverage_hits(
                &seeded.symbol_id,
                &seeded.path,
                &capture.capture_id,
            ))
            .context("seed coverage hits")?;
    }

    let query = match mode {
        RegisteredStageQueryMode::Current => format!(
            r#"repo("temp2")->file("{}")->artefacts(kind:"function")->{}()->limit(10)"#,
            seeded.path, stage_name
        ),
        RegisteredStageQueryMode::AsOfCommit => format!(
            r#"repo("temp2")->asOf(commit:"{}")->file("{}")->artefacts(kind:"function")->{}()->limit(10)"#,
            commit_sha.trim(),
            seeded.path,
            stage_name
        ),
        RegisteredStageQueryMode::AsOfRef => format!(
            r#"repo("temp2")->asOf(ref:"main")->file("{}")->artefacts(kind:"function")->{}()->limit(10)"#,
            seeded.path, stage_name
        ),
    };

    let expected_artefact_id = match mode {
        RegisteredStageQueryMode::Current => seeded.current_artefact_id.as_str(),
        RegisteredStageQueryMode::AsOfCommit | RegisteredStageQueryMode::AsOfRef => {
            seeded.historical_artefact_id.as_str()
        }
    };

    let parsed = parse_devql_query(&query).context("parse query")?;
    let relational = RelationalStorage::local_only(sqlite_path.clone());
    let events_cfg = crate::config::EventsBackendConfig {
        duckdb_path: None,
        clickhouse_url: None,
        clickhouse_user: None,
        clickhouse_password: None,
        clickhouse_database: None,
    };
    let base_rows = execute_devql_query(&cfg, &parsed, &events_cfg, Some(&relational))
        .await
        .context("execute base pipeline")?;
    let mut rows = execute_registered_stages(&cfg, &parsed, base_rows)
        .await
        .context("execute registered stage")?;
    let row = rows
        .pop()
        .context("expected registered stage response row")?;
    if stage_name == "tests"
        && row
            .get("covering_tests")
            .and_then(Value::as_array)
            .is_some_and(|tests| tests.is_empty())
    {
        let mode_label = match mode {
            RegisteredStageQueryMode::Current => "current",
            RegisteredStageQueryMode::AsOfCommit => "asof_commit",
            RegisteredStageQueryMode::AsOfRef => "asof_ref",
        };
        eprintln!(
            "empty tests() response for mode {mode_label}: {}",
            serde_json::to_string_pretty(&row).unwrap_or_else(|_| "<unserializable>".to_string())
        );
    }
    let artefact_id = row
        .get("artefact")
        .and_then(|artefact| artefact.get("artefact_id"))
        .and_then(Value::as_str)
        .context("response artefact_id should exist")?;
    anyhow::ensure!(
        artefact_id == expected_artefact_id,
        "expected artefact_id `{expected_artefact_id}`, got `{artefact_id}`"
    );

    Ok(row)
}

fn when_test_discovery(
    world: &mut DevqlBddWorld,
    _ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        run_test_discovery(world);
    })
}

fn when_linkage_resolution(
    world: &mut DevqlBddWorld,
    _ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        run_linkage_resolution(world);
    })
}

fn when_linkage_and_tests_query(
    world: &mut DevqlBddWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        world.tests_query_response = Some(
            execute_registered_stage_query(
                world,
                "tests",
                &ctx.matches[1].1,
                RegisteredStageQueryMode::Current,
            )
            .await
            .expect("execute current tests() query"),
        );
    })
}

fn when_linkage_and_tests_query_asof_commit(
    world: &mut DevqlBddWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        world.tests_query_response = Some(
            execute_registered_stage_query(
                world,
                "tests",
                &ctx.matches[1].1,
                RegisteredStageQueryMode::AsOfCommit,
            )
            .await
            .expect("execute historical tests() commit query"),
        );
    })
}

fn when_linkage_and_tests_query_asof_ref(
    world: &mut DevqlBddWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        world.tests_query_response = Some(
            execute_registered_stage_query(
                world,
                "tests",
                &ctx.matches[1].1,
                RegisteredStageQueryMode::AsOfRef,
            )
            .await
            .expect("execute historical tests() ref query"),
        );
    })
}

fn when_test_discovery_with_diagnostics(
    world: &mut DevqlBddWorld,
    _ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let lang: tree_sitter::Language = LANGUAGE_RUST.into();
        let mut parser = Parser::new();
        parser.set_language(&lang).expect("set rust language");

        for (path, source) in &world.test_sources {
            let tree = parser.parse(source, None);
            if tree.is_none() || source.matches('{').count() != source.matches('}').count() {
                world.discovery_issues.push(
                    crate::capability_packs::test_harness::mapping::model::DiscoveryIssue {
                        path: path.clone(),
                        message: "parse error or incomplete source".to_string(),
                    },
                );
            }
        }
    })
}

fn then_test_suites_include(
    world: &mut DevqlBddWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        for row in table_row_maps(&ctx) {
            let name = row.get("name").expect("name column should exist");
            let expected_count: usize = row
                .get("scenario_count")
                .expect("scenario_count column should exist")
                .parse()
                .expect("scenario_count should be numeric");

            let suite = world.discovered_suites.iter().find(|s| s.name == *name);

            assert!(
                suite.is_some(),
                "expected suite `{name}` in discovered suites, found: {:?}",
                world
                    .discovered_suites
                    .iter()
                    .map(|s| &s.name)
                    .collect::<Vec<_>>()
            );

            let suite = suite.unwrap();
            let actual_count = world
                .discovered_scenarios
                .iter()
                .filter(|scenario| scenario.parent_symbol_id.as_deref() == Some(&suite.symbol_id))
                .count();

            assert_eq!(
                actual_count, expected_count,
                "suite `{name}` expected {expected_count} scenarios, got {actual_count}"
            );
        }
    })
}

fn then_test_scenarios_include(
    world: &mut DevqlBddWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        for row in table_row_maps(&ctx) {
            let name = row.get("name").expect("name column should exist");
            let discovery_source = row
                .get("discovery_source")
                .expect("discovery_source column should exist");

            let found = world
                .discovered_scenarios
                .iter()
                .any(|s| s.name == *name && s.discovery_source == *discovery_source);

            assert!(
                found,
                "expected scenario `{name}` with discovery_source `{discovery_source}`, found: {:?}",
                world
                    .discovered_scenarios
                    .iter()
                    .map(|s| (&s.name, &s.discovery_source))
                    .collect::<Vec<_>>()
            );
        }
    })
}

fn then_direct_links_include(
    world: &mut DevqlBddWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        for row in table_row_maps(&ctx) {
            let production_name = row
                .get("production_name")
                .expect("production_name column should exist");
            let expected_confidence: f64 = row
                .get("confidence")
                .expect("confidence column should exist")
                .parse()
                .expect("confidence should be numeric");
            let expected_status = row
                .get("linkage_status")
                .expect("linkage_status column should exist");

            let found = world.materialized_links.iter().any(|link| {
                link.to_artefact_id
                    .as_deref()
                    .is_some_and(|artefact_id| artefact_id.contains(production_name))
                    && (link_confidence(link) - expected_confidence).abs() < 0.01
                    && link_status(link) == expected_status.as_str()
            });

            assert!(
                found,
                "expected link to `{production_name}` with confidence {expected_confidence} and status `{expected_status}`, found: {:?}",
                world
                    .materialized_links
                    .iter()
                    .map(|l| {
                        (
                            l.to_artefact_id.as_deref().unwrap_or("<unresolved>"),
                            link_confidence(l),
                            link_status(l).to_string(),
                        )
                    })
                    .collect::<Vec<_>>()
            );
        }
    })
}

fn then_no_links_are_created(
    world: &mut DevqlBddWorld,
    _ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        assert!(
            world.materialized_links.is_empty(),
            "expected no links, got {:?}",
            world.materialized_links
        );
    })
}

fn then_no_links_to_from(
    world: &mut DevqlBddWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let production_name = &ctx.matches[1].1;
        let test_name = &ctx.matches[2].1;

        let found = world.materialized_links.iter().any(|link| {
            link.to_artefact_id
                .as_deref()
                .is_some_and(|artefact_id| artefact_id.contains(production_name.as_str()))
                && world
                    .discovered_scenarios
                    .iter()
                    .find(|scenario| scenario.symbol_id == link.from_symbol_id)
                    .is_some_and(|scenario| scenario.name.contains(test_name.as_str()))
        });

        assert!(
            !found,
            "expected no link from `{test_name}` to `{production_name}`, but found one"
        );
    })
}

fn then_diagnostics_include(
    world: &mut DevqlBddWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        for row in table_row_maps(&ctx) {
            let path = row.get("path").expect("path column should exist");
            let _severity = row.get("severity").expect("severity column should exist");

            let found = world
                .discovery_issues
                .iter()
                .any(|issue| issue.path == *path);
            assert!(
                found,
                "expected diagnostic for path `{path}`, found: {:?}",
                world.discovery_issues
            );
        }
    })
}

fn then_response_has_covering_tests(
    world: &mut DevqlBddWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let response = world
            .tests_query_response
            .as_ref()
            .expect("tests query response should be set");
        let covering_tests = response
            .get("covering_tests")
            .and_then(Value::as_array)
            .expect("response should have covering_tests");

        for row in table_row_maps(&ctx) {
            let test_name = row.get("test_name").expect("test_name column should exist");
            let expected_confidence: f64 = row
                .get("confidence")
                .expect("confidence column should exist")
                .parse()
                .expect("confidence should be numeric");

            let found = covering_tests.iter().any(|test| {
                test.get("test_name")
                    .and_then(Value::as_str)
                    .is_some_and(|n| n == test_name)
                    && test
                        .get("confidence")
                        .and_then(Value::as_f64)
                        .is_some_and(|c| (c - expected_confidence).abs() < 0.01)
            });

            assert!(
                found,
                "expected covering test `{test_name}` with confidence {expected_confidence}, found: {covering_tests:?}"
            );
        }
    })
}

fn then_logs_validation_error(
    world: &mut DevqlBddWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let reason = &ctx.matches[1].1;
        let entries = world.read_log_entries();
        assert!(
            entries.iter().any(|entry| {
                entry.get("msg").and_then(Value::as_str) == Some("devql query validation failed")
                    && entry.get("reason").and_then(Value::as_str) == Some(reason.as_str())
                    && entry.get("component").and_then(Value::as_str) == Some("devql")
            }),
            "expected validation-error log entry containing `{reason}`, got {entries:#?}"
        );
    })
}

fn when_coverage_ingested_and_query(
    world: &mut DevqlBddWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        world.tests_query_response = Some(
            execute_registered_stage_query(
                world,
                "coverage",
                &ctx.matches[1].1,
                RegisteredStageQueryMode::Current,
            )
            .await
            .expect("execute current coverage() query"),
        );
    })
}

fn when_coverage_ingested_and_query_asof_commit(
    world: &mut DevqlBddWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        world.tests_query_response = Some(
            execute_registered_stage_query(
                world,
                "coverage",
                &ctx.matches[1].1,
                RegisteredStageQueryMode::AsOfCommit,
            )
            .await
            .expect("execute historical coverage() commit query"),
        );
    })
}

fn when_coverage_ingested_and_query_asof_ref(
    world: &mut DevqlBddWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        world.tests_query_response = Some(
            execute_registered_stage_query(
                world,
                "coverage",
                &ctx.matches[1].1,
                RegisteredStageQueryMode::AsOfRef,
            )
            .await
            .expect("execute historical coverage() ref query"),
        );
    })
}

fn then_response_has_coverage_pct(
    world: &mut DevqlBddWorld,
    _ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let response = world
            .tests_query_response
            .as_ref()
            .expect("coverage query response should be set");
        let coverage = response
            .get("coverage")
            .expect("response should have coverage");
        let line_pct = coverage
            .get("line_coverage_pct")
            .and_then(Value::as_f64)
            .expect("coverage should have line_coverage_pct");
        assert!(
            line_pct >= 0.0,
            "line_coverage_pct should be non-negative, got {line_pct}"
        );
    })
}

fn then_response_artefact_has_id(
    world: &mut DevqlBddWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let expected = &ctx.matches[1].1;
        let response = world
            .tests_query_response
            .as_ref()
            .expect("stage response should be set");
        let artefact_id = response
            .get("artefact")
            .and_then(|artefact| artefact.get("artefact_id"))
            .and_then(Value::as_str)
            .expect("response should include artefact.artefact_id");
        assert_eq!(artefact_id, expected, "unexpected response artefact_id");
    })
}

pub(super) fn collection() -> Collection<DevqlBddWorld> {
    Collection::new()
        .given(
            None,
            regex(r#"^a TypeScript source file at "([^"]+)":$"#),
            step_fn(given_typescript_source),
        )
        .given(
            None,
            regex(r#"^a Rust source file at "([^"]+)":$"#),
            step_fn(given_rust_source),
        )
        .given(
            None,
            regex(r#"^a Rust production file at "([^"]+)":$"#),
            step_fn(given_rust_production_file),
        )
        .given(
            None,
            regex(r#"^a Rust test file at "([^"]+)":$"#),
            step_fn(given_rust_test_file),
        )
        .when(
            None,
            regex(r"^devql ingest extracts artefacts$"),
            step_fn(when_extract_artefacts),
        )
        .when(
            None,
            regex(r"^devql ingest extracts dependency edges$"),
            step_fn(when_extract_dependency_edges),
        )
        .when(
            None,
            regex(r"^devql parses the query:$"),
            step_fn(when_parse_query),
        )
        .when(
            None,
            regex(r"^devql builds the deps SQL$"),
            step_fn(when_build_deps_sql),
        )
        .when(
            None,
            regex(r"^devql executes the query without a Postgres client$"),
            step_fn(when_execute_query_without_pg_client),
        )
        .when(
            None,
            regex(r"^devql extracts artefacts and dependency edges with logger capture$"),
            step_fn(when_extract_artefacts_and_edges_with_logger),
        )
        .then(
            None,
            regex(r"^artefacts include:$"),
            step_fn(then_artefacts_include),
        )
        .then(
            None,
            regex(r"^edges include:$"),
            step_fn(then_edges_include),
        )
        .then(
            None,
            regex(r"^no artefacts are emitted$"),
            step_fn(then_no_artefacts_are_emitted),
        )
        .then(
            None,
            regex(r"^no edges are emitted$"),
            step_fn(then_no_edges_are_emitted),
        )
        .then(
            None,
            regex(r"^the generated SQL contains:$"),
            step_fn(then_generated_sql_contains),
        )
        .then(
            None,
            regex(r#"^the query fails with message containing "([^"]+)"$"#),
            step_fn(then_query_fails_with_message),
        )
        .then(
            None,
            regex(r#"^the export edge named "([^"]+)" appears (\d+) time\(s\)$"#),
            step_fn(then_export_edge_named_appears_count),
        )
        .then(
            None,
            regex(r#"^devql logs a parse-failure event with path "([^"]+)"$"#),
            step_fn(then_logs_parse_failure),
        )
        .then(
            None,
            regex(r#"^devql logs a validation-error event containing "([^"]+)"$"#),
            step_fn(then_logs_validation_error),
        )
        .when(
            None,
            regex(r"^test discovery runs$"),
            step_fn(when_test_discovery),
        )
        .when(
            None,
            regex(r"^linkage resolution runs$"),
            step_fn(when_linkage_resolution),
        )
        .when(
            None,
            regex(r#"^linkage resolution runs and tests\(\) query executes for "([^"]+)"$"#),
            step_fn(when_linkage_and_tests_query),
        )
        .when(
            None,
            regex(r#"^linkage resolution runs and asOf\(commit\) tests\(\) query executes for "([^"]+)"$"#),
            step_fn(when_linkage_and_tests_query_asof_commit),
        )
        .when(
            None,
            regex(r#"^linkage resolution runs and asOf\(ref\) tests\(\) query executes for "([^"]+)"$"#),
            step_fn(when_linkage_and_tests_query_asof_ref),
        )
        .when(
            None,
            regex(r"^test discovery runs with diagnostics$"),
            step_fn(when_test_discovery_with_diagnostics),
        )
        .then(
            None,
            regex(r"^test suites include:$"),
            step_fn(then_test_suites_include),
        )
        .then(
            None,
            regex(r"^test scenarios include:$"),
            step_fn(then_test_scenarios_include),
        )
        .then(
            None,
            regex(r"^direct links include:$"),
            step_fn(then_direct_links_include),
        )
        .then(
            None,
            regex(r"^no links are created$"),
            step_fn(then_no_links_are_created),
        )
        .then(
            None,
            regex(r#"^no links to "([^"]+)" from "([^"]+)"$"#),
            step_fn(then_no_links_to_from),
        )
        .then(
            None,
            regex(r"^diagnostics include:$"),
            step_fn(then_diagnostics_include),
        )
        .then(
            None,
            regex(r"^the response has covering_tests with:$"),
            step_fn(then_response_has_covering_tests),
        )
        .then(
            None,
            regex(r#"^the response artefact has artefact_id "([^"]+)"$"#),
            step_fn(then_response_artefact_has_id),
        )
        .when(
            None,
            regex(r#"^coverage is ingested and coverage\(\) query executes for "([^"]+)"$"#),
            step_fn(when_coverage_ingested_and_query),
        )
        .when(
            None,
            regex(r#"^coverage is ingested and asOf\(commit\) coverage\(\) query executes for "([^"]+)"$"#),
            step_fn(when_coverage_ingested_and_query_asof_commit),
        )
        .when(
            None,
            regex(r#"^coverage is ingested and asOf\(ref\) coverage\(\) query executes for "([^"]+)"$"#),
            step_fn(when_coverage_ingested_and_query_asof_ref),
        )
        .then(
            None,
            regex(r"^the response has coverage with line_coverage_pct$"),
            step_fn(then_response_has_coverage_pct),
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_world() -> DevqlBddWorld {
        let mut world = DevqlBddWorld::default();
        world.production_sources.push((
            "src/user/service.rs".to_string(),
            r#"
pub fn create_user(name: &str) -> String {
    name.to_string()
}

pub fn delete_user(id: u64) -> bool {
    let _ = id;
    true
}
"#
            .trim()
            .to_string(),
        ));
        world.test_sources.push((
            "src/user/service_tests.rs".to_string(),
            r#"
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn test_create_user() {
        create_user("Alice");
    }
}
"#
            .trim()
            .to_string(),
        ));
        world
    }

    #[tokio::test]
    async fn execute_registered_stage_query_tests_current_uses_live_artefact_rows() {
        let mut world = fixture_world();
        let response = execute_registered_stage_query(
            &mut world,
            "tests",
            "create_user",
            RegisteredStageQueryMode::Current,
        )
        .await
        .expect("execute current tests query");

        assert_eq!(
            response["artefact"]["artefact_id"],
            Value::String("current:src/user/service.rs:create_user".to_string())
        );
        assert_eq!(response["covering_tests"].as_array().map(Vec::len), Some(1));
    }

    #[tokio::test]
    async fn execute_registered_stage_query_tests_asof_commit_uses_historical_artefact_rows() {
        let mut world = fixture_world();
        let response = execute_registered_stage_query(
            &mut world,
            "tests",
            "create_user",
            RegisteredStageQueryMode::AsOfCommit,
        )
        .await
        .expect("execute commit tests query");

        assert_eq!(
            response["artefact"]["artefact_id"],
            Value::String("historical:src/user/service.rs:create_user".to_string())
        );
        assert_eq!(response["covering_tests"].as_array().map(Vec::len), Some(1));
    }

    #[tokio::test]
    async fn execute_registered_stage_query_tests_asof_ref_uses_historical_artefact_rows() {
        let mut world = fixture_world();
        let response = execute_registered_stage_query(
            &mut world,
            "tests",
            "create_user",
            RegisteredStageQueryMode::AsOfRef,
        )
        .await
        .expect("execute ref tests query");

        assert_eq!(
            response["artefact"]["artefact_id"],
            Value::String("historical:src/user/service.rs:create_user".to_string())
        );
        assert_eq!(response["covering_tests"].as_array().map(Vec::len), Some(1));
    }

    #[tokio::test]
    async fn execute_registered_stage_query_tests_current_does_not_invent_links_for_other_artefacts()
     {
        let mut world = fixture_world();
        let response = execute_registered_stage_query(
            &mut world,
            "tests",
            "delete_user",
            RegisteredStageQueryMode::Current,
        )
        .await
        .expect("execute current tests query");

        assert_eq!(
            response["artefact"]["artefact_id"],
            Value::String("current:src/user/service.rs:delete_user".to_string())
        );
        assert_eq!(response["covering_tests"].as_array().map(Vec::len), Some(0));
    }

    #[tokio::test]
    async fn execute_registered_stage_query_coverage_current_uses_live_artefact_rows() {
        let mut world = fixture_world();
        let response = execute_registered_stage_query(
            &mut world,
            "coverage",
            "create_user",
            RegisteredStageQueryMode::Current,
        )
        .await
        .expect("execute current coverage query");

        assert_eq!(
            response["artefact"]["artefact_id"],
            Value::String("current:src/user/service.rs:create_user".to_string())
        );
        assert!(
            response["coverage"]["line_coverage_pct"]
                .as_f64()
                .is_some_and(|value| value >= 0.0)
        );
    }

    #[tokio::test]
    async fn execute_registered_stage_query_coverage_asof_commit_uses_historical_artefact_rows() {
        let mut world = fixture_world();
        let response = execute_registered_stage_query(
            &mut world,
            "coverage",
            "create_user",
            RegisteredStageQueryMode::AsOfCommit,
        )
        .await
        .expect("execute commit coverage query");

        assert_eq!(
            response["artefact"]["artefact_id"],
            Value::String("historical:src/user/service.rs:create_user".to_string())
        );
        assert!(
            response["coverage"]["line_coverage_pct"]
                .as_f64()
                .is_some_and(|value| value >= 0.0)
        );
    }

    #[tokio::test]
    async fn execute_registered_stage_query_coverage_asof_ref_uses_historical_artefact_rows() {
        let mut world = fixture_world();
        let response = execute_registered_stage_query(
            &mut world,
            "coverage",
            "create_user",
            RegisteredStageQueryMode::AsOfRef,
        )
        .await
        .expect("execute ref coverage query");

        assert_eq!(
            response["artefact"]["artefact_id"],
            Value::String("historical:src/user/service.rs:create_user".to_string())
        );
        assert!(
            response["coverage"]["line_coverage_pct"]
                .as_f64()
                .is_some_and(|value| value >= 0.0)
        );
    }

    #[tokio::test]
    async fn execute_registered_stage_query_coverage_current_does_not_invent_hits_for_other_artefacts()
     {
        let mut world = fixture_world();
        let response = execute_registered_stage_query(
            &mut world,
            "coverage",
            "delete_user",
            RegisteredStageQueryMode::Current,
        )
        .await
        .expect("execute current coverage query");

        assert_eq!(
            response["artefact"]["artefact_id"],
            Value::String("current:src/user/service.rs:delete_user".to_string())
        );
        assert_eq!(
            response["coverage"]["line_data_available"].as_bool(),
            Some(false)
        );
        assert_eq!(
            response["coverage"]["branch_data_available"].as_bool(),
            Some(false)
        );
    }
}
