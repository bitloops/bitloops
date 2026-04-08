use crate::artefact_query_planner::{ArtefactQuerySpec, plan_graphql_artefact_query};
use crate::graphql::ResolverScope;
use crate::graphql::types::{ArtefactFilterInput, ClonesFilterInput, SemanticClone};
use crate::host::devql::artefact_sql::build_filtered_artefacts_cte_sql;
use crate::host::devql::{esc_pg, escape_like_pattern, sql_like_with_escape};
use anyhow::{Context, Result, anyhow};
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
        let Some(project_path) = scope.project_path() else {
            return Ok(Vec::new());
        };
        let repo_id = self.repo_id_for_scope(scope)?;

        let sql = build_project_clones_sql(
            &repo_id,
            &self.current_branch_name(scope),
            project_path,
            filter,
        );
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
        let sql = build_artefact_clones_sql(
            &repo_id,
            &self.current_branch_name(scope),
            artefact_id,
            scope.project_path(),
            filter,
        );
        let rows = self.query_devql_sqlite_rows(&sql).await?;
        rows.into_iter()
            .map(clone_from_row)
            .map(|result| result.map(|clone| clone.with_scope(scope.clone())))
            .collect()
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

        let repo_id = self.repo_id_for_scope(scope)?;
        let sql = build_selected_clones_sql(&repo_id, artefact_ids, filter);
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
    repo_id: &str,
    _branch: &str,
    project_path: &str,
    filter: Option<&ClonesFilterInput>,
) -> String {
    let mut clauses = build_clone_filters(repo_id, filter);
    clauses.push(repo_path_prefix_clause("src.path", project_path));

    format!(
        "SELECT ce.source_artefact_id, ce.target_artefact_id, \
                src.start_line AS source_start_line, src.end_line AS source_end_line, \
                tgt.start_line AS target_start_line, tgt.end_line AS target_end_line, \
                ce.relation_kind, ce.score, ce.semantic_score, ce.lexical_score, ce.structural_score, ce.explanation_json \
           FROM symbol_clone_edges ce \
           JOIN artefacts_current src ON src.repo_id = ce.repo_id \
                                     AND src.symbol_id = ce.source_symbol_id \
           JOIN artefacts_current tgt ON tgt.repo_id = ce.repo_id \
                                     AND tgt.symbol_id = ce.target_symbol_id \
          WHERE {} \
       ORDER BY ce.score DESC, tgt.path, COALESCE(tgt.symbol_fqn, ''), ce.target_artefact_id",
        clauses.join(" AND "),
    )
}

fn build_artefact_clones_sql(
    repo_id: &str,
    _branch: &str,
    artefact_id: &str,
    project_path: Option<&str>,
    filter: Option<&ClonesFilterInput>,
) -> String {
    let mut clauses = build_clone_filters(repo_id, filter);
    clauses.push(format!("ce.source_artefact_id = '{}'", esc_pg(artefact_id)));
    if let Some(project_path) = project_path {
        clauses.push(repo_path_prefix_clause("src.path", project_path));
    }

    format!(
        "SELECT ce.source_artefact_id, ce.target_artefact_id, \
                src.start_line AS source_start_line, src.end_line AS source_end_line, \
                tgt.start_line AS target_start_line, tgt.end_line AS target_end_line, \
                ce.relation_kind, ce.score, ce.semantic_score, ce.lexical_score, ce.structural_score, ce.explanation_json \
           FROM symbol_clone_edges ce \
           JOIN artefacts_current src ON src.repo_id = ce.repo_id \
                                     AND src.symbol_id = ce.source_symbol_id \
           JOIN artefacts_current tgt ON tgt.repo_id = ce.repo_id \
                                     AND tgt.symbol_id = ce.target_symbol_id \
          WHERE {} \
       ORDER BY ce.score DESC, tgt.path, COALESCE(tgt.symbol_fqn, ''), ce.target_artefact_id",
        clauses.join(" AND "),
    )
}

fn build_selected_clones_sql(
    repo_id: &str,
    artefact_ids: &[String],
    filter: Option<&ClonesFilterInput>,
) -> String {
    let mut clauses = build_clone_filters(repo_id, filter);
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
           FROM symbol_clone_edges ce \
           JOIN artefacts_current src ON src.repo_id = ce.repo_id \
                                     AND src.symbol_id = ce.source_symbol_id \
           JOIN artefacts_current tgt ON tgt.repo_id = ce.repo_id \
                                     AND tgt.symbol_id = ce.target_symbol_id \
          WHERE {} \
       ORDER BY ce.score DESC, src.path, COALESCE(src.symbol_fqn, ''), \
                tgt.path, COALESCE(tgt.symbol_fqn, ''), \
                ce.source_artefact_id, ce.target_artefact_id",
        clauses.join(" AND "),
    )
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

fn build_clone_summary_sql(spec: &ArtefactQuerySpec, filter: Option<&ClonesFilterInput>) -> String {
    let filtered_cte = build_filtered_artefacts_cte_sql(spec);
    let clauses = build_clone_filters(spec.repo_id.as_str(), filter);

    format!(
        "{filtered_cte} \
         SELECT ce.relation_kind AS relation_kind, COUNT(*) AS count \
           FROM filtered fa \
           JOIN symbol_clone_edges ce \
             ON ce.repo_id = '{repo_id}' \
            AND ce.source_artefact_id = fa.artefact_id \
          WHERE {clauses} \
       GROUP BY ce.relation_kind",
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

fn clone_from_row(row: Value) -> Result<SemanticClone> {
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

    fn clone_summary_spec() -> ArtefactQuerySpec {
        ArtefactQuerySpec {
            repo_id: "repo-1".to_string(),
            branch: Some("main".to_string()),
            historical_path_blob_sha: None,
            scope: ArtefactScope {
                project_path: Some("packages/api".to_string()),
                path: None,
                files_path: None,
            },
            temporal_scope: ArtefactTemporalScope::Current,
            structural_filter: ArtefactStructuralFilter::default(),
            activity_filter: None,
            pagination: None,
        }
    }

    #[test]
    fn clone_summary_sql_aggregates_filtered_sources_by_relation_kind() {
        let sql = build_clone_summary_sql(
            &clone_summary_spec(),
            Some(&ClonesFilterInput {
                relation_kind: Some("similar_implementation".to_string()),
                min_score: Some(0.75),
            }),
        );

        assert!(sql.contains("WITH filtered AS"));
        assert!(sql.contains("FROM filtered fa"));
        assert!(sql.contains("JOIN symbol_clone_edges ce"));
        assert!(sql.contains("ce.source_artefact_id = fa.artefact_id"));
        assert!(sql.contains("ce.relation_kind = 'similar_implementation'"));
        assert!(sql.contains("ce.score >= 0.75"));
        assert!(sql.contains("GROUP BY ce.relation_kind"));
    }
}
