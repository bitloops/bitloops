use super::*;

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
fn java_canonical_mapping_covers_supported_kind_table() {
    let expected = [
        (LanguageKind::java(JavaKind::Package), true, Some("module")),
        (LanguageKind::java(JavaKind::Import), true, Some("import")),
        (LanguageKind::java(JavaKind::Class), true, Some("type")),
        (
            LanguageKind::java(JavaKind::Interface),
            true,
            Some("interface"),
        ),
        (LanguageKind::java(JavaKind::Enum), true, Some("enum")),
        (
            LanguageKind::java(JavaKind::Constructor),
            true,
            Some("method"),
        ),
        (LanguageKind::java(JavaKind::Method), true, Some("method")),
        (LanguageKind::java(JavaKind::Field), true, Some("variable")),
    ];

    for (language_kind, supported, canonical_kind) in expected {
        assert_eq!(
            is_supported_language_kind(JAVA_SUPPORTED_LANGUAGE_KINDS, language_kind),
            supported
        );
        assert_eq!(
            resolve_canonical_kind(JAVA_CANONICAL_MAPPINGS, language_kind, false)
                .map(CanonicalKindProjection::as_str),
            canonical_kind
        );
    }
}

#[test]
fn java_canonical_mapping_preserves_java_specific_structure() {
    let content = r#"package com.acme;

import java.util.List;

class Base {}
interface Runner {}

class Greeter extends Base implements Runner {
    private int count;

    Greeter() {}

    void greet(List<String> names) {}
}
"#;

    let artefacts = extract_java_artefacts(content, "src/com/acme/Greeter.java").unwrap();

    let package = artefact_by_name_and_language_kind(
        &artefacts,
        LanguageKind::java(JavaKind::Package),
        "com.acme",
    );
    assert_eq!(canonical_kind(package), Some("module"));

    let import = artefact_by_language_kind(&artefacts, LanguageKind::java(JavaKind::Import));
    assert_eq!(canonical_kind(import), Some("import"));

    let class = artefact_by_name_and_language_kind(
        &artefacts,
        LanguageKind::java(JavaKind::Class),
        "Greeter",
    );
    assert_eq!(canonical_kind(class), Some("type"));

    let constructor = artefact_by_name_and_language_kind(
        &artefacts,
        LanguageKind::java(JavaKind::Constructor),
        "<init>",
    );
    assert_eq!(canonical_kind(constructor), Some("method"));

    let method = artefact_by_name_and_language_kind(
        &artefacts,
        LanguageKind::java(JavaKind::Method),
        "greet",
    );
    assert_eq!(canonical_kind(method), Some("method"));

    let field = artefact_by_name_and_language_kind(
        &artefacts,
        LanguageKind::java(JavaKind::Field),
        "count",
    );
    assert_eq!(canonical_kind(field), Some("variable"));
    assert_eq!(
        field.parent_symbol_fqn.as_deref(),
        Some("src/com/acme/Greeter.java::Greeter")
    );
}

#[test]
fn csharp_canonical_mapping_covers_supported_kind_table() {
    let expected = [
        (
            LanguageKind::csharp(CSharpKind::Class),
            false,
            true,
            Some("type"),
        ),
        (
            LanguageKind::csharp(CSharpKind::Struct),
            false,
            true,
            Some("type"),
        ),
        (
            LanguageKind::csharp(CSharpKind::Record),
            false,
            true,
            Some("type"),
        ),
        (
            LanguageKind::csharp(CSharpKind::Interface),
            false,
            true,
            Some("interface"),
        ),
        (
            LanguageKind::csharp(CSharpKind::Enum),
            false,
            true,
            Some("enum"),
        ),
        (
            LanguageKind::csharp(CSharpKind::Constructor),
            false,
            true,
            Some("method"),
        ),
        (
            LanguageKind::csharp(CSharpKind::Property),
            false,
            true,
            Some("variable"),
        ),
        (
            LanguageKind::csharp(CSharpKind::Field),
            false,
            true,
            Some("variable"),
        ),
        (
            LanguageKind::csharp(CSharpKind::Using),
            false,
            true,
            Some("import"),
        ),
        (
            LanguageKind::csharp(CSharpKind::Namespace),
            false,
            true,
            None,
        ),
        (
            LanguageKind::csharp(CSharpKind::FileScopedNamespace),
            false,
            true,
            None,
        ),
        (
            LanguageKind::csharp(CSharpKind::Method),
            true,
            true,
            Some("method"),
        ),
    ];

    for (language_kind, inside_parent, supported, canonical_kind) in expected {
        assert_eq!(
            is_supported_language_kind(CSHARP_SUPPORTED_LANGUAGE_KINDS, language_kind),
            supported
        );
        assert_eq!(
            resolve_canonical_kind(CSHARP_CANONICAL_MAPPINGS, language_kind, inside_parent)
                .map(CanonicalKindProjection::as_str),
            canonical_kind
        );
    }
}

#[test]
fn csharp_canonical_mapping_preserves_csharp_specific_structure() {
    let content = r#"using System.Collections.Generic;

namespace MyApp.Services;

public interface IUserService
{
    Task<User> GetUserAsync(int id);
}

public class UserService : IUserService
{
    private readonly IRepository _repo;

    public UserService(IRepository repo)
    {
        _repo = repo;
    }

    public string Name { get; }

    public Task<User> GetUserAsync(int id)
    {
        return _repo.FindByIdAsync(id);
    }
}

public enum UserRole
{
    Admin,
    Member
}
"#;

    let artefacts = extract_csharp_artefacts(content, "src/UserService.cs").unwrap();

    let import = artefact_by_language_kind(&artefacts, LanguageKind::csharp(CSharpKind::Using));
    assert_eq!(canonical_kind(import), Some("import"));

    let interface = artefact_by_name_and_language_kind(
        &artefacts,
        LanguageKind::csharp(CSharpKind::Interface),
        "IUserService",
    );
    assert_eq!(canonical_kind(interface), Some("interface"));

    let class = artefact_by_name_and_language_kind(
        &artefacts,
        LanguageKind::csharp(CSharpKind::Class),
        "UserService",
    );
    assert_eq!(canonical_kind(class), Some("type"));

    let constructor = artefact_by_name_and_language_kind(
        &artefacts,
        LanguageKind::csharp(CSharpKind::Constructor),
        "UserService",
    );
    assert_eq!(canonical_kind(constructor), Some("method"));

    let method = artefact_by_name_and_language_kind(
        &artefacts,
        LanguageKind::csharp(CSharpKind::Method),
        "GetUserAsync",
    );
    assert_eq!(canonical_kind(method), Some("method"));

    let property = artefact_by_name_and_language_kind(
        &artefacts,
        LanguageKind::csharp(CSharpKind::Property),
        "Name",
    );
    assert_eq!(canonical_kind(property), Some("variable"));

    let field = artefact_by_name_and_language_kind(
        &artefacts,
        LanguageKind::csharp(CSharpKind::Field),
        "_repo",
    );
    assert_eq!(canonical_kind(field), Some("variable"));
    assert_eq!(
        field.parent_symbol_fqn.as_deref(),
        Some("src/UserService.cs::UserService")
    );

    let enum_item = artefact_by_name_and_language_kind(
        &artefacts,
        LanguageKind::csharp(CSharpKind::Enum),
        "UserRole",
    );
    assert_eq!(canonical_kind(enum_item), Some("enum"));
}
