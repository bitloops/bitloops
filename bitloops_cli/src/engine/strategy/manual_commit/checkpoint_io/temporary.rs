#[derive(Debug, Clone)]
struct TemporaryCheckpointRecord {
    session_id: String,
    tree_hash: String,
    step_number: i64,
    modified_files: Vec<String>,
    new_files: Vec<String>,
    deleted_files: Vec<String>,
    author_name: String,
    author_email: String,
    tool_use_id: Option<String>,
    agent_id: Option<String>,
    is_incremental: bool,
    incremental_sequence: Option<i64>,
    incremental_type: Option<String>,
    incremental_data: Option<String>,
    commit_message: String,
}

fn resolve_temporary_checkpoint_sqlite_path(repo_root: &Path) -> Result<PathBuf> {
    let cfg = crate::store_config::resolve_store_backend_config_for_repo(repo_root)
        .context("resolving backend config for temporary checkpoints")?;
    if let Some(path) = cfg.relational.sqlite_path.as_deref() {
        return crate::store_config::resolve_sqlite_db_path_for_repo(repo_root, Some(path))
            .context("resolving configured SQLite path for temporary checkpoints");
    }

    Ok(paths::default_relational_db_path(repo_root))
}

fn insert_temporary_checkpoint_record(
    repo_root: &Path,
    record: &TemporaryCheckpointRecord,
) -> Result<()> {
    let sqlite_path = resolve_temporary_checkpoint_sqlite_path(repo_root)?;
    let sqlite = crate::engine::db::SqliteConnectionPool::connect_existing(sqlite_path)
        .context("opening temporary checkpoint SQLite database")?;
    sqlite
        .initialise_checkpoint_schema()
        .context("initialising temporary checkpoint schema")?;

    let repo_id = crate::engine::devql::resolve_repo_identity(repo_root)
        .context("resolving repo identity for temporary checkpoints")?
        .repo_id;

    let modified_files = serde_json::to_string(&record.modified_files)
        .context("serialising modified_files for temporary checkpoint row")?;
    let new_files = serde_json::to_string(&record.new_files)
        .context("serialising new_files for temporary checkpoint row")?;
    let deleted_files = serde_json::to_string(&record.deleted_files)
        .context("serialising deleted_files for temporary checkpoint row")?;

    sqlite.with_connection(|conn| {
        conn.execute(
            "INSERT INTO temporary_checkpoints (
                session_id, repo_id, tree_hash, step_number,
                modified_files, new_files, deleted_files,
                author_name, author_email,
                tool_use_id, agent_id, is_incremental,
                incremental_sequence, incremental_type, incremental_data,
                commit_message
            ) VALUES (
                ?1, ?2, ?3, ?4,
                ?5, ?6, ?7,
                ?8, ?9,
                ?10, ?11, ?12,
                ?13, ?14, ?15,
                ?16
            )",
            rusqlite::params![
                record.session_id,
                repo_id,
                record.tree_hash,
                record.step_number,
                modified_files,
                new_files,
                deleted_files,
                record.author_name,
                record.author_email,
                record.tool_use_id.as_deref(),
                record.agent_id.as_deref(),
                if record.is_incremental { 1_i64 } else { 0_i64 },
                record.incremental_sequence,
                record.incremental_type.as_deref(),
                record.incremental_data.as_deref(),
                record.commit_message,
            ],
        )
        .context("inserting temporary checkpoint row")?;
        Ok(())
    })
}

