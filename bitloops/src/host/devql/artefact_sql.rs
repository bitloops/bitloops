use super::*;
use crate::artefact_query_planner::{
    ArtefactActivityFilter, ArtefactKindFilter, ArtefactQuerySpec,
};
use crate::host::devql::checkpoint_file_snapshots::{
    CheckpointFileSnapshotActivityFilter, CheckpointFileSnapshotExistsSql,
    build_checkpoint_file_snapshot_exists_clause,
};

pub(crate) fn build_filtered_artefacts_select_sql(spec: &ArtefactQuerySpec) -> String {
    let filtered_cte = build_filtered_artefacts_cte_sql(spec);
    let use_historical_tables = spec.temporal_scope.use_historical_tables();
    format!(
        "{filtered_cte} \
         SELECT {columns} \
           FROM filtered \
       ORDER BY {order}",
        columns = filtered_artefact_columns_sql(use_historical_tables),
        order = filtered_artefact_order_sql(),
    )
}

pub(crate) fn build_filtered_artefacts_cte_sql(spec: &ArtefactQuerySpec) -> String {
    let use_historical_tables = spec.temporal_scope.use_historical_tables();
    let clauses = build_artefact_where_clauses("a", spec);
    format!(
        "WITH filtered AS ( \
             SELECT {columns}, {kind_rank} AS kind_rank \
               FROM {table} a \
              WHERE {clauses} \
         )",
        columns = artefact_select_columns_sql("a", use_historical_tables),
        kind_rank = artefact_kind_rank_sql("a"),
        table = artefacts_table_sql(use_historical_tables),
        clauses = clauses.join(" AND "),
    )
}

pub(crate) fn filtered_artefact_columns_sql(_use_historical_tables: bool) -> &'static str {
    "symbol_id, artefact_id, path, language, canonical_kind, language_kind, symbol_fqn, \
     parent_artefact_id, start_line, end_line, start_byte, end_byte, signature, modifiers, \
     docstring, summary, embedding_representations, blob_sha, content_hash, created_at"
}

pub(crate) fn filtered_artefact_order_sql() -> &'static str {
    "path, kind_rank, start_line, end_line, artefact_id"
}

