use super::*;

// Top-level orchestration: refresh/upsert/delete current state and persist language artefacts.

pub(super) type LanguagePackArtefactExtractor = fn(&str, &str) -> Result<Vec<JsTsArtefact>>;
pub(super) type LanguagePackDependencyEdgeExtractor =
    fn(&str, &str, &[JsTsArtefact]) -> Result<Vec<JsTsDependencyEdge>>;
pub(super) type LanguagePackFileDocstringExtractor = fn(&str) -> Option<String>;
pub(super) type LanguagePackExtraction =
    (Vec<JsTsArtefact>, Vec<JsTsDependencyEdge>, Option<String>);

// First-party runtime adapter for built-in language packs.
#[derive(Debug, Clone, Copy)]
pub(super) struct BuiltInLanguagePackRuntime {
    pub(super) extract_artefacts: LanguagePackArtefactExtractor,
    pub(super) extract_dependency_edges: LanguagePackDependencyEdgeExtractor,
    pub(super) extract_file_docstring: LanguagePackFileDocstringExtractor,
}

pub(super) fn no_file_docstring(_: &str) -> Option<String> {
    None
}

// Runtime registry keyed by the host-registered language-pack descriptor id.
pub(super) fn built_in_language_pack_registry()
-> &'static HashMap<&'static str, BuiltInLanguagePackRuntime> {
    static BUILT_IN_LANGUAGE_PACKS: OnceLock<HashMap<&'static str, BuiltInLanguagePackRuntime>> =
        OnceLock::new();
    BUILT_IN_LANGUAGE_PACKS.get_or_init(|| {
        HashMap::from([
            (
                RUST_LANGUAGE_PACK_ID,
                BuiltInLanguagePackRuntime {
                    extract_artefacts: extract_rust_artefacts,
                    extract_dependency_edges: extract_rust_dependency_edges,
                    extract_file_docstring: extract_rust_file_docstring,
                },
            ),
            (
                TS_JS_LANGUAGE_PACK_ID,
                BuiltInLanguagePackRuntime {
                    extract_artefacts: extract_js_ts_artefacts,
                    extract_dependency_edges: extract_js_ts_dependency_edges,
                    extract_file_docstring: no_file_docstring,
                },
            ),
        ])
    })
}

pub(super) fn resolve_built_in_language_pack(pack_id: &str) -> Option<BuiltInLanguagePackRuntime> {
    built_in_language_pack_registry().get(pack_id).copied()
}

pub(super) fn resolve_built_in_language_pack_for_source(
    path: &str,
    language: &str,
) -> Option<BuiltInLanguagePackRuntime> {
    resolve_language_pack_owner_for_input(language, Some(path))
        .or_else(|| resolve_language_pack_owner(language))
        .and_then(resolve_built_in_language_pack)
}

pub(super) fn extract_file_docstring_for_language_pack(
    path: &str,
    language: &str,
    content: &str,
) -> Option<String> {
    resolve_built_in_language_pack_for_source(path, language)
        .and_then(|pack| (pack.extract_file_docstring)(content))
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

    let Some(pack) = resolve_built_in_language_pack(pack_id) else {
        bail!("language `{language}` resolved to unsupported language pack `{pack_id}`");
    };

    let items = (pack.extract_artefacts)(content, rev.path)?;
    let edges = (pack.extract_dependency_edges)(content, rev.path, &items)?;
    let file_docstring = (pack.extract_file_docstring)(content);
    Ok(Some((items, edges, file_docstring)))
}

pub(super) async fn refresh_current_state_for_path(
    cfg: &DevqlConfig,
    relational: &RelationalStorage,
    rev: &FileRevision<'_>,
    file_artefact: &FileArtefactRow,
    file_docstring: Option<String>,
    symbol_records: &[PersistedArtefactRecord],
    edges: Vec<JsTsDependencyEdge>,
) -> Result<()> {
    let branch = active_branch_name(&cfg.repo_root);
    let existing = load_current_file_revision(cfg, relational, &branch, rev.path).await?;
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
        overwrite_current_revision_metadata_for_path(cfg, relational, &branch, rev).await?;
        return Ok(());
    }

    let old_symbol_records =
        load_current_artefacts_for_path(cfg, relational, &branch, rev.path).await?;
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
        upsert_current_artefact(
            cfg,
            relational,
            rev,
            &file_artefact.language,
            &branch,
            record,
        )
        .await?;
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
        load_current_external_target_lookup(cfg, relational, &branch, rev.path, &target_refs)
            .await?;
    let current_edge_records = build_current_edge_records(
        cfg,
        rev.path,
        &file_artefact.language,
        edges,
        &current_by_fqn,
        &external_targets,
    );

    let old_edges =
        load_current_outgoing_edges_for_path(cfg, relational, &branch, rev.path).await?;
    let next_edge_ids = current_edge_records
        .iter()
        .map(|record| record.edge_id.clone())
        .collect::<HashSet<_>>();
    let old_edge_ids = old_edges.keys().cloned().collect::<HashSet<_>>();
    let deleted_edge_ids = old_edge_ids
        .difference(&next_edge_ids)
        .cloned()
        .collect::<HashSet<_>>();
    delete_current_outgoing_edges_for_ids(cfg, relational, &branch, &deleted_edge_ids).await?;
    for record in current_edge_records {
        let unchanged = old_edges
            .get(&record.edge_id)
            .map(|existing| edge_payload_equal(&existing.record, &record))
            .unwrap_or(false);
        if unchanged {
            continue;
        }
        upsert_current_edge(cfg, relational, rev, &branch, &record).await?;
    }

    delete_current_artefacts_for_path_symbols(
        cfg,
        relational,
        &branch,
        rev.path,
        &deleted_symbol_ids,
    )
    .await?;
    repair_inbound_current_edges(
        cfg,
        relational,
        &branch,
        &refreshed_symbol_ids,
        &deleted_symbol_ids,
    )
    .await?;
    Ok(())
}

pub(super) async fn upsert_current_state_for_content(
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

    let (items, dependency_edges, file_docstring) = match extract_language_pack_artefacts_and_edges(
        cfg,
        rev,
        &file_artefact.language,
        content,
    ) {
        Ok(Some(value)) => value,
        Ok(None) => (Vec::new(), Vec::new(), None),
        Err(err) => {
            log::warn!(
                "devql watcher extraction failed for `{}`; keeping file-level current state only: {err:#}",
                rev.path
            );
            (
                Vec::new(),
                Vec::new(),
                extract_file_docstring_for_language_pack(
                    rev.path,
                    &file_artefact.language,
                    content,
                ),
            )
        }
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

pub(super) async fn delete_current_state_for_path(
    cfg: &DevqlConfig,
    relational: &RelationalStorage,
    path: &str,
) -> Result<()> {
    let branch = active_branch_name(&cfg.repo_root);
    let deleted_symbol_ids = load_current_artefacts_for_path(cfg, relational, &branch, path)
        .await?
        .keys()
        .cloned()
        .collect::<HashSet<_>>();
    delete_current_outgoing_edges_for_path(cfg, relational, &branch, path).await?;

    delete_current_artefacts_for_path_symbols(cfg, relational, &branch, path, &deleted_symbol_ids)
        .await?;
    repair_inbound_current_edges(
        cfg,
        relational,
        &branch,
        &HashSet::new(),
        &deleted_symbol_ids,
    )
    .await?;
    Ok(())
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
            record,
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
