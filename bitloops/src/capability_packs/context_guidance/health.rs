use crate::capability_packs::context_guidance::descriptor::{
    CONTEXT_GUIDANCE_CAPABILITY_ID, CONTEXT_GUIDANCE_TEXT_GENERATION_SLOT,
};
use crate::config::{ContextGuidanceConfig, InferenceTask};
use crate::host::capability_host::{
    CapabilityHealthCheck, CapabilityHealthContext, CapabilityHealthResult,
};

pub fn check_context_guidance_config(ctx: &dyn CapabilityHealthContext) -> CapabilityHealthResult {
    let Ok(view) = ctx.config_view(CONTEXT_GUIDANCE_CAPABILITY_ID) else {
        return CapabilityHealthResult::failed(
            "context_guidance.config",
            "context guidance config is unavailable",
        );
    };
    let Some(scoped) = view.scoped() else {
        return CapabilityHealthResult::ok("context guidance config defaults active");
    };
    match serde_json::from_value::<ContextGuidanceConfig>(scoped.clone()) {
        Ok(_) => CapabilityHealthResult::ok("context guidance config valid"),
        Err(err) => CapabilityHealthResult::failed("context_guidance.config", err.to_string()),
    }
}

pub fn check_context_guidance_storage(ctx: &dyn CapabilityHealthContext) -> CapabilityHealthResult {
    let Some(store) = ctx.context_guidance_store() else {
        return CapabilityHealthResult::failed(
            "context_guidance.storage",
            "context guidance store is not available",
        );
    };
    match store.health_check(ctx.repo().repo_id.as_str()) {
        Ok(()) => CapabilityHealthResult::ok("context guidance storage ready"),
        Err(err) => CapabilityHealthResult::failed("context_guidance.storage", err.to_string()),
    }
}

pub fn check_context_guidance_inference(
    ctx: &dyn CapabilityHealthContext,
) -> CapabilityHealthResult {
    let slot_name = CONTEXT_GUIDANCE_TEXT_GENERATION_SLOT;
    if !ctx.inference().has_slot(slot_name) {
        return CapabilityHealthResult::ok("context guidance guidance_generation not configured");
    }
    let Some(slot) = ctx.inference().describe(slot_name) else {
        return CapabilityHealthResult::failed(
            "context_guidance.inference",
            "guidance_generation slot is unresolved",
        );
    };
    if slot.task != Some(InferenceTask::TextGeneration) {
        return CapabilityHealthResult::failed(
            "context_guidance.inference",
            format!(
                "guidance_generation slot points to profile `{}` with task `{}` instead of `text_generation`",
                slot.profile_name,
                slot.task
                    .map(|task| task.to_string())
                    .unwrap_or_else(|| "<unknown>".to_string())
            ),
        );
    }

    match ctx.inference().text_generation(slot_name) {
        Ok(service) => CapabilityHealthResult::ok(format!(
            "context guidance guidance_generation ready ({})",
            service.descriptor()
        )),
        Err(err) => {
            CapabilityHealthResult::failed("context_guidance.inference", format!("{err:#}"))
        }
    }
}

