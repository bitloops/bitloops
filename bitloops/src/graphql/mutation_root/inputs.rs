use async_graphql::InputObject;

use crate::graphql::types::TaskKind;

#[derive(Debug, Clone, InputObject)]
pub struct AddKnowledgeInput {
    pub url: String,
    pub commit_ref: Option<String>,
}

#[derive(Debug, Clone, InputObject)]
pub struct AssociateKnowledgeInput {
    pub source_ref: String,
    pub target_ref: String,
}

#[derive(Debug, Clone, InputObject)]
pub struct RefreshKnowledgeInput {
    pub knowledge_ref: String,
}

#[derive(Debug, Clone, InputObject)]
pub struct EnqueueSyncTaskInput {
    #[graphql(default = false)]
    pub full: bool,
    #[graphql(default)]
    pub paths: Option<Vec<String>>,
    #[graphql(default = false)]
    pub repair: bool,
    #[graphql(default = false)]
    pub validate: bool,
    #[graphql(default)]
    pub source: Option<String>,
}

#[derive(Debug, Clone, InputObject)]
pub struct EnqueueIngestTaskInput {
    #[graphql(default)]
    pub backfill: Option<i32>,
}

#[derive(Debug, Clone, InputObject)]
pub struct EnqueueTaskInput {
    pub kind: TaskKind,
    #[graphql(default)]
    pub sync: Option<EnqueueSyncTaskInput>,
    #[graphql(default)]
    pub ingest: Option<EnqueueIngestTaskInput>,
}

#[derive(Debug, Clone, InputObject)]
pub struct CodeCityRefreshInput {
    #[graphql(default)]
    pub project_path: Option<String>,
}
