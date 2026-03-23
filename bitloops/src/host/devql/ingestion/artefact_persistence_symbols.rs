use super::*;

// Symbol record building, content hashing, and artefact DB upserts.

pub(super) fn artefact_source_slice<'a>(content: &'a str, item: &JsTsArtefact) -> &'a str {
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

pub(super) fn symbol_content_hash(item: &JsTsArtefact, content: &str) -> String {
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
    blob_sha: &str,
    file_artefact: &FileArtefactRow,
    items: &[JsTsArtefact],
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
        let artefact_id = revision_artefact_id(&cfg.repo.repo_id, blob_sha, &symbol_id);
        let content_hash = symbol_content_hash(item, content);
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

pub(super) async fn persist_historical_artefact(
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

pub(super) async fn upsert_current_artefact(
    cfg: &DevqlConfig,
    relational: &RelationalStorage,
    rev: &FileRevision<'_>,
    language: &str,
    record: &PersistedArtefactRecord,
) -> Result<()> {
    let _temporal_scope = CanonicalProvenanceRef::for_blob(&cfg.repo.repo_id, rev.blob_sha)
        .with_source_anchor(rev.commit_sha, rev.path)
        .temporal_identity_scope();
    let canonical_kind_sql = sql_nullable_text(record.canonical_kind.as_deref());
    let parent_symbol_sql = sql_nullable_text(record.parent_symbol_id.as_deref());
    let parent_artefact_sql = sql_nullable_text(record.parent_artefact_id.as_deref());
    let signature_sql = sql_nullable_text(record.signature.as_deref());
    let modifiers_sql = sql_json_text_array(relational, &record.modifiers);
    let docstring_sql = sql_nullable_text(record.docstring.as_deref());
    let temp_checkpoint_id_sql = rev
        .revision
        .temp_checkpoint_id
        .map(|value| value.to_string())
        .unwrap_or_else(|| "NULL".to_string());
    let updated_at_sql = revision_timestamp_sql(relational, rev.commit_unix);
    let sql = format!(
        "INSERT INTO artefacts_current (repo_id, symbol_id, artefact_id, commit_sha, revision_kind, revision_id, temp_checkpoint_id, blob_sha, path, language, canonical_kind, language_kind, symbol_fqn, parent_symbol_id, parent_artefact_id, start_line, end_line, start_byte, end_byte, signature, modifiers, docstring, content_hash, updated_at) \
VALUES ('{}', '{}', '{}', '{}', '{}', '{}', {}, '{}', '{}', '{}', {}, '{}', '{}', {}, {}, {}, {}, {}, {}, {}, {}, {}, '{}', {}) \
ON CONFLICT (repo_id, symbol_id) DO UPDATE SET artefact_id = EXCLUDED.artefact_id, commit_sha = EXCLUDED.commit_sha, revision_kind = EXCLUDED.revision_kind, revision_id = EXCLUDED.revision_id, temp_checkpoint_id = EXCLUDED.temp_checkpoint_id, blob_sha = EXCLUDED.blob_sha, path = EXCLUDED.path, language = EXCLUDED.language, canonical_kind = EXCLUDED.canonical_kind, language_kind = EXCLUDED.language_kind, symbol_fqn = EXCLUDED.symbol_fqn, parent_symbol_id = EXCLUDED.parent_symbol_id, parent_artefact_id = EXCLUDED.parent_artefact_id, start_line = EXCLUDED.start_line, end_line = EXCLUDED.end_line, start_byte = EXCLUDED.start_byte, end_byte = EXCLUDED.end_byte, signature = EXCLUDED.signature, modifiers = EXCLUDED.modifiers, docstring = EXCLUDED.docstring, content_hash = EXCLUDED.content_hash, updated_at = {}",
        esc_pg(&cfg.repo.repo_id),
        esc_pg(&record.symbol_id),
        esc_pg(&record.artefact_id),
        esc_pg(rev.commit_sha),
        esc_pg(rev.revision.kind.as_str()),
        esc_pg(rev.revision.id),
        temp_checkpoint_id_sql,
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
        updated_at_sql,
        updated_at_sql,
    );

    relational.exec(&sql).await
}
