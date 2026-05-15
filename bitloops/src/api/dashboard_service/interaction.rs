use std::path::PathBuf;

use chrono::{DateTime, SecondsFormat, Utc};
use tokio::task;

use crate::adapters::agents::AgentRegistry;
use crate::api::dashboard_types::{
    DashboardInteractionActorBucket, DashboardInteractionAgentBucket,
    DashboardInteractionCommitAuthorBucket, DashboardInteractionFilterInput,
    DashboardInteractionKpis, DashboardInteractionSearchInput, DashboardInteractionSession,
    DashboardInteractionSessionDetail, DashboardInteractionSessionSearchHit,
    DashboardInteractionTurnSearchHit, DashboardInteractionUpdate, DashboardTranscriptEntry,
};
use crate::api::{API_DEFAULT_PAGE_LIMIT, ApiError, ApiPage, DashboardState, paginate};
use crate::host::interactions::query;
use crate::host::interactions::transcript_entry::{
    TranscriptActor, TranscriptEntry, TranscriptSource,
};
use crate::host::interactions::types::InteractionTurn;
use crate::host::interactions::{
    derive_session_transcript_entries, derive_turn_transcript_entries,
    partition_session_entries_to_turns, read_session_transcript_text, strip_user_query_tags,
};

use super::repository::resolve_dashboard_repo_root;

fn normalise_optional(value: Option<String>) -> Option<String> {
    value.and_then(|raw| {
        let trimmed = raw.trim();
        (!trimmed.is_empty()).then(|| trimmed.to_string())
    })
}

fn parse_optional_rfc3339(
    field: &str,
    value: Option<String>,
) -> std::result::Result<Option<String>, ApiError> {
    let Some(raw) = normalise_optional(value) else {
        return Ok(None);
    };
    let parsed = DateTime::parse_from_rfc3339(&raw).map_err(|_| {
        ApiError::bad_request(format!("invalid {field}; expected RFC3339 datetime"))
    })?;
    Ok(Some(
        parsed
            .with_timezone(&Utc)
            .to_rfc3339_opts(SecondsFormat::Secs, true),
    ))
}

fn validate_rfc3339_window(
    since: Option<&str>,
    until: Option<&str>,
) -> std::result::Result<(), ApiError> {
    if let (Some(since), Some(until)) = (since, until) {
        let since = DateTime::parse_from_rfc3339(since)
            .map_err(|_| ApiError::bad_request("invalid since; expected RFC3339 datetime"))?;
        let until = DateTime::parse_from_rfc3339(until)
            .map_err(|_| ApiError::bad_request("invalid until; expected RFC3339 datetime"))?;
        if since > until {
            return Err(ApiError::bad_request(
                "since must be less than or equal to until",
            ));
        }
    }
    Ok(())
}

fn parse_optional_page_value(
    field: &str,
    value: Option<i32>,
) -> std::result::Result<Option<usize>, ApiError> {
    let Some(value) = value else {
        return Ok(None);
    };
    if value < 0 {
        return Err(ApiError::bad_request(format!(
            "invalid {field}; expected non-negative integer"
        )));
    }
    Ok(Some(value as usize))
}

fn parse_page(limit: Option<i32>, offset: Option<i32>) -> std::result::Result<ApiPage, ApiError> {
    Ok(ApiPage {
        limit: parse_optional_page_value("limit", limit)?.unwrap_or(API_DEFAULT_PAGE_LIMIT),
        offset: parse_optional_page_value("offset", offset)?.unwrap_or(0),
    }
    .normalized())
}

