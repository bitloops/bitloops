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
use crate::host::interactions::transcript_entry::{TranscriptActor, TranscriptEntry, TranscriptSource};
use crate::host::interactions::types::InteractionTurn;
use crate::host::interactions::{
    derive_session_transcript_entries, derive_turn_transcript_entries,
    partition_session_entries_to_turns, read_session_transcript_text,
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

/// Convert a domain `InteractionSessionDetail` into the dashboard response,
/// enriching it with canonical transcript entries derived from the session's
/// agent. Reads the transcript file from disk **once** and slices it per turn
/// using each turn's `transcript_offset_start`/`end` markers — call from
/// inside `spawn_blocking`.
fn enrich_session_detail_with_transcript_entries(
    detail: query::InteractionSessionDetail,
) -> DashboardInteractionSessionDetail {
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
        let turns_refs: Vec<&InteractionTurn> =
            detail.turns.iter().map(|s| &s.turn).collect();
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
        .map(|entries| entries.into_iter().map(DashboardTranscriptEntry::from).collect())
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
