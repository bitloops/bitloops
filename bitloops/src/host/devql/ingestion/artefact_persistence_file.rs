use super::*;

// File state row, file artefact upsert, and revision comparison/management.

pub(super) async fn upsert_file_state_row(
    repo_id: &str,
    relational: &RelationalStorage,
    commit_sha: &str,
    path: &str,
    blob_sha: &str,
) -> Result<()> {
    let sql = format!(
        "INSERT INTO file_state (repo_id, commit_sha, path, blob_sha) VALUES ('{}', '{}', '{}', '{}') \
ON CONFLICT (repo_id, commit_sha, path) DO UPDATE SET blob_sha = EXCLUDED.blob_sha",
        esc_pg(repo_id),
        esc_pg(commit_sha),
        esc_pg(path),
        esc_pg(blob_sha),
    );

    relational.exec(&sql).await
}

pub(super) fn build_file_artefact_row_from_content(
    repo_id: &str,
    path: &str,
    blob_sha: &str,
    content: Option<&str>,
) -> FileArtefactRow {
    let line_count = content
        .map(|value| value.lines().count() as i32)
        .unwrap_or(1)
        .max(1);
    let byte_count = content.map(|value| value.len() as i32).unwrap_or(0).max(0);

    FileArtefactRow {
        artefact_id: revision_artefact_id(repo_id, blob_sha, &file_symbol_id(path)),
        symbol_id: file_symbol_id(path),
        language: detect_language(path),
        end_line: line_count,
        end_byte: byte_count,
    }
}

pub(super) async fn upsert_file_artefact_row(
    repo_id: &str,
    repo_root: &Path,
    relational: &RelationalStorage,
    path: &str,
    blob_sha: &str,
) -> Result<FileArtefactRow> {
    let symbol_id = file_symbol_id(path);
    let artefact_id = revision_artefact_id(repo_id, blob_sha, &symbol_id);
    let language = detect_language(path);
    let line_count = git_blob_line_count(repo_root, blob_sha).unwrap_or(1).max(1);
    let blob_content = git_blob_content(repo_root, blob_sha);
    let byte_count = blob_content
        .as_ref()
        .map(|content| content.len() as i32)
        .unwrap_or(0)
        .max(0);
    let modifiers_sql = sql_json_text_array(relational, &[]);
    let file_docstring = blob_content
        .as_deref()
        .and_then(|content| extract_file_docstring_for_language_pack(path, &language, content));
    let docstring_sql = sql_nullable_text(file_docstring.as_deref());

    let sql = format!(
        "INSERT INTO artefacts (artefact_id, symbol_id, repo_id, blob_sha, path, language, canonical_kind, language_kind, symbol_fqn, parent_artefact_id, start_line, end_line, start_byte, end_byte, signature, modifiers, docstring, content_hash) \
VALUES ('{}', '{}', '{}', '{}', '{}', '{}', 'file', 'file', '{}', NULL, 1, {}, 0, {}, NULL, {}, {}, '{}') \
ON CONFLICT (artefact_id) DO UPDATE SET symbol_id = EXCLUDED.symbol_id, repo_id = EXCLUDED.repo_id, blob_sha = EXCLUDED.blob_sha, path = EXCLUDED.path, language = EXCLUDED.language, canonical_kind = EXCLUDED.canonical_kind, language_kind = EXCLUDED.language_kind, symbol_fqn = EXCLUDED.symbol_fqn, start_line = EXCLUDED.start_line, end_line = EXCLUDED.end_line, start_byte = EXCLUDED.start_byte, end_byte = EXCLUDED.end_byte, signature = EXCLUDED.signature, modifiers = EXCLUDED.modifiers, docstring = EXCLUDED.docstring, content_hash = EXCLUDED.content_hash",
        esc_pg(&artefact_id),
        esc_pg(&symbol_id),
        esc_pg(repo_id),
        esc_pg(blob_sha),
        esc_pg(path),
        esc_pg(&language),
        esc_pg(path),
        line_count,
        byte_count,
        modifiers_sql,
        docstring_sql,
        esc_pg(blob_sha),
    );

    relational.exec(&sql).await?;
    Ok(FileArtefactRow {
        artefact_id,
        symbol_id,
        language,
        end_line: line_count,
        end_byte: byte_count,
    })
}

pub(super) fn build_file_current_record(
    path: &str,
    blob_sha: &str,
    file_artefact: &FileArtefactRow,
    docstring: Option<String>,
) -> PersistedArtefactRecord {
    PersistedArtefactRecord {
        symbol_id: file_artefact.symbol_id.clone(),
        artefact_id: file_artefact.artefact_id.clone(),
        canonical_kind: Some("file".to_string()),
        language_kind: "file".to_string(),
        symbol_fqn: path.to_string(),
        parent_symbol_id: None,
        parent_artefact_id: None,
        start_line: 1,
        end_line: file_artefact.end_line,
        start_byte: 0,
        end_byte: file_artefact.end_byte,
        signature: None,
        modifiers: vec![],
        docstring,
        content_hash: blob_sha.to_string(),
    }
}

