use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result, anyhow};
use serde_json::Value;
use uuid::Uuid;

use super::progress::{SyncObserver, SyncProgressPhase, SyncProgressUpdate};
use super::summary::{SyncSummary, SyncValidationFileDrift, SyncValidationSummary};
use super::*;

#[derive(Default)]
struct TableDiffByPath {
    missing: usize,
    stale: usize,
    mismatched: usize,
}

#[derive(Default)]
struct TableDiff {
    missing: usize,
    stale: usize,
    mismatched: usize,
    by_path: std::collections::HashMap<String, TableDiffByPath>,
}

struct TempValidationDirCleanup {
    path: PathBuf,
}

impl Drop for TempValidationDirCleanup {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

#[cfg(test)]
pub(crate) async fn execute_sync_validation(
    cfg: &DevqlConfig,
    relational: &RelationalStorage,
) -> Result<SyncSummary> {
    execute_sync_validation_with_observer(cfg, relational, None).await
}

pub(crate) async fn execute_sync_validation_with_observer(
    cfg: &DevqlConfig,
    relational: &RelationalStorage,
    observer: Option<&dyn SyncObserver>,
) -> Result<SyncSummary> {
    let temp_parent = std::env::temp_dir().join("bitloops").join("sync-validate");
    fs::create_dir_all(&temp_parent).with_context(|| {
        format!(
            "creating temporary sync validation directory at {}",
            temp_parent.display()
        )
    })?;
    let temp_run_dir = temp_parent.join(format!("sync-validate-{}", Uuid::new_v4()));
    fs::create_dir(&temp_run_dir).with_context(|| {
        format!(
            "creating temporary sync validation run directory at {}",
            temp_run_dir.display()
        )
    })?;
    let _cleanup = TempValidationDirCleanup {
        path: temp_run_dir.clone(),
    };
    let sqlite_path = temp_run_dir.join("sync_validate.sqlite");
    init_sqlite_schema(&sqlite_path)
        .await
        .context("initialising temporary SQLite schema for sync validation")?;
    let expected_store =
        crate::host::relational_store::DefaultRelationalStore::local_only(sqlite_path).into_inner();
    emit_validation_progress(
        observer,
        SyncProgressPhase::BuildingValidationProjection,
        None,
    );
    let expected_projection = if let Some(observer) = observer {
        let validation_observer = ValidationProjectionObserver { inner: observer };
        super::orchestrator::execute_sync_with_observer(
            cfg,
            &expected_store,
            sync::types::SyncMode::Full,
            Some(&validation_observer),
        )
        .await?
    } else {
        super::orchestrator::execute_sync(cfg, &expected_store, sync::types::SyncMode::Full).await?
    };

    emit_validation_progress(
        observer,
        SyncProgressPhase::LoadingValidationRows,
        Some(&expected_projection),
    );
    let expected_artefacts = load_artefact_rows(&expected_store, &cfg.repo.repo_id).await?;
    let actual_artefacts = load_artefact_rows(relational, &cfg.repo.repo_id).await?;
    let expected_edges = load_edge_rows(&expected_store, &cfg.repo.repo_id).await?;
    let actual_edges = load_edge_rows(relational, &cfg.repo.repo_id).await?;

    emit_validation_progress(
        observer,
        SyncProgressPhase::ComparingValidationRows,
        Some(&expected_projection),
    );
    let artefact_diff = compare_rows_by_key(
        &expected_artefacts,
        &actual_artefacts,
        &["path", "symbol_id"],
    );
    let edge_diff = compare_rows_by_key(&expected_edges, &actual_edges, &["path", "edge_id"]);
    let files_with_drift = merge_file_drift(&artefact_diff, &edge_diff);

    let validation = SyncValidationSummary {
        valid: artefact_diff.missing == 0
            && artefact_diff.stale == 0
            && artefact_diff.mismatched == 0
            && edge_diff.missing == 0
            && edge_diff.stale == 0
            && edge_diff.mismatched == 0,
        expected_artefacts: expected_artefacts.len(),
        actual_artefacts: actual_artefacts.len(),
        expected_edges: expected_edges.len(),
        actual_edges: actual_edges.len(),
        missing_artefacts: artefact_diff.missing,
        stale_artefacts: artefact_diff.stale,
        mismatched_artefacts: artefact_diff.mismatched,
        missing_edges: edge_diff.missing,
        stale_edges: edge_diff.stale,
        mismatched_edges: edge_diff.mismatched,
        files_with_drift,
    };

    Ok(SyncSummary {
        success: validation.valid,
        mode: "validate".to_string(),
        parser_version: expected_projection.parser_version,
        extractor_version: expected_projection.extractor_version,
        active_branch: expected_projection.active_branch,
        head_commit_sha: expected_projection.head_commit_sha,
        head_tree_sha: expected_projection.head_tree_sha,
        paths_unchanged: expected_projection.paths_unchanged,
        paths_added: expected_projection.paths_added,
        paths_changed: expected_projection.paths_changed,
        paths_removed: expected_projection.paths_removed,
        cache_hits: expected_projection.cache_hits,
        cache_misses: expected_projection.cache_misses,
        parse_errors: expected_projection.parse_errors,
        validation: Some(validation),
    })
}

struct ValidationProjectionObserver<'a> {
    inner: &'a dyn SyncObserver,
}

