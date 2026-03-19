pub use crate::engine::adapters::connectors::jira::JiraKnowledgeAdapter as JiraKnowledgeClient;
#[cfg(test)]
pub(crate) use crate::engine::adapters::connectors::jira::build_jira_document;

use anyhow::Result;

use crate::engine::adapters::connectors::jira::JiraKnowledgeAdapter;
use crate::engine::adapters::connectors::KnowledgeConnectorAdapter;
use crate::engine::devql::capabilities::knowledge::{
    BoxFuture, FetchedKnowledgeDocument, KnowledgeHostContext, ParsedKnowledgeUrl,
};

use super::KnowledgeProviderClient;

impl KnowledgeProviderClient for JiraKnowledgeAdapter {
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
