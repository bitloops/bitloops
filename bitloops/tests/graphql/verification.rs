use crate::fixtures::{run_query_json, seeded_rust_graphql_workspace};
use serde_json::Value;

const FIXTURE_FILE_PATH: &str = "src/repositories/user_repository.rs";

fn localhost_bind_available(test_name: &str) -> bool {
    match std::net::TcpListener::bind("127.0.0.1:0") {
        Ok(listener) => {
            drop(listener);
            true
        }
        Err(err) if err.kind() == std::io::ErrorKind::PermissionDenied => {
            eprintln!(
                "skipping {test_name}: loopback sockets are unavailable in this environment ({err})"
            );
            false
        }
        Err(err) => panic!("bind localhost for {test_name}: {err}"),
    }
}

#[test]
fn bitloops_devql_query_dsl_matches_raw_graphql_output_end_to_end() {
    if !localhost_bind_available("bitloops_devql_query_dsl_matches_raw_graphql_output_end_to_end") {
        return;
    }
    let seeded = seeded_rust_graphql_workspace("graphql-cli-parity");
    let dsl_output = run_query_json(
        &seeded,
        &[
            "devql",
            "query",
            "--compact",
            r#"file("src/repositories/user_repository.rs")->artefacts()->select(path,canonical_kind,symbol_fqn)->limit(10)"#,
        ],
    );

    let raw_query = r#"
        {
          file(path: "src/repositories/user_repository.rs") {
            artefacts(first: 10) {
              edges {
                node {
                  path
                  canonicalKind
                  symbolFqn
                }
              }
            }
          }
        }
    "#;
    let raw_output = run_query_json(
        &seeded,
        &["devql", "query", "--graphql", "--compact", raw_query],
    );

    let raw_nodes = Value::Array(extract_file_connection_nodes(&raw_output));
    assert!(
        raw_nodes.as_array().is_some_and(|rows| !rows.is_empty()),
        "expected seeded GraphQL query to return artefacts"
    );
    assert_eq!(
        normalise_artefact_rows(&dsl_output),
        normalise_artefact_rows(&raw_nodes)
    );
}

#[test]
fn bitloops_devql_query_accepts_graphql_as_default_input_mode_end_to_end() {
    if !localhost_bind_available(
        "bitloops_devql_query_accepts_graphql_as_default_input_mode_end_to_end",
    ) {
        return;
    }
    let seeded = seeded_rust_graphql_workspace("graphql-cli-default");
    let query = r#"
        {
          file(path: "src/repositories/user_repository.rs") {
            artefacts(first: 2) {
              edges {
                node {
                  path
                  symbolFqn
                  canonicalKind
                }
              }
            }
          }
        }
    "#;

    let default_output = run_query_json(&seeded, &["devql", "query", "--compact", query]);
    let explicit_output = run_query_json(
        &seeded,
        &["devql", "query", "--graphql", "--compact", query],
    );

    let default_nodes = extract_file_connection_nodes(&default_output);
    assert!(
        !default_nodes.is_empty(),
        "expected seeded GraphQL default-input query to return artefacts for {FIXTURE_FILE_PATH}"
    );
    assert_eq!(default_output, explicit_output);
}

fn extract_file_connection_nodes(payload: &Value) -> Vec<Value> {
    payload["file"]["artefacts"]["edges"]
        .as_array()
        .expect("file artefact connection edges")
        .iter()
        .map(|edge| edge["node"].clone())
        .collect()
}

fn normalise_artefact_rows(value: &Value) -> Vec<Value> {
    let mut rows = value
        .as_array()
        .expect("artefact query should return an array")
        .clone();
    rows.sort_by_key(artefact_row_key);
    rows
}

fn artefact_row_key(row: &Value) -> (String, String, String) {
    (
        row.get("path")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        row.get("canonicalKind")
            .or_else(|| row.get("canonical_kind"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        row.get("symbolFqn")
            .or_else(|| row.get("symbol_fqn"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
    )
}
