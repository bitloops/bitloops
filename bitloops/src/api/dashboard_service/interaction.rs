use chrono::{DateTime, SecondsFormat, Utc};
use tokio::task;

use crate::api::dashboard_types::{
    DashboardInteractionActorBucket, DashboardInteractionAgentBucket,
    DashboardInteractionCommitAuthorBucket, DashboardInteractionFilterInput,
    DashboardInteractionKpis, DashboardInteractionSearchInput, DashboardInteractionSession,
    DashboardInteractionSessionDetail, DashboardInteractionSessionSearchHit,
    DashboardInteractionTurnSearchHit,
};
use crate::api::{API_DEFAULT_PAGE_LIMIT, ApiError, ApiPage, DashboardState, paginate};
use crate::host::interactions::query;

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
    let detail = task::spawn_blocking(move || query::load_session_detail(&repo_root, &session_id))
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
    let Some(detail) = detail else {
        return Err(ApiError::not_found("unknown interaction session"));
    };
    Ok(DashboardInteractionSessionDetail::from_domain(&detail))
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
