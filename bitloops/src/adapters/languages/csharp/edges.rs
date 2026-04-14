use std::collections::{HashMap, HashSet};

use anyhow::{Context, Result};
use tree_sitter::Node;

use super::extraction::{trimmed_node_text, using_target_name};
use crate::host::devql::{
    CallForm, CanonicalKindProjection, EdgeKind, ImportForm, RefKind, Resolution,
};
use crate::host::language_adapter::{
    DependencyEdge, EdgeMetadata, LanguageArtefact,
    edges_shared::{EdgeCollector, SymbolLookup, push_extends_edge, push_reference_edge},
};

struct CSharpTraversalCtx<'a> {
    content: &'a str,
    path: &'a str,
    artefacts: &'a [LanguageArtefact],
    callable_name_to_fqn: &'a HashMap<String, String>,
    type_targets: &'a HashMap<String, String>,
    non_interface_type_targets: &'a HashSet<String>,
    out: &'a mut Vec<DependencyEdge>,
    seen_imports: &'a mut HashSet<String>,
    seen_calls: &'a mut HashSet<String>,
    seen_refs: &'a mut HashSet<String>,
    seen_extends: &'a mut HashSet<String>,
    seen_implements: &'a mut HashSet<String>,
}

pub(crate) fn extract_csharp_dependency_edges(
    content: &str,
    path: &str,
    artefacts: &[LanguageArtefact],
) -> Result<Vec<DependencyEdge>> {
    let mut parser = tree_sitter::Parser::new();
    let lang: tree_sitter::Language = tree_sitter_c_sharp::LANGUAGE.into();
    parser
        .set_language(&lang)
        .context("setting tree-sitter c# language")?;
    let Some(tree) = parser.parse(content, None) else {
        return Ok(Vec::new());
    };

    let mut callable_name_to_fqn = HashMap::new();
    let mut type_targets = HashMap::new();
    let mut non_interface_type_targets = HashSet::new();
    for artefact in artefacts {
        let projected = artefact
            .canonical_kind
            .as_deref()
            .and_then(CanonicalKindProjection::from_str);
        if projected.is_some_and(|kind| {
            matches!(
                kind,
                CanonicalKindProjection::Function | CanonicalKindProjection::Method
            )
        }) {
            callable_name_to_fqn
                .entry(artefact.name.clone())
                .or_insert_with(|| artefact.symbol_fqn.clone());
        }
        if projected.is_some_and(|kind| {
            matches!(
                kind,
                CanonicalKindProjection::Type
                    | CanonicalKindProjection::Interface
                    | CanonicalKindProjection::Enum
            )
        }) {
            type_targets
                .entry(artefact.name.clone())
                .or_insert_with(|| artefact.symbol_fqn.clone());
        }
        if projected.is_some_and(|kind| {
            matches!(
                kind,
                CanonicalKindProjection::Type | CanonicalKindProjection::Enum
            )
        }) {
            non_interface_type_targets.insert(artefact.name.clone());
        }
    }

    let root = tree.root_node();
    let mut edges = Vec::new();
    let mut seen_imports = HashSet::new();
    let mut seen_calls = HashSet::new();
    let mut seen_refs = HashSet::new();
    let mut seen_extends = HashSet::new();
    let mut seen_implements = HashSet::new();
    let mut ctx = CSharpTraversalCtx {
        content,
        path,
        artefacts,
        callable_name_to_fqn: &callable_name_to_fqn,
        type_targets: &type_targets,
        non_interface_type_targets: &non_interface_type_targets,
        out: &mut edges,
        seen_imports: &mut seen_imports,
        seen_calls: &mut seen_calls,
        seen_refs: &mut seen_refs,
        seen_extends: &mut seen_extends,
        seen_implements: &mut seen_implements,
    };
    collect_csharp_edges_recursive(root, &mut ctx);
    Ok(edges)
}

fn collect_csharp_edges_recursive(node: Node<'_>, ctx: &mut CSharpTraversalCtx<'_>) {
    match node.kind() {
        "using_directive" => collect_using_edge(node, ctx),
        "invocation_expression" => collect_call_edge(node, ctx),
        "base_list" => collect_inheritance_edges(node, ctx),
        "identifier" | "identifier_name" | "qualified_name" | "generic_name"
        | "predefined_type" | "nullable_type" => collect_type_reference_edge(node, ctx),
        _ => {}
    }

    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        collect_csharp_edges_recursive(child, ctx);
    }
}

