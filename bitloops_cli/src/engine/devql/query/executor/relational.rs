async fn execute_relational_pipeline(
    cfg: &DevqlConfig,
    events_cfg: &EventsBackendConfig,
    parsed: &ParsedDevqlQuery,
    relational: &RelationalStorage,
) -> Result<Vec<Value>> {
    let repo_id = resolve_repo_id_for_query(cfg, parsed.repo.as_deref());

    if parsed.has_clones_stage {
        return execute_relational_clones_pipeline(cfg, events_cfg, parsed, relational, &repo_id)
            .await;
    }

    if parsed.has_deps_stage {
        return execute_relational_deps_pipeline(cfg, parsed, relational, &repo_id).await;
    }

    if parsed.has_tests_stage {
        return execute_relational_tests_pipeline(cfg, events_cfg, parsed, relational, &repo_id)
            .await;
    }

    let sql = build_relational_artefacts_query(cfg, events_cfg, parsed, Some(relational), &repo_id)
        .await?;
    let rows = relational
        .query_rows(&sql)
        .await?
        .into_iter()
        .map(normalise_relational_result_row)
        .collect::<Vec<_>>();
    if parsed.has_chat_history_stage {
        return attach_chat_history_to_artefacts(cfg, events_cfg, relational, &repo_id, rows).await;
    }
    Ok(rows)
}

async fn build_relational_artefacts_query(
    cfg: &DevqlConfig,
    events_cfg: &EventsBackendConfig,
    parsed: &ParsedDevqlQuery,
    relational: Option<&RelationalStorage>,
    repo_id: &str,
) -> Result<String> {
    let use_historical_tables = parsed.as_of.is_some();
    let artefacts_table = if use_historical_tables {
        "artefacts"
    } else {
        "artefacts_current"
    };
    let created_at_select = if use_historical_tables {
        "a.created_at"
    } else {
        "a.updated_at AS created_at"
    };

    let mut where_clauses = vec![format!("a.repo_id = '{}'", esc_pg(repo_id))];

    if let Some(kind) = parsed.artefacts.kind.as_deref() {
        where_clauses.push(canonical_kind_filter_sql("a.canonical_kind", kind));
    }
    if let Some(symbol_fqn) = parsed.artefacts.symbol_fqn.as_deref() {
        where_clauses.push(format!("a.symbol_fqn = '{}'", esc_pg(symbol_fqn)));
    }

    if let Some((start, end)) = parsed.artefacts.lines {
        where_clauses.push(format!("a.start_line <= {end} AND a.end_line >= {start}"));
    }

    if let Some(path) = parsed.file.as_deref() {
        let path_candidates = build_path_candidates(path);
        if let Some(commit_sha) = resolve_commit_selector(cfg, parsed)? {
            let git_blob = path_candidates.iter().find_map(|candidate| {
                git_blob_sha_at_commit(&cfg.repo_root, &commit_sha, candidate)
            });

            if let Some(blob_sha) = git_blob {
                where_clauses.push(format!(
                    "a.blob_sha = '{}' AND ({})",
                    esc_pg(&blob_sha),
                    sql_path_candidates_clause("a.path", &path_candidates),
                ));
            } else {
                where_clauses.push(format!(
                     "a.blob_sha = (SELECT blob_sha FROM file_state WHERE repo_id = '{}' AND commit_sha = '{}' AND ({}) LIMIT 1) AND ({})",                    esc_pg(repo_id),
                    esc_pg(&commit_sha),
                    sql_path_candidates_clause("path", &path_candidates),
                    sql_path_candidates_clause("a.path", &path_candidates),
                ));
            }
        } else {
            where_clauses.push(format!(
                "({})",
                sql_path_candidates_clause("a.path", &path_candidates)
            ));
        }
    }

    if let Some(glob) = parsed.files_path.as_deref() {
        let like = glob_to_sql_like(glob);
        where_clauses.push(format!("a.path LIKE '{}'", esc_pg(&like)));
    }

    if parsed.artefacts.agent.is_some() || parsed.artefacts.since.is_some() {
        let relational = relational.ok_or_else(|| anyhow!("relational storage is required"))?;
        let blob_shas = blob_shas_changed_in_events(
            cfg,
            events_cfg,
            relational,
            repo_id,
            parsed.artefacts.agent.as_deref(),
            parsed.artefacts.since.as_deref(),
        )
        .await?;
        if blob_shas.is_empty() {
            where_clauses.push("1 = 0".to_string());
        } else {
            where_clauses.push(format!(
                "a.blob_sha IN ({})",
                sql_string_list_pg(&blob_shas)
            ));
        }
    }

    let sql = format!(
        "SELECT a.artefact_id, a.path, a.canonical_kind, a.language_kind, a.language, a.start_line, a.end_line, a.start_byte, a.end_byte, a.signature, a.modifiers, a.docstring, a.blob_sha, a.symbol_fqn, a.content_hash, {} \
FROM {} a \
WHERE {} \
ORDER BY a.path, a.start_line \
LIMIT {}",
        created_at_select,
        artefacts_table,
        where_clauses.join(" AND "),
        parsed.limit.max(1)
    );

    Ok(sql)
}

