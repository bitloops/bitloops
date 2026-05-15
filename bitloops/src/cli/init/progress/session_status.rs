use crate::runtime_presentation::waiting_reason_label;

pub(crate) fn compact_session_status_line(
    snapshot: &crate::cli::devql::graphql::RuntimeSnapshotGraphqlRecord,
    session: &crate::cli::devql::graphql::RuntimeInitSessionGraphqlRecord,
) -> String {
    let mut parts = vec![compact_session_status_text(session)];
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

fn compact_session_status_text(
    session: &crate::cli::devql::graphql::RuntimeInitSessionGraphqlRecord,
) -> String {
    if let Some(reason) = session.waiting_reason.as_deref() {
        return match reason {
            "waiting_for_top_level_work" if selected_semantic_lane_is_active(session) => {
                "Building your project's Intelligence Layer".to_string()
            }
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

    let status = effective_compact_session_status(session);
    match status.as_str() {
        "completed" => "Setup tasks completed".to_string(),
        "completed_with_warnings" => "Setup tasks completed with warnings".to_string(),
        "failed" => "Setup failed".to_string(),
        "queued" => "Waiting to start background processing".to_string(),
        "running" => "Building your project's Intelligence Layer".to_string(),
        _ => "Background processing is running".to_string(),
    }
}

fn effective_compact_session_status(
    session: &crate::cli::devql::graphql::RuntimeInitSessionGraphqlRecord,
) -> String {
    if session.status.eq_ignore_ascii_case("completed") && selected_warning_lane(session) {
        "completed_with_warnings".to_string()
    } else {
        session.status.to_ascii_lowercase()
    }
}

fn selected_semantic_lane_is_active(
    session: &crate::cli::devql::graphql::RuntimeInitSessionGraphqlRecord,
) -> bool {
    fn lane_is_active(lane: &crate::cli::devql::graphql::RuntimeInitLaneGraphqlRecord) -> bool {
        lane.status.eq_ignore_ascii_case("queued") || lane.status.eq_ignore_ascii_case("running")
    }

    (session.embeddings_selected && lane_is_active(&session.code_embeddings_lane))
        || (session.summaries_selected && lane_is_active(&session.summaries_lane))
        || (session.summary_embeddings_selected && lane_is_active(&session.summary_embeddings_lane))
}

fn selected_warning_lane(
    session: &crate::cli::devql::graphql::RuntimeInitSessionGraphqlRecord,
) -> bool {
    (session.run_sync && session.sync_lane.status.eq_ignore_ascii_case("warning"))
        || (session.run_ingest && session.ingest_lane.status.eq_ignore_ascii_case("warning"))
        || (session.embeddings_selected
            && session
                .code_embeddings_lane
                .status
                .eq_ignore_ascii_case("warning"))
        || (session.summaries_selected
            && session
                .summaries_lane
                .status
                .eq_ignore_ascii_case("warning"))
        || (session.summary_embeddings_selected
            && session
                .summary_embeddings_lane
                .status
                .eq_ignore_ascii_case("warning"))
}

#[cfg(test)]
mod tests {
    use super::compact_session_status_text;
    use crate::cli::devql::graphql::{
        RuntimeInitLaneGraphqlRecord, RuntimeInitLaneQueueGraphqlRecord,
        RuntimeInitSessionGraphqlRecord,
    };

    fn lane(status: &str) -> RuntimeInitLaneGraphqlRecord {
        RuntimeInitLaneGraphqlRecord {
            status: status.to_string(),
            waiting_reason: None,
            detail: None,
            activity_label: None,
            task_id: None,
            run_id: None,
            progress: None,
            queue: RuntimeInitLaneQueueGraphqlRecord::default(),
            warnings: Vec::new(),
            pending_count: 0,
            running_count: 0,
            failed_count: 0,
            completed_count: 0,
        }
    }

    fn session(waiting_reason: &str) -> RuntimeInitSessionGraphqlRecord {
        RuntimeInitSessionGraphqlRecord {
            init_session_id: "init-session-1".to_string(),
            status: "waiting".to_string(),
            waiting_reason: Some(waiting_reason.to_string()),
            warning_summary: None,
            follow_up_sync_required: false,
            run_sync: true,
            run_ingest: true,
            embeddings_selected: true,
            summaries_selected: true,
            summary_embeddings_selected: true,
            initial_sync_task_id: None,
            ingest_task_id: None,
            follow_up_sync_task_id: None,
            embeddings_bootstrap_task_id: None,
            summary_bootstrap_task_id: None,
            terminal_error: None,
            sync_lane: lane("completed"),
            ingest_lane: lane("running"),
            code_embeddings_lane: lane("queued"),
            summaries_lane: lane("running"),
            summary_embeddings_lane: lane("queued"),
        }
    }

    #[test]
    fn compact_session_status_prefers_running_copy_when_semantic_lanes_are_active() {
        assert_eq!(
            compact_session_status_text(&session("waiting_for_top_level_work")),
            "Building your project's Intelligence Layer"
        );
    }

    #[test]
    fn compact_session_status_keeps_waiting_copy_without_active_semantic_lanes() {
        let mut session = session("waiting_for_top_level_work");
        session.code_embeddings_lane = lane("waiting");
        session.summaries_lane = lane("waiting");
        session.summary_embeddings_lane = lane("waiting");

        assert_eq!(
            compact_session_status_text(&session),
            "Waiting for codebase processing to stabilise"
        );
    }
}
