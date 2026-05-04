use std::future::Future;
use std::pin::Pin;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::discussion::KnowledgeDiscussion;

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParsedKnowledgeUrl {
    pub provider: KnowledgeProvider,
    pub source_kind: KnowledgeSourceKind,
    pub canonical_external_id: String,
    pub canonical_url: String,
    pub provider_site: Option<String>,
    pub locator: KnowledgeLocator,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct KnowledgePayloadEnvelope {
    pub raw_payload: Value,
    pub body_text: Option<String>,
    pub body_html: Option<String>,
    pub body_adf: Option<Value>,
    pub discussion: Option<KnowledgeDiscussion>,
}

pub type KnowledgePayloadData = KnowledgePayloadEnvelope;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FetchedKnowledgeDocument {
    pub external_id: String,
    pub title: String,
    pub web_url: String,
    pub state: Option<String>,
    pub author: Option<String>,
    pub updated_at: Option<String>,
    pub body_preview: Option<String>,
    pub normalized_fields: Value,
    pub payload: KnowledgePayloadEnvelope,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IngestKnowledgeRequest {
    pub url: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum KnowledgeAssociationTarget {
    Commit {
        sha: String,
    },
    KnowledgeItem {
        knowledge_item_id: String,
        target_knowledge_item_version_id: Option<String>,
    },
    Checkpoint {
        checkpoint_id: String,
    },
    Artefact {
        artefact_id: String,
    },
    Path {
        path: String,
    },
    SymbolFqn {
        symbol_fqn: String,
    },
}

impl KnowledgeAssociationTarget {
    pub fn target_type(&self) -> &'static str {
        match self {
            Self::Commit { .. } => "commit",
            Self::KnowledgeItem { .. } => "knowledge_item",
            Self::Checkpoint { .. } => "checkpoint",
            Self::Artefact { .. } => "artefact",
            Self::Path { .. } => "path",
            Self::SymbolFqn { .. } => "symbol_fqn",
        }
    }

    pub fn target_id(&self) -> &str {
        match self {
            Self::Commit { sha } => sha.as_str(),
            Self::KnowledgeItem {
                knowledge_item_id, ..
            } => knowledge_item_id.as_str(),
            Self::Checkpoint { checkpoint_id } => checkpoint_id.as_str(),
            Self::Artefact { artefact_id } => artefact_id.as_str(),
            Self::Path { path } => path.as_str(),
            Self::SymbolFqn { symbol_fqn } => symbol_fqn.as_str(),
        }
    }

    pub fn target_knowledge_item_version_id(&self) -> Option<&str> {
        match self {
            Self::KnowledgeItem {
                target_knowledge_item_version_id,
                ..
            } => target_knowledge_item_version_id.as_deref(),
            Self::Commit { .. }
            | Self::Checkpoint { .. }
            | Self::Artefact { .. }
            | Self::Path { .. }
            | Self::SymbolFqn { .. } => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssociateKnowledgeRequest {
    pub knowledge_item_id: String,
    pub source_knowledge_item_version_id: String,
    pub target: KnowledgeAssociationTarget,
    pub relation_type: String,
    pub association_method: String,
    pub command: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssociateKnowledgeResult {
    pub relation_assertion_id: String,
    pub target_type: String,
    pub target_id: String,
    pub relation_type: String,
    pub association_method: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum KnowledgeItemStatus {
    Created,
    Reused,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum KnowledgeVersionStatus {
    Created,
    Reused,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IngestKnowledgeResult {
    pub provider: String,
    pub source_kind: String,
    pub repo_identity: String,
    pub knowledge_item_id: String,
    pub knowledge_item_version_id: String,
    pub item_status: KnowledgeItemStatus,
    pub version_status: KnowledgeVersionStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RefreshSourceRequest {
    pub knowledge_ref: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RefreshSourceResult {
    pub knowledge_item_id: String,
    pub latest_document_version_id: String,
    pub content_changed: bool,
    pub new_version_created: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListVersionsRequest {
    pub knowledge_ref: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocumentVersionSummary {
    pub knowledge_item_version_id: String,
    pub content_hash: String,
    pub title: String,
    pub updated_at: Option<String>,
    pub created_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListVersionsResult {
    pub knowledge_item_id: String,
    pub versions: Vec<DocumentVersionSummary>,
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

pub fn format_knowledge_refresh_result(result: &RefreshSourceResult) -> String {
    [
        "Knowledge refreshed".to_string(),
        format!("  knowledge item: {}", result.knowledge_item_id),
        format!(
            "  latest knowledge item version: {}",
            result.latest_document_version_id
        ),
        format!("  content changed: {}", result.content_changed),
        format!("  new version created: {}", result.new_version_created),
    ]
    .join("\n")
}

pub fn format_knowledge_versions_result(result: &ListVersionsResult) -> String {
    let mut lines = vec![
        "Knowledge versions".to_string(),
        format!("  knowledge item: {}", result.knowledge_item_id),
        format!("  versions: {}", result.versions.len()),
    ];

    for version in &result.versions {
        lines.push(format!(
            "  - {} | hash={} | title={} | updated_at={} | created_at={}",
            version.knowledge_item_version_id,
            version.content_hash,
            version.title,
            version.updated_at.as_deref().unwrap_or("<none>"),
            version.created_at.as_deref().unwrap_or("<none>"),
        ));
    }

    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::*;

    #[test]
    fn provider_and_source_kind_string_values_are_stable() {
        assert_eq!(KnowledgeProvider::Github.as_str(), "github");
        assert_eq!(KnowledgeProvider::Jira.as_str(), "jira");
        assert_eq!(KnowledgeProvider::Confluence.as_str(), "confluence");

        assert_eq!(KnowledgeSourceKind::GithubIssue.as_str(), "github_issue");
        assert_eq!(
            KnowledgeSourceKind::GithubPullRequest.as_str(),
            "github_pull_request"
        );
        assert_eq!(KnowledgeSourceKind::JiraIssue.as_str(), "jira_issue");
        assert_eq!(
            KnowledgeSourceKind::ConfluencePage.as_str(),
            "confluence_page"
        );
    }

    #[test]
    fn association_target_helpers_return_expected_type_and_id() {
        let commit = KnowledgeAssociationTarget::Commit {
            sha: "abc123".to_string(),
        };
        assert_eq!(commit.target_type(), "commit");
        assert_eq!(commit.target_id(), "abc123");

        let knowledge = KnowledgeAssociationTarget::KnowledgeItem {
            knowledge_item_id: "item-1".to_string(),
            target_knowledge_item_version_id: Some("version-1".to_string()),
        };
        assert_eq!(knowledge.target_type(), "knowledge_item");
        assert_eq!(knowledge.target_id(), "item-1");
        assert_eq!(
            knowledge.target_knowledge_item_version_id(),
            Some("version-1")
        );

        let checkpoint = KnowledgeAssociationTarget::Checkpoint {
            checkpoint_id: "deadbeef1234".to_string(),
        };
        assert_eq!(checkpoint.target_type(), "checkpoint");
        assert_eq!(checkpoint.target_id(), "deadbeef1234");

        let artefact = KnowledgeAssociationTarget::Artefact {
            artefact_id: "artefact-42".to_string(),
        };
        assert_eq!(artefact.target_type(), "artefact");
        assert_eq!(artefact.target_id(), "artefact-42");

        let path = KnowledgeAssociationTarget::Path {
            path: "src/lib.rs".to_string(),
        };
        assert_eq!(path.target_type(), "path");
        assert_eq!(path.target_id(), "src/lib.rs");
        assert_eq!(path.target_knowledge_item_version_id(), None);

        let symbol = KnowledgeAssociationTarget::SymbolFqn {
            symbol_fqn: "crate::lib::run".to_string(),
        };
        assert_eq!(symbol.target_type(), "symbol_fqn");
        assert_eq!(symbol.target_id(), "crate::lib::run");
        assert_eq!(symbol.target_knowledge_item_version_id(), None);
    }

    #[test]
    fn formatters_render_human_output_with_expected_sections() {
        let ingest = IngestKnowledgeResult {
            provider: "github".to_string(),
            source_kind: "github_issue".to_string(),
            repo_identity: "repo://bitloops".to_string(),
            knowledge_item_id: "item-1".to_string(),
            knowledge_item_version_id: "version-1".to_string(),
            item_status: KnowledgeItemStatus::Created,
            version_status: KnowledgeVersionStatus::Created,
        };
        let association = AssociateKnowledgeResult {
            relation_assertion_id: "relation-1".to_string(),
            target_type: "commit".to_string(),
            target_id: "abc123".to_string(),
            relation_type: "associated_with".to_string(),
            association_method: "manual_attachment".to_string(),
        };

        let add_output = format_knowledge_add_result(&ingest, Some(&association));
        assert!(add_output.contains("Knowledge added"));
        assert!(add_output.contains("Association created"));
        assert!(add_output.contains("status: new item, new version"));

        let add_without_association = format_knowledge_add_result(&ingest, None);
        assert!(add_without_association.contains("Association: none"));

        let associate_output = format_knowledge_associate_result(&association);
        assert!(associate_output.contains("Knowledge associated"));
        assert!(associate_output.contains("target: commit:abc123"));

        let refresh_output = format_knowledge_refresh_result(&RefreshSourceResult {
            knowledge_item_id: "item-1".to_string(),
            latest_document_version_id: "version-2".to_string(),
            content_changed: true,
            new_version_created: true,
        });
        assert!(refresh_output.contains("Knowledge refreshed"));
        assert!(refresh_output.contains("content changed: true"));

        let versions_output = format_knowledge_versions_result(&ListVersionsResult {
            knowledge_item_id: "item-1".to_string(),
            versions: vec![
                DocumentVersionSummary {
                    knowledge_item_version_id: "version-2".to_string(),
                    content_hash: "hash-2".to_string(),
                    title: "Issue title".to_string(),
                    updated_at: Some("2026-03-19T10:00:00Z".to_string()),
                    created_at: Some("2026-03-19T10:01:00Z".to_string()),
                },
                DocumentVersionSummary {
                    knowledge_item_version_id: "version-1".to_string(),
                    content_hash: "hash-1".to_string(),
                    title: "Issue title".to_string(),
                    updated_at: None,
                    created_at: None,
                },
            ],
        });
        assert!(versions_output.contains("Knowledge versions"));
        assert!(versions_output.contains("versions: 2"));
        assert!(versions_output.contains("hash=hash-2"));
        assert!(versions_output.contains("updated_at=<none>"));

        let document = FetchedKnowledgeDocument {
            external_id: "github://bitloops/bitloops/issues/42".to_string(),
            title: "Issue title".to_string(),
            web_url: "https://github.com/bitloops/bitloops/issues/42".to_string(),
            state: Some("open".to_string()),
            author: Some("spiros".to_string()),
            updated_at: Some("2026-03-19T10:00:00Z".to_string()),
            body_preview: Some("Issue body".to_string()),
            normalized_fields: json!({ "title": "Issue title" }),
            payload: KnowledgePayloadEnvelope {
                raw_payload: json!({ "title": "Issue title" }),
                body_text: Some("Issue body".to_string()),
                body_html: None,
                body_adf: None,
                discussion: None,
            },
        };
        assert_eq!(
            document
                .payload
                .raw_payload
                .get("title")
                .and_then(Value::as_str),
            Some("Issue title")
        );
    }
}
