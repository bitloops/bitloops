use std::collections::HashSet;

use anyhow::{Context, Result};
use regex::Regex;

use super::canonical::{RUST_CANONICAL_MAPPINGS, RUST_SUPPORTED_LANGUAGE_KINDS};
use crate::host::language_adapter::{
    LanguageArtefact, is_supported_language_kind, resolve_canonical_kind,
};

// Rust artefact extraction via tree-sitter.

pub(crate) fn extract_rust_artefacts(content: &str, path: &str) -> Result<Vec<LanguageArtefact>> {
    let mut parser = tree_sitter::Parser::new();
    let lang: tree_sitter::Language = tree_sitter_rust::LANGUAGE.into();
    parser
        .set_language(&lang)
        .context("setting tree-sitter rust language")?;
    let Some(tree) = parser.parse(content, None) else {
        return Ok(Vec::new());
    };

    let root = tree.root_node();
    let mut out = Vec::new();
    let mut seen: HashSet<(String, String, i32)> = HashSet::new();
    collect_rust_nodes_recursive(root, content, path, &mut out, &mut seen, None);
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

pub(crate) fn extract_rust_file_docstring(content: &str) -> Option<String> {
    extract_rust_inner_docstring_from_lines(content.lines().collect::<Vec<_>>().as_slice(), 0)
}

pub(crate) fn collect_rust_nodes_recursive(
    node: tree_sitter::Node,
    content: &str,
    path: &str,
    out: &mut Vec<LanguageArtefact>,
    seen: &mut HashSet<(String, String, i32)>,
    current_impl_fqn: Option<String>,
) {
    let kind = node.kind();
    let start_line = node.start_position().row as i32 + 1;
    let end_line = node.end_position().row as i32 + 1;
    let start_byte = node.start_byte() as i32;
    let end_byte = node.end_byte() as i32;
    let signature = node
        .utf8_text(content.as_bytes())
        .ok()
        .and_then(|s| s.lines().next())
        .unwrap_or("")
        .trim()
        .to_string();

    let push = |out: &mut Vec<LanguageArtefact>,
                seen: &mut HashSet<(String, String, i32)>,
                language_kind: &str,
                name: String,
                symbol_fqn: String,
                parent_symbol_fqn: Option<String>,
                inside_impl: bool,
                docstring: Option<String>| {
        if name.is_empty() {
            return;
        }
        if !is_supported_language_kind(RUST_SUPPORTED_LANGUAGE_KINDS, language_kind) {
            return;
        }
        if !seen.insert((language_kind.to_string(), name.clone(), start_line)) {
            return;
        }
        out.push(LanguageArtefact {
            canonical_kind: resolve_canonical_kind(
                RUST_CANONICAL_MAPPINGS,
                language_kind,
                inside_impl,
            )
            .map(|p| p.as_str().to_string()),
            language_kind: language_kind.to_string(),
            name,
            symbol_fqn,
            parent_symbol_fqn,
            start_line,
            end_line,
            start_byte,
            end_byte,
            signature: signature.clone(),
            modifiers: extract_rust_modifiers(node, content),
            docstring,
        });
    };

    let mut next_impl_fqn = current_impl_fqn.clone();

    match kind {
        "mod_item" => {
            if let Some(name_node) = node.child_by_field_name("name")
                && let Ok(name) = name_node.utf8_text(content.as_bytes())
            {
                push(
                    out,
                    seen,
                    "mod_item",
                    name.to_string(),
                    format!("{path}::{name}"),
                    None,
                    false,
                    extract_rust_docstring(node, content),
                );
            }
        }
        "struct_item" => {
            if let Some(name_node) = node.child_by_field_name("name")
                && let Ok(name) = name_node.utf8_text(content.as_bytes())
            {
                push(
                    out,
                    seen,
                    "struct_item",
                    name.to_string(),
                    format!("{path}::{name}"),
                    None,
                    false,
                    extract_rust_docstring(node, content),
                );
            }
        }
        "enum_item" => {
            if let Some(name_node) = node.child_by_field_name("name")
                && let Ok(name) = name_node.utf8_text(content.as_bytes())
            {
                push(
                    out,
                    seen,
                    "enum_item",
                    name.to_string(),
                    format!("{path}::{name}"),
                    None,
                    false,
                    extract_rust_docstring(node, content),
                );
            }
        }
        "trait_item" => {
            if let Some(name_node) = node.child_by_field_name("name")
                && let Ok(name) = name_node.utf8_text(content.as_bytes())
            {
                push(
                    out,
                    seen,
                    "trait_item",
                    name.to_string(),
                    format!("{path}::{name}"),
                    None,
                    false,
                    extract_rust_docstring(node, content),
                );
            }
        }
        "type_item" => {
            if let Some(name_node) = node.child_by_field_name("name")
                && let Ok(name) = name_node.utf8_text(content.as_bytes())
            {
                push(
                    out,
                    seen,
                    "type_item",
                    name.to_string(),
                    format!("{path}::{name}"),
                    None,
                    false,
                    extract_rust_docstring(node, content),
                );
            }
        }
        "const_item" => {
            if let Some(name_node) = node.child_by_field_name("name")
                && let Ok(name) = name_node.utf8_text(content.as_bytes())
            {
                push(
                    out,
                    seen,
                    "const_item",
                    name.to_string(),
                    format!("{path}::{name}"),
                    None,
                    false,
                    extract_rust_docstring(node, content),
                );
            }
        }
        "static_item" => {
            if let Some(name_node) = node.child_by_field_name("name")
                && let Ok(name) = name_node.utf8_text(content.as_bytes())
            {
                push(
                    out,
                    seen,
                    "static_item",
                    name.to_string(),
                    format!("{path}::{name}"),
                    None,
                    false,
                    extract_rust_docstring(node, content),
                );
            }
        }
        "use_declaration" => {
            let name = format!("use@{start_line}");
            push(
                out,
                seen,
                "use_declaration",
                name.clone(),
                format!("{path}::{name}"),
                None,
                false,
                extract_rust_docstring(node, content),
            );
        }
        "impl_item" => {
            let name = format!("impl@{start_line}");
            let impl_fqn = format!("{path}::{name}");
            push(
                out,
                seen,
                "impl_item",
                name.clone(),
                impl_fqn.clone(),
                None,
                false,
                extract_rust_docstring(node, content),
            );
            next_impl_fqn = Some(impl_fqn);
        }
        "function_item" => {
            if let Some(name_node) = node.child_by_field_name("name")
                && let Ok(name) = name_node.utf8_text(content.as_bytes())
            {
                if let Some(impl_fqn) = current_impl_fqn.clone() {
                    push(
                        out,
                        seen,
                        "function_item",
                        name.to_string(),
                        format!("{impl_fqn}::{name}"),
                        Some(impl_fqn),
                        true,
                        extract_rust_docstring(node, content),
                    );
                } else {
                    push(
                        out,
                        seen,
                        "function_item",
                        name.to_string(),
                        format!("{path}::{name}"),
                        None,
                        false,
                        extract_rust_docstring(node, content),
                    );
                }
            }
        }
        _ => {}
    }

    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        collect_rust_nodes_recursive(child, content, path, out, seen, next_impl_fqn.clone());
    }
}