fn normalise_filter(
    filter: Option<DashboardInteractionFilterInput>,
) -> std::result::Result<query::InteractionBrowseFilter, ApiError> {
    let filter = filter.unwrap_or_default();
    let since = parse_optional_rfc3339("since", filter.since)?;
    let until = parse_optional_rfc3339("until", filter.until)?;
    validate_rfc3339_window(since.as_deref(), until.as_deref())?;

    Ok(query::InteractionBrowseFilter {
        since,
        until,
        actor: normalise_optional(filter.actor),
        actor_id: normalise_optional(filter.actor_id),
        actor_email: normalise_optional(filter.actor_email),
        commit_author: normalise_optional(filter.commit_author),
        commit_author_email: normalise_optional(filter.commit_author_email),
        agent: normalise_optional(filter.agent),
        model: normalise_optional(filter.model),
        branch: normalise_optional(filter.branch),
        session_id: normalise_optional(filter.session_id),
        turn_id: normalise_optional(filter.turn_id),
        checkpoint_id: normalise_optional(filter.checkpoint_id),
        tool_use_id: normalise_optional(filter.tool_use_id),
        tool_kind: normalise_optional(filter.tool_kind),
        has_checkpoint: filter.has_checkpoint,
        path: normalise_optional(filter.path),
    })
}

fn normalise_search_input(
    input: DashboardInteractionSearchInput,
) -> std::result::Result<query::InteractionSearchInput, ApiError> {
    let query_string = input.query.trim().to_string();
    if query_string.is_empty() {
        return Err(ApiError::bad_request("query is required"));
    }
    Ok(query::InteractionSearchInput {
        filter: normalise_filter(input.filter)?,
        query: query_string,
        limit: input
            .limit
            .map(|value| usize::try_from(value.max(1)).unwrap_or(25))
            .unwrap_or(25),
    })
}

pub(in crate::api) async fn load_dashboard_interaction_kpis(
    state: &DashboardState,
    repo_id: Option<String>,
    filter: Option<DashboardInteractionFilterInput>,
) -> std::result::Result<DashboardInteractionKpis, ApiError> {
    let repo_root = resolve_dashboard_repo_root(state, repo_id.as_deref()).await?;
    let filter = normalise_filter(filter)?;
    task::spawn_blocking(move || query::compute_kpis(&repo_root, &filter))
        .await
        .map_err(|err| ApiError::internal(format!("failed to join interaction KPI task: {err:#}")))?
        .map(|kpis| DashboardInteractionKpis::from_domain(&kpis))
        .map_err(|err| ApiError::internal(format!("failed to load interaction KPIs: {err:#}")))
}

pub(in crate::api) async fn load_dashboard_interaction_update(
    state: &DashboardState,
    repo_id: Option<String>,
) -> std::result::Result<DashboardInteractionUpdate, ApiError> {
    let repo_root = resolve_dashboard_repo_root(state, repo_id.as_deref()).await?;
    load_dashboard_interaction_update_for_repo_root(repo_root).await
}

pub(in crate::api) async fn load_dashboard_interaction_update_for_repo_root(
    repo_root: PathBuf,
) -> std::result::Result<DashboardInteractionUpdate, ApiError> {
    task::spawn_blocking(move || query::interaction_change_snapshot(&repo_root))
        .await
        .map_err(|err| {
            ApiError::internal(format!(
                "failed to join interaction update snapshot task: {err:#}"
            ))
        })?
        .map(|snapshot| DashboardInteractionUpdate::from_domain(&snapshot))
        .map_err(|err| {
            ApiError::internal(format!(
                "failed to load interaction update snapshot: {err:#}"
            ))
        })
}

pub(in crate::api) async fn load_dashboard_interaction_sessions(
    state: &DashboardState,
    repo_id: Option<String>,
    filter: Option<DashboardInteractionFilterInput>,
    limit: Option<i32>,
    offset: Option<i32>,
) -> std::result::Result<Vec<DashboardInteractionSession>, ApiError> {
    let repo_root = resolve_dashboard_repo_root(state, repo_id.as_deref()).await?;
    let filter = normalise_filter(filter)?;
    let page = parse_page(limit, offset)?;
    let rows = task::spawn_blocking(move || query::list_session_summaries(&repo_root, &filter))
        .await
        .map_err(|err| {
            ApiError::internal(format!("failed to join interaction sessions task: {err:#}"))
        })?
        .map_err(|err| {
            ApiError::internal(format!("failed to load interaction sessions: {err:#}"))
        })?;
    Ok(paginate(&rows, page)
        .into_iter()
        .map(|summary| DashboardInteractionSession::from_summary(&summary))
        .collect())
}

