use crate::artefact_query_planner::{ArtefactPaginationDirection, ArtefactQuerySpec};
use crate::graphql::ResolvedTemporalScope;
use crate::graphql::types::{DepsDirection, DepsFilterInput};
use crate::host::devql::artefact_sql::{
    build_filtered_artefacts_cte_sql, build_filtered_artefacts_select_sql,
    filtered_artefact_columns_sql, filtered_artefact_order_sql,
};
use crate::host::devql::{esc_pg, escape_like_pattern, glob_to_sql_like, sql_like_with_escape};
use std::path::{Component, Path};

pub(super) fn build_file_context_lookup_sql(
    repo_id: &str,
    _branch: &str,
    path: &str,
    temporal_scope: Option<&ResolvedTemporalScope>,
) -> String {
    if temporal_scope.is_some_and(ResolvedTemporalScope::use_historical_tables) {
        let commit_sha = temporal_scope
            .expect("historical temporal scope must exist")
            .resolved_commit();
        return format!(
            "SELECT fs.path AS path, fs.blob_sha AS blob_sha, \
                    (SELECT a.language FROM artefacts_historical a \
                      WHERE a.repo_id = fs.repo_id AND a.path = fs.path AND a.blob_sha = fs.blob_sha \
                      ORDER BY a.start_line, a.artefact_id LIMIT 1) AS language \
               FROM file_state fs \
              WHERE fs.repo_id = '{repo_id}' AND fs.commit_sha = '{commit_sha}' AND fs.path = '{path}' \
              LIMIT 1",
            repo_id = esc_pg(repo_id),
            commit_sha = esc_pg(commit_sha),
            path = esc_pg(path),
        );
    }

    if temporal_scope
        .and_then(ResolvedTemporalScope::save_revision)
        .is_some()
    {
        // Save revision scoping relied on columns (revision_kind, revision_id) that no
        // longer exist on artefacts_current. Fall through to the default current lookup.
    }

    format!(
        "SELECT path, blob_sha, language FROM ( \
            SELECT c.path AS path, c.effective_content_id AS blob_sha, \
                   (SELECT a.language FROM artefacts_current a \
                    WHERE a.repo_id = c.repo_id AND a.path = c.path \
                    ORDER BY a.start_line, a.artefact_id LIMIT 1) AS language, \
                   0 AS precedence \
              FROM current_file_state c \
             WHERE c.repo_id = '{repo_id}' AND c.path = '{path}' \
            UNION ALL \
            SELECT a.path AS path, a.content_id AS blob_sha, a.language AS language, 1 AS precedence \
              FROM artefacts_current a \
             WHERE a.repo_id = '{repo_id}' AND a.path = '{path}' \
        ) \
        ORDER BY precedence \
        LIMIT 1",
        repo_id = esc_pg(repo_id),
        path = esc_pg(path),
    )
}

