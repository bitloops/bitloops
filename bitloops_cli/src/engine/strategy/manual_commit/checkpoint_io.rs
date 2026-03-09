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

fn token_usage_from_metadata_value(meta_value: &serde_json::Value) -> Option<TokenUsageMetadata> {
    if let Some(raw_token_usage) = meta_value.get("token_usage")
        && !raw_token_usage.is_null()
    {
        if let Ok(parsed) = serde_json::from_value::<TokenUsageMetadata>(raw_token_usage.clone()) {
            return Some(parsed);
        }
        return Some(TokenUsageMetadata {
            input_tokens: raw_token_usage
                .get("input_tokens")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0),
            cache_creation_tokens: raw_token_usage
                .get("cache_creation_tokens")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0),
            cache_read_tokens: raw_token_usage
                .get("cache_read_tokens")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0),
            output_tokens: raw_token_usage
                .get("output_tokens")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0),
            api_call_count: raw_token_usage
                .get("api_call_count")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0),
            subagent_tokens: None,
        });
    }

    let has_legacy_fields = meta_value.get("token_usage_input").is_some()
        || meta_value.get("token_usage_output").is_some()
        || meta_value.get("token_usage_api_call_count").is_some();
    if !has_legacy_fields {
        return None;
    }

    Some(TokenUsageMetadata {
        input_tokens: meta_value
            .get("token_usage_input")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0),
        output_tokens: meta_value
            .get("token_usage_output")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0),
        api_call_count: meta_value
            .get("token_usage_api_call_count")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0),
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

/// Top-level checkpoint metadata written to `<cp[:2]>/<cp[2:]>/metadata.json`.
///
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

