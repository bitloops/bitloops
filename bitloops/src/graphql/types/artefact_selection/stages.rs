use async_graphql::{ComplexObject, Enum, ID, Result, SimpleObject, types::Json};
use serde_json::Value;

use super::super::{
    Checkpoint, CheckpointFileRelation, DateTimeScalar, DependencyEdge, ExpandHintParameter,
    JsonScalar, TestHarnessTestsResult,
};
use super::support::take_stage_items;

#[derive(Debug, Clone)]
pub(super) struct CheckpointStageData {
    pub(super) summary: Value,
    pub(super) schema: Option<String>,
    pub(super) items: Vec<Checkpoint>,
}

#[derive(Debug, Clone)]
pub(super) struct CloneStageData {
    pub(super) summary: Value,
    pub(super) expand_hint: Option<CloneExpandHint>,
    pub(super) schema: Option<String>,
    pub(super) items: Vec<super::super::SemanticClone>,
}

#[derive(Debug, Clone)]
pub(super) struct DependencyStageData {
    pub(super) summary: Value,
    pub(super) expand_hint: Option<DependencyExpandHint>,
    pub(super) schema: Option<String>,
    pub(super) items: Vec<DependencyEdge>,
}

#[derive(Debug, Clone)]
pub(super) struct TestsStageData {
    pub(super) summary: Value,
    pub(super) schema: Option<String>,
    pub(super) items: Vec<TestHarnessTestsResult>,
}

#[derive(Debug, Clone)]
pub(super) struct HistoricalContextStageData {
    pub(super) summary: Value,
    pub(super) schema: Option<String>,
    pub(super) items: Vec<HistoricalContextItem>,
}

#[derive(Debug, Clone)]
pub(super) struct ContextGuidanceStageData {
    pub(super) summary: Value,
    pub(super) schema: Option<String>,
    pub(super) items: Vec<ContextGuidanceItem>,
}

#[derive(Debug, Clone)]
pub(super) struct ArchitectureOverviewStageData {
    pub(super) summary: Value,
    pub(super) expand_hint: Option<Value>,
    pub(super) schema: Option<String>,
}

impl ArchitectureOverviewStageData {
    pub(super) fn unavailable(selected_artefact_count: usize, reason: &str) -> Self {
        Self {
            summary: serde_json::json!({
                "available": false,
                "reason": reason,
                "selectedArtefactCount": selected_artefact_count,
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
            }),
            expand_hint: None,
            schema: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Enum)]
pub enum HistoricalEvidenceKind {
    SymbolProvenance,
    FileRelation,
    LineOverlap,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Enum)]
pub enum HistoricalMatchReason {
    SymbolProvenance,
    FileRelation,
    LineOverlap,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Enum)]
pub enum HistoricalMatchStrength {
    High,
    Medium,
    Low,
}

#[derive(Debug, Clone, PartialEq, Eq, SimpleObject)]
pub struct HistoricalToolEvent {
    pub tool_kind: Option<String>,
    pub input_summary: Option<String>,
    pub output_summary: Option<String>,
    pub command: Option<String>,
}

#[derive(Debug, Clone, SimpleObject)]
pub struct HistoricalContextItem {
    pub checkpoint_id: ID,
    pub session_id: String,
    pub turn_id: Option<String>,
    pub agent_type: Option<String>,
    pub model: Option<String>,
    pub event_time: DateTimeScalar,
    pub match_reason: HistoricalMatchReason,
    pub match_strength: HistoricalMatchStrength,
    pub prompt_preview: Option<String>,
    pub turn_summary: Option<String>,
    pub transcript_preview: Option<String>,
    pub files_modified: Vec<String>,
    pub file_relations: Vec<CheckpointFileRelation>,
    pub tool_events: Vec<HistoricalToolEvent>,
    #[graphql(skip)]
    pub(crate) evidence_kinds: Vec<HistoricalMatchReason>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Enum, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ContextGuidanceCategory {
    Decision,
    Constraint,
    Pattern,
    Risk,
    Verification,
    Context,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Enum, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ContextGuidanceConfidence {
    High,
    Medium,
    Low,
}

#[derive(Debug, Clone, PartialEq, SimpleObject, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextGuidanceSource {
    pub source_type: String,
    pub source_id: String,
    pub checkpoint_id: Option<ID>,
    pub session_id: Option<String>,
    pub turn_id: Option<String>,
    pub tool_kind: Option<String>,
    pub knowledge_item_id: Option<ID>,
    pub knowledge_item_version_id: Option<ID>,
    pub relation_assertion_id: Option<ID>,
    pub provider: Option<String>,
    pub source_kind: Option<String>,
    pub title: Option<String>,
    pub url: Option<String>,
    pub excerpt: Option<String>,
}

#[derive(Debug, Clone, SimpleObject, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextGuidanceItem {
    pub id: ID,
    pub category: ContextGuidanceCategory,
    pub kind: String,
    pub label: String,
    pub guidance: String,
    pub evidence_excerpt: String,
    pub confidence: ContextGuidanceConfidence,
    pub relevance_score: f64,
    pub generated_at: Option<DateTimeScalar>,
    pub source_model: Option<String>,
    pub source_count: i32,
    pub sources: Vec<ContextGuidanceSource>,
}

#[derive(Debug, Clone, SimpleObject)]
#[graphql(complex)]
pub struct CheckpointStageResult {
    #[graphql(name = "overview")]
    pub summary: JsonScalar,
    pub schema: Option<String>,
    #[graphql(skip)]
    pub(crate) items: Vec<Checkpoint>,
}

