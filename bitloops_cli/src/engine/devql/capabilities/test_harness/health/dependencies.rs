use serde_json::Value;

use crate::engine::devql::capability_host::{CapabilityHealthContext, CapabilityHealthResult};

use super::super::types::resolve_test_harness_config;

pub fn check_test_harness_dependencies(
    ctx: &dyn CapabilityHealthContext,
) -> CapabilityHealthResult {
    let config = match ctx.config_view("test_harness") {
        Ok(view) => view,
        Err(err) => {
            return CapabilityHealthResult::failed(
                "test harness dependency config unavailable",
                err.to_string(),
            );
        }
    };

    let Some(test_harness_config) = resolve_test_harness_config(&config) else {
        return CapabilityHealthResult::failed(
            "test harness dependencies missing",
            "no `test_harness` namespace found in capability config".to_string(),
        );
    };

    let Some(dependencies) = test_harness_config
        .get("dependencies")
        .and_then(Value::as_object)
    else {
        return CapabilityHealthResult::failed(
            "test harness dependencies missing",
            "expected `test_harness.dependencies` to declare coverage/test-discovery/language support hooks".to_string(),
        );
    };

    let mut missing = Vec::new();
    if !dependency_enabled(dependencies.get("coverage_adapter")) {
        missing.push("coverage_adapter");
    }
    if !dependency_enabled(dependencies.get("test_discovery_adapter")) {
        missing.push("test_discovery_adapter");
    }
    if !dependency_enabled(dependencies.get("language_support")) {
        missing.push("language_support");
    }

    if !missing.is_empty() {
        return CapabilityHealthResult::failed(
            "test harness dependencies not configured",
            format!(
                "missing dependency hooks: {}; Test Harness scaffold remains dependency-gated",
                missing.join(", ")
            ),
        );
    }

    let _ = ctx.connectors();
    CapabilityHealthResult::ok("test harness dependencies configured")
}

fn dependency_enabled(value: Option<&Value>) -> bool {
    match value {
        Some(Value::Bool(enabled)) => *enabled,
        Some(Value::String(entry)) => !entry.trim().is_empty(),
        Some(Value::Object(entries)) => !entries.is_empty(),
        Some(Value::Array(entries)) => !entries.is_empty(),
        Some(Value::Number(_)) => true,
        Some(Value::Null) | None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::connectors::{
        ConnectorContext, ConnectorRegistry, KnowledgeConnectorAdapter,
    };
    use crate::config::ProviderConfig;
    use crate::engine::devql::RepoIdentity;
    use crate::engine::devql::capabilities::knowledge::ParsedKnowledgeUrl;
    use crate::engine::devql::capability_host::CapabilityConfigView;
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
            unreachable!("connector adapter lookup is not used by this health check")
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
        config_root: Value,
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

    fn make_context(config_root: Value) -> TestHealthContext {
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
    fn check_dependencies_fails_when_hooks_are_missing() {
        let ctx = make_context(json!({
            "test_harness": {
                "dependencies": {
                    "coverage_adapter": true
                }
            }
        }));

        let result = check_test_harness_dependencies(&ctx);

        assert!(!result.is_healthy());
        assert_eq!(result.message, "test harness dependencies not configured");
        assert!(
            result
                .details
                .as_deref()
                .expect("details")
                .contains("test_discovery_adapter")
        );
    }

    #[test]
    fn check_dependencies_passes_when_all_hooks_are_declared() {
        let ctx = make_context(json!({
            "test_harness": {
                "dependencies": {
                    "coverage_adapter": true,
                    "test_discovery_adapter": true,
                    "language_support": true
                }
            }
        }));

        let result = check_test_harness_dependencies(&ctx);

        assert!(result.is_healthy());
        assert_eq!(result.message, "test harness dependencies configured");
    }

    #[test]
    fn check_dependencies_accepts_capability_scoped_configuration_shape() {
        let ctx = make_context(json!({
            "dependencies": {
                "coverage_adapter": {"provider": "lcov"},
                "test_discovery_adapter": "ast-index",
                "language_support": true
            }
        }));

        let result = check_test_harness_dependencies(&ctx);

        assert!(result.is_healthy());
    }
}
