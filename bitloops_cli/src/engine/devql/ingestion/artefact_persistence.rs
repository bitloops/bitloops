// Top-level orchestration: refresh/upsert/delete current state and persist language artefacts.

async fn refresh_current_state_for_path(
    cfg: &DevqlConfig,
    relational: &RelationalStorage,
    rev: &FileRevision<'_>,
    file_artefact: &FileArtefactRow,
    file_docstring: Option<String>,
    symbol_records: &[PersistedArtefactRecord],
    edges: Vec<JsTsDependencyEdge>,
) -> Result<()> {
    let existing = load_current_file_revision(cfg, relational, rev.path).await?;
    if !incoming_revision_is_newer(
        existing.as_ref(),
        rev.revision.kind,
        rev.revision.id,
        rev.commit_unix,
    ) {
        return Ok(());
    }
    if existing
        .as_ref()
        .is_some_and(|state| state.blob_sha == rev.blob_sha)
    {
        overwrite_current_revision_metadata_for_path(cfg, relational, rev).await?;
        return Ok(());
    }

    let old_symbol_records = load_current_artefacts_for_path(cfg, relational, rev.path).await?;
    let old_symbol_ids = old_symbol_records.keys().cloned().collect::<HashSet<_>>();

    let mut all_records = Vec::with_capacity(symbol_records.len() + 1);
    all_records.push(build_file_current_record(
        rev.path,
        rev.blob_sha,
        file_artefact,
        file_docstring,
    ));
    all_records.extend(symbol_records.iter().cloned());

    let mut current_by_fqn = HashMap::new();
    let mut refreshed_symbol_ids = HashSet::new();
    for record in &all_records {
        let unchanged = old_symbol_records
            .get(&record.symbol_id)
            .map(|state| artefact_payload_equal(&state.record, record))
            .unwrap_or(false);
        if unchanged {
            if let Some(state) = old_symbol_records.get(&record.symbol_id) {
                current_by_fqn.insert(state.symbol_fqn.clone(), state.record.clone());
            }
            continue;
        }
        upsert_current_artefact(cfg, relational, rev, &file_artefact.language, record).await?;
        refreshed_symbol_ids.insert(record.symbol_id.clone());
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
        .filter_map(|edge| {
            edge.to_symbol_ref
                .clone()
                .or_else(|| edge.to_target_symbol_fqn.clone())
        })
        .collect::<HashSet<_>>();
    let external_targets =
        load_current_external_target_lookup(cfg, relational, rev.path, &target_refs).await?;
    let current_edge_records = build_current_edge_records(
        cfg,
        rev.path,
        &file_artefact.language,
        edges,
        &current_by_fqn,
        &external_targets,
    );

    let old_edges = load_current_outgoing_edges_for_path(cfg, relational, rev.path).await?;
    let next_edge_ids = current_edge_records
        .iter()
        .map(|record| record.edge_id.clone())
        .collect::<HashSet<_>>();
    let old_edge_ids = old_edges.keys().cloned().collect::<HashSet<_>>();
    let deleted_edge_ids = old_edge_ids
        .difference(&next_edge_ids)
        .cloned()
        .collect::<HashSet<_>>();
    delete_current_outgoing_edges_for_ids(cfg, relational, &deleted_edge_ids).await?;
    for record in current_edge_records {
        let unchanged = old_edges
            .get(&record.edge_id)
            .map(|existing| edge_payload_equal(&existing.record, &record))
            .unwrap_or(false);
        if unchanged {
            continue;
        }
        upsert_current_edge(cfg, relational, rev, &record).await?;
    }

    delete_current_artefacts_for_path_symbols(cfg, relational, rev.path, &deleted_symbol_ids)
        .await?;
    repair_inbound_current_edges(cfg, relational, &refreshed_symbol_ids, &deleted_symbol_ids)
        .await?;
    Ok(())
}

async fn upsert_current_state_for_content(
    cfg: &DevqlConfig,
    relational: &RelationalStorage,
    rev: &FileRevision<'_>,
    content: &str,
) -> Result<()> {
    let file_artefact = build_file_artefact_row_from_content(
        &cfg.repo.repo_id,
        rev.path,
        rev.blob_sha,
        Some(content),
    );

    let (items, dependency_edges, file_docstring) = if is_supported_symbol_language(
        &file_artefact.language,
    ) {
        let extraction =
            || -> Result<(Vec<JsTsArtefact>, Vec<JsTsDependencyEdge>, Option<String>)> {
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
                Ok((items, edges, file_docstring))
            };

        match extraction() {
            Ok(value) => value,
            Err(err) => {
                log::warn!(
                    "devql watcher extraction failed for `{}`; keeping file-level current state only: {err:#}",
                    rev.path
                );
                (
                    Vec::new(),
                    Vec::new(),
                    if file_artefact.language == "rust" {
                        extract_rust_file_docstring(content)
                    } else {
                        None
                    },
                )
            }
        }
    } else {
        (Vec::new(), Vec::new(), None)
    };

    let symbol_records =
        build_symbol_records(cfg, rev.path, rev.blob_sha, &file_artefact, &items, content);
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
    let deleted_symbol_ids = load_current_artefacts_for_path(cfg, relational, path)
        .await?
        .keys()
        .cloned()
        .collect::<HashSet<_>>();
    delete_current_outgoing_edges_for_path(cfg, relational, path).await?;

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
    let (items, dependency_edges, file_docstring, source_content) =
        if is_supported_symbol_language(&file_artefact.language) {
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
            (items, edges, file_docstring, content)
        } else {
            (Vec::new(), Vec::new(), None, String::new())
        };

    let symbol_records =
        build_symbol_records(cfg, rev.path, rev.blob_sha, file_artefact, &items, &source_content);
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