pub(in crate::api) async fn load_dashboard_interaction_session(
    state: &DashboardState,
    repo_id: Option<String>,
    session_id: String,
) -> std::result::Result<DashboardInteractionSessionDetail, ApiError> {
    let repo_root = resolve_dashboard_repo_root(state, repo_id.as_deref()).await?;
    let session_id = session_id.trim().to_string();
    if session_id.is_empty() {
        return Err(ApiError::bad_request("sessionId is required"));
    }
    let dashboard_detail = task::spawn_blocking(move || {
        let detail = query::load_session_detail(&repo_root, &session_id)?;
        Ok::<Option<DashboardInteractionSessionDetail>, anyhow::Error>(
            detail.map(enrich_session_detail_with_transcript_entries),
        )
    })
    .await
    .map_err(|err| {
        ApiError::internal(format!(
            "failed to join interaction session detail task: {err:#}"
        ))
    })?
    .map_err(|err| {
        ApiError::internal(format!(
            "failed to load interaction session detail: {err:#}"
        ))
    })?;
    let Some(dashboard_detail) = dashboard_detail else {
        return Err(ApiError::not_found("unknown interaction session"));
    };
    Ok(dashboard_detail)
}

/// Drop "phantom" duplicate turn rows before the dashboard renders them.
///
/// When two lifecycle paths both write turn rows for the same prompt (e.g.,
/// Cursor's `before-submit-prompt` fires once and a Claude-style hook also
/// fires on the same session), the spool ends up with two `interaction_turns`
/// rows sharing the same normalized prompt. One is "complete" (has
/// `transcript_offset_end`, `ended_at`, and/or `transcript_fragment`), the
/// other is an orphan that never closed. The complete row carries the
/// assistant response; the orphan, if left in, becomes a duplicate
/// prompt-only turn on the timeline.
///
/// This filter is conservative: it only drops a turn when *another turn in
/// the same session* covers the same normalized prompt (post-`<user_query>`
/// strip, trim, lowercase) AND the dropped turn has zero transcript signal.
/// In-flight turns at the tail of a session — where no sibling exists — are
/// preserved.
fn dedupe_phantom_turns(
    mut detail: query::InteractionSessionDetail,
) -> query::InteractionSessionDetail {
    use std::collections::{HashMap, HashSet};

    fn is_orphan(turn: &InteractionTurn) -> bool {
        let no_end_offset = turn.transcript_offset_end.unwrap_or(0) <= 0;
        let no_ended_at = turn
            .ended_at
            .as_deref()
            .map(str::trim)
            .unwrap_or("")
            .is_empty();
        let no_fragment = turn.transcript_fragment.trim().is_empty();
        no_end_offset && no_ended_at && no_fragment
    }

    let normalize =
        |prompt: &str| -> String { strip_user_query_tags(prompt).trim().to_lowercase() };

    if detail.turns.len() <= 1 {
        return detail;
    }

    let mut by_prompt: HashMap<String, Vec<usize>> = HashMap::new();
    for (idx, summary) in detail.turns.iter().enumerate() {
        let key = normalize(&summary.turn.prompt);
        if key.is_empty() {
            continue;
        }
        by_prompt.entry(key).or_default().push(idx);
    }

    let mut drop_idx: HashSet<usize> = HashSet::new();
    for indices in by_prompt.values() {
        if indices.len() <= 1 {
            continue;
        }
        let any_complete = indices.iter().any(|&i| !is_orphan(&detail.turns[i].turn));
        if !any_complete {
            // No row in this group is complete — keep them all so we don't
            // hide an in-flight conversation. (At most one of these will be
            // the current live turn.)
            continue;
        }
        for &i in indices {
            if is_orphan(&detail.turns[i].turn) {
                drop_idx.insert(i);
            }
        }
    }

    if drop_idx.is_empty() {
        return detail;
    }

    detail.turns = detail
        .turns
        .into_iter()
        .enumerate()
        .filter(|(idx, _)| !drop_idx.contains(idx))
        .map(|(_, summary)| summary)
        .collect();
    detail
}

