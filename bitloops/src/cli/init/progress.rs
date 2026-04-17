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
    current_code_embedding_artefact_count, current_embedding_queue_snapshot,
    current_summary_queue_snapshot, refresh_init_progress_task,
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
    total: u64,
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
        completion_source: EmbeddingCompletionSource,
    },
    BootstrapFailed(crate::cli::devql::graphql::TaskGraphqlRecord),
    Hidden,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EmbeddingCompletionSource {
    Queue,
    InlineSync,
    NoneRequired,
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
    let initial_code_embedding_artefact_count = if options.queued_embeddings_bootstrap.is_some() {
        current_code_embedding_artefact_count(&scope.repo_root, &scope.repo.repo_id)
            .await
            .unwrap_or_default()
    } else {
        0
    };
    let mut latest_code_embedding_artefact_count = initial_code_embedding_artefact_count;
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
    let mut embeddings_bootstrap_ready = matches!(
        &bottom_state,
        BottomProgressState::Bootstrap(task)
            if task.is_terminal() && task.status.eq_ignore_ascii_case("completed")
    );
    let mut initial_sync_completed = false;
    let mut enqueue_follow_up_sync_after_embeddings_ready = false;
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
                                    if !initial_sync_completed {
                                        initial_sync_completed = true;
                                        if options.queued_embeddings_bootstrap.is_some()
                                            && !embeddings_bootstrap_ready
                                        {
                                            enqueue_follow_up_sync_after_embeddings_ready = true;
                                        }
                                    }
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
                            if !initial_sync_completed {
                                initial_sync_completed = true;
                                if options.queued_embeddings_bootstrap.is_some()
                                    && !embeddings_bootstrap_ready
                                {
                                    enqueue_follow_up_sync_after_embeddings_ready = true;
                                }
                            }
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
                                if !initial_sync_completed {
                                    initial_sync_completed = true;
                                    if options.queued_embeddings_bootstrap.is_some()
                                        && !embeddings_bootstrap_ready
                                    {
                                        enqueue_follow_up_sync_after_embeddings_ready = true;
                                    }
                                }
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
                if checklist.show_embeddings
                    && let Ok(count) =
                        current_code_embedding_artefact_count(&scope.repo_root, &scope.repo.repo_id)
                            .await
                {
                    latest_code_embedding_artefact_count = count;
                }
                let inline_completed_embedding_artefacts = latest_code_embedding_artefact_count
                    .saturating_sub(initial_code_embedding_artefact_count);

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
                                current_summary_queue_snapshot(
                                    &scope.repo_root,
                                    &scope.repo.repo_id,
                                )
                                .await?
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
                                    embeddings_bootstrap_ready = true;
                                    let queue_snapshot = current_embedding_queue_snapshot(
                                        &scope.repo_root,
                                        &scope.repo.repo_id,
                                    )
                                    .await?;
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
                                        inline_completed_embedding_artefacts,
                                    )
                                } else {
                                    BottomProgressState::BootstrapFailed(refreshed)
                                }
                            }
                            Some(refreshed) => BottomProgressState::Bootstrap(refreshed),
                            None => {
                                let queue_snapshot = current_embedding_queue_snapshot(
                                    &scope.repo_root,
                                    &scope.repo.repo_id,
                                )
                                .await?;
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
                                    inline_completed_embedding_artefacts,
                                )
                            }
                        }
                    }
                    BottomProgressState::Queue {
                        snapshot,
                        baseline_total,
                        completed_floor,
                    } => update_embeddings_queue_state(
                        current_embedding_queue_snapshot(&scope.repo_root, &scope.repo.repo_id)
                            .await?,
                        allow_empty_queue_completion,
                        baseline_total,
                        completed_floor,
                        snapshot.completed,
                        snapshot.failed,
                        inline_completed_embedding_artefacts,
                    ),
                    BottomProgressState::WaitingForQueue {
                        baseline_total,
                        completed_floor,
                        completed_jobs,
                        failed_jobs,
                    } => update_embeddings_queue_state(
                        current_embedding_queue_snapshot(&scope.repo_root, &scope.repo.repo_id)
                            .await?,
                        allow_empty_queue_completion,
                        baseline_total,
                        completed_floor,
                        completed_jobs,
                        failed_jobs,
                        inline_completed_embedding_artefacts,
                    ),
                    BottomProgressState::QueueComplete {
                        failed_jobs,
                        baseline_total,
                        ..
                    } => update_embeddings_queue_state(
                        current_embedding_queue_snapshot(&scope.repo_root, &scope.repo.repo_id)
                            .await?,
                        allow_empty_queue_completion,
                        baseline_total,
                        0,
                        baseline_total,
                        failed_jobs,
                        inline_completed_embedding_artefacts,
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
                        let queue_snapshot =
                            current_summary_queue_snapshot(&scope.repo_root, &scope.repo.repo_id)
                                .await?;
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
                    SummaryProgressState::WaitingForQueue {
                        result,
                        baseline_total,
                        completed_floor,
                        completed_jobs,
                        failed_jobs,
                    } => {
                        let queue_snapshot =
                            current_summary_queue_snapshot(&scope.repo_root, &scope.repo.repo_id)
                                .await?;
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
                    SummaryProgressState::Complete {
                        result,
                        failed_jobs,
                        baseline_total,
                    } if summary_queue_tracking_enabled(&result) => {
                        let queue_snapshot =
                            current_summary_queue_snapshot(&scope.repo_root, &scope.repo.repo_id)
                                .await?;
                        update_summary_queue_state(
                            result,
                            queue_snapshot,
                            allow_empty_queue_completion,
                            baseline_total,
                            0,
                            baseline_total,
                            failed_jobs,
                        )
                    }
                    other => other,
                };

                if enqueue_follow_up_sync_after_embeddings_ready
                    && embeddings_bootstrap_ready
                    && init_pipeline_complete(
                        checklist,
                        top_task.is_none(),
                        enqueue_ingest_after_sync,
                    )
                {
                    let (follow_up_sync_task, _merged) =
                        crate::cli::devql::graphql::enqueue_sync_task_via_graphql(
                            scope,
                            false,
                            None,
                            false,
                            false,
                            "init",
                            false,
                        )
                        .await?;
                    top_task = Some(follow_up_sync_task);
                    enqueue_follow_up_sync_after_embeddings_ready = false;
                }

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

fn update_embeddings_queue_state(
    snapshot: Option<EmbeddingQueueSnapshot>,
    allow_empty_completion: bool,
    baseline_total: u64,
    _completed_floor: u64,
    completed_jobs: u64,
    failed_jobs: u64,
    inline_completed_artefacts: u64,
) -> BottomProgressState {
    match snapshot {
        Some(snapshot) => {
            let baseline_total = baseline_total.max(snapshot.total).max(snapshot.completed);
            let completed_since_start = snapshot.completed.min(baseline_total);
            let failed_jobs = snapshot.failed;
            if snapshot.remaining() > 0 {
                BottomProgressState::Queue {
                    snapshot: EmbeddingQueueSnapshot {
                        completed: completed_since_start,
                        total: baseline_total,
                        ..snapshot
                    },
                    baseline_total,
                    completed_floor: 0,
                }
            } else if allow_empty_completion {
                completed_embeddings_state(failed_jobs, baseline_total, inline_completed_artefacts)
            } else {
                BottomProgressState::WaitingForQueue {
                    baseline_total,
                    completed_floor: 0,
                    completed_jobs: completed_since_start,
                    failed_jobs,
                }
            }
        }
        None if allow_empty_completion => {
            completed_embeddings_state(failed_jobs, baseline_total, inline_completed_artefacts)
        }
        None => BottomProgressState::WaitingForQueue {
            baseline_total,
            completed_floor: 0,
            completed_jobs,
            failed_jobs,
        },
    }
}

fn completed_embeddings_state(
    failed_jobs: u64,
    baseline_total: u64,
    inline_completed_artefacts: u64,
) -> BottomProgressState {
    if baseline_total == 0 && failed_jobs == 0 && inline_completed_artefacts > 0 {
        return BottomProgressState::QueueComplete {
            failed_jobs,
            baseline_total: inline_completed_artefacts,
            completion_source: EmbeddingCompletionSource::InlineSync,
        };
    }

    BottomProgressState::QueueComplete {
        failed_jobs,
        baseline_total,
        completion_source: if baseline_total > 0 || failed_jobs > 0 {
            EmbeddingCompletionSource::Queue
        } else {
            EmbeddingCompletionSource::NoneRequired
        },
    }
}

fn update_summary_queue_state(
    result: crate::cli::inference::SummarySetupExecutionResult,
    snapshot: Option<EmbeddingQueueSnapshot>,
    allow_empty_completion: bool,
    baseline_total: u64,
    _completed_floor: u64,
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
            let baseline_total = baseline_total.max(snapshot.total).max(snapshot.completed);
            let completed_since_start = snapshot.completed.min(baseline_total);
            let failed_jobs = snapshot.failed;
            if snapshot.remaining() > 0 {
                SummaryProgressState::Queue {
                    result,
                    snapshot: EmbeddingQueueSnapshot {
                        completed: completed_since_start,
                        total: baseline_total,
                        ..snapshot
                    },
                    baseline_total,
                    completed_floor: 0,
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
                    completed_floor: 0,
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
            completed_floor: 0,
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
        let state = update_embeddings_queue_state(None, false, 0, 0, 0, 0, 0);
        assert!(matches!(state, BottomProgressState::WaitingForQueue { .. }));

        let state = update_embeddings_queue_state(None, true, 0, 0, 0, 0, 0);
        assert!(matches!(
            state,
            BottomProgressState::QueueComplete {
                completion_source: EmbeddingCompletionSource::NoneRequired,
                ..
            }
        ));
    }

    #[test]
    fn embeddings_can_report_inline_sync_work_when_queue_stays_empty() {
        let state = update_embeddings_queue_state(None, true, 0, 0, 0, 0, 12);
        assert!(matches!(
            state,
            BottomProgressState::QueueComplete {
                baseline_total: 12,
                completion_source: EmbeddingCompletionSource::InlineSync,
                ..
            }
        ));
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
    fn embeddings_queue_progress_uses_snapshot_artefact_totals() {
        let state = update_embeddings_queue_state(
            Some(EmbeddingQueueSnapshot {
                pending: 1,
                running: 1,
                failed: 0,
                completed: 42,
                total: 100,
            }),
            false,
            0,
            0,
            0,
            0,
            0,
        );

        match state {
            BottomProgressState::Queue {
                snapshot,
                baseline_total,
                ..
            } => {
                assert_eq!(snapshot.completed, 42);
                assert_eq!(baseline_total, 100);
            }
            _ => panic!("expected queue state"),
        }
    }

    #[test]
    fn summary_queue_progress_uses_snapshot_summary_totals() {
        let state = update_summary_queue_state(
            configured_summary_result(),
            Some(EmbeddingQueueSnapshot {
                pending: 3,
                running: 1,
                failed: 0,
                completed: 75,
                total: 120,
            }),
            false,
            0,
            0,
            0,
            0,
        );

        match state {
            SummaryProgressState::Queue {
                snapshot,
                baseline_total,
                ..
            } => {
                assert_eq!(snapshot.completed, 75);
                assert_eq!(baseline_total, 120);
            }
            _ => panic!("expected queue state"),
        }
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
}