impl SyncObserver for ValidationProjectionObserver<'_> {
    fn on_progress(&self, mut update: SyncProgressUpdate) {
        update.phase = SyncProgressPhase::BuildingValidationProjection;
        self.inner.on_progress(update);
    }
}

fn emit_validation_progress(
    observer: Option<&dyn SyncObserver>,
    phase: SyncProgressPhase,
    summary: Option<&SyncSummary>,
) {
    let Some(observer) = observer else {
        return;
    };

    let progress = summary
        .map(|summary| validation_progress_from_summary(summary, phase))
        .unwrap_or_else(|| SyncProgressUpdate {
            phase,
            ..SyncProgressUpdate::default()
        });
    observer.on_progress(progress);
}

fn validation_progress_from_summary(
    summary: &SyncSummary,
    phase: SyncProgressPhase,
) -> SyncProgressUpdate {
    let total = summary.paths_unchanged
        + summary.paths_added
        + summary.paths_changed
        + summary.paths_removed;
    SyncProgressUpdate {
        phase,
        current_path: None,
        paths_total: total,
        paths_completed: total,
        paths_remaining: 0,
        paths_unchanged: summary.paths_unchanged,
        paths_added: summary.paths_added,
        paths_changed: summary.paths_changed,
        paths_removed: summary.paths_removed,
        cache_hits: summary.cache_hits,
        cache_misses: summary.cache_misses,
        parse_errors: summary.parse_errors,
    }
}

async fn load_artefact_rows(
    relational: &RelationalStorage,
    repo_id: &str,
) -> Result<Vec<serde_json::Map<String, Value>>> {
    let rows = relational
        .query_rows(&format!(
            "SELECT repo_id, path, content_id, symbol_id, artefact_id, language, canonical_kind, language_kind, symbol_fqn, parent_symbol_id, parent_artefact_id, start_line, end_line, start_byte, end_byte, signature, modifiers, docstring \
             FROM artefacts_current \
             WHERE repo_id = '{}' \
             ORDER BY path, symbol_id",
            esc_pg(repo_id),
        ))
        .await?;
    rows_to_objects(rows, "artefacts_current")
}

async fn load_edge_rows(
    relational: &RelationalStorage,
    repo_id: &str,
) -> Result<Vec<serde_json::Map<String, Value>>> {
    let rows = relational
        .query_rows(&format!(
            "SELECT repo_id, edge_id, path, content_id, from_symbol_id, from_artefact_id, to_symbol_id, to_artefact_id, to_symbol_ref, edge_kind, language, start_line, end_line, metadata \
             FROM artefact_edges_current \
             WHERE repo_id = '{}' \
             ORDER BY path, edge_id",
            esc_pg(repo_id),
        ))
        .await?;
    rows_to_objects(rows, "artefact_edges_current")
}

fn rows_to_objects(
    rows: Vec<Value>,
    table_name: &str,
) -> Result<Vec<serde_json::Map<String, Value>>> {
    rows.into_iter()
        .enumerate()
        .map(|(index, row)| {
            row.as_object().cloned().ok_or_else(|| {
                anyhow!(
                    "expected object row from `{table_name}` at index {index}, got {}",
                    row
                )
            })
        })
        .collect()
}

