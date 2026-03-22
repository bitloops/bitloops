struct CheckpointStorageContext {
    sqlite: crate::storage::SqliteConnectionPool,
    blob_store: Box<dyn crate::storage::blob::BlobStore>,
    blob_backend: String,
    repo_id: String,
}

fn open_checkpoint_storage_context(repo_root: &Path) -> Result<CheckpointStorageContext> {
    let cfg = crate::config::resolve_store_backend_config_for_repo(repo_root)
        .context("resolving backend config for committed checkpoints")?;
    let sqlite_path = resolve_temporary_checkpoint_sqlite_path(repo_root)?;
    let sqlite = crate::storage::SqliteConnectionPool::connect_existing(sqlite_path)
        .context("opening committed checkpoint SQLite database")?;
    sqlite
        .initialise_checkpoint_schema()
        .context("initialising committed checkpoint schema")?;

    let resolved_blob_store =
        crate::storage::blob::create_blob_store_with_backend_for_repo(&cfg.blobs, repo_root)
            .context("initialising blob storage for committed checkpoints")?;

    let repo_id = crate::host::devql::resolve_repo_identity(repo_root)
        .context("resolving repo identity for committed checkpoints")?
        .repo_id;

    Ok(CheckpointStorageContext {
        sqlite,
        blob_store: resolved_blob_store.store,
        blob_backend: resolved_blob_store.backend.to_string(),
        repo_id,
    })
}

fn find_checkpoint_session_index(
    sqlite: &crate::storage::SqliteConnectionPool,
    checkpoint_id: &str,
    session_id: &str,
) -> Result<Option<i64>> {
    use rusqlite::OptionalExtension;

    sqlite.with_connection(|conn| {
        conn.query_row(
            "SELECT session_index
             FROM checkpoint_sessions
             WHERE checkpoint_id = ?1 AND session_id = ?2
             ORDER BY session_index ASC
             LIMIT 1",
            rusqlite::params![checkpoint_id, session_id],
            |row| row.get::<_, i64>(0),
        )
        .optional()
        .map_err(anyhow::Error::from)
    })
}

fn latest_checkpoint_session_index(
    sqlite: &crate::storage::SqliteConnectionPool,
    checkpoint_id: &str,
) -> Result<Option<i64>> {
    use rusqlite::OptionalExtension;

    sqlite.with_connection(|conn| {
        conn.query_row(
            "SELECT session_index
             FROM checkpoint_sessions
             WHERE checkpoint_id = ?1
             ORDER BY session_index DESC
             LIMIT 1",
            rusqlite::params![checkpoint_id],
            |row| row.get::<_, i64>(0),
        )
        .optional()
        .map_err(anyhow::Error::from)
    })
}

fn resolve_checkpoint_session_index_for_write(
    sqlite: &crate::storage::SqliteConnectionPool,
    checkpoint_id: &str,
    session_id: &str,
) -> Result<i64> {
    if let Some(existing) = find_checkpoint_session_index(sqlite, checkpoint_id, session_id)? {
        return Ok(existing);
    }
    Ok(latest_checkpoint_session_index(sqlite, checkpoint_id)?.map_or(0, |idx| idx + 1))
}

fn aggregate_checkpoint_metadata_from_db(
    sqlite: &crate::storage::SqliteConnectionPool,
    checkpoint_id: &str,
) -> Result<(u32, Vec<String>, Option<TokenUsageMetadata>)> {
    let (checkpoints_total, files_touched, token_usage) = sqlite.with_connection(|conn| {
        let mut checkpoints_total = 0u64;
        let mut files_touched: std::collections::BTreeSet<String> =
            std::collections::BTreeSet::new();
        let mut token_usage: Option<TokenUsageMetadata> = None;
        let mut stmt = conn.prepare(
            "SELECT checkpoints_count, files_touched, token_usage
             FROM checkpoint_sessions
             WHERE checkpoint_id = ?1
             ORDER BY session_index ASC",
        )?;
        let mut rows = stmt.query(rusqlite::params![checkpoint_id])?;
        while let Some(row) = rows.next()? {
            let count: i64 = row.get(0)?;
            checkpoints_total += count.max(0) as u64;

            let files_raw: String = row.get(1)?;
            if let Ok(files) = serde_json::from_str::<Vec<String>>(&files_raw) {
                for file in files {
                    if !file.is_empty() {
                        files_touched.insert(file);
                    }
                }
            }

            let token_usage_raw: Option<String> = row.get(2)?;
            let parsed_token_usage = token_usage_raw
                .as_deref()
                .and_then(|raw| serde_json::from_str::<TokenUsageMetadata>(raw).ok());
            token_usage = aggregate_token_usage(token_usage, parsed_token_usage);
        }
        Ok((checkpoints_total, files_touched, token_usage))
    })?;

    Ok((
        checkpoints_total.min(u32::MAX as u64) as u32,
        files_touched.into_iter().collect(),
        token_usage,
    ))
}

