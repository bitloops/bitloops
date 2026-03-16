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
    pub commit: Option<String>,
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
    pub document_version_id: String,
    pub item_status: KnowledgeItemStatus,
    pub version_status: KnowledgeVersionStatus,
    pub relation_assertion_id: Option<String>,
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

pub fn format_knowledge_add_result(result: &IngestKnowledgeResult) -> String {
    let status = match (&result.item_status, &result.version_status) {
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
    };
    let association = result.relation_assertion_id.as_deref().unwrap_or("none");

    [
        "Knowledge added".to_string(),
        format!("  provider: {}", result.provider),
        format!("  source kind: {}", result.source_kind),
        format!("  repository: {}", result.repo_identity),
        format!("  knowledge item: {}", result.knowledge_item_id),
        format!("  document version: {}", result.document_version_id),
        format!("  status: {status}"),
        format!("  association: {association}"),
    ]
    .join("\n")
}
