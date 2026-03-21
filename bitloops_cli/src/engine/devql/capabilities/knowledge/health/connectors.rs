use crate::engine::devql::capability_host::{CapabilityHealthContext, CapabilityHealthResult};

pub fn check_knowledge_connectors(ctx: &dyn CapabilityHealthContext) -> CapabilityHealthResult {
    let config = match ctx.config_view("knowledge") {
        Ok(view) => view,
        Err(err) => {
            return CapabilityHealthResult::failed(
                "knowledge connector config unavailable",
                err.to_string(),
            );
        }
    };

    let knowledge_config = config
        .scoped()
        .or_else(|| config.root().get("knowledge"))
        .unwrap_or(config.root());

    let has_provider_config = knowledge_config
        .get("providers")
        .and_then(serde_json::Value::as_object)
        .map(|providers| {
            ["github", "jira", "confluence", "atlassian"]
                .iter()
                .any(|key| providers.get(*key).is_some_and(|value| !value.is_null()))
        })
        .unwrap_or(false);

    if !has_provider_config {
        return CapabilityHealthResult::failed(
            "knowledge connectors not configured",
            "no knowledge provider configuration found".to_string(),
        );
    }

    let _ = ctx.connectors();
    CapabilityHealthResult::ok("knowledge connectors configured")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ProviderConfig;
    use crate::engine::adapters::connectors::{
        ConnectorContext, ConnectorRegistry, KnowledgeConnectorAdapter,
    };
    use crate::engine::devql::RepoIdentity;
    use crate::engine::devql::capabilities::knowledge::ParsedKnowledgeUrl;
    use crate::engine::devql::capability_host::config_view::CapabilityConfigView;
    use crate::engine::devql::capability_host::gateways::StoreHealthGateway;
    use anyhow::Result;
    use serde_json::json;
    use std::path::{Path, PathBuf};

    struct DummyConnectors {
        provider_config: ProviderConfig,
    }

    impl ConnectorContext for DummyConnectors {
        fn provider_config(&self) -> &ProviderConfig {
            &self.provider_config
        }
    }

    impl ConnectorRegistry for DummyConnectors {
        fn knowledge_adapter_for(
            &self,
            _parsed: &ParsedKnowledgeUrl,
        ) -> Result<&dyn KnowledgeConnectorAdapter> {
            unreachable!("connector adapter lookup is not used by health checks")
        }
    }

    struct DummyStores;

    impl StoreHealthGateway for DummyStores {
        fn check_relational(&self) -> Result<()> {
            Ok(())
        }

        fn check_documents(&self) -> Result<()> {
            Ok(())
        }

        fn check_blobs(&self) -> Result<()> {
            Ok(())
        }
    }

    struct TestHealthContext {
        repo: RepoIdentity,
        repo_root: PathBuf,
        config_root: serde_json::Value,
        connectors: DummyConnectors,
        stores: DummyStores,
    }

    impl CapabilityHealthContext for TestHealthContext {
        fn repo(&self) -> &RepoIdentity {
            &self.repo
        }

        fn repo_root(&self) -> &Path {
            self.repo_root.as_path()
        }

        fn config_view(&self, capability_id: &str) -> Result<CapabilityConfigView> {
            Ok(CapabilityConfigView::new(
                capability_id.to_string(),
                self.config_root.clone(),
            ))
        }

        fn connectors(&self) -> &dyn ConnectorRegistry {
            &self.connectors
        }

        fn stores(&self) -> &dyn StoreHealthGateway {
            &self.stores
        }
    }

    fn test_repo() -> RepoIdentity {
        RepoIdentity {
            provider: "local".to_string(),
            organization: "bitloops".to_string(),
            name: "bitloops-cli".to_string(),
            identity: "local/bitloops/bitloops-cli".to_string(),
            repo_id: "repo-1".to_string(),
        }
    }

    fn make_context(config_root: serde_json::Value) -> TestHealthContext {
        TestHealthContext {
            repo: test_repo(),
            repo_root: PathBuf::from("."),
            config_root,
            connectors: DummyConnectors {
                provider_config: ProviderConfig::default(),
            },
            stores: DummyStores,
        }
    }

    #[test]
    fn check_connectors_fails_when_no_provider_configuration_exists() {
        let ctx = make_context(json!({
            "knowledge": {
                "providers": {}
            }
        }));

        let result = check_knowledge_connectors(&ctx);
        assert!(!result.is_healthy());
        assert_eq!(result.message, "knowledge connectors not configured");
    }

    #[test]
    fn check_connectors_passes_with_top_level_knowledge_provider_configuration() {
        let ctx = make_context(json!({
            "knowledge": {
                "providers": {
                    "atlassian": { "site_url": "https://bitloops.atlassian.net" }
                }
            }
        }));

        let result = check_knowledge_connectors(&ctx);
        assert!(result.is_healthy());
    }

    #[test]
    fn check_connectors_passes_with_capability_scoped_configuration_shape() {
        let ctx = make_context(json!({
            "providers": {
                "github": { "configured": true }
            }
        }));

        let result = check_knowledge_connectors(&ctx);
        assert!(result.is_healthy());
    }
}
