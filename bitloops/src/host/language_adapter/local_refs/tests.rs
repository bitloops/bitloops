use super::{
    LocalSourceFacts, LocalTargetInfo, normalize_local_edge_symbol_refs, resolve_local_symbol_ref,
    rust_local_symbol_fqn_candidates,
};

fn target(
    symbol_fqn: &str,
    symbol_id: &str,
    artefact_id: &str,
    language_kind: &str,
) -> LocalTargetInfo {
    LocalTargetInfo {
        symbol_fqn: symbol_fqn.to_string(),
        symbol_id: symbol_id.to_string(),
        artefact_id: artefact_id.to_string(),
        language_kind: language_kind.to_string(),
    }
}

#[test]
fn rust_local_symbol_fqn_candidates_handle_ruff_style_super_paths() {
    let candidates = rust_local_symbol_fqn_candidates(
        "crates/ruff_linter/src/rules/pyflakes/rules/strings.rs",
        "super::super::fixes::remove_unused_positional_arguments_from_format_call",
    );

    assert_eq!(
        candidates,
        vec![
            "crates/ruff_linter/src/rules/pyflakes/fixes.rs::remove_unused_positional_arguments_from_format_call".to_string(),
            "crates/ruff_linter/src/rules/pyflakes/fixes/mod.rs::remove_unused_positional_arguments_from_format_call".to_string(),
        ]
    );
}

#[test]
fn normalize_local_edge_symbol_refs_expands_grouped_rust_imports() {
    let refs = normalize_local_edge_symbol_refs(
        "rust",
        "crates/ruff_linter/src/rules/pyflakes/rules/strings.rs",
        "imports",
        "super::super::fixes::{remove_unused_positional_arguments_from_format_call, self}",
    );

    assert_eq!(
        refs,
        vec![
            "super::super::fixes::remove_unused_positional_arguments_from_format_call".to_string(),
            "super::super::fixes::self".to_string(),
        ]
    );
}

#[test]
fn normalize_local_edge_symbol_refs_preserves_rust_wildcard_imports() {
    let refs = normalize_local_edge_symbol_refs("rust", "src/lib.rs", "imports", "crate::math::*");

    assert_eq!(refs, vec!["crate::math::*".to_string()]);
}

#[test]
fn resolve_local_symbol_ref_handles_rust_grouped_self_imports() {
    let resolved = resolve_local_symbol_ref(
        "rust",
        "crates/ruff_linter/src/rules/pyflakes/rules/strings.rs",
        "imports",
        "super::super::fixes::self",
        &LocalSourceFacts::default(),
        &[target(
            "crates/ruff_linter/src/rules/pyflakes/fixes.rs",
            "fixes-file",
            "fixes-file-artefact",
            "file",
        )],
    )
    .expect("expected grouped self import to resolve to the module file");

    assert_eq!(
        resolved.symbol_fqn,
        "crates/ruff_linter/src/rules/pyflakes/fixes.rs"
    );
}

#[test]
fn resolve_local_symbol_ref_handles_typescript_relative_module_imports() {
    let resolved = resolve_local_symbol_ref(
        "typescript",
        "src/caller.ts",
        "imports",
        "./utils",
        &LocalSourceFacts::default(),
        &[target("src/utils.ts", "file", "artefact", "file")],
    )
    .expect("expected relative module import to resolve");

    assert_eq!(resolved.symbol_fqn, "src/utils.ts");
}

#[test]
fn resolve_local_symbol_ref_handles_typescript_relative_imports() {
    let resolved = resolve_local_symbol_ref(
        "typescript",
        "src/caller.ts",
        "calls",
        "./utils::helper",
        &LocalSourceFacts::default(),
        &[target(
            "src/utils.ts::helper",
            "helper",
            "artefact",
            "function_declaration",
        )],
    )
    .expect("expected relative import to resolve");

    assert_eq!(resolved.symbol_fqn, "src/utils.ts::helper");
}

