#![allow(dead_code)]

mod reconcile;
#[cfg(test)]
mod tests;

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result};
use rusqlite::{Connection, Transaction, params, params_from_iter};

use self::reconcile::{
    apply_current_edge_replacements_tx, canonical_local_symbol_fqn_path, recompute_current_edge_id,
};
use super::super::content_cache::CachedExtraction;
use super::super::types::DesiredFileState;
use super::derive::prepare_materialization_rows;
use super::sql::{
    ArtefactInsertSqlInput, bool_sql, delete_artefacts_sql, delete_current_file_state_sql,
    delete_edges_sql, insert_artefact_sql, insert_edge_sql, upsert_current_file_state_sql,
};
use super::types::PreparedMaterialisationRows;

const SUPPORTED_LOCAL_RESOLUTION_LANGUAGES: &[&str] = &[
    "rust",
    "typescript",
    "javascript",
    "python",
    "go",
    "java",
    "csharp",
];

#[derive(Debug, Clone, PartialEq, Eq)]
struct CurrentEdgeRecord {
    edge_id: String,
    path: String,
    content_id: String,
    from_symbol_id: String,
    from_artefact_id: String,
    to_symbol_id: Option<String>,
    to_artefact_id: Option<String>,
    to_symbol_ref: Option<String>,
    edge_kind: String,
    language: String,
    start_line: Option<i32>,
    end_line: Option<i32>,
    metadata_json: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CurrentEdgeReplacement {
    old_edge_id: String,
    new_edges: Vec<CurrentEdgeRecord>,
}

pub(crate) async fn materialize_path(
    cfg: &crate::host::devql::DevqlConfig,
    relational: &crate::host::devql::RelationalStorage,
    desired: &DesiredFileState,
    extraction: &CachedExtraction,
    parser_version: &str,
    extractor_version: &str,
) -> Result<()> {
    let mut prepared =
        prepare_materialization_rows(cfg, desired, extraction, parser_version, extractor_version)?;
    resolve_prepared_local_edges(cfg, relational, desired, &mut prepared).await?;

    let now_sql = crate::host::devql::sql_now(relational);
    let mut statements = vec![
        delete_edges_sql(&cfg.repo.repo_id, &desired.path),
        delete_artefacts_sql(&cfg.repo.repo_id, &desired.path),
    ];
    statements.extend(prepared.materialized_artefacts.iter().map(|artefact| {
        insert_artefact_sql(
            relational,
            &ArtefactInsertSqlInput {
                repo_id: &cfg.repo.repo_id,
                path: &desired.path,
                content_id: &desired.effective_content_id,
                language: &extraction.language,
                extraction_fingerprint: &desired.extraction_fingerprint,
                artefact,
                now_sql,
            },
        )
    }));
    statements.extend(prepared.materialized_edges.iter().map(|edge| {
        insert_edge_sql(
            relational,
            &cfg.repo.repo_id,
            &desired.path,
            &desired.effective_content_id,
            edge,
            now_sql,
        )
    }));
    statements.push(upsert_current_file_state_sql(
        &cfg.repo.repo_id,
        desired,
        parser_version,
        extractor_version,
        now_sql,
    ));

    relational.exec_batch_transactional(&statements).await?;
    reconcile_current_local_edges_for_paths(
        relational,
        &cfg.repo.repo_id,
        std::slice::from_ref(&desired.path),
    )
    .await?;
    Ok(())
}

pub(crate) async fn resolve_prepared_local_edges(
    cfg: &crate::host::devql::DevqlConfig,
    relational: &crate::host::devql::RelationalStorage,
    desired: &DesiredFileState,
    prepared: &mut PreparedMaterialisationRows,
) -> Result<()> {
    let source_facts = source_facts_from_materialized_rows(desired.path.as_str(), prepared);
    let current_targets = load_current_targets_for_resolution(
        relational,
        &cfg.repo.repo_id,
        &desired.path,
        &desired.language,
    )
    .await?;
    apply_local_edge_resolutions(cfg, desired, prepared, &source_facts, &current_targets);
    Ok(())
}

pub(crate) fn resolve_prepared_local_edges_with_connection(
    connection: &Connection,
    cfg: &crate::host::devql::DevqlConfig,
    desired: &DesiredFileState,
    prepared: &mut PreparedMaterialisationRows,
) -> Result<()> {
    let source_facts = source_facts_from_materialized_rows(desired.path.as_str(), prepared);
    let current_targets = load_current_targets_for_resolution_with_connection(
        connection,
        &cfg.repo.repo_id,
        &desired.path,
        &desired.language,
    )?;
    apply_local_edge_resolutions(cfg, desired, prepared, &source_facts, &current_targets);
    Ok(())
}

fn source_facts_from_materialized_rows(
    source_path: &str,
    prepared: &PreparedMaterialisationRows,
) -> crate::host::language_adapter::LocalSourceFacts {
    let import_refs = prepared
        .materialized_edges
        .iter()
        .filter(|edge| edge.edge_kind == "imports")
        .filter_map(|edge| edge.to_symbol_ref.clone())
        .collect::<Vec<_>>();
    let package_refs = prepared
        .materialized_artefacts
        .iter()
        .filter(|artefact| {
            artefact.symbol_fqn.starts_with(&format!("{source_path}::"))
                && artefact.language_kind == "package_declaration"
        })
        .filter_map(|artefact| {
            artefact
                .symbol_fqn
                .split_once("::")
                .map(|(_, package)| package.to_string())
        })
        .collect::<Vec<_>>();
    let namespace_refs = prepared
        .materialized_artefacts
        .iter()
        .filter(|artefact| {
            artefact
                .symbol_fqn
                .starts_with(&format!("{source_path}::ns::"))
                && matches!(
                    artefact.language_kind.as_str(),
                    "namespace_declaration" | "file_scoped_namespace_declaration"
                )
        })
        .filter_map(|artefact| {
            artefact
                .symbol_fqn
                .split_once("::ns::")
                .map(|(_, namespace)| namespace.to_string())
        })
        .collect::<Vec<_>>();

    crate::host::language_adapter::LocalSourceFacts {
        import_refs,
        package_refs,
        namespace_refs,
    }
}

fn in_flight_local_targets(
    prepared: &PreparedMaterialisationRows,
) -> Vec<crate::host::language_adapter::LocalTargetInfo> {
    prepared
        .materialized_artefacts
        .iter()
        .map(|artefact| crate::host::language_adapter::LocalTargetInfo {
            symbol_fqn: artefact.symbol_fqn.clone(),
            symbol_id: artefact.symbol_id.clone(),
            artefact_id: artefact.artefact_id.clone(),
            language_kind: artefact.language_kind.clone(),
        })
        .collect()
}

fn apply_local_edge_resolutions(
    cfg: &crate::host::devql::DevqlConfig,
    desired: &DesiredFileState,
    prepared: &mut PreparedMaterialisationRows,
    source_facts: &crate::host::language_adapter::LocalSourceFacts,
    current_targets: &[crate::host::language_adapter::LocalTargetInfo],
) {
    let mut targets = in_flight_local_targets(prepared);
    targets.extend_from_slice(current_targets);

    for edge in &mut prepared.materialized_edges {
        if edge.to_symbol_id.is_some() {
            continue;
        }
        let Some(symbol_ref) = edge.to_symbol_ref.as_deref() else {
            continue;
        };
        let Some(resolved) = crate::host::language_adapter::resolve_local_symbol_ref(
            &edge.language,
            desired.path.as_str(),
            &edge.edge_kind,
            symbol_ref,
            source_facts,
            &targets,
        ) else {
            continue;
        };

        edge.edge_kind = resolved.edge_kind;
        edge.to_symbol_id = Some(resolved.symbol_id);
        edge.to_artefact_id = Some(resolved.artefact_id);
        edge.to_symbol_ref = Some(resolved.symbol_fqn);
        edge.edge_id = crate::host::devql::deterministic_uuid(&format!(
            "{}|{}|{}|{}|{}|{}|{}|{}|{}",
            cfg.repo.repo_id,
            desired.path,
            edge.from_symbol_id,
            edge.edge_kind,
            edge.to_symbol_id.clone().unwrap_or_default(),
            edge.to_symbol_ref.clone().unwrap_or_default(),
            edge.start_line.unwrap_or(-1),
            edge.end_line.unwrap_or(-1),
            edge.metadata,
        ));
    }
}

fn compatible_resolution_languages(language: &str) -> Vec<&'static str> {
    match language.trim().to_ascii_lowercase().as_str() {
        "typescript" | "javascript" => vec!["typescript", "javascript"],
        "rust" => vec!["rust"],
        "python" => vec!["python"],
        "go" => vec!["go"],
        "java" => vec!["java"],
        "csharp" | "c#" => vec!["csharp"],
        _ => vec![],
    }
}

