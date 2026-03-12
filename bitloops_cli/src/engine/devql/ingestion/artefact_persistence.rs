// File state, file artefact, and language artefact DB upserts.

async fn upsert_file_state_row(
    cfg: &DevqlConfig,
    pg_client: &tokio_postgres::Client,
    commit_sha: &str,
    path: &str,
    blob_sha: &str,
) -> Result<()> {
    let sql = format!(
        "INSERT INTO file_state (repo_id, commit_sha, path, blob_sha) VALUES ('{}', '{}', '{}', '{}') \
ON CONFLICT (repo_id, commit_sha, path) DO UPDATE SET blob_sha = EXCLUDED.blob_sha",
        esc_pg(&cfg.repo.repo_id),
        esc_pg(commit_sha),
        esc_pg(path),
        esc_pg(blob_sha),
    );

    postgres_exec(pg_client, &sql).await
}

fn sql_nullable_text(value: Option<&str>) -> String {
    value
        .map(|text| format!("'{}'", esc_pg(text)))
        .unwrap_or_else(|| "NULL".to_string())
}

async fn upsert_file_artefact_row(
    cfg: &DevqlConfig,
    pg_client: &tokio_postgres::Client,
    path: &str,
    blob_sha: &str,
) -> Result<FileArtefactRow> {
    let symbol_id = file_symbol_id(path);
    let artefact_id = revision_artefact_id(&cfg.repo.repo_id, blob_sha, &symbol_id);
    let language = detect_language(path);
    let line_count = git_blob_line_count(&cfg.repo_root, blob_sha)
        .unwrap_or(1)
        .max(1);
    let byte_count = git_blob_content(&cfg.repo_root, blob_sha)
        .map(|content| content.len() as i32)
        .unwrap_or(0)
        .max(0);

    let sql = format!(
        "INSERT INTO artefacts (artefact_id, symbol_id, repo_id, blob_sha, path, language, canonical_kind, language_kind, symbol_fqn, parent_artefact_id, start_line, end_line, start_byte, end_byte, signature, content_hash) \
VALUES ('{}', '{}', '{}', '{}', '{}', '{}', 'file', 'file', '{}', NULL, 1, {}, 0, {}, NULL, '{}') \
ON CONFLICT (artefact_id) DO UPDATE SET symbol_id = EXCLUDED.symbol_id, repo_id = EXCLUDED.repo_id, blob_sha = EXCLUDED.blob_sha, path = EXCLUDED.path, language = EXCLUDED.language, canonical_kind = EXCLUDED.canonical_kind, language_kind = EXCLUDED.language_kind, symbol_fqn = EXCLUDED.symbol_fqn, start_line = EXCLUDED.start_line, end_line = EXCLUDED.end_line, start_byte = EXCLUDED.start_byte, end_byte = EXCLUDED.end_byte, signature = EXCLUDED.signature, content_hash = EXCLUDED.content_hash",
        esc_pg(&artefact_id),
        esc_pg(&symbol_id),
        esc_pg(&cfg.repo.repo_id),
        esc_pg(blob_sha),
        esc_pg(path),
        esc_pg(&language),
        esc_pg(path),
        line_count,
        byte_count,
        esc_pg(blob_sha),
    );

    postgres_exec(pg_client, &sql).await?;
    Ok(FileArtefactRow {
        artefact_id,
        symbol_id,
        language,
    })
}

