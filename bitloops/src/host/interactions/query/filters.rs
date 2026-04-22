use chrono::{DateTime, FixedOffset};

use super::state::InteractionQueryState;
use super::types::{InteractionBrowseFilter, InteractionSessionSummary, InteractionTurnSummary};
use crate::host::interactions::types::InteractionEvent;

pub(super) fn session_matches_filter(
    summary: &InteractionSessionSummary,
    filter: &InteractionBrowseFilter,
) -> bool {
    if !matches_time_window(
        session_timestamp(summary),
        filter.since.as_deref(),
        filter.until.as_deref(),
    ) {
        return false;
    }
    if !matches_optional_contains(
        [
            summary.session.actor_name.as_str(),
            summary.session.actor_email.as_str(),
            summary.session.actor_id.as_str(),
        ],
        filter.actor.as_deref(),
    ) {
        return false;
    }
    if !matches_optional_equals(
        summary.session.actor_id.as_str(),
        filter.actor_id.as_deref(),
    ) {
        return false;
    }
    if !matches_optional_equals(
        summary.session.actor_email.as_str(),
        filter.actor_email.as_deref(),
    ) {
        return false;
    }
    if !matches_optional_commit_author(
        summary,
        filter.commit_author.as_deref(),
        filter.commit_author_email.as_deref(),
    ) {
        return false;
    }
    if !matches_optional_equals(summary.session.agent_type.as_str(), filter.agent.as_deref()) {
        return false;
    }
    if !matches_optional_equals(summary.session.model.as_str(), filter.model.as_deref()) {
        return false;
    }
    if !matches_optional_equals(summary.session.branch.as_str(), filter.branch.as_deref()) {
        return false;
    }
    if !matches_optional_equals(
        summary.session.session_id.as_str(),
        filter.session_id.as_deref(),
    ) {
        return false;
    }
    if let Some(turn_id) = filter
        .turn_id
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        && !summary
            .turn_ids
            .iter()
            .any(|candidate| candidate == turn_id)
    {
        return false;
    }
    if let Some(checkpoint_id) = filter
        .checkpoint_id
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        && !summary.checkpoint_ids.iter().any(|id| id == checkpoint_id)
    {
        return false;
    }
    if let Some(tool_use_id) = filter
        .tool_use_id
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        && !summary
            .tool_uses
            .iter()
            .any(|tool_use| tool_use.tool_use_id == tool_use_id)
    {
        return false;
    }
    if let Some(tool_kind) = filter
        .tool_kind
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        && !summary
            .tool_uses
            .iter()
            .any(|tool_use| eq_ignore_ascii_case(tool_use.tool_name.as_str(), tool_kind))
    {
        return false;
    }
    if let Some(has_checkpoint) = filter.has_checkpoint
        && has_checkpoint == summary.checkpoint_ids.is_empty()
    {
        return false;
    }
    if let Some(path) = filter
        .path
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        && !summary
            .file_paths
            .iter()
            .any(|candidate| contains_ignore_ascii_case(candidate, path))
    {
        return false;
    }
    true
}

