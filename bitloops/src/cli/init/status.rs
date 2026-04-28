use std::borrow::Cow;
use std::io::Write;
use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result};
use serde_json::json;

use crate::cli::devql::graphql::{
    RuntimeInitLaneGraphqlRecord, RuntimeInitLaneProgressGraphqlRecord,
    RuntimeInitSessionGraphqlRecord, RuntimeSnapshotGraphqlRecord, runtime_snapshot_via_graphql,
};
use crate::devql_transport::discover_slim_cli_repo_scope;
use crate::runtime_presentation::{
    INIT_CODE_EMBEDDINGS_LANE_LABEL, INIT_CODE_EMBEDDINGS_SECTION_TITLE, INIT_INGEST_LANE_LABEL,
    INIT_INGEST_SECTION_TITLE, INIT_SUMMARIES_LANE_LABEL, INIT_SUMMARIES_SECTION_TITLE,
    INIT_SUMMARY_EMBEDDINGS_LANE_LABEL, INIT_SUMMARY_EMBEDDINGS_SECTION_TITLE,
    INIT_SYNC_LANE_LABEL, INIT_SYNC_SECTION_TITLE, queue_state_summary, session_status_label,
    waiting_reason_label,
};

use super::InitStatusArgs;

const STATUS_POLL_INTERVAL: Duration = Duration::from_secs(1);

struct SelectedLane<'a> {
    title: &'static str,
    label: &'static str,
    lane: &'a RuntimeInitLaneGraphqlRecord,
}

pub(super) async fn run_for_project_root(
    args: InitStatusArgs,
    project_root: &Path,
    out: &mut dyn Write,
) -> Result<()> {
    let scope = discover_slim_cli_repo_scope(Some(project_root))
        .context("discovering repository scope for `bitloops init status`")?;
    let repo_id = scope.repo.repo_id.clone();

    if args.watch {
        let snapshot = load_snapshot(&scope, &repo_id).await?;
        if selected_session(&snapshot, args.session_id.as_deref()).is_none() {
            write_rendered_output(
                out,
                args.json,
                &render_status_output(args.json, &repo_id, args.session_id.as_deref(), &snapshot),
            )?;
            return Ok(());
        }

        let mut last_output =
            render_status_output(args.json, &repo_id, args.session_id.as_deref(), &snapshot);
        write_rendered_output(out, args.json, &last_output)?;
        if selected_session(&snapshot, args.session_id.as_deref())
            .is_some_and(|session| is_terminal_status(session.status.as_str()))
        {
            return Ok(());
        }

        loop {
            tokio::time::sleep(STATUS_POLL_INTERVAL).await;
            let snapshot = load_snapshot(&scope, &repo_id).await?;
            let rendered =
                render_status_output(args.json, &repo_id, args.session_id.as_deref(), &snapshot);
            if last_output != rendered {
                write_rendered_output(out, args.json, &rendered)?;
                last_output = rendered;
            }

            if selected_session(&snapshot, args.session_id.as_deref())
                .is_some_and(|session| is_terminal_status(session.status.as_str()))
            {
                return Ok(());
            }
        }
    }

    if args.wait {
        let mut wait_notice_printed = false;
        loop {
            let snapshot = load_snapshot(&scope, &repo_id).await?;
            if let Some(session) = selected_session(&snapshot, args.session_id.as_deref())
                && is_terminal_status(session.status.as_str())
            {
                write_rendered_output(
                    out,
                    args.json,
                    &render_status_output(
                        args.json,
                        &repo_id,
                        args.session_id.as_deref(),
                        &snapshot,
                    ),
                )?;
                return Ok(());
            }

            if !args.json && !wait_notice_printed {
                writeln!(
                    out,
                    "{}",
                    wait_message(&repo_id, args.session_id.as_deref(), &snapshot)
                )?;
                out.flush()?;
                wait_notice_printed = true;
            }

            tokio::time::sleep(STATUS_POLL_INTERVAL).await;
        }
    }

    let snapshot = load_snapshot(&scope, &repo_id).await?;
    write_rendered_output(
        out,
        args.json,
        &render_status_output(args.json, &repo_id, args.session_id.as_deref(), &snapshot),
    )
}

async fn load_snapshot(
    scope: &crate::devql_transport::SlimCliRepoScope,
    repo_id: &str,
) -> Result<RuntimeSnapshotGraphqlRecord> {
    runtime_snapshot_via_graphql(scope, repo_id)
        .await
        .with_context(|| format!("loading runtime snapshot for repo `{repo_id}`"))
}

