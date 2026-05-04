use anyhow::{Context, Result, anyhow, bail};
use serde_json::json;

use crate::host::capability_host::{
    BoxFuture, CapabilityIngestContext, IngestRequest, IngestResult, IngesterHandler,
    IngesterRegistration,
};

use super::super::descriptor::{
    CONTEXT_GUIDANCE_CAPABILITY_ID, CONTEXT_GUIDANCE_KNOWLEDGE_DISTILLATION_INGESTER_ID,
    CONTEXT_GUIDANCE_TEXT_GENERATION_SLOT,
};
use super::super::distillation::{GuidanceDistiller, KnowledgeGuidanceDistillationInput};
use super::super::storage::PersistGuidanceOutcome;
use super::super::workplane::{
    ContextGuidanceMailboxPayload, context_guidance_work_item_count, enqueue_target_compaction,
};

pub fn build_context_guidance_knowledge_distillation_ingester() -> IngesterRegistration {
    IngesterRegistration::new(
        CONTEXT_GUIDANCE_CAPABILITY_ID,
        CONTEXT_GUIDANCE_KNOWLEDGE_DISTILLATION_INGESTER_ID,
        std::sync::Arc::new(ContextGuidanceKnowledgeDistillationIngester),
    )
}

struct ContextGuidanceKnowledgeDistillationIngester;

impl IngesterHandler for ContextGuidanceKnowledgeDistillationIngester {
    fn ingest<'a>(
        &'a self,
        request: IngestRequest,
        ctx: &'a mut dyn CapabilityIngestContext,
    ) -> BoxFuture<'a, Result<IngestResult>> {
        Box::pin(async move {
            let payload: ContextGuidanceMailboxPayload = request.parse_json()?;
            let ContextGuidanceMailboxPayload::KnowledgeEvidence(knowledge_payload) = &payload
            else {
                bail!("knowledge distillation ingester received incompatible payload");
            };
            let knowledge_payload = knowledge_payload.as_ref();
            let input = KnowledgeGuidanceDistillationInput {
                knowledge_item_id: knowledge_payload.knowledge_item_id.clone(),
                knowledge_item_version_id: knowledge_payload.knowledge_item_version_id.clone(),
                relation_assertion_id: knowledge_payload.relation_assertion_id.clone(),
                provider: knowledge_payload.provider.clone(),
                source_kind: knowledge_payload.source_kind.clone(),
                title: knowledge_payload.title.clone(),
                url: knowledge_payload.url.clone(),
                updated_at: knowledge_payload.updated_at.clone(),
                body_preview: knowledge_payload.body_preview.clone(),
                normalized_fields_json: knowledge_payload.normalized_fields_json.clone(),
                target_paths: knowledge_payload.target_paths.clone(),
                target_symbols: knowledge_payload.target_symbols.clone(),
            };
            let service = match ctx
                .inference()
                .text_generation(CONTEXT_GUIDANCE_TEXT_GENERATION_SLOT)
            {
                Ok(service) => service,
                Err(err) => {
                    log::warn!(
                        "context guidance knowledge distillation skipped: repo_id={} knowledge_item_version_id={} reason={err:#}",
                        knowledge_payload.repo_id,
                        knowledge_payload.knowledge_item_version_id
                    );
                    return Ok(IngestResult::new(
                        json!({
                            "accepted": true,
                            "skipped": true,
                            "reason": "text_generation_unavailable",
                            "work_item_count": context_guidance_work_item_count(&payload)
                        }),
                        "skipped context guidance knowledge distillation because text generation is unavailable",
                    ));
                }
            };
            let service_descriptor = service.descriptor();
            let slot = ctx
                .inference()
                .describe(CONTEXT_GUIDANCE_TEXT_GENERATION_SLOT);
            let output = GuidanceDistiller::new(service)
                .distill_knowledge(&input)
                .context("distilling context guidance knowledge evidence")?;
            let store = ctx
                .context_guidance_store()
                .ok_or_else(|| anyhow!("context guidance store is not available"))?;
            let source_model = slot
                .as_ref()
                .and_then(|slot| slot.model.clone())
                .or(Some(service_descriptor));
            let source_profile = slot.as_ref().map(|slot| slot.profile_name.clone());
            let outcome = store.persist_knowledge_guidance_distillation(
                knowledge_payload.repo_id.as_str(),
                &input,
                &output,
                source_model.as_deref(),
                source_profile.as_deref(),
            )?;
            enqueue_target_compactions(
                knowledge_payload.repo_id.as_str(),
                &outcome,
                ctx.workplane(),
            )?;
            Ok(IngestResult::new(
                json!({
                    "accepted": true,
                    "insertedRun": outcome.inserted_run,
                    "insertedFacts": outcome.inserted_facts,
                    "unchanged": outcome.unchanged,
                    "work_item_count": context_guidance_work_item_count(&payload)
                }),
                "completed context guidance knowledge distillation work",
            ))
        })
    }
}