#[test]
fn resolve_local_symbol_ref_rejects_bare_typescript_packages_for_import_edges() {
    let resolved = resolve_local_symbol_ref(
        "typescript",
        "src/caller.ts",
        "imports",
        "react",
        &LocalSourceFacts::default(),
        &[target("src/react.ts", "file", "artefact", "file")],
    );

    assert!(resolved.is_none());
}

#[test]
fn resolve_local_symbol_ref_handles_python_module_import_edges() {
    let resolved = resolve_local_symbol_ref(
        "python",
        "pkg/main.py",
        "imports",
        "pkg.helpers",
        &LocalSourceFacts::default(),
        &[target("pkg/helpers.py", "file", "artefact", "file")],
    )
    .expect("expected python module import edge to resolve");

    assert_eq!(resolved.symbol_fqn, "pkg/helpers.py");
}

#[test]
fn resolve_local_symbol_ref_handles_python_relative_import_edges() {
    let resolved = resolve_local_symbol_ref(
        "python",
        "pkg/sub/main.py",
        "imports",
        "..helpers",
        &LocalSourceFacts::default(),
        &[target("pkg/helpers.py", "file", "artefact", "file")],
    )
    .expect("expected python relative import edge to resolve");

    assert_eq!(resolved.symbol_fqn, "pkg/helpers.py");
}

#[test]
fn resolve_local_symbol_ref_handles_python_module_imports() {
    let resolved = resolve_local_symbol_ref(
        "python",
        "pkg/main.py",
        "calls",
        "pkg.helpers::helper",
        &LocalSourceFacts::default(),
        &[target(
            "pkg/helpers.py::helper",
            "helper",
            "artefact",
            "function_definition",
        )],
    )
    .expect("expected python module import to resolve");

    assert_eq!(resolved.symbol_fqn, "pkg/helpers.py::helper");
}

#[test]
fn resolve_local_symbol_ref_handles_go_same_package_refs() {
    let resolved = resolve_local_symbol_ref(
        "go",
        "service/run.go",
        "calls",
        "package::service::helper",
        &LocalSourceFacts::default(),
        &[target(
            "service/helper.go::helper",
            "helper",
            "artefact",
            "function_declaration",
        )],
    )
    .expect("expected go package ref to resolve");

    assert_eq!(resolved.symbol_fqn, "service/helper.go::helper");
}

#[test]
fn resolve_local_symbol_ref_handles_java_type_import_edges() {
    let resolved = resolve_local_symbol_ref(
        "java",
        "src/com/acme/Greeter.java",
        "imports",
        "com.acme.Util",
        &LocalSourceFacts {
            package_refs: vec!["com.acme".to_string()],
            ..LocalSourceFacts::default()
        },
        &[target(
            "src/com/acme/Util.java::Util",
            "util",
            "artefact",
            "class_declaration",
        )],
    )
    .expect("expected java type import to resolve");

    assert_eq!(resolved.symbol_fqn, "src/com/acme/Util.java::Util");
}

#[test]
fn resolve_local_symbol_ref_handles_java_static_member_import_edges() {
    let resolved = resolve_local_symbol_ref(
        "java",
        "src/com/acme/Greeter.java",
        "imports",
        "com.acme.Util.helper",
        &LocalSourceFacts {
            package_refs: vec!["com.acme".to_string()],
            ..LocalSourceFacts::default()
        },
        &[target(
            "src/com/acme/Util.java::Util::helper",
            "helper",
            "artefact",
            "method_declaration",
        )],
    )
    .expect("expected java static import to resolve");

    assert_eq!(resolved.symbol_fqn, "src/com/acme/Util.java::Util::helper");
}

