use std::io::Write;
use std::time::Duration;

use anyhow::{Result, bail};
use tokio::sync::{mpsc, oneshot};

use crate::devql_transport::SlimCliRepoScope;

use super::QueuedEmbeddingsBootstrapTask;

#[path = "progress/render.rs"]
mod render;
#[path = "progress/tasks.rs"]
mod tasks;

use render::InitProgressRenderer;
use tasks::{
    current_embedding_queue_snapshot, current_summary_queue_snapshot, refresh_init_progress_task,
};

const INIT_PROGRESS_POLL_INTERVAL: Duration = Duration::from_secs(1);
const INIT_PROGRESS_TICK_INTERVAL: Duration = Duration::from_millis(120);
const INIT_POST_TOP_QUEUE_GRACE_PERIOD: Duration = Duration::from_secs(2);
const INIT_SPINNER_FRAMES: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
const SUCCESS_GREEN_HEX: &str = "#22c55e";

pub(super) struct InitProgressOptions {
    pub(super) show_sync: bool,
    pub(super) show_ingest: bool,
    pub(super) enqueue_ingest_after_sync: bool,
    pub(super) ingest_backfill: usize,
    pub(super) queued_embeddings_bootstrap: Option<QueuedEmbeddingsBootstrapTask>,
    pub(super) prepared_summary_setup: Option<crate::cli::inference::PreparedSummarySetupPlan>,
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
    WaitingForQueue {
        baseline_total: u64,
        completed_floor: u64,
        completed_jobs: u64,
        failed_jobs: u64,
    },
    QueueComplete {
        failed_jobs: u64,
        baseline_total: u64,
    },
    BootstrapFailed(crate::cli::devql::graphql::TaskGraphqlRecord),
    Hidden,
}

struct SummarySetupRunner {
    progress_rx: mpsc::UnboundedReceiver<crate::cli::inference::SummarySetupProgress>,
    result_rx: oneshot::Receiver<Result<crate::cli::inference::SummarySetupExecutionResult>>,
}

enum SummaryProgressState {
    Running(crate::cli::inference::SummarySetupProgress),
    Queue {
        result: crate::cli::inference::SummarySetupExecutionResult,
        snapshot: EmbeddingQueueSnapshot,
        baseline_total: u64,
        completed_floor: u64,
    },
    WaitingForQueue {
        result: crate::cli::inference::SummarySetupExecutionResult,
        baseline_total: u64,
        completed_floor: u64,
        completed_jobs: u64,
        failed_jobs: u64,
    },
    Complete {
        result: crate::cli::inference::SummarySetupExecutionResult,
        failed_jobs: u64,
        baseline_total: u64,
    },
    Failed {
        progress: crate::cli::inference::SummarySetupProgress,
        error: String,
    },
    Hidden,
}

