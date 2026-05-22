use std::collections::HashSet;
use std::path::Path;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};

use super::*;

pub(super) fn sync_internal_ignored_paths(
    relational: &RelationalStorage,
    repo_root: &Path,
) -> HashSet<String> {
    let mut ignored = HashSet::new();
    let sqlite_path = relational.sqlite_path();
    let Some(relative) = sqlite_path
        .strip_prefix(repo_root)
        .ok()
        .map(|path| normalize_repo_path(path.to_string_lossy().as_ref()))
        .filter(|path| !path.is_empty())
    else {
        return ignored;
    };

    ignored.insert(relative.clone());
    ignored
}

pub(super) fn is_sync_internal_ignored_path(path: &str, ignored: &HashSet<String>) -> bool {
    ignored
        .iter()
        .any(|base| path == base || path.starts_with(&format!("{base}-")))
}

#[allow(clippy::too_many_arguments)]
pub(super) fn handle_prepared_outcome(
    counters: &mut sync::types::SyncCounters,
    stats: &mut SyncExecutionStats,
    writer: &mut SqliteSyncWriter,
    outcome: super::sqlite_writer::PreparedSyncOutcome,
    observer: Option<&dyn SyncObserver>,
    paths_total: usize,
    unchanged_total: usize,
    removed_completed: usize,
    transform_paths_total: usize,
    extracted_completed: &mut usize,
    materialized_completed: &mut usize,
    paths_completed: &mut usize,
) -> Result<()> {
    if let Some(error_message) = outcome.error_message.as_deref() {
        log::warn!(
            "skipping sync path `{}` due prepare failure: {error_message}",
            outcome.path
        );
    }
    stats.add_prepared_path(&outcome.stats);
    if outcome.cache_hit {
        counters.cache_hits += 1;
    }
    if outcome.cache_miss {
        counters.cache_misses += 1;
    }
    if outcome.parse_error {
        counters.parse_errors += 1;
    }

    *extracted_completed += 1;
    if outcome.prepared_item.is_none() && outcome.parse_error {
        *materialized_completed += 1;
    }
    *paths_completed = estimate_sync_progress_paths_completed(
        unchanged_total,
        removed_completed,
        transform_paths_total,
        *extracted_completed,
        *materialized_completed,
    );
    emit_progress(
        observer,
        SyncProgressPhase::ExtractingPaths,
        Some(outcome.path),
        counters,
        paths_total,
        *paths_completed,
    );

    if let Some(prepared_item) = outcome.prepared_item {
        writer.push_item(prepared_item);
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn flush_pending_materialisations(
    writer: &mut SqliteSyncWriter,
    repo_id: &str,
    parser_version: &str,
    extractor_version: &str,
    diff_collector: &mut SyncDiffCollector,
    stats: &mut SyncExecutionStats,
    observer: Option<&dyn SyncObserver>,
    counters: &sync::types::SyncCounters,
    paths_total: usize,
    unchanged_total: usize,
    removed_completed: usize,
    transform_paths_total: usize,
    extracted_completed: usize,
    materialized_completed: &mut usize,
    paths_completed: &mut usize,
    touched_paths: &mut HashSet<String>,
) -> Result<()> {
    if !writer.has_pending_items() {
        return Ok(());
    }

    let flush_started = Instant::now();
    let outcome = writer
        .flush(repo_id, parser_version, extractor_version)
        .await
        .context("flushing pending SQLite sync materialisations")?;
    for artefact in outcome.pre_artefacts.clone() {
        diff_collector.record_pre_artefacts(artefact.path.clone(), vec![artefact]);
    }
    for artefact in outcome.post_artefacts.clone() {
        diff_collector.record_post_artefacts(artefact.path.clone(), vec![artefact]);
    }
    let flush_duration = flush_started.elapsed();
    stats.add_writer_commit(outcome.sqlite_commits, outcome.sqlite_rows_written);
    apply_writer_duration(stats, flush_duration, &outcome);

    for path in outcome.materialized_paths {
        touched_paths.insert(path.clone());
        *materialized_completed += 1;
        *paths_completed = estimate_sync_progress_paths_completed(
            unchanged_total,
            removed_completed,
            transform_paths_total,
            extracted_completed,
            *materialized_completed,
        );
        emit_progress(
            observer,
            SyncProgressPhase::MaterialisingPaths,
            Some(path),
            counters,
            paths_total,
            *paths_completed,
        );
    }
    Ok(())
}

pub(super) fn apply_writer_duration(
    stats: &mut SyncExecutionStats,
    duration: Duration,
    outcome: &WriterCommitOutcome,
) {
    let total_estimate =
        outcome.cache_store_operation_estimate + outcome.materialisation_operation_estimate;
    if total_estimate == 0 {
        return;
    }

    let cache_ratio = outcome.cache_store_operation_estimate as f64 / total_estimate as f64;
    let cache_duration = duration.mul_f64(cache_ratio);
    stats.cache_store_total += cache_duration;
    stats.materialisation_total += duration.checked_sub(cache_duration).unwrap_or_default();
}

pub(super) fn estimate_sync_progress_paths_completed(
    unchanged_total: usize,
    removed_completed: usize,
    transform_paths_total: usize,
    extracted_completed: usize,
    materialized_completed: usize,
) -> usize {
    let transform_units_total = transform_paths_total.saturating_mul(2);
    let transform_units_completed = extracted_completed
        .saturating_add(materialized_completed)
        .min(transform_units_total);
    let transform_credit = transform_units_completed
        .saturating_add(1)
        .checked_div(2)
        .unwrap_or(0)
        .min(transform_paths_total);

    unchanged_total
        .saturating_add(removed_completed)
        .saturating_add(transform_credit)
}

#[cfg(test)]
mod tests {
    use super::estimate_sync_progress_paths_completed;

    #[test]
    fn extraction_phase_contributes_partial_progress() {
        assert_eq!(estimate_sync_progress_paths_completed(0, 0, 4, 1, 0), 1);
        assert_eq!(estimate_sync_progress_paths_completed(0, 0, 4, 2, 0), 1);
        assert_eq!(estimate_sync_progress_paths_completed(0, 0, 4, 3, 0), 2);
    }

    #[test]
    fn completed_transform_work_reaches_total_paths() {
        assert_eq!(estimate_sync_progress_paths_completed(5, 2, 4, 4, 4), 11);
    }
}
