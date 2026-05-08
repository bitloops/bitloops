use async_graphql::Result;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet, HashSet};

use crate::capability_packs::semantic_clones::scoring::{
    RELATION_KIND_DIVERGED_IMPLEMENTATION, RELATION_KIND_EXACT_DUPLICATE,
    RELATION_KIND_SHARED_LOGIC_CANDIDATE, RELATION_KIND_SIMILAR_IMPLEMENTATION,
    RELATION_KIND_WEAK_CLONE_CANDIDATE,
};
use crate::capability_packs::test_harness::types::test_harness_tests_expand_hint_json;
use crate::graphql::{backend_error, bad_user_input_error};

use super::super::{
    Artefact, Checkpoint, CloneSummary, DependencyEdge, DepsDirection, EdgeKind,
    ExpandHintParameter, TestHarnessTestsResult,
};
use super::stages::{
    CheckpointStageData, CloneExpandHint, CloneStageData, ContextGuidanceStageData,
    DependencyExpandHint, DependencyStageData, HistoricalContextItem, HistoricalContextStageData,
    HistoricalMatchReason, TestsStageData,
};

pub(super) fn decode_stage_rows<T: DeserializeOwned>(
    stage: &str,
    rows: Vec<Value>,
) -> Result<Vec<T>> {
    rows.into_iter()
        .map(|row| {
            serde_json::from_value(row).map_err(|err| {
                backend_error(format!(
                    "failed to decode `{stage}` stage payload into typed GraphQL result: {err}"
                ))
            })
        })
        .collect()
}

pub(super) fn build_tests_stage_args(
    min_confidence: Option<f64>,
    linkage_source: Option<String>,
) -> Result<Value> {
    if let Some(min_confidence) = min_confidence
        && !(0.0..=1.0).contains(&min_confidence)
    {
        return Err(bad_user_input_error(
            "`minConfidence` must be between 0 and 1",
        ));
    }

    let mut args = serde_json::Map::new();
    if let Some(min_confidence) = min_confidence {
        args.insert("min_confidence".to_string(), json!(min_confidence));
    }
    if let Some(linkage_source) = linkage_source
        && !linkage_source.trim().is_empty()
    {
        args.insert(
            "linkage_source".to_string(),
            Value::String(linkage_source.trim().to_string()),
        );
    }
    Ok(Value::Object(args))
}

pub(super) fn selection_stage_row_from_artefact(artefact: &Artefact) -> Value {
    json!({
        "artefact_id": artefact.id.as_ref(),
        "symbol_id": &artefact.symbol_id,
        "symbol_fqn": &artefact.symbol_fqn,
        "canonical_kind": artefact.canonical_kind.map(|kind| kind.as_devql_value()),
        "path": &artefact.path,
        "start_line": artefact.start_line,
        "end_line": artefact.end_line,
    })
}

pub(super) fn build_checkpoint_summary(checkpoints: &[Checkpoint]) -> Value {
    let latest_at = checkpoints
        .first()
        .map(|checkpoint| checkpoint.event_time.as_str().to_string());
    let agents = checkpoints
        .iter()
        .filter_map(|checkpoint| checkpoint.agent.as_deref())
        .map(str::trim)
        .filter(|agent| !agent.is_empty())
        .map(str::to_string)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();

    json!({
        "totalCount": checkpoints.len(),
        "latestAt": latest_at,
        "agents": agents,
    })
}

pub(super) fn build_clone_summary(clones: &[super::super::SemanticClone]) -> Value {
    let summary = CloneSummary::from_clones(clones);
    let mut counts = summary
        .groups
        .iter()
        .map(|group| (group.relation_kind.clone(), json!(group.count)))
        .collect::<serde_json::Map<String, Value>>();
    counts.insert("total".to_string(), json!(summary.total_count));

    let mut payload = serde_json::Map::new();
    payload.insert("counts".to_string(), Value::Object(counts));

    if let Some(expand_hint) = build_clone_expand_hint(clones.len()) {
        payload.insert(
            "expandHint".to_string(),
            clone_expand_hint_to_value(&expand_hint),
        );
    }

    Value::Object(payload)
}

