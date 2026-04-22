use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::Result;

use crate::capability_packs::semantic_clones::types::{
    SEMANTIC_CLONES_CLONE_REBUILD_MAILBOX, SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX,
    SEMANTIC_CLONES_IDENTITY_EMBEDDING_MAILBOX, SEMANTIC_CLONES_SUMMARY_EMBEDDING_MAILBOX,
    SEMANTIC_CLONES_SUMMARY_REFRESH_MAILBOX,
};
use crate::daemon::enrichment::worker_count::configured_enrichment_worker_budgets_for_repo;
use crate::daemon::types::BlockedMailboxStatus;
use crate::host::capability_host::gateways::CapabilityMailboxStatus;
use crate::host::runtime_store::DaemonSqliteRuntimeStore;
use crate::runtime_presentation::{mailbox_label, workplane_pool_label};

use super::types::{
    InitRuntimeWorkplaneMailboxSnapshot, InitRuntimeWorkplanePoolSnapshot,
    InitRuntimeWorkplaneSnapshot,
};

pub(crate) fn workplane_snapshot_from_mailboxes(
    repo_root: &Path,
    mailboxes: &BTreeMap<String, CapabilityMailboxStatus>,
    blocked_mailboxes: &[BlockedMailboxStatus],
) -> InitRuntimeWorkplaneSnapshot {
    let budgets = configured_enrichment_worker_budgets_for_repo(repo_root);
    let blocked_by_mailbox = blocked_mailboxes
        .iter()
        .map(|blocked| (blocked.mailbox_name.as_str(), blocked.reason.as_str()))
        .collect::<BTreeMap<_, _>>();
    let mut snapshot_mailboxes = mailboxes
        .iter()
        .map(
            |(mailbox_name, status)| InitRuntimeWorkplaneMailboxSnapshot {
                mailbox_name: mailbox_name.clone(),
                display_name: mailbox_label(mailbox_name).to_string(),
                pending_jobs: status.pending_jobs,
                running_jobs: status.running_jobs,
                failed_jobs: status.failed_jobs,
                completed_recent_jobs: status.completed_recent_jobs,
                pending_cursor_runs: status.pending_cursor_runs,
                running_cursor_runs: status.running_cursor_runs,
                failed_cursor_runs: status.failed_cursor_runs,
                completed_recent_cursor_runs: status.completed_recent_cursor_runs,
                intent_active: status.intent_active,
                blocked_reason: blocked_by_mailbox
                    .get(mailbox_name.as_str())
                    .map(|reason| (*reason).to_string()),
            },
        )
        .collect::<Vec<_>>();
    snapshot_mailboxes.sort_by(|left, right| left.mailbox_name.cmp(&right.mailbox_name));
    let summary_mailbox = snapshot_mailboxes
        .iter()
        .find(|mailbox| mailbox.mailbox_name == SEMANTIC_CLONES_SUMMARY_REFRESH_MAILBOX);
    let code_embedding_mailbox = snapshot_mailboxes
        .iter()
        .find(|mailbox| mailbox.mailbox_name == SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX);
    let identity_embedding_mailbox = snapshot_mailboxes
        .iter()
        .find(|mailbox| mailbox.mailbox_name == SEMANTIC_CLONES_IDENTITY_EMBEDDING_MAILBOX);
    let summary_embedding_mailbox = snapshot_mailboxes
        .iter()
        .find(|mailbox| mailbox.mailbox_name == SEMANTIC_CLONES_SUMMARY_EMBEDDING_MAILBOX);
    let clone_rebuild_mailbox = snapshot_mailboxes
        .iter()
        .find(|mailbox| mailbox.mailbox_name == SEMANTIC_CLONES_CLONE_REBUILD_MAILBOX);
    let pools = vec![
        InitRuntimeWorkplanePoolSnapshot {
            pool_name: "summary_refresh".to_string(),
            display_name: workplane_pool_label("summary_refresh").to_string(),
            worker_budget: budgets.summary_refresh as u64,
            active_workers: summary_mailbox
                .map(|mailbox| mailbox.running_jobs)
                .unwrap_or_default(),
            pending_jobs: summary_mailbox
                .map(|mailbox| mailbox.pending_jobs)
                .unwrap_or_default(),
            running_jobs: summary_mailbox
                .map(|mailbox| mailbox.running_jobs)
                .unwrap_or_default(),
            failed_jobs: summary_mailbox
                .map(|mailbox| mailbox.failed_jobs)
                .unwrap_or_default(),
            completed_recent_jobs: summary_mailbox
                .map(|mailbox| mailbox.completed_recent_jobs)
                .unwrap_or_default(),
        },
        InitRuntimeWorkplanePoolSnapshot {
            pool_name: "embeddings".to_string(),
            display_name: workplane_pool_label("embeddings").to_string(),
            worker_budget: budgets.embeddings as u64,
            active_workers: code_embedding_mailbox
                .map(|mailbox| mailbox.running_jobs)
                .unwrap_or_default()
                + identity_embedding_mailbox
                    .map(|mailbox| mailbox.running_jobs)
                    .unwrap_or_default()
                + summary_embedding_mailbox
                    .map(|mailbox| mailbox.running_jobs)
                    .unwrap_or_default(),
            pending_jobs: code_embedding_mailbox
                .map(|mailbox| mailbox.pending_jobs)
                .unwrap_or_default()
                + identity_embedding_mailbox
                    .map(|mailbox| mailbox.pending_jobs)
                    .unwrap_or_default()
                + summary_embedding_mailbox
                    .map(|mailbox| mailbox.pending_jobs)
                    .unwrap_or_default(),
            running_jobs: code_embedding_mailbox
                .map(|mailbox| mailbox.running_jobs)
                .unwrap_or_default()
                + identity_embedding_mailbox
                    .map(|mailbox| mailbox.running_jobs)
                    .unwrap_or_default()
                + summary_embedding_mailbox
                    .map(|mailbox| mailbox.running_jobs)
                    .unwrap_or_default(),
            failed_jobs: code_embedding_mailbox
                .map(|mailbox| mailbox.failed_jobs)
                .unwrap_or_default()
                + identity_embedding_mailbox
                    .map(|mailbox| mailbox.failed_jobs)
                    .unwrap_or_default()
                + summary_embedding_mailbox
                    .map(|mailbox| mailbox.failed_jobs)
                    .unwrap_or_default(),
            completed_recent_jobs: code_embedding_mailbox
                .map(|mailbox| mailbox.completed_recent_jobs)
                .unwrap_or_default()
                + identity_embedding_mailbox
                    .map(|mailbox| mailbox.completed_recent_jobs)
                    .unwrap_or_default()
                + summary_embedding_mailbox
                    .map(|mailbox| mailbox.completed_recent_jobs)
                    .unwrap_or_default(),
        },
        InitRuntimeWorkplanePoolSnapshot {
            pool_name: "clone_rebuild".to_string(),
            display_name: workplane_pool_label("clone_rebuild").to_string(),
            worker_budget: budgets.clone_rebuild as u64,
            active_workers: clone_rebuild_mailbox
                .map(|mailbox| mailbox.running_jobs)
                .unwrap_or_default(),
            pending_jobs: clone_rebuild_mailbox
                .map(|mailbox| mailbox.pending_jobs)
                .unwrap_or_default(),
            running_jobs: clone_rebuild_mailbox
                .map(|mailbox| mailbox.running_jobs)
                .unwrap_or_default(),
            failed_jobs: clone_rebuild_mailbox
                .map(|mailbox| mailbox.failed_jobs)
                .unwrap_or_default(),
            completed_recent_jobs: clone_rebuild_mailbox
                .map(|mailbox| mailbox.completed_recent_jobs)
                .unwrap_or_default(),
        },
    ];
    InitRuntimeWorkplaneSnapshot {
        pending_jobs: snapshot_mailboxes
            .iter()
            .map(|mailbox| mailbox.pending_jobs)
            .sum(),
        running_jobs: snapshot_mailboxes
            .iter()
            .map(|mailbox| mailbox.running_jobs)
            .sum(),
        failed_jobs: snapshot_mailboxes
            .iter()
            .map(|mailbox| mailbox.failed_jobs)
            .sum(),
        completed_recent_jobs: snapshot_mailboxes
            .iter()
            .map(|mailbox| mailbox.completed_recent_jobs)
            .sum(),
        pools,
        mailboxes: snapshot_mailboxes,
    }
}

pub(crate) fn repo_blocked_mailboxes(
    db_path: PathBuf,
    repo_id: &str,
) -> Result<Vec<BlockedMailboxStatus>> {
    let runtime_store = DaemonSqliteRuntimeStore::open()?;
    let workplane_store = DaemonSqliteRuntimeStore::open_at(db_path)?;
    crate::daemon::enrichment::blocked_mailboxes_for_repo(&workplane_store, &runtime_store, repo_id)
}
