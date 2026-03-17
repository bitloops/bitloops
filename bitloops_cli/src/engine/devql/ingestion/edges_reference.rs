// Reference edge extraction for JS/TS and Rust.

fn collect_js_ts_reference_edges_recursive(
    node: tree_sitter::Node,
    content: &str,
    ctx: &ReferenceCtx,
    col: &mut EdgeCollector,
) {
    let line_no = node.start_position().row as i32 + 1;
    if let Some(owner) = smallest_enclosing_callable(line_no, ctx.callables) {
        match node.kind() {
            "type_identifier" => {
                if let Ok(name) = node.utf8_text(content.as_bytes()) {
                    push_reference_edge(
                        col,
                        &owner.symbol_fqn,
                        name,
                        line_no,
                        RefKind::Type.as_str(),
                        &SymbolLookup {
                            local_targets: ctx.type_targets,
                            imported_symbol_refs: Some(ctx.imported_symbol_refs),
                        },
                    );
                }
            }
            "identifier" if js_ts_identifier_is_value_reference(node) => {
                if let Ok(name) = node.utf8_text(content.as_bytes()) {
                    push_reference_edge(
                        col,
                        &owner.symbol_fqn,
                        name,
                        line_no,
                        RefKind::Value.as_str(),
                        &SymbolLookup {
                            local_targets: ctx.value_targets,
                            imported_symbol_refs: Some(ctx.imported_symbol_refs),
                        },
                    );
                }
            }
            _ => {}
        }
    }

    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        collect_js_ts_reference_edges_recursive(child, content, ctx, col);
    }
}

fn collect_rust_reference_edges_recursive(
    node: tree_sitter::Node,
    content: &str,
    ctx: &ReferenceCtx,
    col: &mut EdgeCollector,
) {
    let line_no = node.start_position().row as i32 + 1;
    if let Some(owner) = smallest_enclosing_callable(line_no, ctx.callables) {
        match node.kind() {
            "type_identifier" => {
                if let Ok(name) = node.utf8_text(content.as_bytes()) {
                    push_reference_edge(
                        col,
                        &owner.symbol_fqn,
                        name,
                        line_no,
                        RefKind::Type.as_str(),
                        &SymbolLookup {
                            local_targets: ctx.type_targets,
                            imported_symbol_refs: Some(ctx.imported_symbol_refs),
                        },
                    );
                }
            }
            "identifier" if rust_identifier_is_value_reference(node) => {
                if let Ok(name) = node.utf8_text(content.as_bytes()) {
                    push_reference_edge(
                        col,
                        &owner.symbol_fqn,
                        name,
                        line_no,
                        RefKind::Value.as_str(),
                        &SymbolLookup {
                            local_targets: ctx.value_targets,
                            imported_symbol_refs: Some(ctx.imported_symbol_refs),
                        },
                    );
                }
            }
            _ => {}
        }
    }

    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        collect_rust_reference_edges_recursive(child, content, ctx, col);
    }
}
