use async_graphql::{ComplexObject, Result, SimpleObject, types::Json};
use serde_json::Value;

use super::super::{
    Checkpoint, DependencyEdge, ExpandHintParameter, JsonScalar, TestHarnessTestsResult,
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