fn build_artefact_where_clauses(alias: &str, spec: &ArtefactQuerySpec) -> Vec<String> {
    let use_historical_tables = spec.temporal_scope.use_historical_tables();
    let mut clauses = vec![format!(
        "{alias}.repo_id = '{}'",
        esc_pg(spec.repo_id.as_str())
    )];
    // Sync-shaped artefacts_current has no revision_kind / revision_id columns; save-revision
    // scopes are handled at the GraphQL layer without extra SQL predicates here.
    if let Some(commit_sha) = spec.temporal_scope.resolved_commit() {
        let blob_column = if use_historical_tables {
            format!("{alias}.blob_sha")
        } else {
            format!("{alias}.content_id")
        };
        if let Some(blob_sha) = spec.historical_path_blob_sha.as_deref() {
            clauses.push(format!("{blob_column} = '{}'", esc_pg(blob_sha)));
        } else {
            clauses.push(file_state_exists_clause(
                &format!("{alias}.path"),
                &blob_column,
                spec.repo_id.as_str(),
                commit_sha,
            ));
        }
    }
    if let Some(kind) = spec.structural_filter.kind.as_ref() {
        clauses.push(canonical_kind_clause(
            &format!("{alias}.canonical_kind"),
            kind,
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
        let path_column = format!("{alias}.path");
        let path_candidates = build_path_candidates(path);
        clauses.push(format!(
            "({})",
            sql_path_candidates_clause(&path_column, &path_candidates)
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
    if let Some(activity_filter) = spec.activity_filter.as_ref() {
        clauses.push(checkpoint_file_snapshot_exists_clause(
            alias,
            spec.repo_id.as_str(),
            use_historical_tables,
            activity_filter,
        ));
    }

    clauses
}

fn checkpoint_file_snapshot_exists_clause(
    alias: &str,
    repo_id: &str,
    use_historical_tables: bool,
    activity_filter: &ArtefactActivityFilter,
) -> String {
    let path_column = format!("{alias}.path");
    let blob_sha_column = if use_historical_tables {
        format!("{alias}.blob_sha")
    } else {
        format!("{alias}.content_id")
    };
    build_checkpoint_file_snapshot_exists_clause(CheckpointFileSnapshotExistsSql {
        repo_id,
        path_column: path_column.as_str(),
        blob_sha_column: blob_sha_column.as_str(),
        activity_filter: CheckpointFileSnapshotActivityFilter {
            agent: activity_filter.agent.as_deref(),
            since: activity_filter.since.as_deref(),
        },
    })
}

fn canonical_kind_clause(column: &str, kind: &ArtefactKindFilter) -> String {
    let values: &[&str] = match kind {
        ArtefactKindFilter::File => &["file"],
        ArtefactKindFilter::Namespace => &["namespace"],
        ArtefactKindFilter::Module => &["module"],
        ArtefactKindFilter::Import => &["import"],
        ArtefactKindFilter::Type => &["type", "interface", "enum"],
        ArtefactKindFilter::Interface => &["interface"],
        ArtefactKindFilter::Enum => &["enum"],
        ArtefactKindFilter::Callable => &["callable", "function", "method"],
        ArtefactKindFilter::Function => &["function"],
        ArtefactKindFilter::Method => &["method"],
        ArtefactKindFilter::Value => &["value", "variable", "constant"],
        ArtefactKindFilter::Variable => &["variable", "constant"],
        ArtefactKindFilter::Constant => &["constant"],
        ArtefactKindFilter::Member => &["member"],
        ArtefactKindFilter::Parameter => &["parameter"],
        ArtefactKindFilter::TypeParameter => &["type_parameter"],
        ArtefactKindFilter::Alias => &["alias"],
        ArtefactKindFilter::Raw(value) => return format!("{column} = '{}'", esc_pg(value)),
    };

    if values.len() == 1 {
        return format!("{column} = '{}'", esc_pg(values[0]));
    }

    format!(
        "({})",
        values
            .iter()
            .map(|value| format!("{column} = '{}'", esc_pg(value)))
            .collect::<Vec<_>>()
            .join(" OR ")
    )
}

fn file_state_exists_clause(
    path_column: &str,
    blob_column: &str,
    repo_id: &str,
    commit_sha: &str,
) -> String {
    format!(
        "EXISTS (SELECT 1 FROM file_state fs WHERE fs.repo_id = '{repo_id}' \
           AND fs.commit_sha = '{commit_sha}' AND fs.path = {path_column} AND fs.blob_sha = {blob_column})",
        repo_id = esc_pg(repo_id),
        commit_sha = esc_pg(commit_sha),
        path_column = path_column,
        blob_column = blob_column,
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

fn artefacts_table_sql(use_historical_tables: bool) -> &'static str {
    if use_historical_tables {
        "artefacts_historical"
    } else {
        "artefacts_current"
    }
}

fn artefact_select_columns_sql(alias: &str, use_historical_tables: bool) -> String {
    let summary_expr = artefact_summary_sql(alias, use_historical_tables);
    let embedding_representations_expr =
        artefact_embedding_representations_sql(alias, use_historical_tables);
    if use_historical_tables {
        format!(
            "{alias}.symbol_id, {alias}.artefact_id, {alias}.path, {alias}.language, \
             {alias}.canonical_kind, {alias}.language_kind, {alias}.symbol_fqn, \
             {alias}.parent_artefact_id, {alias}.start_line, {alias}.end_line, \
             {alias}.start_byte, {alias}.end_byte, {alias}.signature, {alias}.modifiers, \
             {alias}.docstring, {summary_expr} AS summary, \
             {embedding_representations_expr} AS embedding_representations, \
             {alias}.blob_sha, {alias}.content_hash, {alias}.created_at AS created_at",
        )
    } else {
        format!(
            "{alias}.symbol_id, {alias}.artefact_id, {alias}.path, {alias}.language, \
             {alias}.canonical_kind, {alias}.language_kind, {alias}.symbol_fqn, \
             {alias}.parent_artefact_id, {alias}.start_line, {alias}.end_line, \
             {alias}.start_byte, {alias}.end_byte, {alias}.signature, {alias}.modifiers, \
             {alias}.docstring, {summary_expr} AS summary, \
             {embedding_representations_expr} AS embedding_representations, \
             {alias}.content_id AS blob_sha, NULL AS content_hash, {alias}.updated_at AS created_at",
        )
    }
}

fn artefact_summary_sql(alias: &str, use_historical_tables: bool) -> String {
    if use_historical_tables {
        format!(
            "(SELECT ss.summary FROM symbol_semantics ss \
               WHERE ss.repo_id = {alias}.repo_id \
                 AND ss.artefact_id = {alias}.artefact_id \
                 AND ss.blob_sha = {alias}.blob_sha \
               LIMIT 1)"
        )
    } else {
        format!(
            "COALESCE( \
               (SELECT ss.summary FROM symbol_semantics_current ss \
                  WHERE ss.repo_id = {alias}.repo_id \
                    AND ss.artefact_id = {alias}.artefact_id \
                    AND ss.content_id = {alias}.content_id \
                  LIMIT 1), \
               (SELECT hs.summary FROM symbol_semantics hs \
                  WHERE hs.repo_id = {alias}.repo_id \
                    AND hs.artefact_id = {alias}.artefact_id \
                    AND hs.blob_sha = {alias}.content_id \
                  LIMIT 1) \
             )"
        )
    }
}

fn artefact_embedding_representations_sql(alias: &str, use_historical_tables: bool) -> String {
    let (table, blob_column) = if use_historical_tables {
        ("symbol_embeddings", format!("{alias}.blob_sha"))
    } else {
        ("symbol_embeddings_current", format!("{alias}.content_id"))
    };

    format!(
        "CASE \
           WHEN EXISTS (SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = '{table}') \
           THEN COALESCE((SELECT json_group_array(representation_kind) \
               FROM (SELECT DISTINCT se.representation_kind AS representation_kind \
                       FROM {table} se \
                      WHERE se.repo_id = {alias}.repo_id \
                        AND se.artefact_id = {alias}.artefact_id \
                        AND se.{embedding_blob_column} = {blob_column} \
                   ORDER BY CASE se.representation_kind \
                       WHEN 'identity' THEN 0 \
                       WHEN 'locator' THEN 0 \
                       WHEN 'code' THEN 1 \
                       WHEN 'baseline' THEN 1 \
                       WHEN 'enriched' THEN 1 \
                       WHEN 'summary' THEN 2 \
                       ELSE 9 \
                   END)), '[]') \
           ELSE '[]' \
         END",
        table = table,
        alias = alias,
        embedding_blob_column = if use_historical_tables {
            "blob_sha"
        } else {
            "content_id"
        },
        blob_column = blob_column,
    )
}

fn artefact_kind_rank_sql(alias: &str) -> String {
    format!("CASE WHEN {alias}.canonical_kind = 'file' THEN 0 ELSE 1 END")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::artefact_query_planner::{
        ArtefactLineRange, ArtefactPagination, ArtefactScope, ArtefactStructuralFilter,
        ArtefactTemporalScope,
    };

    fn current_activity_spec() -> ArtefactQuerySpec {
        ArtefactQuerySpec {
            repo_id: "repo-1".to_string(),
            branch: Some("main".to_string()),
            historical_path_blob_sha: None,
            scope: ArtefactScope {
                project_path: Some("packages/api".to_string()),
                path: Some("./packages/api/src/lib.rs".to_string()),
                files_path: Some("packages/api/src/*.rs".to_string()),
            },
            temporal_scope: ArtefactTemporalScope::Current,
            structural_filter: ArtefactStructuralFilter {
                kind: Some(ArtefactKindFilter::Function),
                symbol_fqn: Some("packages/api/src/lib.rs::run".to_string()),
                lines: Some(ArtefactLineRange { start: 10, end: 20 }),
            },
            activity_filter: Some(ArtefactActivityFilter {
                agent: Some("codex".to_string()),
                since: Some("2026-03-20T00:00:00Z".to_string()),
            }),
            pagination: Some(ArtefactPagination::forward(None, 25)),
        }
    }

    #[test]
    fn filtered_artefacts_cte_uses_projection_exists_for_activity_filters() {
        let sql = build_filtered_artefacts_cte_sql(&current_activity_spec());

        assert!(sql.contains("WITH filtered AS"));
        assert!(sql.contains("FROM artefacts_current a"));
        assert!(sql.contains("EXISTS (SELECT 1 FROM checkpoint_files cf WHERE"));
        assert!(sql.contains("cf.repo_id = 'repo-1'"));
        assert!(sql.contains("cf.path_after = a.path"));
        assert!(sql.contains("cf.blob_sha_after = a.content_id"));
        assert!(sql.contains("cf.agent = 'codex'"));
        assert!(sql.contains("cf.event_time >= '2026-03-20T00:00:00Z'"));
        assert!(sql.contains("a.path = './packages/api/src/lib.rs'"));
        assert!(sql.contains("a.path = 'packages/api/src/lib.rs'"));
        assert!(sql.contains("a.symbol_fqn = 'packages/api/src/lib.rs::run'"));
        assert!(!sql.contains("a.branch ="));
        assert!(!sql.contains("blob_sha IN"));
    }

    #[test]
    fn filtered_artefacts_select_orders_by_filtered_relation() {
        let sql = build_filtered_artefacts_select_sql(&current_activity_spec());

        assert!(sql.contains("FROM filtered"));
        assert!(sql.contains("summary"));
        assert!(sql.contains("embedding_representations"));
        assert!(sql.contains("FROM symbol_embeddings_current se"));
        assert!(sql.contains("FROM symbol_semantics_current ss"));
        assert!(sql.contains("FROM symbol_semantics hs"));
        assert!(sql.contains("hs.blob_sha ="));
        assert!(sql.contains("content_id"));
        assert!(sql.contains("ORDER BY path, kind_rank, start_line, end_line, artefact_id"));
        assert!(!sql.contains("blob_sha IN"));
    }

    #[test]
    fn filtered_artefacts_cte_uses_resolved_historical_blob_for_file_scopes() {
        let sql = build_filtered_artefacts_cte_sql(&ArtefactQuerySpec {
            repo_id: "repo-1".to_string(),
            branch: None,
            historical_path_blob_sha: Some("blob-123".to_string()),
            scope: ArtefactScope {
                project_path: None,
                path: Some("src/main.rs".to_string()),
                files_path: None,
            },
            temporal_scope: ArtefactTemporalScope::HistoricalCommit {
                commit_sha: "commit-123".to_string(),
            },
            structural_filter: ArtefactStructuralFilter::default(),
            activity_filter: None,
            pagination: None,
        });

        assert!(sql.contains("FROM artefacts_historical a"));
        assert!(sql.contains("a.path = 'src/main.rs'"));
        assert!(sql.contains("a.blob_sha = 'blob-123'"));
        assert!(sql.contains("FROM symbol_semantics ss"));
        assert!(!sql.contains("FROM file_state fs"));
    }
}