async fn upsert_language_artefacts(
    cfg: &DevqlConfig,
    pg_client: &tokio_postgres::Client,
    path: &str,
    blob_sha: &str,
    file_artefact: &FileArtefactRow,
) -> Result<()> {
    if file_artefact.language != "typescript"
        && file_artefact.language != "javascript"
        && file_artefact.language != "rust"
    {
        return Ok(());
    }

    let Some(content) = git_blob_content(&cfg.repo_root, blob_sha) else {
        return Ok(());
    };

    let items = if file_artefact.language == "rust" {
        extract_rust_artefacts(&content, path)?
    } else {
        extract_js_ts_artefacts(&content, path)?
    };
    let mut symbol_to_artefact_id: HashMap<String, String> = HashMap::new();
    let mut symbol_to_symbol_id: HashMap<String, String> = HashMap::new();
    symbol_to_artefact_id.insert(path.to_string(), file_artefact.artefact_id.clone());
    symbol_to_symbol_id.insert(path.to_string(), file_artefact.symbol_id.clone());

    for item in &items {
        let parent_symbol_id = item
            .parent_symbol_fqn
            .as_ref()
            .and_then(|fqn| symbol_to_symbol_id.get(fqn))
            .map(String::as_str);
        let symbol_id = semantic_symbol_id_for_artefact(item, parent_symbol_id);
        let artefact_id = revision_artefact_id(&cfg.repo.repo_id, blob_sha, &symbol_id);
        let content_hash = deterministic_uuid(&format!(
            "{}|{}|{}|{}|{}|{}",
            blob_sha,
            path,
            item.canonical_kind.as_deref().unwrap_or("<null>"),
            item.name,
            item.start_line,
            item.end_line
        ));
        let parent_artefact_id = item
            .parent_symbol_fqn
            .as_ref()
            .and_then(|fqn| symbol_to_artefact_id.get(fqn))
            .cloned()
            .unwrap_or_else(|| file_artefact.artefact_id.clone());

        let canonical_kind_sql = sql_nullable_text(item.canonical_kind.as_deref());
        let sql = format!(
            "INSERT INTO artefacts (artefact_id, symbol_id, repo_id, blob_sha, path, language, canonical_kind, language_kind, symbol_fqn, parent_artefact_id, start_line, end_line, start_byte, end_byte, signature, content_hash) \
VALUES ('{}', '{}', '{}', '{}', '{}', '{}', {}, '{}', '{}', '{}', {}, {}, {}, {}, '{}', '{}') \
ON CONFLICT (artefact_id) DO UPDATE SET symbol_id = EXCLUDED.symbol_id, repo_id = EXCLUDED.repo_id, blob_sha = EXCLUDED.blob_sha, path = EXCLUDED.path, language = EXCLUDED.language, canonical_kind = EXCLUDED.canonical_kind, language_kind = EXCLUDED.language_kind, symbol_fqn = EXCLUDED.symbol_fqn, parent_artefact_id = EXCLUDED.parent_artefact_id, start_line = EXCLUDED.start_line, end_line = EXCLUDED.end_line, start_byte = EXCLUDED.start_byte, end_byte = EXCLUDED.end_byte, signature = EXCLUDED.signature, content_hash = EXCLUDED.content_hash",
            esc_pg(&artefact_id),
            esc_pg(&symbol_id),
            esc_pg(&cfg.repo.repo_id),
            esc_pg(blob_sha),
            esc_pg(path),
            esc_pg(&file_artefact.language),
            canonical_kind_sql,
            esc_pg(&item.language_kind),
            esc_pg(&item.symbol_fqn),
            esc_pg(&parent_artefact_id),
            item.start_line,
            item.end_line,
            item.start_byte,
            item.end_byte,
            esc_pg(&item.signature),
            esc_pg(&content_hash),
        );

        postgres_exec(pg_client, &sql).await?;
        symbol_to_artefact_id.insert(item.symbol_fqn.clone(), artefact_id);
        symbol_to_symbol_id.insert(item.symbol_fqn.clone(), symbol_id);
    }

    let edges = if file_artefact.language == "rust" {
        extract_rust_dependency_edges(&content, path, &items)?
    } else {
        extract_js_ts_dependency_edges(&content, path, &items)?
    };
    for edge in edges {
        let Some(from_artefact_id) = symbol_to_artefact_id.get(&edge.from_symbol_fqn).cloned() else {
            continue;
        };

        let to_artefact_id = edge
            .to_target_symbol_fqn
            .as_ref()
            .and_then(|fqn| symbol_to_artefact_id.get(fqn))
            .cloned();
        let to_symbol_ref = if to_artefact_id.is_some() {
            None
        } else {
            edge.to_symbol_ref.clone()
        };

        if to_artefact_id.is_none() && to_symbol_ref.is_none() {
            continue;
        }

        let edge_id = deterministic_uuid(&format!(
            "{}|{}|{}|{}|{}|{}|{}|{}",
            cfg.repo.repo_id,
            blob_sha,
            from_artefact_id,
            edge.edge_kind,
            to_artefact_id.clone().unwrap_or_default(),
            to_symbol_ref.clone().unwrap_or_default(),
            edge.start_line.unwrap_or(-1),
            edge.end_line.unwrap_or(-1)
        ));

        let to_artefact_sql = to_artefact_id
            .as_ref()
            .map(|id| format!("'{}'", esc_pg(id)))
            .unwrap_or_else(|| "NULL".to_string());
        let to_symbol_sql = to_symbol_ref
            .as_ref()
            .map(|s| format!("'{}'", esc_pg(s)))
            .unwrap_or_else(|| "NULL".to_string());
        let start_line_sql = edge
            .start_line
            .map(|v| v.to_string())
            .unwrap_or_else(|| "NULL".to_string());
        let end_line_sql = edge
            .end_line
            .map(|v| v.to_string())
            .unwrap_or_else(|| "NULL".to_string());
        let metadata_sql = format!("'{}'::jsonb", esc_pg(&edge.metadata.to_string()));

        let sql = format!(
            "INSERT INTO artefact_edges (edge_id, repo_id, blob_sha, from_artefact_id, to_artefact_id, to_symbol_ref, edge_kind, language, start_line, end_line, metadata) \
VALUES ('{}', '{}', '{}', '{}', {}, {}, '{}', '{}', {}, {}, {}) \
ON CONFLICT (edge_id) DO UPDATE SET repo_id = EXCLUDED.repo_id, blob_sha = EXCLUDED.blob_sha, from_artefact_id = EXCLUDED.from_artefact_id, to_artefact_id = EXCLUDED.to_artefact_id, to_symbol_ref = EXCLUDED.to_symbol_ref, edge_kind = EXCLUDED.edge_kind, language = EXCLUDED.language, start_line = EXCLUDED.start_line, end_line = EXCLUDED.end_line, metadata = EXCLUDED.metadata",
            esc_pg(&edge_id),
            esc_pg(&cfg.repo.repo_id),
            esc_pg(blob_sha),
            esc_pg(&from_artefact_id),
            to_artefact_sql,
            to_symbol_sql,
            esc_pg(&edge.edge_kind),
            esc_pg(&file_artefact.language),
            start_line_sql,
            end_line_sql,
            metadata_sql,
        );
        postgres_exec(pg_client, &sql).await?;
    }

    Ok(())
}
