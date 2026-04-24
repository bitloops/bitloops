use std::path::Path;

use crate::daemon;
use crate::runtime_presentation::{
    INIT_CODE_EMBEDDINGS_LANE_LABEL, RETRY_FAILED_ENRICHMENTS_COMMAND, mailbox_label,
    queue_state_summary, session_status_label, waiting_reason_label, workplane_pool_label,
};

fn current_state_consumer_status(
    report: &daemon::DaemonStatusReport,
) -> Option<&daemon::CapabilityEventQueueStatus> {
    report
        .current_state_consumers
        .as_ref()
        .or(report.capability_events.as_ref())
}

pub(super) fn status_lines(report: &daemon::DaemonStatusReport) -> Vec<String> {
    let log_path = daemon::daemon_log_file_path();
    status_lines_with_log_path(report, &log_path)
}

pub(super) fn status_lines_with_log_path(
    report: &daemon::DaemonStatusReport,
    log_path: &Path,
) -> Vec<String> {
    let mut lines = Vec::new();

    if let Some(runtime) = report.runtime.as_ref() {
        lines.push("Bitloops daemon: running".to_string());
        lines.push(format!("Mode: {}", runtime.mode));
        lines.push(format!("URL: {}", runtime.url));
        lines.push(format!("Config: {}", runtime.config_path.display()));
        lines.push(format!("Log file: {}", log_path.display()));
        lines.push(format!("PID: {}", runtime.pid));
        append_supervisor_lines(&mut lines, report);
        if let Some(health) = report.health.as_ref() {
            append_health_lines(&mut lines, health);
        }
        if let Some(enrichment) = report.enrichment.as_ref() {
            append_enrichment_lines(&mut lines, enrichment);
        }
        if let Some(capability_events) = current_state_consumer_status(report) {
            append_capability_event_lines(&mut lines, capability_events);
        }
        if let Some(session) = report.current_init_session.as_ref() {
            append_current_init_session_lines(&mut lines, session);
        }
        if let Some(devql_tasks) = report.devql_tasks.as_ref() {
            append_devql_task_lines(&mut lines, devql_tasks);
        }
        return lines;
    }

    if let Some(service) = report.service.as_ref() {
        lines.push("Bitloops daemon: stopped".to_string());
        lines.push("Mode: always-on service".to_string());
        lines.push(format!("Config: {}", service.config_path.display()));
        lines.push(format!("Log file: {}", log_path.display()));
        lines.push(format!(
            "Supervisor service: {} ({}, installed)",
            service.service_name, service.manager
        ));
        lines.push(format!(
            "Supervisor state: {}",
            if report.service_running {
                "running"
            } else {
                "stopped"
            }
        ));
        if let Some(url) = service.last_url.as_ref() {
            lines.push(format!("Last URL: {url}"));
        }
        if let Some(enrichment) = report.enrichment.as_ref() {
            append_enrichment_lines(&mut lines, enrichment);
        }
        if let Some(capability_events) = current_state_consumer_status(report) {
            append_capability_event_lines(&mut lines, capability_events);
        }
        if let Some(session) = report.current_init_session.as_ref() {
            append_current_init_session_lines(&mut lines, session);
        }
        if let Some(devql_tasks) = report.devql_tasks.as_ref() {
            append_devql_task_lines(&mut lines, devql_tasks);
        }
        return lines;
    }

    lines.push("Bitloops daemon: stopped".to_string());
    lines.push("Mode: not running".to_string());
    lines.push(format!("Log file: {}", log_path.display()));
    if let Some(enrichment) = report.enrichment.as_ref() {
        append_enrichment_lines(&mut lines, enrichment);
    }
    if let Some(capability_events) = current_state_consumer_status(report) {
        append_capability_event_lines(&mut lines, capability_events);
    }
    if let Some(session) = report.current_init_session.as_ref() {
        append_current_init_session_lines(&mut lines, session);
    }
    if let Some(devql_tasks) = report.devql_tasks.as_ref() {
        append_devql_task_lines(&mut lines, devql_tasks);
    }
    lines
}

fn append_supervisor_lines(lines: &mut Vec<String>, report: &daemon::DaemonStatusReport) {
    if let Some(service) = report.service.as_ref() {
        lines.push(format!(
            "Supervisor service: {} ({}, installed)",
            service.service_name, service.manager
        ));
        lines.push(format!(
            "Supervisor state: {}",
            if report.service_running {
                "running"
            } else {
                "stopped"
            }
        ));
    }
}

