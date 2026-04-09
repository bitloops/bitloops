use crate::artefact_query_planner::{ArtefactQuerySpec, plan_graphql_artefact_query};
use crate::capability_packs::semantic_clones::scoring::SymbolCloneEdgeRow;
use crate::graphql::ResolverScope;
use crate::graphql::types::{ArtefactFilterInput, ClonesFilterInput, SemanticClone};
use crate::host::devql::artefact_sql::build_filtered_artefacts_cte_sql;
use crate::host::devql::{esc_pg, escape_like_pattern, sql_like_with_escape};
use crate::host::relational_store::DefaultRelationalStore;
use anyhow::{Context, Result, anyhow, bail};
use async_graphql::types::Json;
use serde_json::{Map, Value};
use std::collections::BTreeMap;
use std::convert::TryFrom;

use super::super::DevqlGraphqlContext;

impl DevqlGraphqlContext {
    pub(crate) async fn list_project_clones(
        &self,
        scope: &ResolverScope,
        filter: Option<&ClonesFilterInput>,
    ) -> Result<Vec<SemanticClone>> {
        if filter
            .and_then(ClonesFilterInput::neighbors_override)
            .is_some()
        {
            bail!("`neighbors` override is only supported for artefact-scoped `clones` queries");
        }
        let Some(project_path) = scope.project_path() else {
            return Ok(Vec::new());
        };
        let repo_id = self.repo_id_for_scope(scope)?;
        let spec = plan_graphql_artefact_query(
            &repo_id,
            &self.current_branch_name(scope),
            None,
            None,
            scope,
            None,
        );

        let sql = build_project_clones_sql(&spec, project_path, filter);
        let rows = self.query_devql_sqlite_rows(&sql).await?;
        rows.into_iter()
            .map(clone_from_row)
            .map(|result| result.map(|clone| clone.with_scope(scope.clone())))
            .collect()
    }

    pub(crate) async fn list_artefact_clones(
        &self,
        artefact_id: &str,
        filter: Option<&ClonesFilterInput>,
        scope: &ResolverScope,
    ) -> Result<Vec<SemanticClone>> {
        let repo_id = self.repo_id_for_scope(scope)?;
        if let Some(options) = filter.and_then(ClonesFilterInput::neighbors_override) {
            let Some(source_symbol_id) = self
                .load_symbol_id_for_artefact(&repo_id, artefact_id)
                .await?
            else {
                return Ok(Vec::new());
            };
            let repo_root = self.repo_root_for_scope(scope)?;
            let relational_store = DefaultRelationalStore::open_local_for_repo_root(&repo_root)
                .context("opening relational store for GraphQL clone neighbors query")?;
            let relational = relational_store.to_local_inner();
            let mut edges = crate::capability_packs::semantic_clones::pipeline::score_symbol_clone_edges_for_source_with_options(
                &relational,
                &repo_id,
                &source_symbol_id,
                options,
            )
            .await?
            .edges;
            edges.retain(|edge| {
                edge.source_artefact_id == artefact_id && clone_edge_matches_filter(edge, filter)
            });
            edges.sort_by(|left, right| {
                right
                    .score
                    .partial_cmp(&left.score)
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then_with(|| left.target_artefact_id.cmp(&right.target_artefact_id))
            });
            return edges
                .into_iter()
                .map(clone_from_edge)
                .map(|result| result.map(|clone| clone.with_scope(scope.clone())))
                .collect();
        }
        let spec = plan_graphql_artefact_query(
            &repo_id,
            &self.current_branch_name(scope),
            None,
            None,
            scope,
            None,
        );
        let sql = build_artefact_clones_sql(&spec, artefact_id, filter);
        let rows = self.query_devql_sqlite_rows(&sql).await?;
        rows.into_iter()
            .map(clone_from_row)
            .map(|result| result.map(|clone| clone.with_scope(scope.clone())))
            .collect()
    }

    async fn load_symbol_id_for_artefact(
        &self,
        repo_id: &str,
        artefact_id: &str,
    ) -> Result<Option<String>> {
        let sql = format!(
            "SELECT symbol_id \
FROM artefacts_current \
WHERE repo_id = '{}' AND artefact_id = '{}' \
LIMIT 1",
            esc_pg(repo_id),
            esc_pg(artefact_id),
        );
        let rows = self.query_devql_sqlite_rows(&sql).await?;
        Ok(rows
            .into_iter()
            .find_map(|row| optional_string(&row, "symbol_id")))
    }

