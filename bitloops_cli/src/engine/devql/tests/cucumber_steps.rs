use super::cucumber_world::{DevqlBddWorld, EdgeExpectation};
use super::*;
use crate::engine::logging;
use crate::test_support::logger_lock::with_logger_test_lock;
use crate::test_support::process_state::with_cwd;
use cucumber::{codegen::LocalBoxFuture, step::Collection};
use regex::Regex;
use serde_json::Value;
use std::future::Future;
use std::task::{Context as TaskContext, Poll, RawWaker, RawWakerVTable, Waker};

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
}
