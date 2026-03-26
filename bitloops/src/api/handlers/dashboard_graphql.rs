use std::collections::HashSet;

use async_graphql::{Request as GraphqlRequest, Variables};
use chrono::{DateTime, TimeZone, Utc};
use serde::{Deserialize, de::DeserializeOwned};
use serde_json::json;

use super::super::dto::{
    ApiBranchSummaryDto, ApiCommitDto, ApiCommitFileDiffDto, ApiCommitRowDto, ApiError,
    ApiKpisResponse, ApiTokenUsageDto,
};
use super::super::{CommitCheckpointQuery, DashboardState, canonical_agent_key};

const DASHBOARD_GRAPHQL_REPO_NAME: &str = "dashboard";

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DashboardGraphqlBranchesData {
    repo: DashboardGraphqlBranchesRepo,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DashboardGraphqlBranchesRepo {
    branches: Vec<DashboardGraphqlBranch>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DashboardGraphqlBranch {
    name: String,
    checkpoint_count: usize,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DashboardGraphqlCommitsData {
    repo: DashboardGraphqlCommitsRepo,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DashboardGraphqlCommitsRepo {
    commits: DashboardGraphqlCommitConnection,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DashboardGraphqlCommitConnection {
    edges: Vec<DashboardGraphqlCommitEdge>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DashboardGraphqlCommitEdge {
    node: DashboardGraphqlCommitNode,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct DashboardGraphqlCommitNode {
    pub(super) sha: String,
    pub(super) parents: Vec<String>,
    pub(super) author_name: String,
    pub(super) author_email: String,
    pub(super) committed_at: String,
    pub(super) commit_message: String,
    checkpoints: DashboardGraphqlCheckpointConnection,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DashboardGraphqlCheckpointConnection {
    edges: Vec<DashboardGraphqlCheckpointEdge>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DashboardGraphqlCheckpointEdge {
    node: DashboardGraphqlCheckpointNode,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct DashboardGraphqlCheckpointNode {
    pub(super) id: String,
    pub(super) branch: Option<String>,
    pub(super) agent: Option<String>,
    pub(super) strategy: Option<String>,
    pub(super) files_touched: Vec<String>,
    pub(super) checkpoints_count: usize,
    pub(super) session_count: usize,
    pub(super) token_usage: Option<DashboardGraphqlTokenUsage>,
    pub(super) session_id: String,
    pub(super) agents: Vec<String>,
    pub(super) first_prompt_preview: Option<String>,
    pub(super) created_at: Option<String>,
    pub(super) is_task: bool,
    pub(super) tool_use_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct DashboardGraphqlTokenUsage {
    input_tokens: u64,
    output_tokens: u64,
    cache_creation_tokens: u64,
    cache_read_tokens: u64,
    api_call_count: u64,
}

#[derive(Debug, Clone)]
pub(super) struct DashboardGraphqlCommitRow {
    pub(super) commit: DashboardGraphqlCommitNode,
    pub(super) checkpoint: DashboardGraphqlCheckpointNode,
}

pub(super) async fn load_dashboard_branches_via_graphql(
    state: &DashboardState,
    from_unix: Option<i64>,
    to_unix: Option<i64>,
) -> std::result::Result<Vec<ApiBranchSummaryDto>, ApiError> {
    let data: DashboardGraphqlBranchesData = execute_dashboard_graphql(
        state,
        r#"
        query DashboardBranches($repo: String!, $since: DateTime, $until: DateTime) {
          repo(name: $repo) {
            branches(since: $since, until: $until) {
              name
              checkpointCount
            }
          }
        }
        "#,
        json!({
            "repo": DASHBOARD_GRAPHQL_REPO_NAME,
            "since": optional_rfc3339_from_unix_seconds(from_unix),
            "until": optional_rfc3339_from_unix_seconds(to_unix),
        }),
    )
    .await?;

    Ok(data
        .repo
        .branches
        .into_iter()
        .map(|branch| ApiBranchSummaryDto {
            branch: branch.name,
            checkpoint_commits: branch.checkpoint_count,
        })
        .collect())
}

pub(super) async fn load_dashboard_commit_rows_via_graphql(
    state: &DashboardState,
    filter: &CommitCheckpointQuery,
) -> std::result::Result<Vec<DashboardGraphqlCommitRow>, ApiError> {
    let data: DashboardGraphqlCommitsData = execute_dashboard_graphql(
        state,
        r#"
        query DashboardCommits($repo: String!, $branch: String!, $since: DateTime, $until: DateTime) {
          repo(name: $repo) {
            commits(branch: $branch, since: $since, until: $until, first: 5000) {
              edges {
                node {
                  sha
                  parents
                  authorName
                  authorEmail
                  committedAt
                  commitMessage
                  checkpoints(first: 1) {
                    edges {
                      node {
                        id
                        branch
                        agent
                        strategy
                        filesTouched
                        checkpointsCount
                        sessionCount
                        tokenUsage {
                          inputTokens
                          outputTokens
                          cacheCreationTokens
                          cacheReadTokens
                          apiCallCount
                        }
                        sessionId
                        agents
                        firstPromptPreview
                        createdAt
                        isTask
                        toolUseId
                      }
                    }
                  }
                }
              }
            }
          }
        }
        "#,
        json!({
            "repo": DASHBOARD_GRAPHQL_REPO_NAME,
            "branch": filter.branch.as_str(),
            "since": optional_rfc3339_from_unix_seconds(filter.from_unix),
            "until": optional_rfc3339_from_unix_seconds(filter.to_unix),
        }),
    )
    .await?;

    let mut rows = Vec::new();
    for edge in data.repo.commits.edges {
        let commit = edge.node;
        let Some(checkpoint) = commit
            .checkpoints
            .edges
            .first()
            .cloned()
            .map(|edge| edge.node)
        else {
            continue;
        };

        let user = super::super::dashboard_user(&commit.author_name, &commit.author_email);
        if !super::super::user_matches_filter(&user, filter.user.as_deref()) {
            continue;
        }
        if !graphql_checkpoint_matches_agent_filter(&checkpoint, filter.agent.as_deref()) {
            continue;
        }

        rows.push(DashboardGraphqlCommitRow { commit, checkpoint });
    }

    Ok(rows)
}

async fn execute_dashboard_graphql<T: DeserializeOwned>(
    state: &DashboardState,
    query: &str,
    variables: serde_json::Value,
) -> std::result::Result<T, ApiError> {
    let response = state
        .devql_schema()
        .execute(GraphqlRequest::new(query).variables(Variables::from_json(variables)))
        .await;

    if let Some(error) = response.errors.first() {
        return Err(map_dashboard_graphql_error(error));
    }

    let data = response.data.into_json().map_err(|err| {
        ApiError::internal(format!("failed to decode dashboard GraphQL data: {err:#}"))
    })?;

    serde_json::from_value(data).map_err(|err| {
        ApiError::internal(format!(
            "failed to decode dashboard GraphQL response payload: {err}"
        ))
    })
}

fn map_dashboard_graphql_error(error: &async_graphql::ServerError) -> ApiError {
    let code = error
        .extensions
        .as_ref()
        .and_then(|extensions| extensions.get("code"))
        .and_then(|value| match value {
            async_graphql::Value::String(value) => Some(value.as_str()),
            _ => None,
        });
    match code {
        Some("BAD_USER_INPUT") | Some("BAD_CURSOR") => ApiError::bad_request(error.message.clone()),
        _ => ApiError::internal(format!(
            "dashboard GraphQL wrapper failed: {}",
            error.message
        )),
    }
}

fn optional_rfc3339_from_unix_seconds(value: Option<i64>) -> Option<String> {
    value.and_then(|timestamp| {
        Utc.timestamp_opt(timestamp, 0)
            .single()
            .map(|value| value.to_rfc3339())
    })
}

pub(super) fn build_kpis_response_from_graphql_rows(
    rows: &[DashboardGraphqlCommitRow],
) -> ApiKpisResponse {
    let mut unique_checkpoint_ids: HashSet<String> = HashSet::new();
    let mut unique_agents: HashSet<String> = HashSet::new();
    let mut total_sessions = 0usize;
    let mut files_touched: HashSet<String> = HashSet::new();
    let mut input_tokens = 0u64;
    let mut output_tokens = 0u64;
    let mut cache_creation_tokens = 0u64;
    let mut cache_read_tokens = 0u64;
    let mut api_call_count = 0u64;

    for row in rows {
        if !unique_checkpoint_ids.insert(row.checkpoint.id.clone()) {
            continue;
        }

        for agent_key in checkpoint_agents(&row.checkpoint) {
            unique_agents.insert(agent_key);
        }
        total_sessions += row.checkpoint.session_count;
        for file in &row.checkpoint.files_touched {
            files_touched.insert(file.clone());
        }

        if let Some(token_usage) = row.checkpoint.token_usage.as_ref() {
            input_tokens += token_usage.input_tokens;
            output_tokens += token_usage.output_tokens;
            cache_creation_tokens += token_usage.cache_creation_tokens;
            cache_read_tokens += token_usage.cache_read_tokens;
            api_call_count += token_usage.api_call_count;
        }
    }

    let total_checkpoints = unique_checkpoint_ids.len();
    let total_tokens = input_tokens + output_tokens + cache_creation_tokens + cache_read_tokens;
    let average_tokens_per_checkpoint = if total_checkpoints == 0 {
        0.0
    } else {
        total_tokens as f64 / total_checkpoints as f64
    };
    let average_sessions_per_checkpoint = if total_checkpoints == 0 {
        0.0
    } else {
        total_sessions as f64 / total_checkpoints as f64
    };

    ApiKpisResponse {
        total_commits: rows.len(),
        total_checkpoints,
        total_agents: unique_agents.len(),
        total_sessions,
        files_touched_count: files_touched.len(),
        input_tokens,
        output_tokens,
        cache_creation_tokens,
        cache_read_tokens,
        api_call_count,
        average_tokens_per_checkpoint,
        average_sessions_per_checkpoint,
    }
}

fn graphql_checkpoint_matches_agent_filter(
    checkpoint: &DashboardGraphqlCheckpointNode,
    agent_filter: Option<&str>,
) -> bool {
    let Some(filter) = agent_filter else {
        return true;
    };

    let normalized = canonical_agent_key(filter);
    if normalized.is_empty() {
        return true;
    }

    checkpoint_agents(checkpoint)
        .into_iter()
        .any(|agent| agent == normalized)
}

pub(super) fn checkpoint_agents(info: &DashboardGraphqlCheckpointNode) -> Vec<String> {
    let mut keys: Vec<String> = Vec::new();

    if info.agents.is_empty() {
        let key = canonical_agent_key(info.agent.as_deref().unwrap_or_default());
        if !key.is_empty() {
            keys.push(key);
        }
        return keys;
    }

    for agent in &info.agents {
        let key = canonical_agent_key(agent);
        if key.is_empty() || keys.iter().any(|existing| existing == &key) {
            continue;
        }
        keys.push(key);
    }
    keys
}

fn api_token_usage_from_graphql(
    usage: Option<&DashboardGraphqlTokenUsage>,
) -> Option<ApiTokenUsageDto> {
    usage.map(|usage| ApiTokenUsageDto {
        input_tokens: usage.input_tokens,
        output_tokens: usage.output_tokens,
        cache_creation_tokens: usage.cache_creation_tokens,
        cache_read_tokens: usage.cache_read_tokens,
        api_call_count: usage.api_call_count,
    })
}

pub(super) fn api_commit_row_from_graphql(
    row: DashboardGraphqlCommitRow,
    files_touched: Vec<ApiCommitFileDiffDto>,
) -> ApiCommitRowDto {
    let DashboardGraphqlCommitRow { commit, checkpoint } = row;
    let agents = checkpoint_agents(&checkpoint);
    let checkpoint_files_touched = files_touched.clone();
    let timestamp = DateTime::parse_from_rfc3339(&commit.committed_at)
        .map(|value| value.timestamp())
        .unwrap_or(0);
    ApiCommitRowDto {
        commit: ApiCommitDto {
            sha: commit.sha,
            parents: commit.parents,
            author_name: commit.author_name,
            author_email: commit.author_email,
            timestamp,
            message: commit.commit_message,
            files_touched,
        },
        checkpoint: super::super::dto::ApiCheckpointDto {
            checkpoint_id: checkpoint.id,
            strategy: checkpoint.strategy.unwrap_or_default(),
            branch: checkpoint.branch.unwrap_or_default(),
            checkpoints_count: checkpoint.checkpoints_count.try_into().unwrap_or(u32::MAX),
            files_touched: checkpoint_files_touched,
            session_count: checkpoint.session_count,
            token_usage: api_token_usage_from_graphql(checkpoint.token_usage.as_ref()),
            session_id: checkpoint.session_id,
            agents,
            first_prompt_preview: checkpoint.first_prompt_preview.unwrap_or_default(),
            created_at: checkpoint.created_at.unwrap_or_default(),
            is_task: checkpoint.is_task,
            tool_use_id: checkpoint.tool_use_id.unwrap_or_default(),
        },
    }
}
