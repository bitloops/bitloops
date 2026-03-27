use async_graphql::types::Json;
use async_graphql::{ComplexObject, Context, Enum, ID, Result, SimpleObject};

use crate::capability_packs::knowledge::KnowledgePayloadEnvelope;
use crate::graphql::{DevqlGraphqlContext, backend_error, loaders::DataLoaders};

use super::{
    DateTimeScalar, JsonScalar, KnowledgeRelationConnection, KnowledgeRelationEdge,
    KnowledgeVersionConnection, KnowledgeVersionEdge, paginate_items,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Enum)]
pub enum KnowledgeProvider {
    Github,
    Jira,
    Confluence,
}

impl KnowledgeProvider {
    pub(crate) const fn as_storage_value(self) -> &'static str {
        match self {
            Self::Github => "github",
            Self::Jira => "jira",
            Self::Confluence => "confluence",
        }
    }

    pub(crate) fn from_storage_value(value: &str) -> std::result::Result<Self, String> {
        match value.trim().to_ascii_lowercase().as_str() {
            "github" => Ok(Self::Github),
            "jira" => Ok(Self::Jira),
            "confluence" => Ok(Self::Confluence),
            other => Err(format!("unknown knowledge provider `{other}`")),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Enum)]
pub enum KnowledgeSourceKind {
    Issue,
    PullRequest,
    JiraIssue,
    ConfluencePage,
}

impl KnowledgeSourceKind {
    pub(crate) fn from_storage_value(value: &str) -> std::result::Result<Self, String> {
        match value.trim().to_ascii_lowercase().as_str() {
            "github_issue" => Ok(Self::Issue),
            "github_pull_request" => Ok(Self::PullRequest),
            "jira_issue" => Ok(Self::JiraIssue),
            "confluence_page" => Ok(Self::ConfluencePage),
            other => Err(format!("unknown knowledge source kind `{other}`")),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Enum)]
pub enum KnowledgeTargetType {
    Commit,
    Checkpoint,
    Artefact,
    Knowledge,
}

impl KnowledgeTargetType {
    pub(crate) fn from_storage_value(value: &str) -> std::result::Result<Self, String> {
        match value.trim().to_ascii_lowercase().as_str() {
            "commit" => Ok(Self::Commit),
            "checkpoint" => Ok(Self::Checkpoint),
            "artefact" => Ok(Self::Artefact),
            "knowledge_item" => Ok(Self::Knowledge),
            other => Err(format!("unknown knowledge target type `{other}`")),
        }
    }
}

#[derive(Debug, Clone, PartialEq, SimpleObject)]
pub struct KnowledgePayload {
    pub body_text: Option<String>,
    pub body_html: Option<String>,
    pub body_adf: Option<JsonScalar>,
    pub discussion: Option<JsonScalar>,
    pub raw_payload: JsonScalar,
}

impl From<KnowledgePayloadEnvelope> for KnowledgePayload {
    fn from(value: KnowledgePayloadEnvelope) -> Self {
        Self {
            body_text: value.body_text,
            body_html: value.body_html,
            body_adf: value.body_adf.map(Json),
            discussion: value
                .discussion
                .and_then(|discussion| serde_json::to_value(discussion).ok())
                .map(Json),
            raw_payload: Json(value.raw_payload),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, SimpleObject)]
#[graphql(complex)]
pub struct KnowledgeItem {
    pub id: ID,
    pub source_id: ID,
    pub provider: KnowledgeProvider,
    pub source_kind: KnowledgeSourceKind,
    pub canonical_external_id: String,
    pub external_url: String,
    #[graphql(skip)]
    pub(crate) latest_knowledge_item_version_id: String,
}

impl KnowledgeItem {
    pub fn cursor(&self) -> String {
        self.id.to_string()
    }

    async fn latest_version_opt(&self, ctx: &Context<'_>) -> Result<Option<KnowledgeVersion>> {
        let versions = ctx
            .data_unchecked::<DataLoaders>()
            .load_knowledge_versions_by_item(self.id.as_ref())
            .await
            .map_err(|err| {
                backend_error(format!(
                    "failed to resolve knowledge versions for {}: {err:#}",
                    self.id.as_ref()
                ))
            })?;

        Ok(versions
            .into_iter()
            .find(|version| version.id.as_ref() == self.latest_knowledge_item_version_id))
    }
}

#[ComplexObject]
impl KnowledgeItem {
    async fn title(&self, ctx: &Context<'_>) -> Result<Option<String>> {
        Ok(self
            .latest_version_opt(ctx)
            .await?
            .map(|version| version.title))
    }