pub(super) fn build_clone_expand_hint(match_count: usize) -> Option<CloneExpandHint> {
    (match_count > 0).then(|| CloneExpandHint {
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
}

pub(super) fn build_dependency_summary(
    incoming: &[DependencyEdge],
    outgoing: &[DependencyEdge],
    selected_artefact_count: usize,
    expand_hint: Option<&DependencyExpandHint>,
) -> Value {
    let mut unique_by_id = BTreeMap::<String, EdgeKind>::new();
    for edge in incoming.iter().chain(outgoing.iter()) {
        unique_by_id
            .entry(edge.id.as_ref().to_string())
            .or_insert(edge.edge_kind);
    }

    let mut kind_counts = BTreeMap::<&'static str, usize>::new();
    for kind in unique_by_id.values().copied() {
        *kind_counts.entry(edge_kind_name(kind)).or_default() += 1;
    }

    let mut payload = serde_json::Map::new();
    payload.insert(
        "dependencies".to_string(),
        json!({
            "selectedArtefact": selected_artefact_count,
            "total": unique_by_id.len(),
            "incoming": incoming.len(),
            "outgoing": outgoing.len(),
            "kindCounts": {
                "calls": kind_counts.get("calls").copied().unwrap_or(0),
                "exports": kind_counts.get("exports").copied().unwrap_or(0),
                "extends": kind_counts.get("extends").copied().unwrap_or(0),
                "implements": kind_counts.get("implements").copied().unwrap_or(0),
                "imports": kind_counts.get("imports").copied().unwrap_or(0),
                "references": kind_counts.get("references").copied().unwrap_or(0),
            }
        }),
    );
    if let Some(expand_hint) = expand_hint {
        payload.insert(
            "expandHint".to_string(),
            dependency_expand_hint_to_value(expand_hint),
        );
    }
    Value::Object(payload)
}

pub(super) fn build_dependency_expand_hint(
    dependency_count: usize,
) -> Option<DependencyExpandHint> {
    (dependency_count > 0).then(|| DependencyExpandHint {
        intent: "Use direction to filter dependencies by flow relative to the selected artefacts: incoming maps to IN and outgoing maps to OUT. Use kind to filter dependencies by relationship type: kindCounts.calls maps to CALLS, kindCounts.imports maps to IMPORTS and so on.".to_string(),
        template: "bitloops devql query '{ selectArtefacts(...) { dependencies(direction: IN, kind: CALLS) { items(first: 20) { edgeKind toSymbolRef } } } }'".to_string(),
        parameters: vec![
            ExpandHintParameter {
                name: "direction".to_string(),
                intent: "Choose dependency flow relative to the selected artefacts".to_string(),
                supported_values: vec![
                    deps_direction_name(DepsDirection::In).to_string(),
                    deps_direction_name(DepsDirection::Out).to_string(),
                ],
            },
            ExpandHintParameter {
                name: "kind".to_string(),
                intent: "Choose dependency relationship type".to_string(),
                supported_values: vec![
                    edge_kind_graphql_name(EdgeKind::Calls).to_string(),
                    edge_kind_graphql_name(EdgeKind::Exports).to_string(),
                    edge_kind_graphql_name(EdgeKind::Extends).to_string(),
                    edge_kind_graphql_name(EdgeKind::Implements).to_string(),
                    edge_kind_graphql_name(EdgeKind::Imports).to_string(),
                    edge_kind_graphql_name(EdgeKind::References).to_string(),
                ],
            },
        ],
    })
}

pub(super) fn build_tests_summary(
    rows: &[TestHarnessTestsResult],
    selected_artefact_count: usize,
) -> Value {
    let total_covering_tests = rows
        .iter()
        .map(|row| i64::from(row.summary.total_covering_tests))
        .sum::<i64>();

    json!({
        "selectedArtefactCount": selected_artefact_count,
        "matchedArtefactCount": rows.len(),
        "totalCoveringTests": total_covering_tests,
        "expandHint": test_harness_tests_expand_hint_json(),
    })
}

pub(crate) fn captured_preview(value: &str, max_chars: usize) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() || max_chars == 0 {
        return None;
    }
    Some(trimmed.chars().take(max_chars).collect())
}

pub(super) fn build_historical_context_expand_hint(item_count: usize) -> Option<Value> {
    (item_count > 0).then(|| {
        json!({
            "intent": "Inspect captured historical context for selected artefacts",
            "template": "bitloops devql query '{ selectArtefacts(...) { historicalContext { overview items(first: 20) { checkpointId sessionId turnId promptPreview transcriptPreview toolEvents { toolKind inputSummary outputSummary command } } } } }'",
        })
    })
}

fn historical_context_item_evidence_kinds(
    row: &HistoricalContextItem,
) -> Vec<HistoricalMatchReason> {
    if row.evidence_kinds.is_empty() {
        return vec![row.match_reason];
    }
    let mut evidence_kinds = Vec::new();
    for reason in row.evidence_kinds.iter().copied() {
        if !evidence_kinds.contains(&reason) {
            evidence_kinds.push(reason);
        }
    }
    evidence_kinds
}

pub(super) fn build_historical_context_summary(rows: &[HistoricalContextItem]) -> Value {
    let latest_at = rows.first().map(|row| row.event_time.as_str().to_string());
    let agents = rows
        .iter()
        .filter_map(|row| row.agent_type.as_deref())
        .map(str::trim)
        .filter(|agent| !agent.is_empty())
        .map(str::to_string)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let checkpoint_count = rows
        .iter()
        .map(|row| row.checkpoint_id.as_ref().to_string())
        .collect::<BTreeSet<_>>()
        .len();
    let session_count = rows
        .iter()
        .map(|row| row.session_id.as_str())
        .filter(|value| !value.trim().is_empty())
        .collect::<BTreeSet<_>>()
        .len();
    let turn_count = rows
        .iter()
        .filter_map(|row| row.turn_id.as_deref())
        .filter(|value| !value.trim().is_empty())
        .collect::<BTreeSet<_>>()
        .len();
    let symbol_provenance = rows
        .iter()
        .flat_map(historical_context_item_evidence_kinds)
        .filter(|reason| *reason == HistoricalMatchReason::SymbolProvenance)
        .count();
    let file_relation = rows
        .iter()
        .flat_map(historical_context_item_evidence_kinds)
        .filter(|reason| *reason == HistoricalMatchReason::FileRelation)
        .count();
    let line_overlap = rows
        .iter()
        .flat_map(historical_context_item_evidence_kinds)
        .filter(|reason| *reason == HistoricalMatchReason::LineOverlap)
        .count();

    let mut payload = json!({
        "totalCount": rows.len(),
        "latestAt": latest_at,
        "agents": agents,
        "checkpointCount": checkpoint_count,
        "sessionCount": session_count,
        "turnCount": turn_count,
        "evidenceCounts": {
            "symbolProvenance": symbol_provenance,
            "fileRelation": file_relation,
            "lineOverlap": line_overlap
        }
    });
    if let Some(expand_hint) = build_historical_context_expand_hint(rows.len()) {
        payload
            .as_object_mut()
            .expect("historical context summary is an object")
            .insert("expandHint".to_string(), expand_hint);
    }
    payload
}

pub(super) fn build_selection_summary(
    selected_artefact_count: usize,
    checkpoints: &CheckpointStageData,
    clones: &CloneStageData,
    deps: &DependencyStageData,
    tests: &TestsStageData,
    historical_context: &HistoricalContextStageData,
    context_guidance: &ContextGuidanceStageData,
    http: &Value,
) -> Value {
    json!({
        "selectedArtefactCount": selected_artefact_count,
        "checkpoints": selection_stage_entry(&checkpoints.summary, None, checkpoints.schema.as_deref()),
        "codeMatches": selection_stage_entry(&clones.summary, None, clones.schema.as_deref()),
        "dependencies": selection_stage_entry(
            &deps.summary,
            deps.expand_hint.as_ref(),
            deps.schema.as_deref(),
        ),
        "tests": selection_stage_entry(&tests.summary, None, tests.schema.as_deref()),
        "historicalContext": selection_stage_entry(
            &historical_context.summary,
            None,
            historical_context.schema.as_deref(),
        ),
        "contextGuidance": selection_stage_entry(
            &context_guidance.summary,
            None,
            context_guidance.schema.as_deref(),
        ),
        "http": http,
    })
}

pub(super) fn dedup_strings<'a>(values: impl Iterator<Item = &'a str>) -> Vec<String> {
    values
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

pub(super) fn dedup_dependency_edges(edges: Vec<DependencyEdge>) -> Vec<DependencyEdge> {
    let mut seen = HashSet::<String>::new();
    let mut deduped = Vec::new();
    for edge in edges {
        if seen.insert(edge.id.as_ref().to_string()) {
            deduped.push(edge);
        }
    }
    deduped
}

pub(super) fn saturating_i32(value: usize) -> i32 {
    i32::try_from(value).unwrap_or(i32::MAX)
}

pub(super) fn take_stage_items<T: Clone>(items: &[T], first: i32) -> Result<Vec<T>> {
    if first <= 0 {
        return Err(bad_user_input_error("`first` must be greater than 0"));
    }
    Ok(items.iter().take(first as usize).cloned().collect())
}

fn selection_stage_entry(
    summary: &Value,
    expand_hint: Option<&DependencyExpandHint>,
    schema: Option<&str>,
) -> Value {
    let mut entry = serde_json::Map::new();
    entry.insert("overview".to_string(), summary.clone());
    if let Some(expand_hint) = expand_hint {
        entry.insert(
            "expandHint".to_string(),
            dependency_expand_hint_to_value(expand_hint),
        );
    }
    entry.insert("schema".to_string(), json!(schema));
    Value::Object(entry)
}

fn dependency_expand_hint_to_value(expand_hint: &DependencyExpandHint) -> Value {
    json!({
        "intent": expand_hint.intent.as_str(),
        "template": expand_hint.template.as_str(),
        "parameters": expand_hint
            .parameters
            .iter()
            .map(expand_hint_parameter_to_value)
            .collect::<Vec<_>>(),
    })
}

fn clone_expand_hint_to_value(expand_hint: &CloneExpandHint) -> Value {
    json!({
        "intent": expand_hint.intent.as_str(),
        "template": expand_hint.template.as_str(),
        "parameters": expand_hint
            .parameters
            .iter()
            .map(expand_hint_parameter_to_value)
            .collect::<Vec<_>>(),
    })
}

fn expand_hint_parameter_to_value(parameter: &ExpandHintParameter) -> Value {
    json!({
        "name": parameter.name.as_str(),
        "intent": parameter.intent.as_str(),
        "supportedValues": parameter.supported_values,
    })
}

fn edge_kind_name(kind: EdgeKind) -> &'static str {
    match kind {
        EdgeKind::Imports => "imports",
        EdgeKind::Calls => "calls",
        EdgeKind::References => "references",
        EdgeKind::Extends => "extends",
        EdgeKind::Implements => "implements",
        EdgeKind::Exports => "exports",
    }
}

