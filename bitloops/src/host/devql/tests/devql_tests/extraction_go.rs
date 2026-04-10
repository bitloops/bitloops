use crate::adapters::languages::go::edges::extract_go_dependency_edges;
use crate::adapters::languages::go::extraction::extract_go_artefacts;
use crate::host::devql::EdgeKind;
use crate::host::language_adapter::DependencyEdge;

fn edges_for(content: &str) -> Vec<DependencyEdge> {
    let path = "service/run.go";
    let artefacts = extract_go_artefacts(content, path).unwrap();
    extract_go_dependency_edges(content, path, &artefacts).unwrap()
}

// Precision tests verify the expected edge kind, target, and metadata.
#[test]
fn extract_go_import_edges_are_precise() {
    let content = r#"package service

        import (
            "context"
            "net/http"
        )
        "#;

    let edges = edges_for(content);
    let import_edges = edges
        .iter()
        .filter(|edge| edge.edge_kind == EdgeKind::Imports)
        .collect::<Vec<_>>();

    assert_eq!(import_edges.len(), 2);
    assert_eq!(import_edges[0].from_symbol_fqn, "service/run.go");
    assert_eq!(import_edges[0].to_symbol_ref.as_deref(), Some("context"));
    assert_eq!(import_edges[0].metadata["import_form"], "binding");
    assert_eq!(import_edges[1].from_symbol_fqn, "service/run.go");
    assert_eq!(import_edges[1].to_symbol_ref.as_deref(), Some("net/http"));
    assert_eq!(import_edges[1].metadata["import_form"], "binding");
}

#[test]
fn extract_go_call_edges_distinguish_local_import_and_unresolved_calls() {
    let content = r#"package service

        import "net/http"

        func helper() {}

        func Run() {
            helper()
            http.ListenAndServe(":8080", nil)
            missing()
        }
        "#;

    let edges = edges_for(content);
    let call_edges = edges
        .iter()
        .filter(|edge| edge.edge_kind == EdgeKind::Calls)
        .collect::<Vec<_>>();

    assert_eq!(call_edges.len(), 3);
    assert!(call_edges.iter().any(|edge| {
        edge.from_symbol_fqn == "service/run.go::Run"
            && edge.to_target_symbol_fqn.as_deref() == Some("service/run.go::helper")
            && edge.metadata["call_form"] == "function"
            && edge.metadata["resolution"] == "local"
    }));
    assert!(call_edges.iter().any(|edge| {
        edge.from_symbol_fqn == "service/run.go::Run"
            && edge.to_symbol_ref.as_deref() == Some("net/http::ListenAndServe")
            && edge.metadata["call_form"] == "associated"
            && edge.metadata["resolution"] == "import"
    }));
    assert!(call_edges.iter().any(|edge| {
        edge.from_symbol_fqn == "service/run.go::Run"
            && edge.to_symbol_ref.as_deref() == Some("package::service::missing")
            && edge.metadata["call_form"] == "function"
            && edge.metadata["resolution"] == "unresolved"
    }));
}

#[test]
fn extract_go_call_edges_resolve_receiver_methods() {
    let content = r#"package service

        type Handler struct{}

        func (h *Handler) ServeHTTP() {}

        func Run() {
            Handler{}.ServeHTTP()
        }
        "#;

    let edges = edges_for(content);
    let call_edges = edges
        .iter()
        .filter(|edge| edge.edge_kind == EdgeKind::Calls)
        .collect::<Vec<_>>();

    assert_eq!(call_edges.len(), 1);
    assert_eq!(call_edges[0].from_symbol_fqn, "service/run.go::Run");
    assert_eq!(
        call_edges[0].to_target_symbol_fqn.as_deref(),
        Some("service/run.go::Handler::ServeHTTP")
    );
    assert_eq!(call_edges[0].metadata["call_form"], "method");
    assert_eq!(call_edges[0].metadata["resolution"], "local");
}

