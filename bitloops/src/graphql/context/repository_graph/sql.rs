use crate::graphql::ResolvedTemporalScope;
use crate::graphql::types::{ArtefactFilterInput, CanonicalKind, DepsDirection, DepsFilterInput};
use crate::host::devql::{esc_pg, escape_like_pattern, glob_to_sql_like, sql_like_with_escape};
use std::path::{Component, Path};

pub(super) struct CurrentArtefactsWindowSql<'a> {
    pub repo_id: &'a str,
    pub branch: &'a str,
    pub path: Option<&'a str>,
    pub project_path: Option<&'a str>,
    pub filter: Option<&'a ArtefactFilterInput>,
    pub temporal_scope: Option<&'a ResolvedTemporalScope>,
    pub after: Option<&'a str>,
    pub limit: usize,
}

pub(super) fn build_file_context_lookup_sql(
    repo_id: &str,
    branch: &str,
    path: &str,
    temporal_scope: Option<&ResolvedTemporalScope>,
) -> String {
    if temporal_scope.is_some_and(ResolvedTemporalScope::use_historical_tables) {
        let commit_sha = temporal_scope
            .expect("historical temporal scope must exist")
            .resolved_commit();
        return format!(
            "SELECT fs.path AS path, fs.blob_sha AS blob_sha, \
                    (SELECT a.language FROM artefacts a \
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

    if let Some(revision_id) = temporal_scope.and_then(ResolvedTemporalScope::save_revision) {
        return format!(
            "SELECT a.path AS path, a.blob_sha AS blob_sha, a.language AS language \
               FROM artefacts_current a \
              WHERE a.repo_id = '{repo_id}' \
                AND a.branch = '{branch}' \
                AND a.canonical_kind = 'file' \
                AND a.revision_kind = 'temporary' \
                AND a.revision_id = '{revision_id}' \
                AND a.path = '{path}' \
              ORDER BY a.updated_at DESC, a.start_line, a.artefact_id \
              LIMIT 1",
            repo_id = esc_pg(repo_id),
            branch = esc_pg(branch),
            revision_id = esc_pg(revision_id),
            path = esc_pg(path),
        );
    }

    format!(
        "SELECT path, blob_sha, language FROM ( \
            SELECT c.path AS path, c.blob_sha AS blob_sha, \
                   (SELECT a.language FROM artefacts_current a \
                    WHERE a.repo_id = c.repo_id AND a.branch = '{branch}' AND a.path = c.path \
                    ORDER BY a.start_line, a.artefact_id LIMIT 1) AS language, \
                   0 AS precedence \
              FROM current_file_state c \
             WHERE c.repo_id = '{repo_id}' AND c.path = '{path}' \
            UNION ALL \
            SELECT a.path AS path, a.blob_sha AS blob_sha, a.language AS language, 1 AS precedence \
              FROM artefacts_current a \
             WHERE a.repo_id = '{repo_id}' AND a.branch = '{branch}' AND a.path = '{path}' \
        ) \
        ORDER BY precedence \
        LIMIT 1",
        branch = esc_pg(branch),
        repo_id = esc_pg(repo_id),
        path = esc_pg(path),
    )
}

pub(super) fn build_file_context_list_sql(
    repo_id: &str,
    branch: &str,
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
                    (SELECT a.language FROM artefacts a \
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

    if let Some(revision_id) = temporal_scope.and_then(ResolvedTemporalScope::save_revision) {
        return format!(
            "SELECT a.path AS path, a.blob_sha AS blob_sha, MIN(a.language) AS language \
               FROM artefacts_current a \
              WHERE a.repo_id = '{repo_id}' \
                AND a.branch = '{branch}' \
                AND a.canonical_kind = 'file' \
                AND a.revision_kind = 'temporary' \
                AND a.revision_id = '{revision_id}' \
                AND {like_a} \
           GROUP BY a.path, a.blob_sha \
           ORDER BY a.path",
            repo_id = esc_pg(repo_id),
            branch = esc_pg(branch),
            revision_id = esc_pg(revision_id),
            like_a = like_a,
        );
    }

    format!(
        "SELECT path, blob_sha, MIN(language) AS language \
           FROM ( \
                SELECT c.path AS path, c.blob_sha AS blob_sha, \
                       (SELECT a.language FROM artefacts_current a \
                        WHERE a.repo_id = c.repo_id AND a.branch = '{branch}' AND a.path = c.path \
                        ORDER BY a.start_line, a.artefact_id LIMIT 1) AS language \
                  FROM current_file_state c \
                 WHERE c.repo_id = '{repo_id}' AND {like_c} \
                UNION ALL \
                SELECT a.path AS path, a.blob_sha AS blob_sha, a.language AS language \
                  FROM artefacts_current a \
                 WHERE a.repo_id = '{repo_id}' AND a.branch = '{branch}' AND {like_a} \
           ) files \
       GROUP BY path, blob_sha \
       ORDER BY path",
        branch = esc_pg(branch),
        repo_id = esc_pg(repo_id),
        like_c = like_c,
        like_a = like_a,
    )
}

pub(super) fn build_current_artefacts_sql(
    repo_id: &str,
    branch: &str,
    path: Option<&str>,
    project_path: Option<&str>,
    filter: Option<&ArtefactFilterInput>,
    temporal_scope: Option<&ResolvedTemporalScope>,
) -> String {
    let (use_historical_tables, clauses) =
        build_artefact_where_clauses(repo_id, branch, path, project_path, filter, temporal_scope);
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

pub(super) fn build_current_artefacts_count_sql(
    repo_id: &str,
    branch: &str,
    path: Option<&str>,
    project_path: Option<&str>,
    filter: Option<&ArtefactFilterInput>,
    temporal_scope: Option<&ResolvedTemporalScope>,
) -> String {
    let filtered_cte = build_filtered_artefacts_cte_sql(
        repo_id,
        branch,
        path,
        project_path,
        filter,
        temporal_scope,
    );
    format!("{filtered_cte} SELECT COUNT(*) AS total_count FROM filtered")
}

pub(super) fn build_current_artefacts_cursor_exists_sql(
    repo_id: &str,
    branch: &str,
    path: Option<&str>,
    project_path: Option<&str>,
    filter: Option<&ArtefactFilterInput>,
    temporal_scope: Option<&ResolvedTemporalScope>,
    cursor: &str,
) -> String {
    let filtered_cte = build_filtered_artefacts_cte_sql(
        repo_id,
        branch,
        path,
        project_path,
        filter,
        temporal_scope,
    );
    format!(
        "{filtered_cte} \
         SELECT 1 AS cursor_match \
           FROM filtered \
          WHERE artefact_id = '{cursor}' \
          LIMIT 1",
        cursor = esc_pg(cursor),
    )
}

pub(super) fn build_current_artefacts_window_sql(params: CurrentArtefactsWindowSql<'_>) -> String {
    let filtered_cte = build_filtered_artefacts_cte_sql(
        params.repo_id,
        params.branch,
        params.path,
        params.project_path,
        params.filter,
        params.temporal_scope,
    );
    let pagination_clause = params.after.map_or_else(String::new, |cursor| {
        format!(
            " WHERE (path, kind_rank, start_line, end_line, artefact_id) > \
                    (SELECT path, kind_rank, start_line, end_line, artefact_id \
                       FROM filtered \
                      WHERE artefact_id = '{cursor}')",
            cursor = esc_pg(cursor),
        )
    });

    format!(
        "{filtered_cte} \
         SELECT {columns} \
           FROM filtered{pagination_clause} \
       ORDER BY {order} \
          LIMIT {limit}",
        columns = filtered_artefact_columns_sql(),
        order = filtered_artefact_order_sql(),
        limit = params.limit,
    )
}

pub(super) fn build_artefacts_by_ids_sql(
    repo_id: &str,
    branch: &str,
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
    if !use_historical_tables {
        clauses.push(format!("a.branch = '{}'", esc_pg(branch)));
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
    branch: &str,
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
    if !use_historical_tables {
        clauses.push(format!("a.branch = '{}'", esc_pg(branch)));
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
    branch: &str,
    scope: DependencyScope<'_>,
    project_path: Option<&str>,
    filter: DepsFilterInput,
    temporal_scope: Option<&ResolvedTemporalScope>,
) -> String {
    let use_historical_tables =
        temporal_scope.is_some_and(ResolvedTemporalScope::use_historical_tables);
    let mut clauses = vec![format!("e.repo_id = '{}'", esc_pg(repo_id))];
    if !use_historical_tables {
        clauses.push(format!("e.branch = '{}'", esc_pg(branch)));
    }
    if let Some(revision_id) = temporal_scope.and_then(ResolvedTemporalScope::save_revision) {
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

    if let Some(revision_id) = temporal_scope.and_then(ResolvedTemporalScope::save_revision) {
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

    format!(
        "SELECT e.edge_id, e.edge_kind, e.language, e.from_artefact_id, e.to_artefact_id, \
                e.to_symbol_ref, e.start_line, e.end_line, e.metadata, \
                src.path AS from_path, src.symbol_fqn AS from_symbol_fqn, \
                tgt.path AS to_path, tgt.symbol_fqn AS to_symbol_fqn \
           FROM {edges_table} e \
           JOIN {artefacts_table} src ON src.repo_id = e.repo_id {src_branch_join} \
                                     AND src.artefact_id = e.from_artefact_id \
      LEFT JOIN {artefacts_table} tgt ON tgt.repo_id = e.repo_id {tgt_branch_join} \
                                     AND tgt.artefact_id = e.to_artefact_id \
          WHERE {clauses} \
       ORDER BY src.path, COALESCE(e.start_line, 0), COALESCE(e.end_line, 0), \
                e.edge_kind, COALESCE(tgt.path, ''), e.edge_id",
        edges_table = if use_historical_tables {
            "artefact_edges"
        } else {
            "artefact_edges_current"
        },
        artefacts_table = if use_historical_tables {
            "artefacts"
        } else {
            "artefacts_current"
        },
        src_branch_join = if use_historical_tables {
            String::new()
        } else {
            " AND src.branch = e.branch".to_string()
        },
        tgt_branch_join = if use_historical_tables {
            String::new()
        } else {
            " AND tgt.branch = e.branch".to_string()
        },
        clauses = clauses.join(" AND ")
    )
}

pub(super) fn build_current_dependency_batch_sql(
    repo_id: &str,
    branch: &str,
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
        clauses.push(format!("e.branch = '{}'", esc_pg(branch)));
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

    format!(
        "SELECT {owner_column} AS owner_artefact_id, \
                e.edge_id, e.edge_kind, e.language, e.from_artefact_id, e.to_artefact_id, \
                e.to_symbol_ref, e.start_line, e.end_line, e.metadata, \
                src.path AS from_path, src.symbol_fqn AS from_symbol_fqn, \
                tgt.path AS to_path, tgt.symbol_fqn AS to_symbol_fqn \
           FROM {edges_table} e \
           JOIN {artefacts_table} src ON src.repo_id = e.repo_id {src_branch_join} \
                                     AND src.artefact_id = e.from_artefact_id \
      LEFT JOIN {artefacts_table} tgt ON tgt.repo_id = e.repo_id {tgt_branch_join} \
                                     AND tgt.artefact_id = e.to_artefact_id \
          WHERE {clauses} \
       ORDER BY owner_artefact_id, src.path, COALESCE(e.start_line, 0), \
                COALESCE(e.end_line, 0), e.edge_kind, COALESCE(tgt.path, ''), e.edge_id",
        edges_table = if use_historical_tables {
            "artefact_edges"
        } else {
            "artefact_edges_current"
        },
        artefacts_table = if use_historical_tables {
            "artefacts"
        } else {
            "artefacts_current"
        },
        src_branch_join = if use_historical_tables {
            String::new()
        } else {
            " AND src.branch = e.branch".to_string()
        },
        tgt_branch_join = if use_historical_tables {
            String::new()
        } else {
            " AND tgt.branch = e.branch".to_string()
        },
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

pub(super) fn quote_devql_string(value: &str) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "\"\"".to_string())
}

#[derive(Debug, Clone, Copy)]
pub(super) enum DependencyScope<'a> {
    File(&'a str),
    Project(&'a str),
}

fn canonical_kind_clause(column: &str, kind: CanonicalKind) -> String {
    let values: &[&str] = match kind {
        CanonicalKind::File => &["file"],
        CanonicalKind::Namespace => &["namespace"],
        CanonicalKind::Module => &["module"],
        CanonicalKind::Import => &["import"],
        CanonicalKind::Type => &["type", "interface", "enum"],
        CanonicalKind::Interface => &["interface"],
        CanonicalKind::Enum => &["enum"],
        CanonicalKind::Callable => &["callable", "function", "method"],
        CanonicalKind::Function => &["function"],
        CanonicalKind::Method => &["method"],
        CanonicalKind::Value => &["value", "variable", "constant"],
        CanonicalKind::Variable => &["variable", "constant"],
        CanonicalKind::Member => &["member"],
        CanonicalKind::Parameter => &["parameter"],
        CanonicalKind::TypeParameter => &["type_parameter"],
        CanonicalKind::Alias => &["alias"],
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

fn build_artefact_where_clauses(
    repo_id: &str,
    branch: &str,
    path: Option<&str>,
    project_path: Option<&str>,
    filter: Option<&ArtefactFilterInput>,
    temporal_scope: Option<&ResolvedTemporalScope>,
) -> (bool, Vec<String>) {
    let use_historical_tables =
        temporal_scope.is_some_and(ResolvedTemporalScope::use_historical_tables);
    let mut clauses = vec![format!("a.repo_id = '{}'", esc_pg(repo_id))];
    if !use_historical_tables {
        clauses.push(format!("a.branch = '{}'", esc_pg(branch)));
    }
    if let Some(revision_id) = temporal_scope.and_then(ResolvedTemporalScope::save_revision) {
        clauses.push("a.revision_kind = 'temporary'".to_string());
        clauses.push(format!("a.revision_id = '{}'", esc_pg(revision_id)));
    }
    if let Some(scope) = temporal_scope.filter(|scope| scope.use_historical_tables()) {
        clauses.push(file_state_exists_clause(
            "a.path",
            "a.blob_sha",
            repo_id,
            scope.resolved_commit(),
        ));
    }

    if let Some(path) = path {
        clauses.push(format!("a.path = '{}'", esc_pg(path)));
    }
    if let Some(project_path) = project_path {
        clauses.push(repo_path_prefix_clause("a.path", project_path));
    }

    if let Some(filter) = filter {
        if let Some(kind) = filter.kind {
            clauses.push(canonical_kind_clause("a.canonical_kind", kind));
        }
        if let Some(symbol_fqn) = filter.symbol_fqn.as_deref() {
            clauses.push(format!("a.symbol_fqn = '{}'", esc_pg(symbol_fqn)));
        }
        if let Some(lines) = filter.lines.as_ref() {
            clauses.push(format!(
                "a.start_line <= {} AND a.end_line >= {}",
                lines.end, lines.start
            ));
        }
    }

    (use_historical_tables, clauses)
}

fn build_filtered_artefacts_cte_sql(
    repo_id: &str,
    branch: &str,
    path: Option<&str>,
    project_path: Option<&str>,
    filter: Option<&ArtefactFilterInput>,
    temporal_scope: Option<&ResolvedTemporalScope>,
) -> String {
    let (use_historical_tables, clauses) =
        build_artefact_where_clauses(repo_id, branch, path, project_path, filter, temporal_scope);
    format!(
        "WITH filtered AS ( \
             SELECT {}, {} AS kind_rank \
               FROM {} a \
              WHERE {} \
         )",
        artefact_select_columns_sql("a", use_historical_tables),
        artefact_kind_rank_sql("a"),
        artefacts_table_sql(use_historical_tables),
        clauses.join(" AND "),
    )
}

fn artefacts_table_sql(use_historical_tables: bool) -> &'static str {
    if use_historical_tables {
        "artefacts"
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
    format!(
        "{alias}.symbol_id, {alias}.artefact_id, {alias}.path, {alias}.language, \
         {alias}.canonical_kind, {alias}.language_kind, {alias}.symbol_fqn, \
         {alias}.parent_artefact_id, {alias}.start_line, {alias}.end_line, \
         {alias}.start_byte, {alias}.end_byte, {alias}.signature, {alias}.modifiers, \
         {alias}.docstring, {alias}.blob_sha, {alias}.content_hash, {created_at_column}",
    )
}

fn filtered_artefact_columns_sql() -> &'static str {
    "symbol_id, artefact_id, path, language, canonical_kind, language_kind, symbol_fqn, \
     parent_artefact_id, start_line, end_line, start_byte, end_byte, signature, modifiers, \
     docstring, blob_sha, content_hash, created_at"
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

fn filtered_artefact_order_sql() -> &'static str {
    "path, kind_rank, start_line, end_line, artefact_id"
}