    pub(crate) async fn list_selected_clones(
        &self,
        artefact_ids: &[String],
        filter: Option<&ClonesFilterInput>,
        scope: &ResolverScope,
    ) -> Result<Vec<SemanticClone>> {
        if artefact_ids.is_empty() {
            return Ok(Vec::new());
        }
        if filter
            .and_then(ClonesFilterInput::neighbors_override)
            .is_some()
        {
            bail!("`neighbors` override is only supported for artefact-scoped `clones` queries");
        }

        let repo_id = self.repo_id_for_scope(scope)?;
        let spec = plan_graphql_artefact_query(
            &repo_id,
            &self.current_branch_name(scope),
            None,
            None,
            scope,
            None,
        );
        let sql = build_selected_clones_sql(&spec, artefact_ids, filter);
        let rows = self.query_devql_sqlite_rows(&sql).await?;
        rows.into_iter()
            .map(clone_from_row)
            .map(|result| result.map(|clone| clone.with_scope(scope.clone())))
            .collect()
    }

    pub(crate) async fn summarize_clones(
        &self,
        path: Option<&str>,
        artefact_filter: Option<&ArtefactFilterInput>,
        clone_filter: Option<&ClonesFilterInput>,
        scope: &ResolverScope,
    ) -> Result<BTreeMap<String, usize>> {
        let repo_id = self.repo_id_for_scope(scope)?;
        let spec = plan_graphql_artefact_query(
            &repo_id,
            &self.current_branch_name(scope),
            path,
            artefact_filter,
            scope,
            None,
        );
        let sql = build_clone_summary_sql(&spec, clone_filter);
        let rows = self.query_devql_sqlite_rows(&sql).await?;

        rows.into_iter()
            .map(clone_summary_group_from_row)
            .collect::<Result<BTreeMap<_, _>>>()
    }
}

fn build_project_clones_sql(
    spec: &ArtefactQuerySpec,
    project_path: &str,
    filter: Option<&ClonesFilterInput>,
) -> String {
    let filtered_cte = build_filtered_artefacts_cte_sql(spec);
    let (clone_edges_table, artefacts_table) = clone_projection_tables(spec);
    let mut clauses = build_clone_filters(spec.repo_id.as_str(), filter);
    let project_clause = repo_path_prefix_clause("src.path", project_path);
    clauses.push(project_clause);

    format!(
        "{filtered_cte} \
         SELECT ce.source_artefact_id, ce.target_artefact_id, \
                src.start_line AS source_start_line, src.end_line AS source_end_line, \
                tgt.start_line AS target_start_line, tgt.end_line AS target_end_line, \
                ce.relation_kind, ce.score, \
                ce.semantic_score, ce.lexical_score, ce.structural_score, ce.explanation_json \
           FROM {clone_edges_table} ce \
           JOIN filtered src ON src.artefact_id = ce.source_artefact_id \
           JOIN {artefacts_table} tgt ON tgt.repo_id = ce.repo_id \
                                     AND tgt.artefact_id = ce.target_artefact_id \
          WHERE {clauses} \
       ORDER BY ce.score DESC, tgt.path, COALESCE(tgt.symbol_fqn, ''), ce.target_artefact_id",
        filtered_cte = filtered_cte,
        clone_edges_table = clone_edges_table,
        artefacts_table = artefacts_table,
        clauses = clauses.join(" AND "),
    )
}

fn build_artefact_clones_sql(
    spec: &ArtefactQuerySpec,
    artefact_id: &str,
    filter: Option<&ClonesFilterInput>,
) -> String {
    let filtered_cte = build_filtered_artefacts_cte_sql(spec);
    let (clone_edges_table, artefacts_table) = clone_projection_tables(spec);
    let mut clauses = build_clone_filters(spec.repo_id.as_str(), filter);
    clauses.push(format!("ce.source_artefact_id = '{}'", esc_pg(artefact_id)));

    format!(
        "{filtered_cte} \
         SELECT ce.source_artefact_id, ce.target_artefact_id, \
                src.start_line AS source_start_line, src.end_line AS source_end_line, \
                tgt.start_line AS target_start_line, tgt.end_line AS target_end_line, \
                ce.relation_kind, ce.score, \
                ce.semantic_score, ce.lexical_score, ce.structural_score, ce.explanation_json \
           FROM {clone_edges_table} ce \
           JOIN filtered src ON src.artefact_id = ce.source_artefact_id \
           JOIN {artefacts_table} tgt ON tgt.repo_id = ce.repo_id \
                                     AND tgt.artefact_id = ce.target_artefact_id \
          WHERE {clauses} \
       ORDER BY ce.score DESC, tgt.path, COALESCE(tgt.symbol_fqn, ''), ce.target_artefact_id",
        filtered_cte = filtered_cte,
        clone_edges_table = clone_edges_table,
        artefacts_table = artefacts_table,
        clauses = clauses.join(" AND "),
    )
}

