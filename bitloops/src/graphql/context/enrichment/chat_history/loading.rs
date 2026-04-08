use super::session::load_session_messages;
use super::types::{CheckpointChatEvent, SessionMessageRecord};
use crate::graphql::ResolverScope;
use crate::graphql::types::{ChatEntry, DateTimeScalar};
use crate::host::checkpoints::strategy::manual_commit::list_committed;
use anyhow::{Context, Result};
use async_graphql::types::Json;
use serde_json::{Map, Value};
use std::collections::HashMap;

use super::super::super::DevqlGraphqlContext;

impl DevqlGraphqlContext {
    pub(crate) async fn load_chat_history_by_paths(
        &self,
        paths: &[String],
        scope: &ResolverScope,
    ) -> Result<HashMap<String, Vec<ChatEntry>>> {
        let unique_paths = dedup_paths(paths);
        if unique_paths.is_empty() {
            return Ok(HashMap::new());
        }

        let path_candidates = unique_paths
            .iter()
            .map(|path| (path.clone(), build_path_candidates(path)))
            .collect::<HashMap<_, _>>();
        let events = self
            .list_chat_history_events(scope, &path_candidates)
            .await?;

        let mut entries_by_path = unique_paths
            .iter()
            .cloned()
            .map(|path| (path, Vec::new()))
            .collect::<HashMap<_, _>>();
        let mut session_cache = HashMap::<(String, String), Vec<SessionMessageRecord>>::new();

        for event in events {
            let matching_paths = path_candidates
                .iter()
                .filter(|(_, candidates)| files_touched_matches(&event.files_touched, candidates))
                .map(|(path, _)| path.clone())
                .collect::<Vec<_>>();
            if matching_paths.is_empty() {
                continue;
            }

            let cache_key = (event.checkpoint_id.clone(), event.session_id.clone());
            let session_messages = session_cache
                .entry(cache_key)
                .or_insert_with(|| {
                    load_session_messages(
                        self.repo_root_for_scope(scope)
                            .ok()
                            .as_deref()
                            .unwrap_or(self.repo_root.as_path()),
                        &event.checkpoint_id,
                        &event.session_id,
                        &event.event_time,
                    )
                })
                .clone();
            if session_messages.is_empty() {
                continue;
            }

            for path in matching_paths {
                let path_entries = entries_by_path.entry(path.clone()).or_default();
                let start_index = path_entries.len();

                for (message_index, message) in session_messages.iter().enumerate() {
                    let metadata =
                        build_chat_metadata(&event, message.raw_role.as_deref(), message_index);
                    path_entries.push(ChatEntry {
                        session_id: event.session_id.clone(),
                        agent: event.agent.clone(),
                        timestamp: message.timestamp.clone(),
                        role: message.role,
                        content: message.content.clone(),
                        metadata: Some(Json(metadata)),
                        cursor: format!(
                            "chat::{path}::{}::{}::{}",
                            event.checkpoint_id,
                            event.session_id,
                            start_index + message_index
                        ),
                    });
                }
            }
        }

        Ok(entries_by_path)
    }

