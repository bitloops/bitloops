use super::*;

#[test]
fn extract_rust_artefacts_covers_phase1_kinds() {
    let content = r#"use std::fmt::Debug;

struct User {
    id: u64,
}

trait DoThing {
    fn do_it(&self);
}

impl DoThing for User {
    fn do_it(&self) {}
}

fn run() {
    println!("ok");
}
"#;
    let artefacts = extract_rust_artefacts(content, "src/lib.rs").unwrap();
    let kinds = artefacts
        .iter()
        .map(|a| a.canonical_kind.as_deref())
        .collect::<Vec<_>>();
    assert!(kinds.contains(&Some("import")));
    assert!(kinds.contains(&None));
    assert!(kinds.contains(&Some("interface")));
    assert!(kinds.contains(&Some("method")));
    assert!(kinds.contains(&Some("function")));

    let trait_item = artefacts
        .iter()
        .find(|a| a.language_kind == "trait_item" && a.name == "DoThing")
        .expect("expected trait artefact");
    assert_eq!(trait_item.canonical_kind.as_deref(), Some("interface"));

    let struct_item = artefacts
        .iter()
        .find(|a| a.language_kind == "struct_item" && a.name == "User")
        .expect("expected struct artefact");
    assert_eq!(struct_item.canonical_kind, None);

    let impl_item = artefacts
        .iter()
        .find(|a| a.language_kind == "impl_item")
        .expect("expected impl artefact");
    assert_eq!(impl_item.canonical_kind, None);
}

#[test]
fn extract_rust_dependency_edges_emits_import_calls_and_implements() {
    let content = r#"use crate::math::sum;

trait DoThing { fn do_it(&self); }
struct User;

impl DoThing for User {
    fn do_it(&self) {
        sum(1, 2);
    }
}
"#;
    let artefacts = extract_rust_artefacts(content, "src/lib.rs").unwrap();
    let edges = extract_rust_dependency_edges(content, "src/lib.rs", &artefacts).unwrap();

    assert!(edges.iter().any(|e| {
        e.edge_kind == "imports" && e.to_symbol_ref.as_deref() == Some("crate::math::sum")
    }));
    assert!(
        edges
            .iter()
            .any(|e| e.edge_kind == "implements" && e.to_symbol_ref.as_deref() == Some("DoThing"))
    );
    assert!(edges.iter().any(|e| e.edge_kind == "calls"));
}

#[test]
fn extract_rust_dependency_edges_are_ordered_and_keep_local_resolution_stable() {
    let content = r#"use crate::math::sum;
fn sum() {}
trait DoThing { fn do_it(&self); }
struct User;
impl DoThing for User {
    fn do_it(&self) {
        sum();
        missing();
    }
}
"#;
    let artefacts = extract_rust_artefacts(content, "src/lib.rs").unwrap();
    let snapshot = |edges: &[JsTsDependencyEdge]| {
        edges
            .iter()
            .map(|edge| {
                let metadata = |field: &str| {
                    edge.metadata
                        .get(field)
                        .and_then(|value| value.as_str())
                        .unwrap_or("-")
                };
                format!(
                    "{}|{}|{}|{}|{}|{}|{}",
                    edge.edge_kind,
                    edge.from_symbol_fqn,
                    edge.to_target_symbol_fqn.as_deref().unwrap_or("-"),
                    edge.to_symbol_ref.as_deref().unwrap_or("-"),
                    edge.start_line.unwrap_or_default(),
                    metadata("import_form"),
                    metadata("resolution"),
                )
            })
            .collect::<Vec<_>>()
    };

    let edges = extract_rust_dependency_edges(content, "src/lib.rs", &artefacts).unwrap();
    let edge_snapshot = snapshot(&edges);
    let repeated_snapshot =
        snapshot(&extract_rust_dependency_edges(content, "src/lib.rs", &artefacts).unwrap());

    assert_eq!(
        edge_snapshot,
        vec![
            "imports|src/lib.rs|-|crate::math::sum|1|binding|-".to_string(),
            "implements|src/lib.rs::impl@5|-|DoThing|5|-|-".to_string(),
            "calls|src/lib.rs::impl@5::do_it|src/lib.rs::sum|-|7|-|local".to_string(),
        ]
    );
    assert_eq!(edge_snapshot, repeated_snapshot);
}