fn write_rendered_output(out: &mut dyn Write, json: bool, rendered: &str) -> Result<()> {
    if json {
        writeln!(out, "{rendered}")?;
    } else {
        write!(out, "{rendered}")?;
        if !rendered.ends_with('\n') {
            writeln!(out)?;
        }
    }
    out.flush().context("writing init status output")
}

fn render_status_output(
    json: bool,
    repo_id: &str,
    requested_session_id: Option<&str>,
    snapshot: &RuntimeSnapshotGraphqlRecord,
) -> String {
    let selected_session = selected_session(snapshot, requested_session_id);
    if json {
        return render_status_json(repo_id, requested_session_id, snapshot, selected_session);
    }

    render_status_text(repo_id, requested_session_id, snapshot, selected_session)
}

fn render_status_text(
    repo_id: &str,
    requested_session_id: Option<&str>,
    snapshot: &RuntimeSnapshotGraphqlRecord,
    selected_session: Option<&RuntimeInitSessionGraphqlRecord>,
) -> String {
    let Some(session) = selected_session else {
        return no_matching_session_text(
            repo_id,
            requested_session_id,
            snapshot.current_init_session.as_ref(),
        );
    };

    let mut lines = vec![
        format!("Repository: {repo_id}"),
        format!("Init session: {}", session.init_session_id),
        format!(
            "Status: {}",
            runtime_status_label(effective_session_status(session).as_ref())
        ),
        format!("Summary: {}", session_summary_text(snapshot, session)),
    ];

    if let Some(error) = session.terminal_error.as_ref() {
        lines.push(format!("Error: {error}"));
    }

    let selected_lanes = selected_lanes(session);
    if !selected_lanes.is_empty() {
        lines.push(String::new());
        lines.push("Selected lanes:".to_string());
        for selected in selected_lanes {
            lines.push(format!(
                "  {}: {}",
                selected.title,
                lane_summary_text(selected.title, selected.lane),
            ));
        }
    }

    lines.join("\n") + "\n"
}

fn no_matching_session_text(
    repo_id: &str,
    requested_session_id: Option<&str>,
    current_session: Option<&RuntimeInitSessionGraphqlRecord>,
) -> String {
    let detail = match (requested_session_id, current_session) {
        (Some(requested), Some(current)) => format!(
            "No active init session matching {requested} for this repository. Current active session: {}.",
            current.init_session_id
        ),
        (Some(requested), None) => {
            format!("No active init session matching {requested} for this repository.")
        }
        (None, _) => "No active init session for this repository.".to_string(),
    };

    format!("Repository: {repo_id}\n{detail}\n")
}

fn render_status_json(
    repo_id: &str,
    requested_session_id: Option<&str>,
    snapshot: &RuntimeSnapshotGraphqlRecord,
    selected_session: Option<&RuntimeInitSessionGraphqlRecord>,
) -> String {
    let payload = json!({
        "repoId": repo_id,
        "requestedSessionId": requested_session_id,
        "currentInitSessionId": snapshot
            .current_init_session
            .as_ref()
            .map(|session| session.init_session_id.clone()),
        "session": selected_session.map(|session| {
            let status = effective_session_status(session);
            json!({
                "initSessionId": session.init_session_id.as_str(),
                "status": status.as_ref(),
                "statusLabel": runtime_status_label(status.as_ref()),
                "waitingReason": session.waiting_reason.clone(),
                "waitingLabel": session
                    .waiting_reason
                    .as_deref()
                    .map(waiting_reason_label),
                "warningSummary": session.warning_summary.clone(),
                "followUpSyncRequired": session.follow_up_sync_required,
                "summaryText": session_summary_text(snapshot, session),
                "terminalError": session.terminal_error.clone(),
                "lanes": selected_lanes(session)
                    .into_iter()
                    .map(|selected| {
                        json!({
                            "title": selected.title,
                            "label": selected.label,
                            "status": selected.lane.status.as_str(),
                            "statusLabel": runtime_status_label(selected.lane.status.as_str()),
                            "waitingReason": selected.lane.waiting_reason.clone(),
                            "waitingLabel": selected
                                .lane
                                .waiting_reason
                                .as_deref()
                                .map(waiting_reason_label),
                            "activityLabel": selected.lane.activity_label.clone(),
                            "detail": selected.lane.detail.clone(),
                            "summaryText": lane_summary_text(selected.title, selected.lane),
                            "progress": selected.lane.progress.as_ref().map(|progress| {
                                json!({
                                    "completed": progress.completed,
                                    "inMemoryCompleted": progress.in_memory_completed,
                                    "total": progress.total,
                                    "remaining": progress.remaining,
                                })
                            }),
                            "queue": {
                                "queued": selected.lane.queue.queued,
                                "running": selected.lane.queue.running,
                                "failed": selected.lane.queue.failed,
                            },
                            "warnings": selected
                                .lane
                                .warnings
                                .iter()
                                .map(|warning| {
                                    json!({
                                        "componentLabel": warning.component_label.as_str(),
                                        "message": warning.message.as_str(),
                                        "retryCommand": warning.retry_command.as_str(),
                                    })
                                })
                                .collect::<Vec<_>>(),
                        })
                    })
                    .collect::<Vec<_>>(),
            })
        }),
    });

    serde_json::to_string(&payload).expect("serialising init status payload")
}

