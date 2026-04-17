use std::collections::BTreeSet;

use serde_json::json;

use crate::host::capability_host::gateways::CapabilityWorkplaneJob;
use crate::host::capability_host::{
    CapabilityConfigView, CurrentStateConsumer, CurrentStateConsumerContext,
    CurrentStateConsumerFuture, CurrentStateConsumerRequest, CurrentStateConsumerResult,
    ReconcileMode,
};

use super::runtime_config::resolve_semantic_clones_config;
use super::types::{
    SEMANTIC_CLONES_CAPABILITY_ID, SEMANTIC_CLONES_CLONE_REBUILD_MAILBOX,
    SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX, SEMANTIC_CLONES_CURRENT_STATE_CONSUMER_ID,
    SEMANTIC_CLONES_SUMMARY_EMBEDDING_MAILBOX, SEMANTIC_CLONES_SUMMARY_REFRESH_MAILBOX,
};
use super::workplane::{
    SemanticClonesMailboxPayload, repo_backfill_dedupe_key, resolve_effective_mailbox_intent,
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
            let affected_paths = collect_affected_paths(request);
            let cleared_paths = clear_current_projection_rows(
                context.storage.as_ref(),
                &request.repo_id,
                &affected_paths,
            )
            .await?;

            if !intent.has_any_pipeline_intent() {
                super::pipeline::delete_repo_current_symbol_clone_edges(
                    context.storage.as_ref(),
                    &request.repo_id,
                )
                .await?;
                return Ok(CurrentStateConsumerResult {
                    applied_to_generation_seq: request.to_generation_seq_inclusive,
                    warnings: Vec::new(),
                    metrics: Some(json!({
                        "affected_paths": affected_paths.len(),
                        "cleared_paths": cleared_paths,
                        "enqueued_summary_jobs": 0,
                        "enqueued_code_embedding_jobs": 0,
                        "enqueued_summary_embedding_jobs": 0,
                        "enqueued_clone_rebuild": 0,
                        "reconcile_mode": reconcile_mode_label(request.reconcile_mode),
                    })),
                });
            }

            let artefact_ids = request
                .artefact_upserts
                .iter()
                .map(|artefact| artefact.artefact_id.clone())
                .collect::<BTreeSet<_>>();
            let has_removals =
                !request.file_removals.is_empty() || !request.artefact_removals.is_empty();
            let is_full_reconcile = matches!(request.reconcile_mode, ReconcileMode::FullReconcile);
            let full_reconcile_work_item_count =
                if is_full_reconcile
                    && (intent.summary_refresh_active
                        || intent.code_embeddings_active
                        || intent.summary_embeddings_active)
                {
                    Some(
                        current_repo_work_item_count(
                            context.storage.as_ref(),
                            &request.repo_root,
                            &request.repo_id,
                        )
                        .await?,
                    )
                } else {
                    None
                };

            let mut jobs = Vec::new();
            let mut summary_job_count = 0_u64;
            let mut code_embedding_job_count = 0_u64;
            let mut summary_embedding_job_count = 0_u64;
            let mut clone_rebuild_job_count = 0_u64;

            if intent.summary_refresh_active {
                if is_full_reconcile {
                    jobs.push(repo_backfill_job(
                        SEMANTIC_CLONES_SUMMARY_REFRESH_MAILBOX,
                        full_reconcile_work_item_count,
                    )?);
                    summary_job_count += 1;
                } else {
                    for artefact_id in &artefact_ids {
                        jobs.push(artefact_job(
                            SEMANTIC_CLONES_SUMMARY_REFRESH_MAILBOX,
                            artefact_id,
                        )?);
                        summary_job_count += 1;
                    }
                }
            }

            if intent.code_embeddings_active {
                if is_full_reconcile {
                    jobs.push(repo_backfill_job(
                        SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX,
                        full_reconcile_work_item_count,
                    )?);
                    code_embedding_job_count += 1;
                } else {
                    for artefact_id in &artefact_ids {
                        jobs.push(artefact_job(
                            SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX,
                            artefact_id,
                        )?);
                        code_embedding_job_count += 1;
                    }
                }
            }

            if intent.summary_embeddings_active && !intent.summary_refresh_active {
                if is_full_reconcile {
                    jobs.push(repo_backfill_job(
                        SEMANTIC_CLONES_SUMMARY_EMBEDDING_MAILBOX,
                        full_reconcile_work_item_count,
                    )?);
                    summary_embedding_job_count += 1;
                } else {
                    for artefact_id in &artefact_ids {
                        jobs.push(artefact_job(
                            SEMANTIC_CLONES_SUMMARY_EMBEDDING_MAILBOX,
                            artefact_id,
                        )?);
                        summary_embedding_job_count += 1;
                    }
                }
            }

            let embedding_pipeline_scheduled = code_embedding_job_count > 0
                || summary_embedding_job_count > 0
                || (summary_job_count > 0 && intent.summary_embeddings_active);
            if intent.clone_rebuild_active
                && (is_full_reconcile || has_removals || !embedding_pipeline_scheduled)
            {
                jobs.push(repo_backfill_job(
                    SEMANTIC_CLONES_CLONE_REBUILD_MAILBOX,
                    Some(1),
                )?);
                clone_rebuild_job_count += 1;
            }

            if !jobs.is_empty() {
                context.workplane.enqueue_jobs(jobs)?;
            }

            if !intent.has_any_embedding_intent() {
                super::pipeline::delete_repo_current_symbol_clone_edges(
                    context.storage.as_ref(),
                    &request.repo_id,
                )
                .await?;
            }

            Ok(CurrentStateConsumerResult {
                applied_to_generation_seq: request.to_generation_seq_inclusive,
                warnings: Vec::new(),
                metrics: Some(json!({
                    "affected_paths": affected_paths.len(),
                    "cleared_paths": cleared_paths,
                    "enqueued_summary_jobs": summary_job_count,
                    "enqueued_code_embedding_jobs": code_embedding_job_count,
                    "enqueued_summary_embedding_jobs": summary_embedding_job_count,
                    "enqueued_clone_rebuild": clone_rebuild_job_count,
                    "reconcile_mode": reconcile_mode_label(request.reconcile_mode),
                })),
            })
        })
    }
}

