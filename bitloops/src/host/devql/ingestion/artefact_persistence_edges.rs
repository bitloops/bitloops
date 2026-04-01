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

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_repo_root() -> PathBuf {
        std::env::temp_dir().join("bitloops-artefact-persistence-edges")
    }

    fn sample_cfg() -> DevqlConfig {
        let repo_root = sample_repo_root();
        DevqlConfig {
            config_root: repo_root.clone(),
            repo_root,
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
            embedding_cache_dir: None,
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
}
