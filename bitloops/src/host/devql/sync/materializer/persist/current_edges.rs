use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result};
use rusqlite::Connection;

use super::reconcile::{
    apply_current_edge_replacements_tx, canonical_local_symbol_fqn_path, recompute_current_edge_id,
};
use super::{CurrentEdgeRecord, CurrentEdgeReplacement, SUPPORTED_LOCAL_RESOLUTION_LANGUAGES};

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

pub(super) fn reconcile_current_local_edges_for_paths_with_connection(
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
    let tx = connection
        .unchecked_transaction()
        .context("starting touched current edge reconciliation temp table transaction")?;
    tx.execute_batch(
        "DROP TABLE IF EXISTS temp_touched_current_edge_paths;
         CREATE TEMP TABLE temp_touched_current_edge_paths (
             path TEXT PRIMARY KEY
         ) WITHOUT ROWID;",
    )
    .context("resetting touched current edge reconciliation temp table")?;

    {
        let mut stmt = tx
            .prepare(
                "INSERT OR IGNORE INTO temp_touched_current_edge_paths (path)
                 VALUES (?1)",
            )
            .context("preparing touched current edge reconciliation temp table insert")?;
        for path in touched_paths {
            stmt.execute([path.as_str()])
                .context("inserting touched current edge reconciliation temp path")?;
        }
    }

    tx.commit()
        .context("committing touched current edge reconciliation temp table transaction")?;

    Ok(())
}

pub(crate) fn load_current_edges_for_local_reconciliation_with_connection(
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