fn build_selected_clones_sql(
    spec: &ArtefactQuerySpec,
    artefact_ids: &[String],
    filter: Option<&ClonesFilterInput>,
) -> String {
    let (clone_edges_table, artefacts_table) = clone_projection_tables(spec);
    let mut clauses = build_clone_filters(spec.repo_id.as_str(), filter);
    let ids = artefact_ids
        .iter()
        .map(|artefact_id| format!("'{}'", esc_pg(artefact_id)))
        .collect::<Vec<_>>()
        .join(", ");
    clauses.push(format!(
        "(ce.source_artefact_id IN ({ids}) OR ce.target_artefact_id IN ({ids}))"
    ));

    format!(
        "SELECT ce.source_artefact_id, ce.target_artefact_id, \
                src.start_line AS source_start_line, src.end_line AS source_end_line, \
                tgt.start_line AS target_start_line, tgt.end_line AS target_end_line, \
                ce.relation_kind, ce.score, ce.semantic_score, ce.lexical_score, ce.structural_score, ce.explanation_json \
           FROM {clone_edges_table} ce \
           JOIN {artefacts_table} src ON src.repo_id = ce.repo_id \
                                     AND src.artefact_id = ce.source_artefact_id \
           JOIN {artefacts_table} tgt ON tgt.repo_id = ce.repo_id \
                                     AND tgt.artefact_id = ce.target_artefact_id \
          WHERE {clauses} \
       ORDER BY ce.score DESC, src.path, COALESCE(src.symbol_fqn, ''), \
                tgt.path, COALESCE(tgt.symbol_fqn, ''), \
                ce.source_artefact_id, ce.target_artefact_id",
        clone_edges_table = clone_edges_table,
        artefacts_table = artefacts_table,
        clauses = clauses.join(" AND "),
    )
}

fn clone_projection_tables(spec: &ArtefactQuerySpec) -> (&'static str, &'static str) {
    if spec.temporal_scope.use_historical_tables() {
        ("symbol_clone_edges", "artefacts")
    } else {
        ("symbol_clone_edges_current", "artefacts_current")
    }
}

fn build_clone_filters(repo_id: &str, filter: Option<&ClonesFilterInput>) -> Vec<String> {
    let mut clauses = vec![format!("ce.repo_id = '{}'", esc_pg(repo_id))];
    if let Some(filter) = filter {
        if let Some(relation_kind) = filter.relation_kind() {
            clauses.push(format!("ce.relation_kind = '{}'", esc_pg(relation_kind)));
        }
        if let Some(min_score) = filter.min_score {
            clauses.push(format!("ce.score >= {}", min_score.clamp(0.0, 1.0)));
        }
    }

    clauses
}

