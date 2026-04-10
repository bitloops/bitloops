use super::*;
use crate::host::checkpoints::transcript::metadata::{
    SessionMetadataBundle, TaskCheckpointMetadataBundle,
};

// ── Checkpoint metadata structs ───────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TokenUsageMetadata {
    #[serde(default)]
    pub input_tokens: u64,
    #[serde(default)]
    pub cache_creation_tokens: u64,
    #[serde(default)]
    pub cache_read_tokens: u64,
    #[serde(default)]
    pub output_tokens: u64,
    #[serde(default)]
    pub api_call_count: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subagent_tokens: Option<Box<TokenUsageMetadata>>,
}

pub(crate) fn canonicalize_agent_type(agent: &str) -> String {
    canonical_agent_key(agent)
}

pub(crate) fn token_usage_from_options(
    input_tokens: Option<u64>,
    output_tokens: Option<u64>,
    api_call_count: Option<u64>,
) -> Option<TokenUsageMetadata> {
    if input_tokens.is_none() && output_tokens.is_none() && api_call_count.is_none() {
        return None;
    }
    Some(TokenUsageMetadata {
        input_tokens: input_tokens.unwrap_or(0),
        output_tokens: output_tokens.unwrap_or(0),
        api_call_count: api_call_count.unwrap_or(0),
        ..Default::default()
    })
}

pub(crate) fn aggregate_token_usage(
    existing: Option<TokenUsageMetadata>,
    incoming: Option<TokenUsageMetadata>,
) -> Option<TokenUsageMetadata> {
    match (existing, incoming) {
        (None, None) => None,
        (Some(tokens), None) | (None, Some(tokens)) => Some(tokens),
        (Some(mut left), Some(right)) => {
            left.input_tokens += right.input_tokens;
            left.cache_creation_tokens += right.cache_creation_tokens;
            left.cache_read_tokens += right.cache_read_tokens;
            left.output_tokens += right.output_tokens;
            left.api_call_count += right.api_call_count;
            left.subagent_tokens = aggregate_token_usage(
                left.subagent_tokens.map(|tokens| *tokens),
                right.subagent_tokens.map(|tokens| *tokens),
            )
            .map(Box::new);
            Some(left)
        }
    }
}

#[cfg(test)]
#[derive(Debug, Serialize, Deserialize, Default)]
pub(crate) struct CheckpointTopMetadata {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub(crate) cli_version: String,
    #[serde(default)]
    pub(crate) checkpoint_id: String,
    #[serde(default)]
    pub(crate) strategy: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub(crate) branch: String,
    #[serde(default)]
    pub(crate) checkpoints_count: u32,
    #[serde(default)]
    pub(crate) files_touched: Vec<String>,
    #[serde(default)]
    pub(crate) sessions: Vec<CheckpointSessionRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) token_usage: Option<TokenUsageMetadata>,
}

/// Per-session metadata written to `<cp[:2]>/<cp[2:]>/0/metadata.json`.
///
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct CommittedMetadata {
    pub(crate) checkpoint_id: String,
    pub(crate) session_id: String,
    #[serde(default)]
    pub(crate) checkpoints_count: u32,
    pub(crate) strategy: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub(crate) agent: String,
    pub(crate) created_at: String,
    pub(crate) cli_version: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub(crate) turn_id: String,
    #[serde(default, skip_serializing_if = "is_false")]
    pub(crate) is_task: bool,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub(crate) tool_use_id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub(crate) transcript_identifier_at_start: String,
    #[serde(default, skip_serializing_if = "is_zero_i64")]
    pub(crate) checkpoint_transcript_start: i64,
    #[serde(default, skip_serializing_if = "is_zero_i64")]
    pub(crate) transcript_lines_at_start: i64,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub(crate) branch: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) summary: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) token_usage: Option<TokenUsageMetadata>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) initial_attribution: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub(crate) transcript_path: String,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum CheckpointType {
    #[default]
    Temporary,
    Committed,
}

#[allow(dead_code)]
pub(crate) fn checkpoint_type_for_ref(reference: &str) -> CheckpointType {
    if reference.ends_with(paths::METADATA_BRANCH_NAME) {
        return CheckpointType::Committed;
    }

    let short = reference
        .strip_prefix("refs/heads/")
        .or_else(|| reference.strip_prefix("refs/remotes/origin/"))
        .unwrap_or(reference);
    if is_shadow_branch(short) {
        return CheckpointType::Temporary;
    }
    CheckpointType::Committed
}

#[derive(Debug, Clone)]
pub(crate) struct UpdateCommittedOptions {
    pub(crate) checkpoint_id: String,
    pub(crate) session_id: String,
    pub(crate) transcript: Option<Vec<u8>>,
    pub(crate) prompts: Option<Vec<String>>,
    pub(crate) context: Option<Vec<u8>>,
    pub(crate) agent: String,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct WriteCommittedOptions {
    pub(crate) checkpoint_id: String,
    pub(crate) session_id: String,
    pub(crate) strategy: String,
    pub(crate) agent: String,
    pub(crate) transcript: Vec<u8>,
    pub(crate) prompts: Option<Vec<String>>,
    pub(crate) context: Option<Vec<u8>>,
    pub(crate) checkpoints_count: u32,
    #[allow(dead_code)]
    pub(crate) files_touched: Vec<String>,
    pub(crate) token_usage_input: Option<u64>,
    pub(crate) token_usage_output: Option<u64>,
    pub(crate) token_usage_api_call_count: Option<u64>,
    pub(crate) turn_id: String,
    pub(crate) transcript_identifier_at_start: String,
    pub(crate) checkpoint_transcript_start: i64,
    pub(crate) token_usage: Option<TokenUsageMetadata>,
    pub(crate) initial_attribution: Option<serde_json::Value>,
    pub(crate) author_name: String,
    pub(crate) author_email: String,
    pub(crate) summary: Option<serde_json::Value>,
    pub(crate) is_task: bool,
    pub(crate) tool_use_id: String,
    pub(crate) agent_id: String,
    pub(crate) transcript_path: String,
    pub(crate) subagent_transcript_path: String,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct WriteTemporaryOptions {
    pub(crate) session_id: String,
    pub(crate) base_commit: String,
    pub(crate) step_number: u32,
    pub(crate) modified_files: Vec<String>,
    pub(crate) new_files: Vec<String>,
    pub(crate) deleted_files: Vec<String>,
    pub(crate) session_metadata: Option<SessionMetadataBundle>,
    pub(crate) commit_message: String,
    pub(crate) author_name: String,
    pub(crate) author_email: String,
    pub(crate) is_first_checkpoint: bool,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct WriteTemporaryResult {
    pub(crate) skipped: bool,
    pub(crate) commit_hash: String,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct WriteTemporaryTaskOptions {
    pub(crate) session_id: String,
    pub(crate) base_commit: String,
    pub(crate) step_number: u32,
    pub(crate) tool_use_id: String,
    pub(crate) agent_id: String,
    pub(crate) modified_files: Vec<String>,
    pub(crate) new_files: Vec<String>,
    pub(crate) deleted_files: Vec<String>,
    pub(crate) session_metadata: Option<SessionMetadataBundle>,
    pub(crate) task_metadata: Option<TaskCheckpointMetadataBundle>,
    pub(crate) is_incremental: bool,
    pub(crate) incremental_sequence: u32,
    pub(crate) incremental_type: String,
    pub(crate) incremental_data: String,
    pub(crate) commit_message: String,
    pub(crate) author_name: String,
    pub(crate) author_email: String,
}
