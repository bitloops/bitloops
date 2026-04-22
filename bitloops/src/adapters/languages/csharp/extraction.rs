use std::collections::HashSet;

use anyhow::{Context, Result};
use tree_sitter::Node;

use super::canonical::{CSHARP_CANONICAL_MAPPINGS, CSHARP_SUPPORTED_LANGUAGE_KINDS};
use crate::host::language_adapter::{
    CSharpKind, LanguageArtefact, LanguageKind, is_supported_language_kind,
    normalize_artefact_signature, resolve_canonical_kind,
};

struct CSharpArtefactDescriptor {
    language_kind: LanguageKind,
    name: String,
    symbol_fqn: String,
    parent_symbol_fqn: Option<String>,
    signature: String,
    modifiers: Vec<String>,
    docstring: Option<String>,
}

pub(crate) fn extract_csharp_artefacts(content: &str, path: &str) -> Result<Vec<LanguageArtefact>> {
    let mut parser = tree_sitter::Parser::new();
    let lang: tree_sitter::Language = tree_sitter_c_sharp::LANGUAGE.into();
    parser
        .set_language(&lang)
        .context("setting tree-sitter c# language")?;
    let Some(tree) = parser.parse(content, None) else {
        return Ok(Vec::new());
    };

    let root = tree.root_node();
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    collect_csharp_nodes_recursive(root, content, path, None, &mut out, &mut seen);
    out.sort_by_key(|artefact| {
        (
            artefact.start_line,
            artefact.end_line,
            artefact.canonical_kind.clone(),
            artefact.name.clone(),
        )
    });
    Ok(out)
}

pub(crate) fn trimmed_node_text(node: Node<'_>, content: &str) -> Option<String> {
    node.utf8_text(content.as_bytes())
        .ok()
        .map(str::trim)
        .map(str::to_string)
        .filter(|text| !text.is_empty())
}

fn collect_modifiers(node: Node<'_>, content: &str) -> Vec<String> {
    let mut modifiers = Vec::new();
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "modifier"
            && let Some(text) = trimmed_node_text(child, content)
        {
            modifiers.push(text);
        }
    }
    modifiers
}

fn first_line_of(node: Node<'_>, content: &str) -> String {
    trimmed_node_text(node, content)
        .and_then(|text| text.lines().next().map(normalize_artefact_signature))
        .unwrap_or_default()
}