pub(super) fn build_clone_summary_sql(
    spec: &ArtefactQuerySpec,
    filter: Option<&ClonesFilterInput>,
) -> String {
    let filtered_cte = build_filtered_artefacts_cte_sql(spec);
    let (clone_edges_table, _) = clone_projection_tables(spec);
    let clauses = build_clone_filters(spec.repo_id.as_str(), filter);

    format!(
        "{filtered_cte} \
         SELECT ce.relation_kind AS relation_kind, COUNT(*) AS count \
           FROM filtered fa \
           JOIN {clone_edges_table} ce \
             ON ce.repo_id = '{repo_id}' \
            AND ce.source_artefact_id = fa.artefact_id \
          WHERE {clauses} \
       GROUP BY ce.relation_kind",
        clone_edges_table = clone_edges_table,
        repo_id = esc_pg(spec.repo_id.as_str()),
        clauses = clauses.join(" AND "),
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

pub(super) fn clone_from_row(row: Value) -> Result<SemanticClone> {
    let source_artefact_id = required_string(&row, "source_artefact_id")?;
    let target_artefact_id = required_string(&row, "target_artefact_id")?;
    let relation_kind = required_string(&row, "relation_kind")?;

    let mut metadata = Map::new();
    if let Some(score) = optional_f64(&row, "semantic_score") {
        metadata.insert("semanticScore".to_string(), Value::from(score));
    }
    if let Some(score) = optional_f64(&row, "lexical_score") {
        metadata.insert("lexicalScore".to_string(), Value::from(score));
    }
    if let Some(score) = optional_f64(&row, "structural_score") {
        metadata.insert("structuralScore".to_string(), Value::from(score));
    }
    if let Some(explanation) = parse_json_column(row.get("explanation_json"))? {
        metadata.insert("explanation".to_string(), explanation);
    }

    Ok(SemanticClone {
        id: format!("clone::{source_artefact_id}::{target_artefact_id}::{relation_kind}").into(),
        source_artefact_id: source_artefact_id.into(),
        target_artefact_id: target_artefact_id.into(),
        source_start_line: optional_i32(&row, "source_start_line"),
        source_end_line: optional_i32(&row, "source_end_line"),
        target_start_line: optional_i32(&row, "target_start_line"),
        target_end_line: optional_i32(&row, "target_end_line"),
        relation_kind,
        score: required_f64(&row, "score")?,
        metadata: (!metadata.is_empty()).then_some(Json(Value::Object(metadata))),
        scope: ResolverScope::default(),
    })
}

pub(super) fn clone_from_edge(edge: SymbolCloneEdgeRow) -> Result<SemanticClone> {
    let mut metadata = Map::new();
    metadata.insert(
        "semanticScore".to_string(),
        Value::from(f64::from(edge.semantic_score)),
    );
    metadata.insert(
        "lexicalScore".to_string(),
        Value::from(f64::from(edge.lexical_score)),
    );
    metadata.insert(
        "structuralScore".to_string(),
        Value::from(f64::from(edge.structural_score)),
    );
    metadata.insert("explanation".to_string(), edge.explanation_json);

    Ok(SemanticClone {
        id: format!(
            "clone::{}::{}::{}",
            edge.source_artefact_id, edge.target_artefact_id, edge.relation_kind
        )
        .into(),
        source_artefact_id: edge.source_artefact_id.into(),
        target_artefact_id: edge.target_artefact_id.into(),
        source_start_line: None,
        source_end_line: None,
        target_start_line: None,
        target_end_line: None,
        relation_kind: edge.relation_kind,
        score: f64::from(edge.score),
        metadata: Some(Json(Value::Object(metadata))),
        scope: ResolverScope::default(),
    })
}

pub(super) fn clone_edge_matches_filter(
    edge: &SymbolCloneEdgeRow,
    filter: Option<&ClonesFilterInput>,
) -> bool {
    let Some(filter) = filter else {
        return true;
    };
    if let Some(relation_kind) = filter.relation_kind()
        && !edge.relation_kind.eq_ignore_ascii_case(relation_kind)
    {
        return false;
    }
    if let Some(min_score) = filter.min_score
        && f64::from(edge.score) < min_score.clamp(0.0, 1.0)
    {
        return false;
    }
    true
}

fn clone_summary_group_from_row(row: Value) -> Result<(String, usize)> {
    let relation_kind = required_string(&row, "relation_kind")?;
    let count = row
        .get("count")
        .and_then(|value| {
            value
                .as_u64()
                .or_else(|| value.as_i64().and_then(|count| u64::try_from(count).ok()))
        })
        .and_then(|count| usize::try_from(count).ok())
        .ok_or_else(|| anyhow!("missing `count`"))?;
    Ok((relation_kind, count))
}

fn required_string(row: &Value, key: &str) -> Result<String> {
    optional_string(row, key).ok_or_else(|| anyhow!("missing `{key}`"))
}

fn optional_string(row: &Value, key: &str) -> Option<String> {
    row.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn required_f64(row: &Value, key: &str) -> Result<f64> {
    optional_f64(row, key).ok_or_else(|| anyhow!("missing `{key}`"))
}

fn optional_f64(row: &Value, key: &str) -> Option<f64> {
    row.get(key).and_then(Value::as_f64).or_else(|| {
        row.get(key)
            .and_then(Value::as_i64)
            .map(|value| value as f64)
    })
}

fn optional_i32(row: &Value, key: &str) -> Option<i32> {
    row.get(key)
        .and_then(Value::as_i64)
        .and_then(|value| i32::try_from(value).ok())
}

fn parse_json_column(value: Option<&Value>) -> Result<Option<Value>> {
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(raw)) => {
            let trimmed = raw.trim();
            if trimmed.is_empty() {
                return Ok(None);
            }

            serde_json::from_str(trimmed)
                .map(Some)
                .with_context(|| "parsing JSON payload column")
        }
        Some(other) => Ok(Some(other.clone())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn clone_from_row_preserves_metadata_and_missing_spans() {
        let clone = clone_from_row(json!({
            "source_artefact_id": "artefact::source",
            "target_artefact_id": "artefact::target",
            "relation_kind": "similar_implementation",
            "score": 0.9,
            "semantic_score": 0.8,
            "lexical_score": 0.7,
            "structural_score": 0.6,
            "explanation_json": "{\"reason\":\"shared structure\"}"
        }))
        .expect("clone row parses");

        assert_eq!(clone.source_start_line, None);
        assert_eq!(clone.source_end_line, None);
        assert_eq!(clone.target_start_line, None);
        assert_eq!(clone.target_end_line, None);
        let metadata = clone.metadata.expect("metadata should be present");
        assert_eq!(metadata.0["semanticScore"], json!(0.8));
        assert_eq!(metadata.0["lexicalScore"], json!(0.7));
        assert_eq!(metadata.0["structuralScore"], json!(0.6));
        assert_eq!(metadata.0["explanation"]["reason"], "shared structure");
    }
}

#[cfg(test)]
mod clone_summary_tests {
    use super::*;
    use crate::artefact_query_planner::{
        ArtefactQuerySpec, ArtefactScope, ArtefactStructuralFilter, ArtefactTemporalScope,
    };

    fn clone_spec(temporal_scope: ArtefactTemporalScope) -> ArtefactQuerySpec {
        ArtefactQuerySpec {
            repo_id: "repo-1".to_string(),
            branch: (!temporal_scope.use_historical_tables()).then(|| "main".to_string()),
            historical_path_blob_sha: None,
            scope: ArtefactScope {
                project_path: Some("packages/api".to_string()),
                path: None,
                files_path: None,
            },
            temporal_scope,
            structural_filter: ArtefactStructuralFilter::default(),
            activity_filter: None,
            pagination: None,
        }
    }

    #[test]
    fn project_clone_sql_uses_current_projection_tables_for_current_scope() {
        let sql = build_project_clones_sql(
            &clone_spec(ArtefactTemporalScope::Current),
            "packages/api",
            Some(&ClonesFilterInput {
                relation_kind: Some("similar_implementation".to_string()),
                min_score: Some(0.75),
                neighbors: None,
            }),
        );

        assert!(sql.contains("WITH filtered AS"));
        assert!(sql.contains("FROM symbol_clone_edges_current ce"));
        assert!(sql.contains("JOIN filtered src ON src.artefact_id = ce.source_artefact_id"));
        assert!(sql.contains("JOIN artefacts_current tgt ON tgt.repo_id = ce.repo_id"));
        assert!(sql.contains("tgt.artefact_id = ce.target_artefact_id"));
        assert!(!sql.contains("src.symbol_id = ce.source_symbol_id"));
        assert!(!sql.contains("tgt.symbol_id = ce.target_symbol_id"));
    }

    #[test]
    fn selected_clone_sql_uses_historical_projection_tables_for_historical_scope() {
        let sql = build_selected_clones_sql(
            &clone_spec(ArtefactTemporalScope::HistoricalCommit {
                commit_sha: "abc123".to_string(),
            }),
            &["artefact::source".to_string()],
            None,
        );

        assert!(sql.contains("FROM symbol_clone_edges ce"));
        assert!(sql.contains("JOIN artefacts src ON src.repo_id = ce.repo_id"));
        assert!(sql.contains("src.artefact_id = ce.source_artefact_id"));
        assert!(sql.contains("JOIN artefacts tgt ON tgt.repo_id = ce.repo_id"));
        assert!(sql.contains("tgt.artefact_id = ce.target_artefact_id"));
    }

    #[test]
    fn clone_summary_sql_uses_current_projection_tables_for_current_scope() {
        let sql = build_clone_summary_sql(
            &clone_spec(ArtefactTemporalScope::Current),
            Some(&ClonesFilterInput {
                relation_kind: Some("similar_implementation".to_string()),
                min_score: Some(0.75),
                neighbors: None,
            }),
        );

        assert!(sql.contains("WITH filtered AS"));
        assert!(sql.contains("FROM filtered fa"));
        assert!(sql.contains("JOIN symbol_clone_edges_current ce"));
        assert!(sql.contains("ce.source_artefact_id = fa.artefact_id"));
        assert!(sql.contains("ce.relation_kind = 'similar_implementation'"));
        assert!(sql.contains("ce.score >= 0.75"));
        assert!(sql.contains("GROUP BY ce.relation_kind"));
    }
}
