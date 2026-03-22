use tree_sitter::Node;

use crate::capability_packs::test_harness::mapping::model::{
    DiscoveredTestScenario, ReferenceCandidate, ScenarioDiscoverySource,
};

use super::scenarios::{RustDiscoveredScenario, rust_suite_for_node};

pub(crate) fn collect_rust_doctest_scenarios(
    root: Node<'_>,
    source: &str,
    relative_path: &str,
) -> Vec<RustDiscoveredScenario> {
    let mut scenarios = Vec::new();
    let mut stack = vec![root];
    let source_bytes = source.as_bytes();
    let lines: Vec<&str> = source.lines().collect();

    while let Some(node) = stack.pop() {
        if !is_rust_doctest_candidate(node.kind()) {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                stack.push(child);
            }
            continue;
        }

        let Some(item_name) = node
            .child_by_field_name("name")
            .and_then(|name| name.utf8_text(source_bytes).ok())
            .map(str::to_string)
        else {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                stack.push(child);
            }
            continue;
        };

        let item_start_line = node.start_position().row as i64 + 1;
        let doc_lines = collect_preceding_doc_lines(&lines, item_start_line);
        if doc_lines.is_empty() {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                stack.push(child);
            }
            continue;
        }

        let (suite_name, suite_start_line, suite_end_line) =
            rust_suite_for_node(node, source_bytes, relative_path);
        for block in extract_rust_doctest_blocks(&doc_lines) {
            scenarios.push(RustDiscoveredScenario {
                suite_name: format!("{suite_name}::doctests"),
                suite_start_line,
                suite_end_line,
                scenario: DiscoveredTestScenario {
                    name: format!("{item_name}[doctest:{}]", block.start_line),
                    start_line: block.start_line,
                    end_line: block.end_line,
                    reference_candidates: vec![
                        ReferenceCandidate::SymbolName(item_name.clone()),
                        ReferenceCandidate::ExplicitTarget {
                            path: relative_path.to_string(),
                            start_line: item_start_line,
                        },
                    ],
                    discovery_source: ScenarioDiscoverySource::Doctest,
                },
            });
        }

        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            stack.push(child);
        }
    }

    scenarios
}

pub(crate) fn is_rust_doctest_candidate(kind: &str) -> bool {
    matches!(
        kind,
        "function_item"
            | "struct_item"
            | "enum_item"
            | "trait_item"
            | "type_item"
            | "const_item"
            | "mod_item"
    )
}

#[derive(Debug, Clone)]
struct RustDocLine {
    line_number: i64,
    text: String,
}

#[derive(Debug, Clone)]
struct RustDoctestBlock {
    start_line: i64,
    end_line: i64,
}

fn collect_preceding_doc_lines(lines: &[&str], item_start_line: i64) -> Vec<RustDocLine> {
    let mut doc_lines = Vec::new();
    let mut index = item_start_line.saturating_sub(2) as isize;

    while index >= 0 {
        let line = lines[index as usize];
        let trimmed = line.trim_start();
        if let Some(content) = trimmed.strip_prefix("///") {
            doc_lines.push(RustDocLine {
                line_number: index as i64 + 1,
                text: content.trim_start().to_string(),
            });
            index -= 1;
            continue;
        }
        if let Some(content) = trimmed.strip_prefix("//!") {
            doc_lines.push(RustDocLine {
                line_number: index as i64 + 1,
                text: content.trim_start().to_string(),
            });
            index -= 1;
            continue;
        }
        if trimmed.starts_with("#[") || trimmed.is_empty() {
            index -= 1;
            continue;
        }
        break;
    }

    doc_lines.reverse();
    doc_lines
}

fn extract_rust_doctest_blocks(doc_lines: &[RustDocLine]) -> Vec<RustDoctestBlock> {
    let mut blocks = Vec::new();
    let mut active_start: Option<i64> = None;

    for line in doc_lines {
        let trimmed = line.text.trim();
        if active_start.is_none() {
            if let Some(info) = trimmed.strip_prefix("```")
                && rust_doc_fence_is_testable(info)
            {
                active_start = Some(line.line_number);
            }
            continue;
        }

        if trimmed.starts_with("```")
            && let Some(start_line) = active_start.take()
        {
            blocks.push(RustDoctestBlock {
                start_line,
                end_line: line.line_number,
            });
        }
    }

    blocks
}

fn rust_doc_fence_is_testable(info: &str) -> bool {
    let normalized = info.trim().to_ascii_lowercase();
    normalized.is_empty()
        || normalized.contains("rust")
        || normalized.contains("should_panic")
        || normalized.contains("no_run")
        || normalized.contains("compile_fail")
        || normalized.contains("edition")
        || normalized.contains("ignore")
}
