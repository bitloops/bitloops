use super::*;

// Symbol record building, content hashing, and artefact DB upserts.

pub(crate) fn artefact_source_slice<'a>(content: &'a str, item: &LanguageArtefact) -> &'a str {
    let len = content.len();
    let start = usize::try_from(item.start_byte)
        .unwrap_or_default()
        .min(len);
    let end = usize::try_from(item.end_byte).unwrap_or_default().min(len);
    if start >= end {
        return "";
    }

    content.get(start..end).unwrap_or("")
}

pub(super) fn symbol_content_hash(item: &LanguageArtefact, content: &str) -> String {
    deterministic_uuid(&format!(
        "{}|{}|{}|{}|{}|{}",
        item.canonical_kind.as_deref().unwrap_or("<null>"),
        item.language_kind,
        item.signature,
        serde_json::to_string(&item.modifiers).unwrap_or_else(|_| "[]".to_string()),
        item.docstring.as_deref().unwrap_or(""),
        artefact_source_slice(content, item)
    ))
}

pub(super) fn build_symbol_records(
    cfg: &DevqlConfig,
    path: &str,
    _blob_sha: &str,
    file_artefact: &FileArtefactRow,
    items: &[LanguageArtefact],
    content: &str,
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
        let content_hash = symbol_content_hash(item, content);
        let artefact_id =
            historical_symbol_artefact_id(&cfg.repo.repo_id, &symbol_id, &content_hash);
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
            language_kind: item.language_kind.to_string(),
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

#[allow(dead_code)] // Kept for call-site parity during phased migration to batched execution.
pub(super) async fn persist_historical_artefact(
    cfg: &DevqlConfig,
    relational: &RelationalStorage,
    path: &str,
    blob_sha: &str,
    language: &str,
    record: &PersistedArtefactRecord,
) -> Result<()> {
    let sql =
        build_upsert_historical_artefact_sql(cfg, relational, path, blob_sha, language, record);
    relational.exec(&sql).await
}

pub(super) fn build_upsert_historical_artefact_sql(
    cfg: &DevqlConfig,
    relational: &RelationalStorage,
    _path: &str,
    _blob_sha: &str,
    language: &str,
    record: &PersistedArtefactRecord,
) -> String {
    let canonical_kind_sql = sql_nullable_text(record.canonical_kind.as_deref());
    let signature_sql = sql_nullable_text(record.signature.as_deref());
    let modifiers_sql = sql_json_text_array(relational, &record.modifiers);
    let docstring_sql = sql_nullable_text(record.docstring.as_deref());
    format!(
        "INSERT INTO artefacts (artefact_id, symbol_id, repo_id, language, canonical_kind, language_kind, symbol_fqn, signature, modifiers, docstring, content_hash) \
VALUES ('{}', '{}', '{}', '{}', {}, '{}', '{}', {}, {}, {}, '{}') \
ON CONFLICT (artefact_id) DO UPDATE SET symbol_id = EXCLUDED.symbol_id, repo_id = EXCLUDED.repo_id, language = EXCLUDED.language, canonical_kind = EXCLUDED.canonical_kind, language_kind = EXCLUDED.language_kind, symbol_fqn = EXCLUDED.symbol_fqn, signature = EXCLUDED.signature, modifiers = EXCLUDED.modifiers, docstring = EXCLUDED.docstring, content_hash = EXCLUDED.content_hash",
        esc_pg(&record.artefact_id),
        esc_pg(&record.symbol_id),
        esc_pg(&cfg.repo.repo_id),
        esc_pg(language),
        canonical_kind_sql,
        esc_pg(&record.language_kind),
        esc_pg(&record.symbol_fqn),
        signature_sql,
        modifiers_sql,
        docstring_sql,
        esc_pg(&record.content_hash),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_repo_root() -> PathBuf {
        std::env::temp_dir().join("bitloops-artefact-persistence-symbols")
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

    fn sample_record() -> PersistedArtefactRecord {
        PersistedArtefactRecord {
            symbol_id: "symbol-id".to_string(),
            artefact_id: "artefact-id".to_string(),
            canonical_kind: Some("function".to_string()),
            language_kind: "function_item".to_string(),
            symbol_fqn: "src/lib.rs::name".to_string(),
            parent_symbol_id: Some("parent-symbol-id".to_string()),
            parent_artefact_id: Some("parent-artefact-id".to_string()),
            start_line: 1,
            end_line: 2,
            start_byte: 0,
            end_byte: 16,
            signature: Some("fn name()".to_string()),
            modifiers: vec!["pub".to_string()],
            docstring: Some("docs".to_string()),
            content_hash: "content-hash".to_string(),
        }
    }

    #[test]
    fn build_upsert_historical_artefact_sql_targets_historical_table() {
        let cfg = sample_cfg();
        let relational = RelationalStorage::local_only(PathBuf::from("devql.sqlite"));
        let sql = build_upsert_historical_artefact_sql(
            &cfg,
            &relational,
            "src/lib.rs",
            "blob-sha",
            "rust",
            &sample_record(),
        );
        assert!(
            sql.contains("INSERT INTO artefacts"),
            "historical builder should insert into artefacts"
        );
        assert!(
            sql.contains("ON CONFLICT (artefact_id) DO UPDATE"),
            "historical builder should upsert on artefact_id"
        );
    }
}