fn collect_csharp_nodes_recursive(
    node: Node<'_>,
    content: &str,
    path: &str,
    parent_fqn: Option<&str>,
    out: &mut Vec<LanguageArtefact>,
    seen: &mut HashSet<(LanguageKind, String, i32)>,
) {
    match node.kind() {
        "class_declaration"
        | "struct_declaration"
        | "record_declaration"
        | "interface_declaration"
        | "enum_declaration"
        | "delegate_declaration" => {
            let language_kind = LanguageKind::csharp(
                CSharpKind::from_tree_sitter_kind(node.kind()).expect("validated csharp type kind"),
            );
            if let Some(name_node) = node.child_by_field_name("name")
                && let Some(name) = trimmed_node_text(name_node, content)
            {
                let symbol_fqn = parent_fqn
                    .map(|parent| format!("{parent}::{name}"))
                    .unwrap_or_else(|| format!("{path}::{name}"));
                push_csharp_artefact(
                    out,
                    seen,
                    node,
                    CSharpArtefactDescriptor {
                        language_kind,
                        name,
                        symbol_fqn: symbol_fqn.clone(),
                        parent_symbol_fqn: parent_fqn.map(str::to_string),
                        signature: first_line_of(node, content),
                        modifiers: collect_modifiers(node, content),
                        docstring: extract_xml_doc_comment(node, content),
                    },
                );

                let mut cursor = node.walk();
                for child in node.named_children(&mut cursor) {
                    collect_csharp_nodes_recursive(
                        child,
                        content,
                        path,
                        Some(&symbol_fqn),
                        out,
                        seen,
                    );
                }
                return;
            }
        }
        "namespace_declaration" | "file_scoped_namespace_declaration" => {
            let language_kind = LanguageKind::csharp(
                CSharpKind::from_tree_sitter_kind(node.kind())
                    .expect("validated csharp namespace kind"),
            );
            if let Some(name_node) = node.child_by_field_name("name")
                && let Some(name) = trimmed_node_text(name_node, content)
            {
                push_csharp_artefact(
                    out,
                    seen,
                    node,
                    CSharpArtefactDescriptor {
                        language_kind,
                        name: name.clone(),
                        symbol_fqn: format!("{path}::ns::{name}"),
                        parent_symbol_fqn: None,
                        signature: first_line_of(node, content),
                        modifiers: collect_modifiers(node, content),
                        docstring: extract_xml_doc_comment(node, content),
                    },
                );
            }
        }
        "method_declaration" | "constructor_declaration" => {
            let language_kind = LanguageKind::csharp(
                CSharpKind::from_tree_sitter_kind(node.kind())
                    .expect("validated csharp callable kind"),
            );
            if let Some(name_node) = node.child_by_field_name("name")
                && let Some(name) = trimmed_node_text(name_node, content)
            {
                let symbol_fqn = parent_fqn
                    .map(|parent| format!("{parent}::{name}"))
                    .unwrap_or_else(|| format!("{path}::{name}"));
                push_csharp_artefact(
                    out,
                    seen,
                    node,
                    CSharpArtefactDescriptor {
                        language_kind,
                        name,
                        symbol_fqn,
                        parent_symbol_fqn: parent_fqn.map(str::to_string),
                        signature: first_line_of(node, content),
                        modifiers: collect_modifiers(node, content),
                        docstring: extract_xml_doc_comment(node, content),
                    },
                );
            }
        }
        "property_declaration" => {
            if let Some(name_node) = node.child_by_field_name("name")
                && let Some(name) = trimmed_node_text(name_node, content)
            {
                let symbol_fqn = parent_fqn
                    .map(|parent| format!("{parent}::{name}"))
                    .unwrap_or_else(|| format!("{path}::{name}"));
                push_csharp_artefact(
                    out,
                    seen,
                    node,
                    CSharpArtefactDescriptor {
                        language_kind: LanguageKind::csharp(CSharpKind::Property),
                        name,
                        symbol_fqn,
                        parent_symbol_fqn: parent_fqn.map(str::to_string),
                        signature: first_line_of(node, content),
                        modifiers: collect_modifiers(node, content),
                        docstring: extract_xml_doc_comment(node, content),
                    },
                );
            }
        }
        "field_declaration" => {
            let signature = first_line_of(node, content);
            let modifiers = collect_modifiers(node, content);
            let docstring = extract_xml_doc_comment(node, content);
            let mut cursor = node.walk();
            for child in node.named_children(&mut cursor) {
                if child.kind() != "variable_declaration" {
                    continue;
                }
                let mut inner = child.walk();
                for declarator in child.named_children(&mut inner) {
                    if declarator.kind() != "variable_declarator" {
                        continue;
                    }
                    if let Some(name_node) = declarator.child_by_field_name("name")
                        && let Some(name) = trimmed_node_text(name_node, content)
                    {
                        let symbol_fqn = parent_fqn
                            .map(|parent| format!("{parent}::{name}"))
                            .unwrap_or_else(|| format!("{path}::{name}"));
                        push_csharp_artefact(
                            out,
                            seen,
                            declarator,
                            CSharpArtefactDescriptor {
                                language_kind: LanguageKind::csharp(CSharpKind::Field),
                                name,
                                symbol_fqn,
                                parent_symbol_fqn: parent_fqn.map(str::to_string),
                                signature: signature.clone(),
                                modifiers: modifiers.clone(),
                                docstring: docstring.clone(),
                            },
                        );
                    }
                }
            }
        }
        "using_directive" => {
            if let Some(name) = using_target_name(node, content) {
                let line_no = node.start_position().row as i32 + 1;
                push_csharp_artefact(
                    out,
                    seen,
                    node,
                    CSharpArtefactDescriptor {
                        language_kind: LanguageKind::csharp(CSharpKind::Using),
                        name: name.clone(),
                        symbol_fqn: format!("{path}::using::{name}@{line_no}"),
                        parent_symbol_fqn: None,
                        signature: first_line_of(node, content),
                        modifiers: collect_modifiers(node, content),
                        docstring: extract_xml_doc_comment(node, content),
                    },
                );
            }
        }
        _ => {}
    }

    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        collect_csharp_nodes_recursive(child, content, path, parent_fqn, out, seen);
    }
}