    async fn list_chat_history_events(
        &self,
        scope: &ResolverScope,
        path_candidates: &HashMap<String, Vec<String>>,
    ) -> Result<Vec<CheckpointChatEvent>> {
        if path_candidates.is_empty() {
            return Ok(Vec::new());
        }

        let repo_root = self.repo_root_for_scope(scope)?;
        let commit_mappings =
            super::super::super::commit_checkpoints::read_commit_checkpoint_mappings_all(
                repo_root.as_path(),
            )?;
        let mut checkpoint_commits = HashMap::<String, String>::new();
        for (commit_sha, checkpoint_ids) in commit_mappings {
            for checkpoint_id in checkpoint_ids {
                checkpoint_commits
                    .entry(checkpoint_id)
                    .or_insert_with(|| commit_sha.clone());
            }
        }

        let checkpoints = list_committed(repo_root.as_path())?;
        let mut events = Vec::new();
        for checkpoint in checkpoints {
            let matches = path_candidates
                .values()
                .any(|candidates| files_touched_matches(&checkpoint.files_touched, candidates));
            if !matches {
                continue;
            }
            let event_time = DateTimeScalar::from_rfc3339(checkpoint.created_at.clone())
                .or_else(|_| DateTimeScalar::from_rfc3339("1970-01-01T00:00:00+00:00"))
                .context("parsing committed checkpoint timestamp for chat history")?;
            events.push(CheckpointChatEvent {
                checkpoint_id: checkpoint.checkpoint_id.clone(),
                session_id: checkpoint.session_id.clone(),
                agent: checkpoint.agent.clone(),
                event_time,
                commit_sha: checkpoint_commits.get(&checkpoint.checkpoint_id).cloned(),
                branch: (!checkpoint.branch.trim().is_empty()).then_some(checkpoint.branch.clone()),
                strategy: (!checkpoint.strategy.trim().is_empty())
                    .then_some(checkpoint.strategy.clone()),
                files_touched: checkpoint.files_touched.clone(),
                payload: None,
            });
        }

        Ok(events)
    }
}

fn build_chat_metadata(
    event: &CheckpointChatEvent,
    raw_role: Option<&str>,
    message_index: usize,
) -> Value {
    let mut metadata = Map::new();
    metadata.insert(
        "checkpointId".to_string(),
        Value::String(event.checkpoint_id.clone()),
    );
    metadata.insert(
        "messageIndex".to_string(),
        Value::from(message_index as i64),
    );
    if let Some(raw_role) = raw_role
        && !raw_role.trim().is_empty()
    {
        metadata.insert(
            "rawRole".to_string(),
            Value::String(raw_role.trim().to_string()),
        );
    }
    if let Some(commit_sha) = event.commit_sha.as_ref() {
        metadata.insert("commitSha".to_string(), Value::String(commit_sha.clone()));
    }
    if let Some(branch) = event.branch.as_ref() {
        metadata.insert("branch".to_string(), Value::String(branch.clone()));
    }
    if let Some(strategy) = event.strategy.as_ref() {
        metadata.insert("strategy".to_string(), Value::String(strategy.clone()));
    }
    if let Some(payload) = event.payload.as_ref() {
        metadata.insert("eventPayload".to_string(), payload.clone());
    }

    Value::Object(metadata)
}

fn dedup_paths(paths: &[String]) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    let mut unique = Vec::new();

    for path in paths {
        let trimmed = path.trim();
        if trimmed.is_empty() {
            continue;
        }
        if seen.insert(trimmed.to_string()) {
            unique.push(trimmed.to_string());
        }
    }

    unique
}

fn build_path_candidates(path: &str) -> Vec<String> {
    let mut candidates = Vec::new();
    let raw = path.trim();
    if !raw.is_empty() {
        candidates.push(raw.to_string());
    }

    let normalised = normalise_repo_path(raw);
    if !normalised.is_empty() {
        candidates.push(normalised.clone());
        candidates.push(format!("./{normalised}"));
    }

    candidates.sort();
    candidates.dedup();
    candidates
}

fn files_touched_matches(files_touched: &[String], candidates: &[String]) -> bool {
    let candidate_set = candidates
        .iter()
        .map(|candidate| normalise_repo_path(candidate))
        .collect::<std::collections::HashSet<_>>();

    files_touched
        .iter()
        .map(|path| normalise_repo_path(path))
        .any(|path| candidate_set.contains(&path))
}

fn normalise_repo_path(path: &str) -> String {
    let mut normalised = path.trim().replace('\\', "/");
    while normalised.starts_with("./") {
        normalised = normalised[2..].to_string();
    }
    normalised.trim_start_matches('/').to_string()
}
