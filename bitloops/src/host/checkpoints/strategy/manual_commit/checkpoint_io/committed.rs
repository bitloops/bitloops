use super::*;
use crate::storage::BlobStorageRole;
use serde_json::Value;

pub(crate) struct CheckpointStorageContext {
    pub(crate) sqlite: crate::storage::SqliteConnectionPool,
    pub(crate) relational: crate::host::relational_store::DefaultRelationalStore,
    pub(crate) blob_store: Box<dyn crate::storage::blob::BlobStore>,
    pub(crate) blob_backend: String,
    pub(crate) repo_id: String,
}

pub(crate) fn initialise_checkpoint_relational_roles(
    relational: &crate::host::relational_store::DefaultRelationalStore,
) -> Result<()> {
    relational
        .initialise_local_relational_checkpoint_schema()
        .context("initialising committed checkpoint relational schema")?;
    if relational.has_remote_shared_relational_authority() {
        let dsn = relational.inner().remote_dsn().ok_or_else(|| {
            anyhow::anyhow!("remote Postgres shared relational backend is configured without a DSN")
        })?;
        crate::storage::PostgresSyncConnection::connect(dsn)?
            .initialise_relational_checkpoint_schema()
            .context("initialising shared relational checkpoint schema")?;
    }
    Ok(())
}

pub(crate) fn open_checkpoint_relational_store(
    repo_root: &Path,
) -> Result<(
    crate::host::relational_store::DefaultRelationalStore,
    String,
)> {
    let relational = crate::host::relational_store::DefaultRelationalStore::
        open_primary_for_repo_root_preferring_bound_config(repo_root)
        .context("opening relational store for committed checkpoints")?;
    initialise_checkpoint_relational_roles(&relational)?;
    let repo_id = crate::host::devql::resolve_repo_identity(repo_root)
        .context("resolving repo identity for committed checkpoints")?
        .repo_id;
    Ok((relational, repo_id))
}

pub(crate) fn query_checkpoint_metadata_rows(
    relational: &crate::host::relational_store::DefaultRelationalStore,
    sql: &str,
) -> Result<Vec<Value>> {
    relational
        .query_rows_for_role_blocking(
            crate::host::devql::RelationalStorageRole::SharedRelational,
            sql,
        )
        .context("querying committed checkpoint metadata rows")
}

pub(crate) fn exec_checkpoint_metadata_statements(
    relational: &crate::host::relational_store::DefaultRelationalStore,
    statements: &[String],
) -> Result<()> {
    relational
        .exec_batch_transactional_for_role_blocking(
            crate::host::devql::RelationalStorageRole::SharedRelational,
            statements,
        )
        .context("executing committed checkpoint metadata statements")
}