fn wait_message(
    repo_id: &str,
    requested_session_id: Option<&str>,
    snapshot: &RuntimeSnapshotGraphqlRecord,
) -> String {
    match (requested_session_id, snapshot.current_init_session.as_ref()) {
        (Some(requested), Some(current))
            if !session_id_matches(Some(requested), current.init_session_id.as_str()) =>
        {
            format!(
                "Waiting for init session {requested} in repository {repo_id}. Current active session: {}.",
                current.init_session_id
            )
        }
        (Some(requested), _) => {
            format!("Waiting for init session {requested} in repository {repo_id}.")
        }
        (None, Some(current)) => format!(
            "Waiting for init session {} in repository {repo_id} to finish.",
            current.init_session_id
        ),
        (None, None) => {
            format!("Waiting for an init session in repository {repo_id} to become active.")
        }
    }
}

fn selected_session<'a>(
    snapshot: &'a RuntimeSnapshotGraphqlRecord,
    requested_session_id: Option<&str>,
) -> Option<&'a RuntimeInitSessionGraphqlRecord> {
    snapshot.current_init_session.as_ref().filter(|session| {
        session_id_matches(requested_session_id, session.init_session_id.as_str())
    })
}

fn session_id_matches(requested_session_id: Option<&str>, current_session_id: &str) -> bool {
    requested_session_id.is_none_or(|requested| {
        current_session_id == requested || current_session_id.starts_with(requested)
    })
}

fn selected_lanes(session: &RuntimeInitSessionGraphqlRecord) -> Vec<SelectedLane<'_>> {
    let mut selected = Vec::new();

    if session.run_sync {
        selected.push(SelectedLane {
            title: INIT_SYNC_SECTION_TITLE,
            label: INIT_SYNC_LANE_LABEL,
            lane: &session.sync_lane,
        });
    }
    if session.run_ingest {
        selected.push(SelectedLane {
            title: INIT_INGEST_SECTION_TITLE,
            label: INIT_INGEST_LANE_LABEL,
            lane: &session.ingest_lane,
        });
    }
    if session.embeddings_selected {
        selected.push(SelectedLane {
            title: INIT_CODE_EMBEDDINGS_SECTION_TITLE,
            label: INIT_CODE_EMBEDDINGS_LANE_LABEL,
            lane: &session.code_embeddings_lane,
        });
    }
    if session.summaries_selected {
        selected.push(SelectedLane {
            title: INIT_SUMMARIES_SECTION_TITLE,
            label: INIT_SUMMARIES_LANE_LABEL,
            lane: &session.summaries_lane,
        });
    }
    if session.summary_embeddings_selected {
        selected.push(SelectedLane {
            title: INIT_SUMMARY_EMBEDDINGS_SECTION_TITLE,
            label: INIT_SUMMARY_EMBEDDINGS_LANE_LABEL,
            lane: &session.summary_embeddings_lane,
        });
    }

    selected
}

fn effective_session_status(session: &RuntimeInitSessionGraphqlRecord) -> Cow<'_, str> {
    if session.status.eq_ignore_ascii_case("completed") && selected_lane_warning(session) {
        Cow::Borrowed("completed_with_warnings")
    } else {
        Cow::Borrowed(session.status.as_str())
    }
}

fn selected_lane_warning(session: &RuntimeInitSessionGraphqlRecord) -> bool {
    selected_lanes(session)
        .iter()
        .any(|selected| selected.lane.status.eq_ignore_ascii_case("warning"))
}

fn session_summary_text(
    snapshot: &RuntimeSnapshotGraphqlRecord,
    session: &RuntimeInitSessionGraphqlRecord,
) -> String {
    let mut parts = vec![session_summary_status_text(session)];
    if let Some(summary) = session.warning_summary.as_ref() {
        parts.push(summary.clone());
    } else if !snapshot.blocked_mailboxes.is_empty() {
        let blocked = snapshot
            .blocked_mailboxes
            .iter()
            .map(|blocked| blocked.display_name.as_str())
            .collect::<Vec<_>>();
        match blocked.as_slice() {
            [] => {}
            [label] => parts.push(format!("Blocked worker pool: {label}")),
            [label, ..] => parts.push(format!(
                "Blocked worker pools: {label} +{} more",
                blocked.len() - 1
            )),
        }
    }
    parts.join(" | ")
}

