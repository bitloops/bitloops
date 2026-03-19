use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;

use serde_json::Value;

use crate::engine::devql::RepoIdentity;
use crate::store_config::{ProviderConfig, StoreBackendConfig};

use super::storage::{
    BlobKnowledgePayloadStore, DuckdbKnowledgeDocumentStore, SqliteKnowledgeRelationalStore,
};

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KnowledgeProvider {
    Github,
    Jira,
    Confluence,
}

impl KnowledgeProvider {
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Github => "github",
            Self::Jira => "jira",
            Self::Confluence => "confluence",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KnowledgeSourceKind {
    GithubIssue,
    GithubPullRequest,
    JiraIssue,
    ConfluencePage,
}

impl KnowledgeSourceKind {
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::GithubIssue => "github_issue",
            Self::GithubPullRequest => "github_pull_request",
            Self::JiraIssue => "jira_issue",
            Self::ConfluencePage => "confluence_page",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KnowledgeLocator {
    GithubIssue {
        owner: String,
        repo: String,
        number: u64,
    },
    GithubPullRequest {
        owner: String,
        repo: String,
        number: u64,
    },
    JiraIssue {
        site: String,
        key: String,
    },
    ConfluencePage {
        site: String,
        page_id: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedKnowledgeUrl {
    pub provider: KnowledgeProvider,
    pub source_kind: KnowledgeSourceKind,
    pub canonical_external_id: String,
    pub canonical_url: String,
    pub provider_site: Option<String>,
    pub locator: KnowledgeLocator,
}

#[derive(Debug, Clone, PartialEq)]
pub struct KnowledgePayloadData {
    pub raw_payload: Value,
    pub body_text: Option<String>,
    pub body_html: Option<String>,
    pub body_adf: Option<Value>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FetchedKnowledgeDocument {
    pub external_id: String,
    pub title: String,
    pub web_url: String,
    pub state: Option<String>,
    pub author: Option<String>,
    pub updated_at: Option<String>,
    pub body_preview: Option<String>,
    pub normalized_fields: Value,
    pub payload: KnowledgePayloadData,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IngestKnowledgeRequest {
    pub url: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KnowledgeAssociationTarget {
    Commit { sha: String },
    KnowledgeItem { knowledge_item_id: String },
    Checkpoint { checkpoint_id: String },
    Artefact { artefact_id: String },
}

impl KnowledgeAssociationTarget {
    pub fn target_type(&self) -> &'static str {
        match self {
            Self::Commit { .. } => "commit",
            Self::KnowledgeItem { .. } => "knowledge_item",
            Self::Checkpoint { .. } => "checkpoint",
            Self::Artefact { .. } => "artefact",
        }
    }

    pub fn target_id(&self) -> &str {
        match self {
            Self::Commit { sha } => sha.as_str(),
            Self::KnowledgeItem { knowledge_item_id } => knowledge_item_id.as_str(),
            Self::Checkpoint { checkpoint_id } => checkpoint_id.as_str(),
            Self::Artefact { artefact_id } => artefact_id.as_str(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssociateKnowledgeRequest {
    pub knowledge_item_id: String,
    pub source_knowledge_item_version_id: String,
    pub target: KnowledgeAssociationTarget,
    pub relation_type: String,
    pub association_method: String,
    pub command: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssociateKnowledgeResult {
    pub relation_assertion_id: String,
    pub target_type: String,
    pub target_id: String,
    pub relation_type: String,
    pub association_method: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KnowledgeItemStatus {
    Created,
    Reused,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KnowledgeVersionStatus {
    Created,
    Reused,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IngestKnowledgeResult {
    pub provider: String,
    pub source_kind: String,
    pub repo_identity: String,
    pub knowledge_item_id: String,
    pub knowledge_item_version_id: String,
    pub item_status: KnowledgeItemStatus,
    pub version_status: KnowledgeVersionStatus,
}

pub struct KnowledgeHostContext {
    pub repo_root: PathBuf,
    pub repo: RepoIdentity,
    pub backends: StoreBackendConfig,
    pub provider_config: ProviderConfig,
    pub relational_store: SqliteKnowledgeRelationalStore,
    pub document_store: DuckdbKnowledgeDocumentStore,
    pub payload_store: BlobKnowledgePayloadStore,
}

fn render_ingest_status(result: &IngestKnowledgeResult) -> &'static str {
    match (&result.item_status, &result.version_status) {
        (KnowledgeItemStatus::Created, KnowledgeVersionStatus::Created) => "new item, new version",
        (KnowledgeItemStatus::Created, KnowledgeVersionStatus::Reused) => {
            "new item, reused version"
        }
        (KnowledgeItemStatus::Reused, KnowledgeVersionStatus::Created) => {
            "reused item, new version"
        }
        (KnowledgeItemStatus::Reused, KnowledgeVersionStatus::Reused) => {
            "reused item, reused version"
        }
    }
}

pub fn format_knowledge_add_result(
    ingest: &IngestKnowledgeResult,
    association: Option<&AssociateKnowledgeResult>,
) -> String {
    let mut lines = vec![
        "Knowledge added".to_string(),
        format!("  provider: {}", ingest.provider),
        format!("  source kind: {}", ingest.source_kind),
        format!("  repository: {}", ingest.repo_identity),
        format!("  knowledge item: {}", ingest.knowledge_item_id),
        format!(
            "  knowledge item version: {}",
            ingest.knowledge_item_version_id
        ),
        format!("  status: {}", render_ingest_status(ingest)),
    ];

    if let Some(association) = association {
        lines.extend([
            "Association created".to_string(),
            format!(
                "  relation assertion: {}",
                association.relation_assertion_id
            ),
            format!(
                "  target: {}:{}",
                association.target_type, association.target_id
            ),
            format!("  relation: {}", association.relation_type),
            format!("  method: {}", association.association_method),
        ]);
    } else {
        lines.push("Association: none".to_string());
    }

    lines.join("\n")
}

pub fn format_knowledge_associate_result(result: &AssociateKnowledgeResult) -> String {
    [
        "Knowledge associated".to_string(),
        format!("  relation assertion: {}", result.relation_assertion_id),
        format!("  target: {}:{}", result.target_type, result.target_id),
        format!("  relation: {}", result.relation_type),
        format!("  method: {}", result.association_method),
    ]
    .join("\n")
}
