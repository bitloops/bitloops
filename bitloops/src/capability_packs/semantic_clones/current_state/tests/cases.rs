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
use super::super::projection::current_repo_backfill_artefact_ids;
use super::support::{
    CapturingWorkplaneGateway, config_root, count_rows, metrics_map, request,
    seed_current_artefact, seed_current_artefact_ids, seed_current_file_state, seed_current_rows,
    test_context,
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
async fn reconcile_clears_current_projection_rows_for_affected_paths_in_bulk() -> Result<()> {
    let repo = tempdir().expect("temp repo");
    let repo_id = "repo-bulk-clear";
    let request = request(
        repo.path(),
        repo_id,
        ReconcileMode::MergedDelta,
        vec![
            ChangedFile {
                path: "src/a.rs".to_string(),
                language: "rust".to_string(),
                content_id: "content-a".to_string(),
            },
            ChangedFile {
                path: "src/b.rs".to_string(),
                language: "rust".to_string(),
                content_id: "content-b".to_string(),
            },
        ],
        Vec::new(),
        Vec::new(),
        Vec::new(),
    );
    let workplane = CapturingWorkplaneGateway::default();
    let ctx = test_context(config_root(None, None, None), workplane, request).await?;
    seed_current_rows(
        ctx.storage.as_ref(),
        repo_id,
        &ctx.sqlite_path,
        "src/a.rs",
        "a",
    )
    .await;
    seed_current_rows(
        ctx.storage.as_ref(),
        repo_id,
        &ctx.sqlite_path,
        "src/b.rs",
        "b",
    )
    .await;
    seed_current_rows(
        ctx.storage.as_ref(),
        repo_id,
        &ctx.sqlite_path,
        "src/c.rs",
        "c",
    )
    .await;

    let result = SemanticClonesCurrentStateConsumer
        .reconcile(&ctx.request, &ctx.context)
        .await?;
    let metrics = metrics_map(&result);

    assert_eq!(metrics["cleared_paths"], json!(2));
    assert_eq!(
        count_rows(
            &ctx.sqlite_path,
            "SELECT COUNT(*) FROM symbol_semantics_current WHERE repo_id = ?1 AND path = ?2",
            repo_id,
            Some("src/a.rs"),
        ),
        0
    );
    assert_eq!(
        count_rows(
            &ctx.sqlite_path,
            "SELECT COUNT(*) FROM symbol_features_current WHERE repo_id = ?1 AND path = ?2",
            repo_id,
            Some("src/b.rs"),
        ),
        0
    );
    assert_eq!(
        count_rows(
            &ctx.sqlite_path,
            "SELECT COUNT(*) FROM symbol_embeddings_current WHERE repo_id = ?1 AND path = ?2",
            repo_id,
            Some("src/a.rs"),
        ),
        0
    );
    assert_eq!(
        count_rows(
            &ctx.sqlite_path,
            "SELECT COUNT(*) FROM symbol_semantics_current WHERE repo_id = ?1 AND path = ?2",
            repo_id,
            Some("src/c.rs"),
        ),
        1
    );
    assert_eq!(
        count_rows(
            &ctx.sqlite_path,
            "SELECT COUNT(*) FROM symbol_embeddings_current WHERE repo_id = ?1 AND path = ?2",
            repo_id,
            Some("src/c.rs"),
        ),
        2
    );
    Ok(())
}

#[tokio::test]
async fn reconcile_large_delta_chunks_code_and_identity_embedding_jobs() -> Result<()> {
    let repo = tempdir().expect("temp repo");
    let repo_id = "repo-large-delta";
    let artefact_count = 125usize;
    let expected_embedding_job_count = artefact_count.div_ceil(REPO_BACKFILL_MAILBOX_CHUNK_SIZE);
    let artefact_upserts = (0..artefact_count)
        .map(|idx| ChangedArtefact {
            artefact_id: format!("artefact-{idx:03}"),
            symbol_id: format!("symbol-{idx:03}"),
            path: format!("src/file-{idx:03}.rs"),
            canonical_kind: Some("function".to_string()),
            name: format!("function_{idx:03}"),
        })
        .collect::<Vec<_>>();
    let request = request(
        repo.path(),
        repo_id,
        ReconcileMode::MergedDelta,
        Vec::new(),
        Vec::new(),
        artefact_upserts,
        Vec::new(),
    );
    let workplane = CapturingWorkplaneGateway::default();
    let ctx = test_context(config_root(None, Some("code"), None), workplane, request).await?;

    let result = SemanticClonesCurrentStateConsumer
        .reconcile(&ctx.request, &ctx.context)
        .await?;
    let jobs = ctx.workplane.jobs();
    let metrics = metrics_map(&result);
    let code_jobs = jobs
        .iter()
        .filter(|job| job.mailbox_name == SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX)
        .collect::<Vec<_>>();
    let identity_jobs = jobs
        .iter()
        .filter(|job| job.mailbox_name == SEMANTIC_CLONES_IDENTITY_EMBEDDING_MAILBOX)
        .collect::<Vec<_>>();

    assert_eq!(code_jobs.len(), expected_embedding_job_count);
    assert_eq!(identity_jobs.len(), expected_embedding_job_count);
    assert!(code_jobs.iter().all(|job| {
        crate::capability_packs::semantic_clones::workplane::payload_is_repo_backfill(&job.payload)
    }));
    assert!(identity_jobs.iter().all(|job| {
        crate::capability_packs::semantic_clones::workplane::payload_is_repo_backfill(&job.payload)
    }));
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
        artefact_count as u64
    );
    assert_eq!(
        metrics["enqueued_code_embedding_jobs"],
        json!(expected_embedding_job_count)
    );
    assert_eq!(
        metrics["enqueued_identity_embedding_jobs"],
        json!(expected_embedding_job_count)
    );
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
        crate::capability_packs::semantic_clones::workplane::payload_repo_backfill_artefact_ids(
            &jobs[0].payload,
        ),
        Some(Vec::new()),
        "empty eligible summary backfill should stay explicit instead of becoming repo-wide",
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
        crate::capability_packs::semantic_clones::workplane::payload_repo_backfill_artefact_ids(
            &jobs[1].payload,
        ),
        Some(Vec::new()),
        "empty eligible embedding backfill should stay explicit instead of becoming repo-wide",
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
    for key in [
        "clear_current_projection_ms",
        "load_backfill_ids_ms",
        "build_jobs_ms",
        "enqueue_jobs_ms",
        "total_ms",
    ] {
        assert!(
            metrics
                .get(key)
                .and_then(serde_json::Value::as_u64)
                .is_some(),
            "missing numeric metric {key}: {metrics:?}"
        );
    }
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

#[tokio::test]
async fn full_reconcile_backfill_skips_non_code_analysis_mode() -> Result<()> {
    let repo = tempdir().expect("temp repo");
    let repo_id = "repo-analysis-mode";
    let ctx = test_context(
        config_root(None, Some("code"), None),
        CapturingWorkplaneGateway::default(),
        request(
            repo.path(),
            repo_id,
            ReconcileMode::FullReconcile,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
        ),
    )
    .await?;
    seed_current_file_state(&ctx.sqlite_path, repo_id, "src/lib.rs", "code", "rust");
    seed_current_file_state(&ctx.sqlite_path, repo_id, "README.md", "text", "markdown");
    seed_current_artefact(
        &ctx.sqlite_path,
        repo_id,
        "src/lib.rs",
        "code-1",
        "callable",
    );
    seed_current_artefact(&ctx.sqlite_path, repo_id, "README.md", "doc-1", "file");

    let ids = current_repo_backfill_artefact_ids(ctx.storage.as_ref(), repo_id).await?;

    assert_eq!(ids, vec!["code-1".to_string()]);
    Ok(())
}
