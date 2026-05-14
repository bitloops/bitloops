use crate::capability_packs::semantic_clones::scoring::{
    RELATION_KIND_DIVERGED_IMPLEMENTATION, RELATION_KIND_EXACT_DUPLICATE,
    RELATION_KIND_SHARED_LOGIC_CANDIDATE, RELATION_KIND_SIMILAR_IMPLEMENTATION,
    RELATION_KIND_WEAK_CLONE_CANDIDATE,
};
use crate::graphql::context::ArchitectureGraphTargetOverview;
use crate::graphql::types::{
    ArchitectureGraphEdge, ArchitectureGraphEdgeKind, ArchitectureGraphNode,
    ArchitectureGraphNodeKind,
};
use crate::graphql::types::{DependencyEdge, EdgeKind, ExpandHintParameter, LineRangeInput};

use super::support::{
    CONTEXT_GUIDANCE_STAGE_SCHEMA, SelectionSummaryStages, build_clone_expand_hint,
    build_dependency_expand_hint, build_dependency_summary, build_historical_context_expand_hint,
    build_historical_context_summary, build_selection_summary, captured_preview, take_stage_items,
};
use super::{
    ArtefactSelectorInput, ArtefactSelectorMode, CloneExpandHint, DependencyExpandHint, SearchMode,
};
use crate::graphql::types::artefact_selection::stages::{
    ArchitectureOverviewStageData, CheckpointStageData, CloneStageData, ContextGuidanceItem,
    ContextGuidanceStageData, DependencyStageData, HistoricalContextItem,
    HistoricalContextStageData, HistoricalMatchReason, HistoricalMatchStrength,
    HistoricalToolEvent, TestsStageData,
};

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

fn test_architecture_node(
    id: &str,
    kind: ArchitectureGraphNodeKind,
    label: &str,
    artefact_id: Option<&str>,
    path: Option<&str>,
) -> ArchitectureGraphNode {
    ArchitectureGraphNode {
        id: id.to_string(),
        kind,
        label: label.to_string(),
        artefact_id: artefact_id.map(str::to_string),
        symbol_id: None,
        path: path.map(str::to_string),
        entry_kind: None,
        source_kind: "COMPUTED".to_string(),
        confidence: 1.0,
        computed: true,
        asserted: false,
        suppressed: false,
        effective: true,
        provenance: async_graphql::types::Json(serde_json::json!({})),
        computed_provenance: async_graphql::types::Json(serde_json::json!({})),
        asserted_provenance: async_graphql::types::Json(serde_json::Value::Null),
        evidence: async_graphql::types::Json(serde_json::json!([])),
        properties: async_graphql::types::Json(serde_json::json!({})),
        annotations: Vec::new(),
    }
}

fn test_architecture_edge(id: &str, from_node_id: &str, to_node_id: &str) -> ArchitectureGraphEdge {
    ArchitectureGraphEdge {
        id: id.to_string(),
        kind: ArchitectureGraphEdgeKind::Implements,
        from_node_id: from_node_id.to_string(),
        to_node_id: to_node_id.to_string(),
        source_kind: "COMPUTED".to_string(),
        confidence: 1.0,
        computed: true,
        asserted: false,
        suppressed: false,
        effective: true,
        provenance: async_graphql::types::Json(serde_json::json!({})),
        computed_provenance: async_graphql::types::Json(serde_json::json!({})),
        asserted_provenance: async_graphql::types::Json(serde_json::Value::Null),
        evidence: async_graphql::types::Json(serde_json::json!([])),
        properties: async_graphql::types::Json(serde_json::json!({})),
        annotations: Vec::new(),
    }
}