async fn load_current_targets_for_resolution(
    relational: &crate::host::devql::RelationalStorage,
    repo_id: &str,
    current_path: &str,
    language: &str,
) -> Result<Vec<crate::host::language_adapter::LocalTargetInfo>> {
    let compatible_languages = compatible_resolution_languages(language);
    if compatible_languages.is_empty() {
        return Ok(Vec::new());
    }
    let in_list = compatible_languages
        .iter()
        .map(|language| format!("'{}'", crate::host::devql::esc_pg(language)))
        .collect::<Vec<_>>()
        .join(", ");
    let sql = format!(
        "SELECT symbol_fqn, symbol_id, artefact_id, language_kind \
         FROM artefacts_current \
         WHERE repo_id = '{}' AND path != '{}' AND language IN ({in_list})",
        crate::host::devql::esc_pg(repo_id),
        crate::host::devql::esc_pg(current_path),
    );
    let rows = relational.query_rows(&sql).await?;
    Ok(rows
        .into_iter()
        .filter_map(|row| {
            let obj = row.as_object()?;
            Some(crate::host::language_adapter::LocalTargetInfo {
                symbol_fqn: obj.get("symbol_fqn")?.as_str()?.to_string(),
                symbol_id: obj.get("symbol_id")?.as_str()?.to_string(),
                artefact_id: obj.get("artefact_id")?.as_str()?.to_string(),
                language_kind: obj.get("language_kind")?.as_str()?.to_string(),
            })
        })
        .collect())
}