async fn execute_relational_clones_pipeline(
    cfg: &DevqlConfig,
    events_cfg: &EventsBackendConfig,
    parsed: &ParsedDevqlQuery,
    relational: &RelationalStorage,
    repo_id: &str,
) -> Result<Vec<Value>> {
    let sql = build_relational_clones_query(cfg, events_cfg, parsed, relational, repo_id).await?;
    Ok(relational
        .query_rows(&sql)
        .await?
        .into_iter()
        .map(normalise_relational_result_row)
        .collect::<Vec<_>>())
}

async fn build_relational_clones_query(
    cfg: &DevqlConfig,
    events_cfg: &EventsBackendConfig,
    parsed: &ParsedDevqlQuery,
    relational: &RelationalStorage,
    repo_id: &str,
) -> Result<String> {
    let mut source_filters = vec![format!("src.repo_id = '{}'", esc_pg(repo_id))];
    if let Some(kind) = parsed.artefacts.kind.as_deref() {
        source_filters.push(canonical_kind_filter_sql("src.canonical_kind", kind));
    }
    if let Some(symbol_fqn) = parsed.artefacts.symbol_fqn.as_deref() {
        source_filters.push(format!("src.symbol_fqn = '{}'", esc_pg(symbol_fqn)));
    }
    if let Some((start, end)) = parsed.artefacts.lines {
        source_filters.push(format!(
            "src.start_line <= {end} AND src.end_line >= {start}"
        ));
    }
    if let Some(path) = parsed.file.as_deref() {
        let path_candidates = build_path_candidates(path);
        source_filters.push(format!(
            "({})",
            sql_path_candidates_clause("src.path", &path_candidates)
        ));
    }
    if let Some(glob) = parsed.files_path.as_deref() {
        let like = glob_to_sql_like(glob);
        source_filters.push(format!("src.path LIKE '{}'", esc_pg(&like)));
    }
    if parsed.artefacts.agent.is_some() || parsed.artefacts.since.is_some() {
        let blob_shas = blob_shas_changed_in_events(
            cfg,
            events_cfg,
            relational,
            repo_id,
            parsed.artefacts.agent.as_deref(),
            parsed.artefacts.since.as_deref(),
        )
        .await?;
        if blob_shas.is_empty() {
            source_filters.push("1 = 0".to_string());
        } else {
            source_filters.push(format!(
                "src.blob_sha IN ({})",
                sql_string_list_pg(&blob_shas)
            ));
        }
    }

    let mut clone_filters = vec![format!("ce.repo_id = '{}'", esc_pg(repo_id))];
    if let Some(relation_kind) = parsed.clones.relation_kind.as_deref() {
        clone_filters.push(format!("ce.relation_kind = '{}'", esc_pg(relation_kind)));
    }
    if let Some(min_score) = parsed.clones.min_score {
        clone_filters.push(format!("ce.score >= {}", min_score.clamp(0.0, 1.0)));
    }

    Ok(format!(
        "SELECT ce.relation_kind, ce.score, ce.semantic_score, ce.lexical_score, ce.structural_score, ce.explanation_json, \
src.artefact_id AS source_artefact_id, src.path AS source_path, src.symbol_fqn AS source_symbol_fqn, \
tgt.artefact_id AS target_artefact_id, tgt.path AS target_path, tgt.symbol_fqn AS target_symbol_fqn, \
tgt.canonical_kind AS target_canonical_kind, tgt.language_kind AS target_language_kind, tgt.language AS target_language, \
ss.summary AS target_summary \
FROM symbol_clone_edges ce \
JOIN artefacts_current src ON src.repo_id = ce.repo_id AND src.symbol_id = ce.source_symbol_id \
JOIN artefacts_current tgt ON tgt.repo_id = ce.repo_id AND tgt.symbol_id = ce.target_symbol_id \
LEFT JOIN symbol_semantics ss ON ss.artefact_id = tgt.artefact_id \
WHERE {} AND {} \
ORDER BY ce.score DESC, tgt.path, tgt.symbol_fqn \
LIMIT {}",
        clone_filters.join(" AND "),
        source_filters.join(" AND "),
        parsed.limit.max(1),
    ))
}