pub(super) fn turn_matches_filter(
    summary: &InteractionTurnSummary,
    filter: &InteractionBrowseFilter,
) -> bool {
    if !matches_time_window(
        turn_timestamp(summary),
        filter.since.as_deref(),
        filter.until.as_deref(),
    ) {
        return false;
    }
    if !matches_optional_contains(
        [
            summary.turn.actor_name.as_str(),
            summary.turn.actor_email.as_str(),
            summary.turn.actor_id.as_str(),
        ],
        filter.actor.as_deref(),
    ) {
        return false;
    }
    if !matches_optional_equals(summary.turn.actor_id.as_str(), filter.actor_id.as_deref()) {
        return false;
    }
    if !matches_optional_equals(
        summary.turn.actor_email.as_str(),
        filter.actor_email.as_deref(),
    ) {
        return false;
    }
    if !matches_optional_commit_author_turn(
        summary,
        filter.commit_author.as_deref(),
        filter.commit_author_email.as_deref(),
    ) {
        return false;
    }
    if !matches_optional_equals(summary.turn.agent_type.as_str(), filter.agent.as_deref()) {
        return false;
    }
    if !matches_optional_equals(summary.turn.model.as_str(), filter.model.as_deref()) {
        return false;
    }
    if !matches_optional_equals(summary.turn.branch.as_str(), filter.branch.as_deref()) {
        return false;
    }
    if !matches_optional_equals(
        summary.turn.session_id.as_str(),
        filter.session_id.as_deref(),
    ) {
        return false;
    }
    if !matches_optional_equals(summary.turn.turn_id.as_str(), filter.turn_id.as_deref()) {
        return false;
    }
    if let Some(checkpoint_id) = filter
        .checkpoint_id
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        && summary.turn.checkpoint_id.as_deref() != Some(checkpoint_id)
    {
        return false;
    }
    if let Some(tool_use_id) = filter
        .tool_use_id
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        && !summary
            .tool_uses
            .iter()
            .any(|tool_use| tool_use.tool_use_id == tool_use_id)
    {
        return false;
    }
    if let Some(tool_kind) = filter
        .tool_kind
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        && !summary
            .tool_uses
            .iter()
            .any(|tool_use| eq_ignore_ascii_case(tool_use.tool_name.as_str(), tool_kind))
    {
        return false;
    }
    if let Some(has_checkpoint) = filter.has_checkpoint
        && has_checkpoint != summary.turn.checkpoint_id.is_some()
    {
        return false;
    }
    if let Some(path) = filter
        .path
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        && !summary
            .turn
            .files_modified
            .iter()
            .any(|candidate| contains_ignore_ascii_case(candidate, path))
    {
        return false;
    }
    true
}

pub(super) fn event_matches_filter(
    event: &InteractionEvent,
    state: &InteractionQueryState,
    filter: &InteractionBrowseFilter,
) -> bool {
    if !matches_time_window(
        &event.event_time,
        filter.since.as_deref(),
        filter.until.as_deref(),
    ) {
        return false;
    }
    if !matches_optional_contains(
        [
            event.actor_name.as_str(),
            event.actor_email.as_str(),
            event.actor_id.as_str(),
        ],
        filter.actor.as_deref(),
    ) {
        return false;
    }
    if !matches_optional_equals(event.actor_id.as_str(), filter.actor_id.as_deref()) {
        return false;
    }
    if !matches_optional_equals(event.actor_email.as_str(), filter.actor_email.as_deref()) {
        return false;
    }
    if !matches_optional_equals(event.agent_type.as_str(), filter.agent.as_deref()) {
        return false;
    }
    if !matches_optional_equals(event.model.as_str(), filter.model.as_deref()) {
        return false;
    }
    if !matches_optional_equals(event.branch.as_str(), filter.branch.as_deref()) {
        return false;
    }
    if !matches_optional_equals(event.session_id.as_str(), filter.session_id.as_deref()) {
        return false;
    }
    if let Some(turn_id) = filter
        .turn_id
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        && event.turn_id.as_deref() != Some(turn_id)
    {
        return false;
    }
    if !matches_optional_equals(event.tool_use_id.as_str(), filter.tool_use_id.as_deref()) {
        return false;
    }
    if !matches_optional_equals(event.tool_kind.as_str(), filter.tool_kind.as_deref()) {
        return false;
    }
    if filter.commit_author.is_some()
        || filter.commit_author_email.is_some()
        || filter.checkpoint_id.is_some()
        || filter.has_checkpoint.is_some()
        || filter.path.is_some()
    {
        let Some(turn_id) = event.turn_id.as_deref() else {
            return false;
        };
        let Some(turn) = state.turn_summaries.get(turn_id) else {
            return false;
        };
        if !turn_matches_filter(turn, filter) {
            return false;
        }
    }
    true
}

pub(super) fn session_sort_key(summary: &InteractionSessionSummary) -> (String, String) {
    (
        session_timestamp(summary).to_string(),
        summary.session.session_id.clone(),
    )
}

