use super::*;

fn test_dependency_edge(id: &str, edge_kind: EdgeKind, to_symbol_ref: &str) -> DependencyEdge {
    DependencyEdge {
        id: async_graphql::ID::from(id),
        edge_kind,
        language: "rust".to_string(),
        from_artefact_id: async_graphql::ID::from("from-artefact"),
        to_artefact_id: Some(async_graphql::ID::from("to-artefact")),
        to_symbol_ref: Some(to_symbol_ref.to_string()),
        start_line: Some(1),
        end_line: Some(1),
        metadata: None,
        scope: crate::graphql::ResolverScope::default(),
    }
}

#[test]
fn artefact_selector_accepts_symbol_fqn_or_path_modes() {
    let symbol = ArtefactSelectorInput {
        symbol_fqn: Some("src/main.rs::main".to_string()),
        search: None,
        path: None,
        lines: None,
    };
    assert_eq!(
        symbol.selection_mode().expect("symbol selector"),
        ArtefactSelectorMode::SymbolFqn("src/main.rs::main".to_string())
    );

    let path = ArtefactSelectorInput {
        symbol_fqn: None,
        search: None,
        path: Some("src/main.rs".to_string()),
        lines: Some(LineRangeInput { start: 20, end: 25 }),
    };
    assert_eq!(
        path.selection_mode().expect("path selector"),
        ArtefactSelectorMode::Path {
            path: "src/main.rs".to_string(),
            lines: Some(LineRangeInput { start: 20, end: 25 }),
        }
    );
}

#[test]
fn artefact_selector_accepts_search_mode() {
    let search = ArtefactSelectorInput {
        symbol_fqn: None,
        search: Some("payLater()".to_string()),
        path: None,
        lines: None,
    };

    assert_eq!(
        search.selection_mode().expect("search selector"),
        ArtefactSelectorMode::Search("payLater()".to_string())
    );
}

#[test]
fn artefact_selector_rejects_invalid_combinations() {
    let err = ArtefactSelectorInput {
        symbol_fqn: Some("src/main.rs::main".to_string()),
        search: None,
        path: Some("src/main.rs".to_string()),
        lines: None,
    }
    .selection_mode()
    .expect_err("mixed selector should fail");
    assert!(
        err.message
            .contains("allows exactly one of `symbolFqn`, `search`, or `path`/`lines`")
    );

    let err = ArtefactSelectorInput {
        symbol_fqn: None,
        search: None,
        path: None,
        lines: Some(LineRangeInput { start: 20, end: 25 }),
    }
    .selection_mode()
    .expect_err("lines without path should fail");
    assert!(
        err.message
            .contains("requires `path` when `lines` is provided")
    );

    let err = ArtefactSelectorInput {
        symbol_fqn: None,
        search: Some("  ".to_string()),
        path: None,
        lines: None,
    }
    .selection_mode()
    .expect_err("blank search selector should fail");
    assert!(err.message.contains("non-empty `search`"));

    let err = ArtefactSelectorInput {
        symbol_fqn: None,
        search: Some("payLater".to_string()),
        path: Some("src/main.rs".to_string()),
        lines: None,
    }
    .selection_mode()
    .expect_err("search selector mixed with path should fail");
    assert!(
        err.message
            .contains("allows exactly one of `symbolFqn`, `search`, or `path`/`lines`")
    );

    let err = ArtefactSelectorInput {
        symbol_fqn: None,
        search: Some("payLater".to_string()),
        path: None,
        lines: Some(LineRangeInput { start: 20, end: 25 }),
    }
    .selection_mode()
    .expect_err("search selector mixed with lines should fail");
    assert!(
        err.message
            .contains("allows exactly one of `symbolFqn`, `search`, or `path`/`lines`")
    );

    let err = ArtefactSelectorInput {
        symbol_fqn: Some("src/main.rs::main".to_string()),
        search: None,
        path: None,
        lines: Some(LineRangeInput { start: 20, end: 25 }),
    }
    .selection_mode()
    .expect_err("symbol selector mixed with lines should fail");
    assert!(
        err.message
            .contains("allows exactly one of `symbolFqn`, `search`, or `path`/`lines`")
    );

    let err = ArtefactSelectorInput {
        symbol_fqn: None,
        search: None,
        path: None,
        lines: None,
    }
    .selection_mode()
    .expect_err("empty selector should fail");
    assert!(err.message.contains("requires exactly one selector mode"));

    let err = ArtefactSelectorInput {
        symbol_fqn: None,
        search: Some("  ".to_string()),
        path: None,
        lines: None,
    }
    .selection_mode()
    .expect_err("blank search selector should fail");
    assert!(err.message.contains("non-empty `search`"));
}

