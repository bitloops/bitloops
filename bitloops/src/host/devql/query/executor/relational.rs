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
    if let Some(neighbors) = parsed.clones.neighbors {
        return execute_relational_clones_neighbors_override(
            cfg, events_cfg, parsed, relational, repo_id, neighbors,
        )
        .await;
    }

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
    let use_historical_tables = spec.temporal_scope.use_historical_tables();
    let clone_edges_table = if use_historical_tables {
        "symbol_clone_edges"
    } else {
        "symbol_clone_edges_current"
    };
    let target_artefacts_table = if use_historical_tables {
        "artefacts"
    } else {
        "artefacts_current"
    };
    let target_semantics_table = if use_historical_tables {
        "symbol_semantics"
    } else {
        "symbol_semantics_current"
    };

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
FROM {clone_edges_table} ce \
JOIN filtered src ON src.artefact_id = ce.source_artefact_id \
JOIN {target_artefacts_table} tgt ON tgt.repo_id = ce.repo_id AND tgt.artefact_id = ce.target_artefact_id \
LEFT JOIN {target_semantics_table} ss ON ss.artefact_id = tgt.artefact_id \
WHERE {} \
	ORDER BY ce.score DESC, tgt.path, tgt.symbol_fqn",
        clone_filters.join(" AND "),
        clone_edges_table = clone_edges_table,
        target_artefacts_table = target_artefacts_table,
        target_semantics_table = target_semantics_table,
        filtered_cte = filtered_cte,
    );

    Ok(match has_registered_clone_summary_stage(parsed) {
        true => sql,
        false => format!("{sql} LIMIT {}", parsed.limit.max(1)),
    })
}

async fn execute_relational_clones_neighbors_override(
    cfg: &DevqlConfig,
    _events_cfg: &EventsBackendConfig,
    parsed: &ParsedDevqlQuery,
    relational: &RelationalStorage,
    repo_id: &str,
    neighbors: i64,
) -> Result<Vec<Value>> {
    let spec = plan_devql_artefact_query(cfg, repo_id, parsed)?;
    let filtered_cte = build_filtered_artefacts_cte_sql(&spec);
    let source_sql = format!(
        "{filtered_cte} \
SELECT artefact_id, symbol_id, path, COALESCE(symbol_fqn, '') AS symbol_fqn \
FROM filtered \
LIMIT 2"
    );
    let source_rows = relational.query_rows(&source_sql).await?;
    if source_rows.len() != 1 {
        bail!(
            "clones(neighbors:...) requires the source artefact set to resolve to exactly one source symbol"
        );
    }

    let source = source_rows
        .first()
        .ok_or_else(|| anyhow!("missing source artefact row"))?;
    let source_symbol_id = source
        .get("symbol_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow!("missing source symbol_id for clones(neighbors:...)"))?;
    let source_artefact_id = source
        .get("artefact_id")
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| anyhow!("missing source artefact_id for clones(neighbors:...)"))?;
    let source_path = source
        .get("path")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let source_symbol_fqn = source
        .get("symbol_fqn")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let options =
        crate::capability_packs::semantic_clones::scoring::CloneScoringOptions::from_i64_clamped(
            neighbors,
        );

    let mut edges =
        crate::capability_packs::semantic_clones::pipeline::score_symbol_clone_edges_for_source_with_options(
            relational,
            repo_id,
            source_symbol_id,
            options,
        )
        .await?
        .edges;
    edges.retain(|edge| edge.source_symbol_id == source_symbol_id);
    if let Some(relation_kind) = parsed.clones.relation_kind.as_deref() {
        edges.retain(|edge| edge.relation_kind.eq_ignore_ascii_case(relation_kind));
    }
    if let Some(min_score) = parsed.clones.min_score {
        edges.retain(|edge| edge.score >= min_score.clamp(0.0, 1.0));
    }
    edges.sort_by(|left, right| {
        right
            .score
            .partial_cmp(&left.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| left.target_symbol_id.cmp(&right.target_symbol_id))
    });
    edges.truncate(parsed.limit.max(1));
    if edges.is_empty() {
        return Ok(Vec::new());
    }

    let target_artefact_ids = edges
        .iter()
        .map(|edge| edge.target_artefact_id.clone())
        .collect::<Vec<_>>();
    let target_sql = format!(
        "SELECT a.artefact_id, a.path, a.symbol_fqn, a.canonical_kind, a.language_kind, a.language, ss.summary \
FROM artefacts_current a \
LEFT JOIN symbol_semantics ss ON ss.artefact_id = a.artefact_id \
WHERE a.repo_id = '{}' \
  AND a.artefact_id IN ({})",
        esc_pg(repo_id),
        sql_string_list_pg(&target_artefact_ids),
    );
    let target_rows = relational.query_rows(&target_sql).await?;
    let target_by_artefact_id = target_rows
        .into_iter()
        .filter_map(|row| {
            row.get("artefact_id")
                .and_then(Value::as_str)
                .map(str::to_string)
                .map(|artefact_id| (artefact_id, row))
        })
        .collect::<std::collections::HashMap<_, _>>();

    Ok(edges
        .into_iter()
        .filter_map(|edge| {
            let target_row = target_by_artefact_id.get(&edge.target_artefact_id);
            target_row.map(|target_row| normalise_relational_result_row(serde_json::json!({
                "relation_kind": edge.relation_kind,
                "score": edge.score,
                "semantic_score": edge.semantic_score,
                "lexical_score": edge.lexical_score,
                "structural_score": edge.structural_score,
                "explanation_json": edge.explanation_json,
                "source_artefact_id": source_artefact_id.clone(),
                "source_path": source_path.clone(),
                "source_symbol_fqn": source_symbol_fqn.clone(),
                "target_artefact_id": edge.target_artefact_id,
                "target_path": target_row.get("path").cloned().unwrap_or(Value::Null),
                "target_symbol_fqn": target_row.get("symbol_fqn").cloned().unwrap_or(Value::Null),
                "target_canonical_kind": target_row.get("canonical_kind").cloned().unwrap_or(Value::Null),
                "target_language_kind": target_row.get("language_kind").cloned().unwrap_or(Value::Null),
                "target_language": target_row.get("language").cloned().unwrap_or(Value::Null),
                "target_summary": target_row.get("summary").cloned().unwrap_or(Value::Null),
            })))
        })
        .collect())
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