async fn execute_relational_deps_pipeline(
    cfg: &DevqlConfig,
    parsed: &ParsedDevqlQuery,
    relational: &RelationalStorage,
    repo_id: &str,
) -> Result<Vec<Value>> {
    let sql = build_relational_deps_query(cfg, parsed, repo_id, relational.dialect())?;
    Ok(relational
        .query_rows(&sql)
        .await?
        .into_iter()
        .map(normalise_relational_result_row)
        .collect::<Vec<_>>())
}

async fn execute_relational_tests_pipeline(
    cfg: &DevqlConfig,
    events_cfg: &EventsBackendConfig,
    parsed: &ParsedDevqlQuery,
    relational: &RelationalStorage,
    repo_id: &str,
) -> Result<Vec<Value>> {
    let artefacts_sql =
        build_relational_artefacts_query(cfg, events_cfg, parsed, Some(relational), repo_id)
            .await?;
    let artefact_rows = relational.query_rows(&artefacts_sql).await?;

    let mut results = Vec::with_capacity(artefact_rows.len());

    for artefact_row in artefact_rows {
        let artefact_row = normalise_relational_result_row(artefact_row);
        let Some(obj) = artefact_row.as_object() else {
            continue;
        };

        let artefact_id = obj
            .get("artefact_id")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if artefact_id.is_empty() {
            continue;
        }

        let artefact_output = json!({
            "artefact_id": artefact_id,
            "name": obj.get("symbol_fqn").and_then(Value::as_str).unwrap_or(artefact_id),
            "kind": obj.get("canonical_kind").and_then(Value::as_str).unwrap_or("unknown"),
            "file_path": obj.get("path").and_then(Value::as_str).unwrap_or(""),
            "start_line": obj.get("start_line").and_then(Value::as_i64).unwrap_or(0),
            "end_line": obj.get("end_line").and_then(Value::as_i64).unwrap_or(0),
        });

        let mut test_conditions = vec![
            format!(
                "tl.production_artefact_id = '{}'",
                esc_pg(artefact_id)
            ),
            format!("tl.repo_id = '{}'", esc_pg(repo_id)),
        ];

        if let Some(min_confidence) = parsed.tests.min_confidence {
            test_conditions.push(format!("tl.confidence >= {}", min_confidence.clamp(0.0, 1.0)));
        }
        if let Some(linkage_source) = parsed.tests.linkage_source.as_deref() {
            test_conditions.push(format!("tl.link_source = '{}'", esc_pg(linkage_source)));
        }

        let tests_sql = format!(
            "SELECT ts.scenario_id AS test_id, ts.name AS test_name, \
            su.name AS suite_name, ts.path AS file_path, \
            tl.confidence, ts.discovery_source, \
            tl.link_source AS linkage_source, tl.linkage_status \
            FROM test_links tl \
            JOIN test_scenarios ts ON ts.scenario_id = tl.test_scenario_id \
            JOIN test_suites su ON su.suite_id = ts.suite_id \
            WHERE {} \
            ORDER BY tl.confidence DESC, ts.path, ts.name \
            LIMIT {}",
            test_conditions.join(" AND "),
            parsed.limit.max(1),
        );

        let covering_tests: Vec<Value> = relational
            .query_rows(&tests_sql)
            .await
            .unwrap_or_default();

        let total_covering_tests = covering_tests.len();

        let summary = json!({
            "total_covering_tests": total_covering_tests,
            "cross_cutting": false,
            "data_sources": ["static_source"],
            "diagnostic_count": 0,
        });

        results.push(json!({
            "artefact": artefact_output,
            "covering_tests": covering_tests,
            "summary": summary,
        }));
    }

    Ok(results)
}
