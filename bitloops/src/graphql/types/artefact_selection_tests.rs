use crate::capability_packs::semantic_clones::scoring::{
    RELATION_KIND_DIVERGED_IMPLEMENTATION, RELATION_KIND_EXACT_DUPLICATE,
    RELATION_KIND_SHARED_LOGIC_CANDIDATE, RELATION_KIND_SIMILAR_IMPLEMENTATION,
    RELATION_KIND_WEAK_CLONE_CANDIDATE,
};
use crate::graphql::context::{
    ArchitectureGraphTargetOverview, ArchitectureRoleOverviewAssignment,
    ArchitectureRoleTargetOverview,
};
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

struct ArchitectureRoleAssignmentFixture<'a> {
    assignment_id: &'a str,
    role_id: &'a str,
    canonical_key: &'a str,
    display_name: &'a str,
    family: &'a str,
    artefact_id: Option<&'a str>,
    symbol_id: Option<&'a str>,
    symbol_fqn: Option<&'a str>,
}

fn architecture_role_assignment(
    fixture: ArchitectureRoleAssignmentFixture<'_>,
) -> ArchitectureRoleOverviewAssignment {
    ArchitectureRoleOverviewAssignment {
        assignment_id: fixture.assignment_id.to_string(),
        role_id: fixture.role_id.to_string(),
        canonical_key: fixture.canonical_key.to_string(),
        display_name: fixture.display_name.to_string(),
        description: "Handles HTTP-facing route definitions and request/response adapter functions for user operations.".to_string(),
        family: Some(fixture.family.to_string()),
        target_kind: "artefact".to_string(),
        artefact_id: fixture.artefact_id.map(str::to_string),
        symbol_id: fixture.symbol_id.map(str::to_string),
        path: "src/api/users_handler.rs".to_string(),
        symbol_fqn: fixture.symbol_fqn.map(str::to_string),
        canonical_kind: Some("function".to_string()),
        priority: "primary".to_string(),
        status: "active".to_string(),
        source: "rule".to_string(),
        confidence: 1.0,
        classifier_version: "architecture_roles.deterministic.contract.v1".to_string(),
        rule_version: None,
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
    let architecture = ArchitectureOverviewStageData::unavailable(
        1,
        "no_matching_architecture_role_assignments",
        false,
    );

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
    let architecture = ArchitectureOverviewStageData::unavailable(
        2,
        "no_matching_architecture_role_assignments",
        false,
    );

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
                "reason": "no_matching_architecture_role_assignments",
                "selectedArtefactCount": 2,
                "assignedSelectedArtefactCount": 0,
                "unassignedSelectedArtefactCount": 2,
                "roleAssignmentCount": 0,
                "roleCount": 0,
                "familyCounts": {},
                "sourceCounts": {},
                "targetKindCounts": {},
                "confidence": null,
                "primaryRoles": [],
                "graphContextAvailable": false
            },
            "expandHint": {
                "intent": "Inspect architecture role assignments for the selected artefacts.",
                "template": "bitloops devql query '{ selectArtefacts(by: <selector>) { architectureRoles { overview items(first: 20) { role { canonicalKey displayName family description } target { path symbolFqn canonicalKind } confidence source status } } architectureGraphContext { overview nodes(first: 20) { id kind label path } edges(first: 20) { id kind fromNodeId toNodeId } } } }'"
            },
            "schema": super::support::ARCHITECTURE_OVERVIEW_SCHEMA
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
            "selectedArtefactCount": 3,
            "assignedSelectedArtefactCount": 2,
            "unassignedSelectedArtefactCount": 1,
            "roleAssignmentCount": 2,
            "roleCount": 1,
            "familyCounts": { "entrypoint": 2 },
            "sourceCounts": { "rule": 2 },
            "targetKindCounts": { "artefact": 2 },
            "confidence": { "min": 1.0, "avg": 1.0, "max": 1.0 },
            "primaryRoles": [
                {
                    "canonicalKey": "http_api_surface",
                    "displayName": "HTTP API Surface",
                    "assignmentCount": 2
                }
            ],
            "graphContextAvailable": true
        }),
        expand_hint: Some(super::support::architecture_overview_expand_hint()),
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
        summary["architecture"]["overview"]["roleAssignmentCount"],
        2
    );
    assert_eq!(summary["architecture"]["overview"]["roleCount"], 1);
    assert_eq!(
        summary["architecture"]["overview"]["primaryRoles"][0]["canonicalKey"],
        "http_api_surface"
    );
    assert_eq!(
        summary["architecture"]["overview"]["graphContextAvailable"],
        true
    );
    assert!(
        summary["architecture"]["overview"]
            .get("edgeCount")
            .is_none()
    );
    assert!(
        summary["architecture"]["schema"]
            .as_str()
            .unwrap()
            .contains("architecture")
    );
}

