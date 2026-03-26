use std::collections::HashSet;
use std::path::Path;

use tree_sitter::Node;

use crate::capability_packs::test_harness::mapping::file_discovery::normalize_rel_path;

pub(crate) fn collect_rust_import_paths_for(
    root: Node<'_>,
    source: &[u8],
    relative_path: &str,
) -> HashSet<String> {
    let mut paths = HashSet::new();
    let mut stack = vec![root];

    while let Some(node) = stack.pop() {
        if node.kind() == "use_declaration"
            && let Ok(raw_use) = node.utf8_text(source)
        {
            for use_expr in expand_rust_use_statement(raw_use) {
                for path in rust_use_path_to_source_paths(&use_expr, relative_path) {
                    paths.insert(path);
                }
            }
        }

        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            stack.push(child);
        }
    }

    paths
}

pub(crate) fn rust_test_context_source_paths(relative_path: &str) -> HashSet<String> {
    let mut paths = HashSet::new();
    if !looks_like_production_source_path(relative_path) || !relative_path.ends_with(".rs") {
        return paths;
    }

    paths.insert(relative_path.to_string());

    let path = Path::new(relative_path);
    let Some(file_stem) = path.file_stem().and_then(|value| value.to_str()) else {
        return paths;
    };
    if !file_stem.contains("test") || matches!(file_stem, "lib" | "main" | "mod") {
        return paths;
    }

    let Some(parent) = path.parent() else {
        return paths;
    };
    let parent_path = normalize_rel_path(parent);
    if parent_path == "src" || parent_path.ends_with("/src") {
        return paths;
    }

    paths.insert(format!("{parent_path}.rs"));
    paths.insert(format!("{parent_path}/mod.rs"));
    paths
}

pub(crate) fn collect_rust_scoped_call_import_paths_for(
    root: Node<'_>,
    source: &[u8],
    relative_path: &str,
) -> HashSet<String> {
    let mut paths = HashSet::new();
    let mut stack = vec![root];

    while let Some(node) = stack.pop() {
        if node.kind() == "call_expression"
            && let Some(function_node) = node.child_by_field_name("function")
            && let Ok(raw_call) = function_node.utf8_text(source)
            && raw_call.contains("::")
        {
            for path in rust_use_path_to_source_paths(raw_call, relative_path) {
                paths.insert(path);
            }
        }

        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            stack.push(child);
        }
    }

    paths
}

pub(crate) fn expand_rust_use_statement(raw_use_statement: &str) -> Vec<String> {
    let mut statement = raw_use_statement.trim();

    if let Some(stripped) = statement.strip_prefix("pub ") {
        statement = stripped.trim_start();
    }
    if let Some(stripped) = statement.strip_prefix("use ") {
        statement = stripped;
    }
    statement = statement.trim().trim_end_matches(';').trim();

    expand_rust_use_expression(statement)
}

fn expand_rust_use_expression(expression: &str) -> Vec<String> {
    let expression = expression.trim();
    if expression.is_empty() {
        return Vec::new();
    }

    if let Some(open_idx) = find_top_level_char(expression, '{')
        && let Some(close_idx) = find_matching_brace(expression, open_idx)
    {
        let prefix = expression[..open_idx].trim().trim_end_matches("::");
        let inside = &expression[open_idx + 1..close_idx];
        let suffix = expression[close_idx + 1..].trim();
        let suffix = suffix.trim_start_matches("::");

        let mut expanded = Vec::new();
        for part in split_top_level_commas(inside) {
            for nested in expand_rust_use_expression(part) {
                let base = if nested == "self" {
                    prefix.to_string()
                } else if prefix.is_empty() {
                    nested
                } else if nested.is_empty() {
                    prefix.to_string()
                } else {
                    format!("{prefix}::{nested}")
                };

                if suffix.is_empty() {
                    if !base.is_empty() {
                        expanded.push(base);
                    }
                } else if !base.is_empty() {
                    expanded.push(format!("{base}::{suffix}"));
                }
            }
        }

        return expanded;
    }

    vec![expression.to_string()]
}

fn find_top_level_char(value: &str, target: char) -> Option<usize> {
    let mut brace_depth = 0i32;
    for (idx, ch) in value.char_indices() {
        match ch {
            '{' => {
                if ch == target && brace_depth == 0 {
                    return Some(idx);
                }
                brace_depth += 1;
            }
            '}' => {
                brace_depth -= 1;
            }
            _ if ch == target && brace_depth == 0 => return Some(idx),
            _ => {}
        }
    }
    None
}

