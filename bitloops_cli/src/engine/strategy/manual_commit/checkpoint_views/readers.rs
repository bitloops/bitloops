pub fn update_summary(
    repo_root: &Path,
    checkpoint_id: &str,
    summary: serde_json::Value,
) -> Result<()> {
    let redacted_summary = redact_json_value(&summary);
    let db_updated =
        update_checkpoint_session_summary_in_db(repo_root, checkpoint_id, &redacted_summary)?;
    if db_updated {
        return Ok(());
    }
    anyhow::bail!("checkpoint not found: {checkpoint_id}");
}

fn build_checkpoint_session_ref(checkpoint_id: &str, session_index: i64) -> CheckpointSessionRef {
    let (a, b) = checkpoint_dir_parts(checkpoint_id);
    let base = format!("{a}/{b}/{session_index}");
    CheckpointSessionRef {
        metadata: format!("/{base}/{}", paths::METADATA_FILE_NAME),
        transcript: format!("/{base}/{}", paths::TRANSCRIPT_FILE_NAME),
        context: format!("/{base}/{}", paths::CONTEXT_FILE_NAME),
        content_hash: format!("/{base}/{}", paths::CONTENT_HASH_FILE_NAME),
        prompt: format!("/{base}/{}", paths::PROMPT_FILE_NAME),
    }
}

fn parse_json_string_array(raw: &str) -> Vec<String> {
    serde_json::from_str::<Vec<String>>(raw).unwrap_or_default()
}

fn read_checkpoint_blob_text(
    storage: &CheckpointStorageContext,
    checkpoint_id: &str,
    session_index: i64,
    blob_type: crate::storage::blob::BlobType,
) -> String {
    let reference = crate::storage::blob::load_checkpoint_blob_reference(
        &storage.sqlite,
        checkpoint_id,
        session_index,
        blob_type.as_str(),
    );
    let Ok(Some(reference)) = reference else {
        return String::new();
    };
    let Ok(bytes) = storage.blob_store.read(&reference.storage_path) else {
        return String::new();
    };
    String::from_utf8_lossy(&bytes).to_string()
}

fn read_committed_from_db(
    storage: &CheckpointStorageContext,
    checkpoint_id: &str,
) -> Result<Option<CheckpointSummaryView>> {
    use rusqlite::OptionalExtension;

    let checkpoint_row = storage.sqlite.with_connection(|conn| {
        conn.query_row(
            "SELECT strategy, branch, cli_version, checkpoints_count, files_touched, token_usage
             FROM checkpoints
             WHERE checkpoint_id = ?1 AND repo_id = ?2
             LIMIT 1",
            rusqlite::params![checkpoint_id, storage.repo_id.as_str()],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, Option<String>>(5)?,
                ))
            },
        )
        .optional()
        .map_err(anyhow::Error::from)
    })?;
    let Some((strategy, branch, cli_version, checkpoints_count, files_touched_raw, token_usage_raw)) =
        checkpoint_row
    else {
        return Ok(None);
    };

    let session_indexes = storage.sqlite.with_connection(|conn| {
        let mut stmt = conn.prepare(
            "SELECT session_index
             FROM checkpoint_sessions
             WHERE checkpoint_id = ?1
             ORDER BY session_index ASC",
        )?;
        let mut rows = stmt.query(rusqlite::params![checkpoint_id])?;
        let mut indexes: Vec<i64> = Vec::new();
        while let Some(row) = rows.next()? {
            indexes.push(row.get::<_, i64>(0)?);
        }
        Ok(indexes)
    })?;

    let sessions = session_indexes
        .into_iter()
        .map(|idx| build_checkpoint_session_ref(checkpoint_id, idx))
        .collect::<Vec<_>>();
    let token_usage = token_usage_raw
        .as_deref()
        .and_then(|raw| serde_json::from_str::<TokenUsageMetadata>(raw).ok());
    let mut summary = CheckpointSummaryView {
        checkpoint_id: checkpoint_id.to_string(),
        cli_version,
        strategy,
        branch,
        checkpoints_count: checkpoints_count.max(0).min(u32::MAX as i64) as u32,
        files_touched: parse_json_string_array(&files_touched_raw),
        sessions,
        token_usage,
        ..Default::default()
    };
    summary.session_count = summary.sessions.len();
    Ok(Some(summary))
}

