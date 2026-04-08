use super::*;
use crate::artefact_query_planner::plan_devql_artefact_query;
use crate::host::devql::artefact_sql::{
    build_filtered_artefacts_cte_sql, build_filtered_artefacts_select_sql,
};

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
    _events_cfg: &EventsBackendConfig,
    parsed: &ParsedDevqlQuery,
    _relational: Option<&RelationalStorage>,
    repo_id: &str,
) -> Result<String> {
    let spec = plan_devql_artefact_query(cfg, repo_id, parsed)?;
    Ok(format!(
        "{} LIMIT {}",
        build_filtered_artefacts_select_sql(&spec),
        spec.pagination
            .as_ref()
            .map_or(1, |pagination| pagination.limit)
    ))
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
    _events_cfg: &EventsBackendConfig,
    parsed: &ParsedDevqlQuery,
    _relational: &RelationalStorage,
    repo_id: &str,
) -> Result<String> {
    let spec = plan_devql_artefact_query(cfg, repo_id, parsed)?;
    let filtered_cte = build_filtered_artefacts_cte_sql(&spec);

    let mut clone_filters = vec![format!("ce.repo_id = '{}'", esc_pg(repo_id))];
    if let Some(relation_kind) = parsed.clones.relation_kind.as_deref() {
        clone_filters.push(format!("ce.relation_kind = '{}'", esc_pg(relation_kind)));
    }
    if let Some(min_score) = parsed.clones.min_score {
        clone_filters.push(format!("ce.score >= {}", min_score.clamp(0.0, 1.0)));
    }

    let sql = format!(
        "{filtered_cte} \
SELECT ce.relation_kind, ce.score, ce.semantic_score, ce.lexical_score, ce.structural_score, ce.explanation_json, \
src.artefact_id AS source_artefact_id, src.path AS source_path, src.symbol_fqn AS source_symbol_fqn, \
tgt.artefact_id AS target_artefact_id, tgt.path AS target_path, tgt.symbol_fqn AS target_symbol_fqn, \
src.start_line AS source_start_line, src.end_line AS source_end_line, \
tgt.start_line AS target_start_line, tgt.end_line AS target_end_line, \
tgt.canonical_kind AS target_canonical_kind, tgt.language_kind AS target_language_kind, tgt.language AS target_language, \
ss.summary AS target_summary \
FROM symbol_clone_edges ce \
JOIN filtered src ON src.symbol_id = ce.source_symbol_id AND src.artefact_id = ce.source_artefact_id \
JOIN artefacts_current tgt ON tgt.repo_id = ce.repo_id AND tgt.symbol_id = ce.target_symbol_id \
LEFT JOIN symbol_semantics ss ON ss.artefact_id = tgt.artefact_id \
WHERE {} \
ORDER BY ce.score DESC, tgt.path, tgt.symbol_fqn",
        clone_filters.join(" AND "),
        filtered_cte = filtered_cte,
    );

    Ok(match has_registered_clone_summary_stage(parsed) {
        true => sql,
        false => format!("{sql} LIMIT {}", parsed.limit.max(1)),
    })
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
