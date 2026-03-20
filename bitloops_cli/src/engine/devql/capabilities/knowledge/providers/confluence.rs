pub use crate::engine::adapters::connectors::confluence::ConfluenceKnowledgeAdapter as ConfluenceKnowledgeClient;
#[cfg(test)]
pub(crate) use crate::engine::adapters::connectors::confluence::build_confluence_document;

use anyhow::Result;

use crate::engine::adapters::connectors::confluence::ConfluenceKnowledgeAdapter;
use crate::engine::adapters::connectors::KnowledgeConnectorAdapter;
use crate::engine::devql::capabilities::knowledge::{
    BoxFuture, FetchedKnowledgeDocument, KnowledgeHostContext, ParsedKnowledgeUrl,
};

use super::KnowledgeProviderClient;

impl KnowledgeProviderClient for ConfluenceKnowledgeAdapter {
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
