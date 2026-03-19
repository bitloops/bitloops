use super::cucumber_world::{DevqlBddWorld, EdgeExpectation};
use super::*;
use crate::app::test_mapping::languages::rust::scenarios::collect_rust_suites;
use crate::app::test_mapping::linker::build_production_index;
use crate::app::test_mapping::materialize::{MaterializationContext, materialize_source_discovery};
use crate::app::test_mapping::model::{
    DiscoveredTestFile, ReferenceCandidate, StructuralMappingStats,
};
use crate::domain::ProductionArtefact;
use crate::engine::logging;
use crate::test_support::logger_lock::with_logger_test_lock;
use crate::test_support::process_state::with_cwd;
use cucumber::{codegen::LocalBoxFuture, step::Collection};
use regex::Regex;
use serde_json::Value;
use std::collections::HashSet;
use std::future::Future;
use std::task::{Context as TaskContext, Poll, RawWaker, RawWakerVTable, Waker};
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
                    provider: EventsProvider::DuckDb,
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
                world
                    .discovery_issues
                    .push(crate::app::test_mapping::model::DiscoveryIssue {
                        path: path.clone(),
                        message: "failed to parse source".to_string(),
                    });
                continue;
            }
        };

        let suites = collect_rust_suites(tree.root_node(), source, path);

        let repo_id = "test-repo";
        let commit_sha = "test-commit";

        for suite in &suites {
            let suite_id = format!("test_suite:{commit_sha}:{path}:{}", suite.start_line);
            world
                .discovered_suites
                .push(crate::domain::TestSuiteRecord {
                    suite_id: suite_id.clone(),
                    repo_id: repo_id.to_string(),
                    commit_sha: commit_sha.to_string(),
                    language: "rust".to_string(),
                    path: path.clone(),
                    name: suite.name.clone(),
                    symbol_fqn: Some(suite.name.clone()),
                    start_line: suite.start_line,
                    end_line: suite.end_line,
                    start_byte: None,
                    end_byte: None,
                    signature: None,
                    discovery_source: "source".to_string(),
                });

            for scenario in &suite.scenarios {
                let scenario_id = format!(
                    "test_case:{commit_sha}:{path}:{}:{}",
                    scenario.start_line, scenario.name
                );
                world
                    .discovered_scenarios
                    .push(crate::domain::TestScenarioRecord {
                        scenario_id,
                        suite_id: suite_id.clone(),
                        repo_id: repo_id.to_string(),
                        commit_sha: commit_sha.to_string(),
                        language: "rust".to_string(),
                        path: path.clone(),
                        name: scenario.name.clone(),
                        symbol_fqn: Some(format!("{}.{}", suite.name, scenario.name)),
                        start_line: scenario.start_line,
                        end_line: scenario.end_line,
                        start_byte: None,
                        end_byte: None,
                        signature: None,
                        discovery_source: scenario.discovery_source.as_str().to_string(),
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
    let mut suites = Vec::new();
    let mut scenarios = Vec::new();
    let mut links = Vec::new();
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
        suites: &mut suites,
        scenarios: &mut scenarios,
        links: &mut links,
        link_keys: &mut link_keys,
        stats: &mut stats,
    };

    materialize_source_discovery(&mut materialization, &discovered_files);

    world.materialized_links = links;
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
        run_linkage_resolution(world);

        let artefact_name = &ctx.matches[1].1;
        let matching_link = world
            .materialized_links
            .iter()
            .find(|link| link.production_artefact_id.contains(artefact_name));

        if let Some(link) = matching_link {
            let covering_tests: Vec<Value> = world
                .materialized_links
                .iter()
                .filter(|l| l.production_artefact_id == link.production_artefact_id)
                .map(|l| {
                    let scenario_name = l
                        .test_scenario_id
                        .rsplit(':')
                        .next()
                        .unwrap_or(&l.test_scenario_id);
                    serde_json::json!({
                        "test_name": scenario_name,
                        "confidence": l.confidence,
                        "linkage_status": l.linkage_status,
                    })
                })
                .collect();

            world.tests_query_response = Some(serde_json::json!({
                "covering_tests": covering_tests,
            }));
        } else {
            world.tests_query_response = Some(serde_json::json!({
                "covering_tests": [],
            }));
        }
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
                world
                    .discovery_issues
                    .push(crate::app::test_mapping::model::DiscoveryIssue {
                        path: path.clone(),
                        message: "parse error or incomplete source".to_string(),
                    });
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
                .filter(|s| s.suite_id == suite.suite_id)
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
                link.production_artefact_id.contains(production_name)
                    && (link.confidence - expected_confidence).abs() < 0.01
                    && link.linkage_status == *expected_status
            });

            assert!(
                found,
                "expected link to `{production_name}` with confidence {expected_confidence} and status `{expected_status}`, found: {:?}",
                world
                    .materialized_links
                    .iter()
                    .map(|l| (&l.production_artefact_id, l.confidence, &l.linkage_status))
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
            link.production_artefact_id
                .contains(production_name.as_str())
                && link.test_scenario_id.contains(test_name.as_str())
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
}
