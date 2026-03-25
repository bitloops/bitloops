use super::*;

pub(super) async fn sync_remote_branch_current_state(
    relational: &crate::host::devql::RelationalStorage,
    repo_id: &str,
    source_branch: &str,
    remote_branch: &str,
) -> Result<()> {
    let artefact_rows = relational
        .query_rows(&format!(
            "SELECT symbol_id, artefact_id, commit_sha, revision_kind, revision_id, temp_checkpoint_id, blob_sha, path, language, canonical_kind, language_kind, symbol_fqn, parent_symbol_id, parent_artefact_id, start_line, end_line, start_byte, end_byte, signature, modifiers, docstring, content_hash \
FROM artefacts_current WHERE repo_id = '{}' AND branch = '{}' AND revision_kind = 'commit'",
            crate::host::devql::esc_pg(repo_id),
            crate::host::devql::esc_pg(source_branch),
        ))
        .await?;
    let edge_rows = relational
        .query_rows(&format!(
            "SELECT edge_id, commit_sha, revision_kind, revision_id, temp_checkpoint_id, blob_sha, path, from_symbol_id, from_artefact_id, to_symbol_id, to_artefact_id, to_symbol_ref, edge_kind, language, start_line, end_line, metadata \
FROM artefact_edges_current WHERE repo_id = '{}' AND branch = '{}' AND revision_kind = 'commit'",
            crate::host::devql::esc_pg(repo_id),
            crate::host::devql::esc_pg(source_branch),
        ))
        .await?;

    let mut statements = vec![
        format!(
            "DELETE FROM artefact_edges_current WHERE repo_id = '{}' AND branch = '{}'",
            crate::host::devql::esc_pg(repo_id),
            crate::host::devql::esc_pg(remote_branch),
        ),
        format!(
            "DELETE FROM artefacts_current WHERE repo_id = '{}' AND branch = '{}'",
            crate::host::devql::esc_pg(repo_id),
            crate::host::devql::esc_pg(remote_branch),
        ),
    ];
    statements.extend(build_artefacts_current_replication_sql(
        repo_id,
        remote_branch,
        &artefact_rows,
    ));
    statements.extend(build_artefact_edges_current_replication_sql(
        repo_id,
        remote_branch,
        &edge_rows,
    ));
    relational
        .exec_remote_batch_transactional(&statements)
        .await
}

fn build_artefacts_current_replication_sql(
    repo_id: &str,
    remote_branch: &str,
    rows: &[serde_json::Value],
) -> Vec<String> {
    let mut statements = Vec::new();
    for chunk in rows.chunks(constants::PRE_PUSH_BATCH_SIZE) {
        let mut values = Vec::new();
        for row in chunk {
            let Some(symbol_id) = sql_helpers::row_text(row, "symbol_id") else {
                continue;
            };
            let Some(artefact_id) = sql_helpers::row_text(row, "artefact_id") else {
                continue;
            };
            let Some(commit_sha) = sql_helpers::row_text(row, "commit_sha") else {
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

            let revision_kind = sql_helpers::row_text(row, "revision_kind").unwrap_or("commit");
            let revision_id = sql_helpers::row_text(row, "revision_id").unwrap_or("");
            let temp_checkpoint_id =
                sql_helpers::sql_nullable_i64(sql_helpers::row_i64(row, "temp_checkpoint_id"));
            let canonical_kind =
                sql_helpers::sql_nullable_text(sql_helpers::row_text(row, "canonical_kind"));
            let language_kind =
                sql_helpers::sql_nullable_text(sql_helpers::row_text(row, "language_kind"));
            let symbol_fqn =
                sql_helpers::sql_nullable_text(sql_helpers::row_text(row, "symbol_fqn"));
            let parent_symbol_id =
                sql_helpers::sql_nullable_text(sql_helpers::row_text(row, "parent_symbol_id"));
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
                "('{}', '{}', '{}', '{}', '{}', '{}', '{}', {}, '{}', '{}', '{}', {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {})",
                crate::host::devql::esc_pg(repo_id),
                crate::host::devql::esc_pg(remote_branch),
                crate::host::devql::esc_pg(symbol_id),
                crate::host::devql::esc_pg(artefact_id),
                crate::host::devql::esc_pg(commit_sha),
                crate::host::devql::esc_pg(revision_kind),
                crate::host::devql::esc_pg(revision_id),
                temp_checkpoint_id,
                crate::host::devql::esc_pg(blob_sha),
                crate::host::devql::esc_pg(path),
                crate::host::devql::esc_pg(language),
                canonical_kind,
                language_kind,
                symbol_fqn,
                parent_symbol_id,
                parent_artefact_id,
                start_line,
                end_line,
                start_byte,
                end_byte,
                signature,
                modifiers,
                docstring,
                content_hash,
                "now()",
            ));
        }
        if values.is_empty() {
            continue;
        }
        statements.push(format!(
            "INSERT INTO artefacts_current (repo_id, branch, symbol_id, artefact_id, commit_sha, revision_kind, revision_id, temp_checkpoint_id, blob_sha, path, language, canonical_kind, language_kind, symbol_fqn, parent_symbol_id, parent_artefact_id, start_line, end_line, start_byte, end_byte, signature, modifiers, docstring, content_hash, updated_at) VALUES {}",
            values.join(","),
        ));
    }
    statements
}

