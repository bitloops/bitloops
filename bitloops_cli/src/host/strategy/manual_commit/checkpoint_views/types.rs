#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CheckpointSessionRef {
    #[serde(default)]
    pub metadata: String,
    #[serde(default)]
    pub transcript: String,
    #[serde(default)]
    pub context: String,
    #[serde(default)]
    pub content_hash: String,
    #[serde(default)]
    pub prompt: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CheckpointSummaryView {
    #[serde(default)]
    pub checkpoint_id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub cli_version: String,
    #[serde(default)]
    pub strategy: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub branch: String,
    #[serde(default)]
    pub checkpoints_count: u32,
    #[serde(default)]
    pub files_touched: Vec<String>,
    #[serde(default)]
    pub sessions: Vec<CheckpointSessionRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_usage: Option<TokenUsageMetadata>,
    #[serde(default, skip)]
    pub session_count: usize,
}

/// List-row view of a committed checkpoint (session-derived fields included).
///
/// Returned only by `list_committed()`. For single-checkpoint root metadata,
/// use `read_committed()`.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CommittedInfo {
    #[serde(default)]
    pub checkpoint_id: String,
    #[serde(default)]
    pub strategy: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub branch: String,
    #[serde(default)]
    pub checkpoints_count: u32,
    #[serde(default)]
    pub files_touched: Vec<String>,
    #[serde(default)]
    pub session_count: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_usage: Option<TokenUsageMetadata>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub session_id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub agent: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub agents: Vec<String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub first_prompt_preview: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub created_at: String,
    #[serde(default, skip_serializing_if = "is_false")]
    pub is_task: bool,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub tool_use_id: String,
}

fn summary_session_count(summary: &CheckpointSummaryView) -> usize {
    summary.sessions.len()
}

#[allow(dead_code)]
fn to_committed_info(
    repo_root: &Path,
    read_ref: &str,
    summary: &CheckpointSummaryView,
) -> CommittedInfo {
    let mut info = CommittedInfo {
        checkpoint_id: summary.checkpoint_id.clone(),
        strategy: summary.strategy.clone(),
        branch: summary.branch.clone(),
        checkpoints_count: summary.checkpoints_count,
        files_touched: summary.files_touched.clone(),
        session_count: summary_session_count(summary),
        token_usage: summary.token_usage.clone(),
        ..Default::default()
    };

    if info.session_count == 0 {
        return info;
    }

    let (a, b) = checkpoint_dir_parts(&summary.checkpoint_id);
    let latest_session_index = info.session_count - 1;

    for idx in 0..info.session_count {
        let meta_path = format!("{a}/{b}/{idx}/{}", paths::METADATA_FILE_NAME);
        let Ok(raw) = git_show_file(repo_root, read_ref, &meta_path) else {
            continue;
        };

        if let Ok(meta) = serde_json::from_str::<CommittedMetadata>(&raw) {
            push_unique_agent(&mut info.agents, &meta.agent);
            if idx == latest_session_index {
                info.session_id = meta.session_id;
                info.agent = canonicalize_agent_type(&meta.agent);
                info.created_at = meta.created_at;
                info.is_task = meta.is_task;
                info.tool_use_id = meta.tool_use_id;
            }
            continue;
        }

        // Keep list/read behavior resilient to legacy metadata with partial fields.
        if let Ok(meta) = serde_json::from_str::<serde_json::Value>(&raw) {
            push_unique_agent(
                &mut info.agents,
                meta.get("agent")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or_default(),
            );

            if idx == latest_session_index {
                info.session_id = meta
                    .get("session_id")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                info.agent = canonicalize_agent_type(
                    meta.get("agent")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or_default(),
                );
                info.created_at = meta
                    .get("created_at")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                info.is_task = meta
                    .get("is_task")
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(false);
                info.tool_use_id = meta
                    .get("tool_use_id")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or_default()
                    .to_string();
            }
        }
    }

    if info.agent.is_empty()
        && let Some(last) = info.agents.last()
    {
        info.agent = last.clone();
    }

    let first_prompt_path = format!("{a}/{b}/0/{}", paths::PROMPT_FILE_NAME);
    if let Ok(raw_prompts) = git_show_file(repo_root, read_ref, &first_prompt_path) {
        info.first_prompt_preview = first_prompt_preview(&raw_prompts);
    }

    info
}

fn push_unique_agent(agents: &mut Vec<String>, agent: &str) {
    let normalized = canonicalize_agent_type(agent);
    if normalized.is_empty() || agents.iter().any(|existing| existing == &normalized) {
        return;
    }
    agents.push(normalized);
}

fn first_prompt_preview(prompts_blob: &str) -> String {
    let first_prompt = prompts_blob.split("\n\n---\n\n").next().unwrap_or_default();
    let stripped = strip_leading_wrapped_tags(first_prompt).trim_start();
    stripped.chars().take(160).collect()
}

fn strip_leading_wrapped_tags(input: &str) -> &str {
    let mut rest = input.trim_start();
    loop {
        let Some((tag_name, after_open)) = parse_leading_open_tag(rest) else {
            return rest;
        };
        let closing_tag = format!("</{tag_name}>");
        let Some(close_idx) = after_open.find(&closing_tag) else {
            return rest;
        };
        rest = after_open[close_idx + closing_tag.len()..].trim_start();
    }
}

fn parse_leading_open_tag(input: &str) -> Option<(String, &str)> {
    let trimmed = input.trim_start();
    if !trimmed.starts_with('<') || trimmed.starts_with("</") {
        return None;
    }

    let open_end = trimmed.find('>')?;
    let inside = trimmed.get(1..open_end)?.trim();
    if inside.is_empty() || inside.ends_with('/') {
        return None;
    }

    let tag_name = inside.split_whitespace().next()?.trim();
    if tag_name.is_empty() {
        return None;
    }

    Some((tag_name.to_string(), trimmed.get(open_end + 1..)?))
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CheckpointAuthor {
    pub name: String,
    pub email: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SessionContentView {
    pub metadata: serde_json::Value,
    pub transcript: String,
    pub prompts: String,
    pub context: String,
}
