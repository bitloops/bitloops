use super::*;

pub(crate) fn build_upsert_checkpoint_file_row_sql(
    row: &CheckpointFileProvenanceRow,
    dialect: RelationalDialect,
) -> String {
    format!(
        "INSERT INTO checkpoint_files (
            relation_id, repo_id, checkpoint_id, session_id, event_time, agent, branch, strategy,
            commit_sha, change_kind, path_before, path_after, blob_sha_before, blob_sha_after,
            copy_source_path, copy_source_blob_sha
        ) VALUES (
            '{relation_id}', '{repo_id}', '{checkpoint_id}', '{session_id}', {event_time}, '{agent}', '{branch}', '{strategy}',
            '{commit_sha}', '{change_kind}', {path_before}, {path_after}, {blob_sha_before}, {blob_sha_after},
            {copy_source_path}, {copy_source_blob_sha}
        )
        ON CONFLICT (relation_id) DO UPDATE SET
            repo_id = EXCLUDED.repo_id,
            checkpoint_id = EXCLUDED.checkpoint_id,
            session_id = EXCLUDED.session_id,
            event_time = EXCLUDED.event_time,
            agent = EXCLUDED.agent,
            branch = EXCLUDED.branch,
            strategy = EXCLUDED.strategy,
            commit_sha = EXCLUDED.commit_sha,
            change_kind = EXCLUDED.change_kind,
            path_before = EXCLUDED.path_before,
            path_after = EXCLUDED.path_after,
            blob_sha_before = EXCLUDED.blob_sha_before,
            blob_sha_after = EXCLUDED.blob_sha_after,
            copy_source_path = EXCLUDED.copy_source_path,
            copy_source_blob_sha = EXCLUDED.copy_source_blob_sha",
        relation_id = esc_pg(&row.relation_id),
        repo_id = esc_pg(&row.repo_id),
        checkpoint_id = esc_pg(&row.checkpoint_id),
        session_id = esc_pg(&row.session_id),
        event_time = checkpoint_event_time_sql(&row.event_time, dialect),
        agent = esc_pg(&row.agent),
        branch = esc_pg(&row.branch),
        strategy = esc_pg(&row.strategy),
        commit_sha = esc_pg(&row.commit_sha),
        change_kind = esc_pg(row.change_kind.as_str()),
        path_before = sql_nullable_text(row.path_before.as_deref()),
        path_after = sql_nullable_text(row.path_after.as_deref()),
        blob_sha_before = sql_nullable_text(row.blob_sha_before.as_deref()),
        blob_sha_after = sql_nullable_text(row.blob_sha_after.as_deref()),
        copy_source_path = sql_nullable_text(row.copy_source_path.as_deref()),
        copy_source_blob_sha = sql_nullable_text(row.copy_source_blob_sha.as_deref()),
    )
}

pub(crate) fn build_upsert_checkpoint_artefact_row_sql(
    row: &CheckpointArtefactProvenanceRow,
    dialect: RelationalDialect,
) -> String {
    format!(
        "INSERT INTO checkpoint_artefacts (
            relation_id, repo_id, checkpoint_id, session_id, event_time, agent, branch, strategy,
            commit_sha, change_kind, before_symbol_id, after_symbol_id, before_artefact_id, after_artefact_id
        ) VALUES (
            '{relation_id}', '{repo_id}', '{checkpoint_id}', '{session_id}', {event_time}, '{agent}', '{branch}', '{strategy}',
            '{commit_sha}', '{change_kind}', {before_symbol_id}, {after_symbol_id}, {before_artefact_id}, {after_artefact_id}
        )
        ON CONFLICT (relation_id) DO UPDATE SET
            repo_id = EXCLUDED.repo_id,
            checkpoint_id = EXCLUDED.checkpoint_id,
            session_id = EXCLUDED.session_id,
            event_time = EXCLUDED.event_time,
            agent = EXCLUDED.agent,
            branch = EXCLUDED.branch,
            strategy = EXCLUDED.strategy,
            commit_sha = EXCLUDED.commit_sha,
            change_kind = EXCLUDED.change_kind,
            before_symbol_id = EXCLUDED.before_symbol_id,
            after_symbol_id = EXCLUDED.after_symbol_id,
            before_artefact_id = EXCLUDED.before_artefact_id,
            after_artefact_id = EXCLUDED.after_artefact_id",
        relation_id = esc_pg(&row.relation_id),
        repo_id = esc_pg(&row.repo_id),
        checkpoint_id = esc_pg(&row.checkpoint_id),
        session_id = esc_pg(&row.session_id),
        event_time = checkpoint_event_time_sql(&row.event_time, dialect),
        agent = esc_pg(&row.agent),
        branch = esc_pg(&row.branch),
        strategy = esc_pg(&row.strategy),
        commit_sha = esc_pg(&row.commit_sha),
        change_kind = esc_pg(row.change_kind.as_str()),
        before_symbol_id = sql_nullable_text(row.before_symbol_id.as_deref()),
        after_symbol_id = sql_nullable_text(row.after_symbol_id.as_deref()),
        before_artefact_id = sql_nullable_text(row.before_artefact_id.as_deref()),
        after_artefact_id = sql_nullable_text(row.after_artefact_id.as_deref()),
    )
}