fn load_current_targets_for_resolution_with_connection(
    connection: &Connection,
    repo_id: &str,
    current_path: &str,
    language: &str,
) -> Result<Vec<crate::host::language_adapter::LocalTargetInfo>> {
    let compatible_languages = compatible_resolution_languages(language);
    if compatible_languages.is_empty() {
        return Ok(Vec::new());
    }
    let in_list = compatible_languages
        .iter()
        .map(|language| format!("'{}'", crate::host::devql::esc_pg(language)))
        .collect::<Vec<_>>()
        .join(", ");
    let sql = format!(
        "SELECT symbol_fqn, symbol_id, artefact_id, language_kind \
         FROM artefacts_current \
         WHERE repo_id = '{}' AND path != '{}' AND language IN ({in_list})",
        crate::host::devql::esc_pg(repo_id),
        crate::host::devql::esc_pg(current_path),
    );
    let mut stmt = connection
        .prepare(&sql)
        .context("preparing current local target lookup query")?;
    let rows = stmt
        .query_map([], |row| {
            Ok(crate::host::language_adapter::LocalTargetInfo {
                symbol_fqn: row.get::<_, String>(0)?,
                symbol_id: row.get::<_, String>(1)?,
                artefact_id: row.get::<_, String>(2)?,
                language_kind: row.get::<_, String>(3)?,
            })
        })
        .context("querying current local target lookup rows")?
        .collect::<Result<Vec<_>, _>>()
        .context("collecting current local target lookup rows")?;
    Ok(rows)
}

pub(crate) async fn reconcile_current_local_edges_for_paths(
    relational: &crate::host::devql::RelationalStorage,
    repo_id: &str,
    touched_paths: &[String],
) -> Result<usize> {
    let sqlite_path = relational.sqlite_path().to_path_buf();
    let repo_id = repo_id.to_string();
    let touched_paths = touched_paths.to_vec();

    tokio::task::spawn_blocking(move || -> Result<usize> {
        let mut connection = open_current_state_reconciliation_connection(&sqlite_path)?;
        reconcile_current_local_edges_for_paths_with_connection(
            &mut connection,
            &repo_id,
            &touched_paths,
        )
    })
    .await
    .context("joining current local edge reconciliation task")?
}