fn upsert_checkpoint_blob(
    storage: &CheckpointStorageContext,
    checkpoint_id: &str,
    session_index: i64,
    blob_type: crate::storage::blob::BlobType,
    payload: &[u8],
) -> Result<String> {
    let key = crate::storage::blob::build_blob_key(&storage.repo_id, checkpoint_id, session_index, blob_type);
    storage
        .blob_store
        .write(&key, payload)
        .with_context(|| format!("writing {} blob for checkpoint {checkpoint_id}", blob_type.as_str()))?;

    let content_hash = format!("sha256:{}", sha256_hex(payload));
    let reference = crate::storage::blob::CheckpointBlobReference::new(
        checkpoint_id,
        session_index,
        blob_type,
        storage.blob_backend.clone(),
        key,
        content_hash.clone(),
        payload.len() as i64,
    );
    crate::storage::blob::upsert_checkpoint_blob_reference(&storage.sqlite, &reference)
        .context("upserting checkpoint blob reference row")?;
    Ok(content_hash)
}

fn upsert_checkpoint_session_row(
    storage: &CheckpointStorageContext,
    session_index: i64,
    session_meta: &CommittedMetadata,
    author_name: &str,
    author_email: &str,
    content_hash: &str,
    subagent_transcript_path: &str,
) -> Result<()> {
    let files_touched = serde_json::to_string(&session_meta.files_touched)
        .context("serializing files_touched for checkpoint_sessions row")?;
    let initial_attribution = session_meta
        .initial_attribution
        .as_ref()
        .map(serde_json::to_string)
        .transpose()
        .context("serializing initial_attribution for checkpoint_sessions row")?;
    let token_usage = session_meta
        .token_usage
        .as_ref()
        .map(serde_json::to_string)
        .transpose()
        .context("serializing token_usage for checkpoint_sessions row")?;
    let summary = session_meta
        .summary
        .as_ref()
        .map(serde_json::to_string)
        .transpose()
        .context("serializing summary for checkpoint_sessions row")?;
    let created_at = session_meta.created_at.clone();

    storage.sqlite.with_connection(|conn| {
        conn.execute(
            "INSERT INTO checkpoint_sessions (
                checkpoint_id, session_id, session_index, agent, turn_id,
                checkpoints_count, files_touched, is_task, tool_use_id,
                transcript_identifier_at_start, checkpoint_transcript_start,
                initial_attribution, token_usage, summary, author_name, author_email,
                transcript_path, subagent_transcript_path, content_hash, created_at
            ) VALUES (
                ?1, ?2, ?3, ?4, ?5,
                ?6, ?7, ?8, ?9,
                ?10, ?11,
                ?12, ?13, ?14, ?15, ?16,
                ?17, ?18, ?19, ?20
            )
            ON CONFLICT(checkpoint_id, session_index) DO UPDATE SET
                session_id = excluded.session_id,
                agent = excluded.agent,
                turn_id = excluded.turn_id,
                checkpoints_count = excluded.checkpoints_count,
                files_touched = excluded.files_touched,
                is_task = excluded.is_task,
                tool_use_id = excluded.tool_use_id,
                transcript_identifier_at_start = excluded.transcript_identifier_at_start,
                checkpoint_transcript_start = excluded.checkpoint_transcript_start,
                initial_attribution = excluded.initial_attribution,
                token_usage = excluded.token_usage,
                summary = excluded.summary,
                author_name = excluded.author_name,
                author_email = excluded.author_email,
                transcript_path = excluded.transcript_path,
                subagent_transcript_path = excluded.subagent_transcript_path,
                content_hash = excluded.content_hash",
            rusqlite::params![
                session_meta.checkpoint_id,
                session_meta.session_id,
                session_index,
                session_meta.agent,
                session_meta.turn_id,
                i64::from(session_meta.checkpoints_count),
                files_touched,
                if session_meta.is_task { 1_i64 } else { 0_i64 },
                session_meta.tool_use_id,
                session_meta.transcript_identifier_at_start,
                session_meta.checkpoint_transcript_start,
                initial_attribution,
                token_usage,
                summary,
                author_name,
                author_email,
                session_meta.transcript_path,
                subagent_transcript_path,
                content_hash,
                created_at,
            ],
        )
        .context("upserting checkpoint_sessions row")?;
        Ok(())
    })
}