/// Convert a domain `InteractionSessionDetail` into the dashboard response,
/// enriching it with canonical transcript entries derived from the session's
/// agent. Reads the transcript file from disk **once** and slices it per turn
/// using each turn's `transcript_offset_start`/`end` markers — call from
/// inside `spawn_blocking`.
fn enrich_session_detail_with_transcript_entries(
    detail: query::InteractionSessionDetail,
) -> DashboardInteractionSessionDetail {
    let detail = dedupe_phantom_turns(detail);
    let agent_type = detail.summary.session.agent_type.clone();
    let session_id_str = detail.summary.session.session_id.clone();
    let transcript_path = detail.summary.session.transcript_path.clone();

    let registry = AgentRegistry::builtin();
    let agent_opt = registry.get_by_agent_type(&agent_type).ok();

    // Read the live transcript file once. Empty when path is blank or file is
    // unreadable; downstream calls degrade gracefully via prompt fallback.
    let full_transcript = read_session_transcript_text(&transcript_path);

    // Derive the session-wide stream (canonical entries from the whole file).
    let session_entries_raw: Vec<TranscriptEntry> = if let Some(agent) = agent_opt {
        derive_session_transcript_entries(&session_id_str, &full_transcript, agent)
    } else {
        Vec::new()
    };

    // Per-turn: try offset-based slicing first. If any turn comes back with
    // only PROMPT_FALLBACK (offset markers missing/invalid for this agent, or
    // the slice produced nothing), fall back to content-based partitioning of
    // the session-wide stream. This covers agents whose offset semantics don't
    // align with `slice_transcript_by_position` for the session shape on disk.
    let offset_based: Vec<Vec<TranscriptEntry>> = detail
        .turns
        .iter()
        .map(|summary| {
            if let Some(agent) = agent_opt {
                derive_turn_transcript_entries(
                    &session_id_str,
                    &summary.turn,
                    &full_transcript,
                    agent,
                )
            } else {
                Vec::new()
            }
        })
        .collect();

    // Cumulative-offset detection: when more than one turn has
    // `transcript_offset_start = 0`, the host wasn't tracking per-turn
    // boundaries (Claude and Cursor exhibit this — the pre-prompt offset
    // capture doesn't fire or doesn't pin a non-zero start). In that case
    // every turn's offset slice covers the cumulative content from the start
    // of the session, which means each turn duplicates the previous turns.
    let zero_start_turns = detail
        .turns
        .iter()
        .filter(|s| {
            s.turn.transcript_offset_start.unwrap_or(-1) == 0
                && s.turn.transcript_offset_end.unwrap_or(0) > 0
        })
        .count();
    let offsets_look_cumulative = zero_start_turns > 1;

    // Missing-assistant detection: the session-wide stream contains assistant
    // (or system) entries but at least one turn's offset slice produced none.
    // This happens for Gemini when older sessions on disk still have
    // line-count offset markers that the message-index slicer cannot
    // interpret — the slice returns just the user prompt line and drops the
    // assistant/tool content. Force the partition fallback in that case so the
    // dashboard shows the assistant response for those turns too.
    let session_has_assistant = session_entries_raw.iter().any(|e| {
        matches!(
            e.actor,
            TranscriptActor::Assistant | TranscriptActor::System
        )
    });
    let per_turn_missing_assistant = offset_based.iter().any(|entries| {
        !entries.is_empty()
            && !entries.iter().any(|e| {
                matches!(
                    e.actor,
                    TranscriptActor::Assistant | TranscriptActor::System
                )
            })
    });

    // Fall back to content-based partitioning when:
    //   - offsets look cumulative (Claude / Cursor case), OR
    //   - any turn came back with only `PROMPT_FALLBACK` entries (offsets
    //     missing/invalid; Gemini exhibits this), OR
    //   - the session has assistant content but a turn's slice doesn't
    //     (Gemini stale-offset case).
    let needs_partition_fallback = !session_entries_raw.is_empty()
        && (offsets_look_cumulative
            || offset_based.iter().any(|entries| {
                entries.is_empty()
                    || entries
                        .iter()
                        .all(|e| matches!(e.source, TranscriptSource::PromptFallback))
            })
            || (session_has_assistant && per_turn_missing_assistant));

    let per_turn_raw: Vec<Vec<TranscriptEntry>> = if needs_partition_fallback {
        let turns_refs: Vec<&InteractionTurn> = detail.turns.iter().map(|s| &s.turn).collect();
        partition_session_entries_to_turns(&session_id_str, &session_entries_raw, &turns_refs)
    } else {
        offset_based
    };

    let session_entries: Vec<DashboardTranscriptEntry> = session_entries_raw
        .into_iter()
        .map(DashboardTranscriptEntry::from)
        .collect();

    let per_turn_entries: Vec<Vec<DashboardTranscriptEntry>> = per_turn_raw
        .into_iter()
        .map(|entries| {
            entries
                .into_iter()
                .map(DashboardTranscriptEntry::from)
                .collect()
        })
        .collect();

    let mut dashboard = DashboardInteractionSessionDetail::from_domain(&detail)
        .with_session_transcript_entries(session_entries);
    for (idx, entries) in per_turn_entries.into_iter().enumerate() {
        if let Some(turn) = dashboard.turns.get_mut(idx) {
            turn.transcript_entries = entries;
        }
    }
    dashboard
}