pub(crate) fn reconcile_current_local_edges_for_paths_with_connection(
    connection: &mut Connection,
    repo_id: &str,
    touched_paths: &[String],
) -> Result<usize> {
    let touched_paths = touched_paths.iter().cloned().collect::<HashSet<_>>();
    let current_targets =
        load_all_current_targets_for_local_resolution_with_connection(connection, repo_id)?;
    let target_by_symbol_fqn = current_targets
        .iter()
        .cloned()
        .map(|target| (target.symbol_fqn.clone(), target))
        .collect::<HashMap<_, _>>();
    let source_facts_by_path = load_current_source_facts_with_connection(connection, repo_id)?;
    let current_edges = load_current_edges_for_local_reconciliation_with_connection(
        connection,
        repo_id,
        &touched_paths,
    )?;
    let replacements = build_current_edge_replacements_for_local_resolution(
        repo_id,
        &touched_paths,
        &source_facts_by_path,
        &current_targets,
        &target_by_symbol_fqn,
        &current_edges,
    );

    if replacements.is_empty() {
        return Ok(0);
    }

    let tx = connection
        .transaction()
        .context("starting current local edge reconciliation transaction")?;
    let affected_rows = apply_current_edge_replacements_tx(&tx, repo_id, &replacements)?;
    tx.commit()
        .context("committing current local edge reconciliation transaction")?;
    Ok(affected_rows)
}

fn open_current_state_reconciliation_connection(path: &Path) -> Result<Connection> {
    let connection = Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE)
        .with_context(|| format!("opening SQLite database at {}", path.display()))?;
    connection
        .busy_timeout(Duration::from_secs(30))
        .context("setting SQLite busy timeout for current edge reconciliation")?;
    Ok(connection)
}

fn is_supported_local_resolution_language(language: &str) -> bool {
    let normalized = language.trim().to_ascii_lowercase();
    SUPPORTED_LOCAL_RESOLUTION_LANGUAGES
        .iter()
        .any(|supported| *supported == normalized)
}

fn load_all_current_targets_for_local_resolution_with_connection(
    connection: &Connection,
    repo_id: &str,
) -> Result<Vec<crate::host::language_adapter::LocalTargetInfo>> {
    let mut stmt = connection
        .prepare(
            "SELECT symbol_fqn, symbol_id, artefact_id, language_kind, language \
             FROM artefacts_current \
             WHERE repo_id = ?1",
        )
        .context("preparing full current local target lookup query")?;
    let rows = stmt
        .query_map([repo_id], |row| {
            Ok((
                crate::host::language_adapter::LocalTargetInfo {
                    symbol_fqn: row.get::<_, String>(0)?,
                    symbol_id: row.get::<_, String>(1)?,
                    artefact_id: row.get::<_, String>(2)?,
                    language_kind: row.get::<_, String>(3)?,
                },
                row.get::<_, String>(4)?,
            ))
        })
        .context("querying full current local target lookup rows")?
        .collect::<Result<Vec<_>, _>>()
        .context("collecting full current local target lookup rows")?;

    Ok(rows
        .into_iter()
        .filter_map(|(target, language)| {
            is_supported_local_resolution_language(&language).then_some(target)
        })
        .collect())
}

