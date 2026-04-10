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

#[test]
fn extract_java_dependency_edges_deduplicates_repeated_imports_in_one_file() {
    let content = r#"package com.acme;

import java.util.List;
import java.util.List;

class Greeter {
    void greet(List<String> names) {}
}
"#;

    let artefacts = extract_java_artefacts(content, "src/com/acme/Greeter.java").unwrap();
    let edges =
        extract_java_dependency_edges(content, "src/com/acme/Greeter.java", &artefacts).unwrap();

    let import_edges = edges
        .iter()
        .filter(|edge| {
            edge.edge_kind == EdgeKind::Imports
                && edge.to_symbol_ref.as_deref() == Some("java.util.List")
        })
        .count();

    assert_eq!(import_edges, 1);
}

#[test]
fn extract_java_dependency_edges_attribute_field_type_reference_to_exact_field_owner() {
    let content = r#"package com.acme;

import java.util.List;

class A {
    List<String> xs;
}

class B {
    List<String> ys;
}
"#;

    let path = "src/com/acme/Greeter.java";
    let artefacts = extract_java_artefacts(content, path).unwrap();
    let edges = extract_java_dependency_edges(content, path, &artefacts).unwrap();

    assert!(edges.iter().any(|edge| {
        edge.edge_kind == EdgeKind::References
            && edge.from_symbol_fqn == "src/com/acme/Greeter.java::B::ys"
            && edge.to_symbol_ref.as_deref() == Some("java.util.List")
    }));
}