fn to_committed_info_from_db(
    storage: &CheckpointStorageContext,
    summary: &CheckpointSummaryView,
) -> Result<CommittedInfo> {
    let mut info = CommittedInfo {
        checkpoint_id: summary.checkpoint_id.clone(),
        strategy: summary.strategy.clone(),
        branch: summary.branch.clone(),
        checkpoints_count: summary.checkpoints_count,
        files_touched: summary.files_touched.clone(),
        session_count: summary.sessions.len(),
        token_usage: summary.token_usage.clone(),
        ..Default::default()
    };
    if info.session_count == 0 {
        return Ok(info);
    }

    let session_rows = storage.sqlite.with_connection(|conn| {
        let mut stmt = conn.prepare(
            "SELECT session_id, agent, created_at, is_task, tool_use_id, session_index
             FROM checkpoint_sessions
             WHERE checkpoint_id = ?1
             ORDER BY session_index ASC",
        )?;
        let mut rows = stmt.query(rusqlite::params![summary.checkpoint_id])?;
        let mut sessions: Vec<(String, String, String, bool, String, i64)> = Vec::new();
        while let Some(row) = rows.next()? {
            let is_task = row.get::<_, i64>(3)? != 0;
            sessions.push((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                is_task,
                row.get::<_, String>(4)?,
                row.get::<_, i64>(5)?,
            ));
        }
        Ok(sessions)
    })?;

    for (_, agent, _, _, _, _) in &session_rows {
        push_unique_agent(&mut info.agents, agent);
    }
    if let Some((session_id, agent, created_at, is_task, tool_use_id, _)) = session_rows.last() {
        info.session_id = session_id.clone();
        info.agent = canonicalize_agent_type(agent);
        info.created_at = created_at.clone();
        info.is_task = *is_task;
        info.tool_use_id = tool_use_id.clone();
    }
    if info.agent.is_empty()
        && let Some(last) = info.agents.last()
    {
        info.agent = last.clone();
    }

    if let Some(first_ref) = crate::storage::blob::load_checkpoint_blob_reference(
        &storage.sqlite,
        &summary.checkpoint_id,
        0,
        crate::storage::blob::BlobType::Prompts.as_str(),
    )? && let Ok(prompt_bytes) = storage.blob_store.read(&first_ref.storage_path)
    {
        info.first_prompt_preview = first_prompt_preview(&String::from_utf8_lossy(&prompt_bytes));
    }

    Ok(info)
}