#[test]
fn historical_context_item_keeps_captured_evidence_fields() {
    let item = HistoricalContextItem {
        checkpoint_id: async_graphql::ID::from("checkpoint-1"),
        session_id: "session-1".to_string(),
        turn_id: Some("turn-1".to_string()),
        agent_type: Some("codex".to_string()),
        model: Some("gpt-5.4".to_string()),
        event_time: crate::graphql::types::DateTimeScalar::from_rfc3339(
            "2026-04-28T12:30:13+00:00",
        )
        .expect("timestamp parses"),
        match_reason: HistoricalMatchReason::FileRelation,
        match_strength: HistoricalMatchStrength::Medium,
        prompt_preview: Some("explain this file".to_string()),
        turn_summary: Some("read attr parsing".to_string()),
        transcript_preview: Some("captured transcript text".to_string()),
        files_modified: vec!["src/lib.rs".to_string()],
        file_relations: Vec::new(),
        tool_events: vec![HistoricalToolEvent {
            tool_kind: Some("Read".to_string()),
            input_summary: Some("src/lib.rs".to_string()),
            output_summary: Some("file contents".to_string()),
            command: None,
        }],
        evidence_kinds: vec![HistoricalMatchReason::FileRelation],
    };

    assert_eq!(item.session_id, "session-1");
    assert_eq!(item.match_reason, HistoricalMatchReason::FileRelation);
    assert_eq!(item.tool_events[0].tool_kind.as_deref(), Some("Read"));
}

#[test]
fn captured_preview_truncates_without_rewriting_text() {
    let text = "0123456789abcdefghijklmnopqrstuvwxyz";
    assert_eq!(captured_preview(text, 12).as_deref(), Some("0123456789ab"));
    assert_eq!(captured_preview("   ", 12), None);
}

#[test]
fn historical_context_summary_counts_distinct_evidence() {
    let event_time =
        crate::graphql::types::DateTimeScalar::from_rfc3339("2026-04-28T12:30:13+00:00")
            .expect("timestamp parses");
    let rows = vec![HistoricalContextItem {
        checkpoint_id: async_graphql::ID::from("checkpoint-1"),
        session_id: "session-1".to_string(),
        turn_id: Some("turn-1".to_string()),
        agent_type: Some("codex".to_string()),
        model: None,
        event_time,
        match_reason: HistoricalMatchReason::FileRelation,
        match_strength: HistoricalMatchStrength::Medium,
        prompt_preview: None,
        turn_summary: None,
        transcript_preview: None,
        files_modified: Vec::new(),
        file_relations: Vec::new(),
        tool_events: Vec::new(),
        evidence_kinds: vec![
            HistoricalMatchReason::FileRelation,
            HistoricalMatchReason::SymbolProvenance,
        ],
    }];

    assert_eq!(
        build_historical_context_summary(&rows),
        serde_json::json!({
            "totalCount": 1,
            "latestAt": "2026-04-28T12:30:13+00:00",
            "agents": ["codex"],
            "checkpointCount": 1,
            "sessionCount": 1,
            "turnCount": 1,
            "evidenceCounts": {
                "symbolProvenance": 1,
                "fileRelation": 1,
                "lineOverlap": 0
            },
            "expandHint": {
                "intent": "Inspect captured historical context for selected artefacts",
                "template": "bitloops devql query '{ selectArtefacts(...) { historicalContext { overview items(first: 20) { checkpointId sessionId turnId promptPreview transcriptPreview toolEvents { toolKind inputSummary outputSummary command } } } } }'"
            }
        })
    );

    assert!(build_historical_context_expand_hint(0).is_none());
}

fn empty_overview_stage_inputs() -> (
    CheckpointStageData,
    CloneStageData,
    DependencyStageData,
    TestsStageData,
    HistoricalContextStageData,
    ContextGuidanceStageData,
) {
    (
        CheckpointStageData {
            summary: serde_json::json!({ "totalCount": 0 }),
            schema: None,
            items: Vec::new(),
        },
        CloneStageData {
            summary: serde_json::json!({ "counts": { "total": 0 } }),
            expand_hint: None,
            schema: None,
            items: Vec::new(),
        },
        DependencyStageData {
            summary: serde_json::json!({ "dependencies": { "total": 0 } }),
            expand_hint: None,
            schema: None,
            items: Vec::new(),
        },
        TestsStageData {
            summary: serde_json::json!({ "totalCoveringTests": 0 }),
            schema: None,
            items: Vec::new(),
        },
        HistoricalContextStageData {
            summary: serde_json::json!({ "totalCount": 0 }),
            schema: None,
            items: Vec::new(),
        },
        ContextGuidanceStageData {
            summary: serde_json::json!({ "totalCount": 0 }),
            schema: None,
            items: Vec::new(),
        },
    )
}