pub(crate) fn extract_rust_modifiers(node: tree_sitter::Node, content: &str) -> Vec<String> {
    let prefix_end = node
        .child_by_field_name("name")
        .map(|child| child.start_byte())
        .unwrap_or_else(|| node.end_byte());
    let prefix = content
        .get(node.start_byte()..prefix_end)
        .unwrap_or_default()
        .trim();

    let patterns = [
        (r"pub(?:\([^)]*\))?", ""),
        (r"\basync\b", "async"),
        (r"\bunsafe\b", "unsafe"),
        (r"\bconst\b", "const"),
        (r"\bdefault\b", "default"),
        (r#"\bextern\b(?:\s*"[^"]*")?"#, "extern"),
        (r"\bstatic\b", "static"),
    ];

    let mut matches = Vec::new();
    for (pattern, normalized) in patterns {
        let regex = Regex::new(pattern).expect("rust modifier regex should compile");
        for found in regex.find_iter(prefix) {
            let value = if normalized.is_empty() {
                found.as_str().trim().to_ascii_lowercase()
            } else {
                normalized.to_string()
            };
            matches.push((found.start(), value));
        }
    }

    matches.sort_by_key(|(start, _)| *start);

    let mut modifiers = Vec::new();
    for (_, modifier) in matches {
        if !modifiers.iter().any(|existing| existing == &modifier) {
            modifiers.push(modifier);
        }
    }
    modifiers
}

pub(crate) fn extract_rust_docstring(node: tree_sitter::Node, content: &str) -> Option<String> {
    let outer = extract_rust_outer_docstring(node, content);
    if node.kind() != "mod_item" {
        return outer;
    }

    let inner = node.child_by_field_name("body").and_then(|body| {
        extract_rust_inner_docstring_from_lines(
            &content.lines().collect::<Vec<_>>(),
            body.start_position().row + 1,
        )
    });

    combine_docstrings(outer, inner)
}

pub(crate) fn extract_rust_outer_docstring(
    node: tree_sitter::Node,
    content: &str,
) -> Option<String> {
    let lines: Vec<&str> = content.lines().collect();
    if lines.is_empty() {
        return None;
    }

    let mut blocks = Vec::new();
    let mut line_idx = node.start_position().row as i32 - 1;
    while line_idx >= 0 {
        let trimmed = lines[line_idx as usize].trim();
        if trimmed.is_empty() {
            break;
        }

        if trimmed.starts_with("///") {
            let mut start = line_idx;
            while start > 0 && lines[(start - 1) as usize].trim().starts_with("///") {
                start -= 1;
            }
            blocks.push(normalize_rust_line_doc_block(
                &lines[start as usize..=line_idx as usize],
                "///",
            ));
            line_idx = start - 1;
            continue;
        }

        if trimmed.contains("*/") || trimmed.starts_with('*') {
            let mut start = line_idx;
            while start >= 0 && !lines[start as usize].contains("/**") {
                start -= 1;
            }
            if start < 0 {
                break;
            }
            blocks.push(normalize_rust_block_doc_block(
                &lines[start as usize..=line_idx as usize],
                "/**",
            ));
            line_idx = start - 1;
            continue;
        }

        break;
    }

    if blocks.is_empty() {
        None
    } else {
        blocks.reverse();
        Some(blocks.join("\n\n"))
    }
}

pub(crate) fn extract_rust_inner_docstring_from_lines(
    lines: &[&str],
    start_line_idx: usize,
) -> Option<String> {
    let mut idx = start_line_idx;
    while idx < lines.len() && lines[idx].trim().is_empty() {
        idx += 1;
    }
    if idx >= lines.len() {
        return None;
    }

    let mut blocks = Vec::new();
    while idx < lines.len() {
        let trimmed = lines[idx].trim();
        if trimmed.starts_with("//!") {
            let start = idx;
            idx += 1;
            while idx < lines.len() && lines[idx].trim().starts_with("//!") {
                idx += 1;
            }
            blocks.push(normalize_rust_line_doc_block(&lines[start..idx], "//!"));
            continue;
        }

        if trimmed.starts_with("/*!") {
            let start = idx;
            idx += 1;
            while idx < lines.len() && !lines[idx - 1].contains("*/") {
                idx += 1;
            }
            blocks.push(normalize_rust_block_doc_block(&lines[start..idx], "/*!"));
            continue;
        }

        break;
    }

    if blocks.is_empty() {
        None
    } else {
        Some(blocks.join("\n\n"))
    }
}

pub(crate) fn normalize_rust_line_doc_block(lines: &[&str], prefix: &str) -> String {
    lines
        .iter()
        .map(|line| line.trim().trim_start_matches(prefix).trim())
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_string()
}

pub(crate) fn normalize_rust_block_doc_block(lines: &[&str], prefix: &str) -> String {
    let mut normalized = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        let mut text = line.trim().to_string();
        if index == 0 {
            text = text.trim_start_matches(prefix).trim().to_string();
        }
        if index + 1 == lines.len() {
            text = text
                .trim_end_matches('/')
                .trim_end_matches('*')
                .trim()
                .to_string();
        }
        text = text.trim_start_matches('*').trim().to_string();
        normalized.push(text);
    }
    normalized.join("\n").trim().to_string()
}

pub(crate) fn combine_docstrings(first: Option<String>, second: Option<String>) -> Option<String> {
    match (first, second) {
        (Some(first), Some(second)) if !first.is_empty() && !second.is_empty() => {
            Some(format!("{first}\n\n{second}"))
        }
        (Some(first), _) if !first.is_empty() => Some(first),
        (_, Some(second)) if !second.is_empty() => Some(second),
        _ => None,
    }
}
