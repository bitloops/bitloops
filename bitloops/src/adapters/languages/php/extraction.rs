use std::collections::HashSet;

use anyhow::{Context, Result};

use super::canonical::{PHP_CANONICAL_MAPPINGS, PHP_SUPPORTED_LANGUAGE_KINDS};
use crate::host::language_adapter::{
    LanguageArtefact, LanguageKind, PhpKind, is_supported_language_kind,
    normalize_artefact_signature, resolve_canonical_kind,
};

pub(crate) fn extract_php_artefacts(content: &str, path: &str) -> Result<Vec<LanguageArtefact>> {
    let mut parser = tree_sitter::Parser::new();
    let lang: tree_sitter::Language = tree_sitter_php::LANGUAGE_PHP.into();
    parser
        .set_language(&lang)
        .context("setting tree-sitter php language")?;
    let Some(tree) = parser.parse(content, None) else {
        return Ok(Vec::new());
    };

    let mut out = Vec::new();
    let mut seen: HashSet<(LanguageKind, String, i32)> = HashSet::new();
    collect_php_nodes_recursive(tree.root_node(), content, path, &mut out, &mut seen, None);
    out.sort_by_key(|i| {
        (
            i.start_line,
            i.end_line,
            i.canonical_kind.clone(),
            i.name.clone(),
        )
    });
    Ok(out)
}

pub(crate) fn extract_php_file_docstring(content: &str) -> Option<String> {
    let trimmed = content.trim_start();
    if !trimmed.starts_with("/**") {
        return None;
    }
    let end = trimmed.find("*/")?;
    Some(trimmed[..end + 2].trim().to_string())
}

fn collect_php_nodes_recursive(
    node: tree_sitter::Node,
    content: &str,
    path: &str,
    out: &mut Vec<LanguageArtefact>,
    seen: &mut HashSet<(LanguageKind, String, i32)>,
    current_type_fqn: Option<&str>,
) {
    match node.kind() {
        "namespace_definition" => {
            if let Some(name_node) = node.child_by_field_name("name")
                && let Ok(name) = name_node.utf8_text(content.as_bytes())
            {
                push_php_artefact(
                    out,
                    seen,
                    node,
                    content,
                    PhpArtefactDescriptor {
                        language_kind: LanguageKind::php(PhpKind::NamespaceDefinition),
                        name: name.trim().to_string(),
                        symbol_fqn: format!("{path}::{}", name.trim()),
                        parent_symbol_fqn: None,
                    },
                );
            }
        }
        "namespace_use_declaration" => {
            let line = node.start_position().row as i32 + 1;
            let name = format!("import@{line}");
            push_php_artefact(
                out,
                seen,
                node,
                content,
                PhpArtefactDescriptor {
                    language_kind: LanguageKind::php(PhpKind::NamespaceUseDeclaration),
                    name: name.clone(),
                    symbol_fqn: format!("{path}::import::{name}"),
                    parent_symbol_fqn: None,
                },
            );
        }
        "class_declaration"
        | "interface_declaration"
        | "trait_declaration"
        | "enum_declaration" => {
            if let Some(name_node) = node.child_by_field_name("name")
                && let Ok(name) = name_node.utf8_text(content.as_bytes())
            {
                let kind = match node.kind() {
                    "class_declaration" => PhpKind::ClassDeclaration,
                    "interface_declaration" => PhpKind::InterfaceDeclaration,
                    "trait_declaration" => PhpKind::TraitDeclaration,
                    _ => PhpKind::EnumDeclaration,
                };
                let type_fqn = format!("{path}::{}", name.trim());
                push_php_artefact(
                    out,
                    seen,
                    node,
                    content,
                    PhpArtefactDescriptor {
                        language_kind: LanguageKind::php(kind),
                        name: name.trim().to_string(),
                        symbol_fqn: type_fqn.clone(),
                        parent_symbol_fqn: None,
                    },
                );
                let mut cursor = node.walk();
                for child in node.named_children(&mut cursor) {
                    collect_php_nodes_recursive(
                        child,
                        content,
                        path,
                        out,
                        seen,
                        Some(type_fqn.as_str()),
                    );
                }
                return;
            }
        }
        "function_definition" => {
            if let Some(name_node) = node.child_by_field_name("name")
                && let Ok(name) = name_node.utf8_text(content.as_bytes())
            {
                let symbol_fqn = format!("{path}::{}", name.trim());
                push_php_artefact(
                    out,
                    seen,
                    node,
                    content,
                    PhpArtefactDescriptor {
                        language_kind: LanguageKind::php(PhpKind::FunctionDefinition),
                        name: name.trim().to_string(),
                        symbol_fqn,
                        parent_symbol_fqn: None,
                    },
                );
            }
        }
        "method_declaration" => {
            if let Some(name_node) = node.child_by_field_name("name")
                && let Ok(name) = name_node.utf8_text(content.as_bytes())
                && let Some(parent) = current_type_fqn
            {
                push_php_artefact(
                    out,
                    seen,
                    node,
                    content,
                    PhpArtefactDescriptor {
                        language_kind: LanguageKind::php(PhpKind::MethodDeclaration),
                        name: name.trim().to_string(),
                        symbol_fqn: format!("{parent}::{}", name.trim()),
                        parent_symbol_fqn: Some(parent.to_string()),
                    },
                );
            }
        }
        "property_declaration" => {
            if let Some(parent) = current_type_fqn {
                let mut cursor = node.walk();
                for child in node.named_children(&mut cursor) {
                    if child.kind() != "property_element" {
                        continue;
                    }
                    if let Some(name_node) = child.child_by_field_name("name")
                        && let Ok(name) = name_node.utf8_text(content.as_bytes())
                    {
                        let clean = name.trim();
                        if clean.is_empty() {
                            continue;
                        }
                        push_php_artefact(
                            out,
                            seen,
                            child,
                            content,
                            PhpArtefactDescriptor {
                                language_kind: LanguageKind::php(PhpKind::PropertyDeclaration),
                                name: clean.to_string(),
                                symbol_fqn: format!("{parent}::{clean}"),
                                parent_symbol_fqn: Some(parent.to_string()),
                            },
                        );
                    }
                }
            }
        }
        "const_declaration" => {
            let mut cursor = node.walk();
            for child in node.named_children(&mut cursor) {
                if child.kind() != "const_element" {
                    continue;
                }
                if let Some(name_node) = child.child_by_field_name("name")
                    && let Ok(name) = name_node.utf8_text(content.as_bytes())
                {
                    let clean = name.trim();
                    if clean.is_empty() {
                        continue;
                    }
                    let parent = current_type_fqn.map(ToString::to_string);
                    let symbol_fqn = parent
                        .as_ref()
                        .map(|p| format!("{p}::{clean}"))
                        .unwrap_or_else(|| format!("{path}::{clean}"));
                    push_php_artefact(
                        out,
                        seen,
                        child,
                        content,
                        PhpArtefactDescriptor {
                            language_kind: LanguageKind::php(PhpKind::ConstDeclaration),
                            name: clean.to_string(),
                            symbol_fqn,
                            parent_symbol_fqn: parent,
                        },
                    );
                }
            }
        }
        _ => {}
    }

    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        collect_php_nodes_recursive(child, content, path, out, seen, current_type_fqn);
    }
}