pub(super) fn build_file_context_list_sql(
    repo_id: &str,
    _branch: &str,
    glob: &str,
    temporal_scope: Option<&ResolvedTemporalScope>,
) -> String {
    let like = glob_to_sql_like(glob);
    let like_fs = sql_like_with_escape("fs.path", &like);
    let like_c = sql_like_with_escape("c.path", &like);
    let like_a = sql_like_with_escape("a.path", &like);
    if temporal_scope.is_some_and(ResolvedTemporalScope::use_historical_tables) {
        let commit_sha = temporal_scope
            .expect("historical temporal scope must exist")
            .resolved_commit();
        return format!(
            "SELECT fs.path AS path, fs.blob_sha AS blob_sha, \
                    (SELECT a.language FROM artefacts_historical a \
                      WHERE a.repo_id = fs.repo_id AND a.path = fs.path AND a.blob_sha = fs.blob_sha \
                      ORDER BY a.start_line, a.artefact_id LIMIT 1) AS language \
               FROM file_state fs \
              WHERE fs.repo_id = '{repo_id}' AND fs.commit_sha = '{commit_sha}' AND {like_fs} \
              ORDER BY fs.path",
            repo_id = esc_pg(repo_id),
            commit_sha = esc_pg(commit_sha),
            like_fs = like_fs,
        );
    }

    if temporal_scope
        .and_then(ResolvedTemporalScope::save_revision)
        .is_some()
    {
        // Fall through to default current lookup — revision columns removed from current tables.
    }

    format!(
        "SELECT path, blob_sha, MIN(language) AS language \
           FROM ( \
                SELECT c.path AS path, c.effective_content_id AS blob_sha, \
                       (SELECT a.language FROM artefacts_current a \
                        WHERE a.repo_id = c.repo_id AND a.path = c.path \
                        ORDER BY a.start_line, a.artefact_id LIMIT 1) AS language \
                  FROM current_file_state c \
                 WHERE c.repo_id = '{repo_id}' AND {like_c} \
                UNION ALL \
                SELECT a.path AS path, a.content_id AS blob_sha, a.language AS language \
                  FROM artefacts_current a \
                 WHERE a.repo_id = '{repo_id}' AND {like_a} \
           ) files \
       GROUP BY path, blob_sha \
       ORDER BY path",
        repo_id = esc_pg(repo_id),
        like_c = like_c,
        like_a = like_a,
    )
}

pub(super) fn build_current_artefacts_sql(spec: &ArtefactQuerySpec) -> String {
    build_filtered_artefacts_select_sql(spec)
}

pub(super) fn build_current_artefacts_count_sql(spec: &ArtefactQuerySpec) -> String {
    let filtered_cte = build_filtered_artefacts_cte_sql(spec);
    format!("{filtered_cte} SELECT COUNT(*) AS total_count FROM filtered")
}

pub(super) fn build_current_artefacts_cursor_exists_sql(
    spec: &ArtefactQuerySpec,
    cursor: &str,
) -> String {
    let filtered_cte = build_filtered_artefacts_cte_sql(spec);
    format!(
        "{filtered_cte} \
         SELECT 1 AS cursor_match \
           FROM filtered \
          WHERE artefact_id = '{cursor}' \
          LIMIT 1",
        cursor = esc_pg(cursor),
    )
}

pub(super) fn build_current_artefacts_window_sql(spec: &ArtefactQuerySpec) -> String {
    let filtered_cte = build_filtered_artefacts_cte_sql(spec);
    let pagination = spec
        .pagination
        .as_ref()
        .expect("artefact window queries require pagination in the shared spec");
    let (pagination_clause, order) = match pagination.direction {
        ArtefactPaginationDirection::Forward => (
            pagination
                .after
                .as_deref()
                .map_or_else(String::new, |cursor| {
                    format!(
                        " WHERE (path, kind_rank, start_line, end_line, artefact_id) > \
                            (SELECT path, kind_rank, start_line, end_line, artefact_id \
                               FROM filtered \
                              WHERE artefact_id = '{cursor}')",
                        cursor = esc_pg(cursor),
                    )
                }),
            filtered_artefact_order_sql().to_string(),
        ),
        ArtefactPaginationDirection::Backward => (
            pagination
                .before
                .as_deref()
                .map_or_else(String::new, |cursor| {
                    format!(
                        " WHERE (path, kind_rank, start_line, end_line, artefact_id) < \
                            (SELECT path, kind_rank, start_line, end_line, artefact_id \
                               FROM filtered \
                              WHERE artefact_id = '{cursor}')",
                        cursor = esc_pg(cursor),
                    )
                }),
            filtered_artefact_reverse_order_sql().to_string(),
        ),
    };

    format!(
        "{filtered_cte} \
         SELECT {columns} \
           FROM filtered{pagination_clause} \
       ORDER BY {order} \
          LIMIT {limit}",
        columns = filtered_artefact_columns_sql(spec.temporal_scope.use_historical_tables(),),
        order = order,
        limit = pagination.limit,
    )
}