#[test]
fn selection_summary_includes_historical_context_stage() {
    let (checkpoints, clones, deps, tests, historical_context, context_guidance) =
        empty_overview_stage_inputs();
    let architecture =
        ArchitectureOverviewStageData::unavailable(1, "no_matching_architecture_facts");

    let summary = build_selection_summary(
        1,
        SelectionSummaryStages {
            checkpoints: &checkpoints,
            clones: &clones,
            deps: &deps,
            tests: &tests,
            historical_context: &historical_context,
            context_guidance: &context_guidance,
            http: &serde_json::json!({
            "bundleCount": 0,
            "riskCount": 0,
            "topRisks": [],
            "expandHint": {
                "template": "selectArtefacts(...){ httpContext { bundles { ... } } }"
            }
            }),
            architecture: &architecture,
        },
    );

    assert_eq!(
        summary["historicalContext"],
        serde_json::json!({
            "overview": { "totalCount": 0 },
            "schema": null
        })
    );
    assert_eq!(
        summary["contextGuidance"],
        serde_json::json!({
            "overview": { "totalCount": 0 },
            "schema": null
        })
    );
    assert_eq!(summary["http"]["bundleCount"], 0);
    assert_eq!(summary["http"]["riskCount"], 0);
}

#[test]
fn selection_summary_includes_unavailable_architecture_stage() {
    let (checkpoints, clones, deps, tests, historical_context, context_guidance) =
        empty_overview_stage_inputs();
    let architecture =
        ArchitectureOverviewStageData::unavailable(2, "no_matching_architecture_facts");

    let summary = build_selection_summary(
        2,
        SelectionSummaryStages {
            checkpoints: &checkpoints,
            clones: &clones,
            deps: &deps,
            tests: &tests,
            historical_context: &historical_context,
            context_guidance: &context_guidance,
            http: &serde_json::json!({
                "bundleCount": 0,
                "riskCount": 0,
                "topRisks": []
            }),
            architecture: &architecture,
        },
    );

    assert_eq!(
        summary["architecture"],
        serde_json::json!({
            "overview": {
                "available": false,
                "reason": "no_matching_architecture_facts",
                "selectedArtefactCount": 2,
                "matchedArtefactCount": 0,
                "directNodeCount": 0,
                "relatedNodeCount": 0,
                "edgeCount": 0,
                "nodeKinds": {},
                "entryPointCount": 0,
                "componentCount": 0,
                "containerCount": 0,
                "assertedCount": 0,
                "suppressedCount": 0,
                "topNodes": []
            },
            "expandHint": null,
            "schema": null
        })
    );
}

#[test]
fn selection_summary_includes_available_architecture_stage() {
    let (checkpoints, clones, deps, tests, historical_context, context_guidance) =
        empty_overview_stage_inputs();
    let architecture = ArchitectureOverviewStageData {
        summary: serde_json::json!({
            "available": true,
            "reason": null,
            "selectedArtefactCount": 1,
            "matchedArtefactCount": 1,
            "directNodeCount": 1,
            "relatedNodeCount": 2,
            "edgeCount": 2,
            "nodeKinds": {
                "NODE": 1,
                "COMPONENT": 1,
                "CONTAINER": 1
            },
            "entryPointCount": 0,
            "componentCount": 1,
            "containerCount": 1,
            "assertedCount": 0,
            "suppressedCount": 0,
            "topNodes": [
                {
                    "id": "component-1",
                    "kind": "COMPONENT",
                    "label": "src/service",
                    "path": "src/service.rs",
                    "artefactId": null,
                    "symbolId": null,
                    "entryKind": null,
                    "confidence": 0.55,
                    "asserted": false,
                    "suppressed": false
                }
            ]
        }),
        expand_hint: Some(serde_json::json!({
            "intent": "Inspect architecture graph facts for the selected artefacts.",
            "template": "bitloops devql query '{ selectArtefacts(...) { artefacts { id path architectureNode { id kind label entryKind confidence } } } }'"
        })),
        schema: Some(super::support::ARCHITECTURE_OVERVIEW_SCHEMA.to_string()),
    };

    let summary = build_selection_summary(
        1,
        SelectionSummaryStages {
            checkpoints: &checkpoints,
            clones: &clones,
            deps: &deps,
            tests: &tests,
            historical_context: &historical_context,
            context_guidance: &context_guidance,
            http: &serde_json::json!({
                "bundleCount": 0,
                "riskCount": 0,
                "topRisks": []
            }),
            architecture: &architecture,
        },
    );

    assert_eq!(summary["architecture"]["overview"]["available"], true);
    assert_eq!(
        summary["architecture"]["overview"]["matchedArtefactCount"],
        1
    );
    assert_eq!(
        summary["architecture"]["overview"]["nodeKinds"]["COMPONENT"],
        1
    );
    assert_eq!(
        summary["architecture"]["expandHint"]["template"],
        "bitloops devql query '{ selectArtefacts(...) { artefacts { id path architectureNode { id kind label entryKind confidence } } } }'"
    );
    assert!(
        summary["architecture"]["schema"]
            .as_str()
            .unwrap()
            .contains("architecture")
    );
}