fn collect_using_edge(node: Node<'_>, ctx: &mut CSharpTraversalCtx<'_>) {
    let Some(namespace_name) = using_target_name(node, ctx.content) else {
        return;
    };
    let key = format!("{}|{}", ctx.path, namespace_name);
    if !ctx.seen_imports.insert(key) {
        return;
    }
    ctx.out.push(DependencyEdge {
        edge_kind: EdgeKind::Imports,
        from_symbol_fqn: ctx.path.to_string(),
        to_target_symbol_fqn: None,
        to_symbol_ref: Some(namespace_name),
        start_line: Some(node.start_position().row as i32 + 1),
        end_line: Some(node.end_position().row as i32 + 1),
        metadata: EdgeMetadata::import(ImportForm::Binding),
    });
}

fn collect_call_edge(node: Node<'_>, ctx: &mut CSharpTraversalCtx<'_>) {
    let line_no = node.start_position().row as i32 + 1;
    let Some(owner) = smallest_enclosing_symbol(line_no, ctx.artefacts) else {
        return;
    };
    let Some(expression) = node.child_by_field_name("function") else {
        return;
    };

    let (call_form, to_target_symbol_fqn, to_symbol_ref, resolution) = match expression.kind() {
        "identifier" | "identifier_name" | "generic_name" => {
            let Some(name) = trimmed_node_text(expression, ctx.content) else {
                return;
            };
            if let Some(target_fqn) = ctx.callable_name_to_fqn.get(&name) {
                (
                    CallForm::Function,
                    Some(target_fqn.clone()),
                    None,
                    Resolution::Local,
                )
            } else {
                (
                    CallForm::Function,
                    None,
                    Some(format!("{}::{name}", ctx.path)),
                    Resolution::Unresolved,
                )
            }
        }
        "member_access_expression" => {
            let receiver_text = expression
                .child_by_field_name("expression")
                .and_then(|receiver_node| trimmed_node_text(receiver_node, ctx.content))
                .unwrap_or_else(|| "member".to_string());
            let Some(member_node) = expression.child_by_field_name("name") else {
                return;
            };
            let Some(member_name) = trimmed_node_text(member_node, ctx.content) else {
                return;
            };
            (
                CallForm::Method,
                None,
                Some(format!(
                    "{}::member::{}::{}",
                    ctx.path, receiver_text, member_name
                )),
                Resolution::Unresolved,
            )
        }
        _ => return,
    };

    let key = format!(
        "{}|{}|{}|{}|{}|{}",
        owner.symbol_fqn,
        to_target_symbol_fqn.as_deref().unwrap_or(""),
        to_symbol_ref.as_deref().unwrap_or(""),
        line_no,
        call_form.as_str(),
        resolution.as_str()
    );
    if !ctx.seen_calls.insert(key) {
        return;
    }

    ctx.out.push(DependencyEdge {
        edge_kind: EdgeKind::Calls,
        from_symbol_fqn: owner.symbol_fqn,
        to_target_symbol_fqn,
        to_symbol_ref,
        start_line: Some(line_no),
        end_line: Some(line_no),
        metadata: EdgeMetadata::call(call_form, resolution),
    });
}

fn collect_inheritance_edges(node: Node<'_>, ctx: &mut CSharpTraversalCtx<'_>) {
    let line_no = node.start_position().row as i32 + 1;
    let Some(owner) = smallest_enclosing_type(line_no, ctx.artefacts) else {
        return;
    };
    let owner_is_interface = owner.canonical_kind.as_deref() == Some("interface");
    let mut cursor = node.walk();
    for (index, child) in node.named_children(&mut cursor).enumerate() {
        let Some(name) = trimmed_node_text(child, ctx.content) else {
            continue;
        };
        let base_name = name.split('<').next().unwrap_or(&name).trim().to_string();
        let known_local_non_interface = ctx.non_interface_type_targets.contains(&base_name);
        if owner_is_interface || index > 0 || !known_local_non_interface {
            push_implements_edge(
                ctx,
                &owner.symbol_fqn,
                &base_name,
                line_no,
                owner_is_interface,
            );
        } else {
            push_extends_edge(
                &mut EdgeCollector {
                    out: ctx.out,
                    seen: ctx.seen_extends,
                },
                &owner.symbol_fqn,
                &base_name,
                line_no,
                &SymbolLookup {
                    local_targets: ctx.type_targets,
                    imported_symbol_refs: None,
                },
            );
        }
    }
}

fn push_implements_edge(
    ctx: &mut CSharpTraversalCtx<'_>,
    owner_symbol_fqn: &str,
    target_name: &str,
    line_no: i32,
    owner_is_interface: bool,
) {
    let to_target_symbol_fqn = ctx.type_targets.get(target_name).cloned();
    let to_symbol_ref = if to_target_symbol_fqn.is_some() {
        None
    } else {
        Some(target_name.to_string())
    };
    let key = format!(
        "{}|{}|{}|{}|{}",
        owner_symbol_fqn,
        if owner_is_interface {
            "extends"
        } else {
            "implements"
        },
        to_target_symbol_fqn.as_deref().unwrap_or(""),
        to_symbol_ref.as_deref().unwrap_or(""),
        line_no
    );
    let seen = if owner_is_interface {
        &mut ctx.seen_extends
    } else {
        &mut ctx.seen_implements
    };
    if !seen.insert(key) {
        return;
    }
    ctx.out.push(DependencyEdge {
        edge_kind: if owner_is_interface {
            EdgeKind::Extends
        } else {
            EdgeKind::Implements
        },
        from_symbol_fqn: owner_symbol_fqn.to_string(),
        to_target_symbol_fqn,
        to_symbol_ref,
        start_line: Some(line_no),
        end_line: Some(line_no),
        metadata: EdgeMetadata::none(),
    });
}

