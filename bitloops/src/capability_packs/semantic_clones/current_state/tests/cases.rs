use std::collections::BTreeMap;

use anyhow::Result;
use serde_json::json;
use tempfile::tempdir;

use crate::capability_packs::semantic_clones::workplane::REPO_BACKFILL_MAILBOX_CHUNK_SIZE;
use crate::host::capability_host::{
    ChangedArtefact, ChangedFile, CurrentStateConsumer, ReconcileMode, RemovedArtefact, RemovedFile,
};

use super::super::super::types::{
    SEMANTIC_CLONES_CLONE_REBUILD_MAILBOX, SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX,
    SEMANTIC_CLONES_IDENTITY_EMBEDDING_MAILBOX, SEMANTIC_CLONES_SUMMARY_REFRESH_MAILBOX,
};
use super::super::consumer::SemanticClonesCurrentStateConsumer;
use super::support::{
    CapturingWorkplaneGateway, config_root, count_rows, metrics_map, request,
    seed_current_artefact_ids, seed_current_rows, test_context,
};

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
            SEMANTIC_CLONES_IDENTITY_EMBEDDING_MAILBOX,
        ]
    );
    assert_eq!(metrics["affected_paths"], json!(1));
    assert_eq!(metrics["cleared_paths"], json!(1));
    assert_eq!(metrics["enqueued_summary_jobs"], json!(1));
    assert_eq!(metrics["enqueued_code_embedding_jobs"], json!(1));
    assert_eq!(metrics["enqueued_identity_embedding_jobs"], json!(1));
    assert_eq!(metrics["enqueued_summary_embedding_jobs"], json!(0));
    assert_eq!(metrics["enqueued_clone_rebuild"], json!(0));
    assert_eq!(metrics["reconcile_mode"], json!("merged_delta"));
    Ok(())
}

#[tokio::test]
async fn reconcile_removals_enqueues_clone_rebuild_when_embeddings_are_not_scheduled() -> Result<()>
{
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
async fn reconcile_with_pipeline_disabled_clears_clone_edges_without_enqueuing_jobs() -> Result<()>
{
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
            SEMANTIC_CLONES_IDENTITY_EMBEDDING_MAILBOX,
            SEMANTIC_CLONES_CLONE_REBUILD_MAILBOX,
        ]
    );
    assert!(jobs.iter().all(|job| {
        crate::capability_packs::semantic_clones::workplane::payload_is_repo_backfill(&job.payload)
    }));
    assert_eq!(
        crate::capability_packs::semantic_clones::workplane::payload_work_item_count(
            &jobs[0].payload,
            jobs[0].mailbox_name.as_str(),
        ),
        0,
        "full reconcile summary backfill should persist the exact current workload size",
    );
    assert_eq!(
        crate::capability_packs::semantic_clones::workplane::payload_work_item_count(
            &jobs[1].payload,
            jobs[1].mailbox_name.as_str(),
        ),
        0,
        "full reconcile embedding backfill should persist the exact current workload size",
    );
    assert_eq!(
        crate::capability_packs::semantic_clones::workplane::payload_work_item_count(
            &jobs[3].payload,
            jobs[3].mailbox_name.as_str(),
        ),
        1,
        "clone rebuild remains a single logical work item",
    );
    assert_eq!(metrics["enqueued_summary_jobs"], json!(1));
    assert_eq!(metrics["enqueued_code_embedding_jobs"], json!(1));
    assert_eq!(metrics["enqueued_identity_embedding_jobs"], json!(1));
    assert_eq!(metrics["enqueued_summary_embedding_jobs"], json!(0));
    assert_eq!(metrics["enqueued_clone_rebuild"], json!(1));
    assert_eq!(metrics["reconcile_mode"], json!("full_reconcile"));
    Ok(())
}

#[tokio::test]
async fn reconcile_full_reconcile_chunks_backfill_jobs_for_parallel_mailbox_work() -> Result<()> {
    let repo = tempdir().expect("temp repo");
    let repo_id = "repo-full-chunked";
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
    seed_current_artefact_ids(&ctx.sqlite_path, repo_id, 55).await;

    let result = SemanticClonesCurrentStateConsumer
        .reconcile(&ctx.request, &ctx.context)
        .await?;
    let jobs = ctx.workplane.jobs();
    let metrics = metrics_map(&result);

    let summary_jobs = jobs
        .iter()
        .filter(|job| job.mailbox_name == SEMANTIC_CLONES_SUMMARY_REFRESH_MAILBOX)
        .collect::<Vec<_>>();
    let code_jobs = jobs
        .iter()
        .filter(|job| job.mailbox_name == SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX)
        .collect::<Vec<_>>();
    let identity_jobs = jobs
        .iter()
        .filter(|job| job.mailbox_name == SEMANTIC_CLONES_IDENTITY_EMBEDDING_MAILBOX)
        .collect::<Vec<_>>();

    assert_eq!(summary_jobs.len(), 2);
    assert_eq!(code_jobs.len(), 2);
    assert_eq!(identity_jobs.len(), 2);
    assert_eq!(
        jobs.iter()
            .filter(|job| job.mailbox_name == SEMANTIC_CLONES_CLONE_REBUILD_MAILBOX)
            .count(),
        1
    );
    assert_eq!(
        summary_jobs
            .iter()
            .map(|job| {
                crate::capability_packs::semantic_clones::workplane::payload_work_item_count(
                    &job.payload,
                    job.mailbox_name.as_str(),
                )
            })
            .sum::<u64>(),
        55
    );
    assert_eq!(
        code_jobs
            .iter()
            .map(|job| {
                crate::capability_packs::semantic_clones::workplane::payload_work_item_count(
                    &job.payload,
                    job.mailbox_name.as_str(),
                )
            })
            .sum::<u64>(),
        55
    );
    assert_eq!(
        identity_jobs
            .iter()
            .map(|job| {
                crate::capability_packs::semantic_clones::workplane::payload_work_item_count(
                    &job.payload,
                    job.mailbox_name.as_str(),
                )
            })
            .sum::<u64>(),
        55
    );
    assert!(summary_jobs.iter().all(|job| {
        crate::capability_packs::semantic_clones::workplane::payload_repo_backfill_artefact_ids(
            &job.payload,
        )
        .is_some_and(|artefact_ids| artefact_ids.len() <= REPO_BACKFILL_MAILBOX_CHUNK_SIZE)
    }));
    assert_eq!(metrics["enqueued_summary_jobs"], json!(2));
    assert_eq!(metrics["enqueued_code_embedding_jobs"], json!(2));
    assert_eq!(metrics["enqueued_identity_embedding_jobs"], json!(2));
    assert_eq!(metrics["enqueued_clone_rebuild"], json!(1));
    Ok(())
}