fn enqueue_target_compactions(
    repo_id: &str,
    outcome: &PersistGuidanceOutcome,
    workplane: Option<&dyn crate::host::capability_host::gateways::CapabilityWorkplaneGateway>,
) -> Result<()> {
    if outcome.inserted_facts == 0 {
        return Ok(());
    }
    let Some(workplane) = workplane else {
        return Ok(());
    };
    for target in &outcome.touched_targets {
        enqueue_target_compaction(
            workplane,
            repo_id,
            target.target_type.as_str(),
            target.target_value.as_str(),
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::path::{Path, PathBuf};
    use std::sync::{Arc, Mutex};

    use anyhow::{Result, bail};
    use serde_json::json;

    use super::*;
    use crate::adapters::connectors::{ConnectorContext, KnowledgeConnectorAdapter};
    use crate::capability_packs::context_guidance::storage::{
        ContextGuidanceRepository, ListSelectedContextGuidanceInput, PersistedGuidanceFact,
        PersistedGuidanceTarget, PersistedGuidanceTargetSummary,
    };
    use crate::capability_packs::context_guidance::types::GuidanceDistillationOutput;
    use crate::capability_packs::context_guidance::workplane::KnowledgeEvidencePayload;
    use crate::capability_packs::knowledge::ParsedKnowledgeUrl;
    use crate::capability_packs::test_harness::storage::BitloopsTestHarnessRepository;
    use crate::config::ProviderConfig;
    use crate::host::capability_host::config_view::CapabilityConfigView;
    use crate::host::capability_host::gateways::{
        BlobPayloadGateway, BlobPayloadRef, CanonicalGraphGateway, CapabilityMailboxStatus,
        CapabilityWorkplaneEnqueueResult, CapabilityWorkplaneGateway, CapabilityWorkplaneJob,
        ConnectorRegistry, ProvenanceBuilder, RelationalGateway,
    };
    use crate::host::devql::RepoIdentity;
    use crate::host::inference::{
        EmbeddingService, InferenceGateway, ResolvedInferenceSlot, TextGenerationService,
    };

    struct FakeTextGenerationService {
        output: String,
    }

    impl TextGenerationService for FakeTextGenerationService {
        fn descriptor(&self) -> String {
            "fake-model".to_string()
        }

        fn complete(&self, _system_prompt: &str, _user_prompt: &str) -> Result<String> {
            Ok(self.output.clone())
        }
    }

    struct FakeInference {
        service: Arc<dyn TextGenerationService>,
    }

    impl InferenceGateway for FakeInference {
        fn text_generation(&self, slot_name: &str) -> Result<Arc<dyn TextGenerationService>> {
            if slot_name == CONTEXT_GUIDANCE_TEXT_GENERATION_SLOT {
                Ok(Arc::clone(&self.service))
            } else {
                bail!("unexpected text generation slot {slot_name}")
            }
        }

        fn embeddings(&self, slot_name: &str) -> Result<Arc<dyn EmbeddingService>> {
            bail!("unexpected embeddings slot {slot_name}")
        }

        fn describe(&self, slot_name: &str) -> Option<ResolvedInferenceSlot> {
            (slot_name == CONTEXT_GUIDANCE_TEXT_GENERATION_SLOT).then(|| ResolvedInferenceSlot {
                capability_id: CONTEXT_GUIDANCE_CAPABILITY_ID.to_string(),
                slot_name: CONTEXT_GUIDANCE_TEXT_GENERATION_SLOT.to_string(),
                profile_name: "guidance-profile".to_string(),
                task: Some(crate::config::InferenceTask::TextGeneration),
                driver: Some("fake".to_string()),
                runtime: None,
                model: Some("fake-model".to_string()),
            })
        }
    }

    #[derive(Default)]
    struct FakeStore {
        persisted: Mutex<Vec<(String, String, usize)>>,
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
            bail!("not used")
        }

        fn persist_knowledge_guidance_distillation(
            &self,
            repo_id: &str,
            input: &KnowledgeGuidanceDistillationInput,
            output: &GuidanceDistillationOutput,
            _source_model: Option<&str>,
            _source_profile: Option<&str>,
        ) -> Result<PersistGuidanceOutcome> {
            assert!(
                output.guidance_facts[0]
                    .applies_to
                    .paths
                    .iter()
                    .all(|path| path == "src/lib.rs")
            );
            self.persisted.lock().expect("persisted").push((
                repo_id.to_string(),
                input.knowledge_item_version_id.clone(),
                output.guidance_facts.len(),
            ));
            Ok(PersistGuidanceOutcome {
                inserted_run: true,
                inserted_facts: output.guidance_facts.len(),
                unchanged: false,
                touched_targets: output
                    .guidance_facts
                    .iter()
                    .flat_map(|fact| {
                        fact.applies_to
                            .paths
                            .iter()
                            .map(|path| PersistedGuidanceTarget {
                                target_type: "path".to_string(),
                                target_value: path.clone(),
                            })
                    })
                    .collect(),
            })
        }

        fn list_selected_context_guidance(
            &self,
            _input: ListSelectedContextGuidanceInput,
        ) -> Result<Vec<PersistedGuidanceFact>> {
            bail!("not used")
        }

        fn list_active_guidance_for_target(
            &self,
            _repo_id: &str,
            _target_type: &str,
            _target_value: &str,
            _limit: usize,
        ) -> Result<Vec<PersistedGuidanceFact>> {
            bail!("not used")
        }

        fn apply_target_compaction(
            &self,
            _repo_id: &str,
            _input: crate::capability_packs::context_guidance::storage::ApplyTargetCompactionInput,
        ) -> Result<crate::capability_packs::context_guidance::storage::ApplyTargetCompactionOutcome>
        {
            bail!("not used")
        }

        fn list_target_summaries(
            &self,
            _repo_id: &str,
            _targets: &[PersistedGuidanceTarget],
        ) -> Result<Vec<PersistedGuidanceTargetSummary>> {
            bail!("not used")
        }

        fn health_check(&self, _repo_id: &str) -> Result<()> {
            Ok(())
        }
    }

    struct EmptyGateway;

    impl BlobPayloadGateway for EmptyGateway {
        fn write_payload(&self, _key: &str, _bytes: &[u8]) -> Result<BlobPayloadRef> {
            bail!("not used")
        }

        fn delete_payload(&self, _payload: &BlobPayloadRef) -> Result<()> {
            bail!("not used")
        }

        fn payload_exists(&self, _storage_path: &str) -> Result<bool> {
            bail!("not used")
        }
    }

    impl CanonicalGraphGateway for EmptyGateway {}

    impl RelationalGateway for EmptyGateway {
        fn resolve_checkpoint_id(&self, _repo_id: &str, _checkpoint_ref: &str) -> Result<String> {
            bail!("not used")
        }

        fn artefact_exists(&self, _repo_id: &str, _artefact_id: &str) -> Result<bool> {
            bail!("not used")
        }

        fn load_repo_id_for_commit(&self, _commit_sha: &str) -> Result<String> {
            bail!("not used")
        }

        fn load_current_production_artefacts(
            &self,
            _repo_id: &str,
        ) -> Result<Vec<crate::models::ProductionArtefact>> {
            bail!("not used")
        }

        fn load_production_artefacts(
            &self,
            _commit_sha: &str,
        ) -> Result<Vec<crate::models::ProductionArtefact>> {
            bail!("not used")
        }

        fn load_artefacts_for_file_lines(
            &self,
            _commit_sha: &str,
            _file_path: &str,
        ) -> Result<Vec<(String, i64, i64)>> {
            bail!("not used")
        }
    }

    impl ConnectorContext for EmptyGateway {
        fn provider_config(&self) -> &ProviderConfig {
            static PROVIDER_CONFIG: std::sync::LazyLock<ProviderConfig> =
                std::sync::LazyLock::new(ProviderConfig::default);
            &PROVIDER_CONFIG
        }
    }

    impl ConnectorRegistry for EmptyGateway {
        fn knowledge_adapter_for(
            &self,
            _parsed: &ParsedKnowledgeUrl,
        ) -> Result<&dyn KnowledgeConnectorAdapter> {
            bail!("not used")
        }
    }

    impl ProvenanceBuilder for EmptyGateway {
        fn build(
            &self,
            capability_id: &str,
            operation: &str,
            details: serde_json::Value,
        ) -> serde_json::Value {
            json!({
                "capability": capability_id,
                "operation": operation,
                "details": details,
            })
        }
    }

    impl CapabilityWorkplaneGateway for EmptyGateway {
        fn enqueue_jobs(
            &self,
            jobs: Vec<CapabilityWorkplaneJob>,
        ) -> Result<CapabilityWorkplaneEnqueueResult> {
            Ok(CapabilityWorkplaneEnqueueResult {
                inserted_jobs: jobs.len() as u64,
                updated_jobs: 0,
            })
        }

        fn mailbox_status(&self) -> Result<BTreeMap<String, CapabilityMailboxStatus>> {
            Ok(BTreeMap::new())
        }
    }

    struct FakeContext<'a> {
        repo: RepoIdentity,
        inference: FakeInference,
        store: &'a dyn ContextGuidanceRepository,
        gateway: EmptyGateway,
        repo_root: PathBuf,
    }

    impl CapabilityIngestContext for FakeContext<'_> {
        fn repo(&self) -> &RepoIdentity {
            &self.repo
        }

        fn repo_root(&self) -> &Path {
            self.repo_root.as_path()
        }

        fn config_view(&self, capability_id: &str) -> Result<CapabilityConfigView> {
            Ok(CapabilityConfigView::new(
                capability_id.to_string(),
                json!({}),
            ))
        }

        fn blob_payloads(&self) -> &dyn BlobPayloadGateway {
            &self.gateway
        }

        fn connectors(&self) -> &dyn ConnectorRegistry {
            &self.gateway
        }

        fn connector_context(&self) -> &dyn ConnectorContext {
            &self.gateway
        }

        fn provenance(&self) -> &dyn ProvenanceBuilder {
            &self.gateway
        }

        fn host_relational(&self) -> &dyn RelationalGateway {
            &self.gateway
        }

        fn inference(&self) -> &dyn InferenceGateway {
            &self.inference
        }

        fn test_harness_store(&self) -> Option<&Mutex<BitloopsTestHarnessRepository>> {
            None
        }

        fn context_guidance_store(&self) -> Option<&dyn ContextGuidanceRepository> {
            Some(self.store)
        }
    }

    #[tokio::test]
    async fn knowledge_ingester_distills_and_persists_knowledge_evidence() -> Result<()> {
        let output = r#"{
            "summary": {
                "intent": "Capture parser boundary knowledge.",
                "outcome": "Stored durable guidance.",
                "decisions": [],
                "rejectedApproaches": [],
                "patterns": [],
                "verification": [],
                "openItems": []
            },
            "guidanceFacts": [{
                "category": "DECISION",
                "kind": "preserve_parser_boundary",
                "guidance": "Keep parser boundary centralized in attr parsing helpers.",
                "evidenceExcerpt": "Keep parser boundary centralized for macro extraction.",
                "appliesTo": { "paths": ["hallucinated.rs"], "symbols": [] },
                "confidence": "HIGH"
            }]
        }"#;
        let store = FakeStore::default();
        let mut ctx = FakeContext {
            repo: RepoIdentity {
                provider: "local".to_string(),
                organization: "bitloops".to_string(),
                name: "repo".to_string(),
                identity: "local/repo".to_string(),
                repo_id: "repo-1".to_string(),
            },
            inference: FakeInference {
                service: Arc::new(FakeTextGenerationService {
                    output: output.to_string(),
                }),
            },
            store: &store,
            gateway: EmptyGateway,
            repo_root: PathBuf::from("."),
        };
        let payload =
            ContextGuidanceMailboxPayload::KnowledgeEvidence(Box::new(KnowledgeEvidencePayload {
                repo_id: "repo-1".to_string(),
                knowledge_item_id: "item-1".to_string(),
                knowledge_item_version_id: "version-1".to_string(),
                relation_assertion_id: Some("relation-1".to_string()),
                provider: "github".to_string(),
                source_kind: "github_issue".to_string(),
                title: Some("Parser boundary".to_string()),
                url: None,
                updated_at: None,
                body_preview: Some(
                    "Keep parser boundary centralized for macro extraction.".to_string(),
                ),
                normalized_fields_json: "{}".to_string(),
                target_paths: vec!["src/lib.rs".to_string()],
                target_symbols: Vec::new(),
                input_hash: "hash-1".to_string(),
            }));

        let ingester = ContextGuidanceKnowledgeDistillationIngester;
        let result = ingester
            .ingest(IngestRequest::new(serde_json::to_value(payload)?), &mut ctx)
            .await?;

        assert_eq!(result.payload["insertedFacts"], json!(1));
        assert_eq!(
            store.persisted.lock().expect("persisted").as_slice(),
            &[("repo-1".to_string(), "version-1".to_string(), 1)]
        );
        Ok(())
    }

    struct CapturingWorkplane {
        jobs: Mutex<Vec<CapabilityWorkplaneJob>>,
    }

    impl CapabilityWorkplaneGateway for CapturingWorkplane {
        fn enqueue_jobs(
            &self,
            jobs: Vec<CapabilityWorkplaneJob>,
        ) -> Result<CapabilityWorkplaneEnqueueResult> {
            let inserted_jobs = jobs.len() as u64;
            self.jobs.lock().expect("jobs").extend(jobs);
            Ok(CapabilityWorkplaneEnqueueResult {
                inserted_jobs,
                updated_jobs: 0,
            })
        }

        fn mailbox_status(&self) -> Result<BTreeMap<String, CapabilityMailboxStatus>> {
            Ok(BTreeMap::new())
        }
    }

    #[test]
    fn inserted_knowledge_guidance_enqueues_target_compaction() -> Result<()> {
        let workplane = CapturingWorkplane {
            jobs: Mutex::new(Vec::new()),
        };
        let outcome = PersistGuidanceOutcome {
            inserted_run: true,
            inserted_facts: 1,
            unchanged: false,
            touched_targets: vec![PersistedGuidanceTarget {
                target_type: "path".to_string(),
                target_value: "src/target.rs".to_string(),
            }],
        };

        enqueue_target_compactions("repo-1", &outcome, Some(&workplane))?;

        let jobs = workplane.jobs.lock().expect("jobs");
        assert_eq!(jobs.len(), 1);
        let queued = &jobs[0];
        assert_eq!(queued.mailbox_name, "context_guidance.target_compaction");
        assert!(queued.payload.to_string().contains("src/target.rs"));
        Ok(())
    }
}
