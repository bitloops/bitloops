use super::*;

#[test]
fn devql_extension_host_resolves_built_in_language_pack_ownership() {
    assert_eq!(
        resolve_language_pack_owner("csharp"),
        Some(CSHARP_LANGUAGE_PACK_ID)
    );
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
        resolve_language_pack_owner("php"),
        Some(PHP_LANGUAGE_PACK_ID)
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
    assert_eq!(
        resolve_language_id_for_file_path("src/main.cs"),
        Some("csharp")
    );
    assert_eq!(
        resolve_language_id_for_file_path("src/main.php"),
        Some("php")
    );
    assert!(resolve_language_id_for_file_path("README").is_none());
}

#[test]
fn devql_language_adapter_registry_resolves_built_in_pack_implementations() {
    let registry = language_adapter_registry().expect("initialize language adapter registry");
    assert_eq!(
        registry.registered_pack_ids(),
        vec![
            CSHARP_LANGUAGE_PACK_ID,
            GO_LANGUAGE_PACK_ID,
            JAVA_LANGUAGE_PACK_ID,
            PHP_LANGUAGE_PACK_ID,
            PYTHON_LANGUAGE_PACK_ID,
            RUST_LANGUAGE_PACK_ID,
            TS_JS_LANGUAGE_PACK_ID
        ]
    );
    assert!(registry.get(CSHARP_LANGUAGE_PACK_ID).is_some());
    assert!(registry.get(GO_LANGUAGE_PACK_ID).is_some());
    assert!(registry.get(JAVA_LANGUAGE_PACK_ID).is_some());
    assert!(registry.get(RUST_LANGUAGE_PACK_ID).is_some());
    assert!(registry.get(TS_JS_LANGUAGE_PACK_ID).is_some());
    assert!(registry.get(PYTHON_LANGUAGE_PACK_ID).is_some());
    assert!(registry.get(PHP_LANGUAGE_PACK_ID).is_some());
    assert!(registry.get("unknown-pack").is_none());
}

