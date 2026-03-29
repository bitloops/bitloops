use super::*;

// Edge record building, DB row deserialization, and current state queries/mutations.

pub(super) fn build_historical_edge_records(
    cfg: &DevqlConfig,
    blob_sha: &str,
    language: &str,
    edges: Vec<DependencyEdge>,
    current_by_fqn: &HashMap<String, PersistedArtefactRecord>,
) -> Vec<PersistedEdgeRecord> {
    let mut out = Vec::new();
    let provenance = CanonicalProvenanceRef::for_blob(&cfg.repo.repo_id, blob_sha);

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
                "{}|{}|{}|{}|{}|{}|{}",
                provenance.artefact_identity_scope(),
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

#[allow(dead_code)] // Kept for call-site parity during phased migration to batched execution.
pub(super) async fn persist_historical_edge(
    cfg: &DevqlConfig,
    relational: &RelationalStorage,
    blob_sha: &str,
    record: &PersistedEdgeRecord,
) -> Result<()> {
    let sql = build_upsert_historical_edge_sql(cfg, relational, blob_sha, record);
    relational.exec(&sql).await
}

pub(super) fn build_upsert_historical_edge_sql(
    cfg: &DevqlConfig,
    relational: &RelationalStorage,
    blob_sha: &str,
    record: &PersistedEdgeRecord,
) -> String {
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

    format!(
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
    )
}

pub(super) fn current_artefact_state_record_from_row(
    row: &serde_json::Map<String, Value>,
) -> Option<CurrentArtefactStateRecord> {
    let symbol_id = row
        .get("symbol_id")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    if symbol_id.is_empty() {
        return None;
    }
    let symbol_fqn = row
        .get("symbol_fqn")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();

    let record = PersistedArtefactRecord {
        symbol_id: symbol_id.clone(),
        artefact_id: row
            .get("artefact_id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        canonical_kind: row
            .get("canonical_kind")
            .and_then(Value::as_str)
            .map(str::to_string),
        language_kind: row
            .get("language_kind")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        symbol_fqn: symbol_fqn.clone(),
        parent_symbol_id: row
            .get("parent_symbol_id")
            .and_then(Value::as_str)
            .map(str::to_string),
        parent_artefact_id: row
            .get("parent_artefact_id")
            .and_then(Value::as_str)
            .map(str::to_string),
        start_line: parse_required_i32(row.get("start_line")),
        end_line: parse_required_i32(row.get("end_line")),
        start_byte: parse_required_i32(row.get("start_byte")),
        end_byte: parse_required_i32(row.get("end_byte")),
        signature: row
            .get("signature")
            .and_then(Value::as_str)
            .map(str::to_string),
        modifiers: parse_json_array_strings(row.get("modifiers")),
        docstring: row
            .get("docstring")
            .and_then(Value::as_str)
            .map(str::to_string),
        content_hash: row
            .get("content_hash")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
    };

    Some(CurrentArtefactStateRecord {
        record,
        symbol_id,
        symbol_fqn,
    })
}

pub(super) fn current_edge_state_record_from_row(
    row: &serde_json::Map<String, Value>,
) -> Option<CurrentEdgeStateRecord> {
    let edge_id = row
        .get("edge_id")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    if edge_id.is_empty() {
        return None;
    }

    let record = PersistedEdgeRecord {
        edge_id: edge_id.clone(),
        from_symbol_id: row
            .get("from_symbol_id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        from_artefact_id: row
            .get("from_artefact_id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        to_symbol_id: row
            .get("to_symbol_id")
            .and_then(Value::as_str)
            .map(str::to_string),
        to_artefact_id: row
            .get("to_artefact_id")
            .and_then(Value::as_str)
            .map(str::to_string),
        to_symbol_ref: row
            .get("to_symbol_ref")
            .and_then(Value::as_str)
            .map(str::to_string),
        edge_kind: row
            .get("edge_kind")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        language: row
            .get("language")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        start_line: parse_nullable_i32(row.get("start_line")),
        end_line: parse_nullable_i32(row.get("end_line")),
        metadata: parse_json_value_or_default(row.get("metadata"), Value::Object(Map::new())),
    };

    Some(CurrentEdgeStateRecord { edge_id, record })
}

pub(super) fn artefact_payload_equal(
    lhs: &PersistedArtefactRecord,
    rhs: &PersistedArtefactRecord,
) -> bool {
    lhs.symbol_id == rhs.symbol_id
        && lhs.canonical_kind == rhs.canonical_kind
        && lhs.language_kind == rhs.language_kind
        && lhs.symbol_fqn == rhs.symbol_fqn
        && lhs.parent_symbol_id == rhs.parent_symbol_id
        && lhs.start_line == rhs.start_line
        && lhs.end_line == rhs.end_line
        && lhs.start_byte == rhs.start_byte
        && lhs.end_byte == rhs.end_byte
        && lhs.signature == rhs.signature
        && lhs.modifiers == rhs.modifiers
        && lhs.docstring == rhs.docstring
        && lhs.content_hash == rhs.content_hash
}

pub(super) fn edge_payload_equal(lhs: &PersistedEdgeRecord, rhs: &PersistedEdgeRecord) -> bool {
    lhs.from_symbol_id == rhs.from_symbol_id
        && lhs.from_artefact_id == rhs.from_artefact_id
        && lhs.to_symbol_id == rhs.to_symbol_id
        && lhs.to_artefact_id == rhs.to_artefact_id
        && lhs.to_symbol_ref == rhs.to_symbol_ref
        && lhs.edge_kind == rhs.edge_kind
        && lhs.language == rhs.language
        && lhs.start_line == rhs.start_line
        && lhs.end_line == rhs.end_line
        && lhs.metadata == rhs.metadata
}

pub(super) async fn load_current_artefacts_for_path(
    cfg: &DevqlConfig,
    relational: &RelationalStorage,
    branch: &str,
    path: &str,
) -> Result<HashMap<String, CurrentArtefactStateRecord>> {
    let sql = format!(
        "SELECT symbol_id, artefact_id, canonical_kind, language_kind, symbol_fqn, parent_symbol_id, parent_artefact_id, \
start_line, end_line, start_byte, end_byte, signature, modifiers, docstring, content_hash \
FROM artefacts_current WHERE repo_id = '{}' AND branch = '{}' AND path = '{}'",
        esc_pg(&cfg.repo.repo_id),
        esc_pg(branch),
        esc_pg(path),
    );
    let rows = relational.query_rows(&sql).await?;
    let mut out = HashMap::new();
    for row in rows {
        let Some(obj) = row.as_object() else {
            continue;
        };
        let Some(record) = current_artefact_state_record_from_row(obj) else {
            continue;
        };
        out.insert(record.symbol_id.clone(), record);
    }
    Ok(out)
}

pub(super) async fn load_current_outgoing_edges_for_path(
    cfg: &DevqlConfig,
    relational: &RelationalStorage,
    branch: &str,
    path: &str,
) -> Result<HashMap<String, CurrentEdgeStateRecord>> {
    let sql = format!(
        "SELECT edge_id, from_symbol_id, from_artefact_id, to_symbol_id, to_artefact_id, to_symbol_ref, edge_kind, language, start_line, end_line, metadata \
FROM artefact_edges_current WHERE repo_id = '{}' AND branch = '{}' AND path = '{}'",
        esc_pg(&cfg.repo.repo_id),
        esc_pg(branch),
        esc_pg(path),
    );
    let rows = relational.query_rows(&sql).await?;
    let mut out = HashMap::new();
    for row in rows {
        let Some(obj) = row.as_object() else {
            continue;
        };
        let Some(record) = current_edge_state_record_from_row(obj) else {
            continue;
        };
        out.insert(record.edge_id.clone(), record);
    }
    Ok(out)
}

pub(super) async fn delete_current_artefacts_for_path_symbols(
    cfg: &DevqlConfig,
    relational: &RelationalStorage,
    branch: &str,
    path: &str,
    symbol_ids: &HashSet<String>,
) -> Result<()> {
    if symbol_ids.is_empty() {
        return Ok(());
    }

    let symbol_ids = symbol_ids.iter().cloned().collect::<Vec<_>>();
    let sql = format!(
        "DELETE FROM artefacts_current WHERE repo_id = '{}' AND branch = '{}' AND path = '{}' AND symbol_id IN ({})",
        esc_pg(&cfg.repo.repo_id),
        esc_pg(branch),
        esc_pg(path),
        sql_string_list_pg(symbol_ids.as_slice()),
    );
    relational.exec(&sql).await
}

pub(super) async fn delete_current_outgoing_edges_for_path(
    cfg: &DevqlConfig,
    relational: &RelationalStorage,
    branch: &str,
    path: &str,
) -> Result<()> {
    let sql = format!(
        "DELETE FROM artefact_edges_current WHERE repo_id = '{}' AND branch = '{}' AND path = '{}'",
        esc_pg(&cfg.repo.repo_id),
        esc_pg(branch),
        esc_pg(path),
    );
    relational.exec(&sql).await
}

pub(super) async fn delete_current_outgoing_edges_for_ids(
    cfg: &DevqlConfig,
    relational: &RelationalStorage,
    branch: &str,
    edge_ids: &HashSet<String>,
) -> Result<()> {
    if edge_ids.is_empty() {
        return Ok(());
    }

    let edge_ids = edge_ids.iter().cloned().collect::<Vec<_>>();
    let sql = format!(
        "DELETE FROM artefact_edges_current WHERE repo_id = '{}' AND branch = '{}' AND edge_id IN ({})",
        esc_pg(&cfg.repo.repo_id),
        esc_pg(branch),
        sql_string_list_pg(edge_ids.as_slice()),
    );
    relational.exec(&sql).await
}

pub(super) async fn load_current_external_target_lookup(
    cfg: &DevqlConfig,
    relational: &RelationalStorage,
    branch: &str,
    path: &str,
    refs: &HashSet<String>,
) -> Result<HashMap<String, (String, String)>> {
    if refs.is_empty() {
        return Ok(HashMap::new());
    }

    let ref_values = refs.iter().cloned().collect::<Vec<_>>();
    let sql = format!(
        "SELECT symbol_fqn, symbol_id, artefact_id FROM artefacts_current \
WHERE repo_id = '{}' AND branch = '{}' AND path <> '{}' AND symbol_fqn IN ({})",
        esc_pg(&cfg.repo.repo_id),
        esc_pg(branch),
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

pub(super) fn build_current_edge_records(
    cfg: &DevqlConfig,
    path: &str,
    language: &str,
    edges: Vec<DependencyEdge>,
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

        let to_symbol_id = resolved_target
            .as_ref()
            .map(|(symbol_id, _)| symbol_id.clone());
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
                path,
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

pub(super) async fn upsert_current_edge(
    cfg: &DevqlConfig,
    relational: &RelationalStorage,
    rev: &FileRevision<'_>,
    branch: &str,
    record: &PersistedEdgeRecord,
) -> Result<()> {
    let sql = build_upsert_current_edge_sql(cfg, relational, rev, record, branch);
    relational.exec(&sql).await
}

pub(super) fn build_upsert_current_edge_sql(
    cfg: &DevqlConfig,
    relational: &RelationalStorage,
    rev: &FileRevision<'_>,
    record: &PersistedEdgeRecord,
    branch: &str,
) -> String {
    let _temporal_scope = CanonicalProvenanceRef::for_blob(&cfg.repo.repo_id, rev.blob_sha)
        .with_source_anchor(rev.commit_sha, rev.path)
        .temporal_identity_scope();
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
    let temp_checkpoint_id_sql = rev
        .revision
        .temp_checkpoint_id
        .map(|value| value.to_string())
        .unwrap_or_else(|| "NULL".to_string());
    let updated_at_sql = revision_timestamp_sql(relational, rev.commit_unix);

    format!(
        "INSERT INTO artefact_edges_current (edge_id, repo_id, branch, commit_sha, revision_kind, revision_id, temp_checkpoint_id, blob_sha, path, from_symbol_id, from_artefact_id, to_symbol_id, to_artefact_id, to_symbol_ref, edge_kind, language, start_line, end_line, metadata, updated_at) \
VALUES ('{}', '{}', '{}', '{}', '{}', '{}', {}, '{}', '{}', '{}', '{}', {}, {}, {}, '{}', '{}', {}, {}, {}, {}) \
ON CONFLICT (repo_id, branch, edge_id) DO UPDATE SET commit_sha = EXCLUDED.commit_sha, revision_kind = EXCLUDED.revision_kind, revision_id = EXCLUDED.revision_id, temp_checkpoint_id = EXCLUDED.temp_checkpoint_id, blob_sha = EXCLUDED.blob_sha, path = EXCLUDED.path, from_symbol_id = EXCLUDED.from_symbol_id, from_artefact_id = EXCLUDED.from_artefact_id, to_symbol_id = EXCLUDED.to_symbol_id, to_artefact_id = EXCLUDED.to_artefact_id, to_symbol_ref = EXCLUDED.to_symbol_ref, edge_kind = EXCLUDED.edge_kind, language = EXCLUDED.language, start_line = EXCLUDED.start_line, end_line = EXCLUDED.end_line, metadata = EXCLUDED.metadata, updated_at = {}",
        esc_pg(&record.edge_id),
        esc_pg(&cfg.repo.repo_id),
        esc_pg(branch),
        esc_pg(rev.commit_sha),
        esc_pg(rev.revision.kind.as_str()),
        esc_pg(rev.revision.id),
        temp_checkpoint_id_sql,
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
        updated_at_sql,
        updated_at_sql,
    )
}

pub(super) async fn repair_inbound_current_edges(
    cfg: &DevqlConfig,
    relational: &RelationalStorage,
    branch: &str,
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
      AND a.branch = artefact_edges_current.branch
      AND a.symbol_id = artefact_edges_current.to_symbol_id
), updated_at = {} \
WHERE repo_id = '{}' AND branch = '{}' AND to_symbol_id IN ({})",
            now_sql,
            esc_pg(&cfg.repo.repo_id),
            esc_pg(branch),
            sql_string_list_pg(refreshed_symbol_ids.as_slice()),
        );
        relational.exec(&sql).await?;
    }

    if !deleted_symbol_ids.is_empty() {
        let deleted_symbol_ids = deleted_symbol_ids.iter().cloned().collect::<Vec<_>>();
        let sql = format!(
            "UPDATE artefact_edges_current \
SET to_symbol_id = NULL, to_artefact_id = NULL, updated_at = {} \
WHERE repo_id = '{}' AND branch = '{}' AND to_symbol_id IN ({})",
            now_sql,
            esc_pg(&cfg.repo.repo_id),
            esc_pg(branch),
            sql_string_list_pg(deleted_symbol_ids.as_slice()),
        );
        relational.exec(&sql).await?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_cfg() -> DevqlConfig {
        DevqlConfig {
            config_root: PathBuf::from("."),
            repo_root: PathBuf::from("."),
            repo: RepoIdentity {
                provider: "git".to_string(),
                organization: "bitloops".to_string(),
                name: "bitloops".to_string(),
                identity: "git:bitloops/bitloops".to_string(),
                repo_id: "repo-id".to_string(),
            },
            pg_dsn: None,
            clickhouse_url: "http://localhost:8123".to_string(),
            clickhouse_user: None,
            clickhouse_password: None,
            clickhouse_database: "default".to_string(),
            semantic_provider: None,
            semantic_model: None,
            semantic_api_key: None,
            semantic_base_url: None,
            embedding_provider: None,
            embedding_model: None,
            embedding_api_key: None,
        }
    }

    fn sample_edge_record() -> PersistedEdgeRecord {
        PersistedEdgeRecord {
            edge_id: "edge-id".to_string(),
            from_symbol_id: "from-symbol".to_string(),
            from_artefact_id: "from-artefact".to_string(),
            to_symbol_id: Some("to-symbol".to_string()),
            to_artefact_id: Some("to-artefact".to_string()),
            to_symbol_ref: None,
            edge_kind: EDGE_KIND_CALLS.to_string(),
            language: "rust".to_string(),
            start_line: Some(1),
            end_line: Some(2),
            metadata: Value::Object(Map::new()),
        }
    }

    #[test]
    fn build_upsert_historical_edge_sql_targets_historical_table() {
        let cfg = sample_cfg();
        let relational = RelationalStorage::local_only(PathBuf::from("devql.sqlite"));
        let sql =
            build_upsert_historical_edge_sql(&cfg, &relational, "blob-sha", &sample_edge_record());
        assert!(
            sql.contains("INSERT INTO artefact_edges"),
            "historical edge builder should target artefact_edges"
        );
        assert!(
            sql.contains("ON CONFLICT (edge_id) DO UPDATE"),
            "historical edge builder should upsert by edge_id"
        );
    }

    #[test]
    fn build_upsert_current_edge_sql_accepts_branch_argument() {
        let cfg = sample_cfg();
        let relational = RelationalStorage::local_only(PathBuf::from("devql.sqlite"));
        let revision = TemporalRevisionRef {
            kind: TemporalRevisionKind::Commit,
            id: "commit-sha",
            temp_checkpoint_id: None,
        };
        let rev = FileRevision {
            commit_sha: "commit-sha",
            revision,
            commit_unix: 1,
            path: "src/lib.rs",
            blob_sha: "blob-sha",
        };
        let sql =
            build_upsert_current_edge_sql(&cfg, &relational, &rev, &sample_edge_record(), "main");
        assert!(
            sql.contains("INSERT INTO artefact_edges_current"),
            "current edge builder should target artefact_edges_current"
        );
        assert!(
            sql.contains("revision_kind"),
            "current edge builder should persist revision metadata"
        );
    }
}
