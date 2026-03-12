use super::*;

const LANGUAGE_ONLY_CANONICAL_KIND: &str = "language_only";

fn artefact_by_language_kind<'a>(
    artefacts: &'a [JsTsArtefact],
    language_kind: &str,
) -> &'a JsTsArtefact {
    artefacts
        .iter()
        .find(|artefact| artefact.language_kind == language_kind)
        .unwrap_or_else(|| panic!("missing artefact with language_kind {language_kind}"))
}

fn artefact_by_name_and_language_kind<'a>(
    artefacts: &'a [JsTsArtefact],
    language_kind: &str,
    name: &str,
) -> &'a JsTsArtefact {
    artefacts
        .iter()
        .find(|artefact| artefact.language_kind == language_kind && artefact.name == name)
        .unwrap_or_else(|| panic!("missing artefact {name} with language_kind {language_kind}"))
}

#[test]
fn js_ts_canonical_mapping_covers_supported_kind_table() {
    let expected = [
        ("function_declaration", Some("function")),
        ("method_definition", Some("method")),
        ("interface_declaration", Some("interface")),
        ("type_alias_declaration", Some("type")),
        ("enum_declaration", Some("enum")),
        ("variable_declarator", Some("variable")),
        ("import_statement", Some("import")),
        ("module_declaration", Some("module")),
        ("internal_module", Some("module")),
        ("class_declaration", Some(LANGUAGE_ONLY_CANONICAL_KIND)),
        ("constructor", Some(LANGUAGE_ONLY_CANONICAL_KIND)),
        ("property_declaration", Some(LANGUAGE_ONLY_CANONICAL_KIND)),
        ("call_expression", None),
    ];

    for (language_kind, canonical_kind) in expected {
        assert_eq!(js_ts_canonical_kind(language_kind), canonical_kind);
    }
}

#[test]
fn js_ts_canonical_mapping_is_abstraction_only_and_preserves_parser_kinds() {
    let content = r#"import { helper } from "./helper";
export interface Contract {
  id: string;
}
export type Identifier = string;
export enum Status {
  Ready,
}
const API_URL = "/v1";
export class Service {
  constructor(private readonly prefix: string) {}

  run() {
    return helper();
  }
}
export function helper() {
  return "ok";
}
"#;

    let artefacts = extract_js_ts_artefacts(content, "src/sample.ts").unwrap();

    let import = artefact_by_language_kind(&artefacts, "import_statement");
    assert_eq!(import.canonical_kind, "import");

    let interface =
        artefact_by_name_and_language_kind(&artefacts, "interface_declaration", "Contract");
    assert_eq!(interface.canonical_kind, "interface");

    let type_alias =
        artefact_by_name_and_language_kind(&artefacts, "type_alias_declaration", "Identifier");
    assert_eq!(type_alias.canonical_kind, "type");

    let variable = artefact_by_name_and_language_kind(&artefacts, "variable_declarator", "API_URL");
    assert_eq!(variable.canonical_kind, "variable");

    let function = artefact_by_name_and_language_kind(&artefacts, "function_declaration", "helper");
    assert_eq!(function.canonical_kind, "function");

    let constructor = artefact_by_name_and_language_kind(&artefacts, "constructor", "constructor");
    assert_eq!(constructor.canonical_kind, LANGUAGE_ONLY_CANONICAL_KIND);

    let method = artefact_by_name_and_language_kind(&artefacts, "method_definition", "run");
    assert_eq!(method.canonical_kind, "method");

    let class = artefact_by_name_and_language_kind(&artefacts, "class_declaration", "Service");
    assert_eq!(class.canonical_kind, LANGUAGE_ONLY_CANONICAL_KIND);
}