    async fn latest_version(&self, ctx: &Context<'_>) -> Result<KnowledgeVersion> {
        self.latest_version_opt(ctx).await?.ok_or_else(|| {
            backend_error(format!(
                "latest knowledge version {} for item {} was not found",
                self.latest_knowledge_item_version_id,
                self.id.as_ref()
            ))
        })
    }

    async fn versions(
        &self,
        ctx: &Context<'_>,
        #[graphql(default = 10)] first: i32,
        after: Option<String>,
    ) -> Result<KnowledgeVersionConnection> {
        let versions = ctx
            .data_unchecked::<DataLoaders>()
            .load_knowledge_versions_by_item(self.id.as_ref())
            .await
            .map_err(|err| {
                backend_error(format!(
                    "failed to resolve knowledge versions for {}: {err:#}",
                    self.id.as_ref()
                ))
            })?;
        let page = paginate_items(&versions, first, after.as_deref(), |version| {
            version.cursor()
        })?;
        Ok(KnowledgeVersionConnection::new(
            page.items
                .into_iter()
                .map(KnowledgeVersionEdge::new)
                .collect(),
            page.page_info,
            page.total_count,
        ))
    }

    async fn relations(
        &self,
        ctx: &Context<'_>,
        #[graphql(default = 25)] first: i32,
        after: Option<String>,
    ) -> Result<KnowledgeRelationConnection> {
        let relations = ctx
            .data_unchecked::<DevqlGraphqlContext>()
            .list_knowledge_relations(self.id.as_ref())
            .await
            .map_err(|err| {
                backend_error(format!(
                    "failed to resolve knowledge relations for {}: {err:#}",
                    self.id.as_ref()
                ))
            })?;
        let page = paginate_items(&relations, first, after.as_deref(), |relation| {
            relation.cursor()
        })?;
        Ok(KnowledgeRelationConnection::new(
            page.items
                .into_iter()
                .map(KnowledgeRelationEdge::new)
                .collect(),
            page.page_info,
            page.total_count,
        ))
    }
}

#[derive(Debug, Clone, PartialEq, SimpleObject)]
#[graphql(complex)]
pub struct KnowledgeVersion {
    pub id: ID,
    pub knowledge_item_id: ID,
    pub content_hash: String,
    pub title: String,
    pub state: Option<String>,
    pub author: Option<String>,
    pub updated_at: Option<DateTimeScalar>,
    pub body_preview: Option<String>,
    pub normalized_fields: JsonScalar,
    pub provenance: JsonScalar,
    pub created_at: DateTimeScalar,
    #[graphql(skip)]
    pub(crate) storage_path: String,
}

impl KnowledgeVersion {
    pub fn cursor(&self) -> String {
        self.id.to_string()
    }
}

#[ComplexObject]
impl KnowledgeVersion {
    async fn payload(&self, ctx: &Context<'_>) -> Result<Option<KnowledgePayload>> {
        ctx.data_unchecked::<DevqlGraphqlContext>()
            .load_knowledge_payload(&self.storage_path)
            .map(|payload| payload.map(Into::into))
            .map_err(|err| {
                backend_error(format!(
                    "failed to resolve knowledge payload for {}: {err:#}",
                    self.id.as_ref()
                ))
            })
    }
}

#[derive(Debug, Clone, PartialEq, SimpleObject)]
pub struct KnowledgeRelation {
    pub id: ID,
    pub source_version_id: ID,
    pub target_type: KnowledgeTargetType,
    pub target_id: String,
    pub target_version_id: Option<ID>,
    pub relation_type: String,
    pub association_method: String,
    pub confidence: Option<f64>,
    pub provenance: JsonScalar,
}

impl KnowledgeRelation {
    pub fn cursor(&self) -> String {
        self.id.to_string()
    }
}
