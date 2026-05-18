use anyhow::Result;
use std::collections::{BTreeMap, HashSet};

use rusqlite::params;

use crate::capability_packs::semantic_clones::embeddings::EmbeddingRepresentationKind;
use crate::capability_packs::semantic_clones::features as semantic_features;
use crate::host::runtime_store::WorkplaneJobStatus;

use super::coordinator::EnrichmentCoordinator;
use super::runtime_events::publish_workplane_runtime_event;
use super::workplane::{
    enqueue_workplane_clone_rebuild, enqueue_workplane_embedding_jobs,
    enqueue_workplane_embedding_repo_backfill_job, enqueue_workplane_summary_jobs,
    enqueue_workplane_summary_repo_backfill_job,
};
use super::{EnrichmentJobTarget, FollowUpJob};

impl EnrichmentCoordinator {
    pub async fn enqueue_semantic_summaries(
        &self,
        target: EnrichmentJobTarget,
        inputs: Vec<semantic_features::SemanticFeatureInput>,
        input_hashes: BTreeMap<String, String>,
    ) -> Result<()> {
        let _ = input_hashes;
        enqueue_workplane_summary_jobs(
            &target,
            inputs.into_iter().map(|input| input.artefact_id).collect(),
        )?;
        let _guard = self.lock.lock().await;
        let mut state = self.load_state()?;
        state.last_action = Some("enqueue_semantic".to_string());
        self.save_state(&mut state)?;
        self.notify.notify_waiters();
        publish_workplane_runtime_event(
            &target,
            crate::capability_packs::semantic_clones::types::SEMANTIC_CLONES_SUMMARY_REFRESH_MAILBOX,
        )?;
        Ok(())
    }

    pub async fn enqueue_symbol_embeddings(
        &self,
        target: EnrichmentJobTarget,
        inputs: Vec<semantic_features::SemanticFeatureInput>,
        input_hashes: BTreeMap<String, String>,
        representation_kind: EmbeddingRepresentationKind,
    ) -> Result<()> {
        let _ = input_hashes;
        enqueue_workplane_embedding_jobs(
            &target,
            inputs.into_iter().map(|input| input.artefact_id).collect(),
            representation_kind,
        )?;
        let _guard = self.lock.lock().await;
        let mut state = self.load_state()?;
        state.last_action = Some("enqueue_embeddings".to_string());
        self.save_state(&mut state)?;
        self.notify.notify_waiters();
        let mailbox_name = match representation_kind {
            EmbeddingRepresentationKind::Code => {
                crate::capability_packs::semantic_clones::types::SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX
            }
            EmbeddingRepresentationKind::Identity => {
                crate::capability_packs::semantic_clones::types::SEMANTIC_CLONES_IDENTITY_EMBEDDING_MAILBOX
            }
            EmbeddingRepresentationKind::Summary => {
                crate::capability_packs::semantic_clones::types::SEMANTIC_CLONES_SUMMARY_EMBEDDING_MAILBOX
            }
        };
        publish_workplane_runtime_event(&target, mailbox_name)?;
        Ok(())
    }

    pub async fn enqueue_clone_edges_rebuild(&self, target: EnrichmentJobTarget) -> Result<()> {
        enqueue_workplane_clone_rebuild(&target)?;
        let _guard = self.lock.lock().await;
        let mut state = self.load_state()?;
        state.last_action = Some("enqueue_clone_edges_rebuild".to_string());
        self.save_state(&mut state)?;
        self.notify.notify_waiters();
        publish_workplane_runtime_event(
            &target,
            crate::capability_packs::semantic_clones::types::SEMANTIC_CLONES_CLONE_REBUILD_MAILBOX,
        )?;
        Ok(())
    }

    pub async fn prune_pending_single_artefact_jobs_after_reconcile(
        &self,
        repo_id: &str,
        relational: &crate::host::devql::RelationalStorage,
    ) -> Result<u64> {
        let repo_id_sql = crate::host::devql::esc_pg(repo_id);
        let existing_artefact_ids = relational
            .query_rows(&format!(
                "SELECT DISTINCT artefact_id FROM artefacts WHERE repo_id = '{repo_id_sql}' \
UNION \
SELECT DISTINCT artefact_id FROM artefacts_current WHERE repo_id = '{repo_id_sql}'"
            ))
            .await?
            .into_iter()
            .filter_map(|row| {
                row.as_object()
                    .and_then(|row| row.get("artefact_id"))
                    .and_then(|value| value.as_str())
                    .map(str::to_string)
            })
            .collect::<HashSet<_>>();

        let _guard = self.lock.lock().await;
        let deleted = self.workplane_store.with_write_connection(|conn| {
            let mut stmt = conn.prepare(
                "SELECT job_id, payload FROM capability_workplane_jobs WHERE repo_id = ?1 AND status = ?2",
            )?;
            let rows = stmt.query_map(
                params![repo_id, WorkplaneJobStatus::Pending.as_str()],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )?;
            let mut job_ids = Vec::new();
            for row in rows {
                let (job_id, payload_raw) = row?;
                let payload = serde_json::from_str::<serde_json::Value>(&payload_raw)
                    .unwrap_or(serde_json::Value::Null);
                if crate::capability_packs::semantic_clones::workplane::payload_is_repo_backfill(
                    &payload,
                ) {
                    continue;
                }
                let Some(artefact_id) =
                    crate::capability_packs::semantic_clones::workplane::payload_artefact_id(
                        &payload,
                    )
                else {
                    continue;
                };
                if !existing_artefact_ids.contains(&artefact_id) {
                    job_ids.push(job_id);
                }
            }
            for job_id in &job_ids {
                conn.execute(
                    "DELETE FROM capability_workplane_jobs WHERE job_id = ?1",
                    params![job_id],
                )?;
            }
            Ok(u64::try_from(job_ids.len()).unwrap_or_default())
        })?;
        Ok(deleted)
    }