fn collect_affected_paths(request: &CurrentStateConsumerRequest) -> BTreeSet<String> {
    request
        .file_upserts
        .iter()
        .map(|file| file.path.clone())
        .chain(request.file_removals.iter().map(|file| file.path.clone()))
        .chain(
            request
                .artefact_upserts
                .iter()
                .map(|artefact| artefact.path.clone()),
        )
        .chain(
            request
                .artefact_removals
                .iter()
                .map(|artefact| artefact.path.clone()),
        )
        .collect()
}

async fn clear_current_projection_rows(
    relational: &crate::host::devql::RelationalStorage,
    repo_id: &str,
    affected_paths: &BTreeSet<String>,
) -> anyhow::Result<usize> {
    let mut cleared = 0usize;
    for path in affected_paths {
        super::clear_current_semantic_feature_rows_for_path(relational, repo_id, path).await?;
        super::clear_current_symbol_embedding_rows_for_path(relational, repo_id, path).await?;
        cleared += 1;
    }
    Ok(cleared)
}

fn artefact_job(mailbox_name: &str, artefact_id: &str) -> anyhow::Result<CapabilityWorkplaneJob> {
    Ok(CapabilityWorkplaneJob::new(
        mailbox_name,
        Some(format!("{mailbox_name}:{artefact_id}")),
        serde_json::to_value(SemanticClonesMailboxPayload::Artefact {
            artefact_id: artefact_id.to_string(),
        })?,
    ))
}

fn repo_backfill_job(
    mailbox_name: &str,
    work_item_count: Option<u64>,
) -> anyhow::Result<CapabilityWorkplaneJob> {
    Ok(CapabilityWorkplaneJob::new(
        mailbox_name,
        Some(repo_backfill_dedupe_key(mailbox_name)),
        serde_json::to_value(SemanticClonesMailboxPayload::RepoBackfill { work_item_count })?,
    ))
}