fn collect_type_reference_edge(node: Node<'_>, ctx: &mut CSharpTraversalCtx<'_>) {
    if !is_type_annotation_context(node) {
        return;
    }
    let line_no = node.start_position().row as i32 + 1;
    let Some(owner) = smallest_enclosing_symbol(line_no, ctx.artefacts) else {
        return;
    };
    let Some(name) = trimmed_node_text(node, ctx.content) else {
        return;
    };
    let base_name = name.split('<').next().unwrap_or(&name).trim();
    push_reference_edge(
        &mut EdgeCollector {
            out: ctx.out,
            seen: ctx.seen_refs,
        },
        &owner.symbol_fqn,
        base_name,
        line_no,
        RefKind::Type,
        &SymbolLookup {
            local_targets: ctx.type_targets,
            imported_symbol_refs: None,
        },
    );
}

fn is_type_annotation_context(node: Node<'_>) -> bool {
    let Some(parent) = node.parent() else {
        return false;
    };
    if let Some(name_node) = parent.child_by_field_name("name")
        && name_node.start_byte() == node.start_byte()
        && name_node.end_byte() == node.end_byte()
    {
        return false;
    }

    matches!(
        parent.kind(),
        "parameter"
            | "return_type"
            | "variable_declaration"
            | "field_declaration"
            | "property_declaration"
            | "base_list"
            | "type_argument_list"
            | "cast_expression"
            | "is_expression"
            | "as_expression"
            | "object_creation_expression"
            | "parameter_list"
    )
}

fn smallest_enclosing_symbol(
    line_no: i32,
    artefacts: &[LanguageArtefact],
) -> Option<LanguageArtefact> {
    artefacts
        .iter()
        .filter(|artefact| artefact.start_line <= line_no && artefact.end_line >= line_no)
        .min_by_key(|artefact| artefact.end_line - artefact.start_line)
        .cloned()
}

fn smallest_enclosing_type(
    line_no: i32,
    artefacts: &[LanguageArtefact],
) -> Option<LanguageArtefact> {
    artefacts
        .iter()
        .filter(|artefact| artefact.start_line <= line_no && artefact.end_line >= line_no)
        .filter(|artefact| {
            matches!(
                artefact.canonical_kind.as_deref(),
                Some("type") | Some("interface")
            )
        })
        .min_by_key(|artefact| artefact.end_line - artefact.start_line)
        .cloned()
}

#[cfg(test)]
mod tests {
    use super::extract_csharp_dependency_edges;
    use crate::adapters::languages::csharp::extraction::extract_csharp_artefacts;
    use crate::host::devql::EdgeKind;

