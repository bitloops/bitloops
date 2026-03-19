pub use crate::engine::adapters::connectors::github::GitHubKnowledgeAdapter as GitHubKnowledgeClient;
#[cfg(test)]
pub(crate) use crate::engine::adapters::connectors::github::build_github_document;

use anyhow::Result;

use crate::engine::adapters::connectors::github::GitHubKnowledgeAdapter;
use crate::engine::adapters::connectors::KnowledgeConnectorAdapter;
use crate::engine::devql::capabilities::knowledge::{
    BoxFuture, FetchedKnowledgeDocument, KnowledgeHostContext, ParsedKnowledgeUrl,
};

use super::KnowledgeProviderClient;

impl KnowledgeProviderClient for GitHubKnowledgeAdapter {
    fn fetch<'a>(
        &'a self,
        parsed: &'a ParsedKnowledgeUrl,
        host: &'a KnowledgeHostContext,
    ) -> BoxFuture<'a, Result<FetchedKnowledgeDocument>> {
        Box::pin(async move {
            let fetched = KnowledgeConnectorAdapter::fetch(self, parsed, host).await?;
            Ok(fetched.into())
        })
    }
}