#[test]
fn resolve_local_symbol_ref_prefers_java_type_import_edges_before_member_like_matches() {
    let targets = [
        target(
            "src/com/acme.java::acme::Util",
            "nested-type",
            "nested-artefact",
            "class_declaration",
        ),
        target(
            "src/com/acme/Util.java::Util",
            "imported-type",
            "type-artefact",
            "class_declaration",
        ),
    ];
    let resolved = resolve_local_symbol_ref(
        "java",
        "src/com/example/Greeter.java",
        "imports",
        "com.acme.Util",
        &LocalSourceFacts {
            package_refs: vec!["com.example".to_string()],
            ..LocalSourceFacts::default()
        },
        &targets,
    )
    .expect("expected java type import to resolve to the imported type");

    assert_eq!(resolved.symbol_fqn, "src/com/acme/Util.java::Util");
}

#[test]
fn resolve_local_symbol_ref_handles_java_package_qualified_calls() {
    let resolved = resolve_local_symbol_ref(
        "java",
        "src/com/acme/Greeter.java",
        "calls",
        "com.acme.Util::helper",
        &LocalSourceFacts {
            package_refs: vec!["com.acme".to_string()],
            ..LocalSourceFacts::default()
        },
        &[target(
            "src/com/acme/Util.java::Util::helper",
            "helper",
            "artefact",
            "method_declaration",
        )],
    )
    .expect("expected java imported type call to resolve");

    assert_eq!(resolved.symbol_fqn, "src/com/acme/Util.java::Util::helper");
}

#[test]
fn resolve_local_symbol_ref_handles_java_same_package_type_refs() {
    let resolved = resolve_local_symbol_ref(
        "java",
        "src/com/acme/Greeter.java",
        "extends",
        "Base",
        &LocalSourceFacts {
            package_refs: vec!["com.acme".to_string()],
            ..LocalSourceFacts::default()
        },
        &[target(
            "src/com/acme/Base.java::Base",
            "base",
            "artefact",
            "class_declaration",
        )],
    )
    .expect("expected java same-package type ref to resolve");

    assert_eq!(resolved.symbol_fqn, "src/com/acme/Base.java::Base");
}

#[test]
fn resolve_local_symbol_ref_rejects_java_wildcard_imports() {
    let resolved = resolve_local_symbol_ref(
        "java",
        "src/com/acme/Greeter.java",
        "imports",
        "com.acme.*",
        &LocalSourceFacts {
            package_refs: vec!["com.acme".to_string()],
            ..LocalSourceFacts::default()
        },
        &[target(
            "src/com/acme/Util.java::Util",
            "util",
            "artefact",
            "class_declaration",
        )],
    );

    assert!(resolved.is_none());
}

#[test]
fn resolve_local_symbol_ref_handles_csharp_namespace_import_edges() {
    let resolved = resolve_local_symbol_ref(
        "csharp",
        "src/UserService.cs",
        "imports",
        "MyApp.Services",
        &LocalSourceFacts::default(),
        &[target(
            "src/BaseService.cs::ns::MyApp.Services",
            "ns",
            "ns-artefact",
            "file_scoped_namespace_declaration",
        )],
    )
    .expect("expected csharp namespace import to resolve");

    assert_eq!(
        resolved.symbol_fqn,
        "src/BaseService.cs::ns::MyApp.Services"
    );
}

#[test]
fn resolve_local_symbol_ref_handles_csharp_fully_qualified_type_import_edges() {
    let targets = [
        target(
            "src/BaseService.cs::ns::MyApp.Services",
            "ns",
            "ns-artefact",
            "file_scoped_namespace_declaration",
        ),
        target(
            "src/BaseService.cs::BaseService",
            "base",
            "base-artefact",
            "class_declaration",
        ),
    ];
    let resolved = resolve_local_symbol_ref(
        "csharp",
        "src/UserService.cs",
        "imports",
        "MyApp.Services.BaseService",
        &LocalSourceFacts::default(),
        &targets,
    )
    .expect("expected csharp fully qualified type import to resolve");

    assert_eq!(resolved.symbol_fqn, "src/BaseService.cs::BaseService");
}

