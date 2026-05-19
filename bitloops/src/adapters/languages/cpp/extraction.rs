use std::collections::HashSet;

use anyhow::{Context, Result};
use tree_sitter::Node;

use super::canonical::{CPP_CANONICAL_MAPPINGS, CPP_SUPPORTED_LANGUAGE_KINDS};
use crate::host::language_adapter::{
    CppKind, LanguageArtefact, LanguageKind, is_supported_language_kind,
    normalize_artefact_signature, resolve_canonical_kind,
};

struct CppArtefactDescriptor {
    language_kind: LanguageKind,
    name: String,
    symbol_fqn: String,
    parent_symbol_fqn: Option<String>,
}

pub(crate) fn extract_cpp_artefacts(content: &str, path: &str) -> Result<Vec<LanguageArtefact>> {
    let mut parser = tree_sitter::Parser::new();
    let lang: tree_sitter::Language = tree_sitter_cpp::LANGUAGE.into();
    parser
        .set_language(&lang)
        .context("setting tree-sitter c++ language")?;
    let Some(tree) = parser.parse(content, None) else {
        return Ok(Vec::new());
    };

    let mut out = Vec::new();
    let mut seen: HashSet<(LanguageKind, String, i32)> = HashSet::new();
    collect_cpp_nodes_recursive(tree.root_node(), content, path, None, &mut out, &mut seen);
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

fn collect_cpp_nodes_recursive(
    node: Node<'_>,
    content: &str,
    path: &str,
    current_type_fqn: Option<String>,
    out: &mut Vec<LanguageArtefact>,
    seen: &mut HashSet<(LanguageKind, String, i32)>,
) {
    let mut next_type_fqn = current_type_fqn.clone();

    match node.kind() {
        "namespace_definition" => {
            if let Some(name) = node
                .child_by_field_name("name")
                .and_then(|name_node| trimmed_node_text(name_node, content))
            {
                let symbol_fqn = format!("{path}::{name}");
                push_cpp_artefact(
                    out,
                    seen,
                    node,
                    content,
                    CppArtefactDescriptor {
                        language_kind: LanguageKind::cpp(CppKind::NamespaceDefinition),
                        name,
                        symbol_fqn: symbol_fqn.clone(),
                        parent_symbol_fqn: None,
                    },
                );
                next_type_fqn = Some(symbol_fqn);
            }
        }
        "class_specifier" | "struct_specifier" => {
            if let Some(name) = class_or_struct_name(node, content) {
                let language_kind = if node.kind() == "class_specifier" {
                    LanguageKind::cpp(CppKind::ClassSpecifier)
                } else {
                    LanguageKind::cpp(CppKind::StructSpecifier)
                };
                let symbol_fqn = current_type_fqn
                    .as_ref()
                    .map(|parent| format!("{parent}::{name}"))
                    .unwrap_or_else(|| format!("{path}::{name}"));
                push_cpp_artefact(
                    out,
                    seen,
                    node,
                    content,
                    CppArtefactDescriptor {
                        language_kind,
                        name,
                        symbol_fqn: symbol_fqn.clone(),
                        parent_symbol_fqn: current_type_fqn.clone(),
                    },
                );
                next_type_fqn = Some(symbol_fqn);
            }
        }
        "enum_specifier" => {
            if let Some(name) = node
                .child_by_field_name("name")
                .and_then(|name_node| trimmed_node_text(name_node, content))
            {
                let symbol_fqn = current_type_fqn
                    .as_ref()
                    .map(|parent| format!("{parent}::{name}"))
                    .unwrap_or_else(|| format!("{path}::{name}"));
                push_cpp_artefact(
                    out,
                    seen,
                    node,
                    content,
                    CppArtefactDescriptor {
                        language_kind: LanguageKind::cpp(CppKind::EnumSpecifier),
                        name,
                        symbol_fqn: symbol_fqn.clone(),
                        parent_symbol_fqn: current_type_fqn.clone(),
                    },
                );
                next_type_fqn = Some(symbol_fqn);
            }
        }
        "type_definition" => {
            if let Some(name) = node
                .child_by_field_name("declarator")
                .and_then(|declarator| trimmed_node_text(declarator, content))
                .or_else(|| {
                    node.child_by_field_name("type")
                        .and_then(|type_node| trimmed_node_text(type_node, content))
                })
            {
                push_cpp_artefact(
                    out,
                    seen,
                    node,
                    content,
                    CppArtefactDescriptor {
                        language_kind: LanguageKind::cpp(CppKind::TypeDefinition),
                        name: name.clone(),
                        symbol_fqn: format!("{path}::{name}"),
                        parent_symbol_fqn: None,
                    },
                );
            }
        }
        "function_definition" => {
            if let Some(name) = function_name(node, content) {
                let parent_symbol_fqn = method_parent_from_name(path, &name, &current_type_fqn);
                let symbol_fqn = if let Some(parent) = parent_symbol_fqn.as_ref() {
                    format!("{parent}::{}", short_function_name(&name))
                } else {
                    format!("{path}::{}", short_function_name(&name))
                };
                push_cpp_artefact(
                    out,
                    seen,
                    node,
                    content,
                    CppArtefactDescriptor {
                        language_kind: LanguageKind::cpp(CppKind::FunctionDefinition),
                        name: short_function_name(&name),
                        symbol_fqn,
                        parent_symbol_fqn,
                    },
                );
            }
        }
        "field_declaration" => {
            if let Some(parent) = current_type_fqn.as_ref() {
                let mut cursor = node.walk();
                for child in node.named_children(&mut cursor) {
                    if child.kind() == "field_identifier"
                        && let Some(name) = trimmed_node_text(child, content)
                    {
                        push_cpp_artefact(
                            out,
                            seen,
                            node,
                            content,
                            CppArtefactDescriptor {
                                language_kind: LanguageKind::cpp(CppKind::FieldDeclaration),
                                name: name.clone(),
                                symbol_fqn: format!("{parent}::{name}"),
                                parent_symbol_fqn: Some(parent.clone()),
                            },
                        );
                    }
                }
            }
        }
        "preproc_include" => {
            let line_no = node.start_position().row as i32 + 1;
            let name = format!("include@{line_no}");
            push_cpp_artefact(
                out,
                seen,
                node,
                content,
                CppArtefactDescriptor {
                    language_kind: LanguageKind::cpp(CppKind::PreprocInclude),
                    name: name.clone(),
                    symbol_fqn: format!("{path}::{name}"),
                    parent_symbol_fqn: None,
                },
            );
        }
        "template_declaration" => {
            let line_no = node.start_position().row as i32 + 1;
            let name = format!("template@{line_no}");
            push_cpp_artefact(
                out,
                seen,
                node,
                content,
                CppArtefactDescriptor {
                    language_kind: LanguageKind::cpp(CppKind::TemplateDeclaration),
                    name: name.clone(),
                    symbol_fqn: format!("{path}::{name}"),
                    parent_symbol_fqn: current_type_fqn.clone(),
                },
            );
        }
        _ => {}
    }

    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        collect_cpp_nodes_recursive(child, content, path, next_type_fqn.clone(), out, seen);
    }
}