pub(crate) fn using_target_name(node: Node<'_>, content: &str) -> Option<String> {
    node.child_by_field_name("name")
        .and_then(|name_node| trimmed_node_text(name_node, content))
        .or_else(|| {
            let mut cursor = node.walk();
            node.named_children(&mut cursor)
                .find_map(|child| trimmed_node_text(child, content))
        })
}

fn push_csharp_artefact(
    out: &mut Vec<LanguageArtefact>,
    seen: &mut HashSet<(LanguageKind, String, i32)>,
    node: Node<'_>,
    descriptor: CSharpArtefactDescriptor,
) {
    let CSharpArtefactDescriptor {
        language_kind,
        name,
        symbol_fqn,
        parent_symbol_fqn,
        signature,
        modifiers,
        docstring,
    } = descriptor;

    if name.is_empty()
        || !is_supported_language_kind(CSHARP_SUPPORTED_LANGUAGE_KINDS, language_kind)
    {
        return;
    }

    let start_line = node.start_position().row as i32 + 1;
    if !seen.insert((language_kind, name.clone(), start_line)) {
        return;
    }

    out.push(LanguageArtefact {
        canonical_kind: resolve_canonical_kind(
            CSHARP_CANONICAL_MAPPINGS,
            language_kind,
            parent_symbol_fqn.is_some(),
        )
        .map(|kind| kind.as_str().to_string()),
        language_kind,
        name,
        symbol_fqn,
        parent_symbol_fqn,
        start_line,
        end_line: node.end_position().row as i32 + 1,
        start_byte: node.start_byte() as i32,
        end_byte: node.end_byte() as i32,
        signature,
        modifiers,
        docstring,
    });
}

fn extract_xml_doc_comment(node: Node<'_>, content: &str) -> Option<String> {
    let mut current = node.prev_sibling();
    let mut lines = Vec::new();

    while let Some(comment) = current {
        if comment.kind() != "comment" && comment.kind() != "single_line_comment" {
            break;
        }
        let Some(text) = trimmed_node_text(comment, content) else {
            break;
        };
        if !text.starts_with("///") {
            break;
        }
        lines.push(text.trim_start_matches("///").trim().to_string());
        current = comment.prev_sibling();
    }

    if lines.is_empty() {
        None
    } else {
        lines.reverse();
        Some(lines.join("\n"))
    }
}

#[cfg(test)]
mod tests {
    use super::extract_csharp_artefacts;
    use crate::host::language_adapter::{CSharpKind, LanguageKind};