fn edge_kind_graphql_name(kind: EdgeKind) -> &'static str {
    match kind {
        EdgeKind::Imports => "IMPORTS",
        EdgeKind::Calls => "CALLS",
        EdgeKind::References => "REFERENCES",
        EdgeKind::Extends => "EXTENDS",
        EdgeKind::Implements => "IMPLEMENTS",
        EdgeKind::Exports => "EXPORTS",
    }
}

fn deps_direction_name(direction: DepsDirection) -> &'static str {
    match direction {
        DepsDirection::Out => "OUT",
        DepsDirection::In => "IN",
        DepsDirection::Both => "BOTH",
    }
}

pub(super) const CHECKPOINT_STAGE_SCHEMA: &str = r#"type ArtefactSelection {
  checkpoints(agent: String, since: DateTime): CheckpointStageResult!
}

type CheckpointStageResult {
  overview: JSON!
  schema: String
  items(first: Int! = 20): [Checkpoint!]!
}

type Checkpoint {
  id: ID!
  sessionId: String!
  commitSha: String
  branch: String
  agent: String
  eventTime: DateTime!
  strategy: String
  filesTouched: [String!]!
  payload: JSON
  commit: Commit
  fileRelations: [CheckpointFileRelation!]!
}"#;

pub(super) const CLONE_STAGE_SCHEMA: &str = r#"type ArtefactSelection {
  codeMatches(relationKind: String, minScore: Float): CloneStageResult!
}