fn write_committed(repo_root: &Path, opts: WriteCommittedOptions) -> Result<()> {
    if opts.checkpoint_id.is_empty() {
        anyhow::bail!("invalid checkpoint options: checkpoint ID is required");
    }
    ensure_metadata_branch(repo_root)?;

    let metadata_ref = format!("refs/heads/{}", paths::METADATA_BRANCH_NAME);
    let (dir1, dir2) = checkpoint_dir_parts(&opts.checkpoint_id);
    let base_tree_path = format!("{dir1}/{dir2}");

    let existing_summary = read_committed(repo_root, &opts.checkpoint_id)?;
    let mut sessions = existing_summary
        .as_ref()
        .map(|s| s.sessions.clone())
        .unwrap_or_default();

    let mut session_index = None;
    for idx in 0..sessions.len() {
        let meta_path = format!("{base_tree_path}/{idx}/{}", paths::METADATA_FILE_NAME);
        if let Ok(raw) = git_show_file(repo_root, &metadata_ref, &meta_path)
            && let Ok(meta) = serde_json::from_str::<serde_json::Value>(&raw)
            && meta.get("session_id").and_then(serde_json::Value::as_str)
                == Some(opts.session_id.as_str())
        {
            session_index = Some(idx);
            break;
        }
    }
    let session_index = session_index.unwrap_or(sessions.len());
    if session_index >= sessions.len() {
        sessions.resize(session_index + 1, CheckpointSessionRef::default());
    }

    let session_base = format!("{base_tree_path}/{session_index}");
    let session_meta_path = format!("{session_base}/{}", paths::METADATA_FILE_NAME);
    let session_transcript_path = format!("{session_base}/{}", paths::TRANSCRIPT_FILE_NAME);
    let session_prompt_path = format!("{session_base}/{}", paths::PROMPT_FILE_NAME);
    let session_context_path = format!("{session_base}/{}", paths::CONTEXT_FILE_NAME);
    let session_content_hash_path = format!("{session_base}/{}", paths::CONTENT_HASH_FILE_NAME);
    let top_meta_path = format!("{base_tree_path}/{}", paths::METADATA_FILE_NAME);

    let branch = current_branch_name(repo_root);
    let redacted_transcript = redact_jsonl_bytes_with_fallback(&opts.transcript);
    let prompt_content = opts.prompts.clone().unwrap_or_default().join("\n\n---\n\n");
    let redacted_prompts = redact_text(&prompt_content);
    let redacted_context = redact_bytes(&opts.context.clone().unwrap_or_default());
    let canonical_agent = canonicalize_agent_type(&opts.agent);
    let redacted_summary = if let Some(summary) = opts.summary.as_ref() {
        if let Ok(parsed) = serde_json::from_value::<Summary>(summary.clone()) {
            redact_summary(Some(&parsed))?.and_then(|redacted| serde_json::to_value(redacted).ok())
        } else {
            Some(redact_json_value(summary))
        }
    } else {
        None
    };

    let session_meta = CommittedMetadata {
        checkpoint_id: opts.checkpoint_id.clone(),
        session_id: opts.session_id.clone(),
        checkpoints_count: opts.checkpoints_count,
        strategy: opts.strategy.clone(),
        agent: canonical_agent.clone(),
        created_at: now_rfc3339(),
        cli_version: CLI_VERSION.to_string(),
        turn_id: opts.turn_id.clone(),
        files_touched: opts.files_touched.clone(),
        is_task: opts.is_task,
        tool_use_id: opts.tool_use_id.clone(),
        transcript_identifier_at_start: opts.transcript_identifier_at_start.clone(),
        checkpoint_transcript_start: opts.checkpoint_transcript_start,
        transcript_lines_at_start: opts.checkpoint_transcript_start,
        branch: branch.clone(),
        summary: redacted_summary,
        token_usage: opts.token_usage.clone().or_else(|| {
            token_usage_from_options(
                opts.token_usage_input,
                opts.token_usage_output,
                opts.token_usage_api_call_count,
            )
        }),
        initial_attribution: opts.initial_attribution.as_ref().map(redact_json_value),
        transcript_path: opts.transcript_path.clone(),
    };

    let mut checkpoints_count_total = 0u64;
    let mut files_touched: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    let mut token_usage: Option<TokenUsageMetadata> = None;

    for idx in 0..sessions.len() {
        let meta_value = if idx == session_index {
            serde_json::to_value(&session_meta).context("serializing session metadata")?
        } else {
            let existing_meta_path =
                format!("{base_tree_path}/{idx}/{}", paths::METADATA_FILE_NAME);
            let raw = git_show_file(repo_root, &metadata_ref, &existing_meta_path)
                .with_context(|| format!("reading existing metadata at {existing_meta_path}"))?;
            serde_json::from_str::<serde_json::Value>(&raw)
                .with_context(|| format!("parsing existing metadata at {existing_meta_path}"))?
        };

        let count = meta_value
            .get("checkpoints_count")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0);
        checkpoints_count_total += count;

        if let Some(arr) = meta_value
            .get("files_touched")
            .and_then(serde_json::Value::as_array)
        {
            for file in arr {
                if let Some(f) = file.as_str() {
                    files_touched.insert(f.to_string());
                }
            }
        }

        token_usage =
            aggregate_token_usage(token_usage, token_usage_from_metadata_value(&meta_value));
    }

    sessions[session_index] = CheckpointSessionRef {
        metadata: format!("/{session_meta_path}"),
        transcript: format!("/{session_transcript_path}"),
        context: format!("/{session_context_path}"),
        content_hash: format!("/{session_content_hash_path}"),
        prompt: format!("/{session_prompt_path}"),
    };

    let top_summary = CheckpointTopMetadata {
        checkpoint_id: opts.checkpoint_id.clone(),
        cli_version: CLI_VERSION.to_string(),
        strategy: opts.strategy.clone(),
        branch: branch.clone(),
        checkpoints_count: checkpoints_count_total as u32,
        files_touched: files_touched.into_iter().collect(),
        sessions,
        token_usage,
    };

    let staging_dir = repo_root
        .join(paths::BITLOOPS_TMP_DIR)
        .join(format!("committed-{}", uuid::Uuid::new_v4().simple()));
    fs::create_dir_all(&staging_dir).context("creating committed staging directory")?;

    let top_meta_disk = staging_dir.join("metadata.json");
    let session_meta_disk = staging_dir.join("session-metadata.json");
    let transcript_disk = staging_dir.join("transcript.jsonl");
    let prompt_disk = staging_dir.join("prompt.txt");
    let context_disk = staging_dir.join("context.md");
    let content_hash_disk = staging_dir.join("content_hash.txt");

    fs::write(
        &top_meta_disk,
        serde_json::to_string_pretty(&top_summary)
            .context("serializing top checkpoint metadata")?,
    )
    .context("writing top checkpoint metadata")?;
    fs::write(
        &session_meta_disk,
        serde_json::to_string_pretty(&session_meta).context("serializing session metadata")?,
    )
    .context("writing session metadata")?;
    fs::write(&transcript_disk, &redacted_transcript).context("writing transcript")?;
    fs::write(&prompt_disk, redacted_prompts).context("writing prompts")?;
    fs::write(&context_disk, redacted_context).context("writing context")?;
    fs::write(
        &content_hash_disk,
        format!("sha256:{}", sha256_hex(&redacted_transcript)),
    )
    .context("writing content hash")?;

    let mut file_pairs: Vec<(PathBuf, String)> = vec![
        (top_meta_disk.clone(), top_meta_path.clone()),
        (session_meta_disk.clone(), session_meta_path.clone()),
        (transcript_disk.clone(), session_transcript_path.clone()),
        (prompt_disk.clone(), session_prompt_path.clone()),
        (context_disk.clone(), session_context_path.clone()),
        (content_hash_disk.clone(), session_content_hash_path.clone()),
    ];

    if opts.is_task && !opts.tool_use_id.is_empty() {
        let task_dir = format!("{base_tree_path}/tasks/{}", opts.tool_use_id);
        let checkpoint_disk = staging_dir.join("task-checkpoint.json");
        fs::write(
            &checkpoint_disk,
            serde_json::to_string_pretty(&serde_json::json!({
                "session_id": opts.session_id,
                "tool_use_id": opts.tool_use_id,
                "agent_id": opts.agent_id,
            }))
            .context("serializing task checkpoint metadata")?,
        )
        .context("writing task checkpoint metadata")?;
        file_pairs.push((
            checkpoint_disk,
            format!("{task_dir}/{}", paths::CHECKPOINT_FILE_NAME),
        ));

        if !opts.subagent_transcript_path.is_empty()
            && !opts.agent_id.is_empty()
            && let Ok(content) = fs::read(&opts.subagent_transcript_path)
        {
            let redacted = redact_jsonl_bytes_with_fallback(&content);
            let agent_disk = staging_dir.join("task-agent.jsonl");
            fs::write(&agent_disk, redacted).context("writing redacted subagent transcript")?;
            file_pairs.push((
                agent_disk,
                format!("{task_dir}/agent-{}.jsonl", opts.agent_id),
            ));
        }
    }

    let (fallback_name, fallback_email) = get_git_author_from_repo(repo_root)?;
    let author_name = if opts.author_name.is_empty() {
        fallback_name
    } else {
        opts.author_name.clone()
    };
    let author_email = if opts.author_email.is_empty() {
        fallback_email
    } else {
        opts.author_email.clone()
    };

    let mut commit_msg = format!(
        "Checkpoint: {}\n\n{}: {}\n{}: {}",
        opts.checkpoint_id,
        SESSION_TRAILER_KEY,
        opts.session_id,
        STRATEGY_TRAILER_KEY,
        opts.strategy,
    );
    if !canonical_agent.is_empty() {
        commit_msg.push_str(&format!("\n{}: {canonical_agent}", AGENT_TRAILER_KEY));
    }

    let result = commit_files_to_metadata_branch(
        repo_root,
        &file_pairs,
        &commit_msg,
        &author_name,
        &author_email,
    );

    let _ = fs::remove_dir_all(&staging_dir);
    result
}

