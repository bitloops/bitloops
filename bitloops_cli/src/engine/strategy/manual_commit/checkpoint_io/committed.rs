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