    #[test]
    fn extract_csharp_artefacts_collects_classes_methods_interfaces_and_usings() {
        let content = r#"
using System;
using System.Collections.Generic;

namespace MyApp.Services
{
    /// The main service class.
    public class UserService : IUserService
    {
        private readonly IRepository _repo;

        public UserService(IRepository repo)
        {
            _repo = repo;
        }

        public async Task<User> GetUserAsync(int id)
        {
            return await _repo.FindByIdAsync(id);
        }
    }

    public interface IUserService
    {
        Task<User> GetUserAsync(int id);
    }

    public enum UserRole { Admin, Member, Guest }
}
"#;

        let artefacts = extract_csharp_artefacts(content, "Services/UserService.cs").unwrap();

        assert!(artefacts.iter().any(|artefact| {
            artefact.language_kind == LanguageKind::csharp(CSharpKind::Class)
                && artefact.name == "UserService"
                && artefact.canonical_kind.as_deref() == Some("type")
        }));
        assert!(artefacts.iter().any(|artefact| {
            artefact.language_kind == LanguageKind::csharp(CSharpKind::Method)
                && artefact.name == "GetUserAsync"
                && artefact.canonical_kind.as_deref() == Some("method")
        }));
        assert!(artefacts.iter().any(|artefact| {
            artefact.language_kind == LanguageKind::csharp(CSharpKind::Interface)
                && artefact.name == "IUserService"
                && artefact.canonical_kind.as_deref() == Some("interface")
        }));
        assert!(artefacts.iter().any(|artefact| {
            artefact.language_kind == LanguageKind::csharp(CSharpKind::Enum)
                && artefact.name == "UserRole"
                && artefact.canonical_kind.as_deref() == Some("enum")
        }));
        assert!(artefacts.iter().any(|artefact| {
            artefact.language_kind == LanguageKind::csharp(CSharpKind::Using)
                && artefact.name == "System"
        }));
        assert!(artefacts.iter().any(|artefact| {
            artefact.language_kind == LanguageKind::csharp(CSharpKind::Constructor)
                && artefact.name == "UserService"
                && artefact.parent_symbol_fqn.is_some()
        }));
    }

    #[test]
    fn extract_csharp_artefacts_handles_file_scoped_namespaces_doc_comments_and_multi_field_decls()
    {
        let content = r#"using MyApp.Core;

namespace MyApp.Services;

/// <summary>
/// Coordinates user operations.
/// </summary>
public record UserService
{
    private int _first, _second;

    /// <summary>
    /// Loads the current user.
    /// </summary>
    public User Load(UserId id)
    {
        return new User();
    }
}
"#;

        let artefacts = extract_csharp_artefacts(content, "src/UserService.cs").unwrap();

        let namespace = artefacts
            .iter()
            .find(|artefact| {
                artefact.language_kind == LanguageKind::csharp(CSharpKind::FileScopedNamespace)
                    && artefact.name == "MyApp.Services"
            })
            .expect("missing file-scoped namespace artefact");
        assert_eq!(
            namespace.symbol_fqn,
            "src/UserService.cs::ns::MyApp.Services"
        );

        let record = artefacts
            .iter()
            .find(|artefact| {
                artefact.language_kind == LanguageKind::csharp(CSharpKind::Record)
                    && artefact.name == "UserService"
            })
            .expect("missing record artefact");
        assert_eq!(record.canonical_kind.as_deref(), Some("type"));
        assert_eq!(
            record.docstring.as_deref(),
            Some("<summary>\nCoordinates user operations.\n</summary>")
        );

        let first_field = artefacts
            .iter()
            .find(|artefact| {
                artefact.language_kind == LanguageKind::csharp(CSharpKind::Field)
                    && artefact.name == "_first"
            })
            .expect("missing first field artefact");
        let second_field = artefacts
            .iter()
            .find(|artefact| {
                artefact.language_kind == LanguageKind::csharp(CSharpKind::Field)
                    && artefact.name == "_second"
            })
            .expect("missing second field artefact");
        assert_eq!(
            first_field.parent_symbol_fqn.as_deref(),
            Some("src/UserService.cs::UserService")
        );
        assert_eq!(
            second_field.parent_symbol_fqn.as_deref(),
            Some("src/UserService.cs::UserService")
        );
        assert!(first_field.start_byte < first_field.end_byte);
        assert!(second_field.start_byte < second_field.end_byte);
        assert_ne!(first_field.start_byte, second_field.start_byte);
        assert_ne!(first_field.end_byte, second_field.end_byte);

        let method = artefacts
            .iter()
            .find(|artefact| {
                artefact.language_kind == LanguageKind::csharp(CSharpKind::Method)
                    && artefact.name == "Load"
            })
            .expect("missing method artefact");
        assert_eq!(method.canonical_kind.as_deref(), Some("method"));
        assert_eq!(
            method.docstring.as_deref(),
            Some("<summary>\nLoads the current user.\n</summary>")
        );
    }
}
