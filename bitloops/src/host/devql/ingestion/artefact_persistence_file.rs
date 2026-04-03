use super::*;

// File state row, file artefact upsert, and revision comparison/management.

pub(super) fn build_upsert_file_state_sql(
    repo_id: &str,
    commit_sha: &str,
    path: &str,
    blob_sha: &str,
) -> String {
    format!(
        "INSERT INTO file_state (repo_id, commit_sha, path, blob_sha) VALUES ('{}', '{}', '{}', '{}') \
ON CONFLICT (repo_id, commit_sha, path) DO UPDATE SET blob_sha = EXCLUDED.blob_sha",
        esc_pg(repo_id),
        esc_pg(commit_sha),
        esc_pg(path),
        esc_pg(blob_sha),
    )
}

pub(super) async fn upsert_file_state_row(
    repo_id: &str,
    relational: &RelationalStorage,
    commit_sha: &str,
    path: &str,
    blob_sha: &str,
) -> Result<()> {
    let sql = build_upsert_file_state_sql(repo_id, commit_sha, path, blob_sha);

    relational.exec(&sql).await
}

#[cfg(test)]
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
    relational
        .exec(&build_upsert_historical_artefact_snapshot_sql(
            repo_id,
            blob_sha,
            &HistoricalArtefactSnapshotRecord {
                artefact_id: artefact_id.clone(),
                path: path.to_string(),
                parent_artefact_id: None,
                start_line: 1,
                end_line: line_count,
                start_byte: 0,
                end_byte: byte_count,
            },
        ))
        .await?;
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

pub(super) fn build_upsert_historical_artefact_snapshot_sql(
    repo_id: &str,
    blob_sha: &str,
    snapshot: &HistoricalArtefactSnapshotRecord,
) -> String {
    let parent_artefact_sql = sql_nullable_text(snapshot.parent_artefact_id.as_deref());
    format!(
        "INSERT INTO artefact_snapshots (
            repo_id, blob_sha, path, artefact_id, parent_artefact_id, start_line, end_line, start_byte, end_byte
         ) VALUES (
            '{repo_id}', '{blob_sha}', '{path}', '{artefact_id}', {parent_artefact_id}, {start_line}, {end_line}, {start_byte}, {end_byte}
         )
         ON CONFLICT (repo_id, blob_sha, artefact_id) DO UPDATE SET
            path = EXCLUDED.path,
            parent_artefact_id = EXCLUDED.parent_artefact_id,
            start_line = EXCLUDED.start_line,
            end_line = EXCLUDED.end_line,
            start_byte = EXCLUDED.start_byte,
            end_byte = EXCLUDED.end_byte",
        repo_id = esc_pg(repo_id),
        blob_sha = esc_pg(blob_sha),
        path = esc_pg(&snapshot.path),
        artefact_id = esc_pg(&snapshot.artefact_id),
        parent_artefact_id = parent_artefact_sql,
        start_line = snapshot.start_line,
        end_line = snapshot.end_line,
        start_byte = snapshot.start_byte,
        end_byte = snapshot.end_byte,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_upsert_file_state_sql_escapes_and_targets_expected_table() {
        let sql = build_upsert_file_state_sql("repo'id", "commit'sha", "src/path.rs", "blob'sha");
        assert!(
            sql.contains("INSERT INTO file_state"),
            "builder should target file_state upsert"
        );
        assert!(
            sql.contains("repo''id"),
            "builder should escape single quotes in repo_id"
        );
        assert!(
            sql.contains("ON CONFLICT (repo_id, commit_sha, path)"),
            "builder should preserve conflict target"
        );
    }
}