pub(super) fn turn_sort_key(summary: &InteractionTurnSummary) -> (String, String) {
    (
        turn_timestamp(summary).to_string(),
        summary.turn.turn_id.clone(),
    )
}

fn matches_optional_commit_author(
    summary: &InteractionSessionSummary,
    commit_author: Option<&str>,
    commit_author_email: Option<&str>,
) -> bool {
    let author_matches = summary.linked_checkpoints.iter().any(|checkpoint| {
        matches_optional_contains(
            [
                checkpoint.author_name.as_str(),
                checkpoint.author_email.as_str(),
            ],
            commit_author,
        ) && matches_optional_equals(checkpoint.author_email.as_str(), commit_author_email)
    });
    commit_author.is_none() && commit_author_email.is_none() || author_matches
}

fn matches_optional_commit_author_turn(
    summary: &InteractionTurnSummary,
    commit_author: Option<&str>,
    commit_author_email: Option<&str>,
) -> bool {
    let author_matches = summary.linked_checkpoints.iter().any(|checkpoint| {
        matches_optional_contains(
            [
                checkpoint.author_name.as_str(),
                checkpoint.author_email.as_str(),
            ],
            commit_author,
        ) && matches_optional_equals(checkpoint.author_email.as_str(), commit_author_email)
    });
    commit_author.is_none() && commit_author_email.is_none() || author_matches
}

fn matches_time_window(value: &str, since: Option<&str>, until: Option<&str>) -> bool {
    let value_dt = parse_rfc3339(value);
    if let Some(since) = since.filter(|candidate| !candidate.trim().is_empty()) {
        let since_dt = parse_rfc3339(since);
        if let (Some(value_dt), Some(since_dt)) = (value_dt.as_ref(), since_dt.as_ref()) {
            if value_dt < since_dt {
                return false;
            }
        } else if value < since {
            return false;
        }
    }
    if let Some(until) = until.filter(|candidate| !candidate.trim().is_empty()) {
        let until_dt = parse_rfc3339(until);
        if let (Some(value_dt), Some(until_dt)) = (value_dt.as_ref(), until_dt.as_ref()) {
            if value_dt > until_dt {
                return false;
            }
        } else if value > until {
            return false;
        }
    }
    true
}

fn parse_rfc3339(value: &str) -> Option<DateTime<FixedOffset>> {
    DateTime::parse_from_rfc3339(value).ok()
}

fn matches_optional_equals(actual: &str, expected: Option<&str>) -> bool {
    let Some(expected) = expected.map(str::trim).filter(|value| !value.is_empty()) else {
        return true;
    };
    eq_ignore_ascii_case(actual, expected)
}

fn matches_optional_contains<'a, I>(values: I, expected: Option<&str>) -> bool
where
    I: IntoIterator<Item = &'a str>,
{
    let Some(expected) = expected.map(str::trim).filter(|value| !value.is_empty()) else {
        return true;
    };
    values
        .into_iter()
        .any(|value| contains_ignore_ascii_case(value, expected))
}

fn contains_ignore_ascii_case(left: &str, right: &str) -> bool {
    left.to_ascii_lowercase()
        .contains(&right.to_ascii_lowercase())
}

fn eq_ignore_ascii_case(left: &str, right: &str) -> bool {
    left.trim().eq_ignore_ascii_case(right.trim())
}

fn session_timestamp(summary: &InteractionSessionSummary) -> &str {
    if summary.session.last_event_at.trim().is_empty() {
        summary.session.started_at.as_str()
    } else {
        summary.session.last_event_at.as_str()
    }
}

fn turn_timestamp(summary: &InteractionTurnSummary) -> &str {
    summary
        .turn
        .ended_at
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or(summary.turn.started_at.as_str())
}

#[cfg(test)]
mod tests {
    use super::matches_time_window;

    #[test]
    fn matches_time_window_accepts_equivalent_zero_offset_boundaries() {
        assert!(matches_time_window(
            "2026-02-27T12:05:00Z",
            Some("2026-02-27T12:00:00+00:00"),
            Some("2026-02-27T12:05:00+00:00"),
        ));
    }
}
