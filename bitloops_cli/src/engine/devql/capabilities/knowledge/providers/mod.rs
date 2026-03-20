mod confluence;
mod github;
mod jira;

use anyhow::Result;

use super::types::{BoxFuture, FetchedKnowledgeDocument, KnowledgeHostContext, ParsedKnowledgeUrl};

pub use confluence::ConfluenceKnowledgeClient;
#[cfg(test)]
pub(crate) use confluence::build_confluence_document;
pub use github::GitHubKnowledgeClient;
#[cfg(test)]
pub(crate) use github::build_github_document;
pub use jira::JiraKnowledgeClient;
#[cfg(test)]
pub(crate) use jira::build_jira_document;

pub trait KnowledgeProviderClient: Send + Sync {
    fn fetch<'a>(
        &'a self,
        parsed: &'a ParsedKnowledgeUrl,
        host: &'a KnowledgeHostContext,
    ) -> BoxFuture<'a, Result<FetchedKnowledgeDocument>>;
}
