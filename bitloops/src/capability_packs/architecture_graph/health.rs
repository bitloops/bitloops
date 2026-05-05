use crate::capability_packs::architecture_graph::types::{
    ARCHITECTURE_GRAPH_FACT_SYNTHESIS_SLOT, ARCHITECTURE_GRAPH_ROLE_ADJUDICATION_SLOT,
};
use crate::config::InferenceTask;
use crate::host::capability_host::{
    CapabilityHealthCheck, CapabilityHealthContext, CapabilityHealthResult,
};

pub fn check_architecture_graph_config(
    ctx: &dyn CapabilityHealthContext,
) -> CapabilityHealthResult {
    match ctx.config_view("architecture_graph") {
        Ok(_) => CapabilityHealthResult::ok("architecture graph config available"),
        Err(err) => {
            CapabilityHealthResult::failed("architecture graph config unavailable", err.to_string())
        }
    }
}

pub fn check_architecture_graph_storage(
    ctx: &dyn CapabilityHealthContext,
) -> CapabilityHealthResult {
    if let Err(err) = ctx.stores().check_relational() {
        return CapabilityHealthResult::failed(
            "architecture graph relational store unavailable",
            err.to_string(),
        );
    }
    CapabilityHealthResult::ok("architecture graph relational store healthy")
}

pub fn check_architecture_graph_inference(
    ctx: &dyn CapabilityHealthContext,
) -> CapabilityHealthResult {
    for (slot_name, label) in [
        (ARCHITECTURE_GRAPH_FACT_SYNTHESIS_SLOT, "fact_synthesis"),
        (
            ARCHITECTURE_GRAPH_ROLE_ADJUDICATION_SLOT,
            "role_adjudication",
        ),
    ] {
        if !ctx.inference().has_slot(slot_name) {
            continue;
        }
        let Some(slot) = ctx.inference().describe(slot_name) else {
            return CapabilityHealthResult::failed(
                "architecture_graph.inference",
                format!("{label} slot is unresolved"),
            );
        };
        if slot.task != Some(InferenceTask::StructuredGeneration) {
            return CapabilityHealthResult::failed(
                "architecture_graph.inference",
                format!(
                    "{label} slot points to profile `{}` with task `{}` instead of `structured_generation`",
                    slot.profile_name,
                    slot.task
                        .map(|task| task.to_string())
                        .unwrap_or_else(|| "<unknown>".to_string())
                ),
            );
        }

        if let Err(err) = ctx.inference().structured_generation(slot_name) {
            return CapabilityHealthResult::failed(
                "architecture_graph.inference",
                format!("{label} slot failed: {err:#}"),
            );
        }
    }
    if !ctx
        .inference()
        .has_slot(ARCHITECTURE_GRAPH_FACT_SYNTHESIS_SLOT)
        && !ctx
            .inference()
            .has_slot(ARCHITECTURE_GRAPH_ROLE_ADJUDICATION_SLOT)
    {
        return CapabilityHealthResult::ok("architecture graph inference slots not configured");
    }

    let descriptor = ctx
        .inference()
        .structured_generation(ARCHITECTURE_GRAPH_FACT_SYNTHESIS_SLOT)
        .or_else(|_| {
            ctx.inference()
                .structured_generation(ARCHITECTURE_GRAPH_ROLE_ADJUDICATION_SLOT)
        })
        .map(|service| service.descriptor())
        .unwrap_or_else(|_| "structured_generation".to_string());

    CapabilityHealthResult::ok(format!(
        "architecture graph inference slots ready ({descriptor})"
    ))
}