fn load_current_source_facts_with_connection(
    connection: &Connection,
    repo_id: &str,
) -> Result<HashMap<String, crate::host::language_adapter::LocalSourceFacts>> {
    let mut facts_by_path =
        HashMap::<String, crate::host::language_adapter::LocalSourceFacts>::new();

    {
        let mut stmt = connection
            .prepare(
                "SELECT path, to_symbol_ref \
                 FROM artefact_edges_current \
                 WHERE repo_id = ?1 AND edge_kind = 'imports' AND to_symbol_ref IS NOT NULL",
            )
            .context("preparing current import refs lookup query")?;
        let import_rows = stmt
            .query_map([repo_id], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .context("querying current import refs rows")?
            .collect::<Result<Vec<_>, _>>()
            .context("collecting current import refs rows")?;
        for (path, symbol_ref) in import_rows {
            facts_by_path
                .entry(path)
                .or_default()
                .import_refs
                .push(symbol_ref);
        }
    }

    {
        let mut stmt = connection
            .prepare(
                "SELECT symbol_fqn \
                 FROM artefacts_current \
                 WHERE repo_id = ?1 AND language_kind = 'package_declaration'",
            )
            .context("preparing current package refs lookup query")?;
        let package_rows = stmt
            .query_map([repo_id], |row| row.get::<_, String>(0))
            .context("querying current package refs rows")?
            .collect::<Result<Vec<_>, _>>()
            .context("collecting current package refs rows")?;
        for symbol_fqn in package_rows {
            let Some((path, package_ref)) = symbol_fqn.split_once("::") else {
                continue;
            };
            facts_by_path
                .entry(path.to_string())
                .or_default()
                .package_refs
                .push(package_ref.to_string());
        }
    }

    {
        let mut stmt = connection
            .prepare(
                "SELECT symbol_fqn \
                 FROM artefacts_current \
                 WHERE repo_id = ?1 AND language_kind IN ('namespace_declaration', 'file_scoped_namespace_declaration')",
            )
            .context("preparing current namespace refs lookup query")?;
        let namespace_rows = stmt
            .query_map([repo_id], |row| row.get::<_, String>(0))
            .context("querying current namespace refs rows")?
            .collect::<Result<Vec<_>, _>>()
            .context("collecting current namespace refs rows")?;
        for symbol_fqn in namespace_rows {
            let Some((path, namespace_ref)) = symbol_fqn.split_once("::ns::") else {
                continue;
            };
            facts_by_path
                .entry(path.to_string())
                .or_default()
                .namespace_refs
                .push(namespace_ref.to_string());
        }
    }

    Ok(facts_by_path)
}

fn map_current_edge_record(row: &rusqlite::Row<'_>) -> rusqlite::Result<CurrentEdgeRecord> {
    Ok(CurrentEdgeRecord {
        edge_id: row.get::<_, String>(0)?,
        path: row.get::<_, String>(1)?,
        content_id: row.get::<_, String>(2)?,
        from_symbol_id: row.get::<_, String>(3)?,
        from_artefact_id: row.get::<_, String>(4)?,
        to_symbol_id: row.get::<_, Option<String>>(5)?,
        to_artefact_id: row.get::<_, Option<String>>(6)?,
        to_symbol_ref: row.get::<_, Option<String>>(7)?,
        edge_kind: row.get::<_, String>(8)?,
        language: row.get::<_, String>(9)?,
        start_line: row.get::<_, Option<i32>>(10)?,
        end_line: row.get::<_, Option<i32>>(11)?,
        metadata_json: row.get::<_, String>(12)?,
    })
}

fn refresh_touched_current_edge_paths_temp_table(
    connection: &Connection,
    touched_paths: &HashSet<String>,
) -> Result<()> {
    connection
        .execute_batch(
            "DROP TABLE IF EXISTS temp_touched_current_edge_paths;
             CREATE TEMP TABLE temp_touched_current_edge_paths (
                 path TEXT PRIMARY KEY
             ) WITHOUT ROWID;",
        )
        .context("resetting touched current edge reconciliation temp table")?;

    let mut stmt = connection
        .prepare(
            "INSERT OR IGNORE INTO temp_touched_current_edge_paths (path)
             VALUES (?1)",
        )
        .context("preparing touched current edge reconciliation temp table insert")?;
    for path in touched_paths {
        stmt.execute([path.as_str()])
            .context("inserting touched current edge reconciliation temp path")?;
    }

    Ok(())
}

fn load_current_edges_for_local_reconciliation_with_connection(
    connection: &Connection,
    repo_id: &str,
    touched_paths: &HashSet<String>,
) -> Result<Vec<CurrentEdgeRecord>> {
    let mut rows = {
        let mut stmt = connection
            .prepare(
                "SELECT edge_id, path, content_id, from_symbol_id, from_artefact_id, to_symbol_id, to_artefact_id, to_symbol_ref, edge_kind, language, start_line, end_line, metadata \
                 FROM artefact_edges_current \
                 WHERE repo_id = ?1 AND to_symbol_ref IS NOT NULL AND to_symbol_id IS NULL",
            )
            .context("preparing unresolved current edge reconciliation query")?;
        stmt.query_map([repo_id], map_current_edge_record)
            .context("querying unresolved current edge reconciliation rows")?
            .collect::<Result<Vec<_>, _>>()
            .context("collecting unresolved current edge reconciliation rows")?
    };

    if !touched_paths.is_empty() {
        refresh_touched_current_edge_paths_temp_table(connection, touched_paths)?;

        let mut stmt = connection
            .prepare(
                "SELECT e.edge_id, e.path, e.content_id, e.from_symbol_id, e.from_artefact_id, e.to_symbol_id, e.to_artefact_id, e.to_symbol_ref, e.edge_kind, e.language, e.start_line, e.end_line, e.metadata \
                 FROM artefact_edges_current e \
                 INNER JOIN temp_touched_current_edge_paths touched \
                    ON (
                        CASE
                            WHEN instr(e.to_symbol_ref, '::') > 0
                                THEN substr(e.to_symbol_ref, 1, instr(e.to_symbol_ref, '::') - 1)
                            ELSE e.to_symbol_ref
                        END
                    ) = touched.path \
                 WHERE e.repo_id = ?1 AND e.to_symbol_ref IS NOT NULL AND e.to_symbol_id IS NOT NULL",
            )
            .context("preparing touched current edge reconciliation query")?;
        let resolved_rows = stmt
            .query_map([repo_id], map_current_edge_record)
            .context("querying touched current edge reconciliation rows")?
            .collect::<Result<Vec<_>, _>>()
            .context("collecting touched current edge reconciliation rows")?;
        rows.extend(resolved_rows);
    }

    Ok(rows
        .into_iter()
        .filter(|edge| is_supported_local_resolution_language(&edge.language))
        .collect())
}