type CloneStageResult {
  overview: JSON!
  expandHint: CloneExpandHint
  schema: String
  items(first: Int! = 20): [Clone!]!
}

interface ExpandHint {
  intent: String!
  template: String!
  parameters: [ExpandHintParameter!]!
}

type ExpandHintParameter {
  name: String!
  intent: String!
  supportedValues: [String!]!
}

type CloneExpandHint implements ExpandHint {
  intent: String!
  template: String!
  parameters: [ExpandHintParameter!]!
}

type Clone {
  id: ID!
  sourceArtefactId: ID!
  targetArtefactId: ID!
  relationKind: String!
  score: Float!
  metadata: JSON
  sourceArtefact: Artefact!
  targetArtefact: Artefact!
}"#;

pub(super) const DEPENDENCY_STAGE_SCHEMA: &str = r#"type ArtefactSelection {
  dependencies(kind: EdgeKind, direction: DependenciesDirection! = BOTH, includeUnresolved: Boolean! = true): DependencyStageResult!
}

type DependencyStageResult {
  overview: JSON!
  expandHint: DependencyExpandHint
  schema: String
  items(first: Int! = 20): [DependencyEdge!]!
}

interface ExpandHint {
  intent: String!
  template: String!
  parameters: [ExpandHintParameter!]!
}