async fn current_repo_work_item_count(
    relational: &crate::host::devql::RelationalStorage,
    repo_root: &std::path::Path,
    repo_id: &str,
) -> anyhow::Result<u64> {
    Ok(u64::try_from(
        crate::capability_packs::semantic_clones::load_semantic_feature_inputs_for_current_repo(
            relational, repo_root, repo_id,
        )
        .await?
        .len(),
    )
    .unwrap_or(u64::MAX))
}

fn reconcile_mode_label(mode: ReconcileMode) -> &'static str {
    match mode {
        ReconcileMode::MergedDelta => "merged_delta",
        ReconcileMode::FullReconcile => "full_reconcile",
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::path::{Path, PathBuf};
    use std::sync::{Arc, Mutex};

    use anyhow::{Result, bail};
    use rusqlite::Connection;
    use serde_json::{Map, Value, json};
    use tempfile::tempdir;

    use super::*;
    use crate::host::capability_host::gateways::{
        CapabilityMailboxStatus, CapabilityWorkplaneEnqueueResult, CapabilityWorkplaneGateway,
        DefaultHostServicesGateway, EmptyLanguageServicesGateway, HostServicesGateway,
        RelationalGateway,
    };
    use crate::host::capability_host::{
        ChangedArtefact, ChangedFile, CurrentStateConsumerContext, RemovedArtefact, RemovedFile,
    };
    use crate::models::ProductionArtefact;

    #[derive(Clone, Default)]
    struct CapturingWorkplaneGateway {
        jobs: Arc<Mutex<Vec<CapabilityWorkplaneJob>>>,
        status: BTreeMap<String, CapabilityMailboxStatus>,
    }

    impl CapturingWorkplaneGateway {
        fn with_status(status: BTreeMap<String, CapabilityMailboxStatus>) -> Self {
            Self {
                jobs: Arc::new(Mutex::new(Vec::new())),
                status,
            }
        }

        fn jobs(&self) -> Vec<CapabilityWorkplaneJob> {
            self.jobs.lock().expect("lock captured jobs").clone()
        }
    }

    impl CapabilityWorkplaneGateway for CapturingWorkplaneGateway {
        fn enqueue_jobs(
            &self,
            jobs: Vec<CapabilityWorkplaneJob>,
        ) -> Result<CapabilityWorkplaneEnqueueResult> {
            let inserted_jobs = jobs.len() as u64;
            self.jobs.lock().expect("lock captured jobs").extend(jobs);
            Ok(CapabilityWorkplaneEnqueueResult {
                inserted_jobs,
                updated_jobs: 0,
            })
        }

        fn mailbox_status(&self) -> Result<BTreeMap<String, CapabilityMailboxStatus>> {
            Ok(self.status.clone())
        }
    }

    struct NoopRelationalGateway;

    impl RelationalGateway for NoopRelationalGateway {
        fn resolve_checkpoint_id(&self, _repo_id: &str, _checkpoint_ref: &str) -> Result<String> {
            bail!("resolve_checkpoint_id is not used in semantic_clones current-state tests")
        }

        fn artefact_exists(&self, _repo_id: &str, _artefact_id: &str) -> Result<bool> {
            bail!("artefact_exists is not used in semantic_clones current-state tests")
        }

        fn load_repo_id_for_commit(&self, _commit_sha: &str) -> Result<String> {
            bail!("load_repo_id_for_commit is not used in semantic_clones current-state tests")
        }

        fn load_current_production_artefacts(
            &self,
            _repo_id: &str,
        ) -> Result<Vec<ProductionArtefact>> {
            bail!(
                "load_current_production_artefacts is not used in semantic_clones current-state tests"
            )
        }

        fn load_production_artefacts(&self, _commit_sha: &str) -> Result<Vec<ProductionArtefact>> {
            bail!("load_production_artefacts is not used in semantic_clones current-state tests")
        }

        fn load_artefacts_for_file_lines(
            &self,
            _commit_sha: &str,
            _file_path: &str,
        ) -> Result<Vec<(String, i64, i64)>> {
            bail!(
                "load_artefacts_for_file_lines is not used in semantic_clones current-state tests"
            )
        }
    }

    struct TestContext {
        _tempdir: tempfile::TempDir,
        storage: Arc<crate::host::devql::RelationalStorage>,
        workplane: CapturingWorkplaneGateway,
        request: CurrentStateConsumerRequest,
        context: CurrentStateConsumerContext,
        sqlite_path: PathBuf,
    }

    async fn test_context(
        config_root: Value,
        workplane: CapturingWorkplaneGateway,
        request: CurrentStateConsumerRequest,
    ) -> Result<TestContext> {
        let tempdir = tempdir().expect("temp dir");
        let sqlite_path = tempdir.path().join("semantic-current-state.sqlite");
        let storage = Arc::new(crate::host::devql::RelationalStorage::local_only(
            sqlite_path.clone(),
        ));
        crate::capability_packs::semantic_clones::init_sqlite_semantic_features_schema(
            &sqlite_path,
        )
        .await?;
        crate::capability_packs::semantic_clones::init_sqlite_semantic_embeddings_schema(
            &sqlite_path,
        )
        .await?;
        crate::capability_packs::semantic_clones::pipeline::delete_repo_current_symbol_clone_edges(
            storage.as_ref(),
            &request.repo_id,
        )
        .await?;

        let context = CurrentStateConsumerContext {
            config_root,
            storage: Arc::clone(&storage),
            relational: Arc::new(NoopRelationalGateway),
            language_services: Arc::new(EmptyLanguageServicesGateway),
            host_services: Arc::new(DefaultHostServicesGateway::new(request.repo_id.clone()))
                as Arc<dyn HostServicesGateway>,
            workplane: Arc::new(workplane.clone()),
        };

        Ok(TestContext {
            _tempdir: tempdir,
            storage,
            workplane,
            request,
            context,
            sqlite_path,
        })
    }

    fn request(
        repo_root: &Path,
        repo_id: &str,
        reconcile_mode: ReconcileMode,
        file_upserts: Vec<ChangedFile>,
        file_removals: Vec<RemovedFile>,
        artefact_upserts: Vec<ChangedArtefact>,
        artefact_removals: Vec<RemovedArtefact>,
    ) -> CurrentStateConsumerRequest {
        CurrentStateConsumerRequest {
            repo_id: repo_id.to_string(),
            repo_root: repo_root.to_path_buf(),
            active_branch: Some("main".to_string()),
            head_commit_sha: Some("abc123".to_string()),
            from_generation_seq_exclusive: 0,
            to_generation_seq_inclusive: 1,
            reconcile_mode,
            file_upserts,
            file_removals,
            artefact_upserts,
            artefact_removals,
        }
    }

    fn config_root(
        summary_generation: Option<&str>,
        code_embeddings: Option<&str>,
        summary_embeddings: Option<&str>,
    ) -> Value {
        let mut inference = Map::new();
        if let Some(summary_generation) = summary_generation {
            inference.insert(
                "summary_generation".to_string(),
                Value::String(summary_generation.to_string()),
            );
        }
        if let Some(code_embeddings) = code_embeddings {
            inference.insert(
                "code_embeddings".to_string(),
                Value::String(code_embeddings.to_string()),
            );
        }
        if let Some(summary_embeddings) = summary_embeddings {
            inference.insert(
                "summary_embeddings".to_string(),
                Value::String(summary_embeddings.to_string()),
            );
        }

        json!({
            SEMANTIC_CLONES_CAPABILITY_ID: {
                "summary_mode": if summary_generation.is_some() { "auto" } else { "off" },
                "embedding_mode": if code_embeddings.is_some() || summary_embeddings.is_some() {
                    "semantic_aware_once"
                } else {
                    "off"
                },
                "ann_neighbors": 5,
                "enrichment_workers": 1,
                "inference": Value::Object(inference),
            }
        })
    }

    async fn seed_current_rows(
        storage: &crate::host::devql::RelationalStorage,
        repo_id: &str,
        sqlite_path: &Path,
        path: &str,
        suffix: &str,
    ) {
        crate::capability_packs::semantic_clones::pipeline::delete_repo_current_symbol_clone_edges(
            storage, repo_id,
        )
        .await
        .expect("ensure current clone edge schema");
        let conn = Connection::open(sqlite_path).expect("open sqlite");
        let symbol_id = format!("symbol-{suffix}");
        let artefact_id = format!("artefact-{suffix}");
        let content_id = format!("content-{suffix}");
        let semantic_hash = format!("semantic-hash-{suffix}");

        conn.execute(
            "INSERT INTO symbol_semantics_current (
                artefact_id, repo_id, path, content_id, symbol_id, semantic_features_input_hash,
                template_summary, summary, confidence
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            rusqlite::params![
                artefact_id,
                repo_id,
                path,
                content_id,
                symbol_id,
                semantic_hash,
                format!("summary {suffix}"),
                format!("summary {suffix}"),
                0.9_f64,
            ],
        )
        .expect("insert current semantics");
        conn.execute(
            "INSERT INTO symbol_features_current (
                artefact_id, repo_id, path, content_id, symbol_id, semantic_features_input_hash,
                normalized_name, normalized_signature, modifiers, identifier_tokens,
                normalized_body_tokens, parent_kind, context_tokens
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, '[]', ?9, ?10, ?11, ?12)",
            rusqlite::params![
                artefact_id,
                repo_id,
                path,
                content_id,
                symbol_id,
                semantic_hash,
                format!("fn_{suffix}"),
                format!("fn {suffix}()"),
                r#"["fn"]"#,
                r#"["return"]"#,
                "module",
                r#"["ctx"]"#,
            ],
        )
        .expect("insert current features");
        for representation_kind in ["code", "summary"] {
            conn.execute(
                "INSERT INTO symbol_embeddings_current (
                    artefact_id, repo_id, path, content_id, symbol_id, representation_kind,
                    setup_fingerprint, provider, model, dimension, embedding_input_hash, embedding
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
                rusqlite::params![
                    artefact_id,
                    repo_id,
                    path,
                    content_id,
                    symbol_id,
                    representation_kind,
                    format!("setup-{representation_kind}"),
                    "local",
                    format!("model-{representation_kind}"),
                    3_i64,
                    format!("embed-{representation_kind}-{suffix}"),
                    "[0.1,0.2,0.3]",
                ],
            )
            .expect("insert current embedding");
        }
        conn.execute(
            "INSERT INTO symbol_clone_edges_current (
                repo_id, source_symbol_id, source_artefact_id, target_symbol_id,
                target_artefact_id, relation_kind, score, semantic_score, lexical_score,
                structural_score, clone_input_hash, explanation_json
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            rusqlite::params![
                repo_id,
                format!("source-{suffix}"),
                format!("source-artefact-{suffix}"),
                symbol_id,
                artefact_id,
                "similar_implementation",
                0.8_f64,
                0.8_f64,
                0.5_f64,
                0.4_f64,
                format!("clone-{suffix}"),
                "{}",
            ],
        )
        .expect("insert current clone edge");
    }

    fn count_rows(sqlite_path: &Path, sql: &str, repo_id: &str, path: Option<&str>) -> i64 {
        let conn = Connection::open(sqlite_path).expect("open sqlite for count");
        match path {
            Some(path) => conn
                .query_row(sql, rusqlite::params![repo_id, path], |row| row.get(0))
                .expect("count rows by path"),
            None => conn
                .query_row(sql, rusqlite::params![repo_id], |row| row.get(0))
                .expect("count rows"),
        }
    }

    fn metrics_map(result: &CurrentStateConsumerResult) -> BTreeMap<String, Value> {
        result
            .metrics
            .clone()
            .and_then(|value| value.as_object().cloned())
            .expect("metrics object")
            .into_iter()
            .collect()
    }

    #[tokio::test]
    async fn reconcile_delta_clears_paths_and_enqueues_expected_jobs() -> Result<()> {
        let repo = tempdir().expect("temp repo");
        let repo_id = "repo-delta";
        let request = request(
            repo.path(),
            repo_id,
            ReconcileMode::MergedDelta,
            vec![ChangedFile {
                path: "src/alpha.ts".to_string(),
                language: "typescript".to_string(),
                content_id: "content-alpha".to_string(),
            }],
            Vec::new(),
            vec![ChangedArtefact {
                artefact_id: "artefact-alpha".to_string(),
                symbol_id: "symbol-alpha".to_string(),
                path: "src/alpha.ts".to_string(),
                canonical_kind: Some("function".to_string()),
                name: "alpha".to_string(),
            }],
            Vec::new(),
        );
        let workplane = CapturingWorkplaneGateway::default();
        let ctx = test_context(
            config_root(Some("summary"), Some("code"), Some("summary-embed")),
            workplane,
            request,
        )
        .await?;
        seed_current_rows(
            ctx.storage.as_ref(),
            repo_id,
            &ctx.sqlite_path,
            "src/alpha.ts",
            "alpha",
        )
        .await;
        seed_current_rows(
            ctx.storage.as_ref(),
            repo_id,
            &ctx.sqlite_path,
            "src/other.ts",
            "other",
        )
        .await;

        let result = SemanticClonesCurrentStateConsumer
            .reconcile(&ctx.request, &ctx.context)
            .await?;
        let jobs = ctx.workplane.jobs();
        let mailboxes = jobs
            .iter()
            .map(|job| job.mailbox_name.as_str())
            .collect::<Vec<_>>();
        let metrics = metrics_map(&result);

        assert_eq!(
            count_rows(
                &ctx.sqlite_path,
                "SELECT COUNT(*) FROM symbol_semantics_current WHERE repo_id = ?1 AND path = ?2",
                repo_id,
                Some("src/alpha.ts"),
            ),
            0
        );
        assert_eq!(
            count_rows(
                &ctx.sqlite_path,
                "SELECT COUNT(*) FROM symbol_features_current WHERE repo_id = ?1 AND path = ?2",
                repo_id,
                Some("src/alpha.ts"),
            ),
            0
        );
        assert_eq!(
            count_rows(
                &ctx.sqlite_path,
                "SELECT COUNT(*) FROM symbol_embeddings_current WHERE repo_id = ?1 AND path = ?2",
                repo_id,
                Some("src/alpha.ts"),
            ),
            0
        );
        assert!(
            count_rows(
                &ctx.sqlite_path,
                "SELECT COUNT(*) FROM symbol_features_current WHERE repo_id = ?1 AND path = ?2",
                repo_id,
                Some("src/other.ts"),
            ) > 0
        );
        assert_eq!(
            mailboxes,
            vec![
                SEMANTIC_CLONES_SUMMARY_REFRESH_MAILBOX,
                SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX,
            ]
        );
        assert_eq!(metrics["affected_paths"], json!(1));
        assert_eq!(metrics["cleared_paths"], json!(1));
        assert_eq!(metrics["enqueued_summary_jobs"], json!(1));
        assert_eq!(metrics["enqueued_code_embedding_jobs"], json!(1));
        assert_eq!(metrics["enqueued_summary_embedding_jobs"], json!(0));
        assert_eq!(metrics["enqueued_clone_rebuild"], json!(0));
        assert_eq!(metrics["reconcile_mode"], json!("merged_delta"));
        Ok(())
    }

    #[tokio::test]
    async fn reconcile_removals_enqueues_clone_rebuild_when_embeddings_are_not_scheduled()
    -> Result<()> {
        let repo = tempdir().expect("temp repo");
        let repo_id = "repo-removals";
        let request = request(
            repo.path(),
            repo_id,
            ReconcileMode::MergedDelta,
            Vec::new(),
            vec![RemovedFile {
                path: "src/removed.ts".to_string(),
            }],
            Vec::new(),
            vec![RemovedArtefact {
                artefact_id: "artefact-removed".to_string(),
                symbol_id: "symbol-removed".to_string(),
                path: "src/removed.ts".to_string(),
            }],
        );
        let workplane = CapturingWorkplaneGateway::default();
        let ctx = test_context(
            config_root(None, Some("code"), Some("summary")),
            workplane,
            request,
        )
        .await?;
        seed_current_rows(
            ctx.storage.as_ref(),
            repo_id,
            &ctx.sqlite_path,
            "src/removed.ts",
            "removed",
        )
        .await;

        let result = SemanticClonesCurrentStateConsumer
            .reconcile(&ctx.request, &ctx.context)
            .await?;
        let jobs = ctx.workplane.jobs();
        let metrics = metrics_map(&result);

        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0].mailbox_name, SEMANTIC_CLONES_CLONE_REBUILD_MAILBOX);
        assert!(
            crate::capability_packs::semantic_clones::workplane::payload_is_repo_backfill(
                &jobs[0].payload
            )
        );
        assert_eq!(
            count_rows(
                &ctx.sqlite_path,
                "SELECT COUNT(*) FROM symbol_embeddings_current WHERE repo_id = ?1 AND path = ?2",
                repo_id,
                Some("src/removed.ts"),
            ),
            0
        );
        assert_eq!(metrics["enqueued_clone_rebuild"], json!(1));
        Ok(())
    }

    #[tokio::test]
    async fn reconcile_with_pipeline_disabled_clears_clone_edges_without_enqueuing_jobs()
    -> Result<()> {
        let repo = tempdir().expect("temp repo");
        let repo_id = "repo-disabled";
        let request = request(
            repo.path(),
            repo_id,
            ReconcileMode::MergedDelta,
            Vec::new(),
            vec![RemovedFile {
                path: "src/removed.ts".to_string(),
            }],
            Vec::new(),
            Vec::new(),
        );
        let workplane = CapturingWorkplaneGateway::default();
        let ctx = test_context(config_root(None, None, None), workplane, request).await?;
        seed_current_rows(
            ctx.storage.as_ref(),
            repo_id,
            &ctx.sqlite_path,
            "src/removed.ts",
            "disabled",
        )
        .await;

        let result = SemanticClonesCurrentStateConsumer
            .reconcile(&ctx.request, &ctx.context)
            .await?;
        let metrics = metrics_map(&result);

        assert!(ctx.workplane.jobs().is_empty());
        assert_eq!(
            count_rows(
                &ctx.sqlite_path,
                "SELECT COUNT(*) FROM symbol_clone_edges_current WHERE repo_id = ?1",
                repo_id,
                None,
            ),
            0
        );
        assert_eq!(metrics["enqueued_clone_rebuild"], json!(0));
        Ok(())
    }

    #[tokio::test]
    async fn reconcile_full_reconcile_enqueues_repo_backfill_jobs() -> Result<()> {
        let repo = tempdir().expect("temp repo");
        let repo_id = "repo-full";
        let request = request(
            repo.path(),
            repo_id,
            ReconcileMode::FullReconcile,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
        );
        let workplane = CapturingWorkplaneGateway::with_status(BTreeMap::new());
        let ctx = test_context(
            config_root(Some("summary"), Some("code"), Some("summary-embed")),
            workplane,
            request,
        )
        .await?;

        let result = SemanticClonesCurrentStateConsumer
            .reconcile(&ctx.request, &ctx.context)
            .await?;
        let jobs = ctx.workplane.jobs();
        let metrics = metrics_map(&result);
        let mailboxes = jobs
            .iter()
            .map(|job| job.mailbox_name.as_str())
            .collect::<Vec<_>>();

        assert_eq!(
            mailboxes,
            vec![
                SEMANTIC_CLONES_SUMMARY_REFRESH_MAILBOX,
                SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX,
                SEMANTIC_CLONES_CLONE_REBUILD_MAILBOX,
            ]
        );
        assert!(jobs.iter().all(|job| {
            crate::capability_packs::semantic_clones::workplane::payload_is_repo_backfill(
                &job.payload,
            )
        }));
        assert_eq!(metrics["enqueued_summary_jobs"], json!(1));
        assert_eq!(metrics["enqueued_code_embedding_jobs"], json!(1));
        assert_eq!(metrics["enqueued_summary_embedding_jobs"], json!(0));
        assert_eq!(metrics["enqueued_clone_rebuild"], json!(1));
        assert_eq!(metrics["reconcile_mode"], json!("full_reconcile"));
        Ok(())
    }
}
