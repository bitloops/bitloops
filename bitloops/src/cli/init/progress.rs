use std::io::Write;
use std::time::Duration;

use anyhow::{Result, bail};

use crate::devql_transport::SlimCliRepoScope;

use super::QueuedEmbeddingsBootstrapTask;

#[path = "progress/render.rs"]
mod render;
#[path = "progress/tasks.rs"]
mod tasks;

use render::InitProgressRenderer;
use tasks::{current_embedding_queue_snapshot, refresh_init_progress_task};

const INIT_PROGRESS_POLL_INTERVAL: Duration = Duration::from_secs(1);
const INIT_PROGRESS_TICK_INTERVAL: Duration = Duration::from_millis(120);
const INIT_SPINNER_FRAMES: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
const SUCCESS_GREEN_HEX: &str = "#22c55e";

pub(super) struct InitProgressOptions {
    pub(super) show_sync: bool,
    pub(super) show_ingest: bool,
    pub(super) enqueue_ingest_after_sync: bool,
    pub(super) ingest_backfill: usize,
    pub(super) queued_embeddings_bootstrap: Option<QueuedEmbeddingsBootstrapTask>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct EmbeddingQueueSnapshot {
    pending: u64,
    running: u64,
    failed: u64,
    completed: u64,
}

impl EmbeddingQueueSnapshot {
    fn remaining(self) -> u64 {
        self.pending + self.running
    }
}

enum BottomProgressState {
    Bootstrap(crate::cli::devql::graphql::TaskGraphqlRecord),
    Queue {
        snapshot: EmbeddingQueueSnapshot,
        baseline_total: u64,
        completed_floor: u64,
    },
    QueueComplete {
        failed_jobs: u64,
        baseline_total: u64,
    },
    BootstrapFailed(crate::cli::devql::graphql::TaskGraphqlRecord),
    Hidden,
}

#[derive(Clone, Copy)]
struct InitChecklistState {
    show_sync: bool,
    show_ingest: bool,
    show_embeddings: bool,
    sync_complete: bool,
    ingest_complete: bool,
}

pub(super) async fn run_dual_init_progress(
    out: &mut dyn Write,
    scope: &SlimCliRepoScope,
    mut top_task: Option<crate::cli::devql::graphql::TaskGraphqlRecord>,
    options: InitProgressOptions,
) -> Result<()> {
    let bootstrap_scope = options
        .queued_embeddings_bootstrap
        .as_ref()
        .map(|task| task.scope.clone());
    let mut enqueue_ingest_after_sync = options.enqueue_ingest_after_sync;
    let mut bottom_state = if let Some(bootstrap) = options.queued_embeddings_bootstrap.as_ref() {
        match crate::cli::devql::graphql::query_task_via_graphql(
            &bootstrap.scope,
            bootstrap.task_id.as_str(),
        )
        .await?
        {
            Some(task) => BottomProgressState::Bootstrap(task),
            None => BottomProgressState::Hidden,
        }
    } else {
        BottomProgressState::Hidden
    };
    let mut checklist = InitChecklistState {
        show_sync: options.show_sync,
        show_ingest: options.show_ingest,
        show_embeddings: options.queued_embeddings_bootstrap.is_some(),
        sync_complete: false,
        ingest_complete: false,
    };
    let mut renderer = InitProgressRenderer::new();
    writeln!(
        out,
        "{}",
        render::fit_init_plain_line(
            "Bitloops is currently updating its local database with the following:",
            renderer.terminal_width(),
        )
    )?;
    writeln!(out)?;
    out.flush()?;
    renderer.render(out, checklist, top_task.as_ref(), &bottom_state)?;

    let mut poll_interval = tokio::time::interval(INIT_PROGRESS_POLL_INTERVAL);
    let mut render_tick = tokio::time::interval(INIT_PROGRESS_TICK_INTERVAL);
    loop {
        tokio::select! {
            _ = render_tick.tick(), if renderer.is_interactive() => {
                renderer.tick(out, checklist, top_task.as_ref(), &bottom_state)?;
            }
            _ = poll_interval.tick() => {
                if let Some(current) = top_task.clone() {
                    match refresh_init_progress_task(scope, &current).await? {
                        Some(refreshed) if refreshed.is_terminal() => {
                            if refreshed.status.eq_ignore_ascii_case("completed") {
                                if refreshed.is_sync() {
                                    checklist.sync_complete = true;
                                } else if refreshed.is_ingest() {
                                    checklist.ingest_complete = true;
                                }
                                if refreshed.is_sync() && enqueue_ingest_after_sync {
                                    let (ingest_task, _merged) = crate::cli::devql::graphql::enqueue_ingest_task_via_graphql(
                                        scope,
                                        Some(options.ingest_backfill),
                                        false,
                                    ).await?;
                                    top_task = Some(ingest_task);
                                    enqueue_ingest_after_sync = false;
                                } else {
                                    top_task = None;
                                }
                            } else if let Some(error) = refreshed.error.as_ref() {
                                renderer.finish(out)?;
                                bail!("task {} failed: {error}", refreshed.task_id);
                            } else {
                                renderer.finish(out)?;
                                bail!(
                                    "task {} ended with status {}",
                                    refreshed.task_id,
                                    refreshed.status
                                );
                            }
                        }
                        Some(refreshed) => top_task = Some(refreshed),
                        None if current.is_sync() && enqueue_ingest_after_sync => {
                            checklist.sync_complete = true;
                            let (ingest_task, _merged) = crate::cli::devql::graphql::enqueue_ingest_task_via_graphql(
                                scope,
                                Some(options.ingest_backfill),
                                false,
                            ).await?;
                            top_task = Some(ingest_task);
                            enqueue_ingest_after_sync = false;
                        }
                        None => {
                            if current.is_sync() {
                                checklist.sync_complete = true;
                            } else if current.is_ingest() {
                                checklist.ingest_complete = true;
                            }
                            top_task = None;
                        }
                    }
                }

                bottom_state = match bottom_state {
                    BottomProgressState::Bootstrap(current_task) => {
                        let bootstrap_scope = bootstrap_scope.as_ref().unwrap_or(scope);
                        match refresh_init_progress_task(bootstrap_scope, &current_task).await? {
                            Some(refreshed) if refreshed.is_terminal() => {
                                if refreshed.status.eq_ignore_ascii_case("completed") {
                                    if let Some(snapshot) =
                                        current_embedding_queue_snapshot(&scope.repo_root).await?
                                    {
                                        if snapshot.remaining() > 0 || snapshot.failed > 0 {
                                            let completed_floor = snapshot.completed;
                                            BottomProgressState::Queue {
                                                baseline_total: snapshot.remaining(),
                                                completed_floor,
                                                snapshot: EmbeddingQueueSnapshot {
                                                    completed: 0,
                                                    ..snapshot
                                                },
                                            }
                                        } else {
                                            BottomProgressState::QueueComplete {
                                                failed_jobs: 0,
                                                baseline_total: 0,
                                            }
                                        }
                                    } else {
                                        BottomProgressState::Hidden
                                    }
                                } else {
                                    BottomProgressState::BootstrapFailed(refreshed)
                                }
                            }
                            Some(refreshed) => BottomProgressState::Bootstrap(refreshed),
                            None => {
                                if let Some(snapshot) =
                                    current_embedding_queue_snapshot(&scope.repo_root).await?
                                {
                                    if snapshot.remaining() > 0 || snapshot.failed > 0 {
                                        let completed_floor = snapshot.completed;
                                        BottomProgressState::Queue {
                                            baseline_total: snapshot.remaining(),
                                            completed_floor,
                                            snapshot: EmbeddingQueueSnapshot {
                                                completed: 0,
                                                ..snapshot
                                            },
                                        }
                                    } else {
                                        BottomProgressState::QueueComplete {
                                            failed_jobs: 0,
                                            baseline_total: 0,
                                        }
                                    }
                                } else {
                                    BottomProgressState::Hidden
                                }
                            }
                        }
                    }
                    BottomProgressState::Queue {
                        snapshot: _,
                        baseline_total,
                        completed_floor,
                    } => {
                        if let Some(snapshot) =
                            current_embedding_queue_snapshot(&scope.repo_root).await?
                        {
                            let completed_since_start =
                                snapshot.completed.saturating_sub(completed_floor);
                            let baseline_total =
                                baseline_total.max(completed_since_start + snapshot.remaining());
                            let snapshot = EmbeddingQueueSnapshot {
                                completed: completed_since_start,
                                ..snapshot
                            };
                            if snapshot.remaining() == 0 {
                                BottomProgressState::QueueComplete {
                                    failed_jobs: snapshot.failed,
                                    baseline_total,
                                }
                            } else {
                                BottomProgressState::Queue {
                                    snapshot,
                                    baseline_total,
                                    completed_floor,
                                }
                            }
                        } else {
                            BottomProgressState::Hidden
                        }
                    }
                    other => other,
                };

                renderer.render(out, checklist, top_task.as_ref(), &bottom_state)?;
                if top_task.is_none()
                    && matches!(
                        bottom_state,
                        BottomProgressState::Hidden
                            | BottomProgressState::QueueComplete { .. }
                            | BottomProgressState::BootstrapFailed(_)
                    )
                {
                    renderer.finish(out)?;
                    return Ok(());
                }
            }
        }
    }
}