fn append_health_lines(lines: &mut Vec<String>, health: &daemon::DaemonHealthSummary) {
    if let (Some(backend), Some(connected)) =
        (&health.relational_backend, health.relational_connected)
    {
        lines.push(format!(
            "Relational: {} ({})",
            backend,
            if connected {
                "connected"
            } else {
                "disconnected"
            }
        ));
    }
    if let (Some(backend), Some(connected)) = (&health.events_backend, health.events_connected) {
        lines.push(format!(
            "Events: {} ({})",
            backend,
            if connected {
                "connected"
            } else {
                "disconnected"
            }
        ));
    }
    if let (Some(backend), Some(connected)) = (&health.blob_backend, health.blob_connected) {
        lines.push(format!(
            "Blob: {} ({})",
            backend,
            if connected {
                "available"
            } else {
                "unavailable"
            }
        ));
    }
}

pub(super) fn enrichment_status_lines(status: &daemon::EnrichmentQueueStatus) -> Vec<String> {
    let mut lines = vec!["Enrichment queue: available".to_string()];
    append_enrichment_lines(&mut lines, status);
    lines
}

fn append_enrichment_lines(lines: &mut Vec<String>, status: &daemon::EnrichmentQueueStatus) {
    lines.push(format!("Enrichment mode: {}", status.state.mode));
    lines.push(format!(
        "Enrichment pending jobs: {}",
        status.state.pending_jobs
    ));
    lines.push(format!(
        "Enrichment pending work items: {}",
        status.state.pending_work_items
    ));
    lines.push(format!(
        "Enrichment pending semantic jobs: {}",
        status.state.pending_semantic_jobs
    ));
    lines.push(format!(
        "Enrichment pending semantic work items: {}",
        status.state.pending_semantic_work_items
    ));
    lines.push(format!(
        "Enrichment pending embedding jobs: {}",
        status.state.pending_embedding_jobs
    ));
    lines.push(format!(
        "Enrichment pending embedding work items: {}",
        status.state.pending_embedding_work_items
    ));
    lines.push(format!(
        "Enrichment pending clone-edge rebuild jobs: {}",
        status.state.pending_clone_edges_rebuild_jobs
    ));
    lines.push(format!(
        "Enrichment pending clone-edge rebuild work items: {}",
        status.state.pending_clone_edges_rebuild_work_items
    ));
    lines.push(format!(
        "Enrichment completed recent jobs: {}",
        status.state.completed_recent_jobs
    ));
    lines.push(format!(
        "Enrichment running jobs: {}",
        status.state.running_jobs
    ));
    lines.push(format!(
        "Enrichment running work items: {}",
        status.state.running_work_items
    ));
    lines.push(format!(
        "Enrichment running semantic jobs: {}",
        status.state.running_semantic_jobs
    ));
    lines.push(format!(
        "Enrichment running semantic work items: {}",
        status.state.running_semantic_work_items
    ));
    lines.push(format!(
        "Enrichment running embedding jobs: {}",
        status.state.running_embedding_jobs
    ));
    lines.push(format!(
        "Enrichment running embedding work items: {}",
        status.state.running_embedding_work_items
    ));
    lines.push(format!(
        "Enrichment running clone-edge rebuild jobs: {}",
        status.state.running_clone_edges_rebuild_jobs
    ));
    lines.push(format!(
        "Enrichment running clone-edge rebuild work items: {}",
        status.state.running_clone_edges_rebuild_work_items
    ));
    lines.push(format!(
        "Enrichment failed jobs: {}",
        status.state.failed_jobs
    ));
    lines.push(format!(
        "Enrichment failed work items: {}",
        status.state.failed_work_items
    ));
    lines.push(format!(
        "Enrichment failed semantic jobs: {}",
        status.state.failed_semantic_jobs
    ));
    lines.push(format!(
        "Enrichment failed semantic work items: {}",
        status.state.failed_semantic_work_items
    ));
    lines.push(format!(
        "Enrichment failed embedding jobs: {}",
        status.state.failed_embedding_jobs
    ));
    lines.push(format!(
        "Enrichment failed embedding work items: {}",
        status.state.failed_embedding_work_items
    ));
    lines.push(format!(
        "Enrichment failed clone-edge rebuild jobs: {}",
        status.state.failed_clone_edges_rebuild_jobs
    ));
    lines.push(format!(
        "Enrichment failed clone-edge rebuild work items: {}",
        status.state.failed_clone_edges_rebuild_work_items
    ));
    lines.push(format!(
        "Enrichment retried failed jobs: {}",
        status.state.retried_failed_jobs
    ));
    for pool in &status.state.worker_pools {
        lines.push(format!(
            "Enrichment pool {}: budget={} active={} queued={} running={} failed={} completed_recent={}",
            workplane_pool_label(&pool.kind.to_string()),
            pool.worker_budget,
            pool.active_workers,
            pool.pending_jobs,
            pool.running_jobs,
            pool.failed_jobs,
            pool.completed_recent_jobs
        ));
    }
    if let Some(action) = status.state.last_action.as_ref() {
        lines.push(format!("Enrichment last action: {action}"));
    }
    if let Some(reason) = status.state.paused_reason.as_ref() {
        lines.push(format!("Enrichment pause reason: {reason}"));
    }
    if let Some(gate) = status.embeddings_gate.as_ref() {
        lines.push(format!(
            "Embeddings gate blocked: {}",
            if gate.blocked { "yes" } else { "no" }
        ));
        if let Some(readiness) = gate.readiness {
            lines.push(format!("Embeddings gate readiness: {readiness}"));
        }
        if let Some(reason) = gate.reason.as_ref() {
            lines.push(format!("Embeddings gate reason: {reason}"));
        }
        if let Some(task_id) = gate.active_task_id.as_ref() {
            lines.push(format!("Embeddings gate active task: {task_id}"));
        }
        if let Some(profile_name) = gate.profile_name.as_ref() {
            lines.push(format!("Embeddings gate profile: {profile_name}"));
        }
        if let Some(config_path) = gate.config_path.as_ref() {
            lines.push(format!("Embeddings gate config: {}", config_path.display()));
        }
        if let Some(last_error) = gate.last_error.as_ref() {
            lines.push(format!("Embeddings gate last error: {last_error}"));
        }
    }
    for mailbox in &status.blocked_mailboxes {
        lines.push(format!(
            "Mailbox blocked: {} ({})",
            mailbox_label(&mailbox.mailbox_name),
            mailbox.reason
        ));
    }
    if let Some(failed) = status.last_failed_embedding.as_ref() {
        lines.push(format!(
            "Last failed embedding job: {} (repo={}, branch={}, kind={}, artefacts={}, attempts={})",
            failed.job_id,
            failed.repo_id,
            failed.branch,
            failed.representation_kind,
            failed.artefact_count,
            failed.attempts,
        ));
        if let Some(error) = failed.error.as_ref() {
            lines.push(format!("Last failed embedding error: {error}"));
        }
    }
    lines.push(format!(
        "Enrichment persisted: {}",
        if status.persisted { "yes" } else { "no" }
    ));
    if status.state.failed_jobs > 0 {
        lines.push(format!(
            "Retry failed enrichment work: {RETRY_FAILED_ENRICHMENTS_COMMAND}"
        ));
    }
}