fn filtered_artefact_reverse_order_sql() -> &'static str {
    "path DESC, kind_rank DESC, start_line DESC, end_line DESC, artefact_id DESC"
}

pub(super) fn build_artefacts_by_ids_sql(
    repo_id: &str,
    _branch: &str,
    artefact_ids: &[String],
    project_path: Option<&str>,
    temporal_scope: Option<&ResolvedTemporalScope>,
) -> String {
    let use_historical_tables =
        temporal_scope.is_some_and(ResolvedTemporalScope::use_historical_tables);
    let mut clauses = vec![
        format!("a.repo_id = '{}'", esc_pg(repo_id)),
        format!("a.artefact_id IN ({})", quoted_string_list(artefact_ids)),
    ];
    if use_historical_tables
        && let Some(commit_sha) = temporal_scope
            .filter(|scope| scope.use_historical_tables())
            .map(ResolvedTemporalScope::resolved_commit)
    {
        clauses.push(file_state_exists_clause(
            "a.path",
            "a.blob_sha",
            repo_id,
            commit_sha,
        ));
    }
    if let Some(project_path) = project_path {
        clauses.push(repo_path_prefix_clause("a.path", project_path));
    }
    format!(
        "SELECT {} \
           FROM {} a \
          WHERE {} \
       ORDER BY {}",
        artefact_select_columns_sql("a", use_historical_tables),
        artefacts_table_sql(use_historical_tables),
        clauses.join(" AND "),
        artefact_order_sql("a"),
    )
}

pub(super) fn build_child_artefacts_sql(
    repo_id: &str,
    _branch: &str,
    parent_artefact_id: &str,
    project_path: Option<&str>,
    temporal_scope: Option<&ResolvedTemporalScope>,
) -> String {
    let use_historical_tables =
        temporal_scope.is_some_and(ResolvedTemporalScope::use_historical_tables);
    let mut clauses = vec![
        format!("a.repo_id = '{}'", esc_pg(repo_id)),
        format!("a.parent_artefact_id = '{}'", esc_pg(parent_artefact_id)),
    ];
    if use_historical_tables
        && let Some(commit_sha) = temporal_scope
            .filter(|scope| scope.use_historical_tables())
            .map(ResolvedTemporalScope::resolved_commit)
    {
        clauses.push(file_state_exists_clause(
            "a.path",
            "a.blob_sha",
            repo_id,
            commit_sha,
        ));
    }
    if let Some(project_path) = project_path {
        clauses.push(repo_path_prefix_clause("a.path", project_path));
    }
    format!(
        "SELECT {} \
           FROM {} a \
          WHERE {} \
       ORDER BY {}",
        artefact_select_columns_sql("a", use_historical_tables),
        artefacts_table_sql(use_historical_tables),
        clauses.join(" AND "),
        artefact_order_sql("a"),
    )
}

