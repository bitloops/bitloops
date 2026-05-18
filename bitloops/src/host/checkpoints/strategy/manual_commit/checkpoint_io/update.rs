use super::*;

pub(crate) fn update_committed(repo_root: &Path, opts: UpdateCommittedOptions) -> Result<()> {
    if opts.checkpoint_id.is_empty() {
        anyhow::bail!("invalid update options: checkpoint ID is required");
    }
    let _ = &opts.agent;

    let db_updated = update_committed_db_and_blobs(repo_root, &opts)?;
    if !db_updated {
        anyhow::bail!("checkpoint not found: {}", opts.checkpoint_id);
    }
    Ok(())
}

pub(crate) fn update_committed_db_and_blobs(
    repo_root: &Path,
    opts: &UpdateCommittedOptions,
) -> Result<bool> {
    let storage = open_checkpoint_storage_context(repo_root)?;
    let target_index =
        find_checkpoint_session_index(&storage.sqlite, &opts.checkpoint_id, &opts.session_id)?.or(
            latest_checkpoint_session_index(&storage.sqlite, &opts.checkpoint_id)?,
        );
    let Some(session_index) = target_index else {
        return Ok(false);
    };

    let mut updated_any = false;
    if let Some(transcript) = opts.transcript.as_ref()
        && !transcript.is_empty()
    {
        let redacted = redact_jsonl_bytes_with_fallback(transcript);
        let content_hash = upsert_checkpoint_blob(
            &storage,
            &opts.checkpoint_id,
            session_index,
            crate::storage::blob::BlobType::Transcript,
            &redacted,
        )?;
        storage.sqlite.with_write_connection(|conn| {
            conn.execute(
                "UPDATE checkpoint_sessions
                 SET content_hash = ?3
                 WHERE checkpoint_id = ?1 AND session_index = ?2",
                rusqlite::params![opts.checkpoint_id, session_index, content_hash],
            )
            .context("updating checkpoint_sessions content_hash")?;
            Ok(())
        })?;
        updated_any = true;
    }

    if let Some(prompts) = opts.prompts.as_ref()
        && !prompts.is_empty()
    {
        let payload = redact_text(&prompts.join("\n\n---\n\n"));
        let _ = upsert_checkpoint_blob(
            &storage,
            &opts.checkpoint_id,
            session_index,
            crate::storage::blob::BlobType::Prompts,
            payload.as_bytes(),
        )?;
        updated_any = true;
    }

    if let Some(context) = opts.context.as_ref()
        && !context.is_empty()
    {
        let payload = redact_bytes(context);
        let _ = upsert_checkpoint_blob(
            &storage,
            &opts.checkpoint_id,
            session_index,
            crate::storage::blob::BlobType::Context,
            &payload,
        )?;
        updated_any = true;
    }

    if updated_any {
        storage.sqlite.with_write_connection(|conn| {
            conn.execute(
                "UPDATE checkpoints
                 SET updated_at = datetime('now')
                 WHERE checkpoint_id = ?1",
                rusqlite::params![opts.checkpoint_id],
            )
            .context("touching checkpoint updated_at after update_committed")?;
            Ok(())
        })?;
    }
    Ok(true)
}