fn session_summary_status_text(session: &RuntimeInitSessionGraphqlRecord) -> String {
    if let Some(reason) = session.waiting_reason.as_deref() {
        return match reason {
            "waiting_for_follow_up_sync" | "waiting_for_top_level_work" => {
                "Waiting for codebase processing to stabilise".to_string()
            }
            "waiting_on_blocked_mailbox" | "blocked_mailbox" => {
                "Waiting for blocked worker pools".to_string()
            }
            "waiting_for_embeddings_bootstrap" => "Waiting for embeddings to be ready".to_string(),
            "waiting_for_summary_bootstrap" => "Waiting for summaries to be ready".to_string(),
            other => waiting_reason_label(other).to_string(),
        };
    }

    let status = effective_session_status(session);
    match status.to_ascii_lowercase().as_str() {
        "completed" => "Setup tasks completed".to_string(),
        "completed_with_warnings" => "Setup tasks completed with warnings".to_string(),
        "failed" => "Setup failed".to_string(),
        "queued" => "Waiting to start background processing".to_string(),
        "running" => "Building your project's Intelligence Layer".to_string(),
        _ => "Background processing is running".to_string(),
    }
}

fn lane_summary_text(title: &str, lane: &RuntimeInitLaneGraphqlRecord) -> String {
    let mut parts = vec![runtime_status_label(lane.status.as_str()).to_string()];
    if let Some(detail) = lane_detail_text(title, lane) {
        parts.push(detail);
    }
    if lane.queue.queued > 0 || lane.queue.running > 0 || lane.queue.failed > 0 {
        parts.push(queue_state_summary(
            lane.queue.queued.max(0) as u64,
            lane.queue.running.max(0) as u64,
            lane.queue.failed.max(0) as u64,
        ));
    }
    if let Some(warning) = lane.warnings.first() {
        parts.push(format!("Warning: {}", warning.message));
    }
    parts.join(" | ")
}

fn lane_detail_text(title: &str, lane: &RuntimeInitLaneGraphqlRecord) -> Option<String> {
    if let Some(reason) = lane.waiting_reason.as_deref() {
        return Some(waiting_reason_label(reason).to_string());
    }

    if let Some(summary) = lane_progress_text(title, lane.progress.as_ref()) {
        return Some(summary);
    }

    lane.activity_label
        .clone()
        .or_else(|| lane.detail.clone())
        .filter(|_| {
            let status = lane.status.to_ascii_lowercase();
            !(status == "completed" || status == "failed" || status == "skipped")
        })
}

fn lane_progress_text(
    title: &str,
    progress: Option<&RuntimeInitLaneProgressGraphqlRecord>,
) -> Option<String> {
    let counts = lane_progress_counts(progress?)?;

    if title == INIT_CODE_EMBEDDINGS_SECTION_TITLE {
        return Some(format!("{} / {} indexed", counts.completed, counts.total));
    }

    if title == INIT_SUMMARIES_SECTION_TITLE {
        if counts.in_memory_completed > 0 {
            return Some(format!(
                "{} / {} generated · {} persisted",
                counts.visible_completed, counts.total, counts.completed
            ));
        }
        return Some(format!("{} / {} ready", counts.completed, counts.total));
    }

    if title == INIT_SUMMARY_EMBEDDINGS_SECTION_TITLE {
        return Some(format!("{} / {} ready", counts.completed, counts.total));
    }

    Some(format!("{} / {} complete", counts.completed, counts.total))
}

#[derive(Clone, Copy)]
struct LaneProgressCounts {
    completed: u64,
    in_memory_completed: u64,
    total: u64,
    visible_completed: u64,
}

fn lane_progress_counts(
    progress: &RuntimeInitLaneProgressGraphqlRecord,
) -> Option<LaneProgressCounts> {
    let total = progress.total.max(0) as u64;
    if total == 0 {
        return None;
    }

    let completed = (progress.completed.max(0) as u64).min(total);
    let in_memory_completed =
        (progress.in_memory_completed.max(0) as u64).min(total.saturating_sub(completed));
    let visible_completed = completed.saturating_add(in_memory_completed).min(total);
    Some(LaneProgressCounts {
        completed,
        in_memory_completed,
        total,
        visible_completed,
    })
}

fn runtime_status_label(status: &str) -> &'static str {
    if status.eq_ignore_ascii_case("skipped") {
        "Skipped"
    } else {
        session_status_label(status)
    }
}

fn is_terminal_status(status: &str) -> bool {
    matches!(
        status.to_ascii_lowercase().as_str(),
        "completed" | "completed_with_warnings" | "failed"
    )
}