fn build_current_edge_replacements_for_local_resolution(
    repo_id: &str,
    touched_paths: &HashSet<String>,
    source_facts_by_path: &HashMap<String, crate::host::language_adapter::LocalSourceFacts>,
    current_targets: &[crate::host::language_adapter::LocalTargetInfo],
    target_by_symbol_fqn: &HashMap<String, crate::host::language_adapter::LocalTargetInfo>,
    current_edges: &[CurrentEdgeRecord],
) -> Vec<CurrentEdgeReplacement> {
    let mut replacements = Vec::new();

    for edge in current_edges {
        if edge.to_symbol_id.is_none() {
            let source_facts = source_facts_by_path
                .get(&edge.path)
                .cloned()
                .unwrap_or_default();
            let expanded_edges = expand_current_edge_for_local_resolution(
                repo_id,
                edge,
                &source_facts,
                current_targets,
            );
            if expanded_edges.len() != 1 || expanded_edges.first() != Some(edge) {
                replacements.push(CurrentEdgeReplacement {
                    old_edge_id: edge.edge_id.clone(),
                    new_edges: expanded_edges,
                });
                continue;
            }
        }

        let mut next_edge = edge.clone();
        let mut changed = false;
        let Some(symbol_ref) = next_edge.to_symbol_ref.as_deref() else {
            continue;
        };
        let Some(target_path) = canonical_local_symbol_fqn_path(&next_edge.language, symbol_ref)
        else {
            continue;
        };
        if !touched_paths.contains(target_path) {
            continue;
        }

        match target_by_symbol_fqn.get(symbol_ref) {
            Some(target) => {
                if next_edge.to_symbol_id.as_deref() != Some(target.symbol_id.as_str())
                    || next_edge.to_artefact_id.as_deref() != Some(target.artefact_id.as_str())
                {
                    next_edge.to_symbol_id = Some(target.symbol_id.clone());
                    next_edge.to_artefact_id = Some(target.artefact_id.clone());
                    changed = true;
                }
            }
            None => {
                if next_edge.to_symbol_id.is_some() || next_edge.to_artefact_id.is_some() {
                    next_edge.to_symbol_id = None;
                    next_edge.to_artefact_id = None;
                    changed = true;
                }
            }
        }

        if !changed {
            continue;
        }

        next_edge.edge_id = recompute_current_edge_id(repo_id, &next_edge);
        if next_edge != *edge {
            replacements.push(CurrentEdgeReplacement {
                old_edge_id: edge.edge_id.clone(),
                new_edges: vec![next_edge],
            });
        }
    }

    replacements
}

fn expand_current_edge_for_local_resolution(
    repo_id: &str,
    edge: &CurrentEdgeRecord,
    source_facts: &crate::host::language_adapter::LocalSourceFacts,
    current_targets: &[crate::host::language_adapter::LocalTargetInfo],
) -> Vec<CurrentEdgeRecord> {
    let Some(symbol_ref) = edge.to_symbol_ref.as_deref() else {
        return vec![edge.clone()];
    };
    let normalized_refs = crate::host::language_adapter::normalize_local_edge_symbol_refs(
        &edge.language,
        &edge.path,
        &edge.edge_kind,
        symbol_ref,
    );
    let normalized_refs = if normalized_refs.is_empty() {
        vec![symbol_ref.to_string()]
    } else {
        normalized_refs
    };

    normalized_refs
        .into_iter()
        .map(|normalized_ref| {
            let mut next_edge = edge.clone();
            next_edge.to_symbol_id = None;
            next_edge.to_artefact_id = None;
            next_edge.to_symbol_ref = Some(normalized_ref.clone());

            if let Some(resolved) = crate::host::language_adapter::resolve_local_symbol_ref(
                &edge.language,
                &edge.path,
                &edge.edge_kind,
                &normalized_ref,
                source_facts,
                current_targets,
            ) {
                next_edge.edge_kind = resolved.edge_kind;
                next_edge.to_symbol_id = Some(resolved.symbol_id);
                next_edge.to_artefact_id = Some(resolved.artefact_id);
                next_edge.to_symbol_ref = Some(resolved.symbol_fqn);
            }

            next_edge.edge_id = recompute_current_edge_id(repo_id, &next_edge);
            next_edge
        })
        .collect()
}