#[test]
fn devql_language_adapter_registry_executes_rust_ts_js_python_go_java_csharp_and_php_built_ins() {
    let count_kind = |edges: &[DependencyEdge], kind: EdgeKind| -> usize {
        edges.iter().filter(|edge| edge.edge_kind == kind).count()
    };

    let registry = language_adapter_registry().expect("initialize language adapter registry");
    let csharp_pack = registry
        .get(CSHARP_LANGUAGE_PACK_ID)
        .expect("resolve csharp built-in language adapter pack");
    let csharp_content = r#"using System.Collections.Generic;

namespace Acme.Services;

public interface IRepository {}

public class BaseService {}

public class UserService : BaseService, IRepository
{
    private readonly Helper _helper;

    public UserService(Helper helper)
    {
        _helper = helper;
    }

    public User GetUser()
    {
        return _helper.Load();
    }
}

public class Helper
{
    public User Load()
    {
        return new User();
    }
}

public class User {}
"#;
    let csharp_artefacts = csharp_pack
        .extract_artefacts(csharp_content, "src/UserService.cs")
        .expect("extract csharp artefacts via language adapter registry");
    assert!(
        csharp_artefacts
            .iter()
            .any(|artefact| artefact.name == "UserService"),
        "csharp built-in registry pack should surface type artefacts"
    );
    let csharp_edges = csharp_pack
        .extract_dependency_edges(csharp_content, "src/UserService.cs", &csharp_artefacts)
        .expect("extract csharp dependency edges via language adapter registry");
    assert_eq!(count_kind(&csharp_edges, EdgeKind::Calls), 1);
    assert_eq!(count_kind(&csharp_edges, EdgeKind::Imports), 1);
    assert_eq!(count_kind(&csharp_edges, EdgeKind::Extends), 1);
    assert_eq!(count_kind(&csharp_edges, EdgeKind::Implements), 1);
    assert!(csharp_edges.iter().any(|edge| {
        edge.edge_kind == EdgeKind::Imports
            && edge.from_symbol_fqn == "src/UserService.cs"
            && edge.to_symbol_ref.as_deref() == Some("System.Collections.Generic")
    }));
    assert!(csharp_edges.iter().any(|edge| {
        edge.edge_kind == EdgeKind::Calls
            && edge.from_symbol_fqn == "src/UserService.cs::UserService::GetUser"
            && edge.to_symbol_ref.as_deref() == Some("src/UserService.cs::member::_helper::Load")
    }));
    assert!(csharp_edges.iter().any(|edge| {
        edge.edge_kind == EdgeKind::Extends
            && edge.from_symbol_fqn == "src/UserService.cs::UserService"
            && edge.to_target_symbol_fqn.as_deref() == Some("src/UserService.cs::BaseService")
    }));
    assert!(csharp_edges.iter().any(|edge| {
        edge.edge_kind == EdgeKind::Implements
            && edge.from_symbol_fqn == "src/UserService.cs::UserService"
            && edge.to_target_symbol_fqn.as_deref() == Some("src/UserService.cs::IRepository")
    }));

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
    assert_eq!(count_kind(&ts_edges, EdgeKind::Calls), 1);
    assert!(ts_edges.iter().any(|edge| {
        edge.edge_kind == EdgeKind::Calls
            && edge.from_symbol_fqn == "src/main.ts::greet"
            && edge.to_target_symbol_fqn.as_deref() == Some("src/main.ts::helper")
    }));

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
    assert_eq!(count_kind(&python_edges, EdgeKind::Calls), 2);
    assert_eq!(count_kind(&python_edges, EdgeKind::Imports), 1);
    assert_eq!(count_kind(&python_edges, EdgeKind::Extends), 1);
    assert!(python_edges.iter().any(|edge| {
        edge.edge_kind == EdgeKind::Imports
            && edge.from_symbol_fqn == "src/main.py"
            && edge.to_symbol_ref.as_deref() == Some("pkg.helpers")
    }));
    assert!(python_edges.iter().any(|edge| {
        edge.edge_kind == EdgeKind::Calls
            && edge.from_symbol_fqn == "src/main.py::Greeter::greet"
            && edge.to_symbol_ref.as_deref() == Some("pkg.helpers::helper")
    }));
    assert!(python_edges.iter().any(|edge| {
        edge.edge_kind == EdgeKind::Extends
            && edge.from_symbol_fqn == "src/main.py::Greeter"
            && edge.to_symbol_ref.as_deref() == Some("BaseGreeter")
    }));

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
    assert_eq!(count_kind(&go_edges, EdgeKind::Calls), 2);
    assert_eq!(count_kind(&go_edges, EdgeKind::Imports), 2);
    assert_eq!(count_kind(&go_edges, EdgeKind::Extends), 1);
    assert!(go_edges.iter().any(|edge| {
        edge.edge_kind == EdgeKind::Imports
            && edge.from_symbol_fqn == "service/run.go"
            && edge.to_symbol_ref.as_deref() == Some("context")
    }));
    assert!(go_edges.iter().any(|edge| {
        edge.edge_kind == EdgeKind::Calls
            && edge.from_symbol_fqn == "service/run.go::Run"
            && edge.to_target_symbol_fqn.as_deref() == Some("service/run.go::helper")
    }));
    assert!(go_edges.iter().any(|edge| {
        edge.edge_kind == EdgeKind::Extends
            && edge.from_symbol_fqn == "service/run.go::Handler"
            && edge.to_target_symbol_fqn.as_deref() == Some("service/run.go::Base")
    }));

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
    assert_eq!(count_kind(&java_edges, EdgeKind::Calls), 4);
    assert_eq!(count_kind(&java_edges, EdgeKind::Imports), 1);
    assert_eq!(count_kind(&java_edges, EdgeKind::Extends), 1);
    assert_eq!(count_kind(&java_edges, EdgeKind::Implements), 1);
    assert!(java_edges.iter().any(|edge| {
        edge.edge_kind == EdgeKind::Imports
            && edge.from_symbol_fqn == "src/com/acme/Greeter.java"
            && edge.to_symbol_ref.as_deref() == Some("java.util.List")
    }));
    assert!(java_edges.iter().any(|edge| {
        edge.edge_kind == EdgeKind::Calls
            && edge.from_symbol_fqn == "src/com/acme/Greeter.java::Greeter::greet"
            && edge.to_target_symbol_fqn.as_deref()
                == Some("src/com/acme/Greeter.java::Greeter::helper")
    }));
    assert!(java_edges.iter().any(|edge| {
        edge.edge_kind == EdgeKind::Extends
            && edge.from_symbol_fqn == "src/com/acme/Greeter.java::Greeter"
            && edge.to_target_symbol_fqn.as_deref() == Some("src/com/acme/Greeter.java::Base")
    }));
    assert!(java_edges.iter().any(|edge| {
        edge.edge_kind == EdgeKind::Implements
            && edge.from_symbol_fqn == "src/com/acme/Greeter.java::Greeter"
            && edge.to_target_symbol_fqn.as_deref() == Some("src/com/acme/Greeter.java::Runner")
    }));

    let php_pack = registry
        .get(PHP_LANGUAGE_PACK_ID)
        .expect("resolve php built-in language adapter pack");
    let php_content = r#"<?php
namespace App\Services;
use App\Core\Helper;

class UserService {
    public function run() {
        return helper();
    }
}

function helper() {
    return 1;
}
"#;
    let php_artefacts = php_pack
        .extract_artefacts(php_content, "src/UserService.php")
        .expect("extract php artefacts via language adapter registry");
    assert!(
        php_artefacts
            .iter()
            .any(|artefact| artefact.name == "UserService"),
        "php built-in registry pack should surface type artefacts"
    );
    let php_edges = php_pack
        .extract_dependency_edges(php_content, "src/UserService.php", &php_artefacts)
        .expect("extract php dependency edges via language adapter registry");
    assert!(php_edges.iter().any(|edge| {
        edge.edge_kind == EdgeKind::Imports
            && edge.to_symbol_ref.as_deref() == Some("App\\Core\\Helper")
    }));
}