pub(super) fn build_current_dependency_sql(
    repo_id: &str,
    _branch: &str,
    scope: DependencyScope<'_>,
    project_path: Option<&str>,
    filter: DepsFilterInput,
    temporal_scope: Option<&ResolvedTemporalScope>,
) -> String {
    let use_historical_tables =
        temporal_scope.is_some_and(ResolvedTemporalScope::use_historical_tables);
    let mut clauses = vec![format!("e.repo_id = '{}'", esc_pg(repo_id))];
    if !use_historical_tables {
        // branch column removed from artefact_edges_current in sync redesign
    }
    if use_historical_tables
        && let Some(revision_id) = temporal_scope.and_then(ResolvedTemporalScope::save_revision)
    {
        clauses.push("e.revision_kind = 'temporary'".to_string());
        clauses.push(format!("e.revision_id = '{}'", esc_pg(revision_id)));
    }

    if let Some(kind) = filter.kind {
        clauses.push(format!(
            "e.edge_kind = '{}'",
            esc_pg(kind.as_storage_value())
        ));
    }
    if !filter.include_unresolved {
        clauses.push("e.to_artefact_id IS NOT NULL".to_string());
    }

    match (scope, filter.direction) {
        (DependencyScope::File(path), DepsDirection::Out) => {
            clauses.push(format!("src.path = '{}'", esc_pg(path)));
        }
        (DependencyScope::File(path), DepsDirection::In) => {
            clauses.push(format!("tgt.path = '{}'", esc_pg(path)));
        }
        (DependencyScope::File(path), DepsDirection::Both) => {
            clauses.push(format!(
                "(src.path = '{}' OR tgt.path = '{}')",
                esc_pg(path),
                esc_pg(path)
            ));
        }
        (DependencyScope::Project(path), DepsDirection::Out) => {
            clauses.push(repo_path_prefix_clause("src.path", path));
        }
        (DependencyScope::Project(path), DepsDirection::In) => {
            clauses.push(repo_path_prefix_clause("tgt.path", path));
        }
        (DependencyScope::Project(path), DepsDirection::Both) => {
            clauses.push(repo_path_either_clause("src.path", "tgt.path", path));
        }
    }

    if use_historical_tables
        && let Some(revision_id) = temporal_scope.and_then(ResolvedTemporalScope::save_revision)
    {
        match filter.direction {
            DepsDirection::Out => clauses.push(current_revision_clause("src", revision_id)),
            DepsDirection::In => clauses.push(current_revision_clause("tgt", revision_id)),
            DepsDirection::Both => clauses.push(format!(
                "({} OR {})",
                current_revision_clause("src", revision_id),
                current_revision_clause("tgt", revision_id),
            )),
        }
    }

    if let Some(project_path) = project_path {
        match filter.direction {
            DepsDirection::Out => clauses.push(repo_path_prefix_clause("src.path", project_path)),
            DepsDirection::In => clauses.push(repo_path_prefix_clause("tgt.path", project_path)),
            DepsDirection::Both => clauses.push(repo_path_either_clause(
                "src.path",
                "tgt.path",
                project_path,
            )),
        }
    }

    if let Some(scope) = temporal_scope.filter(|scope| scope.use_historical_tables()) {
        match filter.direction {
            DepsDirection::Out => clauses.push(file_state_exists_clause(
                "src.path",
                "src.blob_sha",
                repo_id,
                scope.resolved_commit(),
            )),
            DepsDirection::In => clauses.push(file_state_exists_clause(
                "tgt.path",
                "tgt.blob_sha",
                repo_id,
                scope.resolved_commit(),
            )),
            DepsDirection::Both => clauses.push(format!(
                "({} OR {})",
                file_state_exists_clause(
                    "src.path",
                    "src.blob_sha",
                    repo_id,
                    scope.resolved_commit(),
                ),
                file_state_exists_clause(
                    "tgt.path",
                    "tgt.blob_sha",
                    repo_id,
                    scope.resolved_commit(),
                ),
            )),
        }
    }

    let src_join = if use_historical_tables {
        format!(
            "JOIN {artefacts_table} src ON src.repo_id = e.repo_id AND src.artefact_id = e.from_artefact_id AND src.blob_sha = e.blob_sha",
            artefacts_table = artefacts_table_sql(true),
        )
    } else {
        format!(
            "JOIN {artefacts_table} src ON src.repo_id = e.repo_id AND src.artefact_id = e.from_artefact_id",
            artefacts_table = artefacts_table_sql(false),
        )
    };
    let tgt_join = if use_historical_tables {
        let commit_sha = temporal_scope
            .filter(|s| s.use_historical_tables())
            .map(ResolvedTemporalScope::resolved_commit)
            .expect("historical dependency queries require a resolved commit");
        historical_dependency_tgt_join_sql(repo_id, commit_sha)
    } else {
        format!(
            "LEFT JOIN {artefacts_table} tgt ON tgt.repo_id = e.repo_id AND tgt.artefact_id = e.to_artefact_id",
            artefacts_table = artefacts_table_sql(false),
        )
    };

    format!(
        "SELECT e.edge_id, e.edge_kind, e.language, e.from_artefact_id, e.to_artefact_id, \
                e.to_symbol_ref, e.start_line, e.end_line, e.metadata, \
                src.path AS from_path, src.symbol_fqn AS from_symbol_fqn, \
                tgt.path AS to_path, tgt.symbol_fqn AS to_symbol_fqn \
           FROM {edges_table} e \
           {src_join} \
           {tgt_join} \
          WHERE {clauses} \
       ORDER BY src.path, COALESCE(e.start_line, 0), COALESCE(e.end_line, 0), \
                e.edge_kind, COALESCE(tgt.path, ''), e.edge_id",
        edges_table = if use_historical_tables {
            "artefact_edges"
        } else {
            "artefact_edges_current"
        },
        src_join = src_join,
        tgt_join = tgt_join,
        clauses = clauses.join(" AND ")
    )
}

