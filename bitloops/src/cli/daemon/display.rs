use crate::daemon;

pub(super) fn status_lines(report: &daemon::DaemonStatusReport) -> Vec<String> {
    let mut lines = Vec::new();
    let log_path = daemon::daemon_log_file_path();

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
        if let Some(sync) = report.sync.as_ref() {
            append_sync_lines(&mut lines, sync);
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
        if let Some(sync) = report.sync.as_ref() {
            append_sync_lines(&mut lines, sync);
        }
        return lines;
    }

    lines.push("Bitloops daemon: stopped".to_string());
    lines.push("Mode: not running".to_string());
    lines.push(format!("Log file: {}", log_path.display()));
    if let Some(enrichment) = report.enrichment.as_ref() {
        append_enrichment_lines(&mut lines, enrichment);
    }
    if let Some(sync) = report.sync.as_ref() {
        append_sync_lines(&mut lines, sync);
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

fn append_sync_lines(lines: &mut Vec<String>, status: &daemon::SyncQueueStatus) {
    lines.push(format!("Sync pending tasks: {}", status.state.pending_tasks));
    lines.push(format!("Sync running tasks: {}", status.state.running_tasks));
    lines.push(format!("Sync failed tasks: {}", status.state.failed_tasks));
    lines.push(format!(
        "Sync completed recent tasks: {}",
        status.state.completed_recent_tasks
    ));
    if let Some(action) = status.state.last_action.as_ref() {
        lines.push(format!("Sync last action: {action}"));
    }
    if let Some(task) = status.current_repo_task.as_ref() {
        lines.push(format!(
            "Current repo sync task: {} ({}, mode={}, source={})",
            task.task_id, task.status, task.mode, task.source
        ));
        lines.push(format!(
            "Current repo sync phase: {}",
            task.progress.phase.as_str()
        ));
        if task.progress.paths_total > 0 {
            lines.push(format!(
                "Current repo sync progress: {}/{} paths complete ({} remaining)",
                task.progress.paths_completed,
                task.progress.paths_total,
                task.progress.paths_remaining
            ));
        }
        if let Some(position) = task.queue_position {
            lines.push(format!(
                "Current repo sync queue position: {} ({} ahead)",
                position,
                task.tasks_ahead.unwrap_or(position.saturating_sub(1))
            ));
        }
        if let Some(path) = task.progress.current_path.as_ref() {
            lines.push(format!("Current repo sync path: {path}"));
        }
        if let Some(error) = task.error.as_ref() {
            lines.push(format!("Current repo sync error: {error}"));
        }
    }
    lines.push(format!(
        "Sync persisted: {}",
        if status.persisted { "yes" } else { "no" }
    ));
}

pub(super) fn print_legacy_repo_data_warnings() {
    for line in legacy_repo_data_warnings() {
        eprintln!("{line}");
    }
}

pub(super) fn legacy_repo_data_warnings() -> Vec<String> {
    let Some(repo_root) = crate::utils::paths::repo_root().ok() else {
        return Vec::new();
    };

    let legacy_paths = [
        repo_root.join(".bitloops").join("stores"),
        repo_root.join(".bitloops").join("embeddings"),
        repo_root.join(".bitloops").join("tmp"),
        repo_root.join(".bitloops").join("metadata"),
    ];
    let found: Vec<_> = legacy_paths
        .into_iter()
        .filter(|path| path.exists())
        .collect();
    if found.is_empty() {
        return Vec::new();
    }

    let mut lines = Vec::with_capacity(found.len() + 1);
    lines.push(
        "Warning: legacy repo-local Bitloops data was found and is ignored unless you configure those paths explicitly in the daemon config.".to_string(),
    );
    lines.extend(
        found
            .into_iter()
            .map(|path| format!("Legacy path: {}", path.display())),
    );
    lines
}