#[derive(Clone, Copy)]
struct InitChecklistState {
    show_sync: bool,
    show_ingest: bool,
    show_embeddings: bool,
    show_summaries: bool,
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
    let mut summary_runner = options
        .prepared_summary_setup
        .map(|plan| spawn_summary_setup(&scope.repo_root, plan));
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
    let mut summary_state = if summary_runner.is_some() {
        SummaryProgressState::Running(crate::cli::inference::SummarySetupProgress::default())
    } else {
        SummaryProgressState::Hidden
    };
    let mut summary_backfill_enqueued = false;
    let mut checklist = InitChecklistState {
        show_sync: options.show_sync,
        show_ingest: options.show_ingest,
        show_embeddings: options.queued_embeddings_bootstrap.is_some(),
        show_summaries: summary_runner.is_some(),
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
    renderer.render(
        out,
        checklist,
        top_task.as_ref(),
        &bottom_state,
        &summary_state,
    )?;

    let mut poll_interval = tokio::time::interval(INIT_PROGRESS_POLL_INTERVAL);
    let mut render_tick = tokio::time::interval(INIT_PROGRESS_TICK_INTERVAL);
    let top_pipeline_has_work = checklist.show_sync || checklist.show_ingest;
    let mut top_pipeline_completed_at = None::<std::time::Instant>;
    loop {
        tokio::select! {
            _ = render_tick.tick(), if renderer.is_interactive() => {
                renderer.tick(
                    out,
                    checklist,
                    top_task.as_ref(),
                    &bottom_state,
                    &summary_state,
                )?;
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

                let top_pipeline_complete = init_pipeline_complete(
                    checklist,
                    top_task.is_none(),
                    enqueue_ingest_after_sync,
                );
                if top_pipeline_complete {
                    if top_pipeline_has_work && top_pipeline_completed_at.is_none() {
                        top_pipeline_completed_at = Some(std::time::Instant::now());
                    }
                } else {
                    top_pipeline_completed_at = None;
                }
                let allow_empty_queue_completion = top_pipeline_complete
                    && (!top_pipeline_has_work
                        || top_pipeline_completed_at
                            .is_some_and(|started| started.elapsed() >= INIT_POST_TOP_QUEUE_GRACE_PERIOD));

                if let Some(runner) = summary_runner.as_mut() {
                    loop {
                        match runner.progress_rx.try_recv() {
                            Ok(progress) => summary_state = SummaryProgressState::Running(progress),
                            Err(mpsc::error::TryRecvError::Empty) => break,
                            Err(mpsc::error::TryRecvError::Disconnected) => break,
                        }
                    }

                    match runner.result_rx.try_recv() {
                        Ok(Ok(result)) => {
                            let queue_snapshot = if summary_queue_tracking_enabled(&result) {
                                current_summary_queue_snapshot(&scope.repo_root).await?
                            } else {
                                None
                            };
                            let completed_floor =
                                queue_snapshot.map(|snapshot| snapshot.completed).unwrap_or_default();
                            summary_state = update_summary_queue_state(
                                result,
                                queue_snapshot,
                                allow_empty_queue_completion,
                                0,
                                completed_floor,
                                0,
                                0,
                            );
                            summary_runner = None;
                        }
                        Ok(Err(err)) => {
                            let progress = match &summary_state {
                                SummaryProgressState::Running(progress) => progress.clone(),
                                SummaryProgressState::Failed { progress, .. } => progress.clone(),
                                _ => crate::cli::inference::SummarySetupProgress::default(),
                            };
                            summary_state = SummaryProgressState::Failed {
                                progress,
                                error: format!("{err:#}"),
                            };
                            summary_runner = None;
                        }
                        Err(oneshot::error::TryRecvError::Empty) => {}
                        Err(oneshot::error::TryRecvError::Closed) => {
                            let progress = match &summary_state {
                                SummaryProgressState::Running(progress) => progress.clone(),
                                SummaryProgressState::Failed { progress, .. } => progress.clone(),
                                _ => crate::cli::inference::SummarySetupProgress::default(),
                            };
                            summary_state = SummaryProgressState::Failed {
                                progress,
                                error: "semantic summary setup exited unexpectedly".to_string(),
                            };
                            summary_runner = None;
                        }
                    }
                }

                bottom_state = match bottom_state {
                    BottomProgressState::Bootstrap(current_task) => {
                        let bootstrap_scope = bootstrap_scope.as_ref().unwrap_or(scope);
                        match refresh_init_progress_task(bootstrap_scope, &current_task).await? {
                            Some(refreshed) if refreshed.is_terminal() => {
                                if refreshed.status.eq_ignore_ascii_case("completed") {
                                    let queue_snapshot =
                                        current_embedding_queue_snapshot(&scope.repo_root).await?;
                                    let completed_floor = queue_snapshot
                                        .map(|snapshot| snapshot.completed)
                                        .unwrap_or_default();
                                    update_embeddings_queue_state(
                                        queue_snapshot,
                                        allow_empty_queue_completion,
                                        0,
                                        completed_floor,
                                        0,
                                        0,
                                    )
                                } else {
                                    BottomProgressState::BootstrapFailed(refreshed)
                                }
                            }
                            Some(refreshed) => BottomProgressState::Bootstrap(refreshed),
                            None => {
                                let queue_snapshot =
                                    current_embedding_queue_snapshot(&scope.repo_root).await?;
                                let completed_floor = queue_snapshot
                                    .map(|snapshot| snapshot.completed)
                                    .unwrap_or_default();
                                update_embeddings_queue_state(
                                    queue_snapshot,
                                    allow_empty_queue_completion,
                                    0,
                                    completed_floor,
                                    0,
                                    0,
                                )
                            }
                        }
                    }
                    BottomProgressState::Queue {
                        snapshot,
                        baseline_total,
                        completed_floor,
                    } => update_embeddings_queue_state(
                        current_embedding_queue_snapshot(&scope.repo_root).await?,
                        allow_empty_queue_completion,
                        baseline_total,
                        completed_floor,
                        snapshot.completed,
                        snapshot.failed,
                    ),
                    BottomProgressState::WaitingForQueue {
                        baseline_total,
                        completed_floor,
                        completed_jobs,
                        failed_jobs,
                    } => update_embeddings_queue_state(
                        current_embedding_queue_snapshot(&scope.repo_root).await?,
                        allow_empty_queue_completion,
                        baseline_total,
                        completed_floor,
                        completed_jobs,
                        failed_jobs,
                    ),
                    other => other,
                };

                summary_state = match summary_state {
                    SummaryProgressState::Queue {
                        result,
                        snapshot,
                        baseline_total,
                        completed_floor,
                    } => {
                        let queue_snapshot = current_summary_queue_snapshot(&scope.repo_root).await?;
                        if !summary_backfill_enqueued
                            && summary_repo_backfill_needed(
                                &result,
                                queue_snapshot,
                                allow_empty_queue_completion,
                                completed_floor,
                                snapshot.completed,
                                snapshot.failed,
                            )
                        {
                            match crate::capability_packs::semantic_clones::workplane::enqueue_summary_refresh_repo_backfill_for_repo(&scope.repo_root) {
                                Ok(()) => {
                                    summary_backfill_enqueued = true;
                                    let queue_snapshot = current_summary_queue_snapshot(&scope.repo_root).await?;
                                    update_summary_queue_state(
                                        result,
                                        queue_snapshot,
                                        false,
                                        baseline_total,
                                        completed_floor,
                                        snapshot.completed,
                                        snapshot.failed,
                                    )
                                }
                                Err(err) => SummaryProgressState::Failed {
                                    progress: crate::cli::inference::SummarySetupProgress::default(),
                                    error: format!("failed to queue semantic summary catch-up: {err:#}"),
                                },
                            }
                        } else {
                            update_summary_queue_state(
                                result,
                                queue_snapshot,
                                allow_empty_queue_completion,
                                baseline_total,
                                completed_floor,
                                snapshot.completed,
                                snapshot.failed,
                            )
                        }
                    }
                    SummaryProgressState::WaitingForQueue {
                        result,
                        baseline_total,
                        completed_floor,
                        completed_jobs,
                        failed_jobs,
                    } => {
                        let queue_snapshot = current_summary_queue_snapshot(&scope.repo_root).await?;
                        if !summary_backfill_enqueued
                            && summary_repo_backfill_needed(
                                &result,
                                queue_snapshot,
                                allow_empty_queue_completion,
                                completed_floor,
                                completed_jobs,
                                failed_jobs,
                            )
                        {
                            match crate::capability_packs::semantic_clones::workplane::enqueue_summary_refresh_repo_backfill_for_repo(&scope.repo_root) {
                                Ok(()) => {
                                    summary_backfill_enqueued = true;
                                    let queue_snapshot = current_summary_queue_snapshot(&scope.repo_root).await?;
                                    update_summary_queue_state(
                                        result,
                                        queue_snapshot,
                                        false,
                                        baseline_total,
                                        completed_floor,
                                        completed_jobs,
                                        failed_jobs,
                                    )
                                }
                                Err(err) => SummaryProgressState::Failed {
                                    progress: crate::cli::inference::SummarySetupProgress::default(),
                                    error: format!("failed to queue semantic summary catch-up: {err:#}"),
                                },
                            }
                        } else {
                            update_summary_queue_state(
                                result,
                                queue_snapshot,
                                allow_empty_queue_completion,
                                baseline_total,
                                completed_floor,
                                completed_jobs,
                                failed_jobs,
                            )
                        }
                    }
                    other => other,
                };

                renderer.render(
                    out,
                    checklist,
                    top_task.as_ref(),
                    &bottom_state,
                    &summary_state,
                )?;
                if top_task.is_none()
                    && embeddings_state_finished(&bottom_state)
                    && summary_state_finished(&summary_state)
                {
                    renderer.finish(out)?;
                    return Ok(());
                }
            }
        }
    }
}

fn spawn_summary_setup(
    repo_root: &std::path::Path,
    plan: crate::cli::inference::PreparedSummarySetupPlan,
) -> SummarySetupRunner {
    let repo_root = repo_root.to_path_buf();
    let (progress_tx, progress_rx) = mpsc::unbounded_channel();
    let (result_tx, result_rx) = oneshot::channel();
    tokio::task::spawn_blocking(move || {
        let result = crate::cli::inference::execute_prepared_summary_setup_with_progress(
            &repo_root,
            plan,
            |progress| {
                progress_tx.send(progress).map_err(|_| {
                    anyhow::anyhow!("semantic summary setup progress receiver dropped")
                })?;
                Ok(())
            },
        );
        let _ = result_tx.send(result);
    });
    SummarySetupRunner {
        progress_rx,
        result_rx,
    }
}

fn init_pipeline_complete(
    checklist: InitChecklistState,
    top_task_is_none: bool,
    enqueue_ingest_after_sync: bool,
) -> bool {
    top_task_is_none
        && !enqueue_ingest_after_sync
        && (!checklist.show_sync || checklist.sync_complete)
        && (!checklist.show_ingest || checklist.ingest_complete)
}

fn summary_queue_tracking_enabled(
    result: &crate::cli::inference::SummarySetupExecutionResult,
) -> bool {
    matches!(
        result.outcome,
        crate::cli::inference::SummarySetupOutcome::Configured { .. }
    )
}

fn summary_repo_backfill_needed(
    result: &crate::cli::inference::SummarySetupExecutionResult,
    snapshot: Option<EmbeddingQueueSnapshot>,
    allow_empty_completion: bool,
    completed_floor: u64,
    completed_jobs: u64,
    failed_jobs: u64,
) -> bool {
    if !summary_queue_tracking_enabled(result)
        || !allow_empty_completion
        || completed_jobs > 0
        || failed_jobs > 0
    {
        return false;
    }

    match snapshot {
        Some(snapshot) => {
            snapshot.remaining() == 0
                && snapshot.failed == 0
                && snapshot.completed <= completed_floor
        }
        None => true,
    }
}

fn update_embeddings_queue_state(
    snapshot: Option<EmbeddingQueueSnapshot>,
    allow_empty_completion: bool,
    baseline_total: u64,
    completed_floor: u64,
    completed_jobs: u64,
    failed_jobs: u64,
) -> BottomProgressState {
    match snapshot {
        Some(snapshot) => {
            let completed_since_start = snapshot.completed.saturating_sub(completed_floor);
            let baseline_total = baseline_total.max(completed_since_start + snapshot.remaining());
            let failed_jobs = snapshot.failed;
            if snapshot.remaining() > 0 {
                BottomProgressState::Queue {
                    snapshot: EmbeddingQueueSnapshot {
                        completed: completed_since_start,
                        ..snapshot
                    },
                    baseline_total,
                    completed_floor,
                }
            } else if allow_empty_completion {
                BottomProgressState::QueueComplete {
                    failed_jobs,
                    baseline_total,
                }
            } else {
                BottomProgressState::WaitingForQueue {
                    baseline_total,
                    completed_floor,
                    completed_jobs: completed_since_start,
                    failed_jobs,
                }
            }
        }
        None if allow_empty_completion => BottomProgressState::QueueComplete {
            failed_jobs,
            baseline_total,
        },
        None => BottomProgressState::WaitingForQueue {
            baseline_total,
            completed_floor,
            completed_jobs,
            failed_jobs,
        },
    }
}

fn update_summary_queue_state(
    result: crate::cli::inference::SummarySetupExecutionResult,
    snapshot: Option<EmbeddingQueueSnapshot>,
    allow_empty_completion: bool,
    baseline_total: u64,
    completed_floor: u64,
    completed_jobs: u64,
    failed_jobs: u64,
) -> SummaryProgressState {
    if !summary_queue_tracking_enabled(&result) {
        return SummaryProgressState::Complete {
            result,
            failed_jobs: 0,
            baseline_total: 0,
        };
    }

    match snapshot {
        Some(snapshot) => {
            let completed_since_start = snapshot.completed.saturating_sub(completed_floor);
            let baseline_total = baseline_total.max(completed_since_start + snapshot.remaining());
            let failed_jobs = snapshot.failed;
            if snapshot.remaining() > 0 {
                SummaryProgressState::Queue {
                    result,
                    snapshot: EmbeddingQueueSnapshot {
                        completed: completed_since_start,
                        ..snapshot
                    },
                    baseline_total,
                    completed_floor,
                }
            } else if allow_empty_completion {
                SummaryProgressState::Complete {
                    result,
                    failed_jobs,
                    baseline_total,
                }
            } else {
                SummaryProgressState::WaitingForQueue {
                    result,
                    baseline_total,
                    completed_floor,
                    completed_jobs: completed_since_start,
                    failed_jobs,
                }
            }
        }
        None if allow_empty_completion => SummaryProgressState::Complete {
            result,
            failed_jobs,
            baseline_total,
        },
        None => SummaryProgressState::WaitingForQueue {
            result,
            baseline_total,
            completed_floor,
            completed_jobs,
            failed_jobs,
        },
    }
}

fn embeddings_state_finished(state: &BottomProgressState) -> bool {
    matches!(
        state,
        BottomProgressState::Hidden
            | BottomProgressState::QueueComplete { .. }
            | BottomProgressState::BootstrapFailed(_)
    )
}

fn summary_state_finished(state: &SummaryProgressState) -> bool {
    matches!(
        state,
        SummaryProgressState::Hidden
            | SummaryProgressState::Complete { .. }
            | SummaryProgressState::Failed { .. }
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn configured_summary_result() -> crate::cli::inference::SummarySetupExecutionResult {
        crate::cli::inference::SummarySetupExecutionResult {
            outcome: crate::cli::inference::SummarySetupOutcome::Configured {
                model_name: "ministral-3:3b".to_string(),
            },
            message: "Configured semantic summaries to use Ollama model `ministral-3:3b`."
                .to_string(),
        }
    }

    #[test]
    fn embeddings_wait_for_top_pipeline_before_marking_complete() {
        let state = update_embeddings_queue_state(None, false, 0, 0, 0, 0);
        assert!(matches!(state, BottomProgressState::WaitingForQueue { .. }));

        let state = update_embeddings_queue_state(None, true, 0, 0, 0, 0);
        assert!(matches!(state, BottomProgressState::QueueComplete { .. }));
    }

    #[test]
    fn configured_summaries_wait_for_top_pipeline_before_marking_complete() {
        let state =
            update_summary_queue_state(configured_summary_result(), None, false, 0, 0, 0, 0);
        assert!(matches!(
            state,
            SummaryProgressState::WaitingForQueue { .. }
        ));

        let state = update_summary_queue_state(configured_summary_result(), None, true, 0, 0, 0, 0);
        assert!(matches!(state, SummaryProgressState::Complete { .. }));
    }

    #[test]
    fn runtime_only_summary_setup_completes_without_waiting_for_queue_work() {
        let state = update_summary_queue_state(
            crate::cli::inference::SummarySetupExecutionResult {
                outcome: crate::cli::inference::SummarySetupOutcome::InstalledRuntimeOnly,
                message: "Installed `bitloops-inference`; skipped semantic summary setup because Ollama is not running.".to_string(),
            },
            None,
            false,
            0,
            0,
            0,
            0,
        );

        assert!(matches!(state, SummaryProgressState::Complete { .. }));
    }

    #[test]
    fn configured_summaries_request_repo_backfill_when_no_summary_jobs_appeared() {
        assert!(summary_repo_backfill_needed(
            &configured_summary_result(),
            None,
            true,
            0,
            0,
            0,
        ));
        assert!(summary_repo_backfill_needed(
            &configured_summary_result(),
            Some(EmbeddingQueueSnapshot {
                pending: 0,
                running: 0,
                failed: 0,
                completed: 0,
            }),
            true,
            0,
            0,
            0,
        ));
    }

    #[test]
    fn configured_summaries_skip_repo_backfill_once_summary_work_exists() {
        assert!(!summary_repo_backfill_needed(
            &configured_summary_result(),
            Some(EmbeddingQueueSnapshot {
                pending: 1,
                running: 0,
                failed: 0,
                completed: 0,
            }),
            true,
            0,
            0,
            0,
        ));
        assert!(!summary_repo_backfill_needed(
            &configured_summary_result(),
            Some(EmbeddingQueueSnapshot {
                pending: 0,
                running: 0,
                failed: 0,
                completed: 3,
            }),
            true,
            1,
            2,
            0,
        ));
        assert!(!summary_repo_backfill_needed(
            &configured_summary_result(),
            None,
            false,
            0,
            0,
            0,
        ));
    }
}
