use crate::fixtures::{extract_connection_nodes, run_query_json, seeded_rust_graphql_workspace};
use serde_json::Value;

#[test]
fn bitloops_devql_query_dsl_matches_raw_graphql_output_end_to_end() {
    let seeded = seeded_rust_graphql_workspace("graphql-cli-parity");
    let dsl_output = run_query_json(
        &seeded.workspace,
        &[
            "devql",
            "query",
            "--compact",
            r#"repo("graphql-cli-parity")->artefacts()->select(path,canonical_kind,symbol_fqn)->limit(10)"#,
        ],
    );

    let raw_query = format!(
        r#"{{ repo(name: "{repo_name}") {{ artefacts(first: 10) {{ edges {{ node {{ path canonicalKind symbolFqn }} }} }} }} }}"#,
        repo_name = seeded.repo_name
    );
    let raw_output = run_query_json(
        &seeded.workspace,
        &["devql", "query", "--graphql", "--compact", &raw_query],
    );

    let raw_nodes = Value::Array(extract_connection_nodes(&raw_output));
    assert!(
        raw_nodes.as_array().is_some_and(|rows| !rows.is_empty()),
        "expected seeded GraphQL query to return artefacts"
    );
    assert_eq!(dsl_output, raw_nodes);
}

#[test]
fn bitloops_devql_query_accepts_graphql_as_default_input_mode_end_to_end() {
    let seeded = seeded_rust_graphql_workspace("graphql-cli-default");
    let query = format!(
        r#"{{ repo(name: "{repo_name}") {{ artefacts(first: 2) {{ edges {{ node {{ path symbolFqn canonicalKind }} }} }} }} }}"#,
        repo_name = seeded.repo_name
    );

    let default_output =
        run_query_json(&seeded.workspace, &["devql", "query", "--compact", &query]);
    let explicit_output = run_query_json(
        &seeded.workspace,
        &["devql", "query", "--graphql", "--compact", &query],
    );

    assert_eq!(default_output, explicit_output);
}