pub(crate) fn checkpoint_row_text(row: &Value, key: &str) -> Option<String> {
    row.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

pub(crate) fn checkpoint_row_optional_text(row: &Value, key: &str) -> Option<String> {
    row.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

pub(crate) fn checkpoint_row_i64(row: &Value, key: &str) -> Option<i64> {
    row.get(key).and_then(|value| {
        value
            .as_i64()
            .or_else(|| value.as_u64().and_then(|raw| i64::try_from(raw).ok()))
            .or_else(|| {
                value
                    .as_str()
                    .and_then(|raw| raw.trim().parse::<i64>().ok())
            })
    })
}

fn checkpoint_sql_nullable_text(value: Option<&str>) -> String {
    value
        .map(|raw| format!("'{}'", crate::host::devql::esc_pg(raw)))
        .unwrap_or_else(|| "NULL".to_string())
}

fn checkpoint_timestamp_sql(value: &str, dialect: crate::host::devql::RelationalDialect) -> String {
    match dialect {
        crate::host::devql::RelationalDialect::Sqlite => {
            format!("'{}'", crate::host::devql::esc_pg(value))
        }
        crate::host::devql::RelationalDialect::Postgres => {
            format!(
                "CAST('{}' AS TIMESTAMPTZ)",
                crate::host::devql::esc_pg(value)
            )
        }
    }
}

pub(crate) fn build_upsert_checkpoint_session_row_sql(
    session_index: i64,
    session_meta: &CommittedMetadata,
    author_name: &str,
    author_email: &str,
    content_hash: &str,
    subagent_transcript_path: &str,
    dialect: crate::host::devql::RelationalDialect,
) -> Result<String> {
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

    Ok(format!(
        "INSERT INTO checkpoint_sessions (
            checkpoint_id, session_id, session_index, agent, turn_id,
            checkpoints_count, is_task, tool_use_id,
            transcript_identifier_at_start, checkpoint_transcript_start,
            initial_attribution, token_usage, summary, author_name, author_email,
            transcript_path, subagent_transcript_path, content_hash, created_at
        ) VALUES (
            '{checkpoint_id}', '{session_id}', {session_index}, '{agent}', '{turn_id}',
            {checkpoints_count}, {is_task}, '{tool_use_id}',
            '{transcript_identifier_at_start}', {checkpoint_transcript_start},
            {initial_attribution}, {token_usage}, {summary}, '{author_name}', '{author_email}',
            '{transcript_path}', '{subagent_transcript_path}', '{content_hash}', {created_at}
        )
        ON CONFLICT(checkpoint_id, session_index) DO UPDATE SET
            session_id = EXCLUDED.session_id,
            agent = EXCLUDED.agent,
            turn_id = EXCLUDED.turn_id,
            checkpoints_count = EXCLUDED.checkpoints_count,
            is_task = EXCLUDED.is_task,
            tool_use_id = EXCLUDED.tool_use_id,
            transcript_identifier_at_start = EXCLUDED.transcript_identifier_at_start,
            checkpoint_transcript_start = EXCLUDED.checkpoint_transcript_start,
            initial_attribution = EXCLUDED.initial_attribution,
            token_usage = EXCLUDED.token_usage,
            summary = EXCLUDED.summary,
            author_name = EXCLUDED.author_name,
            author_email = EXCLUDED.author_email,
            transcript_path = EXCLUDED.transcript_path,
            subagent_transcript_path = EXCLUDED.subagent_transcript_path,
            content_hash = EXCLUDED.content_hash",
        checkpoint_id = crate::host::devql::esc_pg(&session_meta.checkpoint_id),
        session_id = crate::host::devql::esc_pg(&session_meta.session_id),
        session_index = session_index,
        agent = crate::host::devql::esc_pg(&session_meta.agent),
        turn_id = crate::host::devql::esc_pg(&session_meta.turn_id),
        checkpoints_count = i64::from(session_meta.checkpoints_count),
        is_task = if session_meta.is_task { 1 } else { 0 },
        tool_use_id = crate::host::devql::esc_pg(&session_meta.tool_use_id),
        transcript_identifier_at_start =
            crate::host::devql::esc_pg(&session_meta.transcript_identifier_at_start),
        checkpoint_transcript_start = session_meta.checkpoint_transcript_start,
        initial_attribution = checkpoint_sql_nullable_text(initial_attribution.as_deref()),
        token_usage = checkpoint_sql_nullable_text(token_usage.as_deref()),
        summary = checkpoint_sql_nullable_text(summary.as_deref()),
        author_name = crate::host::devql::esc_pg(author_name),
        author_email = crate::host::devql::esc_pg(author_email),
        transcript_path = crate::host::devql::esc_pg(&session_meta.transcript_path),
        subagent_transcript_path = crate::host::devql::esc_pg(subagent_transcript_path),
        content_hash = crate::host::devql::esc_pg(content_hash),
        created_at = checkpoint_timestamp_sql(&session_meta.created_at, dialect),
    ))
}

pub(crate) fn build_upsert_checkpoint_row_sql(
    repo_id: &str,
    checkpoint_id: &str,
    strategy: &str,
    branch: &str,
    checkpoints_count: u32,
    token_usage: &Option<TokenUsageMetadata>,
    dialect: crate::host::devql::RelationalDialect,
) -> Result<String> {
    let token_usage_json = token_usage
        .as_ref()
        .map(serde_json::to_string)
        .transpose()
        .context("serializing token_usage for checkpoints row")?;
    let now_sql = crate::host::devql::sql_now_for_dialect(dialect);

    Ok(format!(
        "INSERT INTO checkpoints (
            checkpoint_id, repo_id, strategy, branch, cli_version,
            checkpoints_count, token_usage, created_at, updated_at
        ) VALUES (
            '{checkpoint_id}', '{repo_id}', '{strategy}', '{branch}', '{cli_version}',
            {checkpoints_count}, {token_usage}, {now_sql}, {now_sql}
        )
        ON CONFLICT(checkpoint_id) DO UPDATE SET
            repo_id = EXCLUDED.repo_id,
            strategy = EXCLUDED.strategy,
            branch = EXCLUDED.branch,
            cli_version = EXCLUDED.cli_version,
            checkpoints_count = EXCLUDED.checkpoints_count,
            token_usage = EXCLUDED.token_usage,
            updated_at = {now_sql}",
        checkpoint_id = crate::host::devql::esc_pg(checkpoint_id),
        repo_id = crate::host::devql::esc_pg(repo_id),
        strategy = crate::host::devql::esc_pg(strategy),
        branch = crate::host::devql::esc_pg(branch),
        cli_version = crate::host::devql::esc_pg(CLI_VERSION),
        checkpoints_count = i64::from(checkpoints_count),
        token_usage = checkpoint_sql_nullable_text(token_usage_json.as_deref()),
        now_sql = now_sql,
    ))
}