#[test]
fn rust_canonical_mapping_covers_supported_kind_table() {
    let expected = [
        (("function_item", false), Some("function")),
        (("function_item", true), Some("method")),
        (("trait_item", false), Some("interface")),
        (("type_item", false), Some("type")),
        (("enum_item", false), Some("enum")),
        (("use_declaration", false), Some("import")),
        (("mod_item", false), Some("module")),
        (("let_declaration", false), Some("variable")),
        (("impl_item", false), Some(LANGUAGE_ONLY_CANONICAL_KIND)),
        (("struct_item", false), Some(LANGUAGE_ONLY_CANONICAL_KIND)),
        (("const_item", false), Some(LANGUAGE_ONLY_CANONICAL_KIND)),
        (("static_item", false), Some(LANGUAGE_ONLY_CANONICAL_KIND)),
        (
            ("macro_definition", false),
            Some(LANGUAGE_ONLY_CANONICAL_KIND),
        ),
        (("call_expression", false), None),
    ];

    for ((language_kind, inside_impl), canonical_kind) in expected {
        assert_eq!(
            rust_canonical_kind(language_kind, inside_impl),
            canonical_kind
        );
    }
}

#[test]
fn rust_canonical_mapping_normalizes_traits_and_marks_language_only_symbols() {
    let content = r#"use crate::fmt::Display;

mod api {}

type UserId = u64;
enum Role {
    Admin,
}
struct User;

trait Repository {
    fn save(&self);
}

impl Repository for User {
    fn save(&self) {}
}

const LIMIT: usize = 4;
static NAME: &str = "demo";

fn run() {}
"#;

    let artefacts = extract_rust_artefacts(content, "src/lib.rs").unwrap();

    let import = artefact_by_language_kind(&artefacts, "use_declaration");
    assert_eq!(import.canonical_kind, "import");

    let module = artefact_by_name_and_language_kind(&artefacts, "mod_item", "api");
    assert_eq!(module.canonical_kind, "module");

    let type_item = artefact_by_name_and_language_kind(&artefacts, "type_item", "UserId");
    assert_eq!(type_item.canonical_kind, "type");

    let enum_item = artefact_by_name_and_language_kind(&artefacts, "enum_item", "Role");
    assert_eq!(enum_item.canonical_kind, "enum");

    let trait_item = artefact_by_name_and_language_kind(&artefacts, "trait_item", "Repository");
    assert_eq!(trait_item.canonical_kind, "interface");

    let free_function = artefacts
        .iter()
        .find(|artefact| {
            artefact.language_kind == "function_item"
                && artefact.name == "run"
                && artefact.parent_symbol_fqn.is_none()
        })
        .expect("missing free function artefact");
    assert_eq!(free_function.canonical_kind, "function");

    let struct_item = artefact_by_name_and_language_kind(&artefacts, "struct_item", "User");
    assert_eq!(struct_item.canonical_kind, LANGUAGE_ONLY_CANONICAL_KIND);

    let impl_item = artefact_by_language_kind(&artefacts, "impl_item");
    assert_eq!(impl_item.canonical_kind, LANGUAGE_ONLY_CANONICAL_KIND);

    let const_item = artefact_by_name_and_language_kind(&artefacts, "const_item", "LIMIT");
    assert_eq!(const_item.canonical_kind, LANGUAGE_ONLY_CANONICAL_KIND);

    let static_item = artefact_by_name_and_language_kind(&artefacts, "static_item", "NAME");
    assert_eq!(static_item.canonical_kind, LANGUAGE_ONLY_CANONICAL_KIND);
}

#[test]
fn rust_trait_method_signatures_are_not_emitted_as_free_functions() {
    let content = r#"trait Repository {
    fn save(&self);
}

struct User;

impl Repository for User {
    fn save(&self) {}
}
"#;

    let artefacts = extract_rust_artefacts(content, "src/lib.rs").unwrap();

    let trait_item = artefact_by_name_and_language_kind(&artefacts, "trait_item", "Repository");
    assert_eq!(trait_item.canonical_kind, "interface");

    let save_callables = artefacts
        .iter()
        .filter(|artefact| {
            artefact.name == "save"
                && (artefact.canonical_kind == "function" || artefact.canonical_kind == "method")
        })
        .collect::<Vec<_>>();

    assert_eq!(
        save_callables.len(),
        1,
        "trait signatures should not be emitted as standalone callable artefacts"
    );
    assert_eq!(save_callables[0].canonical_kind, "method");
    assert!(
        save_callables[0]
            .parent_symbol_fqn
            .as_deref()
            .is_some_and(|parent| parent.starts_with("src/lib.rs::impl@"))
    );
}
