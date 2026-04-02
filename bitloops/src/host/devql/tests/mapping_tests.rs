use super::*;
use crate::adapters::languages::go::canonical::{
    GO_CANONICAL_MAPPINGS, GO_SUPPORTED_LANGUAGE_KINDS,
};
use crate::adapters::languages::go::extraction::extract_go_artefacts;
use crate::adapters::languages::python::canonical::{
    PYTHON_CANONICAL_MAPPINGS, PYTHON_SUPPORTED_LANGUAGE_KINDS,
};
use crate::adapters::languages::python::extraction::extract_python_artefacts;
use crate::adapters::languages::rust::canonical::{
    RUST_CANONICAL_MAPPINGS, RUST_SUPPORTED_LANGUAGE_KINDS,
};
use crate::adapters::languages::rust::extraction::extract_rust_artefacts;
use crate::adapters::languages::ts_js::canonical::{
    TS_JS_CANONICAL_MAPPINGS, TS_JS_SUPPORTED_LANGUAGE_KINDS,
};
use crate::adapters::languages::ts_js::extraction::extract_js_ts_artefacts;
use crate::host::language_adapter::{GoKind, LanguageKind, PythonKind, RustKind, TsJsKind};
use crate::host::language_adapter::{is_supported_language_kind, resolve_canonical_kind};

fn extension_runtime_cfg() -> DevqlConfig {
    DevqlConfig {
        config_root: PathBuf::from("/tmp/repo"),
        repo_root: PathBuf::from("/tmp/repo"),
        repo: RepoIdentity {
            provider: "github".to_string(),
            organization: "bitloops".to_string(),
            name: "temp2".to_string(),
            identity: "github/bitloops/temp2".to_string(),
            repo_id: deterministic_uuid("repo://github/bitloops/temp2"),
        },
        pg_dsn: None,
        clickhouse_url: "http://localhost:8123".to_string(),
        clickhouse_user: None,
        clickhouse_password: None,
        clickhouse_database: "default".to_string(),
        semantic_provider: None,
        semantic_model: None,
        semantic_api_key: None,
        semantic_base_url: None,
    }
}

fn canonical_kind(artefact: &LanguageArtefact) -> Option<&str> {
    artefact.canonical_kind.as_deref()
}

fn artefact_by_language_kind(
    artefacts: &[LanguageArtefact],
    language_kind: LanguageKind,
) -> &LanguageArtefact {
    artefacts
        .iter()
        .find(|artefact| artefact.language_kind == language_kind)
        .unwrap_or_else(|| panic!("missing artefact with language_kind {}", language_kind))
}

fn artefact_by_name_and_language_kind<'a>(
    artefacts: &'a [LanguageArtefact],
    language_kind: LanguageKind,
    name: &str,
) -> &'a LanguageArtefact {
    artefacts
        .iter()
        .find(|artefact| artefact.language_kind == language_kind && artefact.name == name)
        .unwrap_or_else(|| {
            panic!(
                "missing artefact {name} with language_kind {}",
                language_kind
            )
        })
}

