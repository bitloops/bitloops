use std::path::Path;

use crate::daemon;

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
        if let Some(capability_events) = report.capability_events.as_ref() {
            append_capability_event_lines(&mut lines, capability_events);
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
        if let Some(capability_events) = report.capability_events.as_ref() {
            append_capability_event_lines(&mut lines, capability_events);
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
    if let Some(capability_events) = report.capability_events.as_ref() {
        append_capability_event_lines(&mut lines, capability_events);
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
        "Enrichment pending semantic jobs: {}",
        status.state.pending_semantic_jobs
    ));
    lines.push(format!(
        "Enrichment pending embedding jobs: {}",
        status.state.pending_embedding_jobs
    ));
    lines.push(format!(
        "Enrichment pending clone-edge rebuild jobs: {}",
        status.state.pending_clone_edges_rebuild_jobs
    ));
    lines.push(format!(
        "Enrichment running jobs: {}",
        status.state.running_jobs
    ));
    lines.push(format!(
        "Enrichment running semantic jobs: {}",
        status.state.running_semantic_jobs
    ));
    lines.push(format!(
        "Enrichment running embedding jobs: {}",
        status.state.running_embedding_jobs
    ));
    lines.push(format!(
        "Enrichment running clone-edge rebuild jobs: {}",
        status.state.running_clone_edges_rebuild_jobs
    ));
    lines.push(format!(
        "Enrichment failed jobs: {}",
        status.state.failed_jobs
    ));
    lines.push(format!(
        "Enrichment failed semantic jobs: {}",
        status.state.failed_semantic_jobs
    ));
    lines.push(format!(
        "Enrichment failed embedding jobs: {}",
        status.state.failed_embedding_jobs
    ));
    lines.push(format!(
        "Enrichment failed clone-edge rebuild jobs: {}",
        status.state.failed_clone_edges_rebuild_jobs
    ));
    lines.push(format!(
        "Enrichment retried failed jobs: {}",
        status.state.retried_failed_jobs
    ));
    if let Some(action) = status.state.last_action.as_ref() {
        lines.push(format!("Enrichment last action: {action}"));
    }
    if let Some(reason) = status.state.paused_reason.as_ref() {
        lines.push(format!("Enrichment pause reason: {reason}"));
    }
    lines.push(format!(
        "Enrichment persisted: {}",
        if status.persisted { "yes" } else { "no" }
    ));
}

fn append_capability_event_lines(
    lines: &mut Vec<String>,
    status: &daemon::CapabilityEventQueueStatus,
) {
    lines.push("Capability event queue: available".to_string());
    lines.push(format!(
        "Capability event pending runs: {}",
        status.state.pending_runs
    ));
    lines.push(format!(
        "Capability event running runs: {}",
        status.state.running_runs
    ));
    lines.push(format!(
        "Capability event failed runs: {}",
        status.state.failed_runs
    ));
    lines.push(format!(
        "Capability event completed recent runs: {}",
        status.state.completed_recent_runs
    ));
    if let Some(action) = status.state.last_action.as_ref() {
        lines.push(format!("Capability event last action: {action}"));
    }
    if let Some(run) = status.current_repo_run.as_ref() {
        lines.push(format!(
            "Current repo capability event run: {} ({}, capability={}, handler={}, event_kind={})",
            run.run_id, run.status, run.capability_id, run.handler_id, run.event_kind
        ));
        if let Some(error) = run.error.as_ref() {
            lines.push(format!("Current repo capability event error: {error}"));
        }
    }
    lines.push(format!(
        "Capability event persisted: {}",
        if status.persisted { "yes" } else { "no" }
    ));
}

fn append_devql_task_lines(lines: &mut Vec<String>, status: &daemon::DevqlTaskQueueStatus) {
    lines.push("DevQL task queue: available".to_string());
    lines.push(format!(
        "DevQL queued tasks: {}",
        status.state.queued_tasks
    ));
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
            counts.kind,
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
            lines.push(format!(
                "Current repo ingest phase: {:?}",
                progress.phase
            ));
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
