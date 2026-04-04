use super::*;

#[test]
fn devql_extension_host_resolves_built_in_language_pack_ownership() {
    assert_eq!(
        resolve_language_pack_owner("rust"),
        Some(RUST_LANGUAGE_PACK_ID)
    );
    assert_eq!(
        resolve_language_pack_owner("typescript"),
        Some(TS_JS_LANGUAGE_PACK_ID)
    );
    assert_eq!(
        resolve_language_pack_owner("javascript"),
        Some(TS_JS_LANGUAGE_PACK_ID)
    );
    assert_eq!(
        resolve_language_pack_owner("python"),
        Some(PYTHON_LANGUAGE_PACK_ID)
    );
    assert_eq!(resolve_language_pack_owner("go"), Some(GO_LANGUAGE_PACK_ID));
    assert_eq!(
        resolve_language_pack_owner("java"),
        Some(JAVA_LANGUAGE_PACK_ID)
    );
    assert_eq!(
        resolve_language_id_for_file_path("src/lib.rs"),
        Some("rust")
    );
    assert_eq!(
        resolve_language_id_for_file_path("src/main.ts"),
        Some("typescript")
    );
    assert_eq!(
        resolve_language_id_for_file_path("src/main.jsx"),
        Some("javascript")
    );
    assert_eq!(
        resolve_language_id_for_file_path("src/main.py"),
        Some("python")
    );
    assert_eq!(resolve_language_id_for_file_path("src/main.go"), Some("go"));
    assert_eq!(
        resolve_language_id_for_file_path("src/Main.java"),
        Some("java")
    );
    assert!(resolve_language_id_for_file_path("README").is_none());
}

#[test]
fn devql_language_adapter_registry_resolves_built_in_pack_implementations() {
    let registry = language_adapter_registry().expect("initialize language adapter registry");
    assert_eq!(
        registry.registered_pack_ids(),
        vec![
            GO_LANGUAGE_PACK_ID,
            JAVA_LANGUAGE_PACK_ID,
            PYTHON_LANGUAGE_PACK_ID,
            RUST_LANGUAGE_PACK_ID,
            TS_JS_LANGUAGE_PACK_ID
        ]
    );
    assert!(registry.get(GO_LANGUAGE_PACK_ID).is_some());
    assert!(registry.get(JAVA_LANGUAGE_PACK_ID).is_some());
    assert!(registry.get(RUST_LANGUAGE_PACK_ID).is_some());
    assert!(registry.get(TS_JS_LANGUAGE_PACK_ID).is_some());
    assert!(registry.get(PYTHON_LANGUAGE_PACK_ID).is_some());
    assert!(registry.get("unknown-pack").is_none());
}

#[test]
fn devql_language_adapter_registry_executes_rust_ts_js_python_go_and_java_built_ins() {
    let registry = language_adapter_registry().expect("initialize language adapter registry");
    let rust_pack = registry
        .get(RUST_LANGUAGE_PACK_ID)
        .expect("resolve rust built-in language adapter pack");
    let rust_content = r#"//! crate docs
fn greet() {
    helper();
}

fn helper() {}
"#;
    let rust_artefacts = rust_pack
        .extract_artefacts(rust_content, "src/lib.rs")
        .expect("extract rust artefacts via language adapter registry");
    assert!(
        rust_artefacts
            .iter()
            .any(|artefact| artefact.name == "greet"),
        "rust built-in registry pack should surface function artefacts"
    );
    assert!(
        rust_pack.extract_file_docstring(rust_content).is_some(),
        "rust built-in registry pack should expose crate-level docstrings"
    );

    let ts_pack = registry
        .get(TS_JS_LANGUAGE_PACK_ID)
        .expect("resolve ts/js built-in language adapter pack");
    let ts_content = r#"export function greet() {
    return helper();
}

function helper() {
    return 1;
}
"#;
    let ts_artefacts = ts_pack
        .extract_artefacts(ts_content, "src/main.ts")
        .expect("extract ts artefacts via language adapter registry");
    assert!(
        ts_artefacts.iter().any(|artefact| artefact.name == "greet"),
        "ts/js built-in registry pack should surface function artefacts"
    );
    let ts_edges = ts_pack
        .extract_dependency_edges(ts_content, "src/main.ts", &ts_artefacts)
        .expect("extract ts dependency edges via language adapter registry");
    assert!(
        ts_edges
            .iter()
            .any(|edge| edge.edge_kind == EdgeKind::Calls),
        "ts/js built-in registry pack should emit call edges"
    );

    let python_pack = registry
        .get(PYTHON_LANGUAGE_PACK_ID)
        .expect("resolve python built-in language adapter pack");
    let python_content = r#"