fn build_artefact_edges_current_replication_sql(
    repo_id: &str,
    remote_branch: &str,
    rows: &[serde_json::Value],
) -> Vec<String> {
    let mut statements = Vec::new();
    for chunk in rows.chunks(constants::PRE_PUSH_BATCH_SIZE) {
        let mut values = Vec::new();
        for row in chunk {
            let Some(edge_id) = sql_helpers::row_text(row, "edge_id") else {
                continue;
            };
            let Some(commit_sha) = sql_helpers::row_text(row, "commit_sha") else {
                continue;
            };
            let Some(blob_sha) = sql_helpers::row_text(row, "blob_sha") else {
                continue;
            };
            let Some(path) = sql_helpers::row_text(row, "path") else {
                continue;
            };
            let Some(from_symbol_id) = sql_helpers::row_text(row, "from_symbol_id") else {
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

            let to_symbol_id = sql_helpers::row_text(row, "to_symbol_id");
            let to_symbol_ref = sql_helpers::row_text(row, "to_symbol_ref");
            if to_symbol_id.is_none() && to_symbol_ref.is_none() {
                continue;
            }

            let revision_kind = sql_helpers::row_text(row, "revision_kind").unwrap_or("commit");
            let revision_id = sql_helpers::row_text(row, "revision_id").unwrap_or("");
            let temp_checkpoint_id =
                sql_helpers::sql_nullable_i64(sql_helpers::row_i64(row, "temp_checkpoint_id"));
            let to_artefact_id =
                sql_helpers::sql_nullable_text(sql_helpers::row_text(row, "to_artefact_id"));
            let start_line = sql_helpers::sql_nullable_i64(sql_helpers::row_i64(row, "start_line"));
            let end_line = sql_helpers::sql_nullable_i64(sql_helpers::row_i64(row, "end_line"));
            let metadata = sql_helpers::sql_jsonb_text(row.get("metadata"), "{}");

            values.push(format!(
                "('{}', '{}', '{}', '{}', '{}', '{}', {}, '{}', '{}', '{}', '{}', {}, {}, {}, '{}', '{}', {}, {}, {}, {})",
                crate::host::devql::esc_pg(edge_id),
                crate::host::devql::esc_pg(repo_id),
                crate::host::devql::esc_pg(remote_branch),
                crate::host::devql::esc_pg(commit_sha),
                crate::host::devql::esc_pg(revision_kind),
                crate::host::devql::esc_pg(revision_id),
                temp_checkpoint_id,
                crate::host::devql::esc_pg(blob_sha),
                crate::host::devql::esc_pg(path),
                crate::host::devql::esc_pg(from_symbol_id),
                crate::host::devql::esc_pg(from_artefact_id),
                sql_helpers::sql_nullable_text(to_symbol_id),
                to_artefact_id,
                sql_helpers::sql_nullable_text(to_symbol_ref),
                crate::host::devql::esc_pg(edge_kind),
                crate::host::devql::esc_pg(language),
                start_line,
                end_line,
                metadata,
                "now()",
            ));
        }
        if values.is_empty() {
            continue;
        }

        statements.push(format!(
            "INSERT INTO artefact_edges_current (edge_id, repo_id, branch, commit_sha, revision_kind, revision_id, temp_checkpoint_id, blob_sha, path, from_symbol_id, from_artefact_id, to_symbol_id, to_artefact_id, to_symbol_ref, edge_kind, language, start_line, end_line, metadata, updated_at) VALUES {}",
            values.join(","),
        ));
    }
    statements
}
