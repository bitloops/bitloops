use super::*;

#[test]
fn extract_js_ts_functions_detects_basic_function() {
    let content = r#"export function hello() {
  return "Hello World";
}
"#;
    let functions = extract_js_ts_functions(content).unwrap();
    assert_eq!(functions.len(), 1);
    assert_eq!(functions[0].name, "hello");
    assert_eq!(functions[0].start_line, 1);
    assert_eq!(functions[0].end_line, 3);
    assert_eq!(functions[0].start_byte, 0);
    assert_eq!(functions[0].end_byte as usize, content.len());
    assert_eq!(functions[0].signature, "export function hello() {");
}

#[test]
fn extract_js_ts_functions_detects_arrow_function_assignment() {
    let content = r#"export const hello = () => {
  return "Hello World";
}
"#;
    let functions = extract_js_ts_functions(content).unwrap();
    assert_eq!(functions.len(), 1);
    assert_eq!(functions[0].name, "hello");
    assert_eq!(functions[0].start_line, 1);
    assert_eq!(functions[0].end_line, 3);
    assert_eq!(functions[0].start_byte, 0);
    assert_eq!(functions[0].end_byte as usize, content.len());
    assert_eq!(functions[0].signature, "export const hello = () => {");
}

#[test]
fn extract_js_ts_artefacts_covers_phase1_kinds() {
    let content = r#"import { helper } from "./helper";
export interface User {
  id: string;
}
export type UserId = string;
export class Service {
  run(input: string) {
    return input;
  }
}
export const answer = 42;
export function greet(name: string) {
  return helper(name);
}
"#;

    let artefacts = extract_js_ts_artefacts(content, "src/sample.ts").unwrap();
    let kinds = artefacts
        .iter()
        .map(|a| a.canonical_kind.as_deref())
        .collect::<Vec<_>>();

    assert!(kinds.contains(&Some("import")));
    assert!(kinds.contains(&Some("interface")));
    assert!(kinds.contains(&Some("type")));
    assert!(kinds.contains(&None));
    assert!(kinds.contains(&Some("method")));
    assert!(kinds.contains(&Some("variable")));
    assert!(kinds.contains(&Some("function")));

    let class = artefacts
        .iter()
        .find(|a| a.language_kind == "class_declaration" && a.name == "Service")
        .expect("expected class artefact");
    assert_eq!(class.canonical_kind, None);

    let method = artefacts
        .iter()
        .find(|a| a.canonical_kind.as_deref() == Some("method") && a.name == "run")
        .expect("expected class method artefact");
    assert_eq!(
        method.parent_symbol_fqn.as_deref(),
        Some("src/sample.ts::Service")
    );
    assert_eq!(method.symbol_fqn, "src/sample.ts::Service::run");
}

#[test]
fn extract_js_ts_artefacts_emits_constructor_and_only_top_level_variables() {
    let content = r#"import { helper } from "./helper";
const cacheKey = "demo";
export const API_URL = "/v1";
interface User {
  id: string;
}
type UserId = string;
class Service {
  constructor(private readonly value: string) {}

  run() {
    const localOnly = this.value;
    return helper(localOnly);
  }
}
function boot() {
  const nestedOnly = 1;
  return nestedOnly;
}
"#;

    let artefacts = extract_js_ts_artefacts(content, "src/sample.ts").unwrap();

    let constructor = artefacts
        .iter()
        .find(|a| a.language_kind == "constructor" && a.name == "constructor")
        .expect("expected constructor artefact");
    assert_eq!(constructor.canonical_kind, None);
    assert_eq!(
        constructor.parent_symbol_fqn.as_deref(),
        Some("src/sample.ts::Service")
    );

    assert!(
        artefacts
            .iter()
            .any(|a| a.language_kind == "variable_declarator" && a.name == "cacheKey")
    );
    assert!(
        artefacts
            .iter()
            .any(|a| a.language_kind == "variable_declarator" && a.name == "API_URL")
    );
    assert!(
        !artefacts
            .iter()
            .any(|a| a.language_kind == "variable_declarator" && a.name == "localOnly")
    );
    assert!(
        !artefacts
            .iter()
            .any(|a| a.language_kind == "variable_declarator" && a.name == "nestedOnly")
    );
}