#[test]
fn architecture_graph_context_summary_keeps_graph_counts_out_of_overview() {
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

    let summary = super::support::build_architecture_graph_context_summary(&overview);

    assert_eq!(summary["available"], true);
    assert_eq!(summary["matchedArtefactCount"], 1);
    assert_eq!(summary["directNodeCount"], 1);
    assert_eq!(summary["relatedNodeCount"], 2);
    assert_eq!(summary["edgeCount"], 1);
    assert_eq!(summary["nodeKinds"]["NODE"], 1);
    assert_eq!(summary["nodeKinds"]["COMPONENT"], 1);
    assert_eq!(summary["componentCount"], 1);
    assert!(summary.get("topNodes").is_none());
}

#[test]
fn architecture_overview_stage_prefers_role_assignments_over_graph_counts() {
    let overview = ArchitectureRoleTargetOverview {
        available: true,
        reason: None,
        selected_artefact_count: 3,
        assigned_artefact_ids: vec![
            "62e5ddba-4108-021b-d87d-7689a9b00e05".to_string(),
            "e3662529-8705-9810-0f48-fe353d189ef4".to_string(),
        ],
        assignments: vec![
            architecture_role_assignment(ArchitectureRoleAssignmentFixture {
                assignment_id: "df412e24-4f79-99aa-af57-06684518c264",
                role_id: "bedecda4-4dac-3aea-7a17-036857cc4a13",
                canonical_key: "http_api_surface",
                display_name: "HTTP API Surface",
                family: "entrypoint",
                artefact_id: Some("62e5ddba-4108-021b-d87d-7689a9b00e05"),
                symbol_id: Some("87571974-d08c-d157-26f8-2cd20b09103b"),
                symbol_fqn: Some("src/api/users_handler.rs::create_user_http_handler"),
            }),
            architecture_role_assignment(ArchitectureRoleAssignmentFixture {
                assignment_id: "c50fa84d-71ac-b805-4da7-fe9eaa2713f5",
                role_id: "bedecda4-4dac-3aea-7a17-036857cc4a13",
                canonical_key: "http_api_surface",
                display_name: "HTTP API Surface",
                family: "entrypoint",
                artefact_id: Some("e3662529-8705-9810-0f48-fe353d189ef4"),
                symbol_id: Some("cc0e957d-7ff6-f860-916c-48a678ed37f9"),
                symbol_fqn: Some("src/api/users_handler.rs::get_user_http_handler"),
            }),
        ],
    };

    let stage = super::support::build_architecture_overview_stage(overview, true);

    assert_eq!(stage.summary["available"], true);
    assert_eq!(stage.summary["selectedArtefactCount"], 3);
    assert_eq!(stage.summary["assignedSelectedArtefactCount"], 2);
    assert_eq!(stage.summary["unassignedSelectedArtefactCount"], 1);
    assert_eq!(stage.summary["roleAssignmentCount"], 2);
    assert_eq!(stage.summary["roleCount"], 1);
    assert_eq!(stage.summary["familyCounts"]["entrypoint"], 2);
    assert_eq!(stage.summary["sourceCounts"]["rule"], 2);
    assert_eq!(stage.summary["targetKindCounts"]["artefact"], 2);
    assert_eq!(stage.summary["confidence"]["avg"], 1.0);
    assert_eq!(
        stage.summary["primaryRoles"][0]["canonicalKey"],
        "http_api_surface"
    );
    assert_eq!(stage.summary["primaryRoles"][0]["assignmentCount"], 2);
    assert_eq!(
        stage.summary["primaryRoles"][0]["confidence"],
        serde_json::json!({ "min": 1.0, "avg": 1.0, "max": 1.0 })
    );
    assert_eq!(
        stage.summary["primaryRoles"][0]["targetKinds"],
        serde_json::json!({ "artefact": 2 })
    );
    assert_eq!(
        stage.summary["primaryRoles"][0]["sources"],
        serde_json::json!({ "rule": 2 })
    );
    assert_eq!(
        stage.summary["primaryRoles"][0]["targets"][0]["symbolFqn"],
        "src/api/users_handler.rs::create_user_http_handler"
    );
    assert_eq!(
        stage.summary["primaryRoles"][0]["targets"][1]["symbolFqn"],
        "src/api/users_handler.rs::get_user_http_handler"
    );
    assert_eq!(stage.summary["graphContextAvailable"], true);
    assert!(stage.summary.get("edgeCount").is_none());
    assert!(stage.summary.get("nodeKinds").is_none());
    assert!(stage.summary.get("topNodes").is_none());
}