pub(in crate::api) async fn load_dashboard_interaction_actors(
    state: &DashboardState,
    repo_id: Option<String>,
    filter: Option<DashboardInteractionFilterInput>,
) -> std::result::Result<Vec<DashboardInteractionActorBucket>, ApiError> {
    let repo_root = resolve_dashboard_repo_root(state, repo_id.as_deref()).await?;
    let filter = normalise_filter(filter)?;
    task::spawn_blocking(move || query::list_actor_buckets(&repo_root, &filter))
        .await
        .map_err(|err| {
            ApiError::internal(format!("failed to join interaction actors task: {err:#}"))
        })?
        .map(|rows| {
            rows.iter()
                .map(DashboardInteractionActorBucket::from_domain)
                .collect()
        })
        .map_err(|err| ApiError::internal(format!("failed to load interaction actors: {err:#}")))
}

pub(in crate::api) async fn load_dashboard_interaction_commit_authors(
    state: &DashboardState,
    repo_id: Option<String>,
    filter: Option<DashboardInteractionFilterInput>,
) -> std::result::Result<Vec<DashboardInteractionCommitAuthorBucket>, ApiError> {
    let repo_root = resolve_dashboard_repo_root(state, repo_id.as_deref()).await?;
    let filter = normalise_filter(filter)?;
    task::spawn_blocking(move || query::list_commit_author_buckets(&repo_root, &filter))
        .await
        .map_err(|err| {
            ApiError::internal(format!(
                "failed to join interaction commit authors task: {err:#}"
            ))
        })?
        .map(|rows| {
            rows.iter()
                .map(DashboardInteractionCommitAuthorBucket::from_domain)
                .collect()
        })
        .map_err(|err| {
            ApiError::internal(format!(
                "failed to load interaction commit authors: {err:#}"
            ))
        })
}

