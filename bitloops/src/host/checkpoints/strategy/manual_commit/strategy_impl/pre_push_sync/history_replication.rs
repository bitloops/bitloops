use super::*;

pub(super) async fn replicate_history_for_commit(
    relational: &crate::host::devql::RelationalStorage,
    repo_id: &str,
    commit_sha: &str,
) -> Result<()> {
    let commit_rows = relational
        .query_rows(&format!(
            "SELECT commit_sha, author_name, author_email, commit_message, committed_at \
FROM commits WHERE repo_id = '{}' AND commit_sha = '{}' LIMIT 1",
            crate::host::devql::esc_pg(repo_id),
            crate::host::devql::esc_pg(commit_sha),
        ))
        .await?;

    let file_state_rows = relational
        .query_rows(&format!(
            "SELECT commit_sha, path, blob_sha FROM file_state \
WHERE repo_id = '{}' AND commit_sha = '{}'",
            crate::host::devql::esc_pg(repo_id),
            crate::host::devql::esc_pg(commit_sha),
        ))
        .await?;

    let artefact_rows = relational
        .query_rows(&format!(
            "SELECT artefact_id, symbol_id, blob_sha, path, language, canonical_kind, language_kind, symbol_fqn, parent_artefact_id, start_line, end_line, start_byte, end_byte, signature, modifiers, docstring, content_hash \
FROM artefacts WHERE repo_id = '{}' AND blob_sha IN (\
    SELECT DISTINCT blob_sha FROM file_state WHERE repo_id = '{}' AND commit_sha = '{}'\
)",
            crate::host::devql::esc_pg(repo_id),
            crate::host::devql::esc_pg(repo_id),
            crate::host::devql::esc_pg(commit_sha),
        ))
        .await?;

    let edge_rows = relational
        .query_rows(&format!(
            "SELECT edge_id, blob_sha, from_artefact_id, to_artefact_id, to_symbol_ref, edge_kind, language, start_line, end_line, metadata \
FROM artefact_edges WHERE repo_id = '{}' AND blob_sha IN (\
    SELECT DISTINCT blob_sha FROM file_state WHERE repo_id = '{}' AND commit_sha = '{}'\
)",
            crate::host::devql::esc_pg(repo_id),
            crate::host::devql::esc_pg(repo_id),
            crate::host::devql::esc_pg(commit_sha),
        ))
        .await?;

    let mut statements = Vec::new();
    statements.push(build_commit_replication_sql(
        repo_id,
        commit_sha,
        commit_rows.first(),
    ));
    statements.extend(build_file_state_replication_sql(repo_id, &file_state_rows));
    statements.extend(build_artefacts_replication_sql(repo_id, &artefact_rows));
    statements.extend(build_artefact_edges_replication_sql(repo_id, &edge_rows));

    relational
        .exec_remote_batch_transactional(&statements)
        .await
}

fn build_commit_replication_sql(
    repo_id: &str,
    commit_sha: &str,
    commit_row: Option<&serde_json::Value>,
) -> String {
    let author_name = sql_helpers::sql_nullable_text(
        commit_row.and_then(|row| sql_helpers::row_text(row, "author_name")),
    );
    let author_email = sql_helpers::sql_nullable_text(
        commit_row.and_then(|row| sql_helpers::row_text(row, "author_email")),
    );
    let commit_message = sql_helpers::sql_nullable_text(
        commit_row.and_then(|row| sql_helpers::row_text(row, "commit_message")),
    );
    let committed_at = sql_helpers::sql_nullable_timestamptz(
        commit_row.and_then(|row| sql_helpers::row_text(row, "committed_at")),
    );

    format!(
        "INSERT INTO commits (commit_sha, repo_id, author_name, author_email, commit_message, committed_at) \
VALUES ('{}', '{}', {}, {}, {}, {}) \
ON CONFLICT (commit_sha) DO UPDATE SET \
repo_id = EXCLUDED.repo_id, \
author_name = EXCLUDED.author_name, \
author_email = EXCLUDED.author_email, \
commit_message = EXCLUDED.commit_message, \
committed_at = EXCLUDED.committed_at",
        crate::host::devql::esc_pg(commit_sha),
        crate::host::devql::esc_pg(repo_id),
        author_name,
        author_email,
        commit_message,
        committed_at,
    )
}

fn build_file_state_replication_sql(repo_id: &str, rows: &[serde_json::Value]) -> Vec<String> {
    let mut statements = Vec::new();
    for chunk in rows.chunks(constants::PRE_PUSH_BATCH_SIZE) {
        let mut values = Vec::new();
        for row in chunk {
            let Some(commit_sha) = sql_helpers::row_text(row, "commit_sha") else {
                continue;
            };
            let Some(path) = sql_helpers::row_text(row, "path") else {
                continue;
            };
            let Some(blob_sha) = sql_helpers::row_text(row, "blob_sha") else {
                continue;
            };
            values.push(format!(
                "('{}', '{}', '{}', '{}')",
                crate::host::devql::esc_pg(repo_id),
                crate::host::devql::esc_pg(commit_sha),
                crate::host::devql::esc_pg(path),
                crate::host::devql::esc_pg(blob_sha),
            ));
        }
        if values.is_empty() {
            continue;
        }
        statements.push(format!(
            "INSERT INTO file_state (repo_id, commit_sha, path, blob_sha) VALUES {} \
ON CONFLICT (repo_id, commit_sha, path) DO UPDATE SET blob_sha = EXCLUDED.blob_sha",
            values.join(","),
        ));
    }
    statements
}

