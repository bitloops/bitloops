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

fn canonicalize_agent_type(agent: &str) -> String {
    canonical_agent_key(agent)
}

fn token_usage_from_options(
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

fn aggregate_token_usage(
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
struct CheckpointTopMetadata {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    cli_version: String,
    #[serde(default)]
    checkpoint_id: String,
    #[serde(default)]
    strategy: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    branch: String,
    #[serde(default)]
    checkpoints_count: u32,
    #[serde(default)]
    files_touched: Vec<String>,
    #[serde(default)]
    sessions: Vec<CheckpointSessionRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    token_usage: Option<TokenUsageMetadata>,
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
    pub(crate) files_touched: Vec<String>,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum CheckpointType {
    #[default]
    Temporary,
    Committed,
}

fn checkpoint_type_for_ref(reference: &str) -> CheckpointType {
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
struct UpdateCommittedOptions {
    checkpoint_id: String,
    session_id: String,
    transcript: Option<Vec<u8>>,
    prompts: Option<Vec<String>>,
    context: Option<Vec<u8>>,
    agent: String,
}

#[derive(Debug, Clone, Default)]
struct WriteCommittedOptions {
    checkpoint_id: String,
    session_id: String,
    strategy: String,
    agent: String,
    transcript: Vec<u8>,
    prompts: Option<Vec<String>>,
    context: Option<Vec<u8>>,
    checkpoints_count: u32,
    files_touched: Vec<String>,
    token_usage_input: Option<u64>,
    token_usage_output: Option<u64>,
    token_usage_api_call_count: Option<u64>,
    turn_id: String,
    transcript_identifier_at_start: String,
    checkpoint_transcript_start: i64,
    token_usage: Option<TokenUsageMetadata>,
    initial_attribution: Option<serde_json::Value>,
    author_name: String,
    author_email: String,
    summary: Option<serde_json::Value>,
    is_task: bool,
    tool_use_id: String,
    agent_id: String,
    transcript_path: String,
    subagent_transcript_path: String,
}

#[derive(Debug, Clone, Default)]
struct WriteTemporaryOptions {
    session_id: String,
    base_commit: String,
    step_number: u32,
    modified_files: Vec<String>,
    new_files: Vec<String>,
    deleted_files: Vec<String>,
    metadata_dir: String,
    metadata_dir_abs: String,
    commit_message: String,
    author_name: String,
    author_email: String,
    is_first_checkpoint: bool,
}

#[derive(Debug, Clone, Default)]
struct WriteTemporaryResult {
    skipped: bool,
    commit_hash: String,
}

#[derive(Debug, Clone, Default)]
struct WriteTemporaryTaskOptions {
    session_id: String,
    base_commit: String,
    step_number: u32,
    tool_use_id: String,
    agent_id: String,
    modified_files: Vec<String>,
    new_files: Vec<String>,
    deleted_files: Vec<String>,
    transcript_path: String,
    subagent_transcript_path: String,
    checkpoint_uuid: String,
    is_incremental: bool,
    incremental_sequence: u32,
    incremental_type: String,
    incremental_data: String,
    commit_message: String,
    author_name: String,
    author_email: String,
}
