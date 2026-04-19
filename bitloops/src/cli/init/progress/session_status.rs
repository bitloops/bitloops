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

    match session.status.to_ascii_lowercase().as_str() {
        "completed" => "Setup tasks completed".to_string(),
        "completed_with_warnings" => "Setup tasks completed with warnings".to_string(),
        "failed" => "Setup failed".to_string(),
        "queued" => "Waiting to start background processing".to_string(),
        "running" => "Building your project's Intelligence Layer".to_string(),
        _ => "Background processing is running".to_string(),
    }
}
