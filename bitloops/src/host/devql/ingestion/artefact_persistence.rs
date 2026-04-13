use super::*;

// Top-level orchestration for historical artefact persistence.

pub(super) type LanguagePackExtraction =
    (Vec<LanguageArtefact>, Vec<DependencyEdge>, Option<String>);

pub(super) fn extract_file_docstring_for_language_pack(
    path: &str,
    language: &str,
    content: &str,
) -> Option<String> {
    let pack_id = resolve_language_pack_owner_for_input(language, Some(path))
        .or_else(|| resolve_language_pack_owner(language))?;
    let registry = language_adapter_registry().ok()?;
    registry.extract_file_docstring(pack_id, content)
}

pub(super) fn extract_language_pack_artefacts_and_edges(
    cfg: &DevqlConfig,
    rev: &FileRevision<'_>,
    language: &str,
    content: &str,
) -> Result<Option<LanguagePackExtraction>> {
    let Some((_context, pack_id)) =
        language_pack_context_for_language(cfg, Some(rev.commit_sha), language, Some(rev.path))
            .with_context(|| format!("resolving language pack owner for `{language}`"))?
    else {
        return Ok(None);
    };

    let registry = language_adapter_registry()?;
    let items = registry.extract_artefacts(pack_id, content, rev.path)?;
    let edges = registry.extract_dependency_edges(pack_id, content, rev.path, &items)?;
    let file_docstring = registry.extract_file_docstring(pack_id, content);
    Ok(Some((items, edges, file_docstring)))
}

pub(super) async fn upsert_language_artefacts(
    cfg: &DevqlConfig,
    relational: &RelationalStorage,
    rev: &FileRevision<'_>,
    file_artefact: &FileArtefactRow,
) -> Result<()> {
    let Some(source_content) = git_blob_content(&cfg.repo_root, rev.blob_sha) else {
        return Ok(());
    };
    let (items, dependency_edges, file_docstring) = extract_language_pack_artefacts_and_edges(
        cfg,
        rev,
        &file_artefact.language,
        &source_content,
    )?
    .unwrap_or_default();

    let symbol_records = build_symbol_records(
        cfg,
        rev.path,
        rev.blob_sha,
        file_artefact,
        &items,
        &source_content,
    );
    let mut historical_sql_batch =
        Vec::with_capacity(symbol_records.len() + dependency_edges.len().saturating_mul(2));
    for record in &symbol_records {
        historical_sql_batch.push(build_upsert_historical_artefact_sql(
            cfg,
            relational,
            rev.path,
            rev.blob_sha,
            &file_artefact.language,
            &file_artefact.extraction_fingerprint,
            record,
        ));
        historical_sql_batch.push(build_upsert_historical_artefact_snapshot_sql(
            &cfg.repo.repo_id,
            rev.blob_sha,
            &HistoricalArtefactSnapshotRecord {
                artefact_id: record.artefact_id.clone(),
                path: rev.path.to_string(),
                parent_artefact_id: record.parent_artefact_id.clone(),
                start_line: record.start_line,
                end_line: record.end_line,
                start_byte: record.start_byte,
                end_byte: record.end_byte,
            },
        ));
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
        historical_sql_batch.push(build_upsert_historical_edge_sql(
            cfg,
            relational,
            rev.blob_sha,
            record,
        ));
    }
    relational
        .exec_batch_transactional(&historical_sql_batch)
        .await?;

    Ok(())
}
