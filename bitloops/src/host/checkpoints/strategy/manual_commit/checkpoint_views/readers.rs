use super::*;

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

pub(crate) fn build_checkpoint_session_ref(
    checkpoint_id: &str,
    session_index: i64,
) -> CheckpointSessionRef {
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

pub(crate) fn read_checkpoint_blob_text(
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

pub(crate) fn read_committed_from_db(
    storage: &CheckpointStorageContext,
    checkpoint_id: &str,
) -> Result<Option<CheckpointSummaryView>> {
    let checkpoint_sql = format!(
        "SELECT strategy, branch, cli_version, checkpoints_count, token_usage
         FROM checkpoints
         WHERE checkpoint_id = '{}' AND repo_id = '{}'
         LIMIT 1",
        crate::host::devql::esc_pg(checkpoint_id),
        crate::host::devql::esc_pg(&storage.repo_id),
    );
    let Some(checkpoint_row) =
        query_checkpoint_metadata_rows(&storage.relational, &checkpoint_sql)?
            .into_iter()
            .next()
    else {
        return Ok(None);
    };
    let strategy = checkpoint_row_text(&checkpoint_row, "strategy").unwrap_or_default();
    let branch = checkpoint_row_text(&checkpoint_row, "branch").unwrap_or_default();
    let cli_version = checkpoint_row_text(&checkpoint_row, "cli_version").unwrap_or_default();
    let checkpoints_count =
        checkpoint_row_i64(&checkpoint_row, "checkpoints_count").unwrap_or_default();
    let token_usage_raw = checkpoint_row_optional_text(&checkpoint_row, "token_usage");
    let files_touched = load_checkpoint_files_touched_from_db(storage, checkpoint_id)?;

    let session_indexes_sql = format!(
        "SELECT session_index
         FROM checkpoint_sessions
         WHERE checkpoint_id = '{}'
         ORDER BY session_index ASC",
        crate::host::devql::esc_pg(checkpoint_id),
    );
    let session_indexes =
        query_checkpoint_metadata_rows(&storage.relational, &session_indexes_sql)?
            .into_iter()
            .filter_map(|row| checkpoint_row_i64(&row, "session_index"))
            .collect::<Vec<_>>();

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
        files_touched,
        sessions,
        token_usage,
        ..Default::default()
    };
    summary.session_count = summary.sessions.len();
    Ok(Some(summary))
}

pub(crate) fn to_committed_info_from_db(
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

    let session_rows_sql = format!(
        "SELECT session_id, agent, created_at, is_task, tool_use_id, session_index
         FROM checkpoint_sessions
         WHERE checkpoint_id = '{}'
         ORDER BY session_index ASC",
        crate::host::devql::esc_pg(&summary.checkpoint_id),
    );
    let session_rows = query_checkpoint_metadata_rows(&storage.relational, &session_rows_sql)?
        .into_iter()
        .filter_map(|row| {
            Some((
                checkpoint_row_text(&row, "session_id")?,
                checkpoint_row_text(&row, "agent").unwrap_or_default(),
                checkpoint_row_text(&row, "created_at").unwrap_or_default(),
                checkpoint_row_i64(&row, "is_task").unwrap_or_default() != 0,
                checkpoint_row_text(&row, "tool_use_id").unwrap_or_default(),
                checkpoint_row_i64(&row, "session_index").unwrap_or_default(),
            ))
        })
        .collect::<Vec<_>>();

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

pub(crate) fn read_session_content_from_db(
    storage: &CheckpointStorageContext,
    checkpoint_id: &str,
    session_index: usize,
) -> Result<Option<SessionContentView>> {
    let session_sql = format!(
        "SELECT c.strategy, c.branch, c.cli_version,
                s.session_id, s.agent, s.created_at, s.turn_id, s.checkpoints_count,
                s.is_task, s.tool_use_id,
                s.transcript_identifier_at_start, s.checkpoint_transcript_start,
                s.initial_attribution, s.token_usage, s.summary,
                s.transcript_path
         FROM checkpoint_sessions s
         JOIN checkpoints c ON c.checkpoint_id = s.checkpoint_id
         WHERE s.checkpoint_id = '{}' AND s.session_index = {} AND c.repo_id = '{}'
         LIMIT 1",
        crate::host::devql::esc_pg(checkpoint_id),
        session_index,
        crate::host::devql::esc_pg(&storage.repo_id),
    );
    let Some(session_row) = query_checkpoint_metadata_rows(&storage.relational, &session_sql)?
        .into_iter()
        .next()
    else {
        return Ok(None);
    };
    let strategy = checkpoint_row_text(&session_row, "strategy").unwrap_or_default();
    let branch = checkpoint_row_text(&session_row, "branch").unwrap_or_default();
    let cli_version = checkpoint_row_text(&session_row, "cli_version").unwrap_or_default();
    let session_id = checkpoint_row_text(&session_row, "session_id").unwrap_or_default();
    let agent = checkpoint_row_text(&session_row, "agent").unwrap_or_default();
    let created_at = checkpoint_row_text(&session_row, "created_at").unwrap_or_default();
    let turn_id = checkpoint_row_text(&session_row, "turn_id").unwrap_or_default();
    let checkpoints_count =
        checkpoint_row_i64(&session_row, "checkpoints_count").unwrap_or_default();
    let is_task = checkpoint_row_i64(&session_row, "is_task").unwrap_or_default();
    let tool_use_id = checkpoint_row_text(&session_row, "tool_use_id").unwrap_or_default();
    let transcript_identifier_at_start =
        checkpoint_row_text(&session_row, "transcript_identifier_at_start").unwrap_or_default();
    let checkpoint_transcript_start =
        checkpoint_row_i64(&session_row, "checkpoint_transcript_start").unwrap_or_default();
    let initial_attribution_raw = checkpoint_row_optional_text(&session_row, "initial_attribution");
    let token_usage_raw = checkpoint_row_optional_text(&session_row, "token_usage");
    let summary_raw = checkpoint_row_optional_text(&session_row, "summary");
    let transcript_path = checkpoint_row_text(&session_row, "transcript_path").unwrap_or_default();

    let metadata = CommittedMetadata {
        checkpoint_id: checkpoint_id.to_string(),
        session_id,
        checkpoints_count: checkpoints_count.max(0).min(u32::MAX as i64) as u32,
        strategy,
        agent,
        created_at,
        cli_version,
        turn_id,
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
    let metadata_value = serde_json::to_value(&metadata)
        .context("serializing checkpoint session metadata from DB")?;

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
    let checkpoint_ids_sql = format!(
        "SELECT checkpoint_id
         FROM checkpoints
         WHERE repo_id = '{}'
         ORDER BY created_at DESC, checkpoint_id DESC",
        crate::host::devql::esc_pg(&storage.repo_id),
    );
    let checkpoint_ids = query_checkpoint_metadata_rows(&storage.relational, &checkpoint_ids_sql)?
        .into_iter()
        .filter_map(|row| checkpoint_row_text(&row, "checkpoint_id"))
        .collect::<Vec<_>>();

    let mut out: Vec<CommittedInfo> = Vec::new();
    for checkpoint_id in checkpoint_ids {
        if let Some(summary) = read_committed_from_db(&storage, &checkpoint_id)? {
            out.push(to_committed_info_from_db(&storage, &summary)?);
        }
    }
    Ok(out)
}

pub fn get_checkpoint_author(repo_root: &Path, checkpoint_id: &str) -> Result<CheckpointAuthor> {
    let storage = open_checkpoint_storage_context(repo_root)?;
    let author_sql = format!(
        "SELECT s.author_name, s.author_email
         FROM checkpoint_sessions s
         JOIN checkpoints c ON c.checkpoint_id = s.checkpoint_id
         WHERE s.checkpoint_id = '{}' AND c.repo_id = '{}'
         ORDER BY s.session_index ASC
         LIMIT 1",
        crate::host::devql::esc_pg(checkpoint_id),
        crate::host::devql::esc_pg(&storage.repo_id),
    );
    if let Some(row) = query_checkpoint_metadata_rows(&storage.relational, &author_sql)?
        .into_iter()
        .next()
    {
        let name = checkpoint_row_text(&row, "author_name").unwrap_or_default();
        let email = checkpoint_row_text(&row, "author_email").unwrap_or_default();
        if !name.trim().is_empty() || !email.trim().is_empty() {
            return Ok(CheckpointAuthor { name, email });
        }
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
pub(crate) fn read_committed_with_ref(
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