pub(crate) async fn remove_path(
    cfg: &crate::host::devql::DevqlConfig,
    relational: &crate::host::devql::RelationalStorage,
    path: &str,
) -> Result<()> {
    relational
        .exec_batch_transactional(&[
            delete_edges_sql(&cfg.repo.repo_id, path),
            delete_artefacts_sql(&cfg.repo.repo_id, path),
            delete_current_file_state_sql(&cfg.repo.repo_id, path),
        ])
        .await?;
    reconcile_current_local_edges_for_paths(relational, &cfg.repo.repo_id, &[path.to_string()])
        .await?;
    Ok(())
}

pub(crate) fn persist_prepared_materialisation_tx(
    tx: &Transaction<'_>,
    repo_id: &str,
    desired: &DesiredFileState,
    extraction: &CachedExtraction,
    prepared: &PreparedMaterialisationRows,
    parser_version: &str,
    extractor_version: &str,
) -> Result<usize> {
    let mut affected_rows = 0usize;

    affected_rows += tx
        .execute(
            "DELETE FROM artefact_edges_current WHERE repo_id = ?1 AND path = ?2",
            params![repo_id, desired.path],
        )
        .context("deleting materialised edge rows for path")?;
    affected_rows += tx
        .execute(
            "DELETE FROM artefacts_current WHERE repo_id = ?1 AND path = ?2",
            params![repo_id, desired.path],
        )
        .context("deleting materialised artefact rows for path")?;

    {
        let mut stmt = tx
            .prepare(
                "INSERT INTO artefacts_current \
                 (repo_id, path, content_id, symbol_id, artefact_id, language, extraction_fingerprint, canonical_kind, language_kind, symbol_fqn, parent_symbol_id, parent_artefact_id, start_line, end_line, start_byte, end_byte, signature, modifiers, docstring, updated_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, datetime('now'))",
            )
            .context("preparing current artefact insert")?;
        for artefact in &prepared.materialized_artefacts {
            let modifiers =
                serde_json::to_string(&artefact.modifiers).unwrap_or_else(|_| "[]".to_string());
            affected_rows += stmt
                .execute(params![
                    repo_id,
                    desired.path,
                    desired.effective_content_id,
                    artefact.symbol_id,
                    artefact.artefact_id,
                    extraction.language,
                    desired.extraction_fingerprint,
                    artefact.canonical_kind,
                    artefact.language_kind,
                    artefact.symbol_fqn,
                    artefact.parent_symbol_id,
                    artefact.parent_artefact_id,
                    artefact.start_line,
                    artefact.end_line,
                    artefact.start_byte,
                    artefact.end_byte,
                    artefact.signature,
                    modifiers,
                    artefact.docstring,
                ])
                .context("inserting current artefact row")?;
        }
    }

    {
        let mut stmt = tx
            .prepare(
                "INSERT INTO artefact_edges_current \
                 (repo_id, edge_id, path, content_id, from_symbol_id, from_artefact_id, to_symbol_id, to_artefact_id, to_symbol_ref, edge_kind, language, start_line, end_line, metadata, updated_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, datetime('now'))",
            )
            .context("preparing current edge insert")?;
        for edge in &prepared.materialized_edges {
            let metadata =
                serde_json::to_string(&edge.metadata).unwrap_or_else(|_| "{}".to_string());
            affected_rows += stmt
                .execute(params![
                    repo_id,
                    edge.edge_id,
                    desired.path,
                    desired.effective_content_id,
                    edge.from_symbol_id,
                    edge.from_artefact_id,
                    edge.to_symbol_id,
                    edge.to_artefact_id,
                    edge.to_symbol_ref,
                    edge.edge_kind,
                    edge.language,
                    edge.start_line,
                    edge.end_line,
                    metadata,
                ])
                .context("inserting current edge row")?;
        }
    }

    affected_rows += tx
        .execute(
            "INSERT INTO current_file_state \
             (repo_id, path, analysis_mode, file_role, text_index_mode, language, resolved_language, dialect, primary_context_id, secondary_context_ids_json, frameworks_json, runtime_profile, classification_reason, context_fingerprint, extraction_fingerprint, head_content_id, index_content_id, worktree_content_id, effective_content_id, effective_source, parser_version, extractor_version, exists_in_head, exists_in_index, exists_in_worktree, last_synced_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25, datetime('now')) \
             ON CONFLICT (repo_id, path) DO UPDATE SET \
                 analysis_mode = excluded.analysis_mode, \
                 file_role = excluded.file_role, \
                 text_index_mode = excluded.text_index_mode, \
                 language = excluded.language, \
                 resolved_language = excluded.resolved_language, \
                 dialect = excluded.dialect, \
                 primary_context_id = excluded.primary_context_id, \
                 secondary_context_ids_json = excluded.secondary_context_ids_json, \
                 frameworks_json = excluded.frameworks_json, \
                 runtime_profile = excluded.runtime_profile, \
                 classification_reason = excluded.classification_reason, \
                 context_fingerprint = excluded.context_fingerprint, \
                 extraction_fingerprint = excluded.extraction_fingerprint, \
                 head_content_id = excluded.head_content_id, \
                 index_content_id = excluded.index_content_id, \
                 worktree_content_id = excluded.worktree_content_id, \
                 effective_content_id = excluded.effective_content_id, \
                 effective_source = excluded.effective_source, \
                 parser_version = excluded.parser_version, \
                 extractor_version = excluded.extractor_version, \
                 exists_in_head = excluded.exists_in_head, \
                 exists_in_index = excluded.exists_in_index, \
                 exists_in_worktree = excluded.exists_in_worktree, \
                 last_synced_at = excluded.last_synced_at",
            params![
                repo_id,
                desired.path,
                desired.analysis_mode.as_str(),
                desired.file_role.as_str(),
                desired.text_index_mode.as_str(),
                desired.language,
                desired.resolved_language,
                desired.dialect,
                desired.primary_context_id,
                serde_json::to_string(&desired.secondary_context_ids)
                    .unwrap_or_else(|_| "[]".to_string()),
                serde_json::to_string(&desired.frameworks)
                    .unwrap_or_else(|_| "[]".to_string()),
                desired.runtime_profile,
                desired.classification_reason,
                desired.context_fingerprint,
                desired.extraction_fingerprint,
                desired.head_content_id,
                desired.index_content_id,
                desired.worktree_content_id,
                desired.effective_content_id,
                desired.effective_source.as_str(),
                parser_version,
                extractor_version,
                bool_sql(desired.exists_in_head),
                bool_sql(desired.exists_in_index),
                bool_sql(desired.exists_in_worktree),
            ],
        )
        .context("upserting current file state row")?;

    Ok(affected_rows)
}