#[test]
fn architecture_overview_stage_reports_no_role_assignments_with_graph_flag() {
    let overview =
        ArchitectureRoleTargetOverview::unavailable(3, "no_matching_architecture_role_assignments");

    let stage = super::support::build_architecture_overview_stage(overview, true);

    assert_eq!(
        stage.summary,
        serde_json::json!({
            "available": false,
            "reason": "no_matching_architecture_role_assignments",
            "selectedArtefactCount": 3,
            "assignedSelectedArtefactCount": 0,
            "unassignedSelectedArtefactCount": 3,
            "roleAssignmentCount": 0,
            "roleCount": 0,
            "familyCounts": {},
            "sourceCounts": {},
            "targetKindCounts": {},
            "confidence": null,
            "primaryRoles": [],
            "graphContextAvailable": true
        })
    );
    assert_eq!(
        stage.expand_hint.as_ref().unwrap()["template"],
        "bitloops devql query '{ selectArtefacts(by: <selector>) { architectureRoles { overview items(first: 20) { role { canonicalKey displayName family description } target { path symbolFqn canonicalKind } confidence source status } } architectureGraphContext { overview nodes(first: 20) { id kind label path } edges(first: 20) { id kind fromNodeId toNodeId } } } }'"
    );
}

#[test]
fn architecture_role_assignment_item_preserves_role_and_target_details() {
    let assignment = architecture_role_assignment(ArchitectureRoleAssignmentFixture {
        assignment_id: "assignment-1",
        role_id: "role-http",
        canonical_key: "http_api_surface",
        display_name: "HTTP API Surface",
        family: "entrypoint",
        artefact_id: Some("artefact-create"),
        symbol_id: Some("symbol-create"),
        symbol_fqn: Some("src/api/users_handler.rs::create_user_http_handler"),
    });

    let item = super::support::architecture_role_assignment_item(&assignment);

    assert_eq!(item.assignment_id, "assignment-1");
    assert_eq!(item.role.canonical_key, "http_api_surface");
    assert_eq!(item.role.display_name, "HTTP API Surface");
    assert_eq!(item.target.path, "src/api/users_handler.rs");
    assert_eq!(
        item.target.symbol_fqn.as_deref(),
        Some("src/api/users_handler.rs::create_user_http_handler")
    );
    assert_eq!(item.confidence, 1.0);
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
