// File state, file artefact, and language artefact DB upserts.

struct FileRevision<'a> {
    commit_sha: &'a str,
    commit_unix: i64,
    path: &'a str,
    blob_sha: &'a str,
}

#[derive(Debug, Clone)]
struct PersistedArtefactRecord {
    symbol_id: String,
    artefact_id: String,
    canonical_kind: Option<String>,
    language_kind: String,
    symbol_fqn: String,
    parent_symbol_id: Option<String>,
    parent_artefact_id: Option<String>,
    start_line: i32,
    end_line: i32,
    start_byte: i32,
    end_byte: i32,
    signature: Option<String>,
    modifiers: Vec<String>,
    docstring: Option<String>,
    content_hash: String,
}

#[derive(Debug, Clone)]
struct PersistedEdgeRecord {
    edge_id: String,
    from_symbol_id: String,
    from_artefact_id: String,
    to_symbol_id: Option<String>,
    to_artefact_id: Option<String>,
    to_symbol_ref: Option<String>,
    edge_kind: String,
    language: String,
    start_line: Option<i32>,
    end_line: Option<i32>,
    metadata: Value,
}

async fn upsert_file_state_row(
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

fn sql_nullable_text(value: Option<&str>) -> String {
    value
        .map(|text| format!("'{}'", esc_pg(text)))
        .unwrap_or_else(|| "NULL".to_string())
}

fn sql_json_text_array(relational: &RelationalStorage, values: &[String]) -> String {
    let raw = esc_pg(&serde_json::to_string(values).unwrap_or_else(|_| "[]".to_string()));
    match relational.dialect() {
        RelationalDialect::Postgres => format!("'{raw}'::jsonb"),
        RelationalDialect::Sqlite => format!("'{raw}'"),
    }
}

#[cfg(test)]
fn sql_jsonb_text_array(values: &[String]) -> String {
    format!(
        "'{}'::jsonb",
        esc_pg(&serde_json::to_string(values).unwrap_or_else(|_| "[]".to_string()))
    )
}

fn sql_json_value(relational: &RelationalStorage, value: &Value) -> String {
    let raw = esc_pg(&value.to_string());
    match relational.dialect() {
        RelationalDialect::Postgres => format!("'{raw}'::jsonb"),
        RelationalDialect::Sqlite => format!("'{raw}'"),
    }
}

fn sql_now(relational: &RelationalStorage) -> &'static str {
    match relational.dialect() {
        RelationalDialect::Postgres => "now()",
        RelationalDialect::Sqlite => "datetime('now')",
    }
}

fn is_supported_symbol_language(language: &str) -> bool {
    matches!(language, "typescript" | "javascript" | "rust")
}

fn build_file_artefact_row_from_content(
    repo_id: &str,
    path: &str,
    blob_sha: &str,
    content: Option<&str>,
) -> FileArtefactRow {
    let line_count = content
        .map(|value| value.lines().count() as i32)
        .unwrap_or(1)
        .max(1);
    let byte_count = content
        .map(|value| value.len() as i32)
        .unwrap_or(0)
        .max(0);

    FileArtefactRow {
        artefact_id: revision_artefact_id(repo_id, blob_sha, &file_symbol_id(path)),
        symbol_id: file_symbol_id(path),
        language: detect_language(path),
        end_line: line_count,
        end_byte: byte_count,
    }
}