#[test]
fn extract_rust_dependency_edges_emit_type_and_value_references_with_ref_kind() {
    let content = r#"struct User;
const DEFAULT_USER: User = User;

fn project(user: User) -> User {
    let current: User = DEFAULT_USER;
    current
}
"#;
    let artefacts = extract_rust_artefacts(content, "src/lib.rs").unwrap();
    let edges = extract_rust_dependency_edges(content, "src/lib.rs", &artefacts).unwrap();

    let type_reference = edges
        .iter()
        .find(|edge| {
            edge.edge_kind == "references"
                && edge.from_symbol_fqn == "src/lib.rs::project"
                && edge.to_target_symbol_fqn.as_deref() == Some("src/lib.rs::User")
        })
        .expect("expected local type reference edge for User");
    assert_eq!(
        type_reference
            .metadata
            .get("ref_kind")
            .and_then(|value| value.as_str()),
        Some("type")
    );

    let value_reference = edges
        .iter()
        .find(|edge| {
            edge.edge_kind == "references"
                && edge.from_symbol_fqn == "src/lib.rs::project"
                && edge.to_target_symbol_fqn.as_deref() == Some("src/lib.rs::DEFAULT_USER")
        })
        .expect("expected local value reference edge for DEFAULT_USER");
    assert_eq!(
        value_reference
            .metadata
            .get("ref_kind")
            .and_then(|value| value.as_str()),
        Some("value")
    );
}

#[test]
fn extract_rust_artefacts_preserve_expected_method_and_function_line_ranges() {
    let content = r##"impl AppServer {
    fn handle_factorial(&self, input: &str) -> Response<std::io::Cursor<Vec<u8>>> {
        match input.parse::<u64>() {
            Ok(n) if n <= 20 => {
                let result = factorial(n);
                Response::from_string(format!("{}! = {}\n", n, result))
            }
            Ok(_) => Response::from_string("Error: n must be <= 20\n")
                .with_status_code(400),
            Err(_) => Response::from_string("Error: invalid number\n")
                .with_status_code(400),
        }
    }
}

fn factorial(n: u64) -> u64 {
    (1..=n).product()
}
"##;

    let artefacts = extract_rust_artefacts(content, "src/main.rs").unwrap();
    let method = artefacts
        .iter()
        .find(|artefact| artefact.symbol_fqn == "src/main.rs::impl@1::handle_factorial")
        .expect("expected handle_factorial method artefact");
    assert_eq!(method.start_line, 2);
    assert_eq!(method.end_line, 13);

    let function = artefacts
        .iter()
        .find(|artefact| artefact.symbol_fqn == "src/main.rs::factorial")
        .expect("expected factorial function artefact");
    assert_eq!(function.start_line, 16);
    assert_eq!(function.end_line, 18);
}

#[test]
fn extract_rust_artefacts_collect_modifiers_and_outer_docstrings() {
    let content = r#"/// repository contract
pub(crate) trait Repository {
    fn save(&self);
}

/** stores the cache */
pub(crate) static CACHE: &str = "demo";

/// runs the worker
pub async unsafe fn run() {}
"#;

    let artefacts = extract_rust_artefacts(content, "src/lib.rs").unwrap();

    let trait_item = artefacts
        .iter()
        .find(|artefact| artefact.language_kind == "trait_item" && artefact.name == "Repository")
        .expect("expected trait artefact");
    assert_eq!(trait_item.modifiers, vec!["pub(crate)".to_string()]);
    assert_eq!(trait_item.docstring.as_deref(), Some("repository contract"));

    let static_item = artefacts
        .iter()
        .find(|artefact| artefact.language_kind == "static_item" && artefact.name == "CACHE")
        .expect("expected static artefact");
    assert_eq!(
        static_item.modifiers,
        vec!["pub(crate)".to_string(), "static".to_string()]
    );
    assert_eq!(static_item.docstring.as_deref(), Some("stores the cache"));

    let function = artefacts
        .iter()
        .find(|artefact| {
            artefact.language_kind == "function_item"
                && artefact.name == "run"
                && artefact.parent_symbol_fqn.is_none()
        })
        .expect("expected free function artefact");
    assert_eq!(
        function.modifiers,
        vec!["pub".to_string(), "async".to_string(), "unsafe".to_string()]
    );
    assert_eq!(function.docstring.as_deref(), Some("runs the worker"));
}

#[test]
fn extract_rust_inner_doc_comments_attach_to_file_and_module() {
    let content = r#"//! crate level docs
/*! more crate docs */
mod api {
    //! module docs
    /*! more module docs */
    pub fn call() {}
}
"#;

    assert_eq!(
        extract_rust_file_docstring(content).as_deref(),
        Some("crate level docs\n\nmore crate docs")
    );

    let artefacts = extract_rust_artefacts(content, "src/lib.rs").unwrap();
    let module = artefacts
        .iter()
        .find(|artefact| artefact.language_kind == "mod_item" && artefact.name == "api")
        .expect("expected module artefact");

    assert_eq!(
        module.docstring.as_deref(),
        Some("module docs\n\nmore module docs")
    );
}