#[derive(Debug, Clone, Default)]
struct WriteTemporaryOptions {
    session_id: String,
    base_commit: String,
    worktree_id: String,
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

fn write_temporary(repo_root: &Path, opts: WriteTemporaryOptions) -> Result<WriteTemporaryResult> {
    if opts.base_commit.is_empty() {
        anyhow::bail!("BaseCommit is required for temporary checkpoint");
    }
    validate_session_id(&opts.session_id)
        .map_err(|err| anyhow::anyhow!("invalid temporary checkpoint options: {err}"))?;

    let shadow_ref = shadow_branch_ref(&opts.base_commit, &opts.worktree_id);
    let parent_tree = run_git(repo_root, &["rev-parse", &format!("{shadow_ref}^{{tree}}")])
        .ok()
        .filter(|s| !s.is_empty())
        .or_else(|| {
            run_git(
                repo_root,
                &["rev-parse", &format!("{}^{{tree}}", opts.base_commit)],
            )
            .ok()
        });
    let parent_commit = run_git(repo_root, &["rev-parse", &shadow_ref])
        .ok()
        .filter(|s| !s.is_empty());

    let (mut status_modified, mut status_new, mut status_deleted) = if opts.is_first_checkpoint {
        working_tree_changes(repo_root)?
    } else {
        (vec![], vec![], vec![])
    };
    status_modified.extend(opts.modified_files.clone());
    status_new.extend(opts.new_files.clone());
    status_deleted.extend(opts.deleted_files.clone());

    let mut modified_set: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    let mut new_set: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    let mut deleted_set: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for file in status_modified {
        if !file.is_empty() {
            modified_set.insert(file);
        }
    }
    for file in status_new {
        if !file.is_empty() {
            new_set.insert(file);
        }
    }
    for file in status_deleted {
        if !file.is_empty() {
            deleted_set.insert(file);
        }
    }

    let parent_tree =
        parent_tree.ok_or_else(|| anyhow::anyhow!("failed to resolve base tree for checkpoint"))?;
    let tree = build_tree(
        repo_root,
        Some(parent_tree.as_str()),
        &modified_set.into_iter().collect::<Vec<_>>(),
        &new_set.into_iter().collect::<Vec<_>>(),
        &deleted_set.into_iter().collect::<Vec<_>>(),
    )?;
    let metadata_entries = if !opts.metadata_dir_abs.is_empty() && !opts.metadata_dir.is_empty() {
        copy_metadata_dir(Path::new(&opts.metadata_dir_abs), &opts.metadata_dir)?
    } else {
        BTreeMap::new()
    };
    let mut tree = tree;
    if !metadata_entries.is_empty() {
        let staging_dir = repo_root
            .join(paths::BITLOOPS_TMP_DIR)
            .join(format!("temp-metadata-{}", uuid::Uuid::new_v4().simple()));
        fs::create_dir_all(&staging_dir).context("creating temporary metadata staging dir")?;

        let mut file_pairs: Vec<(PathBuf, String)> = Vec::new();
        for (idx, (tree_path, content)) in metadata_entries.into_iter().enumerate() {
            let disk_path = staging_dir.join(format!("metadata-{idx}.txt"));
            fs::write(&disk_path, content)
                .with_context(|| format!("writing staged metadata file {tree_path}"))?;
            file_pairs.push((disk_path, tree_path));
        }
        let result = build_tree_with_explicit_paths(repo_root, Some(&tree), &file_pairs);
        let _ = fs::remove_dir_all(&staging_dir);
        tree = result?;
    }

    if parent_commit.is_some() && parent_tree == tree {
        return Ok(WriteTemporaryResult {
            skipped: true,
            commit_hash: parent_commit.unwrap_or_default(),
        });
    }

    let mut ct_args: Vec<String> = vec!["commit-tree".into(), tree];
    if let Some(ref parent) = parent_commit {
        ct_args.push("-p".into());
        ct_args.push(parent.clone());
    }
    ct_args.push("-m".into());
    ct_args.push(opts.commit_message.clone());

    let str_args: Vec<&str> = ct_args.iter().map(String::as_str).collect();
    let commit = run_git_env(
        repo_root,
        &str_args,
        &[
            ("GIT_AUTHOR_NAME", &opts.author_name),
            ("GIT_AUTHOR_EMAIL", &opts.author_email),
            ("GIT_COMMITTER_NAME", &opts.author_name),
            ("GIT_COMMITTER_EMAIL", &opts.author_email),
        ],
    )?;
    let commit = commit.trim().to_string();
    run_git(repo_root, &["update-ref", &shadow_ref, &commit])?;

    Ok(WriteTemporaryResult {
        skipped: false,
        commit_hash: commit,
    })
}

#[derive(Debug, Clone, Default)]
struct WriteTemporaryTaskOptions {
    session_id: String,
    base_commit: String,
    worktree_id: String,
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

fn write_temporary_task(
    repo_root: &Path,
    opts: WriteTemporaryTaskOptions,
) -> Result<WriteTemporaryResult> {
    if opts.base_commit.is_empty() {
        anyhow::bail!("BaseCommit is required for task checkpoint");
    }
    validate_session_id(&opts.session_id)
        .map_err(|err| anyhow::anyhow!("invalid task checkpoint options: {err}"))?;
    validate_tool_use_id(&opts.tool_use_id)
        .map_err(|err| anyhow::anyhow!("invalid task checkpoint options: {err}"))?;
    validate_agent_id(&opts.agent_id)
        .map_err(|err| anyhow::anyhow!("invalid task checkpoint options: {err}"))?;

    let shadow_ref = shadow_branch_ref(&opts.base_commit, &opts.worktree_id);
    let parent_tree = run_git(repo_root, &["rev-parse", &format!("{shadow_ref}^{{tree}}")])
        .ok()
        .filter(|s| !s.is_empty())
        .or_else(|| {
            run_git(
                repo_root,
                &["rev-parse", &format!("{}^{{tree}}", opts.base_commit)],
            )
            .ok()
        })
        .ok_or_else(|| anyhow::anyhow!("failed to resolve base tree for task checkpoint"))?;
    let parent_commit = run_git(repo_root, &["rev-parse", &shadow_ref])
        .ok()
        .filter(|s| !s.is_empty());

    let mut tree = build_tree(
        repo_root,
        Some(parent_tree.as_str()),
        &opts.modified_files,
        &opts.new_files,
        &opts.deleted_files,
    )?;

    let session_metadata_dir = paths::session_metadata_dir_from_session_id(&opts.session_id);
    let task_metadata_dir = format!("{session_metadata_dir}/tasks/{}", opts.tool_use_id);
    let staging_dir = repo_root
        .join(paths::BITLOOPS_TMP_DIR)
        .join(format!("task-metadata-{}", uuid::Uuid::new_v4().simple()));
    fs::create_dir_all(&staging_dir).context("creating task metadata staging directory")?;
    let mut file_pairs: Vec<(PathBuf, String)> = Vec::new();

    if opts.is_incremental {
        let data_value = if opts.incremental_data.trim().is_empty() {
            serde_json::Value::Null
        } else if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&opts.incremental_data)
        {
            redact_json_value(&parsed)
        } else {
            serde_json::Value::String(redact_text(&opts.incremental_data))
        };
        let incremental_payload = serde_json::json!({
            "type": opts.incremental_type,
            "tool_use_id": opts.tool_use_id,
            "timestamp": now_rfc3339(),
            "data": data_value,
        });
        let incremental_file = staging_dir.join("incremental-checkpoint.json");
        fs::write(
            &incremental_file,
            serde_json::to_string_pretty(&incremental_payload)
                .context("serializing incremental task checkpoint payload")?,
        )
        .context("writing incremental task checkpoint payload")?;
        let checkpoint_name = format!("{:03}-{}.json", opts.incremental_sequence, opts.tool_use_id);
        file_pairs.push((
            incremental_file,
            format!("{task_metadata_dir}/checkpoints/{checkpoint_name}"),
        ));
    } else {
        if !opts.transcript_path.trim().is_empty()
            && let Ok(content) = fs::read(&opts.transcript_path)
        {
            let transcript_file = staging_dir.join(paths::TRANSCRIPT_FILE_NAME);
            fs::write(&transcript_file, redact_jsonl_bytes_with_fallback(&content))
                .context("writing redacted task session transcript")?;
            file_pairs.push((
                transcript_file,
                format!("{session_metadata_dir}/{}", paths::TRANSCRIPT_FILE_NAME),
            ));
        }

        let checkpoint_payload = serde_json::json!({
            "session_id": opts.session_id,
            "tool_use_id": opts.tool_use_id,
            "checkpoint_uuid": opts.checkpoint_uuid,
            "agent_id": opts.agent_id,
        });
        let checkpoint_file = staging_dir.join(paths::CHECKPOINT_FILE_NAME);
        fs::write(
            &checkpoint_file,
            serde_json::to_string_pretty(&checkpoint_payload)
                .context("serializing task checkpoint payload")?,
        )
        .context("writing task checkpoint payload")?;
        file_pairs.push((
            checkpoint_file,
            format!("{task_metadata_dir}/{}", paths::CHECKPOINT_FILE_NAME),
        ));

        if !opts.subagent_transcript_path.is_empty()
            && !opts.agent_id.is_empty()
            && let Ok(content) = fs::read(&opts.subagent_transcript_path)
        {
            let agent_file = staging_dir.join("subagent-transcript.jsonl");
            fs::write(&agent_file, redact_jsonl_bytes_with_fallback(&content))
                .context("writing redacted task subagent transcript")?;
            file_pairs.push((
                agent_file,
                format!("{task_metadata_dir}/agent-{}.jsonl", opts.agent_id),
            ));
        }
    }

