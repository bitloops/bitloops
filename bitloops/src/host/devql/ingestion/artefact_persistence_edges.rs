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
    let targets = current_by_fqn
        .values()
        .map(|record| crate::host::language_adapter::LocalTargetInfo {
            symbol_fqn: record.symbol_fqn.clone(),
            symbol_id: record.symbol_id.clone(),
            artefact_id: record.artefact_id.clone(),
            language_kind: record.language_kind.clone(),
        })
        .collect::<Vec<_>>();
    let import_refs_by_path = edges
        .iter()
        .filter(|edge| edge.edge_kind == EdgeKind::Imports)
        .filter_map(|edge| {
            edge.to_symbol_ref
                .clone()
                .map(|symbol_ref| (edge.from_symbol_fqn.clone(), symbol_ref))
        })
        .fold(
            HashMap::<String, Vec<String>>::new(),
            |mut acc, (from_symbol_fqn, symbol_ref)| {
                let source_path = from_symbol_fqn
                    .split_once("::")
                    .map(|(path, _)| path)
                    .unwrap_or(from_symbol_fqn.as_str())
                    .to_string();
                acc.entry(source_path).or_default().push(symbol_ref);
                acc
            },
        );
    let package_refs_by_path = current_by_fqn
        .values()
        .filter(|record| record.language_kind == "package_declaration")
        .filter_map(|record| {
            let source_path = record.symbol_fqn.split_once("::")?.0.to_string();
            let package_ref = record.symbol_fqn.split_once("::")?.1.to_string();
            Some((source_path, package_ref))
        })
        .fold(
            HashMap::<String, Vec<String>>::new(),
            |mut acc, (path, package_ref)| {
                acc.entry(path).or_default().push(package_ref);
                acc
            },
        );
    let namespace_refs_by_path = current_by_fqn
        .values()
        .filter(|record| {
            matches!(
                record.language_kind.as_str(),
                "namespace_declaration" | "file_scoped_namespace_declaration"
            )
        })
        .filter_map(|record| {
            let source_path = record.symbol_fqn.split_once("::ns::")?.0.to_string();
            let namespace_ref = record.symbol_fqn.split_once("::ns::")?.1.to_string();
            Some((source_path, namespace_ref))
        })
        .fold(
            HashMap::<String, Vec<String>>::new(),
            |mut acc, (path, namespace_ref)| {
                acc.entry(path).or_default().push(namespace_ref);
                acc
            },
        );

    for edge in edges {
        let Some(from_record) = current_by_fqn.get(&edge.from_symbol_fqn) else {
            continue;
        };
        let source_path = from_record
            .symbol_fqn
            .split_once("::")
            .map(|(path, _)| path)
            .unwrap_or(from_record.symbol_fqn.as_str());
        let source_facts = crate::host::language_adapter::LocalSourceFacts {
            import_refs: import_refs_by_path
                .get(source_path)
                .cloned()
                .unwrap_or_default(),
            package_refs: package_refs_by_path
                .get(source_path)
                .cloned()
                .unwrap_or_default(),
            namespace_refs: namespace_refs_by_path
                .get(source_path)
                .cloned()
                .unwrap_or_default(),
        };

        let expanded_edges = expand_historical_edge_symbol_refs(language, source_path, &edge);
        for expanded_edge in expanded_edges {
            let resolved_local = expanded_edge
                .to_symbol_ref
                .as_deref()
                .and_then(|symbol_ref| {
                    crate::host::language_adapter::resolve_local_symbol_ref(
                        language,
                        source_path,
                        expanded_edge.edge_kind.as_str(),
                        symbol_ref,
                        &source_facts,
                        &targets,
                    )
                });
            let resolved_target = expanded_edge
                .to_target_symbol_fqn
                .as_ref()
                .and_then(|fqn| current_by_fqn.get(fqn))
                .or_else(|| {
                    resolved_local
                        .as_ref()
                        .and_then(|resolved| current_by_fqn.get(&resolved.symbol_fqn))
                });
            let to_artefact_id = resolved_target.map(|record| record.artefact_id.clone());
            let to_symbol_ref = if let Some(record) = resolved_target {
                if expanded_edge.to_target_symbol_fqn.is_some() {
                    None
                } else {
                    Some(record.symbol_fqn.clone())
                }
            } else {
                expanded_edge.to_symbol_ref.clone()
            };
            let edge_kind = resolved_local
                .as_ref()
                .map(|resolved| resolved.edge_kind.as_str())
                .unwrap_or(expanded_edge.edge_kind.as_str());
            if to_artefact_id.is_none() && to_symbol_ref.is_none() {
                continue;
            }

            out.push(PersistedEdgeRecord {
                edge_id: deterministic_uuid(&format!(
                    "{}|{}|{}|{}|{}|{}|{}",
                    provenance.artefact_identity_scope(),
                    from_record.artefact_id,
                    edge_kind,
                    to_artefact_id.clone().unwrap_or_default(),
                    to_symbol_ref.clone().unwrap_or_default(),
                    expanded_edge.start_line.unwrap_or(-1),
                    expanded_edge.end_line.unwrap_or(-1)
                )),
                from_symbol_id: from_record.symbol_id.clone(),
                from_artefact_id: from_record.artefact_id.clone(),
                to_symbol_id: resolved_target.map(|record| record.symbol_id.clone()),
                to_artefact_id,
                to_symbol_ref,
                edge_kind: edge_kind.to_string(),
                language: language.to_string(),
                start_line: expanded_edge.start_line,
                end_line: expanded_edge.end_line,
                metadata: expanded_edge.metadata.to_value(),
            });
        }
    }

    out
}

fn expand_historical_edge_symbol_refs(
    language: &str,
    source_path: &str,
    edge: &DependencyEdge,
) -> Vec<DependencyEdge> {
    let Some(symbol_ref) = edge.to_symbol_ref.as_deref() else {
        return vec![edge.clone()];
    };
    let normalized_refs = crate::host::language_adapter::normalize_local_edge_symbol_refs(
        language,
        source_path,
        edge.edge_kind.as_str(),
        symbol_ref,
    );
    if normalized_refs.is_empty() {
        return vec![edge.clone()];
    }

    normalized_refs
        .into_iter()
        .map(|normalized_ref| {
            let mut expanded = edge.clone();
            expanded.to_symbol_ref = Some(normalized_ref);
            expanded
        })
        .collect()
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
            daemon_config_root: repo_root.clone(),
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