pub(crate) fn remove_paths_tx(
    tx: &Transaction<'_>,
    repo_id: &str,
    paths: &[String],
) -> Result<usize> {
    if paths.is_empty() {
        return Ok(0);
    }

    let mut affected_rows = 0usize;
    affected_rows += delete_paths_tx(
        tx,
        "artefact_edges_current",
        repo_id,
        paths,
        "deleting current edge rows for removed paths",
    )?;
    affected_rows += delete_paths_tx(
        tx,
        "artefacts_current",
        repo_id,
        paths,
        "deleting current artefact rows for removed paths",
    )?;
    affected_rows += delete_paths_tx(
        tx,
        "current_file_state",
        repo_id,
        paths,
        "deleting current file state rows for removed paths",
    )?;
    Ok(affected_rows)
}

fn delete_paths_tx(
    tx: &Transaction<'_>,
    table: &str,
    repo_id: &str,
    paths: &[String],
    context: &str,
) -> Result<usize> {
    let placeholders = std::iter::repeat_n("?", paths.len())
        .collect::<Vec<_>>()
        .join(", ");
    let sql = format!("DELETE FROM {table} WHERE repo_id = ? AND path IN ({placeholders})");
    let params = std::iter::once(repo_id.to_string()).chain(paths.iter().cloned());
    tx.execute(&sql, params_from_iter(params))
        .with_context(|| context.to_string())
}