fn compare_rows_by_key(
    expected_rows: &[serde_json::Map<String, Value>],
    actual_rows: &[serde_json::Map<String, Value>],
    key_columns: &[&str],
) -> TableDiff {
    let expected = expected_rows
        .iter()
        .filter_map(|row| row_key(row, key_columns).map(|key| (key, row.clone())))
        .collect::<std::collections::HashMap<_, _>>();
    let actual = actual_rows
        .iter()
        .filter_map(|row| row_key(row, key_columns).map(|key| (key, row.clone())))
        .collect::<std::collections::HashMap<_, _>>();
    let mut diff = TableDiff::default();

    for (key, expected_row) in &expected {
        match actual.get(key) {
            None => {
                diff.missing += 1;
                diff.by_path
                    .entry(row_path(expected_row))
                    .or_default()
                    .missing += 1;
            }
            Some(actual_row) if expected_row != actual_row => {
                diff.mismatched += 1;
                diff.by_path
                    .entry(row_path(expected_row))
                    .or_default()
                    .mismatched += 1;
            }
            _ => {}
        }
    }

    for (key, actual_row) in &actual {
        if !expected.contains_key(key) {
            diff.stale += 1;
            diff.by_path.entry(row_path(actual_row)).or_default().stale += 1;
        }
    }

    diff
}

fn row_key(row: &serde_json::Map<String, Value>, key_columns: &[&str]) -> Option<String> {
    let mut values = Vec::with_capacity(key_columns.len());
    for column in key_columns {
        values.push(row.get(*column)?.to_string());
    }
    Some(values.join("|"))
}

fn row_path(row: &serde_json::Map<String, Value>) -> String {
    row.get("path")
        .and_then(Value::as_str)
        .unwrap_or("<unknown>")
        .to_string()
}

fn merge_file_drift(
    artefact_diff: &TableDiff,
    edge_diff: &TableDiff,
) -> Vec<SyncValidationFileDrift> {
    let mut files = std::collections::HashMap::<String, SyncValidationFileDrift>::new();

    for (path, counts) in &artefact_diff.by_path {
        let entry = files
            .entry(path.clone())
            .or_insert_with(|| SyncValidationFileDrift {
                path: path.clone(),
                ..SyncValidationFileDrift::default()
            });
        entry.missing_artefacts += counts.missing;
        entry.stale_artefacts += counts.stale;
        entry.mismatched_artefacts += counts.mismatched;
    }

    for (path, counts) in &edge_diff.by_path {
        let entry = files
            .entry(path.clone())
            .or_insert_with(|| SyncValidationFileDrift {
                path: path.clone(),
                ..SyncValidationFileDrift::default()
            });
        entry.missing_edges += counts.missing;
        entry.stale_edges += counts.stale;
        entry.mismatched_edges += counts.mismatched;
    }

    let mut drift = files
        .into_values()
        .filter(|file| {
            file.missing_artefacts > 0
                || file.stale_artefacts > 0
                || file.mismatched_artefacts > 0
                || file.missing_edges > 0
                || file.stale_edges > 0
                || file.mismatched_edges > 0
        })
        .collect::<Vec<_>>();
    drift.sort_by(|left, right| left.path.cmp(&right.path));
    drift
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compare_rows_by_key_reports_missing_stale_and_mismatched_by_path() {
        let expected = vec![
            serde_json::json!({
                "path": "src/lib.rs",
                "symbol_id": "a",
                "kind": "function"
            })
            .as_object()
            .expect("object")
            .clone(),
            serde_json::json!({
                "path": "src/lib.rs",
                "symbol_id": "b",
                "kind": "module"
            })
            .as_object()
            .expect("object")
            .clone(),
        ];
        let actual = vec![
            serde_json::json!({
                "path": "src/lib.rs",
                "symbol_id": "a",
                "kind": "module"
            })
            .as_object()
            .expect("object")
            .clone(),
            serde_json::json!({
                "path": "src/main.rs",
                "symbol_id": "x",
                "kind": "function"
            })
            .as_object()
            .expect("object")
            .clone(),
        ];

        let diff = compare_rows_by_key(&expected, &actual, &["path", "symbol_id"]);
        assert_eq!(diff.missing, 1);
        assert_eq!(diff.stale, 1);
        assert_eq!(diff.mismatched, 1);
        assert_eq!(diff.by_path.get("src/lib.rs").map(|d| d.missing), Some(1));
        assert_eq!(
            diff.by_path.get("src/lib.rs").map(|d| d.mismatched),
            Some(1)
        );
        assert_eq!(diff.by_path.get("src/main.rs").map(|d| d.stale), Some(1));
    }
}
