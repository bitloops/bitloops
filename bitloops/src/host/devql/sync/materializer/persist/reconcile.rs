use anyhow::{Context, Result};
use rusqlite::{Transaction, params};

use super::{CurrentEdgeRecord, CurrentEdgeReplacement};

pub(super) fn canonical_local_symbol_fqn_path<'a>(
    language: &str,
    symbol_ref: &'a str,
) -> Option<&'a str> {
    let path = symbol_ref
        .split_once("::")
        .map(|(path, _)| path)
        .unwrap_or(symbol_ref);
    let is_canonical = match language.trim().to_ascii_lowercase().as_str() {
        "rust" => path.ends_with(".rs"),
        "typescript" | "javascript" => {
            path.ends_with(".ts")
                || path.ends_with(".tsx")
                || path.ends_with(".js")
                || path.ends_with(".jsx")
        }
        "python" => path.ends_with(".py"),
        "go" => path.ends_with(".go"),
        "java" => path.ends_with(".java"),
        "csharp" | "c#" => path.ends_with(".cs"),
        _ => false,
    };
    is_canonical.then_some(path)
}

pub(super) fn recompute_current_edge_id(repo_id: &str, edge: &CurrentEdgeRecord) -> String {
    crate::host::devql::deterministic_uuid(&format!(
        "{}|{}|{}|{}|{}|{}|{}|{}|{}",
        repo_id,
        edge.path,
        edge.from_symbol_id,
        edge.edge_kind,
        edge.to_symbol_id.clone().unwrap_or_default(),
        edge.to_symbol_ref.clone().unwrap_or_default(),
        edge.start_line.unwrap_or(-1),
        edge.end_line.unwrap_or(-1),
        edge.metadata_json,
    ))
}

pub(super) fn apply_current_edge_replacements_tx(
    tx: &Transaction<'_>,
    repo_id: &str,
    replacements: &[CurrentEdgeReplacement],
) -> Result<usize> {
    let mut affected_rows = 0usize;
    let mut delete_stmt = tx
        .prepare("DELETE FROM artefact_edges_current WHERE repo_id = ?1 AND edge_id = ?2")
        .context("preparing current edge reconciliation delete")?;
    let mut insert_stmt = tx
        .prepare(
            "INSERT OR REPLACE INTO artefact_edges_current \
             (repo_id, edge_id, path, content_id, from_symbol_id, from_artefact_id, to_symbol_id, to_artefact_id, to_symbol_ref, edge_kind, language, start_line, end_line, metadata, updated_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, datetime('now'))",
        )
        .context("preparing current edge reconciliation insert")?;

    for replacement in replacements {
        affected_rows += delete_stmt
            .execute(params![repo_id, replacement.old_edge_id])
            .context("deleting superseded current edge row")?;
        for new_edge in &replacement.new_edges {
            affected_rows += insert_stmt
                .execute(params![
                    repo_id,
                    new_edge.edge_id,
                    new_edge.path,
                    new_edge.content_id,
                    new_edge.from_symbol_id,
                    new_edge.from_artefact_id,
                    new_edge.to_symbol_id,
                    new_edge.to_artefact_id,
                    new_edge.to_symbol_ref,
                    new_edge.edge_kind,
                    new_edge.language,
                    new_edge.start_line,
                    new_edge.end_line,
                    new_edge.metadata_json,
                ])
                .context("inserting reconciled current edge row")?;
        }
    }

    Ok(affected_rows)
}
