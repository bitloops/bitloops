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