pub(super) fn build_current_dependency_batch_sql(
    repo_id: &str,
    _branch: &str,
    artefact_ids: &[String],
    direction: DepsDirection,
    filter: DepsFilterInput,
    project_path: Option<&str>,
    temporal_scope: Option<&ResolvedTemporalScope>,
) -> String {
    let use_historical_tables =
        temporal_scope.is_some_and(ResolvedTemporalScope::use_historical_tables);
    let mut clauses = vec![format!("e.repo_id = '{}'", esc_pg(repo_id))];
    if !use_historical_tables {
        // branch column removed from artefact_edges_current in sync redesign
    }

    if let Some(kind) = filter.kind {
        clauses.push(format!(
            "e.edge_kind = '{}'",
            esc_pg(kind.as_storage_value())
        ));
    }
    if !filter.include_unresolved {
        clauses.push("e.to_artefact_id IS NOT NULL".to_string());
    }

    let owner_column = match direction {
        DepsDirection::Out => "e.from_artefact_id",
        DepsDirection::In => "e.to_artefact_id",
        DepsDirection::Both => {
            unreachable!("batch dependency loader only supports a single direction")
        }
    };
    clauses.push(format!(
        "{owner_column} IN ({})",
        quoted_string_list(artefact_ids)
    ));
    if let Some(project_path) = project_path {
        match direction {
            DepsDirection::Out => clauses.push(repo_path_prefix_clause("src.path", project_path)),
            DepsDirection::In => clauses.push(repo_path_prefix_clause("tgt.path", project_path)),
            DepsDirection::Both => clauses.push(repo_path_either_clause(
                "src.path",
                "tgt.path",
                project_path,
            )),
        }
    }

    let src_join = if use_historical_tables {
        format!(
            "JOIN {artefacts_table} src ON src.repo_id = e.repo_id AND src.artefact_id = e.from_artefact_id AND src.blob_sha = e.blob_sha",
            artefacts_table = artefacts_table_sql(true),
        )
    } else {
        format!(
            "JOIN {artefacts_table} src ON src.repo_id = e.repo_id AND src.artefact_id = e.from_artefact_id",
            artefacts_table = artefacts_table_sql(false),
        )
    };
    let tgt_join = if use_historical_tables {
        let commit_sha = temporal_scope
            .filter(|s| s.use_historical_tables())
            .map(ResolvedTemporalScope::resolved_commit)
            .expect("historical batch dependency queries require a resolved commit");
        historical_dependency_tgt_join_sql(repo_id, commit_sha)
    } else {
        format!(
            "LEFT JOIN {artefacts_table} tgt ON tgt.repo_id = e.repo_id AND tgt.artefact_id = e.to_artefact_id",
            artefacts_table = artefacts_table_sql(false),
        )
    };

    format!(
        "SELECT {owner_column} AS owner_artefact_id, \
                e.edge_id, e.edge_kind, e.language, e.from_artefact_id, e.to_artefact_id, \
                e.to_symbol_ref, e.start_line, e.end_line, e.metadata, \
                src.path AS from_path, src.symbol_fqn AS from_symbol_fqn, \
                tgt.path AS to_path, tgt.symbol_fqn AS to_symbol_fqn \
           FROM {edges_table} e \
           {src_join} \
           {tgt_join} \
          WHERE {clauses} \
       ORDER BY owner_artefact_id, src.path, COALESCE(e.start_line, 0), \
                COALESCE(e.end_line, 0), e.edge_kind, COALESCE(tgt.path, ''), e.edge_id",
        edges_table = if use_historical_tables {
            "artefact_edges"
        } else {
            "artefact_edges_current"
        },
        src_join = src_join,
        tgt_join = tgt_join,
        clauses = clauses.join(" AND ")
    )
}