#[test]
fn architecture_overview_stage_summarizes_nodes_edges_and_top_nodes() {
    let overview = ArchitectureGraphTargetOverview {
        available: true,
        reason: None,
        selected_artefact_count: 1,
        matched_artefact_ids: vec!["artefact-main".to_string()],
        direct_node_count: 1,
        nodes: vec![
            test_architecture_node(
                "node-main",
                ArchitectureGraphNodeKind::Node,
                "main",
                Some("artefact-main"),
                Some("src/main.rs"),
            ),
            test_architecture_node(
                "component-main",
                ArchitectureGraphNodeKind::Component,
                "src/main",
                None,
                Some("src/main.rs"),
            ),
        ],
        edges: vec![test_architecture_edge(
            "node-component",
            "node-main",
            "component-main",
        )],
    };

    let stage = super::support::build_architecture_overview_stage(overview);

    assert_eq!(stage.summary["available"], true);
    assert_eq!(stage.summary["selectedArtefactCount"], 1);
    assert_eq!(stage.summary["matchedArtefactCount"], 1);
    assert_eq!(stage.summary["directNodeCount"], 1);
    assert_eq!(stage.summary["relatedNodeCount"], 2);
    assert_eq!(stage.summary["edgeCount"], 1);
    assert_eq!(stage.summary["nodeKinds"]["NODE"], 1);
    assert_eq!(stage.summary["nodeKinds"]["COMPONENT"], 1);
    assert_eq!(stage.summary["componentCount"], 1);
    assert_eq!(stage.summary["topNodes"][0]["id"], "component-main");
    assert!(stage.expand_hint.is_some());
    assert!(
        stage
            .schema
            .as_deref()
            .unwrap()
            .contains("ArchitectureOverview")
    );
}

#[test]
fn context_guidance_stage_item_pagination_rejects_non_positive_first() {
    let err = take_stage_items::<ContextGuidanceItem>(&[], 0)
        .expect_err("context guidance item pagination should reject zero");

    assert!(err.message.contains("`first` must be greater than 0"));
}

#[test]
fn http_selection_terms_split_search_query_for_index_fallback() {
    let terms = super::resolvers::split_http_selection_terms(
        "HEAD, Content-Length RouteFuture strip_body `Empty` (Hyper)",
    );

    assert_eq!(
        terms,
        vec![
            "HEAD",
            "Content-Length",
            "RouteFuture",
            "strip_body",
            "Empty",
            "Hyper"
        ]
    );
}

#[test]
fn context_guidance_stage_schema_matches_contract() {
    assert!(CONTEXT_GUIDANCE_STAGE_SCHEMA.contains(
        "contextGuidance(agent: String, since: DateTime, evidenceKind: HistoricalEvidenceKind, category: ContextGuidanceCategory, kind: String): ContextGuidanceStageResult!"
    ));
    assert!(CONTEXT_GUIDANCE_STAGE_SCHEMA.contains("type ContextGuidanceItem"));
    assert!(CONTEXT_GUIDANCE_STAGE_SCHEMA.contains("type ContextGuidanceSource"));
    assert!(CONTEXT_GUIDANCE_STAGE_SCHEMA.contains("enum ContextGuidanceCategory"));
    assert!(CONTEXT_GUIDANCE_STAGE_SCHEMA.contains("generatedAt"));
    assert!(CONTEXT_GUIDANCE_STAGE_SCHEMA.contains("sourceModel"));
    assert!(CONTEXT_GUIDANCE_STAGE_SCHEMA.contains("knowledgeItemId"));
    assert!(CONTEXT_GUIDANCE_STAGE_SCHEMA.contains("knowledgeItemVersionId"));
    assert!(CONTEXT_GUIDANCE_STAGE_SCHEMA.contains("relationAssertionId"));
}