#[test]
fn extract_rust_dependency_edges_emit_extends_for_supertraits() {
    let content = r#"trait Reader {}
trait Writer {}

trait Repository: Reader + Writer {
    fn load(&self);
}
"#;
    let artefacts = extract_rust_artefacts(content, "src/lib.rs").unwrap();
    let edges = extract_rust_dependency_edges(content, "src/lib.rs", &artefacts).unwrap();

    assert!(edges.iter().any(|edge| {
        edge.edge_kind == "extends"
            && edge.from_symbol_fqn == "src/lib.rs::Repository"
            && edge.to_target_symbol_fqn.as_deref() == Some("src/lib.rs::Reader")
    }));
    assert!(edges.iter().any(|edge| {
        edge.edge_kind == "extends"
            && edge.from_symbol_fqn == "src/lib.rs::Repository"
            && edge.to_target_symbol_fqn.as_deref() == Some("src/lib.rs::Writer")
    }));
}

#[test]
fn extract_rust_dependency_edges_emit_pub_use_exports_with_alias_distinct_dedup() {
    let content = r#"pub fn helper() {}

pub use self::helper;
pub use self::helper;
pub use self::helper as helper_alias;
pub use crate::support::Thing;
pub use crate::support::Thing;
pub use crate::support::Thing as RenamedThing;
"#;
    let artefacts = extract_rust_artefacts(content, "src/lib.rs").unwrap();
    let edges = extract_rust_dependency_edges(content, "src/lib.rs", &artefacts).unwrap();
    let export_edges = edges
        .iter()
        .filter(|edge| edge.edge_kind == "exports")
        .collect::<Vec<_>>();

    assert_eq!(
        export_edges.len(),
        4,
        "expected duplicate pub use exports to collapse while alias-distinct exports stay separate"
    );

    assert_eq!(
        export_edges
            .iter()
            .filter(|edge| {
                edge.from_symbol_fqn == "src/lib.rs"
                    && edge.to_target_symbol_fqn.as_deref() == Some("src/lib.rs::helper")
                    && edge
                        .metadata
                        .get("export_name")
                        .and_then(|value| value.as_str())
                        == Some("helper")
            })
            .count(),
        1,
        "duplicate local pub use edges for the same alias should dedupe"
    );

    let local_alias = export_edges
        .iter()
        .find(|edge| {
            edge.from_symbol_fqn == "src/lib.rs"
                && edge.to_target_symbol_fqn.as_deref() == Some("src/lib.rs::helper")
                && edge
                    .metadata
                    .get("export_name")
                    .and_then(|value| value.as_str())
                    == Some("helper_alias")
        })
        .expect("expected aliased local pub use edge for helper_alias");
    assert_eq!(
        local_alias
            .metadata
            .get("export_form")
            .and_then(|value| value.as_str()),
        Some("pub_use")
    );

    assert_eq!(
        export_edges
            .iter()
            .filter(|edge| {
                edge.from_symbol_fqn == "src/lib.rs"
                    && edge.to_symbol_ref.as_deref() == Some("crate::support::Thing")
                    && edge
                        .metadata
                        .get("export_name")
                        .and_then(|value| value.as_str())
                        == Some("Thing")
            })
            .count(),
        1,
        "duplicate external pub use edges for the same alias should dedupe"
    );

    let external_alias = export_edges
        .iter()
        .find(|edge| {
            edge.from_symbol_fqn == "src/lib.rs"
                && edge.to_symbol_ref.as_deref() == Some("crate::support::Thing")
                && edge
                    .metadata
                    .get("export_name")
                    .and_then(|value| value.as_str())
                    == Some("RenamedThing")
        })
        .expect("expected aliased external pub use edge for RenamedThing");
    assert_eq!(
        external_alias
            .metadata
            .get("export_form")
            .and_then(|value| value.as_str()),
        Some("pub_use")
    );
}

#[test]
fn extract_rust_dependency_edges_drop_unresolved_macro_calls_under_import_local_policy() {
    let content = r#"fn project() {
    println!("hi");
}
"#;
    let artefacts = extract_rust_artefacts(content, "src/lib.rs").unwrap();
    let edges = extract_rust_dependency_edges(content, "src/lib.rs", &artefacts).unwrap();

    assert!(
        !edges.iter().any(|edge| {
            edge.edge_kind == "calls" && edge.from_symbol_fqn == "src/lib.rs::project"
        }),
        "unresolved macro calls should be dropped under the import+local policy"
    );
}