pub(super) fn build_artefacts_replication_sql(
    repo_id: &str,
    rows: &[serde_json::Value],
) -> Vec<String> {
    let mut statements = Vec::new();
    for chunk in rows.chunks(constants::PRE_PUSH_BATCH_SIZE) {
        let mut values = Vec::new();
        for row in chunk {
            let Some(artefact_id) = sql_helpers::row_text(row, "artefact_id") else {
                continue;
            };
            let Some(blob_sha) = sql_helpers::row_text(row, "blob_sha") else {
                continue;
            };
            let Some(path) = sql_helpers::row_text(row, "path") else {
                continue;
            };
            let Some(language) = sql_helpers::row_text(row, "language") else {
                continue;
            };

            let symbol_id = sql_helpers::sql_nullable_text(sql_helpers::row_text(row, "symbol_id"));
            let canonical_kind =
                sql_helpers::sql_nullable_text(sql_helpers::row_text(row, "canonical_kind"));
            let language_kind =
                sql_helpers::sql_nullable_text(sql_helpers::row_text(row, "language_kind"));
            let symbol_fqn =
                sql_helpers::sql_nullable_text(sql_helpers::row_text(row, "symbol_fqn"));
            let parent_artefact_id =
                sql_helpers::sql_nullable_text(sql_helpers::row_text(row, "parent_artefact_id"));
            let start_line = sql_helpers::row_i64(row, "start_line").unwrap_or_default();
            let end_line = sql_helpers::row_i64(row, "end_line").unwrap_or_default();
            let start_byte = sql_helpers::row_i64(row, "start_byte").unwrap_or_default();
            let end_byte = sql_helpers::row_i64(row, "end_byte").unwrap_or_default();
            let signature = sql_helpers::sql_nullable_text(sql_helpers::row_text(row, "signature"));
            let modifiers = sql_helpers::sql_jsonb_text(row.get("modifiers"), "[]");
            let docstring = sql_helpers::sql_nullable_text(sql_helpers::row_text(row, "docstring"));
            let content_hash =
                sql_helpers::sql_nullable_text(sql_helpers::row_text(row, "content_hash"));

            values.push(format!(
                "('{}', {}, '{}', '{}', '{}', '{}', {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {})",
                crate::host::devql::esc_pg(artefact_id),
                symbol_id,
                crate::host::devql::esc_pg(repo_id),
                crate::host::devql::esc_pg(blob_sha),
                crate::host::devql::esc_pg(path),
                crate::host::devql::esc_pg(language),
                canonical_kind,
                language_kind,
                symbol_fqn,
                parent_artefact_id,
                start_line,
                end_line,
                start_byte,
                end_byte,
                signature,
                modifiers,
                docstring,
                content_hash,
            ));
        }
        if values.is_empty() {
            continue;
        }

        statements.push(format!(
            "INSERT INTO artefacts (artefact_id, symbol_id, repo_id, blob_sha, path, language, canonical_kind, language_kind, symbol_fqn, parent_artefact_id, start_line, end_line, start_byte, end_byte, signature, modifiers, docstring, content_hash) VALUES {} \
ON CONFLICT (artefact_id) DO NOTHING",
            values.join(","),
        ));
    }
    statements
}

fn build_artefact_edges_replication_sql(repo_id: &str, rows: &[serde_json::Value]) -> Vec<String> {
    let mut statements = Vec::new();
    for chunk in rows.chunks(constants::PRE_PUSH_BATCH_SIZE) {
        let mut values = Vec::new();
        for row in chunk {
            let Some(edge_id) = sql_helpers::row_text(row, "edge_id") else {
                continue;
            };
            let Some(blob_sha) = sql_helpers::row_text(row, "blob_sha") else {
                continue;
            };
            let Some(from_artefact_id) = sql_helpers::row_text(row, "from_artefact_id") else {
                continue;
            };
            let Some(edge_kind) = sql_helpers::row_text(row, "edge_kind") else {
                continue;
            };
            let Some(language) = sql_helpers::row_text(row, "language") else {
                continue;
            };

            let to_artefact_id = sql_helpers::row_text(row, "to_artefact_id");
            let to_symbol_ref = sql_helpers::row_text(row, "to_symbol_ref");
            if to_artefact_id.is_none() && to_symbol_ref.is_none() {
                continue;
            }

            let start_line = sql_helpers::sql_nullable_i64(sql_helpers::row_i64(row, "start_line"));
            let end_line = sql_helpers::sql_nullable_i64(sql_helpers::row_i64(row, "end_line"));
            let metadata = sql_helpers::sql_jsonb_text(row.get("metadata"), "{}");

            values.push(format!(
                "('{}', '{}', '{}', '{}', {}, {}, '{}', '{}', {}, {}, {})",
                crate::host::devql::esc_pg(edge_id),
                crate::host::devql::esc_pg(repo_id),
                crate::host::devql::esc_pg(blob_sha),
                crate::host::devql::esc_pg(from_artefact_id),
                sql_helpers::sql_nullable_text(to_artefact_id),
                sql_helpers::sql_nullable_text(to_symbol_ref),
                crate::host::devql::esc_pg(edge_kind),
                crate::host::devql::esc_pg(language),
                start_line,
                end_line,
                metadata,
            ));
        }
        if values.is_empty() {
            continue;
        }

        statements.push(format!(
            "INSERT INTO artefact_edges (edge_id, repo_id, blob_sha, from_artefact_id, to_artefact_id, to_symbol_ref, edge_kind, language, start_line, end_line, metadata) VALUES {} \
ON CONFLICT (edge_id) DO NOTHING",
            values.join(","),
        ));
    }
    statements
}
