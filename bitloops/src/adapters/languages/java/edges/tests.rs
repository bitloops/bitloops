use super::extract_java_dependency_edges;
use crate::adapters::languages::java::extraction::extract_java_artefacts;
use crate::host::devql::EdgeKind;

#[test]
fn extract_java_dependency_edges_emit_import_call_extends_and_implements_edges() {
    let content = r#"package com.acme;

import java.util.List;

class Base {}
interface Runner {}

class Greeter extends Base implements Runner {
    private Base base;

    Greeter() {
        this();
    }

    void helper() {}

    void greet(List<String> names) {
        helper();
        System.out.println(names.size());
        new Base();
    }
}
"#;

    let artefacts = extract_java_artefacts(content, "src/com/acme/Greeter.java").unwrap();
    let edges =
        extract_java_dependency_edges(content, "src/com/acme/Greeter.java", &artefacts).unwrap();

    assert!(edges.iter().any(|edge| edge.edge_kind == EdgeKind::Imports));
    assert!(edges.iter().any(|edge| edge.edge_kind == EdgeKind::Calls));
    assert!(edges.iter().any(|edge| edge.edge_kind == EdgeKind::Extends));
    assert!(
        edges
            .iter()
            .any(|edge| edge.edge_kind == EdgeKind::Implements)
    );
    assert!(
        edges
            .iter()
            .any(|edge| edge.edge_kind == EdgeKind::References)
    );
}