fn append_capability_event_lines(
    lines: &mut Vec<String>,
    status: &daemon::CapabilityEventQueueStatus,
) {
    lines.push("Current-state consumer queue: available".to_string());
    lines.push(format!(
        "Current-state consumer pending runs: {}",
        status.state.pending_runs
    ));
    lines.push(format!(
        "Current-state consumer running runs: {}",
        status.state.running_runs
    ));
    lines.push(format!(
        "Current-state consumer failed runs: {}",
        status.state.failed_runs
    ));
    lines.push(format!(
        "Current-state consumer completed recent runs: {}",
        status.state.completed_recent_runs
    ));
    if let Some(action) = status.state.last_action.as_ref() {
        lines.push(format!("Current-state consumer last action: {action}"));
    }
    if let Some(run) = status.current_repo_run.as_ref() {
        lines.push(format!(
            "Current repo current-state consumer run: {} ({}, capability={}, consumer={}, mode={}, generations={}..={})",
            run.run_id,
            run.status,
            run.capability_id,
            run.consumer_id,
            run.reconcile_mode,
            run.from_generation_seq.saturating_add(1),
            run.to_generation_seq
        ));
        if let Some(error) = run.error.as_ref() {
            lines.push(format!(
                "Current repo current-state consumer error: {error}"
            ));
        }
    }
    lines.push(format!(
        "Current-state consumer persisted: {}",
        if status.persisted { "yes" } else { "no" }
    ));
}

fn append_current_init_session_lines(
    lines: &mut Vec<String>,
    session: &daemon::InitRuntimeSessionView,
) {
    lines.push(format!(
        "Current init session: {} ({})",
        session.init_session_id,
        session_status_label(&session.status)
    ));
    if let Some(reason) = session.waiting_reason.as_ref() {
        lines.push(format!(
            "Current init waiting: {}",
            waiting_reason_label(reason)
        ));
    }
    if let Some(summary) = session.warning_summary.as_ref() {
        lines.push(format!("Current init warning: {summary}"));
    }
    if session.embeddings_selected {
        lines.push(format!(
            "Current repo code embeddings: {}",
            format_init_runtime_lane_summary(
                INIT_CODE_EMBEDDINGS_LANE_LABEL,
                &session.code_embeddings_lane,
            )
        ));
    }
    if let Some(error) = session.terminal_error.as_ref() {
        lines.push(format!("Current init error: {error}"));
    }
}