pub(in crate::api) async fn load_dashboard_interaction_agents(
    state: &DashboardState,
    repo_id: Option<String>,
    filter: Option<DashboardInteractionFilterInput>,
) -> std::result::Result<Vec<DashboardInteractionAgentBucket>, ApiError> {
    let repo_root = resolve_dashboard_repo_root(state, repo_id.as_deref()).await?;
    let filter = normalise_filter(filter)?;
    task::spawn_blocking(move || query::list_agent_buckets(&repo_root, &filter))
        .await
        .map_err(|err| {
            ApiError::internal(format!("failed to join interaction agents task: {err:#}"))
        })?
        .map(|rows| {
            rows.iter()
                .map(DashboardInteractionAgentBucket::from_domain)
                .collect()
        })
        .map_err(|err| ApiError::internal(format!("failed to load interaction agents: {err:#}")))
}

pub(in crate::api) async fn search_dashboard_interaction_sessions(
    state: &DashboardState,
    repo_id: Option<String>,
    input: DashboardInteractionSearchInput,
) -> std::result::Result<Vec<DashboardInteractionSessionSearchHit>, ApiError> {
    let repo_root = resolve_dashboard_repo_root(state, repo_id.as_deref()).await?;
    let input = normalise_search_input(input)?;
    task::spawn_blocking(move || query::search_session_summaries(&repo_root, &input))
        .await
        .map_err(|err| {
            ApiError::internal(format!(
                "failed to join interaction session search task: {err:#}"
            ))
        })?
        .map(|rows| {
            rows.iter()
                .map(DashboardInteractionSessionSearchHit::from_domain)
                .collect()
        })
        .map_err(|err| {
            ApiError::internal(format!("failed to search interaction sessions: {err:#}"))
        })
}