#[derive(Debug, Clone, SimpleObject)]
#[graphql(complex)]
pub struct CloneStageResult {
    #[graphql(name = "overview")]
    pub summary: JsonScalar,
    #[graphql(name = "expandHint")]
    pub expand_hint: Option<CloneExpandHint>,
    pub schema: Option<String>,
    #[graphql(skip)]
    pub(crate) items: Vec<super::super::SemanticClone>,
}

#[derive(Debug, Clone, PartialEq, Eq, SimpleObject)]
#[graphql(complex)]
pub struct CloneExpandHint {
    pub intent: String,
    pub template: String,
    #[graphql(skip)]
    pub parameters: Vec<ExpandHintParameter>,
}

#[derive(Debug, Clone, SimpleObject)]
#[graphql(complex)]
pub struct DependencyStageResult {
    #[graphql(name = "overview")]
    pub summary: JsonScalar,
    #[graphql(name = "expandHint")]
    pub expand_hint: Option<DependencyExpandHint>,
    pub schema: Option<String>,
    #[graphql(skip)]
    pub(crate) items: Vec<DependencyEdge>,
}

#[derive(Debug, Clone, PartialEq, Eq, SimpleObject)]
#[graphql(complex)]
pub struct DependencyExpandHint {
    pub intent: String,
    pub template: String,
    #[graphql(skip)]
    pub parameters: Vec<ExpandHintParameter>,
}

#[derive(Debug, Clone, SimpleObject)]
#[graphql(complex)]
pub struct TestsStageResult {
    #[graphql(name = "overview")]
    pub summary: JsonScalar,
    pub schema: Option<String>,
    #[graphql(skip)]
    pub(crate) items: Vec<TestHarnessTestsResult>,
}

#[derive(Debug, Clone, SimpleObject)]
#[graphql(complex)]
pub struct HistoricalContextStageResult {
    #[graphql(name = "overview")]
    pub summary: JsonScalar,
    pub schema: Option<String>,
    #[graphql(skip)]
    pub(crate) items: Vec<HistoricalContextItem>,
}

#[derive(Debug, Clone, SimpleObject)]
#[graphql(complex)]
pub struct ContextGuidanceStageResult {
    #[graphql(name = "overview")]
    pub summary: JsonScalar,
    pub schema: Option<String>,
    #[graphql(skip)]
    pub(crate) items: Vec<ContextGuidanceItem>,
}

impl From<CheckpointStageData> for CheckpointStageResult {
    fn from(data: CheckpointStageData) -> Self {
        Self {
            summary: Json(data.summary),
            schema: data.schema,
            items: data.items,
        }
    }
}

impl From<CloneStageData> for CloneStageResult {
    fn from(data: CloneStageData) -> Self {
        Self {
            summary: Json(data.summary),
            expand_hint: data.expand_hint,
            schema: data.schema,
            items: data.items,
        }
    }
}

impl From<DependencyStageData> for DependencyStageResult {
    fn from(data: DependencyStageData) -> Self {
        Self {
            summary: Json(data.summary),
            expand_hint: data.expand_hint,
            schema: data.schema,
            items: data.items,
        }
    }
}

impl From<TestsStageData> for TestsStageResult {
    fn from(data: TestsStageData) -> Self {
        Self {
            summary: Json(data.summary),
            schema: data.schema,
            items: data.items,
        }
    }
}

impl From<HistoricalContextStageData> for HistoricalContextStageResult {
    fn from(data: HistoricalContextStageData) -> Self {
        Self {
            summary: Json(data.summary),
            schema: data.schema,
            items: data.items,
        }
    }
}

impl From<ContextGuidanceStageData> for ContextGuidanceStageResult {
    fn from(data: ContextGuidanceStageData) -> Self {
        Self {
            summary: Json(data.summary),
            schema: data.schema,
            items: data.items,
        }
    }
}

#[ComplexObject]
impl CloneExpandHint {
    pub async fn parameters(&self) -> &[ExpandHintParameter] {
        &self.parameters
    }
}

#[ComplexObject]
impl DependencyExpandHint {
    pub async fn parameters(&self) -> &[ExpandHintParameter] {
        &self.parameters
    }
}

#[ComplexObject]
impl CheckpointStageResult {
    async fn items(&self, #[graphql(default = 20)] first: i32) -> Result<Vec<Checkpoint>> {
        take_stage_items(&self.items, first)
    }
}

#[ComplexObject]
impl CloneStageResult {
    async fn items(
        &self,
        #[graphql(default = 20)] first: i32,
    ) -> Result<Vec<super::super::SemanticClone>> {
        take_stage_items(&self.items, first)
    }
}

#[ComplexObject]
impl DependencyStageResult {
    async fn items(&self, #[graphql(default = 20)] first: i32) -> Result<Vec<DependencyEdge>> {
        take_stage_items(&self.items, first)
    }
}

#[ComplexObject]
impl TestsStageResult {
    async fn items(
        &self,
        #[graphql(default = 20)] first: i32,
    ) -> Result<Vec<TestHarnessTestsResult>> {
        take_stage_items(&self.items, first)
    }
}

#[ComplexObject]
impl HistoricalContextStageResult {
    async fn items(
        &self,
        #[graphql(default = 20)] first: i32,
    ) -> Result<Vec<HistoricalContextItem>> {
        take_stage_items(&self.items, first)
    }
}

#[ComplexObject]
impl ContextGuidanceStageResult {
    async fn items(&self, #[graphql(default = 20)] first: i32) -> Result<Vec<ContextGuidanceItem>> {
        take_stage_items(&self.items, first)
    }
}