pub(crate) fn build_upsert_checkpoint_artefact_lineage_row_sql(
    row: &CheckpointArtefactLineageRow,
    dialect: RelationalDialect,
) -> String {
    format!(
        "INSERT INTO checkpoint_artefact_lineage (
            relation_id, repo_id, checkpoint_id, session_id, event_time, agent, branch, strategy,
            commit_sha, lineage_kind, source_symbol_id, source_artefact_id, dest_symbol_id, dest_artefact_id
        ) VALUES (
            '{relation_id}', '{repo_id}', '{checkpoint_id}', '{session_id}', {event_time}, '{agent}', '{branch}', '{strategy}',
            '{commit_sha}', '{lineage_kind}', '{source_symbol_id}', '{source_artefact_id}', '{dest_symbol_id}', '{dest_artefact_id}'
        )
        ON CONFLICT (relation_id) DO UPDATE SET
            repo_id = EXCLUDED.repo_id,
            checkpoint_id = EXCLUDED.checkpoint_id,
            session_id = EXCLUDED.session_id,
            event_time = EXCLUDED.event_time,
            agent = EXCLUDED.agent,
            branch = EXCLUDED.branch,
            strategy = EXCLUDED.strategy,
            commit_sha = EXCLUDED.commit_sha,
            lineage_kind = EXCLUDED.lineage_kind,
            source_symbol_id = EXCLUDED.source_symbol_id,
            source_artefact_id = EXCLUDED.source_artefact_id,
            dest_symbol_id = EXCLUDED.dest_symbol_id,
            dest_artefact_id = EXCLUDED.dest_artefact_id",
        relation_id = esc_pg(&row.relation_id),
        repo_id = esc_pg(&row.repo_id),
        checkpoint_id = esc_pg(&row.checkpoint_id),
        session_id = esc_pg(&row.session_id),
        event_time = checkpoint_event_time_sql(&row.event_time, dialect),
        agent = esc_pg(&row.agent),
        branch = esc_pg(&row.branch),
        strategy = esc_pg(&row.strategy),
        commit_sha = esc_pg(&row.commit_sha),
        lineage_kind = esc_pg(row.lineage_kind.as_str()),
        source_symbol_id = esc_pg(&row.source_symbol_id),
        source_artefact_id = esc_pg(&row.source_artefact_id),
        dest_symbol_id = esc_pg(&row.dest_symbol_id),
        dest_artefact_id = esc_pg(&row.dest_artefact_id),
    )
}

pub(crate) fn delete_checkpoint_file_rows_sql(repo_id: &str, checkpoint_id: &str) -> String {
    format!(
        "DELETE FROM checkpoint_files WHERE repo_id = '{}' AND checkpoint_id = '{}'",
        esc_pg(repo_id),
        esc_pg(checkpoint_id),
    )
}

pub(crate) fn delete_checkpoint_artefact_rows_sql(repo_id: &str, checkpoint_id: &str) -> String {
    format!(
        "DELETE FROM checkpoint_artefacts WHERE repo_id = '{}' AND checkpoint_id = '{}'",
        esc_pg(repo_id),
        esc_pg(checkpoint_id),
    )
}

pub(crate) fn delete_checkpoint_artefact_lineage_rows_sql(
    repo_id: &str,
    checkpoint_id: &str,
) -> String {
    format!(
        "DELETE FROM checkpoint_artefact_lineage WHERE repo_id = '{}' AND checkpoint_id = '{}'",
        esc_pg(repo_id),
        esc_pg(checkpoint_id),
    )
}

fn checkpoint_event_time_sql(event_time: &str, dialect: RelationalDialect) -> String {
    let trimmed = event_time.trim();
    match dialect {
        RelationalDialect::Sqlite => format!("'{}'", esc_pg(trimmed)),
        RelationalDialect::Postgres => trimmed
            .parse::<i64>()
            .map(|unix| format!("to_timestamp({unix})"))
            .unwrap_or_else(|_| format!("CAST('{}' AS TIMESTAMPTZ)", esc_pg(trimmed))),
    }
}