    #[test]
    fn extract_csharp_dependency_edges_emit_import_call_extends_and_reference_edges() {
        let content = r#"using System.Collections.Generic;

namespace MyApp.Services;

public interface IRepository {}
public interface IAuditable {}
public class BaseService {}
public class User {}

public class UserService : BaseService, IRepository, IAuditable
{
    private readonly Helper _helper;
    private readonly List<User> _users;

    public UserService(Helper helper)
    {
        _helper = helper;
        _users = new List<User>();
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
"#;

        let path = "src/UserService.cs";
        let artefacts = extract_csharp_artefacts(content, path).unwrap();
        let edges = extract_csharp_dependency_edges(content, path, &artefacts).unwrap();

        assert!(edges.iter().any(|edge| {
            edge.edge_kind == EdgeKind::Imports
                && edge.to_symbol_ref.as_deref() == Some("System.Collections.Generic")
        }));
        assert!(edges.iter().any(|edge| {
            edge.edge_kind == EdgeKind::Calls
                && edge.from_symbol_fqn == "src/UserService.cs::UserService::GetUser"
        }));
        assert!(edges.iter().any(|edge| {
            edge.edge_kind == EdgeKind::Extends
                && edge.from_symbol_fqn == "src/UserService.cs::UserService"
        }));
        assert!(edges.iter().any(|edge| {
            edge.edge_kind == EdgeKind::Implements
                && edge.from_symbol_fqn == "src/UserService.cs::UserService"
                && edge.to_target_symbol_fqn.as_deref() == Some("src/UserService.cs::IRepository")
        }));
        assert!(edges.iter().any(|edge| {
            edge.edge_kind == EdgeKind::Implements
                && edge.from_symbol_fqn == "src/UserService.cs::UserService"
                && edge.to_target_symbol_fqn.as_deref() == Some("src/UserService.cs::IAuditable")
        }));
        assert!(edges.iter().any(|edge| {
            edge.edge_kind == EdgeKind::References
                && edge.from_symbol_fqn == "src/UserService.cs::UserService::_users"
        }));
    }

    #[test]
    fn extract_csharp_dependency_edges_deduplicates_repeated_imports_in_one_file() {
        let content = r#"using System.Collections.Generic;
using System.Collections.Generic;

public class UserService
{
    private readonly List<string> _names;
}
"#;

        let path = "src/UserService.cs";
        let artefacts = extract_csharp_artefacts(content, path).unwrap();
        let edges = extract_csharp_dependency_edges(content, path, &artefacts).unwrap();

        let import_edges = edges
            .iter()
            .filter(|edge| {
                edge.edge_kind == EdgeKind::Imports
                    && edge.to_symbol_ref.as_deref() == Some("System.Collections.Generic")
            })
            .count();

        assert_eq!(import_edges, 1);
    }

    #[test]
    fn extract_csharp_dependency_edges_resolve_local_method_calls_to_target_symbols() {
        let content = r#"public class UserService
{
    public string Helper()
    {
        return "ok";
    }

    public string Run()
    {
        return Helper();
    }
}
"#;

        let path = "src/UserService.cs";
        let artefacts = extract_csharp_artefacts(content, path).unwrap();
        let edges = extract_csharp_dependency_edges(content, path, &artefacts).unwrap();

        assert!(edges.iter().any(|edge| {
            edge.edge_kind == EdgeKind::Calls
                && edge.from_symbol_fqn == "src/UserService.cs::UserService::Run"
                && edge.to_target_symbol_fqn.as_deref()
                    == Some("src/UserService.cs::UserService::Helper")
        }));
    }

    #[test]
    fn extract_csharp_dependency_edges_preserve_receiver_context_for_unresolved_member_calls() {
        let content = r#"public class UserService
{
    public void Run(Repo repo, Cache cache)
    {
        repo.Save();
        cache.Save();
    }
}
"#;

        let path = "src/UserService.cs";
        let artefacts = extract_csharp_artefacts(content, path).unwrap();
        let edges = extract_csharp_dependency_edges(content, path, &artefacts).unwrap();

        assert!(edges.iter().any(|edge| {
            edge.edge_kind == EdgeKind::Calls
                && edge.from_symbol_fqn == "src/UserService.cs::UserService::Run"
                && edge.to_symbol_ref.as_deref() == Some("src/UserService.cs::member::repo::Save")
        }));
        assert!(edges.iter().any(|edge| {
            edge.edge_kind == EdgeKind::Calls
                && edge.from_symbol_fqn == "src/UserService.cs::UserService::Run"
                && edge.to_symbol_ref.as_deref() == Some("src/UserService.cs::member::cache::Save")
        }));
    }

    #[test]
    fn extract_csharp_dependency_edges_keep_interface_inheritance_as_extends() {
        let content = r#"public interface IBase {}

public interface IDerived : IBase {}
"#;

        let path = "src/UserService.cs";
        let artefacts = extract_csharp_artefacts(content, path).unwrap();
        let edges = extract_csharp_dependency_edges(content, path, &artefacts).unwrap();

        assert!(edges.iter().any(|edge| {
            edge.edge_kind == EdgeKind::Extends
                && edge.from_symbol_fqn == "src/UserService.cs::IDerived"
                && edge.to_target_symbol_fqn.as_deref() == Some("src/UserService.cs::IBase")
        }));
        assert!(!edges.iter().any(|edge| {
            edge.edge_kind == EdgeKind::Implements
                && edge.from_symbol_fqn == "src/UserService.cs::IDerived"
        }));
    }

    #[test]
    fn extract_csharp_dependency_edges_treat_external_interface_first_base_entry_as_implements() {
        let content = r#"using System;

public class UserService : IDisposable
{
    public void Dispose() {}
}
"#;

        let path = "src/UserService.cs";
        let artefacts = extract_csharp_artefacts(content, path).unwrap();
        let edges = extract_csharp_dependency_edges(content, path, &artefacts).unwrap();

        assert!(edges.iter().any(|edge| {
            edge.edge_kind == EdgeKind::Implements
                && edge.from_symbol_fqn == "src/UserService.cs::UserService"
                && edge.to_symbol_ref.as_deref() == Some("IDisposable")
        }));
        assert!(!edges.iter().any(|edge| {
            edge.edge_kind == EdgeKind::Extends
                && edge.from_symbol_fqn == "src/UserService.cs::UserService"
        }));
    }
}