#[test]
fn build_dependency_summary_embeds_expand_hint_when_present() {
    let incoming = vec![test_dependency_edge(
        "edge-1",
        EdgeKind::Calls,
        "src/lib.rs::target",
    )];
    let outgoing = vec![
        test_dependency_edge("edge-1", EdgeKind::Calls, "src/lib.rs::target"),
        test_dependency_edge("edge-2", EdgeKind::References, "src/lib.rs::other"),
    ];
    let expand_hint = build_dependency_expand_hint(2);

    assert_eq!(
        build_dependency_summary(&incoming, &outgoing, 1, expand_hint.as_ref()),
        serde_json::json!({
            "dependencies": {
                "selectedArtefact": 1,
                "total": 2,
                "incoming": 1,
                "outgoing": 2,
                "kindCounts": {
                    "calls": 1,
                    "exports": 0,
                    "extends": 0,
                    "implements": 0,
                "imports": 0,
                "references": 1,
                }
            },
            "expandHint": {
                "intent": "Use direction to filter dependencies by flow relative to the selected artefacts: incoming maps to IN and outgoing maps to OUT. Use kind to filter dependencies by relationship type: kindCounts.calls maps to CALLS, kindCounts.imports maps to IMPORTS and so on.",
                "template": "Direction example: bitloops devql query '{ selectArtefacts(...) { dependencies(direction: IN) { items(first: 50) { edgeKind fromArtefact { symbolFqn path startLine endLine } toArtefact { symbolFqn path startLine endLine } toSymbolRef } } } }'\nKind example: bitloops devql query '{ selectArtefacts(...) { dependencies(kind: CALLS) { items(first: 50) { edgeKind fromArtefact { symbolFqn path startLine endLine } toArtefact { symbolFqn path startLine endLine } toSymbolRef } } } }'\nCombined example: bitloops devql query '{ selectArtefacts(...) { dependencies(direction: IN, kind: CALLS) { items(first: 50) { edgeKind fromArtefact { symbolFqn path startLine endLine } toArtefact { symbolFqn path startLine endLine } toSymbolRef } } } }'",
                "parameters": {
                    "direction": {
                        "intent": "Choose dependency flow relative to the selected artefacts",
                        "supportedValues": ["IN", "OUT"]
                    },
                    "kind": {
                        "intent": "Choose dependency relationship type",
                        "supportedValues": ["CALLS", "EXPORTS", "EXTENDS", "IMPLEMENTS", "IMPORTS", "REFERENCES"]
                    }
                }
            }
        })
    );
}

#[test]
fn build_dependency_summary_keeps_zero_value_kind_buckets() {
    assert_eq!(
        build_dependency_summary(&[], &[], 1, None),
        serde_json::json!({
            "dependencies": {
                "selectedArtefact": 1,
                "total": 0,
                "incoming": 0,
                "outgoing": 0,
                "kindCounts": {
                    "calls": 0,
                    "exports": 0,
                    "extends": 0,
                    "implements": 0,
                    "imports": 0,
                    "references": 0,
                }
            }
        })
    );
}

#[test]
fn build_clone_expand_hint_matches_contract_when_matches_exist() {
    assert_eq!(
        build_clone_expand_hint(2),
        Some(CloneExpandHint {
            intent: "Inspect code matches".to_string(),
            template: "bitloops devql query '{ selectArtefacts(by: ...) { codeMatches(relationKind: <KIND>) { items(first: 20) { ... } } } }'".to_string(),
            parameters: CloneExpandHintParameters {
                kind: ExpandHintParameter {
                    intent: "Choose which relation kind to inspect".to_string(),
                    supported_values: vec![
                        RELATION_KIND_EXACT_DUPLICATE.to_string(),
                        RELATION_KIND_SIMILAR_IMPLEMENTATION.to_string(),
                        RELATION_KIND_SHARED_LOGIC_CANDIDATE.to_string(),
                        RELATION_KIND_DIVERGED_IMPLEMENTATION.to_string(),
                        RELATION_KIND_WEAK_CLONE_CANDIDATE.to_string(),
                    ],
                },
            },
        })
    );
}

#[test]
fn build_clone_expand_hint_omits_hint_when_no_matches_exist() {
    assert_eq!(build_clone_expand_hint(0), None);
}

#[test]
fn build_dependency_expand_hint_matches_contract_when_dependencies_exist() {
    assert_eq!(
        build_dependency_expand_hint(2),
        Some(DependencyExpandHint {
            intent: "Use direction to filter dependencies by flow relative to the selected artefacts: incoming maps to IN and outgoing maps to OUT. Use kind to filter dependencies by relationship type: kindCounts.calls maps to CALLS, kindCounts.imports maps to IMPORTS and so on.".to_string(),
            template: "Direction example: bitloops devql query '{ selectArtefacts(...) { dependencies(direction: IN) { items(first: 50) { edgeKind fromArtefact { symbolFqn path startLine endLine } toArtefact { symbolFqn path startLine endLine } toSymbolRef } } } }'\nKind example: bitloops devql query '{ selectArtefacts(...) { dependencies(kind: CALLS) { items(first: 50) { edgeKind fromArtefact { symbolFqn path startLine endLine } toArtefact { symbolFqn path startLine endLine } toSymbolRef } } } }'\nCombined example: bitloops devql query '{ selectArtefacts(...) { dependencies(direction: IN, kind: CALLS) { items(first: 50) { edgeKind fromArtefact { symbolFqn path startLine endLine } toArtefact { symbolFqn path startLine endLine } toSymbolRef } } } }'".to_string(),
            parameters: DependencyExpandHintParameters {
                direction: ExpandHintParameter {
                    intent: "Choose dependency flow relative to the selected artefacts"
                        .to_string(),
                    supported_values: vec!["IN".to_string(), "OUT".to_string()],
                },
                kind: ExpandHintParameter {
                    intent: "Choose dependency relationship type".to_string(),
                    supported_values: vec![
                        "CALLS".to_string(),
                        "EXPORTS".to_string(),
                        "EXTENDS".to_string(),
                        "IMPLEMENTS".to_string(),
                        "IMPORTS".to_string(),
                        "REFERENCES".to_string(),
                    ],
                },
            },
        })
    );
}

#[test]
fn build_dependency_expand_hint_omits_hint_when_no_dependencies_match() {
    assert_eq!(build_dependency_expand_hint(0), None);
}