struct PhpArtefactDescriptor {
    language_kind: LanguageKind,
    name: String,
    symbol_fqn: String,
    parent_symbol_fqn: Option<String>,
}

fn push_php_artefact(
    out: &mut Vec<LanguageArtefact>,
    seen: &mut HashSet<(LanguageKind, String, i32)>,
    node: tree_sitter::Node,
    content: &str,
    descriptor: PhpArtefactDescriptor,
) {
    if descriptor.name.is_empty()
        || !is_supported_language_kind(PHP_SUPPORTED_LANGUAGE_KINDS, descriptor.language_kind)
    {
        return;
    }

    let start_line = node.start_position().row as i32 + 1;
    if !seen.insert((
        descriptor.language_kind,
        descriptor.name.clone(),
        start_line,
    )) {
        return;
    }

    let signature = normalize_artefact_signature(
        node.utf8_text(content.as_bytes())
            .ok()
            .and_then(|text| text.lines().next())
            .unwrap_or(""),
    );

    out.push(LanguageArtefact {
        canonical_kind: resolve_canonical_kind(
            PHP_CANONICAL_MAPPINGS,
            descriptor.language_kind,
            false,
        )
        .map(|projection| projection.as_str().to_string()),
        language_kind: descriptor.language_kind,
        name: descriptor.name,
        symbol_fqn: descriptor.symbol_fqn,
        parent_symbol_fqn: descriptor.parent_symbol_fqn,
        start_line,
        end_line: node.end_position().row as i32 + 1,
        start_byte: node.start_byte() as i32,
        end_byte: node.end_byte() as i32,
        signature,
        modifiers: Vec::new(),
        docstring: None,
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_php_artefacts_captures_core_symbols() {
        let content = r#"<?php
namespace App\Services;
use App\Core\Helper;

class UserService {
    public function run() {
        return helper();
    }

    private string $name;
}

const VERSION = "1.0";

function helper() {
    return 1;
}
"#;
        let artefacts =
            extract_php_artefacts(content, "src/UserService.php").expect("extract php artefacts");
        assert!(artefacts.iter().any(|a| a.name == "App\\Services"));
        assert!(artefacts.iter().any(|a| a.name == "UserService"));
        assert!(artefacts.iter().any(|a| a.name == "run"));
        assert!(artefacts.iter().any(|a| a.name == "$name"));
        assert!(artefacts.iter().any(|a| a.name == "helper"));
    }

    #[test]
    fn extract_php_file_docstring_reads_leading_phpdoc() {
        let content = "/** file docs */\n<?php\nfunction x() {}\n";
        let doc = extract_php_file_docstring(content).expect("phpdoc");
        assert!(doc.contains("file docs"));
    }
}