#[test]
fn extract_js_ts_artefacts_returns_no_symbols_when_treesitter_parse_fails() {
    let content = "export function broken( {";

    let artefacts = extract_js_ts_artefacts(content, "src/broken.ts").unwrap();

    assert!(artefacts.is_empty());
}

#[test]
fn extract_js_ts_artefacts_collect_modifiers_for_methods_fields_and_variables() {
    let content = r#"/* class summary */
class Service {
  // field summary
  public static readonly value: string = "ok";

  // method summary
  public static async run() {
    return Promise.resolve();
  }
}

// variable summary
export const FLAG = "demo";
"#;

    let artefacts = extract_js_ts_artefacts(content, "src/sample.ts").unwrap();

    let class = artefacts
        .iter()
        .find(|artefact| {
            artefact.language_kind == "class_declaration" && artefact.name == "Service"
        })
        .expect("expected class artefact");
    assert!(class.modifiers.is_empty());
    assert_eq!(class.docstring.as_deref(), Some("class summary"));

    let field = artefacts
        .iter()
        .find(|artefact| {
            artefact.language_kind == "public_field_definition" && artefact.name == "value"
        })
        .expect("expected field artefact");
    assert_eq!(
        field.modifiers,
        vec![
            "public".to_string(),
            "static".to_string(),
            "readonly".to_string()
        ]
    );
    assert_eq!(field.docstring.as_deref(), Some("field summary"));

    let method = artefacts
        .iter()
        .find(|artefact| artefact.language_kind == "method_definition" && artefact.name == "run")
        .expect("expected method artefact");
    assert_eq!(
        method.modifiers,
        vec![
            "public".to_string(),
            "static".to_string(),
            "async".to_string()
        ]
    );
    assert_eq!(method.docstring.as_deref(), Some("method summary"));

    let variable = artefacts
        .iter()
        .find(|artefact| artefact.language_kind == "variable_declarator" && artefact.name == "FLAG")
        .expect("expected variable artefact");
    assert_eq!(variable.modifiers, vec!["export".to_string()]);
    assert_eq!(variable.docstring.as_deref(), Some("variable summary"));
}

#[test]
fn extract_js_ts_artefacts_merge_mixed_docstring_comment_blocks() {
    let content = r#"// first line
// second line
/* block detail */
/** final detail */
export async function greet(name: string) {
  return name;
}
"#;

    let artefacts = extract_js_ts_artefacts(content, "src/sample.ts").unwrap();
    let function = artefacts
        .iter()
        .find(|artefact| {
            artefact.language_kind == "function_declaration" && artefact.name == "greet"
        })
        .expect("expected function artefact");

    assert_eq!(
        function.modifiers,
        vec!["export".to_string(), "async".to_string()]
    );
    assert_eq!(
        function.docstring.as_deref(),
        Some("first line\nsecond line\n\nblock detail\n\nfinal detail")
    );
}

#[test]
fn extract_js_ts_dependency_edges_resolves_imports_and_calls() {
    let content = r#"import { helper as extHelper } from "./utils";
function local() {
  return 1;
}
function caller() {
  local();
  extHelper();
}
"#;
    let artefacts = extract_js_ts_artefacts(content, "src/sample.ts").unwrap();
    let edges = extract_js_ts_dependency_edges(content, "src/sample.ts", &artefacts).unwrap();

    assert!(edges.iter().any(|e| {
        e.edge_kind == "imports"
            && e.from_symbol_fqn == "src/sample.ts"
            && e.to_symbol_ref.as_deref() == Some("./utils")
    }));

    assert!(edges.iter().any(|e| {
        e.edge_kind == "calls"
            && e.from_symbol_fqn == "src/sample.ts::caller"
            && e.to_target_symbol_fqn.as_deref() == Some("src/sample.ts::local")
    }));

    assert!(edges.iter().any(|e| {
        e.edge_kind == "calls"
            && e.from_symbol_fqn == "src/sample.ts::caller"
            && e.to_symbol_ref.as_deref() == Some("./utils::helper")
    }));
}

