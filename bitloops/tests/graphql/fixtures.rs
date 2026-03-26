use crate::test_harness_support::{
    Workspace, prepare_graphql_workspace, run_bitloops_or_panic, seed_production_artefacts,
    write_rust_static_link_fixture,
};
use serde_json::Value;

pub struct SeededGraphqlWorkspace {
    pub workspace: Workspace,
    pub repo_name: String,
}

pub fn seeded_rust_graphql_workspace(name: &str) -> SeededGraphqlWorkspace {
    let workspace = Workspace::new(name);
    write_rust_static_link_fixture(&workspace);
    prepare_graphql_workspace(&workspace);
    seed_production_artefacts(&workspace, "C0");

    let repo_name = workspace
        .repo_dir()
        .file_name()
        .and_then(|name| name.to_str())
        .expect("workspace repo dir should have a UTF-8 file name")
        .to_string();

    SeededGraphqlWorkspace {
        workspace,
        repo_name,
    }
}

pub fn run_query_json(workspace: &Workspace, args: &[&str]) -> Value {
    serde_json::from_str(&run_bitloops_or_panic(workspace.repo_dir(), args))
        .expect("bitloops output should be valid JSON")
}

pub fn extract_connection_nodes(payload: &Value) -> Vec<Value> {
    payload["repo"]["artefacts"]["edges"]
        .as_array()
        .expect("artefact connection edges")
        .iter()
        .map(|edge| edge["node"].clone())
        .collect()
}
