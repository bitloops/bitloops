use std::collections::{HashMap, HashSet};

use async_graphql::{Request as GraphqlRequest, Variables};
use chrono::{DateTime, TimeZone, Utc};
use serde::Deserialize;
use serde_json::json;

use crate::api::dashboard_types::{
    DashboardBranchSummary, DashboardCheckpoint, DashboardCommit, DashboardCommitFileDiff,
    DashboardCommitRow, DashboardKpis, DashboardTokenUsage,
};
use crate::api::{
    ApiError, CommitCheckpointQuery, DashboardState, canonical_agent_key, dashboard_user,
    user_matches_filter,
};

use super::repository::dashboard_graphql_context;

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
    pub(super) checkpoints: DashboardGraphqlCheckpointConnection,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct DashboardGraphqlCheckpointConnection {
    pub(super) edges: Vec<DashboardGraphqlCheckpointEdge>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct DashboardGraphqlCheckpointEdge {
    pub(super) node: DashboardGraphqlCheckpointNode,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct DashboardGraphqlCheckpointNode {
    pub(super) id: String,
    pub(super) branch: Option<String>,
    pub(super) agent: Option<String>,
    pub(super) strategy: Option<String>,
    pub(super) files_touched: Vec<String>,
    pub(super) file_relations: Vec<DashboardGraphqlCheckpointFileRelation>,
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
pub(super) struct DashboardGraphqlCheckpointFileRelation {
    filepath: String,
    change_kind: String,
    copied_from_path: Option<String>,
    copied_from_blob_sha: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct DashboardGraphqlTokenUsage {
    pub(super) input_tokens: u64,
    pub(super) output_tokens: u64,
    pub(super) cache_creation_tokens: u64,
    pub(super) cache_read_tokens: u64,
    pub(super) api_call_count: u64,
}

#[derive(Debug, Clone)]
pub(super) struct DashboardGraphqlCommitRow {
    pub(super) commit: DashboardGraphqlCommitNode,
    pub(super) checkpoints: Vec<DashboardGraphqlCheckpointNode>,
}

pub(super) async fn load_dashboard_branches_via_graphql(
    state: &DashboardState,
    repo_selector: &str,
    from_unix: Option<i64>,
    to_unix: Option<i64>,
) -> std::result::Result<Vec<DashboardBranchSummary>, ApiError> {
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
            "repo": repo_selector,
            "since": optional_rfc3339_from_unix_seconds(from_unix),
            "until": optional_rfc3339_from_unix_seconds(to_unix),
        }),
    )
    .await?;

    Ok(data
        .repo
        .branches
        .into_iter()
        .map(|branch| DashboardBranchSummary {
            branch: branch.name,
            checkpoint_commits: branch.checkpoint_count,
        })
        .collect())
}

pub(super) async fn load_dashboard_commit_rows_via_graphql(
    state: &DashboardState,
    repo_selector: &str,
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
                  checkpoints(first: 5000) {
                    edges {
                      node {
                        id
                        branch
                        agent
                        strategy
                        filesTouched
                        fileRelations {
                          filepath
                          changeKind
                          copiedFromPath
                          copiedFromBlobSha
                        }
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
            "repo": repo_selector,
            "branch": filter.branch.as_str(),
            "since": optional_rfc3339_from_unix_seconds(filter.from_unix),
            "until": optional_rfc3339_from_unix_seconds(filter.to_unix),
        }),
    )
    .await?;

    let mut rows = Vec::new();
    for edge in data.repo.commits.edges {
        let mut commit = edge.node;
        let checkpoints = std::mem::take(&mut commit.checkpoints.edges)
            .into_iter()
            .map(|edge| edge.node)
            .filter(|checkpoint| {
                graphql_checkpoint_matches_agent_filter(checkpoint, filter.agent.as_deref())
            })
            .collect::<Vec<_>>();
        if checkpoints.is_empty() {
            continue;
        }

        let user = dashboard_user(&commit.author_name, &commit.author_email);
        if !user_matches_filter(&user, filter.user.as_deref()) {
            continue;
        }

        rows.push(DashboardGraphqlCommitRow {
            commit,
            checkpoints,
        });
    }

    Ok(rows)
}

async fn execute_dashboard_graphql<T: for<'de> Deserialize<'de>>(
    state: &DashboardState,
    query: &str,
    variables: serde_json::Value,
) -> std::result::Result<T, ApiError> {
    let response = state
        .devql_schema()
        .execute(
            GraphqlRequest::new(query)
                .variables(Variables::from_json(variables))
                .data(dashboard_graphql_context(state)),
        )
        .await;

    if let Some(error) = response.errors.first() {
        let code = dashboard_graphql_error_code(error);
        if !matches!(code, Some("BAD_USER_INPUT" | "BAD_CURSOR")) {
            log::error!("dashboard GraphQL request failed: {}", error.message);
        }
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
    let code = dashboard_graphql_error_code(error);
    match code {
        Some("BAD_USER_INPUT") | Some("BAD_CURSOR") => ApiError::bad_request(error.message.clone()),
        _ => ApiError::internal(format!(
            "dashboard GraphQL wrapper failed: {}",
            error.message
        )),
    }
}

fn dashboard_graphql_error_code(error: &async_graphql::ServerError) -> Option<&str> {
    error
        .extensions
        .as_ref()
        .and_then(|extensions| extensions.get("code"))
        .and_then(|value| match value {
            async_graphql::Value::String(value) => Some(value.as_str()),
            _ => None,
        })
}

fn optional_rfc3339_from_unix_seconds(value: Option<i64>) -> Option<String> {
    value.and_then(|timestamp| {
        Utc.timestamp_opt(timestamp, 0)
            .single()
            .map(|value| value.to_rfc3339())
    })
}

pub(super) fn build_dashboard_kpis_from_graphql_rows(
    rows: &[DashboardGraphqlCommitRow],
) -> DashboardKpis {
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
        for checkpoint in &row.checkpoints {
            if !unique_checkpoint_ids.insert(checkpoint.id.clone()) {
                continue;
            }

            for agent_key in checkpoint_agents(checkpoint) {
                unique_agents.insert(agent_key);
            }
            total_sessions += checkpoint.session_count;
            for file in &checkpoint.files_touched {
                files_touched.insert(file.clone());
            }

            if let Some(token_usage) = checkpoint.token_usage.as_ref() {
                input_tokens += token_usage.input_tokens;
                output_tokens += token_usage.output_tokens;
                cache_creation_tokens += token_usage.cache_creation_tokens;
                cache_read_tokens += token_usage.cache_read_tokens;
                api_call_count += token_usage.api_call_count;
            }
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

    DashboardKpis {
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

fn dashboard_token_usage_from_graphql(
    usage: Option<&DashboardGraphqlTokenUsage>,
) -> Option<DashboardTokenUsage> {
    usage.map(|usage| DashboardTokenUsage {
        input_tokens: usage.input_tokens,
        output_tokens: usage.output_tokens,
        cache_creation_tokens: usage.cache_creation_tokens,
        cache_read_tokens: usage.cache_read_tokens,
        api_call_count: usage.api_call_count,
    })
}

pub(super) fn dashboard_commit_row_from_graphql(
    row: DashboardGraphqlCommitRow,
    files_touched: Vec<DashboardCommitFileDiff>,
) -> DashboardCommitRow {
    let DashboardGraphqlCommitRow {
        commit,
        checkpoints,
    } = row;
    let timestamp = DateTime::parse_from_rfc3339(&commit.committed_at)
        .map(|value| value.timestamp())
        .unwrap_or(0);
    let checkpoints = checkpoints
        .into_iter()
        .map(|checkpoint| {
            let checkpoint_files_touched =
                checkpoint_file_diffs_from_graphql(&checkpoint, &files_touched);
            let agents = checkpoint_agents(&checkpoint);
            let token_usage = dashboard_token_usage_from_graphql(checkpoint.token_usage.as_ref());

            DashboardCheckpoint {
                checkpoint_id: checkpoint.id,
                strategy: checkpoint.strategy.unwrap_or_default(),
                branch: checkpoint.branch.unwrap_or_default(),
                checkpoints_count: checkpoint.checkpoints_count.try_into().unwrap_or(u32::MAX),
                files_touched: checkpoint_files_touched,
                session_count: checkpoint.session_count,
                token_usage,
                session_id: checkpoint.session_id,
                agents,
                first_prompt_preview: checkpoint.first_prompt_preview.unwrap_or_default(),
                created_at: checkpoint.created_at.unwrap_or_default(),
                is_task: checkpoint.is_task,
                tool_use_id: checkpoint.tool_use_id.unwrap_or_default(),
            }
        })
        .collect::<Vec<_>>();
    let checkpoint = checkpoints
        .first()
        .cloned()
        .expect("dashboard commit rows always include at least one checkpoint");

    DashboardCommitRow {
        commit: DashboardCommit {
            sha: commit.sha,
            parents: commit.parents,
            author_name: commit.author_name,
            author_email: commit.author_email,
            timestamp,
            message: commit.commit_message,
            files_touched,
        },
        checkpoint,
        checkpoints,
    }
}

fn checkpoint_file_diffs_from_graphql(
    checkpoint: &DashboardGraphqlCheckpointNode,
    commit_file_diffs: &[DashboardCommitFileDiff],
) -> Vec<DashboardCommitFileDiff> {
    if checkpoint.file_relations.is_empty() {
        return commit_file_diffs.to_vec();
    }

    let counts_by_path = commit_file_diffs
        .iter()
        .map(|diff| {
            (
                diff.filepath.clone(),
                (diff.additions_count, diff.deletions_count),
            )
        })
        .collect::<HashMap<_, _>>();
    let mut files_touched = checkpoint
        .file_relations
        .iter()
        .map(|relation| {
            let (additions_count, deletions_count) = counts_by_path
                .get(&relation.filepath)
                .copied()
                .unwrap_or((0, 0));
            DashboardCommitFileDiff {
                filepath: relation.filepath.clone(),
                additions_count,
                deletions_count,
                change_kind: Some(relation.change_kind.clone()),
                copied_from_path: relation.copied_from_path.clone(),
                copied_from_blob_sha: relation.copied_from_blob_sha.clone(),
            }
        })
        .collect::<Vec<_>>();
    files_touched.sort_by(|left, right| left.filepath.cmp(&right.filepath));
    files_touched
}