fn read_session_content_from_db(
    storage: &CheckpointStorageContext,
    checkpoint_id: &str,
    session_index: usize,
) -> Result<Option<SessionContentView>> {
    use rusqlite::OptionalExtension;

    let session_row = storage.sqlite.with_connection(|conn| {
        conn.query_row(
            "SELECT c.strategy, c.branch, c.cli_version,
                    s.session_id, s.agent, s.created_at, s.turn_id, s.checkpoints_count,
                    s.files_touched, s.is_task, s.tool_use_id,
                    s.transcript_identifier_at_start, s.checkpoint_transcript_start,
                    s.initial_attribution, s.token_usage, s.summary,
                    s.transcript_path
             FROM checkpoint_sessions s
             JOIN checkpoints c ON c.checkpoint_id = s.checkpoint_id
             WHERE s.checkpoint_id = ?1
               AND s.session_index = ?2
               AND c.repo_id = ?3
             LIMIT 1",
            rusqlite::params![checkpoint_id, session_index as i64, storage.repo_id.as_str()],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, i64>(7)?,
                    row.get::<_, String>(8)?,
                    row.get::<_, i64>(9)?,
                    row.get::<_, String>(10)?,
                    row.get::<_, String>(11)?,
                    row.get::<_, i64>(12)?,
                    row.get::<_, Option<String>>(13)?,
                    row.get::<_, Option<String>>(14)?,
                    row.get::<_, Option<String>>(15)?,
                    row.get::<_, String>(16)?,
                ))
            },
        )
        .optional()
        .map_err(anyhow::Error::from)
    })?;
    let Some((
        strategy,
        branch,
        cli_version,
        session_id,
        agent,
        created_at,
        turn_id,
        checkpoints_count,
        files_touched_raw,
        is_task,
        tool_use_id,
        transcript_identifier_at_start,
        checkpoint_transcript_start,
        initial_attribution_raw,
        token_usage_raw,
        summary_raw,
        transcript_path,
    )) = session_row
    else {
        return Ok(None);
    };

    let metadata = CommittedMetadata {
        checkpoint_id: checkpoint_id.to_string(),
        session_id,
        checkpoints_count: checkpoints_count.max(0).min(u32::MAX as i64) as u32,
        strategy,
        agent,
        created_at,
        cli_version,
        turn_id,
        files_touched: parse_json_string_array(&files_touched_raw),
        is_task: is_task != 0,
        tool_use_id,
        transcript_identifier_at_start,
        checkpoint_transcript_start,
        transcript_lines_at_start: checkpoint_transcript_start,
        branch,
        summary: summary_raw
            .as_deref()
            .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok()),
        token_usage: token_usage_raw
            .as_deref()
            .and_then(|raw| serde_json::from_str::<TokenUsageMetadata>(raw).ok()),
        initial_attribution: initial_attribution_raw
            .as_deref()
            .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok()),
        transcript_path,
    };
    let metadata_value =
        serde_json::to_value(&metadata).context("serializing checkpoint session metadata from DB")?;

    let transcript = read_checkpoint_blob_text(
        storage,
        checkpoint_id,
        session_index as i64,
        crate::storage::blob::BlobType::Transcript,
    );
    let prompts = read_checkpoint_blob_text(
        storage,
        checkpoint_id,
        session_index as i64,
        crate::storage::blob::BlobType::Prompts,
    );
    let context = read_checkpoint_blob_text(
        storage,
        checkpoint_id,
        session_index as i64,
        crate::storage::blob::BlobType::Context,
    );

    Ok(Some(SessionContentView {
        metadata: metadata_value,
        transcript,
        prompts,
        context,
    }))
}

pub fn list_committed(repo_root: &Path) -> Result<Vec<CommittedInfo>> {
    let storage = open_checkpoint_storage_context(repo_root)?;
    let checkpoint_ids = storage.sqlite.with_connection(|conn| {
        let mut stmt = conn.prepare(
            "SELECT checkpoint_id
             FROM checkpoints
             WHERE repo_id = ?1
             ORDER BY created_at DESC, checkpoint_id DESC",
        )?;
        let mut rows = stmt.query(rusqlite::params![storage.repo_id.as_str()])?;
        let mut ids = Vec::new();
        while let Some(row) = rows.next()? {
            ids.push(row.get::<_, String>(0)?);
        }
        Ok(ids)
    })?;

    let mut out: Vec<CommittedInfo> = Vec::new();
    for checkpoint_id in checkpoint_ids {
        if let Some(summary) = read_committed_from_db(&storage, &checkpoint_id)? {
            out.push(to_committed_info_from_db(&storage, &summary)?);
        }
    }
    Ok(out)
}