fn push_cpp_artefact(
    out: &mut Vec<LanguageArtefact>,
    seen: &mut HashSet<(LanguageKind, String, i32)>,
    node: Node<'_>,
    content: &str,
    descriptor: CppArtefactDescriptor,
) {
    let CppArtefactDescriptor {
        language_kind,
        name,
        symbol_fqn,
        parent_symbol_fqn,
    } = descriptor;

    if !is_supported_language_kind(CPP_SUPPORTED_LANGUAGE_KINDS, language_kind) {
        return;
    }

    let start_line = node.start_position().row as i32 + 1;
    if !seen.insert((language_kind, symbol_fqn.clone(), start_line)) {
        return;
    }

    let signature = node
        .utf8_text(content.as_bytes())
        .ok()
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(normalize_artefact_signature)
        .unwrap_or_default();

    let canonical_kind = resolve_canonical_kind(
        CPP_CANONICAL_MAPPINGS,
        language_kind,
        parent_symbol_fqn.is_some(),
    )
    .map(|projection| projection.as_str().to_string());

    out.push(LanguageArtefact {
        canonical_kind,
        language_kind,
        name,
        symbol_fqn,
        parent_symbol_fqn,
        start_line,
        end_line: node.end_position().row as i32 + 1,
        start_byte: node.start_byte() as i32,
        end_byte: node.end_byte() as i32,
        signature,
        modifiers: Vec::new(),
        docstring: None,
    });
}