#[test]
fn resolve_local_symbol_ref_handles_csharp_namespace_type_refs() {
    let targets = [
        target(
            "src/BaseService.cs::ns::MyApp.Services",
            "ns",
            "ns-artefact",
            "file_scoped_namespace_declaration",
        ),
        target(
            "src/BaseService.cs::BaseService",
            "base",
            "base-artefact",
            "class_declaration",
        ),
    ];
    let resolved = resolve_local_symbol_ref(
        "csharp",
        "src/UserService.cs",
        "extends",
        "BaseService",
        &LocalSourceFacts {
            namespace_refs: vec!["MyApp.Services".to_string()],
            ..LocalSourceFacts::default()
        },
        &targets,
    )
    .expect("expected csharp namespace type ref to resolve");

    assert_eq!(resolved.symbol_fqn, "src/BaseService.cs::BaseService");
    assert_eq!(resolved.edge_kind, "extends");
}

#[test]
fn resolve_local_symbol_ref_handles_csharp_imported_namespace_type_refs() {
    let targets = [
        target(
            "src/BaseService.cs::ns::MyApp.Services",
            "ns",
            "ns-artefact",
            "file_scoped_namespace_declaration",
        ),
        target(
            "src/BaseService.cs::BaseService",
            "base",
            "base-artefact",
            "class_declaration",
        ),
    ];
    let resolved = resolve_local_symbol_ref(
        "csharp",
        "src/UserService.cs",
        "implements",
        "BaseService",
        &LocalSourceFacts {
            import_refs: vec!["MyApp.Services".to_string()],
            ..LocalSourceFacts::default()
        },
        &targets,
    )
    .expect("expected csharp imported namespace type ref to resolve");

    assert_eq!(resolved.symbol_fqn, "src/BaseService.cs::BaseService");
    assert_eq!(resolved.edge_kind, "extends");
}

#[test]
fn resolve_local_symbol_ref_handles_csharp_canonical_imported_type_refs() {
    let targets = [
        target(
            "src/BaseService.cs::ns::MyApp.Services",
            "ns",
            "ns-artefact",
            "file_scoped_namespace_declaration",
        ),
        target(
            "src/BaseService.cs::BaseService",
            "base",
            "base-artefact",
            "class_declaration",
        ),
    ];
    let resolved = resolve_local_symbol_ref(
        "csharp",
        "src/UserService.cs",
        "implements",
        "BaseService",
        &LocalSourceFacts {
            import_refs: vec!["src/BaseService.cs::BaseService".to_string()],
            ..LocalSourceFacts::default()
        },
        &targets,
    )
    .expect("expected csharp canonical imported type ref to resolve");

    assert_eq!(resolved.symbol_fqn, "src/BaseService.cs::BaseService");
    assert_eq!(resolved.edge_kind, "extends");
}

#[test]
fn resolve_local_symbol_ref_rejects_csharp_alias_like_import_edges() {
    let resolved = resolve_local_symbol_ref(
        "csharp",
        "src/UserService.cs",
        "imports",
        "Alias=MyApp.Services",
        &LocalSourceFacts::default(),
        &[target(
            "src/BaseService.cs::ns::MyApp.Services",
            "ns",
            "ns-artefact",
            "file_scoped_namespace_declaration",
        )],
    );

    assert!(resolved.is_none());
}

#[test]
fn resolve_local_symbol_ref_rejects_ambiguous_python_matches() {
    let resolved = resolve_local_symbol_ref(
        "python",
        "pkg/main.py",
        "calls",
        "pkg.helpers::helper",
        &LocalSourceFacts::default(),
        &[
            target(
                "pkg/helpers.py::helper",
                "a",
                "artefact-a",
                "function_definition",
            ),
            target(
                "pkg/helpers/__init__.py::helper",
                "b",
                "artefact-b",
                "function_definition",
            ),
        ],
    );

    assert!(resolved.is_none());
}