fn upsert_checkpoint_row(
    storage: &CheckpointStorageContext,
    checkpoint_id: &str,
    strategy: &str,
    branch: &str,
    checkpoints_count: u32,
    files_touched: &[String],
    token_usage: &Option<TokenUsageMetadata>,
) -> Result<()> {
    let files_touched_json = serde_json::to_string(files_touched)
        .context("serializing files_touched for checkpoints row")?;
    let token_usage_json = token_usage
        .as_ref()
        .map(serde_json::to_string)
        .transpose()
        .context("serializing token_usage for checkpoints row")?;

    storage.sqlite.with_connection(|conn| {
        conn.execute(
            "INSERT INTO checkpoints (
                checkpoint_id, repo_id, strategy, branch, cli_version,
                files_touched, checkpoints_count, token_usage, created_at, updated_at
            ) VALUES (
                ?1, ?2, ?3, ?4, ?5,
                ?6, ?7, ?8, datetime('now'), datetime('now')
            )
            ON CONFLICT(checkpoint_id) DO UPDATE SET
                repo_id = excluded.repo_id,
                strategy = excluded.strategy,
                branch = excluded.branch,
                cli_version = excluded.cli_version,
                files_touched = excluded.files_touched,
                checkpoints_count = excluded.checkpoints_count,
                token_usage = excluded.token_usage,
                updated_at = datetime('now')",
            rusqlite::params![
                checkpoint_id,
                storage.repo_id,
                strategy,
                branch,
                CLI_VERSION,
                files_touched_json,
                i64::from(checkpoints_count),
                token_usage_json,
            ],
        )
        .context("upserting checkpoints row")?;
        Ok(())
    })
}

fn persist_committed_checkpoint_db_and_blobs(
    repo_root: &Path,
    opts: &WriteCommittedOptions,
    session_meta: &CommittedMetadata,
    redacted_transcript: &[u8],
    redacted_prompts: &str,
    redacted_context: &[u8],
) -> Result<()> {
    let storage = open_checkpoint_storage_context(repo_root)?;
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
    let session_index = resolve_checkpoint_session_index_for_write(
        &storage.sqlite,
        &opts.checkpoint_id,
        &opts.session_id,
    )?;

    let content_hash = upsert_checkpoint_blob(
        &storage,
        &opts.checkpoint_id,
        session_index,
        crate::storage::blob::BlobType::Transcript,
        redacted_transcript,
    )?;
    let _ = upsert_checkpoint_blob(
        &storage,
        &opts.checkpoint_id,
        session_index,
        crate::storage::blob::BlobType::Prompts,
        redacted_prompts.as_bytes(),
    )?;
    let _ = upsert_checkpoint_blob(
        &storage,
        &opts.checkpoint_id,
        session_index,
        crate::storage::blob::BlobType::Context,
        redacted_context,
    )?;

    upsert_checkpoint_session_row(
        &storage,
        session_index,
        session_meta,
        &author_name,
        &author_email,
        &content_hash,
        &opts.subagent_transcript_path,
    )?;

    let (checkpoints_count, files_touched, token_usage) =
        aggregate_checkpoint_metadata_from_db(&storage.sqlite, &opts.checkpoint_id)?;
    upsert_checkpoint_row(
        &storage,
        &opts.checkpoint_id,
        &opts.strategy,
        &session_meta.branch,
        checkpoints_count,
        &files_touched,
        &token_usage,
    )
}

fn update_checkpoint_session_summary_in_db(
    repo_root: &Path,
    checkpoint_id: &str,
    summary: &serde_json::Value,
) -> Result<bool> {
    let storage = open_checkpoint_storage_context(repo_root)?;
    let Some(session_index) = latest_checkpoint_session_index(&storage.sqlite, checkpoint_id)? else {
        return Ok(false);
    };

    let summary_json =
        serde_json::to_string(summary).context("serializing summary for checkpoint_sessions row")?;
    storage.sqlite.with_connection(|conn| {
        conn.execute(
            "UPDATE checkpoint_sessions
             SET summary = ?3
             WHERE checkpoint_id = ?1 AND session_index = ?2",
            rusqlite::params![checkpoint_id, session_index, summary_json],
        )
        .context("updating checkpoint_sessions summary")?;
        conn.execute(
            "UPDATE checkpoints
             SET updated_at = datetime('now')
             WHERE checkpoint_id = ?1",
            rusqlite::params![checkpoint_id],
        )
        .context("touching checkpoint updated_at after summary update")?;
        Ok(())
    })?;
    Ok(true)
}

fn write_committed(repo_root: &Path, opts: WriteCommittedOptions) -> Result<()> {
    if opts.checkpoint_id.is_empty() {
        anyhow::bail!("invalid checkpoint options: checkpoint ID is required");
    }
    let _ = &opts.agent_id;

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

    persist_committed_checkpoint_db_and_blobs(
        repo_root,
        &opts,
        &session_meta,
        &redacted_transcript,
        &redacted_prompts,
        &redacted_context,
    )
}