fn format_init_runtime_lane_summary(
    lane_label: &str,
    lane: &daemon::InitRuntimeLaneView,
) -> String {
    let mut summary = lane
        .activity_label
        .clone()
        .unwrap_or_else(|| lane_label.to_string());
    if let Some(reason) = lane.waiting_reason.as_ref() {
        summary.push_str(&format!(" ({})", waiting_reason_label(reason)));
    }
    if let Some(progress) = lane.progress.as_ref()
        && progress.total > 0
    {
        summary.push_str(&format!(
            " · {} of {} ready · {} left",
            progress.completed, progress.total, progress.remaining
        ));
    }
    summary.push_str(&format!(
        " · {}",
        queue_state_summary(lane.queue.queued, lane.queue.running, lane.queue.failed)
    ));
    if let Some(warning) = lane.warnings.first() {
        summary.push_str(&format!(" · Warning: {}", warning.message));
    }
    summary
}

fn append_devql_task_lines(lines: &mut Vec<String>, status: &daemon::DevqlTaskQueueStatus) {
    lines.push("DevQL task queue: available".to_string());
    lines.push(format!("DevQL queued tasks: {}", status.state.queued_tasks));
    lines.push(format!(
        "DevQL running tasks: {}",
        status.state.running_tasks
    ));
    lines.push(format!("DevQL failed tasks: {}", status.state.failed_tasks));
    lines.push(format!(
        "DevQL completed recent tasks: {}",
        status.state.completed_recent_tasks
    ));
    for counts in &status.state.by_kind {
        lines.push(format!(
            "DevQL {} tasks: queued={}, running={}, failed={}, completed_recent={}",
            devql_task_kind_label(counts.kind),
            counts.queued_tasks,
            counts.running_tasks,
            counts.failed_tasks,
            counts.completed_recent_tasks
        ));
    }
    if let Some(action) = status.state.last_action.as_ref() {
        lines.push(format!("DevQL last action: {action}"));
    }
    if let Some(control) = status.current_repo_control.as_ref() {
        lines.push(format!(
            "Current repo task queue: {}",
            if control.paused { "paused" } else { "running" }
        ));
        if let Some(reason) = control.paused_reason.as_ref() {
            lines.push(format!("Current repo task pause reason: {reason}"));
        }
    }

    fn devql_task_kind_label(kind: daemon::DevqlTaskKind) -> &'static str {
        match kind {
            daemon::DevqlTaskKind::Sync => "sync",
            daemon::DevqlTaskKind::Ingest => "ingest",
            daemon::DevqlTaskKind::EmbeddingsBootstrap => "embeddings runtime bootstrap",
            daemon::DevqlTaskKind::SummaryBootstrap => "summary bootstrap",
        }
    }
    for task in &status.current_repo_tasks {
        lines.push(format!(
            "Current repo task: {} ({}, kind={}, source={})",
            task.task_id, task.status, task.kind, task.source
        ));
        if let Some(progress) = task.sync_progress() {
            lines.push(format!(
                "Current repo sync phase: {}",
                progress.phase.as_str()
            ));
            if progress.paths_total > 0 {
                lines.push(format!(
                    "Current repo sync progress: {}/{} paths complete ({} remaining)",
                    progress.paths_completed, progress.paths_total, progress.paths_remaining
                ));
            }
            if let Some(path) = progress.current_path.as_ref() {
                lines.push(format!("Current repo sync path: {path}"));
            }
        }
        if let Some(progress) = task.ingest_progress() {
            lines.push(format!("Current repo ingest phase: {:?}", progress.phase));
            if let Some(commit_sha) = progress.current_commit_sha.as_ref() {
                lines.push(format!("Current repo ingest commit: {commit_sha}"));
            }
        }
        if let Some(position) = task.queue_position {
            lines.push(format!(
                "Current repo task queue position: {} ({} ahead)",
                position,
                task.tasks_ahead.unwrap_or(position.saturating_sub(1))
            ));
        }
        if let Some(error) = task.error.as_ref() {
            lines.push(format!("Current repo task error: {error}"));
        }
    }
    lines.push(format!(
        "DevQL persisted: {}",
        if status.persisted { "yes" } else { "no" }
    ));
}