pub(super) async fn load_current_file_revision(
    cfg: &DevqlConfig,
    relational: &RelationalStorage,
    path: &str,
) -> Result<Option<CurrentFileRevisionRecord>> {
    let updated_at_unix_expr = updated_at_unix_expr(relational);
    let sql = format!(
        "SELECT revision_kind, revision_id, blob_sha, {} AS updated_at_unix \
FROM artefacts_current WHERE repo_id = '{}' AND symbol_id = '{}' LIMIT 1",
        updated_at_unix_expr,
        esc_pg(&cfg.repo.repo_id),
        esc_pg(&file_symbol_id(path)),
    );
    let rows = relational.query_rows(&sql).await?;
    let Some(row) = rows.first() else {
        return Ok(None);
    };

    let revision_kind = row
        .get("revision_kind")
        .and_then(Value::as_str)
        .and_then(TemporalRevisionKind::from_str)
        .unwrap_or(TemporalRevisionKind::Commit);
    let revision_id = row
        .get("revision_id")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let blob_sha = row
        .get("blob_sha")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let updated_at_unix = row
        .get("updated_at_unix")
        .and_then(|value| {
            value
                .as_i64()
                .or_else(|| value.as_str().and_then(|raw| raw.parse().ok()))
        })
        .unwrap_or_default();
    Ok(Some(CurrentFileRevisionRecord {
        revision_kind,
        revision_id,
        blob_sha,
        updated_at_unix,
    }))
}

pub(super) fn incoming_revision_is_newer(
    existing: Option<&CurrentFileRevisionRecord>,
    revision_kind: TemporalRevisionKind,
    revision_id: &str,
    revision_unix: i64,
) -> bool {
    match existing {
        None => true,
        Some(existing) => match (revision_kind, existing.revision_kind) {
            (TemporalRevisionKind::Commit, TemporalRevisionKind::Temporary) => true,
            (TemporalRevisionKind::Temporary, TemporalRevisionKind::Commit) => {
                revision_unix >= existing.updated_at_unix
            }
            _ => {
                revision_unix > existing.updated_at_unix
                    || (revision_unix == existing.updated_at_unix
                        && revision_id_is_newer(revision_id, &existing.revision_id))
            }
        },
    }
}

pub(super) fn revision_id_is_newer(incoming: &str, existing: &str) -> bool {
    match (
        incoming
            .strip_prefix("temp:")
            .and_then(|v| v.parse::<u64>().ok()),
        existing
            .strip_prefix("temp:")
            .and_then(|v| v.parse::<u64>().ok()),
    ) {
        (Some(incoming_idx), Some(existing_idx)) => incoming_idx > existing_idx,
        _ => incoming > existing,
    }
}

pub(super) async fn overwrite_current_revision_metadata_for_path(
    cfg: &DevqlConfig,
    relational: &RelationalStorage,
    rev: &FileRevision<'_>,
) -> Result<()> {
    let updated_at_sql = revision_timestamp_sql(relational, rev.commit_unix);
    let temp_checkpoint_id_sql = rev
        .revision
        .temp_checkpoint_id
        .map(|value| value.to_string())
        .unwrap_or_else(|| "NULL".to_string());

    let artefacts_sql = format!(
        "UPDATE artefacts_current \
SET commit_sha = '{}', revision_kind = '{}', revision_id = '{}', temp_checkpoint_id = {}, blob_sha = '{}', updated_at = {} \
WHERE repo_id = '{}' AND path = '{}'",
        esc_pg(rev.commit_sha),
        esc_pg(rev.revision.kind.as_str()),
        esc_pg(rev.revision.id),
        temp_checkpoint_id_sql,
        esc_pg(rev.blob_sha),
        updated_at_sql,
        esc_pg(&cfg.repo.repo_id),
        esc_pg(rev.path),
    );
    relational.exec(&artefacts_sql).await?;

    let edges_sql = format!(
        "UPDATE artefact_edges_current \
SET commit_sha = '{}', revision_kind = '{}', revision_id = '{}', temp_checkpoint_id = {}, blob_sha = '{}', updated_at = {} \
WHERE repo_id = '{}' AND path = '{}'",
        esc_pg(rev.commit_sha),
        esc_pg(rev.revision.kind.as_str()),
        esc_pg(rev.revision.id),
        temp_checkpoint_id_sql,
        esc_pg(rev.blob_sha),
        updated_at_sql,
        esc_pg(&cfg.repo.repo_id),
        esc_pg(rev.path),
    );
    relational.exec(&edges_sql).await?;

    Ok(())
}