async fn upsert_file_artefact_row(
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
    let file_docstring = if language == "rust" {
        blob_content
            .as_deref()
            .and_then(extract_rust_file_docstring)
    } else {
        None
    };
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

fn build_file_current_record(
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

fn build_symbol_records(
    cfg: &DevqlConfig,
    path: &str,
    blob_sha: &str,
    file_artefact: &FileArtefactRow,
    items: &[JsTsArtefact],
) -> Vec<PersistedArtefactRecord> {
    let mut out = Vec::with_capacity(items.len());
    let mut symbol_to_artefact_id: HashMap<String, String> = HashMap::new();
    let mut symbol_to_symbol_id: HashMap<String, String> = HashMap::new();
    symbol_to_artefact_id.insert(path.to_string(), file_artefact.artefact_id.clone());
    symbol_to_symbol_id.insert(path.to_string(), file_artefact.symbol_id.clone());

    for item in items {
        let semantic_parent_symbol_id = item
            .parent_symbol_fqn
            .as_ref()
            .and_then(|fqn| symbol_to_symbol_id.get(fqn))
            .map(String::as_str);
        let symbol_id = structural_symbol_id_for_artefact(item, semantic_parent_symbol_id);
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
        let parent_symbol_id = item
            .parent_symbol_fqn
            .as_ref()
            .and_then(|fqn| symbol_to_symbol_id.get(fqn))
            .cloned()
            .or_else(|| Some(file_artefact.symbol_id.clone()));
        let parent_artefact_id = item
            .parent_symbol_fqn
            .as_ref()
            .and_then(|fqn| symbol_to_artefact_id.get(fqn))
            .cloned()
            .or_else(|| Some(file_artefact.artefact_id.clone()));

        out.push(PersistedArtefactRecord {
            symbol_id: symbol_id.clone(),
            artefact_id: artefact_id.clone(),
            canonical_kind: item.canonical_kind.clone(),
            language_kind: item.language_kind.clone(),
            symbol_fqn: item.symbol_fqn.clone(),
            parent_symbol_id,
            parent_artefact_id,
            start_line: item.start_line,
            end_line: item.end_line,
            start_byte: item.start_byte,
            end_byte: item.end_byte,
            signature: Some(item.signature.clone()),
            modifiers: item.modifiers.clone(),
            docstring: item.docstring.clone(),
            content_hash,
        });

        symbol_to_artefact_id.insert(item.symbol_fqn.clone(), artefact_id);
        symbol_to_symbol_id.insert(item.symbol_fqn.clone(), symbol_id);
    }

    out
}

async fn persist_historical_artefact(
    cfg: &DevqlConfig,
    relational: &RelationalStorage,
    path: &str,
    blob_sha: &str,
    language: &str,
    record: &PersistedArtefactRecord,
) -> Result<()> {
    let canonical_kind_sql = sql_nullable_text(record.canonical_kind.as_deref());
    let parent_artefact_sql = sql_nullable_text(record.parent_artefact_id.as_deref());
    let signature_sql = sql_nullable_text(record.signature.as_deref());
    let modifiers_sql = sql_json_text_array(relational, &record.modifiers);
    let docstring_sql = sql_nullable_text(record.docstring.as_deref());
    let sql = format!(
        "INSERT INTO artefacts (artefact_id, symbol_id, repo_id, blob_sha, path, language, canonical_kind, language_kind, symbol_fqn, parent_artefact_id, start_line, end_line, start_byte, end_byte, signature, modifiers, docstring, content_hash) \
VALUES ('{}', '{}', '{}', '{}', '{}', '{}', {}, '{}', '{}', {}, {}, {}, {}, {}, {}, {}, {}, '{}') \
ON CONFLICT (artefact_id) DO UPDATE SET symbol_id = EXCLUDED.symbol_id, repo_id = EXCLUDED.repo_id, blob_sha = EXCLUDED.blob_sha, path = EXCLUDED.path, language = EXCLUDED.language, canonical_kind = EXCLUDED.canonical_kind, language_kind = EXCLUDED.language_kind, symbol_fqn = EXCLUDED.symbol_fqn, parent_artefact_id = EXCLUDED.parent_artefact_id, start_line = EXCLUDED.start_line, end_line = EXCLUDED.end_line, start_byte = EXCLUDED.start_byte, end_byte = EXCLUDED.end_byte, signature = EXCLUDED.signature, modifiers = EXCLUDED.modifiers, docstring = EXCLUDED.docstring, content_hash = EXCLUDED.content_hash",
        esc_pg(&record.artefact_id),
        esc_pg(&record.symbol_id),
        esc_pg(&cfg.repo.repo_id),
        esc_pg(blob_sha),
        esc_pg(path),
        esc_pg(language),
        canonical_kind_sql,
        esc_pg(&record.language_kind),
        esc_pg(&record.symbol_fqn),
        parent_artefact_sql,
        record.start_line,
        record.end_line,
        record.start_byte,
        record.end_byte,
        signature_sql,
        modifiers_sql,
        docstring_sql,
        esc_pg(&record.content_hash),
    );

    relational.exec(&sql).await
}

async fn upsert_current_artefact(
    cfg: &DevqlConfig,
    relational: &RelationalStorage,
    rev: &FileRevision<'_>,
    language: &str,
    record: &PersistedArtefactRecord,
) -> Result<()> {
    let canonical_kind_sql = sql_nullable_text(record.canonical_kind.as_deref());
    let parent_symbol_sql = sql_nullable_text(record.parent_symbol_id.as_deref());
    let parent_artefact_sql = sql_nullable_text(record.parent_artefact_id.as_deref());
    let signature_sql = sql_nullable_text(record.signature.as_deref());
    let modifiers_sql = sql_json_text_array(relational, &record.modifiers);
    let docstring_sql = sql_nullable_text(record.docstring.as_deref());
    let now_sql = sql_now(relational);
    let sql = format!(
        "INSERT INTO artefacts_current (repo_id, symbol_id, artefact_id, commit_sha, blob_sha, path, language, canonical_kind, language_kind, symbol_fqn, parent_symbol_id, parent_artefact_id, start_line, end_line, start_byte, end_byte, signature, modifiers, docstring, content_hash, updated_at) \
VALUES ('{}', '{}', '{}', '{}', '{}', '{}', '{}', {}, '{}', '{}', {}, {}, {}, {}, {}, {}, {}, {}, {}, '{}', {}) \
ON CONFLICT (repo_id, symbol_id) DO UPDATE SET artefact_id = EXCLUDED.artefact_id, commit_sha = EXCLUDED.commit_sha, blob_sha = EXCLUDED.blob_sha, path = EXCLUDED.path, language = EXCLUDED.language, canonical_kind = EXCLUDED.canonical_kind, language_kind = EXCLUDED.language_kind, symbol_fqn = EXCLUDED.symbol_fqn, parent_symbol_id = EXCLUDED.parent_symbol_id, parent_artefact_id = EXCLUDED.parent_artefact_id, start_line = EXCLUDED.start_line, end_line = EXCLUDED.end_line, start_byte = EXCLUDED.start_byte, end_byte = EXCLUDED.end_byte, signature = EXCLUDED.signature, modifiers = EXCLUDED.modifiers, docstring = EXCLUDED.docstring, content_hash = EXCLUDED.content_hash, updated_at = {}",
        esc_pg(&cfg.repo.repo_id),
        esc_pg(&record.symbol_id),
        esc_pg(&record.artefact_id),
        esc_pg(rev.commit_sha),
        esc_pg(rev.blob_sha),
        esc_pg(rev.path),
        esc_pg(language),
        canonical_kind_sql,
        esc_pg(&record.language_kind),
        esc_pg(&record.symbol_fqn),
        parent_symbol_sql,
        parent_artefact_sql,
        record.start_line,
        record.end_line,
        record.start_byte,
        record.end_byte,
        signature_sql,
        modifiers_sql,
        docstring_sql,
        esc_pg(&record.content_hash),
        now_sql,
        now_sql,
    );

    relational.exec(&sql).await
}

fn build_historical_edge_records(
    cfg: &DevqlConfig,
    blob_sha: &str,
    language: &str,
    edges: Vec<JsTsDependencyEdge>,
    current_by_fqn: &HashMap<String, PersistedArtefactRecord>,
) -> Vec<PersistedEdgeRecord> {
    let mut out = Vec::new();

    for edge in edges {
        let Some(from_record) = current_by_fqn.get(&edge.from_symbol_fqn) else {
            continue;
        };

        let resolved_target = edge
            .to_target_symbol_fqn
            .as_ref()
            .and_then(|fqn| current_by_fqn.get(fqn));
        let to_artefact_id = resolved_target.map(|record| record.artefact_id.clone());
        let to_symbol_ref = if resolved_target.is_some() {
            None
        } else {
            edge.to_symbol_ref.clone()
        };
        if to_artefact_id.is_none() && to_symbol_ref.is_none() {
            continue;
        }

        out.push(PersistedEdgeRecord {
            edge_id: deterministic_uuid(&format!(
                "{}|{}|{}|{}|{}|{}|{}|{}",
                cfg.repo.repo_id,
                blob_sha,
                from_record.artefact_id,
                edge.edge_kind.as_str(),
                to_artefact_id.clone().unwrap_or_default(),
                to_symbol_ref.clone().unwrap_or_default(),
                edge.start_line.unwrap_or(-1),
                edge.end_line.unwrap_or(-1)
            )),
            from_symbol_id: from_record.symbol_id.clone(),
            from_artefact_id: from_record.artefact_id.clone(),
            to_symbol_id: resolved_target.map(|record| record.symbol_id.clone()),
            to_artefact_id,
            to_symbol_ref,
            edge_kind: edge.edge_kind.as_str().to_string(),
            language: language.to_string(),
            start_line: edge.start_line,
            end_line: edge.end_line,
            metadata: edge.metadata.to_value(),
        });
    }

    out
}

async fn persist_historical_edge(
    cfg: &DevqlConfig,
    relational: &RelationalStorage,
    blob_sha: &str,
    record: &PersistedEdgeRecord,
) -> Result<()> {
    let to_artefact_sql = sql_nullable_text(record.to_artefact_id.as_deref());
    let to_symbol_sql = sql_nullable_text(record.to_symbol_ref.as_deref());
    let start_line_sql = record
        .start_line
        .map(|value| value.to_string())
        .unwrap_or_else(|| "NULL".to_string());
    let end_line_sql = record
        .end_line
        .map(|value| value.to_string())
        .unwrap_or_else(|| "NULL".to_string());
    let metadata_sql = sql_json_value(relational, &record.metadata);

    let sql = format!(
        "INSERT INTO artefact_edges (edge_id, repo_id, blob_sha, from_artefact_id, to_artefact_id, to_symbol_ref, edge_kind, language, start_line, end_line, metadata) \
VALUES ('{}', '{}', '{}', '{}', {}, {}, '{}', '{}', {}, {}, {}) \
ON CONFLICT (edge_id) DO UPDATE SET repo_id = EXCLUDED.repo_id, blob_sha = EXCLUDED.blob_sha, from_artefact_id = EXCLUDED.from_artefact_id, to_artefact_id = EXCLUDED.to_artefact_id, to_symbol_ref = EXCLUDED.to_symbol_ref, edge_kind = EXCLUDED.edge_kind, language = EXCLUDED.language, start_line = EXCLUDED.start_line, end_line = EXCLUDED.end_line, metadata = EXCLUDED.metadata",
        esc_pg(&record.edge_id),
        esc_pg(&cfg.repo.repo_id),
        esc_pg(blob_sha),
        esc_pg(&record.from_artefact_id),
        to_artefact_sql,
        to_symbol_sql,
        esc_pg(&record.edge_kind),
        esc_pg(&record.language),
        start_line_sql,
        end_line_sql,
        metadata_sql,
    );
    relational.exec(&sql).await
}

async fn load_current_file_state(
    cfg: &DevqlConfig,
    relational: &RelationalStorage,
    path: &str,
) -> Result<Option<(String, i64)>> {
    let committed_at_unix_expr = match relational.dialect() {
        RelationalDialect::Postgres => "EXTRACT(EPOCH FROM committed_at)::BIGINT".to_string(),
        RelationalDialect::Sqlite => "CAST(strftime('%s', committed_at) AS INTEGER)".to_string(),
    };
    let sql = format!(
        "SELECT commit_sha, {} AS committed_at_unix \
FROM current_file_state WHERE repo_id = '{}' AND path = '{}' LIMIT 1",
        committed_at_unix_expr,
        esc_pg(&cfg.repo.repo_id),
        esc_pg(path),
    );
    let rows = relational.query_rows(&sql).await?;
    let Some(row) = rows.first() else {
        return Ok(None);
    };

    let commit_sha = row
        .get("commit_sha")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let committed_at_unix = row
        .get("committed_at_unix")
        .and_then(|value| value.as_i64().or_else(|| value.as_str().and_then(|raw| raw.parse().ok())))
        .unwrap_or_default();
    Ok(Some((commit_sha, committed_at_unix)))
}

fn incoming_revision_is_newer(existing: Option<(String, i64)>, commit_sha: &str, commit_unix: i64) -> bool {
    match existing {
        None => true,
        Some((existing_commit_sha, existing_commit_unix)) => {
            commit_unix > existing_commit_unix
                || (commit_unix == existing_commit_unix && commit_sha > existing_commit_sha.as_str())
        }
    }
}

async fn upsert_current_file_state(
    cfg: &DevqlConfig,
    relational: &RelationalStorage,
    rev: &FileRevision<'_>,
) -> Result<()> {
    let committed_at_sql = match relational.dialect() {
        RelationalDialect::Postgres => format!("to_timestamp({})", rev.commit_unix),
        RelationalDialect::Sqlite => format!("datetime({}, 'unixepoch')", rev.commit_unix),
    };
    let now_sql = sql_now(relational);
    let sql = format!(
        "INSERT INTO current_file_state (repo_id, path, commit_sha, blob_sha, committed_at, updated_at) \
VALUES ('{}', '{}', '{}', '{}', {}, {}) \
ON CONFLICT (repo_id, path) DO UPDATE SET commit_sha = EXCLUDED.commit_sha, blob_sha = EXCLUDED.blob_sha, committed_at = EXCLUDED.committed_at, updated_at = {}",
        esc_pg(&cfg.repo.repo_id),
        esc_pg(rev.path),
        esc_pg(rev.commit_sha),
        esc_pg(rev.blob_sha),
        committed_at_sql,
        now_sql,
        now_sql,
    );
    relational.exec(&sql).await
}

async fn load_current_symbol_ids_for_path(
    cfg: &DevqlConfig,
    relational: &RelationalStorage,
    path: &str,
) -> Result<HashSet<String>> {
    let sql = format!(
        "SELECT symbol_id FROM artefacts_current WHERE repo_id = '{}' AND path = '{}'",
        esc_pg(&cfg.repo.repo_id),
        esc_pg(path),
    );
    let rows = relational.query_rows(&sql).await?;
    Ok(rows
        .into_iter()
        .filter_map(|row| row.get("symbol_id").and_then(Value::as_str).map(str::to_string))
        .collect())
}

async fn delete_current_artefacts_for_path_symbols(
    cfg: &DevqlConfig,
    relational: &RelationalStorage,
    path: &str,
    symbol_ids: &HashSet<String>,
) -> Result<()> {
    if symbol_ids.is_empty() {
        return Ok(());
    }

    let symbol_ids = symbol_ids.iter().cloned().collect::<Vec<_>>();
    let sql = format!(
        "DELETE FROM artefacts_current WHERE repo_id = '{}' AND path = '{}' AND symbol_id IN ({})",
        esc_pg(&cfg.repo.repo_id),
        esc_pg(path),
        sql_string_list_pg(symbol_ids.as_slice()),
    );
    relational.exec(&sql).await
}

async fn delete_current_outgoing_edges_for_path(
    cfg: &DevqlConfig,
    relational: &RelationalStorage,
    path: &str,
) -> Result<()> {
    let sql = format!(
        "DELETE FROM artefact_edges_current WHERE repo_id = '{}' AND path = '{}'",
        esc_pg(&cfg.repo.repo_id),
        esc_pg(path),
    );
    relational.exec(&sql).await
}

async fn load_current_external_target_lookup(
    cfg: &DevqlConfig,
    relational: &RelationalStorage,
    path: &str,
    refs: &HashSet<String>,
) -> Result<HashMap<String, (String, String)>> {
    if refs.is_empty() {
        return Ok(HashMap::new());
    }

    let ref_values = refs.iter().cloned().collect::<Vec<_>>();
    let sql = format!(
        "SELECT symbol_fqn, symbol_id, artefact_id FROM artefacts_current \
WHERE repo_id = '{}' AND path <> '{}' AND symbol_fqn IN ({})",
        esc_pg(&cfg.repo.repo_id),
        esc_pg(path),
        sql_string_list_pg(ref_values.as_slice()),
    );
    let rows = relational.query_rows(&sql).await?;

    let mut out = HashMap::new();
    for row in rows {
        let Some(symbol_fqn) = row.get("symbol_fqn").and_then(Value::as_str) else {
            continue;
        };
        let Some(symbol_id) = row.get("symbol_id").and_then(Value::as_str) else {
            continue;
        };
        let Some(artefact_id) = row.get("artefact_id").and_then(Value::as_str) else {
            continue;
        };
        out.insert(
            symbol_fqn.to_string(),
            (symbol_id.to_string(), artefact_id.to_string()),
        );
    }
    Ok(out)
}

fn build_current_edge_records(
    cfg: &DevqlConfig,
    commit_sha: &str,
    _blob_sha: &str,
    language: &str,
    edges: Vec<JsTsDependencyEdge>,
    current_by_fqn: &HashMap<String, PersistedArtefactRecord>,
    external_targets: &HashMap<String, (String, String)>,
) -> Vec<PersistedEdgeRecord> {
    let mut out = Vec::new();

    for edge in edges {
        let Some(from_record) = current_by_fqn.get(&edge.from_symbol_fqn) else {
            continue;
        };

        let fallback_ref = edge
            .to_symbol_ref
            .clone()
            .or_else(|| edge.to_target_symbol_fqn.clone());
        let resolved_target = edge
            .to_target_symbol_fqn
            .as_ref()
            .and_then(|fqn| current_by_fqn.get(fqn))
            .map(|record| (record.symbol_id.clone(), record.artefact_id.clone()))
            .or_else(|| {
                fallback_ref
                    .as_ref()
                    .and_then(|symbol_ref| external_targets.get(symbol_ref).cloned())
            });

        if resolved_target.is_none() && fallback_ref.is_none() {
            continue;
        }

        let to_symbol_id = resolved_target.as_ref().map(|(symbol_id, _)| symbol_id.clone());
        let to_artefact_id = resolved_target
            .as_ref()
            .map(|(_, artefact_id)| artefact_id.clone());
        let to_symbol_ref = fallback_ref.clone();
        let metadata = edge.metadata.to_value();
        let metadata_key = metadata.to_string();

        out.push(PersistedEdgeRecord {
            edge_id: deterministic_uuid(&format!(
                "{}|{}|{}|{}|{}|{}|{}|{}|{}",
                cfg.repo.repo_id,
                commit_sha,
                from_record.symbol_id,
                edge.edge_kind.as_str(),
                to_symbol_id.clone().unwrap_or_default(),
                to_symbol_ref.clone().unwrap_or_default(),
                edge.start_line.unwrap_or(-1),
                edge.end_line.unwrap_or(-1),
                metadata_key,
            )),
            from_symbol_id: from_record.symbol_id.clone(),
            from_artefact_id: from_record.artefact_id.clone(),
            to_symbol_id,
            to_artefact_id,
            to_symbol_ref,
            edge_kind: edge.edge_kind.as_str().to_string(),
            language: language.to_string(),
            start_line: edge.start_line,
            end_line: edge.end_line,
            metadata,
        });
    }

    out
}

async fn upsert_current_edge(
    cfg: &DevqlConfig,
    relational: &RelationalStorage,
    rev: &FileRevision<'_>,
    record: &PersistedEdgeRecord,
) -> Result<()> {
    let to_symbol_id_sql = sql_nullable_text(record.to_symbol_id.as_deref());
    let to_artefact_sql = sql_nullable_text(record.to_artefact_id.as_deref());
    let to_symbol_ref_sql = sql_nullable_text(record.to_symbol_ref.as_deref());
    let start_line_sql = record
        .start_line
        .map(|value| value.to_string())
        .unwrap_or_else(|| "NULL".to_string());
    let end_line_sql = record
        .end_line
        .map(|value| value.to_string())
        .unwrap_or_else(|| "NULL".to_string());
    let metadata_sql = sql_json_value(relational, &record.metadata);
    let now_sql = sql_now(relational);

    let sql = format!(
        "INSERT INTO artefact_edges_current (edge_id, repo_id, commit_sha, blob_sha, path, from_symbol_id, from_artefact_id, to_symbol_id, to_artefact_id, to_symbol_ref, edge_kind, language, start_line, end_line, metadata, updated_at) \
VALUES ('{}', '{}', '{}', '{}', '{}', '{}', '{}', {}, {}, {}, '{}', '{}', {}, {}, {}, {}) \
ON CONFLICT (edge_id) DO UPDATE SET repo_id = EXCLUDED.repo_id, commit_sha = EXCLUDED.commit_sha, blob_sha = EXCLUDED.blob_sha, path = EXCLUDED.path, from_symbol_id = EXCLUDED.from_symbol_id, from_artefact_id = EXCLUDED.from_artefact_id, to_symbol_id = EXCLUDED.to_symbol_id, to_artefact_id = EXCLUDED.to_artefact_id, to_symbol_ref = EXCLUDED.to_symbol_ref, edge_kind = EXCLUDED.edge_kind, language = EXCLUDED.language, start_line = EXCLUDED.start_line, end_line = EXCLUDED.end_line, metadata = EXCLUDED.metadata, updated_at = {}",
        esc_pg(&record.edge_id),
        esc_pg(&cfg.repo.repo_id),
        esc_pg(rev.commit_sha),
        esc_pg(rev.blob_sha),
        esc_pg(rev.path),
        esc_pg(&record.from_symbol_id),
        esc_pg(&record.from_artefact_id),
        to_symbol_id_sql,
        to_artefact_sql,
        to_symbol_ref_sql,
        esc_pg(&record.edge_kind),
        esc_pg(&record.language),
        start_line_sql,
        end_line_sql,
        metadata_sql,
        now_sql,
        now_sql,
    );
    relational.exec(&sql).await
}

async fn repair_inbound_current_edges(
    cfg: &DevqlConfig,
    relational: &RelationalStorage,
    refreshed_symbol_ids: &HashSet<String>,
    deleted_symbol_ids: &HashSet<String>,
) -> Result<()> {
    let now_sql = sql_now(relational);
    if !refreshed_symbol_ids.is_empty() {
        let refreshed_symbol_ids = refreshed_symbol_ids.iter().cloned().collect::<Vec<_>>();
        let sql = format!(
            "UPDATE artefact_edges_current \
SET to_artefact_id = (
    SELECT a.artefact_id
    FROM artefacts_current a
    WHERE a.repo_id = artefact_edges_current.repo_id
      AND a.symbol_id = artefact_edges_current.to_symbol_id
), updated_at = {} \
WHERE repo_id = '{}' AND to_symbol_id IN ({})",
            now_sql,
            esc_pg(&cfg.repo.repo_id),
            sql_string_list_pg(refreshed_symbol_ids.as_slice()),
        );
        relational.exec(&sql).await?;
    }

    if !deleted_symbol_ids.is_empty() {
        let deleted_symbol_ids = deleted_symbol_ids.iter().cloned().collect::<Vec<_>>();
        let sql = format!(
            "UPDATE artefact_edges_current \
SET to_symbol_id = NULL, to_artefact_id = NULL, updated_at = {} \
WHERE repo_id = '{}' AND to_symbol_id IN ({})",
            now_sql,
            esc_pg(&cfg.repo.repo_id),
            sql_string_list_pg(deleted_symbol_ids.as_slice()),
        );
        relational.exec(&sql).await?;
    }

    Ok(())
}

async fn refresh_current_state_for_path(
    cfg: &DevqlConfig,
    relational: &RelationalStorage,
    rev: &FileRevision<'_>,
    file_artefact: &FileArtefactRow,
    file_docstring: Option<String>,
    symbol_records: &[PersistedArtefactRecord],
    edges: Vec<JsTsDependencyEdge>,
) -> Result<()> {
    let existing = load_current_file_state(cfg, relational, rev.path).await?;
    if !incoming_revision_is_newer(existing, rev.commit_sha, rev.commit_unix) {
        return Ok(());
    }

    let old_symbol_ids = load_current_symbol_ids_for_path(cfg, relational, rev.path).await?;
    upsert_current_file_state(cfg, relational, rev).await?;

    let mut all_records = Vec::with_capacity(symbol_records.len() + 1);
    all_records.push(build_file_current_record(
        rev.path,
        rev.blob_sha,
        file_artefact,
        file_docstring,
    ));
    all_records.extend(symbol_records.iter().cloned());

    let mut current_by_fqn = HashMap::new();
    for record in &all_records {
        upsert_current_artefact(cfg, relational, rev, &file_artefact.language, record).await?;
        current_by_fqn.insert(record.symbol_fqn.clone(), record.clone());
    }

    let new_symbol_ids = all_records
        .iter()
        .map(|record| record.symbol_id.clone())
        .collect::<HashSet<_>>();
    let deleted_symbol_ids = old_symbol_ids
        .difference(&new_symbol_ids)
        .cloned()
        .collect::<HashSet<_>>();

    let target_refs = edges
        .iter()
        .filter_map(|edge| edge.to_symbol_ref.clone().or_else(|| edge.to_target_symbol_fqn.clone()))
        .collect::<HashSet<_>>();
    let external_targets =
        load_current_external_target_lookup(cfg, relational, rev.path, &target_refs).await?;
    let current_edge_records = build_current_edge_records(
        cfg,
        rev.commit_sha,
        rev.blob_sha,
        &file_artefact.language,
        edges,
        &current_by_fqn,
        &external_targets,
    );

    delete_current_outgoing_edges_for_path(cfg, relational, rev.path).await?;
    for record in &current_edge_records {
        upsert_current_edge(cfg, relational, rev, record).await?;
    }

    delete_current_artefacts_for_path_symbols(cfg, relational, rev.path, &deleted_symbol_ids)
        .await?;
    repair_inbound_current_edges(cfg, relational, &new_symbol_ids, &deleted_symbol_ids).await?;
    Ok(())
}

async fn upsert_current_state_for_content(
    cfg: &DevqlConfig,
    relational: &RelationalStorage,
    rev: &FileRevision<'_>,
    content: &str,
) -> Result<()> {
    let file_artefact =
        build_file_artefact_row_from_content(&cfg.repo.repo_id, rev.path, rev.blob_sha, Some(content));

    let (items, dependency_edges, file_docstring) =
        if is_supported_symbol_language(&file_artefact.language) {
            let items = if file_artefact.language == "rust" {
                extract_rust_artefacts(content, rev.path)?
            } else {
                extract_js_ts_artefacts(content, rev.path)?
            };
            let edges = if file_artefact.language == "rust" {
                extract_rust_dependency_edges(content, rev.path, &items)?
            } else {
                extract_js_ts_dependency_edges(content, rev.path, &items)?
            };
            let file_docstring = if file_artefact.language == "rust" {
                extract_rust_file_docstring(content)
            } else {
                None
            };
            (items, edges, file_docstring)
        } else {
            (Vec::new(), Vec::new(), None)
        };

    let symbol_records = build_symbol_records(cfg, rev.path, rev.blob_sha, &file_artefact, &items);
    refresh_current_state_for_path(
        cfg,
        relational,
        rev,
        &file_artefact,
        file_docstring,
        &symbol_records,
        dependency_edges,
    )
    .await
}

async fn delete_current_state_for_path(
    cfg: &DevqlConfig,
    relational: &RelationalStorage,
    path: &str,
) -> Result<()> {
    let deleted_symbol_ids = load_current_symbol_ids_for_path(cfg, relational, path).await?;
    delete_current_outgoing_edges_for_path(cfg, relational, path).await?;

    let sql = format!(
        "DELETE FROM current_file_state WHERE repo_id = '{}' AND path = '{}'",
        esc_pg(&cfg.repo.repo_id),
        esc_pg(path),
    );
    relational.exec(&sql).await?;

    delete_current_artefacts_for_path_symbols(cfg, relational, path, &deleted_symbol_ids).await?;
    repair_inbound_current_edges(cfg, relational, &HashSet::new(), &deleted_symbol_ids).await?;
    Ok(())
}

async fn upsert_language_artefacts(
    cfg: &DevqlConfig,
    relational: &RelationalStorage,
    rev: &FileRevision<'_>,
    file_artefact: &FileArtefactRow,
) -> Result<()> {
    let (items, dependency_edges, file_docstring) = if is_supported_symbol_language(&file_artefact.language) {
        let Some(content) = git_blob_content(&cfg.repo_root, rev.blob_sha) else {
            return Ok(());
        };
        let items = if file_artefact.language == "rust" {
            extract_rust_artefacts(&content, rev.path)?
        } else {
            extract_js_ts_artefacts(&content, rev.path)?
        };
        let edges = if file_artefact.language == "rust" {
            extract_rust_dependency_edges(&content, rev.path, &items)?
        } else {
            extract_js_ts_dependency_edges(&content, rev.path, &items)?
        };
        let file_docstring = if file_artefact.language == "rust" {
            extract_rust_file_docstring(&content)
        } else {
            None
        };
        (items, edges, file_docstring)
    } else {
        (Vec::new(), Vec::new(), None)
    };

    let symbol_records = build_symbol_records(cfg, rev.path, rev.blob_sha, file_artefact, &items);
    for record in &symbol_records {
        persist_historical_artefact(
            cfg,
            relational,
            rev.path,
            rev.blob_sha,
            &file_artefact.language,
            record,
        )
        .await?;
    }

    let mut historical_lookup = HashMap::new();
    historical_lookup.insert(
        rev.path.to_string(),
        build_file_current_record(
            rev.path,
            rev.blob_sha,
            file_artefact,
            file_docstring.clone(),
        ),
    );
    for record in &symbol_records {
        historical_lookup.insert(record.symbol_fqn.clone(), record.clone());
    }
    let historical_edge_records = build_historical_edge_records(
        cfg,
        rev.blob_sha,
        &file_artefact.language,
        dependency_edges.clone(),
        &historical_lookup,
    );
    for record in &historical_edge_records {
        persist_historical_edge(cfg, relational, rev.blob_sha, record).await?;
    }

    refresh_current_state_for_path(
        cfg,
        relational,
        rev,
        file_artefact,
        file_docstring,
        &symbol_records,
        dependency_edges,
    )
    .await?;

    Ok(())
}
