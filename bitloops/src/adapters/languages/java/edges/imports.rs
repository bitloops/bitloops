use std::collections::HashMap;

use tree_sitter::Node;

use super::JavaTraversalCtx;
use crate::host::devql::{EdgeKind, ImportForm};
use crate::host::language_adapter::{DependencyEdge, EdgeMetadata};

pub(super) fn collect_java_import_edge(node: Node<'_>, traversal: &mut JavaTraversalCtx<'_>) {
    let Some(import_ref) = parse_import_declaration(node, traversal.content).map(|value| value.0)
    else {
        return;
    };
    let line_no = node.start_position().row as i32 + 1;
    let key = format!("{}|{}|{}", traversal.path, import_ref, line_no);
    if !traversal.seen_imports.insert(key) {
        return;
    }
    traversal.out.push(DependencyEdge {
        edge_kind: EdgeKind::Imports,
        from_symbol_fqn: traversal.path.to_string(),
        to_target_symbol_fqn: None,
        to_symbol_ref: Some(import_ref),
        start_line: Some(line_no),
        end_line: Some(node.end_position().row as i32 + 1),
        metadata: EdgeMetadata::import(ImportForm::Binding),
    });
}

pub(super) fn collect_java_import_data(
    root: Node<'_>,
    content: &str,
    path: &str,
) -> (
    Vec<DependencyEdge>,
    HashMap<String, String>,
    HashMap<String, String>,
) {
    let mut import_edges = Vec::new();
    let mut imported_type_refs = HashMap::new();
    let mut imported_static_refs = HashMap::new();
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        if node.kind() == "import_declaration"
            && let Some((import_ref, is_static)) = parse_import_declaration(node, content)
        {
            let line_no = node.start_position().row as i32 + 1;
            import_edges.push(DependencyEdge {
                edge_kind: EdgeKind::Imports,
                from_symbol_fqn: path.to_string(),
                to_target_symbol_fqn: None,
                to_symbol_ref: Some(import_ref.clone()),
                start_line: Some(line_no),
                end_line: Some(node.end_position().row as i32 + 1),
                metadata: EdgeMetadata::import(ImportForm::Binding),
            });

            if !import_ref.ends_with(".*") {
                let binding_name = import_ref
                    .rsplit('.')
                    .next()
                    .unwrap_or(import_ref.as_str())
                    .to_string();
                if is_static {
                    imported_static_refs.insert(binding_name, import_ref);
                } else {
                    imported_type_refs.insert(binding_name, import_ref);
                }
            }
        }
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            stack.push(child);
        }
    }
    (import_edges, imported_type_refs, imported_static_refs)
}

pub(super) fn parse_import_declaration(node: Node<'_>, content: &str) -> Option<(String, bool)> {
    let raw = node.utf8_text(content.as_bytes()).ok()?.trim();
    let trimmed = raw.trim_end_matches(';').trim();
    let stripped = trimmed.strip_prefix("import")?.trim();
    let is_static = stripped.starts_with("static ");
    let import_ref = if is_static {
        stripped.trim_start_matches("static ").trim()
    } else {
        stripped
    };
    Some((import_ref.to_string(), is_static))
}
