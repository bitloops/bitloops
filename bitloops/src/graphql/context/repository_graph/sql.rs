use crate::graphql::ResolvedTemporalScope;
use crate::graphql::types::{ArtefactFilterInput, CanonicalKind, DepsDirection, DepsFilterInput};
use crate::host::devql::esc_pg;
use std::path::{Component, Path};

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
    let like = glob_to_like(glob);
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
              WHERE fs.repo_id = '{repo_id}' AND fs.commit_sha = '{commit_sha}' AND fs.path LIKE '{like}' \
              ORDER BY fs.path",
            repo_id = esc_pg(repo_id),
            commit_sha = esc_pg(commit_sha),
            like = esc_pg(&like),
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
                AND a.path LIKE '{like}' \
           GROUP BY a.path, a.blob_sha \
           ORDER BY a.path",
            repo_id = esc_pg(repo_id),
            branch = esc_pg(branch),
            revision_id = esc_pg(revision_id),
            like = esc_pg(&like),
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
                 WHERE c.repo_id = '{repo_id}' AND c.path LIKE '{like}' \
                UNION ALL \
                SELECT a.path AS path, a.blob_sha AS blob_sha, a.language AS language \
                  FROM artefacts_current a \
                 WHERE a.repo_id = '{repo_id}' AND a.branch = '{branch}' AND a.path LIKE '{like}' \
           ) files \
       GROUP BY path, blob_sha \
       ORDER BY path",
        branch = esc_pg(branch),
        repo_id = esc_pg(repo_id),
        like = esc_pg(&like),
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

    format!(
        "{} WHERE {} ORDER BY a.path, \
         CASE WHEN a.canonical_kind = 'file' THEN 0 ELSE 1 END, \
         a.start_line, a.end_line, a.artefact_id",
        if use_historical_tables {
            historical_artefact_select_sql()
        } else {
            current_artefact_select_sql()
        },
        clauses.join(" AND ")
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
        "{} WHERE {} \
           ORDER BY a.path, a.start_line, a.artefact_id",
        if use_historical_tables {
            historical_artefact_select_sql()
        } else {
            current_artefact_select_sql()
        },
        clauses.join(" AND "),
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
        "{} WHERE {} \
           ORDER BY a.path, a.start_line, a.artefact_id",
        if use_historical_tables {
            historical_artefact_select_sql()
        } else {
            current_artefact_select_sql()
        },
        clauses.join(" AND "),
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

fn current_artefact_select_sql() -> &'static str {
    "SELECT a.symbol_id, a.artefact_id, a.path, a.language, a.canonical_kind, a.language_kind, \
            a.symbol_fqn, a.parent_artefact_id, a.start_line, a.end_line, a.start_byte, \
            a.end_byte, a.signature, a.modifiers, a.docstring, a.blob_sha, a.content_hash, \
            a.updated_at AS created_at \
       FROM artefacts_current a"
}

fn historical_artefact_select_sql() -> &'static str {
    "SELECT a.symbol_id, a.artefact_id, a.path, a.language, a.canonical_kind, a.language_kind, \
            a.symbol_fqn, a.parent_artefact_id, a.start_line, a.end_line, a.start_byte, \
            a.end_byte, a.signature, a.modifiers, a.docstring, a.blob_sha, a.content_hash, \
            a.created_at AS created_at \
       FROM artefacts a"
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

fn glob_to_like(glob: &str) -> String {
    glob.replace("**", "%").replace('*', "%").replace('?', "_")
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
    format!(
        "({column} = '{path}' OR {column} LIKE '{prefix}')",
        column = column,
        path = esc_pg(project_path),
        prefix = esc_pg(&format!("{project_path}/%")),
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