pub static ARCHITECTURE_GRAPH_HEALTH_CHECKS: &[CapabilityHealthCheck] = &[
    CapabilityHealthCheck {
        name: "architecture_graph.config",
        run: check_architecture_graph_config,
    },
    CapabilityHealthCheck {
        name: "architecture_graph.storage",
        run: check_architecture_graph_storage,
    },
    CapabilityHealthCheck {
        name: "architecture_graph.inference",
        run: check_architecture_graph_inference,
    },
];

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::path::Path;
    use std::sync::Arc;

    use anyhow::{Result, bail};

    use super::*;
    use crate::adapters::connectors::{ConnectorContext, KnowledgeConnectorAdapter};
    use crate::capability_packs::knowledge::ParsedKnowledgeUrl;
    use crate::config::ProviderConfig;
    use crate::host::capability_host::config_view::CapabilityConfigView;
    use crate::host::capability_host::gateways::{ConnectorRegistry, StoreHealthGateway};
    use crate::host::devql::RepoIdentity;
    use crate::host::inference::{
        EmbeddingService, InferenceGateway, ResolvedInferenceSlot, StructuredGenerationRequest,
        StructuredGenerationService, TextGenerationService,
    };

    struct FakeHealthContext<'a> {
        inference: &'a dyn InferenceGateway,
        repo: RepoIdentity,
        connectors: EmptyConnectors,
        stores: EmptyStores,
    }

    impl<'a> FakeHealthContext<'a> {
        fn new(inference: &'a dyn InferenceGateway) -> Self {
            Self {
                inference,
                repo: RepoIdentity {
                    provider: "local".to_string(),
                    organization: "bitloops".to_string(),
                    name: "repo".to_string(),
                    identity: "local/repo".to_string(),
                    repo_id: "repo-1".to_string(),
                },
                connectors: EmptyConnectors {
                    provider_config: ProviderConfig::default(),
                },
                stores: EmptyStores,
            }
        }
    }

    impl CapabilityHealthContext for FakeHealthContext<'_> {
        fn repo(&self) -> &RepoIdentity {
            &self.repo
        }

        fn repo_root(&self) -> &Path {
            Path::new(".")
        }

        fn config_view(&self, capability_id: &str) -> Result<CapabilityConfigView> {
            Ok(CapabilityConfigView::new(
                capability_id.to_string(),
                serde_json::json!({}),
            ))
        }

        fn connectors(&self) -> &dyn ConnectorRegistry {
            &self.connectors
        }

        fn stores(&self) -> &dyn StoreHealthGateway {
            &self.stores
        }

        fn inference(&self) -> &dyn InferenceGateway {
            self.inference
        }
    }

    struct EmptyConnectors {
        provider_config: ProviderConfig,
    }

    impl ConnectorContext for EmptyConnectors {
        fn provider_config(&self) -> &ProviderConfig {
            &self.provider_config
        }
    }

    impl ConnectorRegistry for EmptyConnectors {
        fn knowledge_adapter_for(
            &self,
            _parsed: &ParsedKnowledgeUrl,
        ) -> Result<&dyn KnowledgeConnectorAdapter> {
            bail!("connector lookup is not used in architecture health tests")
        }
    }

    struct EmptyStores;

    impl StoreHealthGateway for EmptyStores {
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

    struct FakeInferenceGateway {
        slots: BTreeMap<String, ResolvedInferenceSlot>,
        service_ok: bool,
    }

    impl FakeInferenceGateway {
        fn with_slot(task: Option<InferenceTask>, service_ok: bool) -> Self {
            let mut slots = BTreeMap::new();
            slots.insert(
                ARCHITECTURE_GRAPH_FACT_SYNTHESIS_SLOT.to_string(),
                ResolvedInferenceSlot {
                    capability_id: "architecture_graph".to_string(),
                    slot_name: ARCHITECTURE_GRAPH_FACT_SYNTHESIS_SLOT.to_string(),
                    profile_name: "local_agent".to_string(),
                    task,
                    driver: Some("codex_exec".to_string()),
                    runtime: Some("codex".to_string()),
                    model: Some("gpt-5.4-mini".to_string()),
                },
            );
            Self { slots, service_ok }
        }
    }

    impl InferenceGateway for FakeInferenceGateway {
        fn embeddings(&self, slot_name: &str) -> Result<Arc<dyn EmbeddingService>> {
            bail!("embedding inference is not available for slot `{slot_name}`")
        }

        fn text_generation(&self, slot_name: &str) -> Result<Arc<dyn TextGenerationService>> {
            bail!("text-generation inference is not available for slot `{slot_name}`")
        }

        fn structured_generation(
            &self,
            slot_name: &str,
        ) -> Result<Arc<dyn StructuredGenerationService>> {
            if self.service_ok && self.slots.contains_key(slot_name) {
                Ok(Arc::new(FakeStructuredGenerationService))
            } else {
                bail!("structured-generation inference is not available for slot `{slot_name}`")
            }
        }

        fn has_slot(&self, slot_name: &str) -> bool {
            self.slots.contains_key(slot_name)
        }

        fn describe(&self, slot_name: &str) -> Option<ResolvedInferenceSlot> {
            self.slots.get(slot_name).cloned()
        }
    }

    struct FakeStructuredGenerationService;

    impl StructuredGenerationService for FakeStructuredGenerationService {
        fn descriptor(&self) -> String {
            "codex:gpt-5.4-mini".to_string()
        }

        fn generate(&self, _request: StructuredGenerationRequest) -> Result<serde_json::Value> {
            Ok(serde_json::json!({ "nodes": [], "edges": [] }))
        }
    }

    #[test]
    fn architecture_inference_health_reports_missing_slot_as_ok() {
        let inference = crate::host::inference::EmptyInferenceGateway;
        let ctx = FakeHealthContext::new(&inference);

        let result = check_architecture_graph_inference(&ctx);

        assert!(result.healthy);
        assert!(result.message.contains("not configured"));
    }

    #[test]
    fn architecture_inference_health_reports_wrong_task() {
        let inference = FakeInferenceGateway::with_slot(Some(InferenceTask::TextGeneration), true);
        let ctx = FakeHealthContext::new(&inference);

        let result = check_architecture_graph_inference(&ctx);

        assert!(!result.healthy);
        assert!(
            result
                .details
                .as_deref()
                .is_some_and(|details| details.contains("structured_generation"))
        );
    }

    #[test]
    fn architecture_inference_health_reports_ready() {
        let inference =
            FakeInferenceGateway::with_slot(Some(InferenceTask::StructuredGeneration), true);
        let ctx = FakeHealthContext::new(&inference);

        let result = check_architecture_graph_inference(&ctx);

        assert!(result.healthy);
        assert!(result.message.contains("ready"));
    }
}
