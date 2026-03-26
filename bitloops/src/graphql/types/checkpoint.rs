use async_graphql::{ComplexObject, Context, ID, Result, SimpleObject};

use crate::graphql::{backend_error, loaders::DataLoaders};
use crate::host::checkpoints::strategy::manual_commit::CommittedInfo;

use super::{Commit, DateTimeScalar, JsonScalar};

const UNIX_EPOCH_RFC3339: &str = "1970-01-01T00:00:00+00:00";

#[derive(Debug, Clone, SimpleObject)]
#[graphql(complex)]
pub struct Checkpoint {
    pub id: ID,
    pub session_id: String,
    pub commit_sha: Option<String>,
    pub branch: Option<String>,
    pub agent: Option<String>,
    pub event_time: DateTimeScalar,
    pub strategy: Option<String>,
    pub files_touched: Vec<String>,
    pub payload: Option<JsonScalar>,
    pub checkpoints_count: i32,
    pub session_count: i32,
    pub token_usage: Option<CheckpointTokenUsage>,
    pub agents: Vec<String>,
    pub first_prompt_preview: Option<String>,
    pub created_at: Option<String>,
    pub is_task: bool,
    pub tool_use_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, SimpleObject)]
pub struct CheckpointTokenUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_creation_tokens: u64,
    pub cache_read_tokens: u64,
    pub api_call_count: u64,
}

impl Checkpoint {
    pub fn from_committed(commit_sha: &str, info: &CommittedInfo) -> Self {
        let event_time = DateTimeScalar::from_rfc3339(info.created_at.clone())
            .or_else(|_| DateTimeScalar::from_rfc3339(UNIX_EPOCH_RFC3339))
            .expect("static epoch timestamp must parse");
        let agents = if info.agents.is_empty() {
            info.agent
                .trim()
                .is_empty()
                .then(Vec::new)
                .unwrap_or_else(|| vec![info.agent.clone()])
        } else {
            info.agents.clone()
        };
        Self {
            id: ID(info.checkpoint_id.clone()),
            session_id: info.session_id.clone(),
            commit_sha: Some(commit_sha.to_string()),
            branch: non_empty(info.branch.as_str()),
            agent: non_empty(info.agent.as_str()),
            event_time: event_time.clone(),
            strategy: non_empty(info.strategy.as_str()),
            files_touched: info.files_touched.clone(),
            payload: None,
            checkpoints_count: info.checkpoints_count.try_into().unwrap_or(i32::MAX),
            session_count: info.session_count.try_into().unwrap_or(i32::MAX),
            token_usage: info
                .token_usage
                .as_ref()
                .map(CheckpointTokenUsage::from_metadata),
            agents,
            first_prompt_preview: non_empty(info.first_prompt_preview.as_str()),
            created_at: non_empty(info.created_at.as_str()),
            is_task: info.is_task,
            tool_use_id: non_empty(info.tool_use_id.as_str()),
        }
    }

    pub fn cursor(&self) -> String {
        self.id.to_string()
    }
}

#[ComplexObject]
impl Checkpoint {
    async fn commit(&self, ctx: &Context<'_>) -> Result<Option<Commit>> {
        let Some(commit_sha) = self.commit_sha.as_deref() else {
            return Ok(None);
        };

        let mut commit = ctx
            .data_unchecked::<DataLoaders>()
            .load_commit_by_sha(commit_sha)
            .await
            .map_err(|err| {
                backend_error(format!(
                    "failed to resolve commit {} for checkpoint {:?}: {err:#}",
                    commit_sha, self.id
                ))
            })?;
        if let Some(commit) = commit.as_mut()
            && commit.branch.is_none()
        {
            commit.branch = self.branch.clone();
        }
        Ok(commit)
    }
}

fn non_empty(value: &str) -> Option<String> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

impl CheckpointTokenUsage {
    fn from_metadata(
        metadata: &crate::host::checkpoints::strategy::manual_commit::TokenUsageMetadata,
    ) -> Self {
        Self {
            input_tokens: metadata.input_tokens,
            output_tokens: metadata.output_tokens,
            cache_creation_tokens: metadata.cache_creation_tokens,
            cache_read_tokens: metadata.cache_read_tokens,
            api_call_count: metadata.api_call_count,
        }
    }
}