pub(crate) fn build_update_checkpoint_session_summary_sql(
    checkpoint_id: &str,
    session_index: i64,
    summary_json: &str,
) -> String {
    format!(
        "UPDATE checkpoint_sessions
         SET summary = '{summary_json}'
         WHERE checkpoint_id = '{checkpoint_id}' AND session_index = {session_index}",
        summary_json = crate::host::devql::esc_pg(summary_json),
        checkpoint_id = crate::host::devql::esc_pg(checkpoint_id),
        session_index = session_index,
    )
}

pub(crate) fn build_update_checkpoint_session_content_hash_sql(
    checkpoint_id: &str,
    session_index: i64,
    content_hash: &str,
) -> String {
    format!(
        "UPDATE checkpoint_sessions
         SET content_hash = '{content_hash}'
         WHERE checkpoint_id = '{checkpoint_id}' AND session_index = {session_index}",
        content_hash = crate::host::devql::esc_pg(content_hash),
        checkpoint_id = crate::host::devql::esc_pg(checkpoint_id),
        session_index = session_index,
    )
}

pub(crate) fn build_touch_checkpoint_updated_at_sql(
    checkpoint_id: &str,
    dialect: crate::host::devql::RelationalDialect,
) -> String {
    format!(
        "UPDATE checkpoints
         SET updated_at = {}
         WHERE checkpoint_id = '{}'",
        crate::host::devql::sql_now_for_dialect(dialect),
        crate::host::devql::esc_pg(checkpoint_id),
    )
}

pub(crate) fn build_insert_commit_checkpoint_mapping_sql(
    repo_id: &str,
    commit_sha: &str,
    checkpoint_id: &str,
) -> String {
    format!(
        "INSERT INTO commit_checkpoints (commit_sha, checkpoint_id, repo_id)
         VALUES ('{commit_sha}', '{checkpoint_id}', '{repo_id}')
         ON CONFLICT (commit_sha, checkpoint_id) DO NOTHING",
        commit_sha = crate::host::devql::esc_pg(commit_sha),
        checkpoint_id = crate::host::devql::esc_pg(checkpoint_id),
        repo_id = crate::host::devql::esc_pg(repo_id),
    )
}

pub(crate) fn open_checkpoint_storage_context(
    repo_root: &Path,
) -> Result<CheckpointStorageContext> {
    let cfg = crate::config::resolve_store_backend_config_for_repo(repo_root)
        .context("resolving backend config for committed checkpoints")?;
    let (relational, repo_id) = open_checkpoint_relational_store(repo_root)?;
    let sqlite = crate::host::relational_store::RelationalStore::local_sqlite_pool(&relational)
        .context("opening committed checkpoint SQLite database")?;

    let resolved_blob_store =
        crate::storage::blob::create_blob_store_with_backend_for_role_for_repo(
            &cfg.blobs,
            repo_root,
            BlobStorageRole::RuntimeSession,
        )
        .context("initialising runtime-local blob storage for committed checkpoints")?;

    Ok(CheckpointStorageContext {
        sqlite,
        relational,
        blob_store: resolved_blob_store.store,
        blob_backend: resolved_blob_store.backend.to_string(),
        repo_id,
    })
}