#[test]
fn extract_go_reference_edges_cover_local_and_imported_types() {
    let content = r#"package service

        import "context"

        type Base struct{}

        func Run(
            ctx context.Context,
            base Base,
        ) Base {
            return base
        }
        "#;

    let edges = edges_for(content);
    let reference_edges = edges
        .iter()
        .filter(|edge| edge.edge_kind == EdgeKind::References)
        .collect::<Vec<_>>();

    assert_eq!(reference_edges.len(), 3);
    assert!(reference_edges.iter().any(|edge| {
        edge.from_symbol_fqn == "service/run.go::Run"
            && edge.to_symbol_ref.as_deref() == Some("context::Context")
            && edge.metadata["ref_kind"] == "type"
            && edge.metadata["resolution"] == "import"
    }));
    let local_base_refs = reference_edges
        .iter()
        .filter(|edge| {
            edge.from_symbol_fqn == "service/run.go::Run"
                && edge.to_target_symbol_fqn.as_deref() == Some("service/run.go::Base")
                && edge.metadata["ref_kind"] == "type"
                && edge.metadata["resolution"] == "local"
        })
        .count();
    assert_eq!(local_base_refs, 2);
}

#[test]
fn extract_go_embedding_edges_cover_local_and_imported_embeddings() {
    let content = r#"package service

        import "io"

        type Base interface {
            Run()
        }

        type Derived interface {
            Base
            io.Reader
        }

        type Handler struct {
            Base
            io.Closer
        }
        "#;

    let edges = edges_for(content);
    let extends_edges = edges
        .iter()
        .filter(|edge| edge.edge_kind == EdgeKind::Extends)
        .collect::<Vec<_>>();

    assert_eq!(extends_edges.len(), 4);
    assert!(extends_edges.iter().any(|edge| {
        edge.from_symbol_fqn == "service/run.go::Derived"
            && edge.to_target_symbol_fqn.as_deref() == Some("service/run.go::Base")
    }));
    assert!(extends_edges.iter().any(|edge| {
        edge.from_symbol_fqn == "service/run.go::Derived"
            && edge.to_symbol_ref.as_deref() == Some("io::Reader")
    }));
    assert!(extends_edges.iter().any(|edge| {
        edge.from_symbol_fqn == "service/run.go::Handler"
            && edge.to_target_symbol_fqn.as_deref() == Some("service/run.go::Base")
    }));
    assert!(extends_edges.iter().any(|edge| {
        edge.from_symbol_fqn == "service/run.go::Handler"
            && edge.to_symbol_ref.as_deref() == Some("io::Closer")
    }));
}

// Duplicate-edge tests verify one logical relationship is emitted only once.
#[test]
fn extract_go_call_edges_do_not_duplicate_same_local_call() {
    let content = r#"package service

        func helper() {}

        func Run() {
            helper()
            helper()
        }
        "#;

    let edges = edges_for(content);
    let local_helper_calls = edges
        .iter()
        .filter(|edge| {
            edge.edge_kind == EdgeKind::Calls
                && edge.from_symbol_fqn == "service/run.go::Run"
                && edge.to_target_symbol_fqn.as_deref() == Some("service/run.go::helper")
                && edge.metadata["resolution"] == "local"
        })
        .count();

    assert_eq!(local_helper_calls, 2);
}

#[test]
fn extract_go_import_edges_do_not_duplicate_same_import_path() {
    let content = r#"package service

import (
    "context"
    "context"
)
"#;

    let edges = edges_for(content);
    let context_import_edges = edges
        .iter()
        .filter(|edge| {
            edge.edge_kind == EdgeKind::Imports
                && edge.from_symbol_fqn == "service/run.go"
                && edge.to_symbol_ref.as_deref() == Some("context")
        })
        .count();

    assert_eq!(context_import_edges, 1);
}