fn find_matching_brace(value: &str, open_idx: usize) -> Option<usize> {
    let mut depth = 0i32;
    for (idx, ch) in value.char_indices().skip_while(|(idx, _)| *idx < open_idx) {
        match ch {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(idx);
                }
            }
            _ => {}
        }
    }
    None
}

fn split_top_level_commas(value: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut start = 0usize;
    let mut depth = 0i32;

    for (idx, ch) in value.char_indices() {
        match ch {
            '{' => depth += 1,
            '}' => depth -= 1,
            ',' if depth == 0 => {
                let part = value[start..idx].trim();
                if !part.is_empty() {
                    parts.push(part);
                }
                start = idx + 1;
            }
            _ => {}
        }
    }

    let tail = value[start..].trim();
    if !tail.is_empty() {
        parts.push(tail);
    }

    parts
}

fn rust_use_path_to_source_paths(raw_path: &str, test_relative_path: &str) -> HashSet<String> {
    let mut paths = HashSet::new();

    let path = raw_path
        .split(" as ")
        .next()
        .unwrap_or(raw_path)
        .trim()
        .trim_end_matches(';')
        .trim();
    if path.is_empty() {
        return paths;
    }

    let mut sanitized = path.trim_end_matches("::*").trim().to_string();
    if sanitized.ends_with("::self") {
        sanitized = sanitized.trim_end_matches("::self").to_string();
    }

    let segments: Vec<&str> = sanitized
        .split("::")
        .filter(|seg| !seg.is_empty())
        .collect();
    if segments.is_empty() {
        return paths;
    }

    add_rust_source_candidates(&mut paths, None, normalized_rust_use_segments(&segments));

    let current_crate_root = workspace_crate_root(test_relative_path);
    let current_crate_name = current_crate_root
        .as_deref()
        .and_then(|root| root.rsplit('/').next())
        .map(str::to_string);

    if let Some(crate_root) = current_crate_root.as_deref() {
        add_rust_source_candidates(
            &mut paths,
            Some(crate_root),
            normalized_rust_use_segments(&segments),
        );
    }

    if segments[0] != "crate"
        && segments[0] != "self"
        && segments[0] != "super"
        && segments.len() > 1
    {
        add_rust_source_candidates(
            &mut paths,
            None,
            normalized_rust_use_segments(&segments[1..]),
        );
    }

    if let Some(crate_root) = current_crate_root.as_deref()
        && current_crate_name.as_deref() == Some(segments[0])
        && segments.len() > 1
    {
        add_rust_source_candidates(&mut paths, Some(crate_root), &segments[1..]);
    }

    if segments[0] != "crate"
        && segments[0] != "self"
        && segments[0] != "super"
        && segments.len() > 1
    {
        let crate_root = format!("crates/{}", segments[0]);
        add_rust_source_candidates(&mut paths, Some(&crate_root), &segments[1..]);
    }

    paths
}

fn normalized_rust_use_segments<'a>(segments: &'a [&'a str]) -> &'a [&'a str] {
    if segments.is_empty() {
        return segments;
    }
    if segments[0] == "crate" || segments[0] == "self" || segments[0] == "super" {
        &segments[1..]
    } else {
        segments
    }
}

fn add_rust_source_candidates(
    paths: &mut HashSet<String>,
    prefix: Option<&str>,
    segments: &[&str],
) {
    if segments.is_empty() {
        return;
    }

    for end in 1..=segments.len() {
        let module = segments[..end].join("/");
        let file_path = format!("src/{module}.rs");
        let mod_path = format!("src/{module}/mod.rs");
        if let Some(prefix) = prefix {
            paths.insert(format!("{prefix}/{file_path}"));
            paths.insert(format!("{prefix}/{mod_path}"));
        } else {
            paths.insert(file_path);
            paths.insert(mod_path);
        }
    }
}

fn workspace_crate_root(relative_path: &str) -> Option<String> {
    let mut segments = relative_path.split('/');
    let first = segments.next()?;
    let second = segments.next()?;
    let third = segments.next()?;

    (first == "crates" && (third == "src" || third == "tests")).then(|| format!("{first}/{second}"))
}

fn looks_like_production_source_path(path: &str) -> bool {
    path.starts_with("src/") || path.contains("/src/")
}