pub(crate) fn find_checkpoint_session_index(
    relational: &crate::host::relational_store::DefaultRelationalStore,
    checkpoint_id: &str,
    session_id: &str,
) -> Result<Option<i64>> {
    let sql = format!(
        "SELECT session_index
         FROM checkpoint_sessions
         WHERE checkpoint_id = '{}' AND session_id = '{}'
         ORDER BY session_index ASC
         LIMIT 1",
        crate::host::devql::esc_pg(checkpoint_id),
        crate::host::devql::esc_pg(session_id),
    );
    Ok(query_checkpoint_metadata_rows(relational, &sql)?
        .into_iter()
        .next()
        .and_then(|row| checkpoint_row_i64(&row, "session_index")))
}

pub(crate) fn latest_checkpoint_session_index(
    relational: &crate::host::relational_store::DefaultRelationalStore,
    checkpoint_id: &str,
) -> Result<Option<i64>> {
    let sql = format!(
        "SELECT session_index
         FROM checkpoint_sessions
         WHERE checkpoint_id = '{}'
         ORDER BY session_index DESC
         LIMIT 1",
        crate::host::devql::esc_pg(checkpoint_id),
    );
    Ok(query_checkpoint_metadata_rows(relational, &sql)?
        .into_iter()
        .next()
        .and_then(|row| checkpoint_row_i64(&row, "session_index")))
}

pub(crate) fn resolve_checkpoint_session_index_for_write(
    relational: &crate::host::relational_store::DefaultRelationalStore,
    checkpoint_id: &str,
    session_id: &str,
) -> Result<i64> {
    if let Some(existing) = find_checkpoint_session_index(relational, checkpoint_id, session_id)? {
        return Ok(existing);
    }
    Ok(latest_checkpoint_session_index(relational, checkpoint_id)?.map_or(0, |idx| idx + 1))
}

pub(crate) fn aggregate_checkpoint_metadata_from_db(
    relational: &crate::host::relational_store::DefaultRelationalStore,
    checkpoint_id: &str,
) -> Result<(u32, Option<TokenUsageMetadata>)> {
    let sql = format!(
        "SELECT checkpoints_count, token_usage
         FROM checkpoint_sessions
         WHERE checkpoint_id = '{}'
         ORDER BY session_index ASC",
        crate::host::devql::esc_pg(checkpoint_id),
    );
    let mut checkpoints_total = 0u64;
    let mut token_usage: Option<TokenUsageMetadata> = None;
    for row in query_checkpoint_metadata_rows(relational, &sql)? {
        let count = checkpoint_row_i64(&row, "checkpoints_count").unwrap_or_default();
        checkpoints_total += count.max(0) as u64;
        let parsed_token_usage = checkpoint_row_optional_text(&row, "token_usage")
            .as_deref()
            .and_then(|raw| serde_json::from_str::<TokenUsageMetadata>(raw).ok());
        token_usage = aggregate_token_usage(token_usage, parsed_token_usage);
    }

    Ok((checkpoints_total.min(u32::MAX as u64) as u32, token_usage))
}