pub fn get_checkpoint_author(repo_root: &Path, checkpoint_id: &str) -> Result<CheckpointAuthor> {
    use rusqlite::OptionalExtension;

    let storage = open_checkpoint_storage_context(repo_root)?;
    let db_author = storage.sqlite.with_connection(|conn| {
        conn.query_row(
            "SELECT s.author_name, s.author_email
             FROM checkpoint_sessions s
             JOIN checkpoints c ON c.checkpoint_id = s.checkpoint_id
             WHERE s.checkpoint_id = ?1
               AND c.repo_id = ?2
             ORDER BY s.session_index ASC
             LIMIT 1",
            rusqlite::params![checkpoint_id, storage.repo_id.as_str()],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()
        .map_err(anyhow::Error::from)
    })?;
    if let Some((name, email)) = db_author
        && (!name.trim().is_empty() || !email.trim().is_empty())
    {
        return Ok(CheckpointAuthor { name, email });
    }
    Ok(CheckpointAuthor::default())
}

pub fn read_committed(
    repo_root: &Path,
    checkpoint_id: &str,
) -> Result<Option<CheckpointSummaryView>> {
    let storage = open_checkpoint_storage_context(repo_root)?;
    read_committed_from_db(&storage, checkpoint_id)
}

#[allow(dead_code)]
fn read_committed_with_ref(
    repo_root: &Path,
    read_ref: &str,
    checkpoint_id: &str,
) -> Result<Option<CheckpointSummaryView>> {
    if checkpoint_type_for_ref(read_ref) != CheckpointType::Committed {
        return Ok(None);
    }

    let (a, b) = checkpoint_dir_parts(checkpoint_id);
    let metadata_path = format!("{a}/{b}/{}", paths::METADATA_FILE_NAME);
    let raw = match git_show_file(repo_root, read_ref, &metadata_path) {
        Ok(v) => v,
        Err(_) => return Ok(None),
    };

    let mut summary: CheckpointSummaryView = serde_json::from_str(&raw)
        .with_context(|| format!("parsing checkpoint {checkpoint_id}"))?;
    summary.session_count = summary.sessions.len();
    Ok(Some(summary))
}

/// Returns one committed checkpoint in list shape (session-derived fields included).
pub fn read_committed_info(repo_root: &Path, checkpoint_id: &str) -> Result<Option<CommittedInfo>> {
    let storage = open_checkpoint_storage_context(repo_root)?;
    if let Some(summary) = read_committed_from_db(&storage, checkpoint_id)? {
        return Ok(Some(to_committed_info_from_db(&storage, &summary)?));
    }
    Ok(None)
}

pub fn read_session_content(
    repo_root: &Path,
    checkpoint_id: &str,
    session_index: usize,
) -> Result<SessionContentView> {
    let storage = open_checkpoint_storage_context(repo_root)?;
    let summary = read_committed_from_db(&storage, checkpoint_id)?
        .ok_or_else(|| anyhow::anyhow!("checkpoint not found"))?;
    let session_count = summary_session_count(&summary);
    if session_index >= session_count {
        anyhow::bail!("session {session_index} not found");
    }
    read_session_content_from_db(&storage, checkpoint_id, session_index)?
        .ok_or_else(|| anyhow::anyhow!("session {session_index} not found"))
}

pub fn read_latest_session_content(
    repo_root: &Path,
    checkpoint_id: &str,
) -> Result<SessionContentView> {
    let summary = read_committed(repo_root, checkpoint_id)?
        .ok_or_else(|| anyhow::anyhow!("checkpoint not found"))?;
    let session_count = summary_session_count(&summary);
    if session_count == 0 {
        anyhow::bail!("checkpoint has no sessions");
    }
    read_session_content(repo_root, checkpoint_id, session_count - 1)
}

pub fn read_session_content_by_id(
    repo_root: &Path,
    checkpoint_id: &str,
    session_id: &str,
) -> Result<SessionContentView> {
    let summary = read_committed(repo_root, checkpoint_id)?
        .ok_or_else(|| anyhow::anyhow!("checkpoint not found"))?;
    let session_count = summary_session_count(&summary);
    for idx in 0..session_count {
        // Skip unreadable session slots while searching by session ID.
        let Ok(content) = read_session_content(repo_root, checkpoint_id, idx) else {
            continue;
        };
        if content
            .metadata
            .get("session_id")
            .and_then(serde_json::Value::as_str)
            == Some(session_id)
        {
            return Ok(content);
        }
    }
    anyhow::bail!("session {session_id:?} not found in checkpoint {checkpoint_id}")
}