    if !file_pairs.is_empty() {
        let result = build_tree_with_explicit_paths(repo_root, Some(&tree), &file_pairs);
        let _ = fs::remove_dir_all(&staging_dir);
        tree = result?;
    } else {
        let _ = fs::remove_dir_all(&staging_dir);
    }

    let mut ct_args: Vec<String> = vec!["commit-tree".into(), tree];
    if let Some(ref parent) = parent_commit {
        ct_args.push("-p".into());
        ct_args.push(parent.clone());
    }
    ct_args.push("-m".into());
    ct_args.push(opts.commit_message.clone());
    let ct_str_args: Vec<&str> = ct_args.iter().map(String::as_str).collect();

    let commit = run_git_env(
        repo_root,
        &ct_str_args,
        &[
            ("GIT_AUTHOR_NAME", &opts.author_name),
            ("GIT_AUTHOR_EMAIL", &opts.author_email),
            ("GIT_COMMITTER_NAME", &opts.author_name),
            ("GIT_COMMITTER_EMAIL", &opts.author_email),
        ],
    )?;
    let commit = commit.trim().to_string();
    run_git(repo_root, &["update-ref", &shadow_ref, &commit])?;

    Ok(WriteTemporaryResult {
        skipped: false,
        commit_hash: commit,
    })
}

fn update_committed(repo_root: &Path, opts: UpdateCommittedOptions) -> Result<()> {
    if opts.checkpoint_id.is_empty() {
        anyhow::bail!("invalid update options: checkpoint ID is required");
    }

    ensure_metadata_branch(repo_root)?;

    let metadata_ref = format!("refs/heads/{}", paths::METADATA_BRANCH_NAME);
    let (a, b) = checkpoint_dir_parts(&opts.checkpoint_id);
    let base_path = format!("{a}/{b}");
    let root_metadata_path = format!("{base_path}/{}", paths::METADATA_FILE_NAME);

    let summary_raw = git_show_file(repo_root, &metadata_ref, &root_metadata_path)
        .map_err(|_| anyhow::anyhow!("checkpoint not found: {}", opts.checkpoint_id))?;
    let summary: CheckpointSummaryView = serde_json::from_str(&summary_raw)
        .with_context(|| format!("parsing checkpoint summary at {root_metadata_path}"))?;
    if summary.sessions.is_empty() {
        anyhow::bail!("checkpoint not found: {}", opts.checkpoint_id);
    }

    let mut session_index: Option<usize> = None;
    for idx in 0..summary.sessions.len() {
        let meta_path = format!("{base_path}/{idx}/{}", paths::METADATA_FILE_NAME);
        let Ok(meta_raw) = git_show_file(repo_root, &metadata_ref, &meta_path) else {
            continue;
        };
        let Ok(meta) = serde_json::from_str::<CommittedMetadata>(&meta_raw) else {
            continue;
        };
        if meta.session_id == opts.session_id {
            session_index = Some(idx);
            break;
        }
    }
    let session_index = session_index.unwrap_or(summary.sessions.len() - 1);
    let session_path = format!("{base_path}/{session_index}");

    // Write replacement blobs to temp files, then commit them at explicit
    // metadata-branch tree paths.
    let staging_dir = repo_root
        .join(paths::BITLOOPS_TMP_DIR)
        .join(format!("update-{}", uuid::Uuid::new_v4().simple()));
    fs::create_dir_all(&staging_dir).context("creating update staging directory")?;

    let mut file_pairs: Vec<(PathBuf, String)> = vec![];
    if let Some(transcript) = opts.transcript
        && !transcript.is_empty()
    {
        let redacted = redact_jsonl_bytes_with_fallback(&transcript);
        let transcript_disk = staging_dir.join(paths::TRANSCRIPT_FILE_NAME);
        fs::write(&transcript_disk, &redacted).context("writing replacement transcript")?;
        file_pairs.push((
            transcript_disk,
            format!("{session_path}/{}", paths::TRANSCRIPT_FILE_NAME),
        ));

        let hash_disk = staging_dir.join(paths::CONTENT_HASH_FILE_NAME);
        fs::write(&hash_disk, format!("sha256:{}", sha256_hex(&redacted)))
            .context("writing replacement transcript content hash")?;
        file_pairs.push((
            hash_disk,
            format!("{session_path}/{}", paths::CONTENT_HASH_FILE_NAME),
        ));
    }

    if let Some(prompts) = opts.prompts
        && !prompts.is_empty()
    {
        let prompt_disk = staging_dir.join(paths::PROMPT_FILE_NAME);
        fs::write(&prompt_disk, redact_text(&prompts.join("\n\n---\n\n")))
            .context("writing replacement prompts")?;
        file_pairs.push((
            prompt_disk,
            format!("{session_path}/{}", paths::PROMPT_FILE_NAME),
        ));
    }

    if let Some(context) = opts.context
        && !context.is_empty()
    {
        let context_disk = staging_dir.join(paths::CONTEXT_FILE_NAME);
        fs::write(&context_disk, redact_bytes(&context)).context("writing replacement context")?;
        file_pairs.push((
            context_disk,
            format!("{session_path}/{}", paths::CONTEXT_FILE_NAME),
        ));
    }

    let result = if file_pairs.is_empty() {
        Ok(())
    } else {
        let _ = &opts.agent;
        let (author_name, author_email) = get_git_author_from_repo(repo_root)?;
        commit_files_to_metadata_branch(
            repo_root,
            &file_pairs,
            &format!("Finalize transcript for Checkpoint: {}", opts.checkpoint_id),
            &author_name,
            &author_email,
        )
    };

    let _ = fs::remove_dir_all(&staging_dir);
    result
}

