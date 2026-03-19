use anyhow::{Result, bail};

use crate::engine::devql::capabilities::knowledge::ParsedKnowledgeUrl;
use crate::store_config::ProviderConfig;

use super::confluence::ConfluenceKnowledgeAdapter;
use super::github::GitHubKnowledgeAdapter;
use super::jira::JiraKnowledgeAdapter;
use super::types::{ConnectorContext, KnowledgeConnectorAdapter};

pub trait ConnectorRegistry: ConnectorContext + Send + Sync {
    fn knowledge_adapter_for(
        &self,
        parsed: &ParsedKnowledgeUrl,
    ) -> Result<&dyn KnowledgeConnectorAdapter>;
}

pub struct BuiltinConnectorRegistry {
    provider_config: ProviderConfig,
    github: GitHubKnowledgeAdapter,
    jira: JiraKnowledgeAdapter,
    confluence: ConfluenceKnowledgeAdapter,
}

impl BuiltinConnectorRegistry {
    pub fn new(provider_config: ProviderConfig) -> Result<Self> {
        Ok(Self {
            provider_config,
            github: GitHubKnowledgeAdapter::new()?,
            jira: JiraKnowledgeAdapter::new()?,
            confluence: ConfluenceKnowledgeAdapter::new()?,
        })
    }
}

impl ConnectorContext for BuiltinConnectorRegistry {
    fn provider_config(&self) -> &ProviderConfig {
        &self.provider_config
    }
}

impl ConnectorRegistry for BuiltinConnectorRegistry {
    fn knowledge_adapter_for(
        &self,
        parsed: &ParsedKnowledgeUrl,
    ) -> Result<&dyn KnowledgeConnectorAdapter> {
        let adapter: &dyn KnowledgeConnectorAdapter = match parsed.provider.as_str() {
            "github" => &self.github,
            "jira" => &self.jira,
            "confluence" => &self.confluence,
            other => bail!("unsupported knowledge provider `{other}`"),
        };

        if adapter.can_handle(parsed) {
            Ok(adapter)
        } else {
            bail!("no connector adapter can handle `{}`", parsed.canonical_external_id)
        }
    }
}