pub(super) fn normalise_repo_relative_path(
    path: &str,
    allow_glob: bool,
) -> std::result::Result<String, String> {
    let normalized = path.trim().replace('\\', "/");
    if normalized.is_empty() {
        return Err("path cannot be empty".to_string());
    }
    if normalized.starts_with('/') {
        return Err("path must be relative to the repository root".to_string());
    }

    let mut components = Vec::new();
    for component in Path::new(&normalized).components() {
        match component {
            Component::CurDir => {}
            Component::Normal(value) => {
                components.push(
                    value
                        .to_str()
                        .ok_or_else(|| "path must be valid UTF-8".to_string())?
                        .to_string(),
                );
            }
            Component::ParentDir => {
                return Err("path must not contain parent path traversal".to_string());
            }
            Component::RootDir | Component::Prefix(_) => {
                return Err("path must be relative to the repository root".to_string());
            }
        }
    }

    if components.is_empty() {
        return Err("path cannot be empty".to_string());
    }

    let normalized = components.join("/");

    if !allow_glob && (normalized.contains('*') || normalized.contains('?')) {
        return Err("single-file lookups do not accept glob patterns".to_string());
    }

    Ok(normalized)
}

#[derive(Debug, Clone, Copy)]
pub(super) enum DependencyScope<'a> {
    File(&'a str),
    Project(&'a str),
}

fn current_revision_clause(alias: &str, revision_id: &str) -> String {
    format!(
        "{alias}.revision_kind = 'temporary' AND {alias}.revision_id = '{}'",
        esc_pg(revision_id),
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

/// Historical `artefacts_historical` can store multiple rows per `artefact_id` (different blobs).
/// Joining the dependency target only on `artefact_id` duplicates rows or picks an arbitrary snapshot.
/// Pin `tgt` to the tree at `commit_sha` by requiring `(tgt.path, tgt.blob_sha)` in `file_state`.
fn historical_dependency_tgt_join_sql(repo_id: &str, commit_sha: &str) -> String {
    format!(
        "LEFT JOIN {artefacts_table} tgt ON tgt.repo_id = e.repo_id AND tgt.artefact_id = e.to_artefact_id \
         AND {file_state_match}",
        artefacts_table = artefacts_table_sql(true),
        file_state_match =
            file_state_exists_clause("tgt.path", "tgt.blob_sha", repo_id, commit_sha),
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

fn repo_path_either_clause(source_column: &str, target_column: &str, project_path: &str) -> String {
    format!(
        "({} OR {})",
        repo_path_prefix_clause(source_column, project_path),
        repo_path_prefix_clause(target_column, project_path),
    )
}

fn quoted_string_list(values: &[String]) -> String {
    values
        .iter()
        .map(|value| format!("'{}'", esc_pg(value)))
        .collect::<Vec<_>>()
        .join(", ")
}

fn artefacts_table_sql(use_historical_tables: bool) -> &'static str {
    if use_historical_tables {
        "artefacts_historical"
    } else {
        "artefacts_current"
    }
}

fn artefact_select_columns_sql(alias: &str, use_historical_tables: bool) -> String {
    let created_at_column = if use_historical_tables {
        format!("{alias}.created_at AS created_at")
    } else {
        format!("{alias}.updated_at AS created_at")
    };
    let (blob_sha_expr, content_hash_expr) = if use_historical_tables {
        (format!("{alias}.blob_sha"), format!("{alias}.content_hash"))
    } else {
        (
            format!("{alias}.content_id AS blob_sha"),
            "NULL AS content_hash".to_string(),
        )
    };
    let summary_expr = if use_historical_tables {
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
    };
    let embedding_representations_expr = if use_historical_tables {
        format!(
            "CASE \
               WHEN EXISTS (SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'symbol_embeddings') \
               THEN COALESCE((SELECT json_group_array(representation_kind) \
                   FROM (SELECT DISTINCT se.representation_kind AS representation_kind \
                           FROM symbol_embeddings se \
                          WHERE se.repo_id = {alias}.repo_id \
                            AND se.artefact_id = {alias}.artefact_id \
                            AND se.blob_sha = {alias}.blob_sha \
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
             END"
        )
    } else {
        format!(
            "CASE \
               WHEN EXISTS (SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'symbol_embeddings_current') \
               THEN COALESCE((SELECT json_group_array(representation_kind) \
                   FROM (SELECT DISTINCT se.representation_kind AS representation_kind \
                           FROM symbol_embeddings_current se \
                          WHERE se.repo_id = {alias}.repo_id \
                            AND se.artefact_id = {alias}.artefact_id \
                            AND se.content_id = {alias}.content_id \
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
             END"
        )
    };
    format!(
        "{alias}.symbol_id, {alias}.artefact_id, {alias}.path, {alias}.language, \
         {alias}.canonical_kind, {alias}.language_kind, {alias}.symbol_fqn, \
         {alias}.parent_artefact_id, {alias}.start_line, {alias}.end_line, \
         {alias}.start_byte, {alias}.end_byte, {alias}.signature, {alias}.modifiers, \
         {alias}.docstring, {summary_expr} AS summary, \
         {embedding_representations_expr} AS embedding_representations, \
         {blob_sha_expr}, {content_hash_expr}, {created_at_column}",
    )
}

fn artefact_kind_rank_sql(alias: &str) -> String {
    format!("CASE WHEN {alias}.canonical_kind = 'file' THEN 0 ELSE 1 END")
}

fn artefact_order_sql(alias: &str) -> String {
    format!(
        "{alias}.path, {}, {alias}.start_line, {alias}.end_line, {alias}.artefact_id",
        artefact_kind_rank_sql(alias),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::artefact_query_planner::{
        ArtefactActivityFilter, ArtefactPagination, ArtefactScope, ArtefactStructuralFilter,
        ArtefactTemporalScope,
    };
    use crate::graphql::types::{DepsDirection, DepsFilterInput};
    use crate::graphql::{ResolvedTemporalScope, TemporalAccessMode};

    fn activity_spec() -> ArtefactQuerySpec {
        ArtefactQuerySpec {
            repo_id: "repo-1".to_string(),
            branch: Some("main".to_string()),
            historical_path_blob_sha: None,
            scope: ArtefactScope {
                project_path: Some("packages/api".to_string()),
                path: Some("packages/api/src/lib.rs".to_string()),
                files_path: None,
            },
            temporal_scope: ArtefactTemporalScope::Current,
            structural_filter: ArtefactStructuralFilter::default(),
            activity_filter: Some(ArtefactActivityFilter {
                agent: Some("codex".to_string()),
                since: Some("2026-03-20T00:00:00Z".to_string()),
            }),
            pagination: Some(ArtefactPagination::forward(Some("cursor-1"), 11)),
        }
    }

    #[test]
    fn count_sql_uses_projection_backed_filtered_relation() {
        let sql = build_current_artefacts_count_sql(&activity_spec());

        assert!(sql.contains("WITH filtered AS"));
        assert!(sql.contains("FROM checkpoint_files cf"));
        assert!(sql.contains("cf.path_after = a.path"));
        assert!(sql.contains("cf.blob_sha_after = a.content_id"));
        assert!(sql.contains("cf.agent = 'codex'"));
        assert!(sql.contains("SELECT COUNT(*) AS total_count FROM filtered"));
        assert!(!sql.contains("blob_sha IN"));
    }

    #[test]
    fn cursor_exists_sql_validates_against_activity_filtered_relation() {
        let sql = build_current_artefacts_cursor_exists_sql(&activity_spec(), "cursor-1");

        assert!(sql.contains("WITH filtered AS"));
        assert!(sql.contains("FROM checkpoint_files cf"));
        assert!(sql.contains("FROM filtered"));
        assert!(sql.contains("WHERE artefact_id = 'cursor-1'"));
        assert!(!sql.contains("blob_sha IN"));
    }

    #[test]
    fn window_sql_pages_over_activity_filtered_relation() {
        let sql = build_current_artefacts_window_sql(&activity_spec());

        assert!(sql.contains("WITH filtered AS"));
        assert!(sql.contains("FROM checkpoint_files cf"));
        assert!(sql.contains("FROM filtered"));
        assert!(sql.contains("ORDER BY path, kind_rank, start_line, end_line, artefact_id"));
        assert!(sql.contains("LIMIT 11"));
        assert!(!sql.contains("blob_sha IN"));
    }

    #[test]
    fn current_select_fields_sql_falls_back_to_historical_summaries() {
        let sql = artefact_select_columns_sql("a", false);

        assert!(sql.contains("FROM symbol_semantics_current ss"));
        assert!(sql.contains("FROM symbol_semantics hs"));
        assert!(sql.contains("hs.blob_sha = a.content_id"));
        assert!(sql.contains("FROM symbol_embeddings_current se"));
        assert!(sql.contains("embedding_representations"));
    }

    #[test]
    fn backward_window_sql_flips_tuple_comparison_and_order() {
        let mut spec = activity_spec();
        spec.pagination = Some(ArtefactPagination::backward(Some("cursor-2"), 7));

        let sql = build_current_artefacts_window_sql(&spec);

        assert!(sql.contains("WITH filtered AS"));
        assert!(sql.contains("FROM filtered"));
        assert!(sql.contains("WHERE (path, kind_rank, start_line, end_line, artefact_id) <"));
        assert!(sql.contains("WHERE artefact_id = 'cursor-2'"));
        assert!(sql.contains(
            "ORDER BY path DESC, kind_rank DESC, start_line DESC, end_line DESC, artefact_id DESC"
        ));
        assert!(sql.contains("LIMIT 7"));
    }

    #[test]
    fn historical_dependency_sql_pins_tgt_to_file_state_at_commit() {
        let scope =
            ResolvedTemporalScope::new("abc123".to_string(), TemporalAccessMode::HistoricalCommit);
        let sql = build_current_dependency_sql(
            "repo-1",
            "main",
            DependencyScope::File("src/a.rs"),
            None,
            DepsFilterInput::default(),
            Some(&scope),
        );
        assert!(sql.contains("LEFT JOIN artefacts_historical tgt"));
        assert!(sql.contains("tgt.artefact_id = e.to_artefact_id"));
        assert!(sql.contains("fs.path = tgt.path"));
        assert!(sql.contains("fs.blob_sha = tgt.blob_sha"));
        assert!(sql.contains("fs.commit_sha = 'abc123'"));
    }

    #[test]
    fn historical_dependency_batch_sql_pins_tgt_to_file_state_at_commit() {
        let scope = ResolvedTemporalScope::new(
            "deadbeef".to_string(),
            TemporalAccessMode::HistoricalCommit,
        );
        let sql = build_current_dependency_batch_sql(
            "repo-1",
            "main",
            &["a1".to_string()],
            DepsDirection::Out,
            DepsFilterInput::default(),
            None,
            Some(&scope),
        );
        assert!(sql.contains("LEFT JOIN artefacts_historical tgt"));
        assert!(sql.contains("fs.path = tgt.path"));
        assert!(sql.contains("fs.commit_sha = 'deadbeef'"));
    }
}
