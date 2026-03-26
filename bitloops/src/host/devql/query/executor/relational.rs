use super::*;
use crate::artefact_query_planner::plan_devql_artefact_query;
use anyhow::anyhow;

pub(crate) async fn execute_relational_pipeline(
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

pub(crate) async fn build_relational_artefacts_query(
    cfg: &DevqlConfig,
    events_cfg: &EventsBackendConfig,
    parsed: &ParsedDevqlQuery,
    relational: Option<&RelationalStorage>,
    repo_id: &str,
) -> Result<String> {
    let spec = plan_devql_artefact_query(cfg, repo_id, parsed)?;
    let use_historical_tables = spec.temporal_scope.use_historical_tables();
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

    let mut where_clauses = build_relational_artefact_filters(repo_id, "a", &spec);

    if let Some(activity_filter) = spec.activity_filter.as_ref() {
        let relational = relational.ok_or_else(|| anyhow!("relational storage is required"))?;
        let blob_shas = blob_shas_changed_in_events(
            cfg,
            events_cfg,
            relational,
            repo_id,
            activity_filter.agent.as_deref(),
            activity_filter.since.as_deref(),
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
        "SELECT a.artefact_id, a.symbol_id, a.path, a.canonical_kind, a.language_kind, a.language, a.parent_artefact_id, a.start_line, a.end_line, a.start_byte, a.end_byte, a.signature, a.modifiers, a.docstring, a.blob_sha, a.symbol_fqn, a.content_hash, {} \
FROM {} a \
WHERE {} \
ORDER BY a.path, a.start_line \
LIMIT {}",
        created_at_select,
        artefacts_table,
        where_clauses.join(" AND "),
        spec.pagination
            .as_ref()
            .map_or(1, |pagination| pagination.limit)
    );

    Ok(sql)
}

fn build_relational_artefact_filters(
    repo_id: &str,
    alias: &str,
    spec: &crate::artefact_query_planner::ArtefactQuerySpec,
) -> Vec<String> {
    let mut clauses = vec![format!("{alias}.repo_id = '{}'", esc_pg(repo_id))];
    if !spec.temporal_scope.use_historical_tables() {
        let branch = spec
            .branch
            .as_deref()
            .expect("current/save artefact queries require a branch in the shared spec");
        clauses.push(format!("{alias}.branch = '{}'", esc_pg(branch)));
    }
    if let Some(revision_id) = spec.temporal_scope.save_revision() {
        clauses.push(format!("{alias}.revision_kind = 'temporary'"));
        clauses.push(format!("{alias}.revision_id = '{}'", esc_pg(revision_id)));
    }
    if let Some(commit_sha) = spec.temporal_scope.resolved_commit() {
        clauses.push(file_state_exists_clause(alias, repo_id, commit_sha));
    }
    if let Some(kind) = spec.structural_filter.kind.as_ref() {
        clauses.push(canonical_kind_filter_sql(
            &format!("{alias}.canonical_kind"),
            kind.as_str(),
        ));
    }
    if let Some(symbol_fqn) = spec.structural_filter.symbol_fqn.as_deref() {
        clauses.push(format!("{alias}.symbol_fqn = '{}'", esc_pg(symbol_fqn)));
    }
    if let Some(lines) = spec.structural_filter.lines.as_ref() {
        clauses.push(format!(
            "{alias}.start_line <= {} AND {alias}.end_line >= {}",
            lines.end, lines.start
        ));
    }
    if let Some(path) = spec.scope.path.as_deref() {
        let path_candidates = build_path_candidates(path);
        clauses.push(format!(
            "({})",
            sql_path_candidates_clause(&format!("{alias}.path"), &path_candidates)
        ));
    }
    if let Some(project_path) = spec.scope.project_path.as_deref() {
        clauses.push(repo_path_prefix_clause(
            &format!("{alias}.path"),
            project_path,
        ));
    }
    if let Some(glob) = spec.scope.files_path.as_deref() {
        let like = glob_to_sql_like(glob);
        clauses.push(sql_like_with_escape(&format!("{alias}.path"), &like));
    }

    clauses
}

fn file_state_exists_clause(alias: &str, repo_id: &str, commit_sha: &str) -> String {
    format!(
        "EXISTS (SELECT 1 FROM file_state fs WHERE fs.repo_id = '{repo_id}' \
           AND fs.commit_sha = '{commit_sha}' AND fs.path = {alias}.path AND fs.blob_sha = {alias}.blob_sha)",
        repo_id = esc_pg(repo_id),
        commit_sha = esc_pg(commit_sha),
        alias = alias,
    )
}

fn repo_path_prefix_clause(column: &str, project_path: &str) -> String {
    let prefix = format!("{}/%", escape_like_pattern(project_path));
    format!(
        "({column} = '{path}' OR {like_clause})",
        column = column,
        path = esc_pg(project_path),
        like_clause = sql_like_with_escape(column, &prefix),
    )
}

pub(crate) async fn execute_relational_clones_pipeline(
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

pub(crate) async fn build_relational_clones_query(
    cfg: &DevqlConfig,
    events_cfg: &EventsBackendConfig,
    parsed: &ParsedDevqlQuery,
    relational: &RelationalStorage,
    repo_id: &str,
) -> Result<String> {
    let branch = active_branch_name(&cfg.repo_root);
    let mut source_filters = vec![format!("src.repo_id = '{}'", esc_pg(repo_id))];
    source_filters.push(format!("src.branch = '{}'", esc_pg(&branch)));
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
        source_filters.push(sql_like_with_escape("src.path", &like));
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
JOIN artefacts_current src ON src.repo_id = ce.repo_id AND src.branch = '{}' AND src.symbol_id = ce.source_symbol_id \
JOIN artefacts_current tgt ON tgt.repo_id = ce.repo_id AND tgt.branch = '{}' AND tgt.symbol_id = ce.target_symbol_id \
LEFT JOIN symbol_semantics ss ON ss.artefact_id = tgt.artefact_id \
WHERE {} AND {} \
ORDER BY ce.score DESC, tgt.path, tgt.symbol_fqn \
LIMIT {}",
        esc_pg(&branch),
        esc_pg(&branch),
        clone_filters.join(" AND "),
        source_filters.join(" AND "),
        parsed.limit.max(1),
    ))
}

pub(crate) async fn execute_relational_deps_pipeline(
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