#[test]
fn js_ts_canonical_mapping_covers_supported_kind_table() {
    let expected = [
        (
            LanguageKind::ts_js(TsJsKind::FunctionDeclaration),
            true,
            Some("function"),
        ),
        (
            LanguageKind::ts_js(TsJsKind::MethodDefinition),
            true,
            Some("method"),
        ),
        (
            LanguageKind::ts_js(TsJsKind::InterfaceDeclaration),
            true,
            Some("interface"),
        ),
        (
            LanguageKind::ts_js(TsJsKind::TypeAliasDeclaration),
            true,
            Some("type"),
        ),
        (
            LanguageKind::ts_js(TsJsKind::EnumDeclaration),
            true,
            Some("enum"),
        ),
        (
            LanguageKind::ts_js(TsJsKind::VariableDeclarator),
            true,
            Some("variable"),
        ),
        (
            LanguageKind::ts_js(TsJsKind::ImportStatement),
            true,
            Some("import"),
        ),
        (
            LanguageKind::ts_js(TsJsKind::ModuleDeclaration),
            true,
            Some("module"),
        ),
        (
            LanguageKind::ts_js(TsJsKind::InternalModule),
            true,
            Some("module"),
        ),
        (LanguageKind::ts_js(TsJsKind::ClassDeclaration), true, None),
        (LanguageKind::ts_js(TsJsKind::Constructor), true, None),
        (
            LanguageKind::ts_js(TsJsKind::PropertyDeclaration),
            true,
            None,
        ),
        (
            LanguageKind::ts_js(TsJsKind::PublicFieldDefinition),
            true,
            None,
        ),
    ];

    for (language_kind, supported, canonical_kind) in expected {
        assert_eq!(
            is_supported_language_kind(TS_JS_SUPPORTED_LANGUAGE_KINDS, language_kind),
            supported
        );
        assert_eq!(
            resolve_canonical_kind(TS_JS_CANONICAL_MAPPINGS, language_kind, false)
                .map(CanonicalKindProjection::as_str),
            canonical_kind
        );
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

    let import =
        artefact_by_language_kind(&artefacts, LanguageKind::ts_js(TsJsKind::ImportStatement));
    assert_eq!(canonical_kind(import), Some("import"));

    let interface = artefact_by_name_and_language_kind(
        &artefacts,
        LanguageKind::ts_js(TsJsKind::InterfaceDeclaration),
        "Contract",
    );
    assert_eq!(canonical_kind(interface), Some("interface"));

    let type_alias = artefact_by_name_and_language_kind(
        &artefacts,
        LanguageKind::ts_js(TsJsKind::TypeAliasDeclaration),
        "Identifier",
    );
    assert_eq!(canonical_kind(type_alias), Some("type"));

    let variable = artefact_by_name_and_language_kind(
        &artefacts,
        LanguageKind::ts_js(TsJsKind::VariableDeclarator),
        "API_URL",
    );
    assert_eq!(canonical_kind(variable), Some("variable"));

    let function = artefact_by_name_and_language_kind(
        &artefacts,
        LanguageKind::ts_js(TsJsKind::FunctionDeclaration),
        "helper",
    );
    assert_eq!(canonical_kind(function), Some("function"));

    let constructor = artefact_by_name_and_language_kind(
        &artefacts,
        LanguageKind::ts_js(TsJsKind::Constructor),
        "constructor",
    );
    assert_eq!(canonical_kind(constructor), None);

    let method = artefact_by_name_and_language_kind(
        &artefacts,
        LanguageKind::ts_js(TsJsKind::MethodDefinition),
        "run",
    );
    assert_eq!(canonical_kind(method), Some("method"));

    let class = artefact_by_name_and_language_kind(
        &artefacts,
        LanguageKind::ts_js(TsJsKind::ClassDeclaration),
        "Service",
    );
    assert_eq!(canonical_kind(class), None);
}

#[test]
fn rust_canonical_mapping_covers_supported_kind_table() {
    let expected = [
        (
            Some(LanguageKind::rust(RustKind::FunctionItem)),
            false,
            true,
            Some("function"),
        ),
        (
            Some(LanguageKind::rust(RustKind::FunctionItem)),
            true,
            true,
            Some("method"),
        ),
        (
            Some(LanguageKind::rust(RustKind::TraitItem)),
            false,
            true,
            Some("interface"),
        ),
        (
            Some(LanguageKind::rust(RustKind::TypeItem)),
            false,
            true,
            Some("type"),
        ),
        (
            Some(LanguageKind::rust(RustKind::EnumItem)),
            false,
            true,
            Some("enum"),
        ),
        (
            Some(LanguageKind::rust(RustKind::UseDeclaration)),
            false,
            true,
            Some("import"),
        ),
        (
            Some(LanguageKind::rust(RustKind::ModItem)),
            false,
            true,
            Some("module"),
        ),
        (
            Some(LanguageKind::rust(RustKind::LetDeclaration)),
            false,
            true,
            Some("variable"),
        ),
        (
            Some(LanguageKind::rust(RustKind::ImplItem)),
            false,
            true,
            None,
        ),
        (
            Some(LanguageKind::rust(RustKind::StructItem)),
            false,
            true,
            None,
        ),
        (
            Some(LanguageKind::rust(RustKind::ConstItem)),
            false,
            true,
            None,
        ),
        (
            Some(LanguageKind::rust(RustKind::StaticItem)),
            false,
            true,
            None,
        ),
        (
            Some(LanguageKind::rust(RustKind::MacroDefinition)),
            false,
            true,
            None,
        ),
        (None, false, false, None),
    ];

    for (language_kind, inside_impl, supported, canonical_kind) in expected {
        let actual_supported = language_kind
            .map(|kind| is_supported_language_kind(RUST_SUPPORTED_LANGUAGE_KINDS, kind))
            .unwrap_or(false);
        let actual_canonical = language_kind
            .and_then(|kind| resolve_canonical_kind(RUST_CANONICAL_MAPPINGS, kind, inside_impl))
            .map(CanonicalKindProjection::as_str);
        assert_eq!(actual_supported, supported);
        assert_eq!(actual_canonical, canonical_kind);
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

    let import =
        artefact_by_language_kind(&artefacts, LanguageKind::rust(RustKind::UseDeclaration));
    assert_eq!(canonical_kind(import), Some("import"));

    let module = artefact_by_name_and_language_kind(
        &artefacts,
        LanguageKind::rust(RustKind::ModItem),
        "api",
    );
    assert_eq!(canonical_kind(module), Some("module"));

    let type_item = artefact_by_name_and_language_kind(
        &artefacts,
        LanguageKind::rust(RustKind::TypeItem),
        "UserId",
    );
    assert_eq!(canonical_kind(type_item), Some("type"));

    let enum_item = artefact_by_name_and_language_kind(
        &artefacts,
        LanguageKind::rust(RustKind::EnumItem),
        "Role",
    );
    assert_eq!(canonical_kind(enum_item), Some("enum"));

    let trait_item = artefact_by_name_and_language_kind(
        &artefacts,
        LanguageKind::rust(RustKind::TraitItem),
        "Repository",
    );
    assert_eq!(canonical_kind(trait_item), Some("interface"));

    let free_function = artefacts
        .iter()
        .find(|artefact| {
            artefact.language_kind == LanguageKind::rust(RustKind::FunctionItem)
                && artefact.name == "run"
                && artefact.parent_symbol_fqn.is_none()
        })
        .expect("missing free function artefact");
    assert_eq!(canonical_kind(free_function), Some("function"));

    let struct_item = artefact_by_name_and_language_kind(
        &artefacts,
        LanguageKind::rust(RustKind::StructItem),
        "User",
    );
    assert_eq!(canonical_kind(struct_item), None);

    let impl_item = artefact_by_language_kind(&artefacts, LanguageKind::rust(RustKind::ImplItem));
    assert_eq!(canonical_kind(impl_item), None);

    let const_item = artefact_by_name_and_language_kind(
        &artefacts,
        LanguageKind::rust(RustKind::ConstItem),
        "LIMIT",
    );
    assert_eq!(canonical_kind(const_item), None);

    let static_item = artefact_by_name_and_language_kind(
        &artefacts,
        LanguageKind::rust(RustKind::StaticItem),
        "NAME",
    );
    assert_eq!(canonical_kind(static_item), None);
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

    let trait_item = artefact_by_name_and_language_kind(
        &artefacts,
        LanguageKind::rust(RustKind::TraitItem),
        "Repository",
    );
    assert_eq!(canonical_kind(trait_item), Some("interface"));

    let save_callables = artefacts
        .iter()
        .filter(|artefact| {
            artefact.name == "save"
                && matches!(
                    artefact.canonical_kind.as_deref(),
                    Some("function") | Some("method")
                )
        })
        .collect::<Vec<_>>();

    assert_eq!(
        save_callables.len(),
        1,
        "trait signatures should not be emitted as standalone callable artefacts"
    );
    assert_eq!(canonical_kind(save_callables[0]), Some("method"));
    assert!(
        save_callables[0]
            .parent_symbol_fqn
            .as_deref()
            .is_some_and(|parent| parent.starts_with("src/lib.rs::impl@"))
    );
}

#[test]
fn python_canonical_mapping_covers_supported_kind_table() {
    let expected = [
        (
            Some(LanguageKind::python(PythonKind::FunctionDefinition)),
            false,
            true,
            Some("function"),
        ),
        (
            Some(LanguageKind::python(PythonKind::FunctionDefinition)),
            true,
            true,
            Some("method"),
        ),
        (
            Some(LanguageKind::python(PythonKind::ClassDefinition)),
            false,
            true,
            Some("type"),
        ),
        (
            Some(LanguageKind::python(PythonKind::ImportStatement)),
            false,
            true,
            Some("import"),
        ),
        (
            Some(LanguageKind::python(PythonKind::ImportFromStatement)),
            false,
            true,
            Some("import"),
        ),
        (
            Some(LanguageKind::python(PythonKind::FutureImportStatement)),
            false,
            true,
            Some("import"),
        ),
        (
            Some(LanguageKind::python(PythonKind::Assignment)),
            false,
            true,
            Some("variable"),
        ),
        (None, false, false, None),
        (None, false, false, None),
    ];

    for (language_kind, inside_parent, supported, canonical_kind) in expected {
        let actual_supported = language_kind
            .map(|kind| is_supported_language_kind(PYTHON_SUPPORTED_LANGUAGE_KINDS, kind))
            .unwrap_or(false);
        let actual_canonical = language_kind
            .and_then(|kind| resolve_canonical_kind(PYTHON_CANONICAL_MAPPINGS, kind, inside_parent))
            .map(CanonicalKindProjection::as_str);
        assert_eq!(actual_supported, supported);
        assert_eq!(actual_canonical, canonical_kind);
    }
}

#[test]
fn python_canonical_mapping_extracts_functions_methods_types_and_docstrings() {
    let content = r#"
"""module docs"""

from pkg.helpers import helper as imported_helper
import os

VALUE = 1

class Greeter(BaseGreeter):
    """class docs"""

    @staticmethod
    def helper():
        return imported_helper()

    async def greet(self):
        """method docs"""
        return self.helper()

def run():
    """function docs"""
    return imported_helper()
"#;

    let artefacts = extract_python_artefacts(content, "src/main.py").unwrap();

    let import = artefact_by_language_kind(
        &artefacts,
        LanguageKind::python(PythonKind::ImportFromStatement),
    );
    assert_eq!(canonical_kind(import), Some("import"));

    let class = artefact_by_name_and_language_kind(
        &artefacts,
        LanguageKind::python(PythonKind::ClassDefinition),
        "Greeter",
    );
    assert_eq!(canonical_kind(class), Some("type"));
    assert_eq!(class.docstring.as_deref(), Some("class docs"));

    let variable = artefact_by_name_and_language_kind(
        &artefacts,
        LanguageKind::python(PythonKind::Assignment),
        "VALUE",
    );
    assert_eq!(canonical_kind(variable), Some("variable"));

    let function = artefact_by_name_and_language_kind(
        &artefacts,
        LanguageKind::python(PythonKind::FunctionDefinition),
        "run",
    );
    assert_eq!(canonical_kind(function), Some("function"));
    assert_eq!(function.docstring.as_deref(), Some("function docs"));

    let method = artefact_by_name_and_language_kind(
        &artefacts,
        LanguageKind::python(PythonKind::FunctionDefinition),
        "greet",
    );
    assert_eq!(canonical_kind(method), Some("method"));
    assert_eq!(method.docstring.as_deref(), Some("method docs"));

    let static_method = artefact_by_name_and_language_kind(
        &artefacts,
        LanguageKind::python(PythonKind::FunctionDefinition),
        "helper",
    );
    assert_eq!(canonical_kind(static_method), Some("method"));
    assert!(
        static_method
            .modifiers
            .iter()
            .any(|modifier| modifier == "staticmethod")
    );
}

#[test]
fn go_canonical_mapping_covers_supported_kind_table() {
    let expected = [
        (
            Some(LanguageKind::go(GoKind::FunctionDeclaration)),
            true,
            Some("function"),
        ),
        (
            Some(LanguageKind::go(GoKind::MethodDeclaration)),
            true,
            Some("method"),
        ),
        (Some(LanguageKind::go(GoKind::TypeSpec)), true, Some("type")),
        (
            Some(LanguageKind::go(GoKind::TypeAlias)),
            true,
            Some("type"),
        ),
        (
            Some(LanguageKind::go(GoKind::StructType)),
            true,
            Some("type"),
        ),
        (
            Some(LanguageKind::go(GoKind::InterfaceType)),
            true,
            Some("interface"),
        ),
        (
            Some(LanguageKind::go(GoKind::ImportSpec)),
            true,
            Some("import"),
        ),
        (
            Some(LanguageKind::go(GoKind::VarSpec)),
            true,
            Some("variable"),
        ),
        (
            Some(LanguageKind::go(GoKind::ConstSpec)),
            true,
            Some("variable"),
        ),
        (None, false, None),
    ];

    for (language_kind, supported, canonical_kind) in expected {
        let actual_supported = language_kind
            .map(|kind| is_supported_language_kind(GO_SUPPORTED_LANGUAGE_KINDS, kind))
            .unwrap_or(false);
        let actual_canonical = language_kind
            .and_then(|kind| resolve_canonical_kind(GO_CANONICAL_MAPPINGS, kind, false))
            .map(CanonicalKindProjection::as_str);
        assert_eq!(actual_supported, supported);
        assert_eq!(actual_canonical, canonical_kind);
    }
}

#[test]
fn go_canonical_mapping_extracts_functions_methods_types_imports_and_values() {
    let content = r#"package service

import (
    "context"
    alias "net/http"
)

const DefaultPort = 8080

type Handler struct{}
type Runner interface {
    Run(context.Context) error
}

func NewHandler() *Handler {
    return &Handler{}
}

func (h *Handler) ServeHTTP() {}
"#;

    let artefacts = extract_go_artefacts(content, "service/handler.go").unwrap();

    let import = artefact_by_name_and_language_kind(
        &artefacts,
        LanguageKind::go(GoKind::ImportSpec),
        "alias",
    );
    assert_eq!(canonical_kind(import), Some("import"));

    let struct_type = artefact_by_name_and_language_kind(
        &artefacts,
        LanguageKind::go(GoKind::StructType),
        "Handler",
    );
    assert_eq!(canonical_kind(struct_type), Some("type"));

    let interface_type = artefact_by_name_and_language_kind(
        &artefacts,
        LanguageKind::go(GoKind::InterfaceType),
        "Runner",
    );
    assert_eq!(canonical_kind(interface_type), Some("interface"));

    let variable = artefact_by_name_and_language_kind(
        &artefacts,
        LanguageKind::go(GoKind::ConstSpec),
        "DefaultPort",
    );
    assert_eq!(canonical_kind(variable), Some("variable"));

    let function = artefact_by_name_and_language_kind(
        &artefacts,
        LanguageKind::go(GoKind::FunctionDeclaration),
        "NewHandler",
    );
    assert_eq!(canonical_kind(function), Some("function"));

    let method = artefact_by_name_and_language_kind(
        &artefacts,
        LanguageKind::go(GoKind::MethodDeclaration),
        "ServeHTTP",
    );
    assert_eq!(canonical_kind(method), Some("method"));
    assert_eq!(
        method.parent_symbol_fqn.as_deref(),
        Some("service/handler.go::Handler")
    );
}

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
    assert!(resolve_language_id_for_file_path("README").is_none());
}

#[test]
fn devql_language_adapter_registry_resolves_built_in_pack_implementations() {
    let registry = language_adapter_registry().expect("initialize language adapter registry");
    assert_eq!(
        registry.registered_pack_ids(),
        vec![
            GO_LANGUAGE_PACK_ID,
            PYTHON_LANGUAGE_PACK_ID,
            RUST_LANGUAGE_PACK_ID,
            TS_JS_LANGUAGE_PACK_ID
        ]
    );
    assert!(registry.get(GO_LANGUAGE_PACK_ID).is_some());
    assert!(registry.get(RUST_LANGUAGE_PACK_ID).is_some());
    assert!(registry.get(TS_JS_LANGUAGE_PACK_ID).is_some());
    assert!(registry.get(PYTHON_LANGUAGE_PACK_ID).is_some());
    assert!(registry.get("unknown-pack").is_none());
}

#[test]
fn devql_language_adapter_registry_executes_rust_ts_js_and_python_built_ins() {
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
}

#[test]
fn devql_extension_host_builds_capability_contexts_from_registered_owners() {
    let cfg = extension_runtime_cfg();

    let ingest_context = capability_ingest_context_for_ingester(
        &cfg,
        Some("abc123"),
        TEST_HARNESS_CAPABILITY_INGESTER_ID,
    )
    .expect("resolve test-harness ingester owner");
    assert_eq!(
        ingest_context.capability_pack_id,
        "test-harness-capability-pack"
    );
    assert_eq!(
        ingest_context.ingester_id,
        TEST_HARNESS_CAPABILITY_INGESTER_ID
    );
    assert_eq!(ingest_context.commit_sha.as_deref(), Some("abc123"));
}

#[test]
fn devql_language_adapter_lifecycle_summary_reports_builtins_and_readiness() {
    let cfg = extension_runtime_cfg();
    let lifecycle = collect_language_adapter_lifecycle(&cfg, "local-cli", false, false)
        .expect("collect language adapter lifecycle summary");

    let pack_ids = lifecycle
        .summary
        .packs
        .iter()
        .map(|pack| pack.id.as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        pack_ids,
        vec![
            GO_LANGUAGE_PACK_ID,
            PYTHON_LANGUAGE_PACK_ID,
            RUST_LANGUAGE_PACK_ID,
            TS_JS_LANGUAGE_PACK_ID
        ]
    );
    assert!(
        lifecycle
            .readiness_reports
            .iter()
            .all(|report| report.ready),
        "built-in language adapters should report ready without pending migrations"
    );
}

#[test]
fn core_extension_host_registry_report_with_language_adapter_snapshot_includes_adapter_entries() {
    let cfg = extension_runtime_cfg();
    let lifecycle = collect_language_adapter_lifecycle(&cfg, "local-cli", false, false)
        .expect("collect language adapter lifecycle summary");
    let ext_host = crate::host::extension_host::CoreExtensionHost::with_builtins()
        .expect("bootstrap core extension host");
    let snapshot = ext_host
        .readiness_snapshot()
        .with_language_adapter_readiness(
            lifecycle
                .summary
                .packs
                .iter()
                .map(|pack| pack.id.clone())
                .collect(),
            lifecycle.readiness_reports,
        );
    let report = ext_host.registry_report_with_snapshot(snapshot);

    assert_eq!(
        report.language_adapter_pack_ids,
        vec![
            GO_LANGUAGE_PACK_ID.to_string(),
            PYTHON_LANGUAGE_PACK_ID.to_string(),
            RUST_LANGUAGE_PACK_ID.to_string(),
            TS_JS_LANGUAGE_PACK_ID.to_string()
        ]
    );
    assert!(
        report
            .readiness
            .iter()
            .any(|entry| entry.family == "language-adapter-pack"),
        "language adapter readiness entries should be present in extension report"
    );
}
