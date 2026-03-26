use crate::graphql::types::{ArtefactFilterInput, CanonicalKind, DepsDirection, DepsFilterInput};
use crate::host::devql::esc_pg;
use std::path::{Component, Path};

pub(super) fn build_file_context_lookup_sql(repo_id: &str, branch: &str, path: &str) -> String {
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

pub(super) fn build_file_context_list_sql(repo_id: &str, branch: &str, glob: &str) -> String {
    let like = glob_to_like(glob);
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
) -> String {
    let mut clauses = vec![
        format!("a.repo_id = '{}'", esc_pg(repo_id)),
        format!("a.branch = '{}'", esc_pg(branch)),
    ];

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
        current_artefact_select_sql(),
        clauses.join(" AND ")
    )
}

pub(super) fn build_artefacts_by_ids_sql(
    repo_id: &str,
    branch: &str,
    artefact_ids: &[String],
    project_path: Option<&str>,
) -> String {
    let mut clauses = vec![
        format!("a.repo_id = '{}'", esc_pg(repo_id)),
        format!("a.branch = '{}'", esc_pg(branch)),
        format!("a.artefact_id IN ({})", quoted_string_list(artefact_ids)),
    ];
    if let Some(project_path) = project_path {
        clauses.push(repo_path_prefix_clause("a.path", project_path));
    }
    format!(
        "{} WHERE {} \
           ORDER BY a.path, a.start_line, a.artefact_id",
        current_artefact_select_sql(),
        clauses.join(" AND "),
    )
}

pub(super) fn build_child_artefacts_sql(
    repo_id: &str,
    branch: &str,
    parent_artefact_id: &str,
    project_path: Option<&str>,
) -> String {
    let mut clauses = vec![
        format!("a.repo_id = '{}'", esc_pg(repo_id)),
        format!("a.branch = '{}'", esc_pg(branch)),
        format!("a.parent_artefact_id = '{}'", esc_pg(parent_artefact_id)),
    ];
    if let Some(project_path) = project_path {
        clauses.push(repo_path_prefix_clause("a.path", project_path));
    }
    format!(
        "{} WHERE {} \
           ORDER BY a.path, a.start_line, a.artefact_id",
        current_artefact_select_sql(),
        clauses.join(" AND "),
    )
}

pub(super) fn build_current_dependency_sql(
    repo_id: &str,
    branch: &str,
    scope: DependencyScope<'_>,
    project_path: Option<&str>,
    filter: DepsFilterInput,
) -> String {
    let mut clauses = vec![
        format!("e.repo_id = '{}'", esc_pg(repo_id)),
        format!("e.branch = '{}'", esc_pg(branch)),
    ];

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

    format!(
        "SELECT e.edge_id, e.edge_kind, e.language, e.from_artefact_id, e.to_artefact_id, \
                e.to_symbol_ref, e.start_line, e.end_line, e.metadata, \
                src.path AS from_path, src.symbol_fqn AS from_symbol_fqn, \
                tgt.path AS to_path, tgt.symbol_fqn AS to_symbol_fqn \
           FROM artefact_edges_current e \
           JOIN artefacts_current src ON src.repo_id = e.repo_id AND src.branch = e.branch \
                                     AND src.artefact_id = e.from_artefact_id \
      LEFT JOIN artefacts_current tgt ON tgt.repo_id = e.repo_id AND tgt.branch = e.branch \
                                     AND tgt.artefact_id = e.to_artefact_id \
          WHERE {} \
       ORDER BY src.path, COALESCE(e.start_line, 0), COALESCE(e.end_line, 0), \
                e.edge_kind, COALESCE(tgt.path, ''), e.edge_id",
        clauses.join(" AND ")
    )
}

pub(super) fn build_current_dependency_batch_sql(
    repo_id: &str,
    branch: &str,
    artefact_ids: &[String],
    direction: DepsDirection,
    filter: DepsFilterInput,
    project_path: Option<&str>,
) -> String {
    let mut clauses = vec![
        format!("e.repo_id = '{}'", esc_pg(repo_id)),
        format!("e.branch = '{}'", esc_pg(branch)),
    ];

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
           FROM artefact_edges_current e \
           JOIN artefacts_current src ON src.repo_id = e.repo_id AND src.branch = e.branch \
                                     AND src.artefact_id = e.from_artefact_id \
      LEFT JOIN artefacts_current tgt ON tgt.repo_id = e.repo_id AND tgt.branch = e.branch \
                                     AND tgt.artefact_id = e.to_artefact_id \
          WHERE {} \
       ORDER BY owner_artefact_id, src.path, COALESCE(e.start_line, 0), \
                COALESCE(e.end_line, 0), e.edge_kind, COALESCE(tgt.path, ''), e.edge_id",
        clauses.join(" AND ")
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