#[test]
fn artefact_selector_accepts_symbol_fqn_or_path_modes() {
    let symbol = ArtefactSelectorInput {
        symbol_fqn: Some("src/main.rs::main".to_string()),
        search: None,
        search_mode: None,
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
        search_mode: None,
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
        search_mode: Some(SearchMode::Lexical),
        path: None,
        lines: None,
    };

    assert_eq!(
        search.selection_mode().expect("search selector"),
        ArtefactSelectorMode::Search {
            query: "payLater()".to_string(),
            mode: SearchMode::Lexical,
        }
    );
}

#[test]
fn artefact_selector_rejects_invalid_combinations() {
    let err = ArtefactSelectorInput {
        symbol_fqn: Some("src/main.rs::main".to_string()),
        search: None,
        search_mode: None,
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
        search_mode: None,
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
        search_mode: None,
        path: None,
        lines: None,
    }
    .selection_mode()
    .expect_err("blank search selector should fail");
    assert!(err.message.contains("non-empty `search`"));

    let err = ArtefactSelectorInput {
        symbol_fqn: None,
        search: Some("payLater".to_string()),
        search_mode: None,
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
        search_mode: None,
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
        search_mode: None,
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
        search_mode: None,
        path: None,
        lines: None,
    }
    .selection_mode()
    .expect_err("empty selector should fail");
    assert!(err.message.contains("requires exactly one selector mode"));

    let err = ArtefactSelectorInput {
        symbol_fqn: None,
        search: Some("  ".to_string()),
        search_mode: None,
        path: None,
        lines: None,
    }
    .selection_mode()
    .expect_err("blank search selector should fail");
    assert!(err.message.contains("non-empty `search`"));

    let err = ArtefactSelectorInput {
        symbol_fqn: None,
        search: None,
        search_mode: Some(SearchMode::Identity),
        path: Some("src/main.rs".to_string()),
        lines: None,
    }
    .selection_mode()
    .expect_err("search mode without search should fail");
    assert!(
        err.message
            .contains("only allows `searchMode` when `search` is provided")
    );
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
                "template": "bitloops devql query '{ selectArtefacts(...) { dependencies(direction: IN, kind: CALLS) { items(first: 20) { edgeKind toSymbolRef } } } }'",
                "parameters": [
                    {
                        "name": "direction",
                        "intent": "Choose dependency flow relative to the selected artefacts",
                        "supportedValues": ["IN", "OUT"]
                    },
                    {
                        "name": "kind",
                        "intent": "Choose dependency relationship type",
                        "supportedValues": ["CALLS", "EXPORTS", "EXTENDS", "IMPLEMENTS", "IMPORTS", "REFERENCES"]
                    }
                ]
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
            parameters: vec![ExpandHintParameter {
                name: "kind".to_string(),
                intent: "Choose which relation kind to inspect".to_string(),
                supported_values: vec![
                    RELATION_KIND_EXACT_DUPLICATE.to_string(),
                    RELATION_KIND_SIMILAR_IMPLEMENTATION.to_string(),
                    RELATION_KIND_SHARED_LOGIC_CANDIDATE.to_string(),
                    RELATION_KIND_DIVERGED_IMPLEMENTATION.to_string(),
                    RELATION_KIND_WEAK_CLONE_CANDIDATE.to_string(),
                ],
            }],
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
            template: "bitloops devql query '{ selectArtefacts(...) { dependencies(direction: IN, kind: CALLS) { items(first: 20) { edgeKind toSymbolRef } } } }'".to_string(),
            parameters: vec![
                ExpandHintParameter {
                    name: "direction".to_string(),
                    intent: "Choose dependency flow relative to the selected artefacts"
                        .to_string(),
                    supported_values: vec!["IN".to_string(), "OUT".to_string()],
                },
                ExpandHintParameter {
                    name: "kind".to_string(),
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
            ],
        })
    );
}

#[test]
fn build_dependency_expand_hint_omits_hint_when_no_dependencies_match() {
    assert_eq!(build_dependency_expand_hint(0), None);
}
