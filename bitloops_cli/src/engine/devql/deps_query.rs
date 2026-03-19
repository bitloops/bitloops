fn resolve_commit_selector(cfg: &DevqlConfig, parsed: &ParsedDevqlQuery) -> Result<Option<String>> {
    let Some(as_of) = parsed.as_of.as_ref() else {
        return Ok(None);
    };

    match as_of {
        AsOfSelector::Commit(commit) => Ok(Some(commit.clone())),
        AsOfSelector::Ref(reference) => {
            let commit = run_git(&cfg.repo_root, &["rev-parse", reference])
                .with_context(|| format!("resolving git ref `{reference}`"))?;
            Ok(Some(commit.trim().to_string()))
        }
        AsOfSelector::SaveCurrent | AsOfSelector::SaveRevision(_) => Ok(None),
    }
}

fn build_relational_deps_query(
    cfg: &DevqlConfig,
    parsed: &ParsedDevqlQuery,
    repo_id: &str,
    dialect: RelationalDialect,
) -> Result<String> {
    validate_deps_filter(&parsed.deps)?;
    let use_historical_tables = matches!(
        parsed.as_of,
        Some(AsOfSelector::Commit(_)) | Some(AsOfSelector::Ref(_))
    );
    let historical_commit_selector = if use_historical_tables {
        resolve_commit_selector(cfg, parsed)?
    } else {
        None
    };
    let artefacts_table = if use_historical_tables {
        "artefacts"
    } else {
        "artefacts_current"
    };
    let edges_table = if use_historical_tables {
        "artefact_edges"
    } else {
        "artefact_edges_current"
    };

    let mut source_filters = vec![format!("a.repo_id = '{}'", esc_pg(repo_id))];
    if let Some(AsOfSelector::SaveRevision(revision_id)) = parsed.as_of.as_ref() {
        source_filters.push("a.revision_kind = 'temporary'".to_string());
        source_filters.push(format!("a.revision_id = '{}'", esc_pg(revision_id)));
    }
    if let Some(kind) = parsed.artefacts.kind.as_deref() {
        source_filters.push(format!("a.canonical_kind = '{}'", esc_pg(kind)));
    }
    if let Some(symbol_fqn) = parsed.artefacts.symbol_fqn.as_deref() {
        source_filters.push(format!("a.symbol_fqn = '{}'", esc_pg(symbol_fqn)));
    }
    if let Some((start, end)) = parsed.artefacts.lines {
        source_filters.push(format!("a.start_line <= {end} AND a.end_line >= {start}"));
    }
    if let Some(path) = parsed.file.as_deref() {
        let path_candidates = build_path_candidates(path);
        if let Some(commit_sha) = historical_commit_selector.as_deref() {
            let git_blob = path_candidates.iter().find_map(|candidate| {
                git_blob_sha_at_commit(&cfg.repo_root, commit_sha, candidate)
            });
            if let Some(blob_sha) = git_blob {
                source_filters.push(format!("a.blob_sha = '{}'", esc_pg(&blob_sha)));
            } else {
                source_filters.push(format!(
                    "a.blob_sha = (SELECT blob_sha FROM file_state WHERE repo_id = '{}' AND commit_sha = '{}' AND ({}) LIMIT 1)",
                    esc_pg(repo_id),
                    esc_pg(commit_sha),
                    sql_path_candidates_clause("path", &path_candidates),
                ));
            }
        } else {
            source_filters.push(format!(
                "({})",
                sql_path_candidates_clause("a.path", &path_candidates)
            ));
        }
    }
    if let Some(glob) = parsed.files_path.as_deref() {
        let like = glob_to_sql_like(glob);
        source_filters.push(format!("a.path LIKE '{}'", esc_pg(&like)));
    }
    if parsed.file.is_none() && let Some(commit_sha) = historical_commit_selector.as_deref() {
        source_filters.push(format!(
            "a.blob_sha IN (SELECT blob_sha FROM file_state WHERE repo_id = '{}' AND commit_sha = '{}')",
            esc_pg(repo_id),
            esc_pg(commit_sha),
        ));
    }

    let mut edge_filters = vec![format!("e.repo_id = '{}'", esc_pg(repo_id))];
    if let Some(AsOfSelector::SaveRevision(revision_id)) = parsed.as_of.as_ref() {
        edge_filters.push("e.revision_kind = 'temporary'".to_string());
        edge_filters.push(format!("e.revision_id = '{}'", esc_pg(revision_id)));
    }
    if let Some(kind) = parsed.deps.kind {
        edge_filters.push(format!("e.edge_kind = '{}'", esc_pg(kind.as_str())));
    }
    if !parsed.deps.include_unresolved {
        edge_filters.push("e.to_artefact_id IS NOT NULL".to_string());
    }

    let order_clause = match dialect {
        RelationalDialect::Postgres => {
            "e.edge_kind, e.from_artefact_id, e.to_artefact_id NULLS LAST, e.to_symbol_ref NULLS LAST".to_string()
        }
        RelationalDialect::Sqlite => {
            "e.edge_kind, e.from_artefact_id, \
CASE WHEN e.to_artefact_id IS NULL THEN 1 ELSE 0 END, e.to_artefact_id, \
CASE WHEN e.to_symbol_ref IS NULL THEN 1 ELSE 0 END, e.to_symbol_ref"
                .to_string()
        }
    };

    let sql = if parsed.deps.direction == DepsDirection::In {
        format!(
            "SELECT e.edge_id, e.edge_kind, e.language, e.from_artefact_id, e.to_artefact_id, e.to_symbol_ref, e.start_line, e.end_line, e.metadata, \
af.path AS from_path, af.symbol_fqn AS from_symbol_fqn, at.path AS to_path, at.symbol_fqn AS to_symbol_fqn \
FROM {} e \
JOIN {} at ON at.artefact_id = e.to_artefact_id \
JOIN {} af ON af.artefact_id = e.from_artefact_id \
WHERE {} AND {} \
ORDER BY {} \
LIMIT {}",
            edges_table,
            artefacts_table,
            artefacts_table,
            edge_filters.join(" AND "),
            source_filters
                .iter()
                .map(|f| f.replace("a.", "at."))
                .collect::<Vec<_>>()
                .join(" AND "),
            order_clause,
            parsed.limit.max(1)
        )
    } else if parsed.deps.direction == DepsDirection::Both {
        format!(
            "WITH out_edges AS ( \
SELECT e.edge_id, e.edge_kind, e.language, e.from_artefact_id, e.to_artefact_id, e.to_symbol_ref, e.start_line, e.end_line, e.metadata \
FROM {} e JOIN {} a ON a.artefact_id = e.from_artefact_id \
WHERE {} AND {} \
), in_edges AS ( \
SELECT e.edge_id, e.edge_kind, e.language, e.from_artefact_id, e.to_artefact_id, e.to_symbol_ref, e.start_line, e.end_line, e.metadata \
FROM {} e JOIN {} a ON a.artefact_id = e.to_artefact_id \
WHERE {} AND {} \
) \
SELECT DISTINCT e.edge_id, e.edge_kind, e.language, e.from_artefact_id, e.to_artefact_id, e.to_symbol_ref, e.start_line, e.end_line, e.metadata, \
af.path AS from_path, af.symbol_fqn AS from_symbol_fqn, at.path AS to_path, at.symbol_fqn AS to_symbol_fqn \
FROM (SELECT * FROM out_edges UNION ALL SELECT * FROM in_edges) e \
JOIN {} af ON af.artefact_id = e.from_artefact_id \
LEFT JOIN {} at ON at.artefact_id = e.to_artefact_id \
ORDER BY {} \
LIMIT {}",
            edges_table,
            artefacts_table,
            edge_filters.join(" AND "),
            source_filters.join(" AND "),
            edges_table,
            artefacts_table,
            edge_filters.join(" AND "),
            source_filters.join(" AND "),
            artefacts_table,
            artefacts_table,
            order_clause,
            parsed.limit.max(1)
        )
    } else {
        format!(
            "SELECT e.edge_id, e.edge_kind, e.language, e.from_artefact_id, e.to_artefact_id, e.to_symbol_ref, e.start_line, e.end_line, e.metadata, \
af.path AS from_path, af.symbol_fqn AS from_symbol_fqn, at.path AS to_path, at.symbol_fqn AS to_symbol_fqn \
FROM {} e \
JOIN {} af ON af.artefact_id = e.from_artefact_id \
LEFT JOIN {} at ON at.artefact_id = e.to_artefact_id \
WHERE {} AND {} \
ORDER BY {} \
LIMIT {}",
            edges_table,
            artefacts_table,
            artefacts_table,
            edge_filters.join(" AND "),
            source_filters
                .iter()
                .map(|f| f.replace("a.", "af."))
                .collect::<Vec<_>>()
                .join(" AND "),
            order_clause,
            parsed.limit.max(1)
        )
    };

    Ok(sql)
}

#[cfg(test)]
fn build_postgres_deps_query(
    cfg: &DevqlConfig,
    parsed: &ParsedDevqlQuery,
    repo_id: &str,
) -> Result<String> {
    build_relational_deps_query(cfg, parsed, repo_id, RelationalDialect::Postgres)
}