// Overlap tests verify one syntax usage does not become two different edge kinds.
#[test]
fn extract_go_callee_identifiers_do_not_also_emit_reference_edges() {
    let content = r#"package service

        func helper() {}

        func Run() {
            helper()
        }
        "#;

    let edges = edges_for(content);
    let helper_call_edges = edges
        .iter()
        .filter(|edge| {
            edge.edge_kind == EdgeKind::Calls
                && edge.from_symbol_fqn == "service/run.go::Run"
                && edge.to_target_symbol_fqn.as_deref() == Some("service/run.go::helper")
        })
        .count();
    let helper_reference_edges = edges
        .iter()
        .filter(|edge| {
            edge.edge_kind == EdgeKind::References
                && edge.from_symbol_fqn == "service/run.go::Run"
                && (edge.to_target_symbol_fqn.as_deref() == Some("service/run.go::helper")
                    || edge.to_symbol_ref.as_deref() == Some("package::service::helper"))
        })
        .count();

    assert_eq!(helper_call_edges, 1);
    assert_eq!(helper_reference_edges, 0);
}

#[test]
fn extract_go_qualified_type_references_do_not_duplicate_single_imported_type_usage() {
    let content = r#"package service

import "context"

func Run(ctx context.Context) {}
"#;

    let edges = edges_for(content);
    let context_reference_edges = edges
        .iter()
        .filter(|edge| {
            edge.edge_kind == EdgeKind::References
                && edge.from_symbol_fqn == "service/run.go::Run"
                && edge.to_symbol_ref.as_deref() == Some("context::Context")
                && edge.metadata["ref_kind"] == "type"
                && edge.metadata["resolution"] == "import"
        })
        .count();

    assert_eq!(context_reference_edges, 1);
}

#[test]
fn extract_go_embedding_edges_do_not_duplicate_same_embedded_type() {
    let content = r#"package service

        type Base interface {
            Run()
        }

        type Handler struct {
            Base
            Base
        }
        "#;

    let edges = edges_for(content);
    let base_embedding_edges = edges
        .iter()
        .filter(|edge| {
            edge.edge_kind == EdgeKind::Extends
                && edge.from_symbol_fqn == "service/run.go::Handler"
                && edge.to_target_symbol_fqn.as_deref() == Some("service/run.go::Base")
        })
        .count();

    assert_eq!(base_embedding_edges, 1);
}

#[test]
fn extract_go_embedding_edges_do_not_duplicate_same_imported_embedded_type() {
    let content = r#"package service

import "io"

type Handler struct {
    io.Reader
    io.Reader
}
"#;

    let edges = edges_for(content);
    let imported_embedding_edges = edges
        .iter()
        .filter(|edge| {
            edge.edge_kind == EdgeKind::Extends
                && edge.from_symbol_fqn == "service/run.go::Handler"
                && edge.to_symbol_ref.as_deref() == Some("io::Reader")
        })
        .count();

    assert_eq!(imported_embedding_edges, 1);
}

#[test]
fn extract_go_method_calls_do_not_emit_fallback_member_edges_when_resolved() {
    let content = r#"package service

        type Handler struct{}

        func (h *Handler) ServeHTTP() {}

        func Run() {
            Handler{}.ServeHTTP()
        }
        "#;

    let edges = edges_for(content);
    let method_call_edges = edges
        .iter()
        .filter(|edge| {
            edge.edge_kind == EdgeKind::Calls
                && edge.from_symbol_fqn == "service/run.go::Run"
                && edge.to_target_symbol_fqn.as_deref()
                    == Some("service/run.go::Handler::ServeHTTP")
                && edge.metadata["call_form"] == "method"
        })
        .count();
    let fallback_member_edges = edges
        .iter()
        .filter(|edge| {
            edge.edge_kind == EdgeKind::Calls
                && edge.from_symbol_fqn == "service/run.go::Run"
                && edge.to_symbol_ref.as_deref() == Some("package::service::Handler::ServeHTTP")
                && edge.metadata["call_form"] == "member"
        })
        .count();

    assert_eq!(method_call_edges, 1);
    assert_eq!(fallback_member_edges, 0);
}