    pub(crate) async fn enqueue_follow_up(&self, follow_up: FollowUpJob) -> Result<()> {
        match follow_up {
            FollowUpJob::RepoBackfillSummaries {
                target,
                artefact_ids,
            } => {
                self.enqueue_repo_backfill_summary_job(target, artefact_ids)
                    .await
            }
            FollowUpJob::RepoBackfillEmbeddings {
                target,
                artefact_ids,
                representation_kind,
            } => {
                self.enqueue_repo_backfill_embedding_job(target, artefact_ids, representation_kind)
                    .await
            }
            FollowUpJob::SymbolEmbeddings {
                target,
                artefact_ids,
                input_hashes,
                representation_kind,
            } => {
                self.enqueue_symbol_embeddings_from_artefact_ids(
                    target,
                    artefact_ids,
                    input_hashes,
                    representation_kind,
                )
                .await
            }
            FollowUpJob::CloneEdgesRebuild { target } => {
                self.enqueue_clone_edges_rebuild(target).await
            }
        }
    }

    async fn enqueue_symbol_embeddings_from_artefact_ids(
        &self,
        target: EnrichmentJobTarget,
        artefact_ids: Vec<String>,
        input_hashes: BTreeMap<String, String>,
        representation_kind: EmbeddingRepresentationKind,
    ) -> Result<()> {
        let _ = input_hashes;
        enqueue_workplane_embedding_jobs(&target, artefact_ids, representation_kind)?;
        let mut state = self.load_state()?;
        state.last_action = Some("enqueue_embeddings".to_string());
        self.save_state(&mut state)?;
        self.notify.notify_waiters();
        let mailbox_name = match representation_kind {
            EmbeddingRepresentationKind::Code => {
                crate::capability_packs::semantic_clones::types::SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX
            }
            EmbeddingRepresentationKind::Identity => {
                crate::capability_packs::semantic_clones::types::SEMANTIC_CLONES_IDENTITY_EMBEDDING_MAILBOX
            }
            EmbeddingRepresentationKind::Summary => {
                crate::capability_packs::semantic_clones::types::SEMANTIC_CLONES_SUMMARY_EMBEDDING_MAILBOX
            }
        };
        publish_workplane_runtime_event(&target, mailbox_name)?;
        Ok(())
    }

    async fn enqueue_repo_backfill_embedding_job(
        &self,
        target: EnrichmentJobTarget,
        artefact_ids: Vec<String>,
        representation_kind: EmbeddingRepresentationKind,
    ) -> Result<()> {
        enqueue_workplane_embedding_repo_backfill_job(&target, artefact_ids, representation_kind)?;
        let mut state = self.load_state()?;
        state.last_action = Some("enqueue_embeddings".to_string());
        self.save_state(&mut state)?;
        self.notify.notify_waiters();
        let mailbox_name = match representation_kind {
            EmbeddingRepresentationKind::Code => {
                crate::capability_packs::semantic_clones::types::SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX
            }
            EmbeddingRepresentationKind::Identity => {
                crate::capability_packs::semantic_clones::types::SEMANTIC_CLONES_IDENTITY_EMBEDDING_MAILBOX
            }
            EmbeddingRepresentationKind::Summary => {
                crate::capability_packs::semantic_clones::types::SEMANTIC_CLONES_SUMMARY_EMBEDDING_MAILBOX
            }
        };
        publish_workplane_runtime_event(&target, mailbox_name)?;
        Ok(())
    }

    async fn enqueue_repo_backfill_summary_job(
        &self,
        target: EnrichmentJobTarget,
        artefact_ids: Vec<String>,
    ) -> Result<()> {
        enqueue_workplane_summary_repo_backfill_job(&target, artefact_ids)?;
        let mut state = self.load_state()?;
        state.last_action = Some("enqueue_semantic".to_string());
        self.save_state(&mut state)?;
        self.notify.notify_waiters();
        publish_workplane_runtime_event(
            &target,
            crate::capability_packs::semantic_clones::types::SEMANTIC_CLONES_SUMMARY_REFRESH_MAILBOX,
        )?;
        Ok(())
    }
}