type ExpandHintParameter {
  name: String!
  intent: String!
  supportedValues: [String!]!
}

type DependencyExpandHint implements ExpandHint {
  intent: String!
  template: String!
  parameters: [ExpandHintParameter!]!
}

type DependencyEdge {
  id: ID!
  edgeKind: EdgeKind!
  fromArtefactId: ID!
  toArtefactId: ID
  toSymbolRef: String
  startLine: Int
  endLine: Int
  metadata: JSON
  fromArtefact: Artefact!
  toArtefact: Artefact
}"#;

pub(super) const TESTS_STAGE_SCHEMA: &str = r#"type ArtefactSelection {
  tests(minConfidence: Float, linkageSource: String): TestsStageResult!
}

type TestsStageResult {
  overview: JSON!
  schema: String
  items(first: Int! = 20): [TestHarnessTestsResult!]!
}

type TestHarnessTestsResult {
  artefact: TestHarnessArtefactRef!
  coveringTests: [TestHarnessCoveringTest!]!
  summary: TestHarnessTestsSummary!
}"#;

pub(super) const HISTORICAL_CONTEXT_STAGE_SCHEMA: &str = r#"type ArtefactSelection {
  historicalContext(agent: String, since: DateTime, evidenceKind: HistoricalEvidenceKind): HistoricalContextStageResult!
}

type HistoricalContextStageResult {
  overview: JSON!
  schema: String
  items(first: Int! = 20): [HistoricalContextItem!]!
}

enum HistoricalEvidenceKind {
  SYMBOL_PROVENANCE
  FILE_RELATION
  LINE_OVERLAP
}

type HistoricalContextItem {
  checkpointId: ID!
  sessionId: String!
  turnId: String
  agentType: String
  model: String
  eventTime: DateTime!
  matchReason: HistoricalMatchReason!
  matchStrength: HistoricalMatchStrength!
  promptPreview: String
  turnSummary: String
  transcriptPreview: String
  filesModified: [String!]!
  fileRelations: [CheckpointFileRelation!]!
  toolEvents: [HistoricalToolEvent!]!
}

enum HistoricalMatchReason {
  SYMBOL_PROVENANCE
  FILE_RELATION
  LINE_OVERLAP
}

enum HistoricalMatchStrength {
  HIGH
  MEDIUM
  LOW
}

type HistoricalToolEvent {
  toolKind: String
  inputSummary: String
  outputSummary: String
  command: String
}"#;

pub(super) const CONTEXT_GUIDANCE_STAGE_SCHEMA: &str = r#"type ArtefactSelection {
  contextGuidance(agent: String, since: DateTime, evidenceKind: HistoricalEvidenceKind, category: ContextGuidanceCategory, kind: String): ContextGuidanceStageResult!
}

type ContextGuidanceStageResult {
  overview: JSON!
  schema: String
  items(first: Int! = 20): [ContextGuidanceItem!]!
}

type ContextGuidanceItem {
  id: ID!
  category: ContextGuidanceCategory!
  kind: String!
  label: String!
  guidance: String!
  evidenceExcerpt: String!
  confidence: ContextGuidanceConfidence!
  relevanceScore: Float!
  generatedAt: DateTime
  sourceModel: String
  sourceCount: Int!
  sources: [ContextGuidanceSource!]!
}

type ContextGuidanceSource {
  sourceType: String!
  sourceId: String!
  checkpointId: ID
  sessionId: String
  turnId: String
  toolKind: String
  knowledgeItemId: ID
  knowledgeItemVersionId: ID
  relationAssertionId: ID
  provider: String
  sourceKind: String
  title: String
  url: String
  excerpt: String
}

enum ContextGuidanceCategory {
  DECISION
  CONSTRAINT
  PATTERN
  RISK
  VERIFICATION
  CONTEXT
}

enum ContextGuidanceConfidence {
  HIGH
  MEDIUM
  LOW
}"#;