pub(crate) fn load_checkpoint_files_touched_from_db(
    storage: &CheckpointStorageContext,
    checkpoint_id: &str,
) -> Result<Vec<String>> {
    let sql = format!(
        "SELECT path_before, path_after
         FROM checkpoint_files
         WHERE repo_id = '{}' AND checkpoint_id = '{}'
         ORDER BY COALESCE(path_after, path_before) ASC, relation_id ASC",
        crate::host::devql::esc_pg(&storage.repo_id),
        crate::host::devql::esc_pg(checkpoint_id),
    );
    let rows = storage
        .relational
        .query_rows_for_role_blocking(
            crate::host::devql::RelationalStorageRole::SharedRelational,
            &sql,
        )
        .context("loading checkpoint files touched for committed checkpoint")?;
    let mut files_touched = std::collections::BTreeSet::new();
    for row in rows {
        let path_before = row.get("path_before").and_then(serde_json::Value::as_str);
        let path_after = row.get("path_after").and_then(serde_json::Value::as_str);
        let display_path = crate::host::devql::checkpoint_provenance::checkpoint_display_path(
            path_before,
            path_after,
        );
        if !display_path.is_empty() {
            files_touched.insert(display_path);
        }
    }
    Ok(files_touched.into_iter().collect())
}

pub(crate) fn upsert_checkpoint_blob(
    storage: &CheckpointStorageContext,
    checkpoint_id: &str,
    session_index: i64,
    blob_type: crate::storage::blob::BlobType,
    payload: &[u8],
) -> Result<String> {
    let key = crate::storage::blob::build_blob_key(
        &storage.repo_id,
        checkpoint_id,
        session_index,
        blob_type,
    );
    storage.blob_store.write(&key, payload).with_context(|| {
        format!(
            "writing {} blob for checkpoint {checkpoint_id}",
            blob_type.as_str()
        )
    })?;

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

pub(crate) fn upsert_checkpoint_session_row(
    storage: &CheckpointStorageContext,
    session_index: i64,
    session_meta: &CommittedMetadata,
    author_name: &str,
    author_email: &str,
    content_hash: &str,
    subagent_transcript_path: &str,
) -> Result<()> {
    let sql = build_upsert_checkpoint_session_row_sql(
        session_index,
        session_meta,
        author_name,
        author_email,
        content_hash,
        subagent_transcript_path,
        storage
            .relational
            .dialect_for_role(crate::host::devql::RelationalStorageRole::SharedRelational),
    )?;
    exec_checkpoint_metadata_statements(&storage.relational, &[sql])
        .context("upserting checkpoint_sessions row")
}

pub(crate) fn upsert_checkpoint_row(
    storage: &CheckpointStorageContext,
    checkpoint_id: &str,
    strategy: &str,
    branch: &str,
    checkpoints_count: u32,
    token_usage: &Option<TokenUsageMetadata>,
) -> Result<()> {
    let sql = build_upsert_checkpoint_row_sql(
        &storage.repo_id,
        checkpoint_id,
        strategy,
        branch,
        checkpoints_count,
        token_usage,
        storage
            .relational
            .dialect_for_role(crate::host::devql::RelationalStorageRole::SharedRelational),
    )?;
    exec_checkpoint_metadata_statements(&storage.relational, &[sql])
        .context("upserting checkpoints row")
}

fn persist_checkpoint_provenance_rows(
    repo_root: &Path,
    storage: &CheckpointStorageContext,
    session_meta: &CommittedMetadata,
) -> Result<()> {
    let Some(commit_sha) = try_head_hash(repo_root)? else {
        return Ok(());
    };
    if commit_sha.trim().is_empty() {
        return Ok(());
    }

    let context = crate::host::devql::checkpoint_provenance::CheckpointProvenanceContext {
        repo_id: &storage.repo_id,
        checkpoint_id: &session_meta.checkpoint_id,
        session_id: &session_meta.session_id,
        event_time: &session_meta.created_at,
        agent: &session_meta.agent,
        branch: &session_meta.branch,
        strategy: &session_meta.strategy,
        commit_sha: &commit_sha,
    };
    let file_rows =
        crate::host::devql::checkpoint_provenance::collect_checkpoint_file_provenance_rows(
            repo_root, context,
        )?;
    let artefact_provenance =
        crate::host::devql::checkpoint_provenance::collect_checkpoint_artefact_provenance(
            repo_root, context, &file_rows,
        )?;
    replace_checkpoint_provenance_rows_for_session(
        &storage.relational,
        &storage.repo_id,
        &session_meta.checkpoint_id,
        &session_meta.session_id,
        &file_rows,
        &artefact_provenance,
    )
    .context("persisting checkpoint provenance rows")
}

fn replace_checkpoint_provenance_rows_for_session(
    relational: &crate::host::relational_store::DefaultRelationalStore,
    repo_id: &str,
    checkpoint_id: &str,
    session_id: &str,
    file_rows: &[crate::host::devql::checkpoint_provenance::CheckpointFileProvenanceRow],
    artefact_provenance:
        &crate::host::devql::checkpoint_provenance::CheckpointArtefactProvenanceBundle,
) -> Result<()> {
    let role = crate::host::devql::RelationalStorageRole::SharedRelational;
    let dialect = relational.dialect_for_role(role);
    let mut statements = Vec::with_capacity(
        3 + file_rows.len()
            + artefact_provenance.semantic_rows.len()
            + artefact_provenance.lineage_rows.len(),
    );
    statements.push(
        crate::host::devql::checkpoint_provenance::delete_checkpoint_artefact_lineage_rows_for_session_sql(
            repo_id,
            checkpoint_id,
            session_id,
        ),
    );
    statements.push(
        crate::host::devql::checkpoint_provenance::delete_checkpoint_artefact_rows_for_session_sql(
            repo_id,
            checkpoint_id,
            session_id,
        ),
    );
    statements.push(
        crate::host::devql::checkpoint_provenance::delete_checkpoint_file_rows_for_session_sql(
            repo_id,
            checkpoint_id,
            session_id,
        ),
    );
    for row in file_rows {
        statements.push(
            crate::host::devql::checkpoint_provenance::build_upsert_checkpoint_file_row_sql(
                row, dialect,
            ),
        );
    }
    for row in &artefact_provenance.semantic_rows {
        statements.push(
            crate::host::devql::checkpoint_provenance::build_upsert_checkpoint_artefact_row_sql(
                row, dialect,
            ),
        );
    }
    for row in &artefact_provenance.lineage_rows {
        statements.push(
            crate::host::devql::checkpoint_provenance::build_upsert_checkpoint_artefact_lineage_row_sql(
                row, dialect,
            ),
        );
    }
    relational
        .exec_batch_transactional_for_role_blocking(role, &statements)
        .context("replacing checkpoint provenance rows for checkpoint session")
}

pub(crate) fn persist_committed_checkpoint_db_and_blobs(
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
        &storage.relational,
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

    persist_checkpoint_provenance_rows(repo_root, &storage, session_meta)?;

    let (checkpoints_count, token_usage) =
        aggregate_checkpoint_metadata_from_db(&storage.relational, &opts.checkpoint_id)?;
    upsert_checkpoint_row(
        &storage,
        &opts.checkpoint_id,
        &opts.strategy,
        &session_meta.branch,
        checkpoints_count,
        &token_usage,
    )
}

pub(crate) fn update_checkpoint_session_summary_in_db(
    repo_root: &Path,
    checkpoint_id: &str,
    summary: &serde_json::Value,
) -> Result<bool> {
    let storage = open_checkpoint_storage_context(repo_root)?;
    let Some(session_index) = latest_checkpoint_session_index(&storage.relational, checkpoint_id)?
    else {
        return Ok(false);
    };

    let summary_json = serde_json::to_string(summary)
        .context("serializing summary for checkpoint_sessions row")?;
    exec_checkpoint_metadata_statements(
        &storage.relational,
        &[
            build_update_checkpoint_session_summary_sql(
                checkpoint_id,
                session_index,
                &summary_json,
            ),
            build_touch_checkpoint_updated_at_sql(
                checkpoint_id,
                storage
                    .relational
                    .dialect_for_role(crate::host::devql::RelationalStorageRole::SharedRelational),
            ),
        ],
    )?;
    Ok(true)
}

pub(crate) fn write_committed(repo_root: &Path, opts: WriteCommittedOptions) -> Result<()> {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_checkpoint_file_row()
    -> crate::host::devql::checkpoint_provenance::CheckpointFileProvenanceRow {
        crate::host::devql::checkpoint_provenance::CheckpointFileProvenanceRow {
            relation_id: "relation-1".to_string(),
            repo_id: "repo-1".to_string(),
            checkpoint_id: "checkpoint-1".to_string(),
            session_id: "session-1".to_string(),
            event_time: "2026-05-18T10:00:00Z".to_string(),
            agent: "claude-code".to_string(),
            branch: "main".to_string(),
            strategy: "manual-commit".to_string(),
            commit_sha: "abc123".to_string(),
            change_kind:
                crate::host::devql::checkpoint_provenance::CheckpointFileChangeKind::Modify,
            path_before: Some("src/lib.rs".to_string()),
            path_after: Some("src/lib.rs".to_string()),
            blob_sha_before: Some("blob-before".to_string()),
            blob_sha_after: Some("blob-after".to_string()),
            copy_source_path: None,
            copy_source_blob_sha: None,
        }
    }

    fn sample_committed_metadata() -> CommittedMetadata {
        CommittedMetadata {
            checkpoint_id: "checkpoint-1".to_string(),
            session_id: "session-1".to_string(),
            checkpoints_count: 2,
            strategy: "manual-commit".to_string(),
            agent: "claude-code".to_string(),
            created_at: "2026-05-18T10:00:00Z".to_string(),
            cli_version: CLI_VERSION.to_string(),
            turn_id: "turn-1".to_string(),
            is_task: false,
            tool_use_id: String::new(),
            transcript_identifier_at_start: "msg-1".to_string(),
            checkpoint_transcript_start: 0,
            transcript_lines_at_start: 0,
            branch: "main".to_string(),
            summary: None,
            token_usage: None,
            initial_attribution: None,
            transcript_path: String::new(),
        }
    }

    #[test]
    fn replace_checkpoint_provenance_rows_for_session_persists_locally_when_shared_role_is_sqlite()
    {
        let tmp = tempfile::tempdir().expect("temp dir");
        let sqlite_path = tmp.path().join("checkpoint-relational.sqlite");
        let sqlite = crate::storage::SqliteConnectionPool::connect(sqlite_path.clone())
            .expect("connect checkpoint sqlite");
        sqlite
            .initialise_relational_checkpoint_schema()
            .expect("initialise checkpoint schema");
        let relational = crate::host::relational_store::DefaultRelationalStore::from_inner(
            crate::host::devql::RelationalStorage::primary_backend_for_tests(
                sqlite_path,
                crate::host::devql::RelationalPrimaryBackend::Sqlite,
            ),
        );

        replace_checkpoint_provenance_rows_for_session(
            &relational,
            "repo-1",
            "checkpoint-1",
            "session-1",
            &[sample_checkpoint_file_row()],
            &crate::host::devql::checkpoint_provenance::CheckpointArtefactProvenanceBundle::default(
            ),
        )
        .expect("local shared role should persist checkpoint provenance rows");

        let local_count = sqlite
            .with_connection(|conn| {
                conn.query_row("SELECT COUNT(*) FROM checkpoint_files", [], |row| {
                    row.get::<_, i64>(0)
                })
                .map_err(anyhow::Error::from)
            })
            .expect("count local checkpoint_files rows");
        assert_eq!(local_count, 1);
    }

    #[test]
    fn replace_checkpoint_provenance_rows_for_session_routes_off_local_sqlite_when_shared_role_is_remote()
     {
        let tmp = tempfile::tempdir().expect("temp dir");
        let sqlite_path = tmp.path().join("checkpoint-relational.sqlite");
        let sqlite = crate::storage::SqliteConnectionPool::connect(sqlite_path.clone())
            .expect("connect checkpoint sqlite");
        sqlite
            .initialise_relational_checkpoint_schema()
            .expect("initialise checkpoint schema");
        let relational = crate::host::relational_store::DefaultRelationalStore::from_inner(
            crate::host::devql::RelationalStorage::primary_backend_for_tests(
                sqlite_path,
                crate::host::devql::RelationalPrimaryBackend::Postgres,
            ),
        );

        let err = replace_checkpoint_provenance_rows_for_session(
            &relational,
            "repo-1",
            "checkpoint-1",
            "session-1",
            &[sample_checkpoint_file_row()],
            &crate::host::devql::checkpoint_provenance::CheckpointArtefactProvenanceBundle::default(
            ),
        )
        .expect_err("remote shared role should not fall back to local checkpoint provenance");
        assert!(
            format!("{err:#}")
                .contains("remote Postgres shared relational backend is configured without a DSN"),
            "expected shared relational remote routing error, got: {err:#}"
        );

        let local_count = sqlite
            .with_connection(|conn| {
                conn.query_row("SELECT COUNT(*) FROM checkpoint_files", [], |row| {
                    row.get::<_, i64>(0)
                })
                .map_err(anyhow::Error::from)
            })
            .expect("count local checkpoint_files rows");
        assert_eq!(local_count, 0);
    }

    #[test]
    fn checkpoint_session_metadata_writes_route_off_local_sqlite_when_shared_role_is_remote() {
        let tmp = tempfile::tempdir().expect("temp dir");
        let sqlite_path = tmp.path().join("checkpoint-relational.sqlite");
        let sqlite = crate::storage::SqliteConnectionPool::connect(sqlite_path.clone())
            .expect("connect checkpoint sqlite");
        sqlite
            .initialise_relational_checkpoint_schema()
            .expect("initialise checkpoint schema");
        let relational = crate::host::relational_store::DefaultRelationalStore::from_inner(
            crate::host::devql::RelationalStorage::primary_backend_for_tests(
                sqlite_path,
                crate::host::devql::RelationalPrimaryBackend::Postgres,
            ),
        );

        let sql = build_upsert_checkpoint_session_row_sql(
            0,
            &sample_committed_metadata(),
            "Test Author",
            "test@example.com",
            "sha256:abc",
            "",
            crate::host::devql::RelationalDialect::Postgres,
        )
        .expect("build checkpoint session upsert SQL");
        let err = exec_checkpoint_metadata_statements(&relational, &[sql])
            .expect_err("remote shared checkpoint metadata should not fall back to local sqlite");
        assert!(
            format!("{err:#}")
                .contains("remote Postgres shared relational backend is configured without a DSN"),
            "expected remote shared metadata routing error, got: {err:#}"
        );

        let local_count = sqlite
            .with_connection(|conn| {
                conn.query_row("SELECT COUNT(*) FROM checkpoint_sessions", [], |row| {
                    row.get::<_, i64>(0)
                })
                .map_err(anyhow::Error::from)
            })
            .expect("count local checkpoint_sessions rows");
        assert_eq!(local_count, 0);
    }

    #[test]
    fn commit_checkpoint_mapping_writes_route_off_local_sqlite_when_shared_role_is_remote() {
        let tmp = tempfile::tempdir().expect("temp dir");
        let sqlite_path = tmp.path().join("checkpoint-relational.sqlite");
        let sqlite = crate::storage::SqliteConnectionPool::connect(sqlite_path.clone())
            .expect("connect checkpoint sqlite");
        sqlite
            .initialise_relational_checkpoint_schema()
            .expect("initialise checkpoint schema");
        let relational = crate::host::relational_store::DefaultRelationalStore::from_inner(
            crate::host::devql::RelationalStorage::primary_backend_for_tests(
                sqlite_path,
                crate::host::devql::RelationalPrimaryBackend::Postgres,
            ),
        );

        let err = exec_checkpoint_metadata_statements(
            &relational,
            &[build_insert_commit_checkpoint_mapping_sql(
                "repo-1",
                "commit-1",
                "checkpoint-1",
            )],
        )
        .expect_err(
            "remote shared commit-checkpoint mappings should not fall back to local sqlite",
        );
        assert!(
            format!("{err:#}")
                .contains("remote Postgres shared relational backend is configured without a DSN"),
            "expected remote shared commit-checkpoint mapping routing error, got: {err:#}"
        );

        let local_count = sqlite
            .with_connection(|conn| {
                conn.query_row("SELECT COUNT(*) FROM commit_checkpoints", [], |row| {
                    row.get::<_, i64>(0)
                })
                .map_err(anyhow::Error::from)
            })
            .expect("count local commit_checkpoints rows");
        assert_eq!(local_count, 0);
    }
}
