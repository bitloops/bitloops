mod test_harness_support;

#[path = "graphql/fixtures.rs"]
mod fixtures;

use crate::fixtures::{run_query_json_until, seeded_rust_graphql_workspace};
use rusqlite::{Connection, params};
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
fn bitloops_devql_query_smoke_end_to_end() {
    if !localhost_bind_available("bitloops_devql_query_smoke_end_to_end") {
        return;
    }

    let seeded = seeded_rust_graphql_workspace("graphql-cli-smoke");
    let graphql_query = r#"
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

    let default_output = run_query_json_until(
        &seeded,
        &["devql", "query", "--compact", graphql_query],
        "default GraphQL artefact rows",
        |payload| !extract_file_connection_nodes(payload).is_empty(),
    );
    let explicit_output = run_query_json_until(
        &seeded,
        &["devql", "query", "--graphql", "--compact", graphql_query],
        "explicit GraphQL artefact rows",
        |payload| !extract_file_connection_nodes(payload).is_empty(),
    );
    assert_eq!(
        default_output, explicit_output,
        "default devql query input mode should match explicit GraphQL mode"
    );

    let raw_nodes = Value::Array(extract_file_connection_nodes(&explicit_output));
    assert!(
        raw_nodes.as_array().is_some_and(|rows| !rows.is_empty()),
        "expected seeded GraphQL query to return artefacts for {FIXTURE_FILE_PATH}"
    );

    let dsl_output = run_query_json_until(
        &seeded,
        &[
            "devql",
            "query",
            "--compact",
            r#"file("src/repositories/user_repository.rs")->artefacts()->select(path,canonical_kind,symbol_fqn)->limit(10)"#,
        ],
        "DSL artefact rows",
        |payload| payload.as_array().is_some_and(|rows| !rows.is_empty()),
    );
    assert_eq!(
        normalise_artefact_rows(&dsl_output),
        normalise_artefact_rows(&raw_nodes),
        "DSL output should match raw GraphQL nodes"
    );
}

#[test]
fn select_artefacts_overview_includes_architecture_stage_end_to_end() {
    if !localhost_bind_available("select_artefacts_overview_includes_architecture_stage_end_to_end")
    {
        return;
    }

    let seeded = seeded_rust_graphql_workspace("graphql-architecture-overview");
    seed_architecture_node_for_fixture(&seeded);

    let graphql_query = r#"
        {
          selectArtefacts(by: { path: "src/repositories/user_repository.rs" }) {
            overview
          }
        }
    "#;

    let output = run_query_json_until(
        &seeded,
        &["devql", "query", "--graphql", "--compact", graphql_query],
        "selected architecture overview",
        |payload| {
            payload["selectArtefacts"]["overview"]["architecture"]["overview"]["available"]
                .as_bool()
                == Some(true)
        },
    );
    let architecture = &output["selectArtefacts"]["overview"]["architecture"];

    assert_eq!(architecture["overview"]["available"], true);
    assert_eq!(architecture["overview"]["matchedArtefactCount"], 1);
    assert_eq!(architecture["overview"]["nodeKinds"]["NODE"], 1);
}

fn seed_architecture_node_for_fixture(seeded: &fixtures::SeededGraphqlWorkspace) {
    let conn = Connection::open(seeded.workspace.db_path()).expect("open seeded DevQL SQLite");
    let (repo_id, artefact_id): (String, String) = conn
        .query_row(
            "SELECT repo_id, artefact_id FROM artefacts_current WHERE path = ?1 ORDER BY symbol_id LIMIT 1",
            [FIXTURE_FILE_PATH],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("fixture artefact row exists");

    conn.execute(
        r#"
INSERT INTO architecture_graph_nodes_current (
  repo_id, node_id, node_kind, label, artefact_id, symbol_id, path, entry_kind,
  source_kind, confidence, provenance_json, evidence_json, properties_json
) VALUES (
  ?1, 'test-architecture-node', 'NODE', 'UserRepository', ?2, NULL, ?3, NULL,
  'COMPUTED', 1.0, '{}', '[]', '{}'
)
ON CONFLICT(repo_id, node_id) DO UPDATE SET
  artefact_id = excluded.artefact_id,
  path = excluded.path,
  updated_at = datetime('now')
"#,
        params![repo_id, artefact_id, FIXTURE_FILE_PATH],
    )
    .expect("seed architecture graph node");
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
