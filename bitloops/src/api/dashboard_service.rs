use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::Instant;

use async_graphql::{Request as GraphqlRequest, Variables};
use chrono::{DateTime, TimeZone, Utc};
use serde::Deserialize;
use serde_json::json;

use super::bundle_types::BundleError;
use super::dashboard_file_diffs::{
    dashboard_checkpoint_file_diff_list_from_relations, dashboard_file_diff_list_from_numstat,
    dashboard_zeroed_file_diff_list,
};
use super::dashboard_params::{
    normalize_checkpoint_id, parse_commit_checkpoint_filter, parse_optional_unix_seconds,
    validate_time_window,
};
use super::dashboard_types::{
    DashboardAgent, DashboardBranchSummary, DashboardBundleVersion, DashboardCheckpoint,
    DashboardCheckpointDetail, DashboardCheckpointSessionDetail, DashboardCommit,
    DashboardCommitFileDiff, DashboardCommitRow, DashboardFetchBundleResult, DashboardKpis,
    DashboardRepository, DashboardTokenUsage, DashboardUser,
};
use super::handlers::{map_resolve_repository_error, resolve_repo_root_from_repo_id};
use super::{
    ApiError, CommitCheckpointQuery, DashboardState, canonical_agent_key, dashboard_user, paginate,
    read_checkpoint_info_for_filtering, read_commit_numstat, user_matches_filter,
    walk_branch_commits_with_checkpoints,
};
use crate::graphql::HealthStatus;
use crate::host::checkpoints::strategy::manual_commit::{CommittedInfo, read_session_content};
use crate::host::devql::checkpoint_provenance::{
    CheckpointFileGateway, CheckpointFileProvenanceDetailRow,
};

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
struct DashboardGraphqlCommitNode {
    sha: String,
    parents: Vec<String>,
    author_name: String,
    author_email: String,
    committed_at: String,
    commit_message: String,
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
struct DashboardGraphqlCheckpointNode {
    id: String,
    branch: Option<String>,
    agent: Option<String>,
    strategy: Option<String>,
    files_touched: Vec<String>,
    file_relations: Vec<DashboardGraphqlCheckpointFileRelation>,
    checkpoints_count: usize,
    session_count: usize,
    token_usage: Option<DashboardGraphqlTokenUsage>,
    session_id: String,
    agents: Vec<String>,
    first_prompt_preview: Option<String>,
    created_at: Option<String>,
    is_task: bool,
    tool_use_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DashboardGraphqlCheckpointFileRelation {
    filepath: String,
    change_kind: String,
    copied_from_path: Option<String>,
    copied_from_blob_sha: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DashboardGraphqlTokenUsage {
    input_tokens: u64,
    output_tokens: u64,
    cache_creation_tokens: u64,
    cache_read_tokens: u64,
    api_call_count: u64,
}

#[derive(Debug, Clone)]
struct DashboardGraphqlCommitRow {
    commit: DashboardGraphqlCommitNode,
    checkpoint: DashboardGraphqlCheckpointNode,
}

pub(super) async fn load_dashboard_health(state: &DashboardState) -> HealthStatus {
    crate::graphql::DevqlGraphqlContext::for_global_request(
        state.config_root.clone(),
        state.repo_root.clone(),
        state.repo_registry_path().map(std::path::Path::to_path_buf),
        state.db.clone(),
    )
    .health_status()
    .await
}

pub(super) async fn load_dashboard_repositories(
    state: &DashboardState,
) -> std::result::Result<Vec<DashboardRepository>, ApiError> {
    let context = crate::graphql::DevqlGraphqlContext::for_global_request(
        state.config_root.clone(),
        state.repo_root.clone(),
        state.repo_registry_path().map(std::path::Path::to_path_buf),
        state.db.clone(),
    );
    let repositories = context.list_known_repositories().await.map_err(|err| {
        ApiError::internal(format!("failed to load dashboard repositories: {err:#}"))
    })?;

    Ok(repositories
        .into_iter()
        .map(|repository| DashboardRepository {
            repo_id: repository.repo_id().to_string(),
            identity: repository.identity().to_string(),
            name: repository.name().to_string(),
            provider: repository.provider().to_string(),
            organization: repository.organization().to_string(),
            default_branch: repository.default_branch().map(str::to_string),
        })
        .collect())
}

pub(super) async fn load_dashboard_kpis(
    state: &DashboardState,
    repo_id: Option<String>,
    filter: CommitCheckpointQuery,
) -> std::result::Result<DashboardKpis, ApiError> {
    let repo_selector = resolve_dashboard_repo_selector(state, repo_id.as_deref()).await?;
    let rows =
        load_dashboard_commit_rows_via_graphql(state, repo_selector.as_str(), &filter).await?;
    Ok(build_dashboard_kpis_from_graphql_rows(&rows))
}

pub(super) async fn load_dashboard_commits(
    state: &DashboardState,
    repo_id: Option<String>,
    filter: CommitCheckpointQuery,
) -> std::result::Result<Vec<DashboardCommitRow>, ApiError> {
    let repo_selector = resolve_dashboard_repo_selector(state, repo_id.as_deref()).await?;
    let repo_root = resolve_dashboard_repo_root(state, repo_id.as_deref()).await?;

    let rows =
        load_dashboard_commit_rows_via_graphql(state, repo_selector.as_str(), &filter).await?;
    let rows = paginate(&rows, filter.page);
    let mut result = Vec::with_capacity(rows.len());
    for row in rows {
        let files_touched = match read_commit_numstat(&repo_root, &row.commit.sha) {
            Ok(stats) => dashboard_file_diff_list_from_numstat(stats),
            Err(err) => {
                log::warn!(
                    "dashboard commits query: failed to read numstat for {}: {:#}",
                    row.commit.sha,
                    err
                );
                dashboard_zeroed_file_diff_list(&row.checkpoint.files_touched)
            }
        };
        result.push(dashboard_commit_row_from_graphql(row, files_touched));
    }
    Ok(result)
}

pub(super) async fn load_dashboard_branches(
    state: &DashboardState,
    repo_id: Option<String>,
    from: Option<String>,
    to: Option<String>,
) -> std::result::Result<Vec<DashboardBranchSummary>, ApiError> {
    let repo_selector = resolve_dashboard_repo_selector(state, repo_id.as_deref()).await?;
    let from_unix = parse_optional_unix_seconds("from", from)?;
    let to_unix = parse_optional_unix_seconds("to", to)?;
    validate_time_window(from_unix, to_unix)?;
    load_dashboard_branches_via_graphql(state, repo_selector.as_str(), from_unix, to_unix).await
}

pub(super) async fn load_dashboard_users(
    state: &DashboardState,
    repo_id: Option<String>,
    branch: String,
    from: Option<String>,
    to: Option<String>,
    agent: Option<String>,
) -> std::result::Result<Vec<DashboardUser>, ApiError> {
    let repo_selector = resolve_dashboard_repo_selector(state, repo_id.as_deref()).await?;
    let filter = parse_commit_checkpoint_filter(Some(branch), from, to, None, agent)?;
    let rows =
        load_dashboard_commit_rows_via_graphql(state, repo_selector.as_str(), &filter).await?;

    let mut users_by_key: HashMap<String, DashboardUser> = HashMap::new();
    for row in rows {
        let user = dashboard_user(&row.commit.author_name, &row.commit.author_email);
        let key = user.key;
        let name = user.name;
        let email = user.email;

        let entry = users_by_key.entry(key.clone()).or_insert(DashboardUser {
            key,
            name: String::new(),
            email: String::new(),
        });
        if entry.name.is_empty() && !name.is_empty() {
            entry.name = name;
        }
        if entry.email.is_empty() && !email.is_empty() {
            entry.email = email;
        }
    }

    let mut users = users_by_key.into_values().collect::<Vec<_>>();
    users.sort_by(|left, right| {
        left.key
            .cmp(&right.key)
            .then_with(|| left.name.cmp(&right.name))
            .then_with(|| left.email.cmp(&right.email))
    });
    Ok(users)
}

pub(super) async fn load_dashboard_agents(
    state: &DashboardState,
    repo_id: Option<String>,
    branch: String,
    from: Option<String>,
    to: Option<String>,
    user: Option<String>,
) -> std::result::Result<Vec<DashboardAgent>, ApiError> {
    let repo_selector = resolve_dashboard_repo_selector(state, repo_id.as_deref()).await?;
    let filter = parse_commit_checkpoint_filter(Some(branch), from, to, user, None)?;
    let rows =
        load_dashboard_commit_rows_via_graphql(state, repo_selector.as_str(), &filter).await?;

    let mut agents: Vec<DashboardAgent> = Vec::new();
    for row in rows {
        for key in checkpoint_agents(&row.checkpoint) {
            agents.push(DashboardAgent { key });
        }
    }
    agents.sort_by(|left, right| left.key.cmp(&right.key));
    agents.dedup_by(|left, right| left.key == right.key);
    Ok(agents)
}

pub(super) async fn load_dashboard_checkpoint(
    state: &DashboardState,
    repo_id: Option<String>,
    checkpoint_id: String,
) -> std::result::Result<DashboardCheckpointDetail, ApiError> {
    let repo_selector = resolve_dashboard_repo_selector(state, repo_id.as_deref()).await?;
    let repo_root = resolve_dashboard_repo_root(state, repo_id.as_deref()).await?;
    load_checkpoint_detail(&repo_root, repo_selector.as_str(), checkpoint_id).await
}

pub(super) async fn check_dashboard_bundle_version(
    state: &DashboardState,
) -> std::result::Result<DashboardBundleVersion, ApiError> {
    let started = Instant::now();
    let bundle_dir = state.bundle_dir.display().to_string();
    log::info!(
        "event=dashboard.bundle.check.started operation=checkBundleVersion status=started bundle_dir={bundle_dir}"
    );

    let response = match super::bundle::check_bundle_version(state).await {
        Ok(result) => {
            let latest_applicable_version = result
                .latest_applicable_version
                .clone()
                .unwrap_or_else(|| "null".to_string());
            log::info!(
                "event=dashboard.bundle.check.succeeded operation=checkBundleVersion status=succeeded bundle_dir={} install_available={} latest_applicable_version={}",
                bundle_dir,
                result.install_available,
                latest_applicable_version
            );
            Ok(DashboardBundleVersion {
                current_version: result.current_version,
                latest_applicable_version: result.latest_applicable_version,
                install_available: result.install_available,
                reason: result.reason.as_str().to_string(),
            })
        }
        Err(error) => {
            let error_code = bundle_error_code(&error);
            let api_error = map_bundle_error(error);
            log::error!(
                "event=dashboard.bundle.check.failed operation=checkBundleVersion status=failed bundle_dir={} error_code={} error_message={}",
                bundle_dir,
                error_code,
                api_error.message
            );
            Err(api_error)
        }
    };

    log::debug!(
        "dashboard checkBundleVersion completed in {}ms",
        started.elapsed().as_millis()
    );
    response
}

pub(super) async fn fetch_dashboard_bundle(
    state: &DashboardState,
) -> std::result::Result<DashboardFetchBundleResult, ApiError> {
    let started = Instant::now();
    let bundle_dir = state.bundle_dir.display().to_string();
    log::info!(
        "event=dashboard.bundle.install.started operation=fetchBundle status=started bundle_dir={bundle_dir}"
    );

    let response = match super::bundle::fetch_bundle(state).await {
        Ok(result) => {
            log::info!(
                "event=dashboard.bundle.install.succeeded operation=fetchBundle status=succeeded bundle_dir={} installed_version={} checksum_verified={}",
                result.bundle_dir,
                result.installed_version,
                result.checksum_verified
            );
            Ok(DashboardFetchBundleResult {
                installed_version: result.installed_version,
                bundle_dir: result.bundle_dir,
                status: result.status,
                checksum_verified: result.checksum_verified,
            })
        }
        Err(BundleError::ChecksumMismatch) => {
            log::warn!(
                "event=dashboard.bundle.install.checksum_mismatch operation=fetchBundle status=failed bundle_dir={} error_code=checksum_mismatch",
                bundle_dir
            );
            let api_error = map_bundle_error(BundleError::ChecksumMismatch);
            log::error!(
                "event=dashboard.bundle.install.failed operation=fetchBundle status=failed bundle_dir={} error_code=checksum_mismatch error_message={}",
                bundle_dir,
                api_error.message
            );
            Err(api_error)
        }
        Err(error) => {
            let error_code = bundle_error_code(&error);
            let api_error = map_bundle_error(error);
            log::error!(
                "event=dashboard.bundle.install.failed operation=fetchBundle status=failed bundle_dir={} error_code={} error_message={}",
                bundle_dir,
                error_code,
                api_error.message
            );
            Err(api_error)
        }
    };

    log::debug!(
        "dashboard fetchBundle completed in {}ms",
        started.elapsed().as_millis()
    );
    response
}

async fn resolve_dashboard_repo_selector(
    state: &DashboardState,
    repo_id: Option<&str>,
) -> std::result::Result<String, ApiError> {
    if let Some(repo_id) = repo_id.map(str::trim).filter(|repo_id| !repo_id.is_empty()) {
        let context = crate::graphql::DevqlGraphqlContext::for_global_request(
            state.config_root.clone(),
            state.repo_root.clone(),
            state.repo_registry_path().map(std::path::Path::to_path_buf),
            state.db.clone(),
        );
        let selection = context
            .resolve_repository_selection(repo_id)
            .await
            .map_err(|err| map_resolve_repository_error(repo_id, err))?;
        return Ok(selection.repo_id().to_string());
    }

    crate::host::devql::resolve_repo_identity(&state.repo_root)
        .map(|repo| repo.repo_id)
        .map_err(|err| {
            ApiError::internal(format!(
                "failed to resolve dashboard repository scope: {err:#}"
            ))
        })
}

async fn resolve_dashboard_repo_root(
    state: &DashboardState,
    repo_id: Option<&str>,
) -> std::result::Result<PathBuf, ApiError> {
    match repo_id.map(str::trim).filter(|repo_id| !repo_id.is_empty()) {
        Some(repo_id) => resolve_repo_root_from_repo_id(state, repo_id).await,
        None => Ok(state.repo_root.clone()),
    }
}

async fn load_dashboard_branches_via_graphql(
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

async fn load_dashboard_commit_rows_via_graphql(
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
                  checkpoints(first: 1) {
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

        let user = dashboard_user(&commit.author_name, &commit.author_email);
        if !user_matches_filter(&user, filter.user.as_deref()) {
            continue;
        }
        if !graphql_checkpoint_matches_agent_filter(&checkpoint, filter.agent.as_deref()) {
            continue;
        }

        rows.push(DashboardGraphqlCommitRow { commit, checkpoint });
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
                .data(crate::graphql::DevqlGraphqlContext::for_global_request(
                    state.config_root.clone(),
                    state.repo_root.clone(),
                    state.repo_registry_path().map(std::path::Path::to_path_buf),
                    state.db.clone(),
                )),
        )
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

fn build_dashboard_kpis_from_graphql_rows(rows: &[DashboardGraphqlCommitRow]) -> DashboardKpis {
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

fn checkpoint_agents(info: &DashboardGraphqlCheckpointNode) -> Vec<String> {
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

fn dashboard_commit_row_from_graphql(
    row: DashboardGraphqlCommitRow,
    files_touched: Vec<DashboardCommitFileDiff>,
) -> DashboardCommitRow {
    let DashboardGraphqlCommitRow { commit, checkpoint } = row;
    let agents = checkpoint_agents(&checkpoint);
    let checkpoint_files_touched = checkpoint_file_diffs_from_graphql(&checkpoint, &files_touched);
    let timestamp = DateTime::parse_from_rfc3339(&commit.committed_at)
        .map(|value| value.timestamp())
        .unwrap_or(0);

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
        checkpoint: DashboardCheckpoint {
            checkpoint_id: checkpoint.id,
            strategy: checkpoint.strategy.unwrap_or_default(),
            branch: checkpoint.branch.unwrap_or_default(),
            checkpoints_count: checkpoint.checkpoints_count.try_into().unwrap_or(u32::MAX),
            files_touched: checkpoint_files_touched,
            session_count: checkpoint.session_count,
            token_usage: dashboard_token_usage_from_graphql(checkpoint.token_usage.as_ref()),
            session_id: checkpoint.session_id,
            agents,
            first_prompt_preview: checkpoint.first_prompt_preview.unwrap_or_default(),
            created_at: checkpoint.created_at.unwrap_or_default(),
            is_task: checkpoint.is_task,
            tool_use_id: checkpoint.tool_use_id.unwrap_or_default(),
        },
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

fn dashboard_token_usage_from_committed(info: &CommittedInfo) -> Option<DashboardTokenUsage> {
    info.token_usage.as_ref().map(|usage| DashboardTokenUsage {
        input_tokens: usage.input_tokens,
        output_tokens: usage.output_tokens,
        cache_creation_tokens: usage.cache_creation_tokens,
        cache_read_tokens: usage.cache_read_tokens,
        api_call_count: usage.api_call_count,
    })
}

async fn load_checkpoint_detail(
    repo_root: &Path,
    repo_id: &str,
    checkpoint_id: String,
) -> std::result::Result<DashboardCheckpointDetail, ApiError> {
    let checkpoint_id = normalize_checkpoint_id(checkpoint_id)?;
    let Some(info) =
        read_checkpoint_info_for_filtering(repo_root, &checkpoint_id).map_err(|err| {
            ApiError::internal(format!(
                "failed to read checkpoint metadata for {checkpoint_id}: {err:#}"
            ))
        })?
    else {
        return Err(ApiError::not_found(format!(
            "checkpoint not found: {checkpoint_id}"
        )));
    };

    let checkpoint_file_relations =
        load_checkpoint_file_relations(repo_root, repo_id, &checkpoint_id)
            .await
            .map_err(|err| {
                ApiError::internal(format!(
                    "failed to read checkpoint file provenance for {checkpoint_id}: {err:#}"
                ))
            })?;

    let mut sessions = Vec::new();
    for session_index in 0..info.session_count {
        let content = match read_session_content(repo_root, &checkpoint_id, session_index) {
            Ok(content) => content,
            Err(err) => {
                log::warn!(
                    "dashboard checkpoint query skipped unreadable session {}#{}: {:#}",
                    checkpoint_id,
                    session_index,
                    err
                );
                continue;
            }
        };

        let metadata = content.metadata;
        sessions.push(DashboardCheckpointSessionDetail {
            session_index,
            session_id: metadata
                .get("session_id")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .to_string(),
            agent: metadata
                .get("agent")
                .and_then(serde_json::Value::as_str)
                .map(canonical_agent_key)
                .unwrap_or_default(),
            created_at: metadata
                .get("created_at")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .to_string(),
            is_task: metadata
                .get("is_task")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false),
            tool_use_id: metadata
                .get("tool_use_id")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .to_string(),
            metadata_json: serde_json::to_string_pretty(&metadata)
                .unwrap_or_else(|_| "{}".to_string()),
            transcript_jsonl: content.transcript,
            prompts_text: content.prompts,
            context_text: content.context,
        });
    }

    let files_touched = resolve_checkpoint_files_touched(
        repo_root,
        &info.branch,
        &info.checkpoint_id,
        &checkpoint_file_relations,
        &info.files_touched,
    );
    let token_usage = dashboard_token_usage_from_committed(&info);
    Ok(DashboardCheckpointDetail {
        checkpoint_id: info.checkpoint_id,
        strategy: info.strategy,
        branch: info.branch,
        checkpoints_count: info.checkpoints_count,
        files_touched,
        session_count: info.session_count,
        token_usage,
        sessions,
    })
}

fn resolve_checkpoint_files_touched(
    repo_root: &Path,
    branch: &str,
    checkpoint_id: &str,
    file_relations: &[CheckpointFileProvenanceDetailRow],
    fallback_files_touched: &[String],
) -> Vec<DashboardCommitFileDiff> {
    let branch_commits = match walk_branch_commits_with_checkpoints(
        repo_root,
        branch,
        None,
        None,
        super::API_GIT_SCAN_LIMIT,
    ) {
        Ok(commits) => commits,
        Err(err) => {
            log::warn!(
                "dashboard checkpoint query: failed to walk branch {} while resolving files_touched for {}: {:#}",
                branch,
                checkpoint_id,
                err
            );
            return fallback_checkpoint_files_touched(file_relations, fallback_files_touched);
        }
    };

    let Some(commit_sha) = branch_commits
        .into_iter()
        .find(|commit| commit.checkpoint_id == checkpoint_id)
        .map(|commit| commit.sha)
    else {
        return fallback_checkpoint_files_touched(file_relations, fallback_files_touched);
    };

    match read_commit_numstat(repo_root, &commit_sha) {
        Ok(stats) => {
            if file_relations.is_empty() {
                dashboard_file_diff_list_from_numstat(stats)
            } else {
                dashboard_checkpoint_file_diff_list_from_relations(file_relations, Some(&stats))
            }
        }
        Err(err) => {
            log::warn!(
                "dashboard checkpoint query: failed to read numstat for {} (checkpoint {}): {:#}",
                commit_sha,
                checkpoint_id,
                err
            );
            fallback_checkpoint_files_touched(file_relations, fallback_files_touched)
        }
    }
}

fn fallback_checkpoint_files_touched(
    file_relations: &[CheckpointFileProvenanceDetailRow],
    fallback_files_touched: &[String],
) -> Vec<DashboardCommitFileDiff> {
    if file_relations.is_empty() {
        dashboard_zeroed_file_diff_list(fallback_files_touched)
    } else {
        dashboard_checkpoint_file_diff_list_from_relations(file_relations, None)
    }
}

async fn load_checkpoint_file_relations(
    repo_root: &Path,
    repo_id: &str,
    checkpoint_id: &str,
) -> anyhow::Result<Vec<CheckpointFileProvenanceDetailRow>> {
    let relational_store =
        crate::host::relational_store::DefaultRelationalStore::open_local_for_repo_root(repo_root)?;
    let sqlite_path = relational_store.sqlite_path().to_path_buf();
    if !sqlite_path.is_file() {
        return Ok(Vec::new());
    }

    let relational = relational_store.to_local_inner();
    CheckpointFileGateway::new(&relational)
        .list_checkpoint_files(repo_id, checkpoint_id)
        .await
}

fn map_bundle_error(error: BundleError) -> ApiError {
    match error {
        BundleError::ManifestFetchFailed(message) => ApiError::with_code(
            axum::http::StatusCode::BAD_GATEWAY,
            "manifest_fetch_failed",
            message,
        ),
        BundleError::ManifestParseFailed(message) => ApiError::with_code(
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            "internal",
            message,
        ),
        BundleError::NoCompatibleVersion => ApiError::with_code(
            axum::http::StatusCode::CONFLICT,
            "no_compatible_version",
            "no compatible dashboard bundle version is available for this CLI version",
        ),
        BundleError::BundleDownloadFailed(message) => ApiError::with_code(
            axum::http::StatusCode::BAD_GATEWAY,
            "bundle_download_failed",
            message,
        ),
        BundleError::ChecksumMismatch => ApiError::with_code(
            axum::http::StatusCode::UNPROCESSABLE_ENTITY,
            "checksum_mismatch",
            "downloaded bundle checksum did not match",
        ),
        BundleError::BundleInstallFailed(message) => ApiError::with_code(
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            "bundle_install_failed",
            message,
        ),
        BundleError::Internal(message) => ApiError::with_code(
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            "internal",
            message,
        ),
    }
}

fn bundle_error_code(error: &BundleError) -> &'static str {
    match error {
        BundleError::ManifestFetchFailed(_) => "manifest_fetch_failed",
        BundleError::ManifestParseFailed(_) => "internal",
        BundleError::NoCompatibleVersion => "no_compatible_version",
        BundleError::BundleDownloadFailed(_) => "bundle_download_failed",
        BundleError::ChecksumMismatch => "checksum_mismatch",
        BundleError::BundleInstallFailed(_) => "bundle_install_failed",
        BundleError::Internal(_) => "internal",
    }
}