#[test]
fn extract_rust_dependency_edges_keep_imported_helper_calls() {
    let content = r#"use crate::utils::slugify;

fn project(value: &str) {
    slugify(value);
}
"#;
    let artefacts = extract_rust_artefacts(content, "src/lib.rs").unwrap();
    let edges = extract_rust_dependency_edges(content, "src/lib.rs", &artefacts).unwrap();

    let imported_call = edges
        .iter()
        .find(|edge| {
            edge.edge_kind == "calls"
                && edge.from_symbol_fqn == "src/lib.rs::project"
                && edge.to_symbol_ref.as_deref() == Some("crate::utils::slugify")
        })
        .expect("expected imported helper call edge");

    assert_eq!(
        imported_call
            .metadata
            .get("call_form")
            .and_then(|value| value.as_str()),
        Some("function")
    );
    assert_eq!(
        imported_call
            .metadata
            .get("resolution")
            .and_then(|value| value.as_str()),
        Some("import")
    );
    assert_eq!(imported_call.start_line, Some(4));
    assert_eq!(imported_call.end_line, Some(4));
}

#[test]
fn extract_rust_dependency_edges_keep_local_call_and_drop_external_noise_in_method() {
    let content = r##"impl AppServer {
    fn handle_factorial(&self, input: &str) -> Response<std::io::Cursor<Vec<u8>>> {
        match input.parse::<u64>() {
            Ok(n) if n <= 20 => {
                let result = factorial(n);
                Response::from_string(format!("{}! = {}\n", n, result))
            }
            Ok(_) => Response::from_string("Error: n must be <= 20\n")
                .with_status_code(400),
            Err(_) => Response::from_string("Error: invalid number\n")
                .with_status_code(400),
        }
    }
}

fn factorial(n: u64) -> u64 {
    (1..=n).product()
}
"##;
    let artefacts = extract_rust_artefacts(content, "src/main.rs").unwrap();
    let edges = extract_rust_dependency_edges(content, "src/main.rs", &artefacts).unwrap();

    let local_factorial_call = edges
        .iter()
        .find(|edge| {
            edge.edge_kind == "calls"
                && edge.from_symbol_fqn == "src/main.rs::impl@1::handle_factorial"
                && edge.to_target_symbol_fqn.as_deref() == Some("src/main.rs::factorial")
        })
        .expect("expected local factorial call edge");

    assert_eq!(
        local_factorial_call
            .metadata
            .get("call_form")
            .and_then(|value| value.as_str()),
        Some("function")
    );
    assert_eq!(
        local_factorial_call
            .metadata
            .get("resolution")
            .and_then(|value| value.as_str()),
        Some("local")
    );
    assert_eq!(local_factorial_call.start_line, Some(5));
    assert_eq!(local_factorial_call.end_line, Some(5));

    assert!(
        !edges.iter().any(|edge| {
            edge.edge_kind == "calls"
                && edge.to_symbol_ref.as_deref().is_some_and(|target| {
                    target.contains("::<u64>")
                        || target.contains("from_string(\"")
                        || target.contains(".with_status_code")
                })
        }),
        "rust call edges should not contain generic fragments or chained receiver text"
    );

    assert!(
        !edges.iter().any(|edge| {
            edge.edge_kind == "calls"
                && edge
                    .metadata
                    .get("resolution")
                    .and_then(|value| value.as_str())
                    == Some("unresolved")
        }),
        "unresolved rust call edges should be filtered out under the import+local policy"
    );
}

// Case 3: Rust associated call `Type::fn()` must NOT also emit a references edge for the type.
// `AppServer::new(...)` produces a calls(associated) edge; `AppServer` as the scoped-identifier
// path qualifier must not additionally produce a references edge.
#[test]
fn extract_rust_dependency_edges_associated_call_does_not_emit_redundant_references_edge() {
    let content = r#"struct AppServer {
    host: String,
}

impl AppServer {
    fn new(host: &str, port: u16) -> Self {
        AppServer { host: host.to_string() }
    }
}

fn boot() {
    let server = AppServer::new("127.0.0.1", 8080);
}
"#;
    let artefacts = extract_rust_artefacts(content, "src/lib.rs").unwrap();
    let edges = extract_rust_dependency_edges(content, "src/lib.rs", &artefacts).unwrap();

    // calls edge must be present
    assert!(
        edges.iter().any(|e| {
            e.edge_kind == "calls"
                && e.from_symbol_fqn == "src/lib.rs::boot"
                && e.to_target_symbol_fqn
                    .as_deref()
                    .is_some_and(|t| t.contains("new"))
        }),
        "expected a calls edge for AppServer::new from boot"
    );

    // no spurious references edge for AppServer from the call site
    assert!(
        !edges.iter().any(|e| {
            e.edge_kind == "references"
                && e.from_symbol_fqn == "src/lib.rs::boot"
                && e.to_target_symbol_fqn
                    .as_deref()
                    .is_some_and(|t| t.contains("AppServer"))
        }),
        "AppServer::new() call should not also emit a references edge for AppServer (duplicate)"
    );
}
