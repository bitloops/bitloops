use crate::host::capability_host::{
    CapabilityConfigView, CurrentStateConsumer, CurrentStateConsumerContext,
    CurrentStateConsumerFuture, CurrentStateConsumerRequest, CurrentStateConsumerResult,
};

use super::runtime_config::resolve_semantic_clones_config;
use super::types::{SEMANTIC_CLONES_CAPABILITY_ID, SEMANTIC_CLONES_CURRENT_STATE_CONSUMER_ID};
use super::workplane::{
    build_artefact_job, build_current_path_job, build_repo_backfill_job,
    resolve_effective_mailbox_intent,
};
pub struct SemanticClonesCurrentStateConsumer;

impl CurrentStateConsumer for SemanticClonesCurrentStateConsumer {
    fn capability_id(&self) -> &str {
        SEMANTIC_CLONES_CAPABILITY_ID
    }

    fn consumer_id(&self) -> &str {
        SEMANTIC_CLONES_CURRENT_STATE_CONSUMER_ID
    }

    fn reconcile<'a>(
        &'a self,
        request: &'a CurrentStateConsumerRequest,
        context: &'a CurrentStateConsumerContext,
    ) -> CurrentStateConsumerFuture<'a> {
        Box::pin(async move {
            let config = resolve_semantic_clones_config(&CapabilityConfigView::new(
                SEMANTIC_CLONES_CAPABILITY_ID,
                context.config_root.clone(),
            ));
            let intent = resolve_effective_mailbox_intent(context.workplane.as_ref(), &config)?;
            if !intent.has_any_pipeline_intent() {
                super::pipeline::delete_repo_current_symbol_clone_edges(
                    context.storage.as_ref(),
                    &request.repo_id,
                )
                .await?;
                return Ok(CurrentStateConsumerResult::applied(
                    request.to_generation_seq_inclusive,
                ));
            }
            if !intent.has_any_embedding_intent() {
                super::pipeline::delete_repo_current_symbol_clone_edges(
                    context.storage.as_ref(),
                    &request.repo_id,
                )
                .await?;
            }
            let current_paths = request
                .file_upserts
                .iter()
                .map(|file| (file.path.as_str(), file.content_id.as_str()))
                .collect::<std::collections::BTreeSet<_>>();
            let current_path_set = current_paths
                .iter()
                .map(|(path, _content_id)| *path)
                .collect::<std::collections::BTreeSet<_>>();
            let artefact_ids = request
                .artefact_upserts
                .iter()
                .filter(|artefact| !current_path_set.contains(artefact.path.as_str()))
                .map(|artefact| artefact.artefact_id.as_str())
                .collect::<std::collections::BTreeSet<_>>();
            let mut jobs = Vec::new();
            for (path, content_id) in current_paths {
                if intent.summary_refresh_active {
                    jobs.push(build_current_path_job(
                        super::types::SEMANTIC_CLONES_SUMMARY_REFRESH_MAILBOX,
                        path,
                        content_id,
                    )?);
                }
                if intent.code_embeddings_active {
                    jobs.push(build_current_path_job(
                        super::types::SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX,
                        path,
                        content_id,
                    )?);
                }
                if intent.summary_embeddings_active && !intent.summary_refresh_active {
                    jobs.push(build_current_path_job(
                        super::types::SEMANTIC_CLONES_SUMMARY_EMBEDDING_MAILBOX,
                        path,
                        content_id,
                    )?);
                }
            }
            for artefact_id in artefact_ids {
                if intent.summary_refresh_active {
                    jobs.push(build_artefact_job(
                        super::types::SEMANTIC_CLONES_SUMMARY_REFRESH_MAILBOX,
                        artefact_id,
                    )?);
                }
                if intent.code_embeddings_active {
                    jobs.push(build_artefact_job(
                        super::types::SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX,
                        artefact_id,
                    )?);
                }
                if intent.summary_embeddings_active && !intent.summary_refresh_active {
                    jobs.push(build_artefact_job(
                        super::types::SEMANTIC_CLONES_SUMMARY_EMBEDDING_MAILBOX,
                        artefact_id,
                    )?);
                }
            }

            let projection_changed = !request.file_upserts.is_empty()
                || !request.file_removals.is_empty()
                || !request.artefact_upserts.is_empty()
                || !request.artefact_removals.is_empty();
            let has_embedding_follow_up = intent.code_embeddings_active
                || (intent.summary_embeddings_active && !intent.summary_refresh_active);
            if intent.clone_rebuild_active
                && projection_changed
                && (!has_embedding_follow_up || request.artefact_upserts.is_empty())
            {
                jobs.push(build_repo_backfill_job(
                    super::types::SEMANTIC_CLONES_CLONE_REBUILD_MAILBOX,
                )?);
            }
            if !jobs.is_empty() {
                let _ = context.workplane.enqueue_jobs(jobs)?;
            }

            Ok(CurrentStateConsumerResult::applied(
                request.to_generation_seq_inclusive,
            ))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capability_packs::semantic_clones::types::{
        SEMANTIC_CLONES_CLONE_REBUILD_MAILBOX, SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX,
        SEMANTIC_CLONES_SUMMARY_REFRESH_MAILBOX,
    };
    use crate::capability_packs::semantic_clones::workplane::{
        SemanticClonesMailboxPayload, repo_backfill_dedupe_key,
    };
    use crate::host::capability_host::gateways::{
        CapabilityWorkplaneEnqueueResult, CapabilityWorkplaneGateway, DefaultHostServicesGateway,
        EmptyLanguageServicesGateway, RelationalGateway,
    };
    use crate::host::capability_host::{
        ChangedArtefact, ChangedFile, CurrentStateConsumerRequest, ReconcileMode, RemovedFile,
    };
    use crate::host::devql::RelationalStorage;
    use crate::models::ProductionArtefact;
    use anyhow::{Result, bail};
    use serde_json::json;
    use std::collections::BTreeMap;
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex};
    use tempfile::tempdir;

    struct FakeRelationalGateway;

    impl RelationalGateway for FakeRelationalGateway {
        fn resolve_checkpoint_id(&self, _repo_id: &str, _checkpoint_ref: &str) -> Result<String> {
            bail!(
                "resolve_checkpoint_id should not be called in semantic_clones current-state tests"
            )
        }

        fn artefact_exists(&self, _repo_id: &str, _artefact_id: &str) -> Result<bool> {
            Ok(false)
        }

        fn load_repo_id_for_commit(&self, _commit_sha: &str) -> Result<String> {
            bail!(
                "load_repo_id_for_commit should not be called in semantic_clones current-state tests"
            )
        }

        fn load_current_production_artefacts(
            &self,
            _repo_id: &str,
        ) -> Result<Vec<ProductionArtefact>> {
            Ok(Vec::new())
        }

        fn load_production_artefacts(&self, _commit_sha: &str) -> Result<Vec<ProductionArtefact>> {
            Ok(Vec::new())
        }

        fn load_artefacts_for_file_lines(
            &self,
            _commit_sha: &str,
            _file_path: &str,
        ) -> Result<Vec<(String, i64, i64)>> {
            Ok(Vec::new())
        }
    }

    #[derive(Default)]
    struct CapturingWorkplaneGateway {
        jobs: Mutex<Vec<crate::host::capability_host::gateways::CapabilityWorkplaneJob>>,
        statuses: BTreeMap<String, crate::host::capability_host::gateways::CapabilityMailboxStatus>,
    }

    impl CapturingWorkplaneGateway {
        fn take_jobs(&self) -> Vec<crate::host::capability_host::gateways::CapabilityWorkplaneJob> {
            self.jobs.lock().unwrap().clone()
        }
    }

    impl CapabilityWorkplaneGateway for CapturingWorkplaneGateway {
        fn enqueue_jobs(
            &self,
            jobs: Vec<crate::host::capability_host::gateways::CapabilityWorkplaneJob>,
        ) -> Result<CapabilityWorkplaneEnqueueResult> {
            let inserted_jobs = jobs.len() as u64;
            self.jobs.lock().unwrap().extend(jobs);
            Ok(CapabilityWorkplaneEnqueueResult {
                inserted_jobs,
                updated_jobs: 0,
            })
        }

        fn mailbox_status(
            &self,
        ) -> Result<BTreeMap<String, crate::host::capability_host::gateways::CapabilityMailboxStatus>>
        {
            Ok(self.statuses.clone())
        }
    }

    fn configured_semantic_clones(summary_mode: &str) -> serde_json::Value {
        json!({
            "semantic_clones": {
                "summary_mode": summary_mode,
                "embedding_mode": "deterministic",
                "ann_neighbors": 50,
                "enrichment_workers": 4,
                "inference": {
                    "summary_generation": "summary_local",
                    "code_embeddings": "alpha",
                    "summary_embeddings": "alpha"
                }
            }
        })
    }

    fn build_context(
        config_root: serde_json::Value,
        workplane: Arc<CapturingWorkplaneGateway>,
    ) -> CurrentStateConsumerContext {
        let temp = tempdir().expect("temp dir");
        let sqlite_path = temp.path().join("semantic-clones-current-state.sqlite");
        std::mem::forget(temp);

        CurrentStateConsumerContext {
            config_root,
            storage: Arc::new(RelationalStorage::local_only(sqlite_path)),
            relational: Arc::new(FakeRelationalGateway),
            language_services: Arc::new(EmptyLanguageServicesGateway),
            host_services: Arc::new(DefaultHostServicesGateway::new("repo-1")),
            workplane,
        }
    }

    fn base_request() -> CurrentStateConsumerRequest {
        CurrentStateConsumerRequest {
            repo_id: "repo-1".to_string(),
            repo_root: PathBuf::from("/tmp/repo-1"),
            active_branch: Some("main".to_string()),
            head_commit_sha: Some("abc123".to_string()),
            from_generation_seq_exclusive: 0,
            to_generation_seq_inclusive: 1,
            reconcile_mode: ReconcileMode::MergedDelta,
            file_upserts: Vec::new(),
            file_removals: Vec::new(),
            artefact_upserts: Vec::new(),
            artefact_removals: Vec::new(),
        }
    }

    #[tokio::test]
    async fn reconcile_enqueues_path_scoped_jobs_for_changed_files() {
        let workplane = Arc::new(CapturingWorkplaneGateway::default());
        let context = build_context(configured_semantic_clones("auto"), Arc::clone(&workplane));
        let mut request = base_request();
        request.file_upserts.push(ChangedFile {
            path: "src/lib.rs".to_string(),
            language: "rust".to_string(),
            content_id: "blob-1".to_string(),
        });
        request.artefact_upserts.push(ChangedArtefact {
            artefact_id: "artefact-1".to_string(),
            symbol_id: "symbol-1".to_string(),
            path: "src/lib.rs".to_string(),
            canonical_kind: Some("function".to_string()),
            name: "do_work".to_string(),
        });
        request.artefact_upserts.push(ChangedArtefact {
            artefact_id: "artefact-2".to_string(),
            symbol_id: "symbol-2".to_string(),
            path: "src/lib.rs".to_string(),
            canonical_kind: Some("function".to_string()),
            name: "do_more_work".to_string(),
        });

        SemanticClonesCurrentStateConsumer
            .reconcile(&request, &context)
            .await
            .expect("reconcile current state");

        let jobs = workplane
            .take_jobs()
            .into_iter()
            .map(|job| (job.mailbox_name, job.dedupe_key, job.payload))
            .collect::<Vec<_>>();

        assert_eq!(
            jobs,
            vec![
                (
                    SEMANTIC_CLONES_SUMMARY_REFRESH_MAILBOX.to_string(),
                    Some(format!(
                        "{SEMANTIC_CLONES_SUMMARY_REFRESH_MAILBOX}:path:{}:{}",
                        "blob-1", "src/lib.rs"
                    )),
                    serde_json::to_value(SemanticClonesMailboxPayload::CurrentPath {
                        path: "src/lib.rs".to_string(),
                        content_id: "blob-1".to_string(),
                    })
                    .expect("serialize summary refresh path payload"),
                ),
                (
                    SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX.to_string(),
                    Some(format!(
                        "{SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX}:path:{}:{}",
                        "blob-1", "src/lib.rs"
                    )),
                    serde_json::to_value(SemanticClonesMailboxPayload::CurrentPath {
                        path: "src/lib.rs".to_string(),
                        content_id: "blob-1".to_string(),
                    })
                    .expect("serialize code embedding path payload"),
                ),
            ]
        );
    }

    #[tokio::test]
    async fn reconcile_enqueues_summary_and_embedding_jobs_for_changed_artefacts() {
        let workplane = Arc::new(CapturingWorkplaneGateway::default());
        let context = build_context(configured_semantic_clones("auto"), Arc::clone(&workplane));
        let mut request = base_request();
        request.artefact_upserts.push(ChangedArtefact {
            artefact_id: "artefact-1".to_string(),
            symbol_id: "symbol-1".to_string(),
            path: "src/lib.rs".to_string(),
            canonical_kind: Some("function".to_string()),
            name: "do_work".to_string(),
        });

        SemanticClonesCurrentStateConsumer
            .reconcile(&request, &context)
            .await
            .expect("reconcile current state");

        let jobs = workplane
            .take_jobs()
            .into_iter()
            .map(|job| (job.mailbox_name, job.dedupe_key, job.payload))
            .collect::<Vec<_>>();

        assert_eq!(
            jobs,
            vec![
                (
                    SEMANTIC_CLONES_SUMMARY_REFRESH_MAILBOX.to_string(),
                    Some(format!(
                        "{SEMANTIC_CLONES_SUMMARY_REFRESH_MAILBOX}:artefact-1"
                    )),
                    serde_json::to_value(SemanticClonesMailboxPayload::Artefact {
                        artefact_id: "artefact-1".to_string(),
                    })
                    .expect("serialize summary refresh payload"),
                ),
                (
                    SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX.to_string(),
                    Some(format!(
                        "{SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX}:artefact-1"
                    )),
                    serde_json::to_value(SemanticClonesMailboxPayload::Artefact {
                        artefact_id: "artefact-1".to_string(),
                    })
                    .expect("serialize code embedding payload"),
                ),
            ]
        );
    }

    #[tokio::test]
    async fn reconcile_enqueues_summary_embeddings_directly_when_summary_refresh_is_off() {
        let workplane = Arc::new(CapturingWorkplaneGateway::default());
        let context = build_context(configured_semantic_clones("off"), Arc::clone(&workplane));
        let mut request = base_request();
        request.artefact_upserts.push(ChangedArtefact {
            artefact_id: "artefact-1".to_string(),
            symbol_id: "symbol-1".to_string(),
            path: "src/lib.rs".to_string(),
            canonical_kind: Some("function".to_string()),
            name: "do_work".to_string(),
        });

        SemanticClonesCurrentStateConsumer
            .reconcile(&request, &context)
            .await
            .expect("reconcile current state");

        let jobs = workplane
            .take_jobs()
            .into_iter()
            .map(|job| (job.mailbox_name, job.dedupe_key, job.payload))
            .collect::<Vec<_>>();

        assert_eq!(
            jobs,
            vec![
                (
                    SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX.to_string(),
                    Some(format!(
                        "{SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX}:artefact-1"
                    )),
                    serde_json::to_value(SemanticClonesMailboxPayload::Artefact {
                        artefact_id: "artefact-1".to_string(),
                    })
                    .expect("serialize code embedding payload"),
                ),
                (
                    crate::capability_packs::semantic_clones::types::SEMANTIC_CLONES_SUMMARY_EMBEDDING_MAILBOX.to_string(),
                    Some(format!(
                        "{}:artefact-1",
                        crate::capability_packs::semantic_clones::types::SEMANTIC_CLONES_SUMMARY_EMBEDDING_MAILBOX
                    )),
                    serde_json::to_value(SemanticClonesMailboxPayload::Artefact {
                        artefact_id: "artefact-1".to_string(),
                    })
                    .expect("serialize summary embedding payload"),
                ),
            ]
        );
    }

    #[tokio::test]
    async fn reconcile_enqueues_clone_rebuild_when_files_change_without_artefact_upserts() {
        let workplane = Arc::new(CapturingWorkplaneGateway::default());
        let context = build_context(configured_semantic_clones("off"), Arc::clone(&workplane));
        let mut request = base_request();
        request.file_removals.push(RemovedFile {
            path: "src/removed.rs".to_string(),
        });

        SemanticClonesCurrentStateConsumer
            .reconcile(&request, &context)
            .await
            .expect("reconcile current state");

        let jobs = workplane
            .take_jobs()
            .into_iter()
            .map(|job| (job.mailbox_name, job.dedupe_key, job.payload))
            .collect::<Vec<_>>();

        assert_eq!(
            jobs,
            vec![(
                SEMANTIC_CLONES_CLONE_REBUILD_MAILBOX.to_string(),
                Some(SEMANTIC_CLONES_CLONE_REBUILD_MAILBOX.to_string()),
                serde_json::to_value(SemanticClonesMailboxPayload::RepoBackfill)
                    .expect("serialize clone rebuild payload"),
            )]
        );
        assert_eq!(
            repo_backfill_dedupe_key(SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX),
            format!("{SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX}:repo_backfill")
        );
    }
}