pub(in crate::api) async fn search_dashboard_interaction_turns(
    state: &DashboardState,
    repo_id: Option<String>,
    input: DashboardInteractionSearchInput,
) -> std::result::Result<Vec<DashboardInteractionTurnSearchHit>, ApiError> {
    let repo_root = resolve_dashboard_repo_root(state, repo_id.as_deref()).await?;
    let input = normalise_search_input(input)?;
    task::spawn_blocking(move || query::search_turn_summaries(&repo_root, &input))
        .await
        .map_err(|err| {
            ApiError::internal(format!(
                "failed to join interaction turn search task: {err:#}"
            ))
        })?
        .map(|rows| {
            rows.iter()
                .map(DashboardInteractionTurnSearchHit::from_domain)
                .collect()
        })
        .map_err(|err| ApiError::internal(format!("failed to search interaction turns: {err:#}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::interactions::InteractionSession;
    use crate::host::interactions::query::{
        InteractionSessionDetail, InteractionSessionSummary, InteractionTurnSummary,
    };

    fn make_summary() -> InteractionSessionSummary {
        InteractionSessionSummary {
            session: InteractionSession {
                session_id: "04ca51b0".to_string(),
                ..InteractionSession::default()
            },
            turn_count: 0,
            turn_ids: Vec::new(),
            checkpoint_count: 0,
            checkpoint_ids: Vec::new(),
            token_usage: None,
            file_paths: Vec::new(),
            tool_uses: Vec::new(),
            subagent_runs: Vec::new(),
            linked_checkpoints: Vec::new(),
            latest_commit_author: None,
        }
    }

    fn make_turn(
        turn_id: &str,
        prompt: &str,
        agent_type: &str,
        started_at: &str,
        offset_end: Option<i64>,
        ended_at: Option<&str>,
    ) -> InteractionTurnSummary {
        InteractionTurnSummary {
            turn: InteractionTurn {
                turn_id: turn_id.to_string(),
                session_id: "04ca51b0".to_string(),
                prompt: prompt.to_string(),
                agent_type: agent_type.to_string(),
                started_at: started_at.to_string(),
                transcript_offset_start: Some(0),
                transcript_offset_end: offset_end,
                ended_at: ended_at.map(str::to_string),
                ..InteractionTurn::default()
            },
            tool_uses: Vec::new(),
            subagent_runs: Vec::new(),
            linked_checkpoints: Vec::new(),
            latest_commit_author: None,
        }
    }

    /// Fixture mirrors the real interaction_turns rows captured for Cursor
    /// session 04ca51b0: four claude-code rows with `<user_query>` wrappers and
    /// populated offsets/ended_at, plus one cursor row for "thanks" with no
    /// offsets and no ended_at — the phantom that produced the duplicate.
    #[test]
    fn dedupe_phantom_turns_drops_orphan_sharing_prompt_with_complete_sibling() {
        let detail = InteractionSessionDetail {
            summary: make_summary(),
            turns: vec![
                make_turn(
                    "a0201f30a07b",
                    "<user_query>\nupdate readme title to cursor\n</user_query>",
                    "claude-code",
                    "2026-05-14T15:39:38Z",
                    Some(4),
                    Some("2026-05-14T15:39:43Z"),
                ),
                make_turn(
                    "56c9b0c09525",
                    "<user_query>\nis it ok now\n</user_query>",
                    "claude-code",
                    "2026-05-14T15:40:26Z",
                    Some(7),
                    Some("2026-05-14T15:40:33Z"),
                ),
                make_turn(
                    "c0ed66e5e4af",
                    "<user_query>\nthanks\n</user_query>",
                    "claude-code",
                    "2026-05-14T15:40:53Z",
                    Some(9),
                    Some("2026-05-14T15:40:56Z"),
                ),
                make_turn(
                    "9f37164ed13a",
                    "thanks",
                    "cursor",
                    "2026-05-14T15:40:53Z",
                    None,
                    None,
                ),
                make_turn(
                    "32440f6813dc",
                    "<user_query>\nREVERT\n</user_query>",
                    "claude-code",
                    "2026-05-14T16:09:33Z",
                    Some(12),
                    Some("2026-05-14T16:09:35Z"),
                ),
            ],
            raw_events: Vec::new(),
        };

        let deduped = dedupe_phantom_turns(detail);
        let kept_ids: Vec<&str> = deduped
            .turns
            .iter()
            .map(|t| t.turn.turn_id.as_str())
            .collect();
        assert_eq!(
            kept_ids,
            vec![
                "a0201f30a07b",
                "56c9b0c09525",
                "c0ed66e5e4af",
                "32440f6813dc",
            ],
            "expected the orphan cursor row 9f37164ed13a to be dropped"
        );
    }

    #[test]
    fn dedupe_phantom_turns_preserves_lone_orphan_with_no_complete_sibling() {
        // An in-flight turn at the tail of a session has no offsets and no
        // ended_at, but it should NOT be hidden — the user is still typing.
        let detail = InteractionSessionDetail {
            summary: make_summary(),
            turns: vec![
                make_turn(
                    "complete-1",
                    "<user_query>first prompt</user_query>",
                    "cursor",
                    "2026-05-14T10:00:00Z",
                    Some(4),
                    Some("2026-05-14T10:00:10Z"),
                ),
                make_turn(
                    "in-flight",
                    "second prompt",
                    "cursor",
                    "2026-05-14T10:01:00Z",
                    None,
                    None,
                ),
            ],
            raw_events: Vec::new(),
        };

        let before = detail.turns.len();
        let deduped = dedupe_phantom_turns(detail);
        assert_eq!(deduped.turns.len(), before, "no sibling means keep both");
    }

    #[test]
    fn dedupe_phantom_turns_is_noop_when_all_prompts_unique() {
        let detail = InteractionSessionDetail {
            summary: make_summary(),
            turns: vec![
                make_turn(
                    "t1",
                    "first",
                    "cursor",
                    "2026-05-14T10:00:00Z",
                    Some(2),
                    Some("2026-05-14T10:00:05Z"),
                ),
                make_turn(
                    "t2",
                    "second",
                    "cursor",
                    "2026-05-14T10:01:00Z",
                    Some(5),
                    Some("2026-05-14T10:01:10Z"),
                ),
            ],
            raw_events: Vec::new(),
        };
        let deduped = dedupe_phantom_turns(detail);
        assert_eq!(deduped.turns.len(), 2);
    }
}