pub static CONTEXT_GUIDANCE_HEALTH_CHECKS: &[CapabilityHealthCheck] = &[
    CapabilityHealthCheck {
        name: "context_guidance.config",
        run: check_context_guidance_config,
    },
    CapabilityHealthCheck {
        name: "context_guidance.storage",
        run: check_context_guidance_storage,
    },
    CapabilityHealthCheck {
        name: "context_guidance.inference",
        run: check_context_guidance_inference,
    },
];

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::path::Path;
    use std::sync::Arc;

    use anyhow::{Result, anyhow, bail};
    use serde_json::json;

    use super::*;
    use crate::adapters::connectors::{ConnectorContext, KnowledgeConnectorAdapter};
    use crate::capability_packs::context_guidance::pack::ContextGuidancePack;
    use crate::capability_packs::context_guidance::storage::{
        ContextGuidanceRepository, ListSelectedContextGuidanceInput, PersistGuidanceOutcome,
        PersistedGuidanceFact,
    };
    use crate::capability_packs::context_guidance::types::GuidanceDistillationOutput;
    use crate::capability_packs::knowledge::ParsedKnowledgeUrl;
    use crate::config::ProviderConfig;
    use crate::host::capability_host::CapabilityPack;
    use crate::host::capability_host::config_view::CapabilityConfigView;
    use crate::host::capability_host::gateways::{ConnectorRegistry, StoreHealthGateway};
    use crate::host::devql::RepoIdentity;
    use crate::host::inference::{
        EmbeddingService, InferenceGateway, ResolvedInferenceSlot, TextGenerationService,
    };

    struct FakeHealthContext<'a> {
        config_root: serde_json::Value,
        inference: &'a dyn InferenceGateway,
        store: Option<&'a dyn ContextGuidanceRepository>,
        repo: RepoIdentity,
        connectors: EmptyConnectors,
        stores: EmptyStores,
    }

    impl<'a> FakeHealthContext<'a> {
        fn new(
            inference: &'a dyn InferenceGateway,
            store: Option<&'a dyn ContextGuidanceRepository>,
        ) -> Self {
            Self {
                config_root: json!({}),
                inference,
                store,
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
                self.config_root.clone(),
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

        fn context_guidance_store(&self) -> Option<&dyn ContextGuidanceRepository> {
            self.store
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
            bail!("connector lookup is not used in context guidance health tests")
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

    struct FakeStore {
        health: Result<()>,
    }

    impl ContextGuidanceRepository for FakeStore {
        fn persist_history_guidance_distillation(
            &self,
            _repo_id: &str,
            _input: &crate::capability_packs::context_guidance::distillation::GuidanceDistillationInput,
            _output: &GuidanceDistillationOutput,
            _source_model: Option<&str>,
            _source_profile: Option<&str>,
        ) -> Result<PersistGuidanceOutcome> {
            bail!("persist is not used in context guidance health tests")
        }

        fn list_selected_context_guidance(
            &self,
            _input: ListSelectedContextGuidanceInput,
        ) -> Result<Vec<PersistedGuidanceFact>> {
            bail!("query is not used in context guidance health tests")
        }

        fn health_check(&self, _repo_id: &str) -> Result<()> {
            match &self.health {
                Ok(()) => Ok(()),
                Err(err) => Err(anyhow!(err.to_string())),
            }
        }
    }

    struct FakeTextGenerationService;

    impl TextGenerationService for FakeTextGenerationService {
        fn descriptor(&self) -> String {
            "fake-text-generation".to_string()
        }

        fn complete(&self, _system_prompt: &str, _user_prompt: &str) -> Result<String> {
            bail!("completion is not used in context guidance health tests")
        }
    }

    struct FakeInferenceGateway {
        slots: BTreeMap<String, ResolvedInferenceSlot>,
        text_generation_ok: bool,
    }

    impl FakeInferenceGateway {
        fn with_slot(task: Option<InferenceTask>, text_generation_ok: bool) -> Self {
            let mut slots = BTreeMap::new();
            slots.insert(
                CONTEXT_GUIDANCE_TEXT_GENERATION_SLOT.to_string(),
                ResolvedInferenceSlot {
                    capability_id: CONTEXT_GUIDANCE_CAPABILITY_ID.to_string(),
                    slot_name: CONTEXT_GUIDANCE_TEXT_GENERATION_SLOT.to_string(),
                    profile_name: "guidance-profile".to_string(),
                    task,
                    driver: Some("fake".to_string()),
                    runtime: Some("fake-runtime".to_string()),
                    model: Some("fake-model".to_string()),
                },
            );
            Self {
                slots,
                text_generation_ok,
            }
        }
    }

    impl InferenceGateway for FakeInferenceGateway {
        fn embeddings(&self, slot_name: &str) -> Result<Arc<dyn EmbeddingService>> {
            bail!("embedding inference is not available for slot `{slot_name}`")
        }

        fn text_generation(&self, slot_name: &str) -> Result<Arc<dyn TextGenerationService>> {
            if self.text_generation_ok && self.slots.contains_key(slot_name) {
                Ok(Arc::new(FakeTextGenerationService))
            } else {
                bail!("text-generation inference is not available for slot `{slot_name}`")
            }
        }

        fn has_slot(&self, slot_name: &str) -> bool {
            self.slots.contains_key(slot_name)
        }

        fn describe(&self, slot_name: &str) -> Option<ResolvedInferenceSlot> {
            self.slots.get(slot_name).cloned()
        }
    }

    #[test]
    fn context_guidance_pack_exposes_three_health_checks() -> Result<()> {
        let pack = ContextGuidancePack::new()?;
        let names = pack
            .health_checks()
            .iter()
            .map(|check| check.name)
            .collect::<Vec<_>>();

        assert_eq!(
            names,
            vec![
                "context_guidance.config",
                "context_guidance.storage",
                "context_guidance.inference"
            ]
        );
        Ok(())
    }

    #[test]
    fn configured_guidance_generation_text_profile_passes() {
        let inference = FakeInferenceGateway::with_slot(Some(InferenceTask::TextGeneration), true);
        let store = FakeStore { health: Ok(()) };
        let ctx = FakeHealthContext::new(&inference, Some(&store));

        let result = check_context_guidance_inference(&ctx);

        assert!(result.healthy);
        assert!(result.message.contains("ready"));
    }

    #[test]
    fn missing_guidance_generation_slot_is_healthy_not_configured() {
        let inference = FakeInferenceGateway {
            slots: BTreeMap::new(),
            text_generation_ok: false,
        };
        let store = FakeStore { health: Ok(()) };
        let ctx = FakeHealthContext::new(&inference, Some(&store));

        let result = check_context_guidance_inference(&ctx);

        assert!(result.healthy);
        assert!(result.message.contains("not configured"));
    }

    #[test]
    fn wrong_task_guidance_generation_slot_is_unhealthy() {
        let inference = FakeInferenceGateway::with_slot(Some(InferenceTask::Embeddings), true);
        let store = FakeStore { health: Ok(()) };
        let ctx = FakeHealthContext::new(&inference, Some(&store));

        let result = check_context_guidance_inference(&ctx);

        assert!(!result.healthy);
        assert!(
            result
                .details
                .as_deref()
                .is_some_and(|details| details.contains("instead of `text_generation`"))
        );
    }

    #[test]
    fn unresolved_guidance_generation_service_is_unhealthy() {
        let inference = FakeInferenceGateway::with_slot(Some(InferenceTask::TextGeneration), false);
        let store = FakeStore { health: Ok(()) };
        let ctx = FakeHealthContext::new(&inference, Some(&store));

        let result = check_context_guidance_inference(&ctx);

        assert!(!result.healthy);
        assert!(
            result.details.as_deref().is_some_and(
                |details| details.contains("text-generation inference is not available")
            )
        );
    }

    #[test]
    fn missing_context_guidance_store_is_unhealthy() {
        let inference = FakeInferenceGateway {
            slots: BTreeMap::new(),
            text_generation_ok: false,
        };
        let ctx = FakeHealthContext::new(&inference, None);

        let result = check_context_guidance_storage(&ctx);

        assert!(!result.healthy);
        assert!(
            result
                .details
                .as_deref()
                .is_some_and(|details| details.contains("store is not available"))
        );
    }
}