"""module docs"""

from pkg.helpers import helper

class Greeter(BaseGreeter):
    def greet(self):
        return helper()

def run():
    return helper()
"#;
    let python_artefacts = python_pack
        .extract_artefacts(python_content, "src/main.py")
        .expect("extract python artefacts via language adapter registry");
    assert!(
        python_artefacts
            .iter()
            .any(|artefact| artefact.name == "run"),
        "python built-in registry pack should surface function artefacts"
    );
    assert!(
        python_pack.extract_file_docstring(python_content).is_some(),
        "python built-in registry pack should expose module docstrings"
    );
    let python_edges = python_pack
        .extract_dependency_edges(python_content, "src/main.py", &python_artefacts)
        .expect("extract python dependency edges via language adapter registry");
    assert!(
        python_edges
            .iter()
            .any(|edge| edge.edge_kind == EdgeKind::Calls),
        "python built-in registry pack should emit call edges"
    );
    assert!(
        python_edges
            .iter()
            .any(|edge| edge.edge_kind == EdgeKind::Imports),
        "python built-in registry pack should emit import edges"
    );
    assert!(
        python_edges
            .iter()
            .any(|edge| edge.edge_kind == EdgeKind::Extends),
        "python built-in registry pack should emit extends edges"
    );

    let go_pack = registry
        .get(GO_LANGUAGE_PACK_ID)
        .expect("resolve go built-in language adapter pack");
    let go_content = r#"package service

import (
    "context"
    "net/http"
)

type Base interface {
    Run(context.Context) error
}

type Handler struct {
    Base
}

func helper() {}

func Run() {
    helper()
    http.ListenAndServe(":8080", nil)
}
"#;
    let go_artefacts = go_pack
        .extract_artefacts(go_content, "service/run.go")
        .expect("extract go artefacts via language adapter registry");
    assert!(
        go_artefacts.iter().any(|artefact| artefact.name == "Run"),
        "go built-in registry pack should surface function artefacts"
    );
    let go_edges = go_pack
        .extract_dependency_edges(go_content, "service/run.go", &go_artefacts)
        .expect("extract go dependency edges via language adapter registry");
    assert!(
        go_edges
            .iter()
            .any(|edge| edge.edge_kind == EdgeKind::Calls),
        "go built-in registry pack should emit call edges"
    );
    assert!(
        go_edges
            .iter()
            .any(|edge| edge.edge_kind == EdgeKind::Imports),
        "go built-in registry pack should emit import edges"
    );
    assert!(
        go_edges
            .iter()
            .any(|edge| edge.edge_kind == EdgeKind::Extends),
        "go built-in registry pack should emit embedding edges"
    );

    let java_pack = registry
        .get(JAVA_LANGUAGE_PACK_ID)
        .expect("resolve java built-in language adapter pack");
    let java_content = r#"package com.acme;

import java.util.List;

class Base {}
interface Runner {}

/**
 * Greeter docs
 */
class Greeter extends Base implements Runner {
    private int count;

    Greeter() {}

    void helper() {}

    void greet(List<String> names) {
        helper();
        System.out.println(names.size());
        new Base();
    }
}
"#;
    let java_artefacts = java_pack
        .extract_artefacts(java_content, "src/com/acme/Greeter.java")
        .expect("extract java artefacts via language adapter registry");
    assert!(
        java_artefacts
            .iter()
            .any(|artefact| artefact.name == "Greeter"),
        "java built-in registry pack should surface type artefacts"
    );
    assert!(
        java_pack.extract_file_docstring(java_content).is_some(),
        "java built-in registry pack should expose file docstrings"
    );
    let java_edges = java_pack
        .extract_dependency_edges(java_content, "src/com/acme/Greeter.java", &java_artefacts)
        .expect("extract java dependency edges via language adapter registry");
    assert!(
        java_edges
            .iter()
            .any(|edge| edge.edge_kind == EdgeKind::Calls),
        "java built-in registry pack should emit call edges"
    );
    assert!(
        java_edges
            .iter()
            .any(|edge| edge.edge_kind == EdgeKind::Imports),
        "java built-in registry pack should emit import edges"
    );
    assert!(
        java_edges
            .iter()
            .any(|edge| edge.edge_kind == EdgeKind::Extends),
        "java built-in registry pack should emit extends edges"
    );
    assert!(
        java_edges
            .iter()
            .any(|edge| edge.edge_kind == EdgeKind::Implements),
        "java built-in registry pack should emit implements edges"
    );
}