#[test]
fn extract_js_ts_dependency_edges_emits_unresolved_call_fallback() {
    let content = r#"function caller() {
  mystery();
}
"#;
    let artefacts = extract_js_ts_artefacts(content, "src/sample.ts").unwrap();
    let edges = extract_js_ts_dependency_edges(content, "src/sample.ts", &artefacts).unwrap();

    assert!(edges.iter().any(|e| {
        e.edge_kind == "calls"
            && e.from_symbol_fqn == "src/sample.ts::caller"
            && e.to_symbol_ref.as_deref() == Some("src/sample.ts::mystery")
    }));
}

#[test]
fn extract_js_ts_dependency_edges_are_ordered_and_resolve_local_before_imports() {
    let content = r#"import defaultHelper, { helper as extHelper } from "./utils";
import "./setup";
function extHelper() {
  return 1;
}
function caller() {
  extHelper();
  defaultHelper();
  mystery();
}
"#;
    let artefacts = extract_js_ts_artefacts(content, "src/sample.ts").unwrap();
    let snapshot = |edges: &[DependencyEdge]| {
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

    let edges = extract_js_ts_dependency_edges(content, "src/sample.ts", &artefacts).unwrap();
    let edge_snapshot = snapshot(&edges);
    let repeated_snapshot =
        snapshot(&extract_js_ts_dependency_edges(content, "src/sample.ts", &artefacts).unwrap());

    assert_eq!(
        edge_snapshot,
        vec![
            "imports|src/sample.ts|-|./utils|1|binding|-".to_string(),
            "imports|src/sample.ts|-|./setup|2|side_effect|-".to_string(),
            "calls|src/sample.ts::caller|src/sample.ts::extHelper|-|7|-|local".to_string(),
            "calls|src/sample.ts::caller|-|./utils::default|8|-|import".to_string(),
            "calls|src/sample.ts::caller|-|src/sample.ts::mystery|9|-|unresolved".to_string(),
        ]
    );
    assert_eq!(edge_snapshot, repeated_snapshot);
}

#[test]
fn extract_js_ts_dependency_edges_emit_type_and_value_references_with_ref_kind() {
    let content = r#"interface User {
  id: string;
}
const DEFAULT_USER: User = { id: "1" };
function project(user: User): User {
  const current: User = DEFAULT_USER;
  return current;
}
"#;
    let artefacts = extract_js_ts_artefacts(content, "src/sample.ts").unwrap();
    let edges = extract_js_ts_dependency_edges(content, "src/sample.ts", &artefacts).unwrap();

    let type_reference = edges
        .iter()
        .find(|edge| {
            edge.edge_kind == "references"
                && edge.from_symbol_fqn == "src/sample.ts::project"
                && edge.to_target_symbol_fqn.as_deref() == Some("src/sample.ts::User")
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
                && edge.from_symbol_fqn == "src/sample.ts::project"
                && edge.to_target_symbol_fqn.as_deref() == Some("src/sample.ts::DEFAULT_USER")
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
fn extract_js_ts_dependency_edges_emit_extends_for_extends_clauses() {
    let content = r#"class BaseService {}
class UserService extends BaseService {}

interface UserShape {
  id: string;
}

interface AdminShape extends UserShape {
  role: string;
}
"#;
    let artefacts = extract_js_ts_artefacts(content, "src/sample.ts").unwrap();
    let edges = extract_js_ts_dependency_edges(content, "src/sample.ts", &artefacts).unwrap();

    assert!(edges.iter().any(|edge| {
        edge.edge_kind == "extends"
            && edge.from_symbol_fqn == "src/sample.ts::UserService"
            && edge.to_target_symbol_fqn.as_deref() == Some("src/sample.ts::BaseService")
    }));
    assert!(edges.iter().any(|edge| {
        edge.edge_kind == "extends"
            && edge.from_symbol_fqn == "src/sample.ts::AdminShape"
            && edge.to_target_symbol_fqn.as_deref() == Some("src/sample.ts::UserShape")
    }));
}

#[test]
fn extract_js_ts_dependency_edges_emit_exports_with_alias_distinct_dedup() {
    let content = r#"function helper() {
  return 1;
}

export { helper };
export { helper };
export { helper as helperAlias };
export { remoteFoo } from "./remote";
export { remoteFoo } from "./remote";
export { remoteFoo as remoteAlias } from "./remote";
"#;
    let artefacts = extract_js_ts_artefacts(content, "src/sample.ts").unwrap();
    let edges = extract_js_ts_dependency_edges(content, "src/sample.ts", &artefacts).unwrap();
    let export_edges = edges
        .iter()
        .filter(|edge| edge.edge_kind == "exports")
        .collect::<Vec<_>>();

    assert_eq!(
        export_edges.len(),
        4,
        "expected duplicate export/re-export edges to collapse while alias-distinct exports stay separate"
    );

    assert_eq!(
        export_edges
            .iter()
            .filter(|edge| {
                edge.from_symbol_fqn == "src/sample.ts"
                    && edge.to_target_symbol_fqn.as_deref() == Some("src/sample.ts::helper")
                    && edge
                        .metadata
                        .get("export_name")
                        .and_then(|value| value.as_str())
                        == Some("helper")
            })
            .count(),
        1,
        "duplicate local exports for the same alias should dedupe"
    );

    let local_alias = export_edges
        .iter()
        .find(|edge| {
            edge.from_symbol_fqn == "src/sample.ts"
                && edge.to_target_symbol_fqn.as_deref() == Some("src/sample.ts::helper")
                && edge
                    .metadata
                    .get("export_name")
                    .and_then(|value| value.as_str())
                    == Some("helperAlias")
        })
        .expect("expected aliased local export edge for helperAlias");
    assert_eq!(
        local_alias
            .metadata
            .get("export_form")
            .and_then(|value| value.as_str()),
        Some("named")
    );

    assert_eq!(
        export_edges
            .iter()
            .filter(|edge| {
                edge.from_symbol_fqn == "src/sample.ts"
                    && edge.to_symbol_ref.as_deref() == Some("./remote::remoteFoo")
                    && edge
                        .metadata
                        .get("export_name")
                        .and_then(|value| value.as_str())
                        == Some("remoteFoo")
            })
            .count(),
        1,
        "duplicate re-exports for the same alias should dedupe"
    );

    let re_export_alias = export_edges
        .iter()
        .find(|edge| {
            edge.from_symbol_fqn == "src/sample.ts"
                && edge.to_symbol_ref.as_deref() == Some("./remote::remoteFoo")
                && edge
                    .metadata
                    .get("export_name")
                    .and_then(|value| value.as_str())
                    == Some("remoteAlias")
        })
        .expect("expected aliased re-export edge for remoteAlias");
    assert_eq!(
        re_export_alias
            .metadata
            .get("export_form")
            .and_then(|value| value.as_str()),
        Some("re_export")
    );
}

// Case 1: extends clause must NOT also produce a references edge
// When a class or interface only mentions a type in its extends/implements clause, that type
// relationship is fully captured by the extends edge. A duplicate references edge for the same
// target is noise. Currently FAILS because the reference extractor also picks up the inherited
// type name and emits a spurious references edge.
#[test]
fn extract_js_ts_dependency_edges_extends_clause_does_not_emit_redundant_references_edge() {
    let content = r#"class Animal {}
class Dog extends Animal {}

interface Base {
  id: string;
}
interface Serializable extends Base {}
"#;
    let artefacts = extract_js_ts_artefacts(content, "src/sample.ts").unwrap();
    let edges = extract_js_ts_dependency_edges(content, "src/sample.ts", &artefacts).unwrap();

    // extends edges must be present — baseline
    assert!(
        edges.iter().any(|e| {
            e.edge_kind == "extends"
                && e.from_symbol_fqn == "src/sample.ts::Dog"
                && e.to_target_symbol_fqn.as_deref() == Some("src/sample.ts::Animal")
        }),
        "expected extends edge Dog → Animal"
    );
    assert!(
        edges.iter().any(|e| {
            e.edge_kind == "extends"
                && e.from_symbol_fqn == "src/sample.ts::Serializable"
                && e.to_target_symbol_fqn.as_deref() == Some("src/sample.ts::Base")
        }),
        "expected extends edge Serializable → Base"
    );

    // no spurious references edges for the same target — the extends edge is sufficient
    assert!(
        !edges.iter().any(|e| {
            e.edge_kind == "references"
                && e.from_symbol_fqn == "src/sample.ts::Dog"
                && e.to_target_symbol_fqn.as_deref() == Some("src/sample.ts::Animal")
        }),
        "extends clause should not also emit a references edge for Animal (duplicate)"
    );
    assert!(
        !edges.iter().any(|e| {
            e.edge_kind == "references"
                && e.from_symbol_fqn == "src/sample.ts::Serializable"
                && e.to_target_symbol_fqn.as_deref() == Some("src/sample.ts::Base")
        }),
        "extends clause should not also emit a references edge for Base (duplicate)"
    );
}

// Case 2: member call + identifier call duplication
// obj.method() fires both regexes: call_ident_re matches `method(` and call_member_re matches
// `.method(`, producing two edges for the same invocation. Only the member edge should be kept.
#[test]
fn extract_js_ts_dependency_edges_member_call_does_not_also_emit_identifier_call() {
    let content = r#"function run() {
  console.timeEnd("label");
  obj.process();
}
"#;
    let artefacts = extract_js_ts_artefacts(content, "src/sample.ts").unwrap();
    let edges = extract_js_ts_dependency_edges(content, "src/sample.ts", &artefacts).unwrap();

    let calls_from_run: Vec<_> = edges
        .iter()
        .filter(|e| e.edge_kind == "calls" && e.from_symbol_fqn == "src/sample.ts::run")
        .collect();

    // timeEnd: should produce exactly one edge (member form), not two (member + identifier)
    let timeend_edges: Vec<_> = calls_from_run
        .iter()
        .filter(|e| {
            e.to_symbol_ref
                .as_deref()
                .map(|r| r.contains("timeEnd"))
                .unwrap_or(false)
        })
        .collect();
    assert_eq!(
        timeend_edges.len(),
        1,
        "timeEnd() call should produce exactly one edge (member), got {:?}",
        timeend_edges
            .iter()
            .map(|e| (e.to_symbol_ref.as_deref(), e.metadata.get("call_form")))
            .collect::<Vec<_>>()
    );
    assert_eq!(
        timeend_edges[0]
            .metadata
            .get("call_form")
            .and_then(|v| v.as_str()),
        Some("member"),
        "the single timeEnd edge should use call_form=member"
    );

    // process: same assertion for a second member call on a different line
    let process_edges: Vec<_> = calls_from_run
        .iter()
        .filter(|e| {
            e.to_symbol_ref
                .as_deref()
                .map(|r| r.contains("process"))
                .unwrap_or(false)
        })
        .collect();
    assert_eq!(
        process_edges.len(),
        1,
        "process() call should produce exactly one edge (member), got {:?}",
        process_edges
            .iter()
            .map(|e| (e.to_symbol_ref.as_deref(), e.metadata.get("call_form")))
            .collect::<Vec<_>>()
    );
    assert_eq!(
        process_edges[0]
            .metadata
            .get("call_form")
            .and_then(|v| v.as_str()),
        Some("member"),
        "the single process edge should use call_form=member"
    );
}