fn trimmed_node_text(node: Node<'_>, content: &str) -> Option<String> {
    node.utf8_text(content.as_bytes())
        .ok()
        .map(str::trim)
        .map(str::to_string)
        .filter(|text| !text.is_empty())
}

fn class_or_struct_name(node: Node<'_>, content: &str) -> Option<String> {
    node.child_by_field_name("name")
        .and_then(|name_node| trimmed_node_text(name_node, content))
        .or_else(|| {
            let mut cursor = node.walk();
            node.named_children(&mut cursor)
                .find(|child| child.kind() == "type_identifier")
                .and_then(|id| trimmed_node_text(id, content))
        })
}

fn function_name(node: Node<'_>, content: &str) -> Option<String> {
    let declarator = node.child_by_field_name("declarator")?;
    function_name_from_declarator(declarator, content)
}

fn function_name_from_declarator(node: Node<'_>, content: &str) -> Option<String> {
    if matches!(node.kind(), "identifier" | "field_identifier" | "qualified_identifier") {
        return trimmed_node_text(node, content);
    }

    if node.kind() == "function_declarator" {
        if let Some(child) = node.child_by_field_name("declarator") {
            return function_name_from_declarator(child, content);
        }
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            if let Some(name) = function_name_from_declarator(child, content) {
                return Some(name);
            }
        }
    }

    if node.kind() == "qualified_identifier" || node.kind() == "scoped_identifier" {
        return trimmed_node_text(node, content);
    }

    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        if let Some(name) = function_name_from_declarator(child, content) {
            return Some(name);
        }
    }
    None
}

fn method_parent_from_name(
    path: &str,
    qualified_name: &str,
    current_type_fqn: &Option<String>,
) -> Option<String> {
    if let Some((parent, _)) = qualified_name.rsplit_once("::") {
        return Some(format!("{path}::{parent}"));
    }
    current_type_fqn.clone()
}

fn short_function_name(name: &str) -> String {
    name.rsplit("::").next().unwrap_or(name).to_string()
}

#[cfg(test)]
mod tests {
    use super::extract_cpp_artefacts;
    use crate::host::language_adapter::{CppKind, LanguageKind};

    #[test]
    fn extract_cpp_artefacts_collects_basic_symbols() {
        let content = r#"
#include <vector>
namespace app {
class UserService {
public:
    int run(int x) { return helper(x); }
    int helper(int x) { return x + 1; }
};
}

int main() { return 0; }
"#;
        let artefacts = extract_cpp_artefacts(content, "src/main.cpp").expect("extracts cpp");

        assert!(artefacts.iter().any(|artefact| {
            artefact.language_kind == LanguageKind::cpp(CppKind::NamespaceDefinition)
                && artefact.name == "app"
        }));
        assert!(artefacts.iter().any(|artefact| {
            artefact.language_kind == LanguageKind::cpp(CppKind::ClassSpecifier)
                && artefact.name == "UserService"
        }));
        assert!(artefacts.iter().any(|artefact| {
            artefact.language_kind == LanguageKind::cpp(CppKind::FunctionDefinition)
                && artefact.name == "main"
        }));
    }
}