fn latest_temporary_checkpoint_tree_hash(repo_root: &Path, session_id: &str) -> Option<String> {
    use rusqlite::OptionalExtension;

    let sqlite_path = resolve_temporary_checkpoint_sqlite_path(repo_root).ok()?;
    let sqlite = crate::engine::db::SqliteConnectionPool::connect_existing(sqlite_path).ok()?;
    sqlite.initialise_checkpoint_schema().ok()?;
    let repo_id = crate::engine::devql::resolve_repo_identity(repo_root)
        .ok()?
        .repo_id;

    sqlite
        .with_connection(|conn| {
            conn.query_row(
                "SELECT tree_hash
                 FROM temporary_checkpoints
                 WHERE session_id = ?1 AND repo_id = ?2
                 ORDER BY id DESC
                 LIMIT 1",
                rusqlite::params![session_id, repo_id],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(anyhow::Error::from)
        })
        .ok()
        .flatten()
}

fn write_temporary(repo_root: &Path, opts: WriteTemporaryOptions) -> Result<WriteTemporaryResult> {
    if opts.base_commit.is_empty() {
        anyhow::bail!("BaseCommit is required for temporary checkpoint");
    }
    validate_session_id(&opts.session_id)
        .map_err(|err| anyhow::anyhow!("invalid temporary checkpoint options: {err}"))?;
    let latest_tree_hash = latest_temporary_checkpoint_tree_hash(repo_root, &opts.session_id);
    let parent_tree = latest_tree_hash.clone().or_else(|| {
        run_git(
            repo_root,
            &["rev-parse", &format!("{}^{{tree}}", opts.base_commit)],
        )
        .ok()
    });

    let (mut status_modified, mut status_new, mut status_deleted) = if opts.is_first_checkpoint {
        working_tree_changes(repo_root)?
    } else {
        (vec![], vec![], vec![])
    };
    status_modified.extend(opts.modified_files.clone());
    status_new.extend(opts.new_files.clone());
    status_deleted.extend(opts.deleted_files.clone());

    let mut modified_set: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    let mut new_set: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    let mut deleted_set: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for file in status_modified {
        if !file.is_empty() {
            modified_set.insert(file);
        }
    }
    for file in status_new {
        if !file.is_empty() {
            new_set.insert(file);
        }
    }
    for file in status_deleted {
        if !file.is_empty() {
            deleted_set.insert(file);
        }
    }

    let parent_tree =
        parent_tree.ok_or_else(|| anyhow::anyhow!("failed to resolve base tree for checkpoint"))?;
    let resolved_modified_files = modified_set.into_iter().collect::<Vec<_>>();
    let resolved_new_files = new_set.into_iter().collect::<Vec<_>>();
    let resolved_deleted_files = deleted_set.into_iter().collect::<Vec<_>>();
    let tree = build_tree(
        repo_root,
        Some(parent_tree.as_str()),
        &resolved_modified_files,
        &resolved_new_files,
        &resolved_deleted_files,
    )?;
    let metadata_entries = if !opts.metadata_dir_abs.is_empty() && !opts.metadata_dir.is_empty() {
        copy_metadata_dir(Path::new(&opts.metadata_dir_abs), &opts.metadata_dir)?
    } else {
        BTreeMap::new()
    };
    let mut tree = tree;
    if !metadata_entries.is_empty() {
        let staging_dir = repo_root
            .join(paths::BITLOOPS_TMP_DIR)
            .join(format!("temp-metadata-{}", uuid::Uuid::new_v4().simple()));
        fs::create_dir_all(&staging_dir).context("creating temporary metadata staging dir")?;

        let mut file_pairs: Vec<(PathBuf, String)> = Vec::new();
        for (idx, (tree_path, content)) in metadata_entries.into_iter().enumerate() {
            let disk_path = staging_dir.join(format!("metadata-{idx}.txt"));
            fs::write(&disk_path, content)
                .with_context(|| format!("writing staged metadata file {tree_path}"))?;
            file_pairs.push((disk_path, tree_path));
        }
        let result = build_tree_with_explicit_paths(repo_root, Some(&tree), &file_pairs);
        let _ = fs::remove_dir_all(&staging_dir);
        tree = result?;
    }

    if latest_tree_hash.as_deref() == Some(tree.as_str()) {
        return Ok(WriteTemporaryResult {
            skipped: true,
            commit_hash: latest_tree_hash.unwrap_or_default(),
        });
    }

    insert_temporary_checkpoint_record(
        repo_root,
        &TemporaryCheckpointRecord {
            session_id: opts.session_id,
            tree_hash: tree.clone(),
            step_number: i64::from(opts.step_number),
            modified_files: resolved_modified_files,
            new_files: resolved_new_files,
            deleted_files: resolved_deleted_files,
            author_name: opts.author_name,
            author_email: opts.author_email,
            tool_use_id: None,
            agent_id: None,
            is_incremental: false,
            incremental_sequence: None,
            incremental_type: None,
            incremental_data: None,
            commit_message: opts.commit_message,
        },
    )?;

    Ok(WriteTemporaryResult {
        skipped: false,
        commit_hash: tree,
    })
}

fn write_temporary_task(
    repo_root: &Path,
    opts: WriteTemporaryTaskOptions,
) -> Result<WriteTemporaryResult> {
    if opts.base_commit.is_empty() {
        anyhow::bail!("BaseCommit is required for task checkpoint");
    }
    validate_session_id(&opts.session_id)
        .map_err(|err| anyhow::anyhow!("invalid task checkpoint options: {err}"))?;
    validate_tool_use_id(&opts.tool_use_id)
        .map_err(|err| anyhow::anyhow!("invalid task checkpoint options: {err}"))?;
    validate_agent_id(&opts.agent_id)
        .map_err(|err| anyhow::anyhow!("invalid task checkpoint options: {err}"))?;
    let parent_tree = latest_temporary_checkpoint_tree_hash(repo_root, &opts.session_id)
        .or_else(|| {
            run_git(
                repo_root,
                &["rev-parse", &format!("{}^{{tree}}", opts.base_commit)],
            )
            .ok()
        })
        .ok_or_else(|| anyhow::anyhow!("failed to resolve base tree for task checkpoint"))?;

    let mut tree = build_tree(
        repo_root,
        Some(parent_tree.as_str()),
        &opts.modified_files,
        &opts.new_files,
        &opts.deleted_files,
    )?;

    let session_metadata_dir = paths::session_metadata_dir_from_session_id(&opts.session_id);
    let task_metadata_dir = format!("{session_metadata_dir}/tasks/{}", opts.tool_use_id);
    let staging_dir = repo_root
        .join(paths::BITLOOPS_TMP_DIR)
        .join(format!("task-metadata-{}", uuid::Uuid::new_v4().simple()));
    fs::create_dir_all(&staging_dir).context("creating task metadata staging directory")?;
    let mut file_pairs: Vec<(PathBuf, String)> = Vec::new();

    if opts.is_incremental {
        let data_value = if opts.incremental_data.trim().is_empty() {
            serde_json::Value::Null
        } else if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&opts.incremental_data)
        {
            redact_json_value(&parsed)
        } else {
            serde_json::Value::String(redact_text(&opts.incremental_data))
        };
        let incremental_payload = serde_json::json!({
            "type": opts.incremental_type,
            "tool_use_id": opts.tool_use_id,
            "timestamp": now_rfc3339(),
            "data": data_value,
        });
        let incremental_file = staging_dir.join("incremental-checkpoint.json");
        fs::write(
            &incremental_file,
            serde_json::to_string_pretty(&incremental_payload)
                .context("serializing incremental task checkpoint payload")?,
        )
        .context("writing incremental task checkpoint payload")?;
        let checkpoint_name = format!("{:03}-{}.json", opts.incremental_sequence, opts.tool_use_id);
        file_pairs.push((
            incremental_file,
            format!("{task_metadata_dir}/checkpoints/{checkpoint_name}"),
        ));
    } else {
        if !opts.transcript_path.trim().is_empty()
            && let Ok(content) = fs::read(&opts.transcript_path)
        {
            let transcript_file = staging_dir.join(paths::TRANSCRIPT_FILE_NAME);
            fs::write(&transcript_file, redact_jsonl_bytes_with_fallback(&content))
                .context("writing redacted task session transcript")?;
            file_pairs.push((
                transcript_file,
                format!("{session_metadata_dir}/{}", paths::TRANSCRIPT_FILE_NAME),
            ));
        }

        let checkpoint_payload = serde_json::json!({
            "session_id": opts.session_id,
            "tool_use_id": opts.tool_use_id,
            "checkpoint_uuid": opts.checkpoint_uuid,
            "agent_id": opts.agent_id,
        });
        let checkpoint_file = staging_dir.join(paths::CHECKPOINT_FILE_NAME);
        fs::write(
            &checkpoint_file,
            serde_json::to_string_pretty(&checkpoint_payload)
                .context("serializing task checkpoint payload")?,
        )
        .context("writing task checkpoint payload")?;
        file_pairs.push((
            checkpoint_file,
            format!("{task_metadata_dir}/{}", paths::CHECKPOINT_FILE_NAME),
        ));

        if !opts.subagent_transcript_path.is_empty()
            && !opts.agent_id.is_empty()
            && let Ok(content) = fs::read(&opts.subagent_transcript_path)
        {
            let agent_file = staging_dir.join("subagent-transcript.jsonl");
            fs::write(&agent_file, redact_jsonl_bytes_with_fallback(&content))
                .context("writing redacted task subagent transcript")?;
            file_pairs.push((
                agent_file,
                format!("{task_metadata_dir}/agent-{}.jsonl", opts.agent_id),
            ));
        }
    }

    if !file_pairs.is_empty() {
        let result = build_tree_with_explicit_paths(repo_root, Some(&tree), &file_pairs);
        let _ = fs::remove_dir_all(&staging_dir);
        tree = result?;
    } else {
        let _ = fs::remove_dir_all(&staging_dir);
    }

    insert_temporary_checkpoint_record(
        repo_root,
        &TemporaryCheckpointRecord {
            session_id: opts.session_id,
            tree_hash: tree.clone(),
            step_number: i64::from(opts.step_number),
            modified_files: opts.modified_files,
            new_files: opts.new_files,
            deleted_files: opts.deleted_files,
            author_name: opts.author_name,
            author_email: opts.author_email,
            tool_use_id: if opts.tool_use_id.trim().is_empty() {
                None
            } else {
                Some(opts.tool_use_id)
            },
            agent_id: if opts.agent_id.trim().is_empty() {
                None
            } else {
                Some(opts.agent_id)
            },
            is_incremental: opts.is_incremental,
            incremental_sequence: if opts.is_incremental {
                Some(i64::from(opts.incremental_sequence))
            } else {
                None
            },
            incremental_type: if opts.incremental_type.trim().is_empty() {
                None
            } else {
                Some(opts.incremental_type)
            },
            incremental_data: if opts.incremental_data.trim().is_empty() {
                None
            } else {
                Some(opts.incremental_data)
            },
            commit_message: opts.commit_message,
        },
    )?;

    Ok(WriteTemporaryResult {
        skipped: false,
        commit_hash: tree,
    })
}
