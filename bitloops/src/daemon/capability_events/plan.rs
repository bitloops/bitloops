use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};
use rusqlite::params;

use crate::host::capability_host::{
    ChangedArtefact, ChangedFile, CurrentStateConsumer, CurrentStateConsumerRequest,
    CurrentStateConsumerResult, DevqlCapabilityHost, ReconcileMode, RemovedArtefact, RemovedFile,
};

use super::super::types::{CapabilityEventRunRecord, unix_timestamp_now};
use super::queue::{
    ArtefactChangeRow, FileChangeRow, GenerationRow, latest_generation_seq, load_artefact_changes,
    load_consumer_cursor, load_file_changes, load_generations, sql_i64,
};

#[derive(Debug, Clone)]
pub(super) struct ExecutionPlan {
    pub(super) record: CapabilityEventRunRecord,
    pub(super) repo_root: PathBuf,
    pub(super) request: CurrentStateConsumerRequest,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum MergedFileChange {
    Upsert(ChangedFile),
    Removed(RemovedFile),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum MergedArtefactChange {
    Upsert(ChangedArtefact),
    Removed(RemovedArtefact),
}

pub(super) fn build_execution_plan(
    conn: &rusqlite::Connection,
    run: &CapabilityEventRunRecord,
    repo_root: &Path,
) -> Result<Option<ExecutionPlan>> {
    let Some(cursor) =
        load_consumer_cursor(conn, &run.repo_id, &run.capability_id, &run.consumer_id)?
    else {
        return Ok(None);
    };
    let Some(latest_generation_seq) = latest_generation_seq(conn, &run.repo_id)? else {
        return Ok(None);
    };
    let from_generation_seq_exclusive = cursor.last_applied_generation_seq.unwrap_or(0);
    if latest_generation_seq <= from_generation_seq_exclusive {
        return Ok(None);
    }

    let generations = load_generations(
        conn,
        &run.repo_id,
        from_generation_seq_exclusive + 1,
        latest_generation_seq,
    )?;
    if generations.is_empty() {
        return Ok(None);
    }

    let file_changes = load_file_changes(
        conn,
        &run.repo_id,
        from_generation_seq_exclusive + 1,
        latest_generation_seq,
    )?;
    let artefact_changes = load_artefact_changes(
        conn,
        &run.repo_id,
        from_generation_seq_exclusive + 1,
        latest_generation_seq,
    )?;
    let merged_files = merge_file_changes(&file_changes);
    let merged_artefacts = merge_artefact_changes(&artefact_changes);
    let reconcile_mode = determine_reconcile_mode(
        cursor.last_applied_generation_seq,
        &generations,
        merged_files.len(),
        merged_artefacts.len(),
    );
    let (file_upserts, file_removals) = partition_file_changes(merged_files);
    let (artefact_upserts, artefact_removals) = partition_artefact_changes(merged_artefacts);
    let latest_generation = generations
        .last()
        .expect("checked non-empty generations before building execution plan");

    let mut record = run.clone();
    record.from_generation_seq = from_generation_seq_exclusive;
    record.to_generation_seq = latest_generation_seq;
    record.reconcile_mode = reconcile_mode_label(reconcile_mode).to_string();
    let run_id = record.run_id.clone();
    let reconcile_mode_label = record.reconcile_mode.clone();

    conn.execute(
        "UPDATE capability_workplane_cursor_runs SET from_generation_seq = ?1, to_generation_seq = ?2, reconcile_mode = ?3, updated_at_unix = ?4 WHERE run_id = ?5",
        params![
            sql_i64(record.from_generation_seq)?,
            sql_i64(record.to_generation_seq)?,
            &reconcile_mode_label,
            sql_i64(unix_timestamp_now())?,
            &run_id,
        ],
    )
    .with_context(|| {
        format!(
            "refreshing current-state consumer execution bounds for `{}`",
            run_id
        )
    })?;

    Ok(Some(ExecutionPlan {
        record,
        repo_root: repo_root.to_path_buf(),
        request: CurrentStateConsumerRequest {
            repo_id: run.repo_id.clone(),
            repo_root: repo_root.to_path_buf(),
            active_branch: latest_generation.active_branch.clone(),
            head_commit_sha: latest_generation.head_commit_sha.clone(),
            from_generation_seq_exclusive,
            to_generation_seq_inclusive: latest_generation_seq,
            reconcile_mode,
            file_upserts,
            file_removals,
            artefact_upserts,
            artefact_removals,
        },
    }))
}

pub(super) fn merge_file_changes(rows: &[FileChangeRow]) -> Vec<MergedFileChange> {
    let mut merged = BTreeMap::new();
    for row in rows {
        let _ = row.generation_seq;
        match row.change_kind.as_str() {
            "added" | "changed" => {
                if let (Some(language), Some(content_id)) = (&row.language, &row.content_id) {
                    merged.insert(
                        row.path.clone(),
                        MergedFileChange::Upsert(ChangedFile {
                            path: row.path.clone(),
                            language: language.clone(),
                            content_id: content_id.clone(),
                        }),
                    );
                }
            }
            "removed" => {
                merged.insert(
                    row.path.clone(),
                    MergedFileChange::Removed(RemovedFile {
                        path: row.path.clone(),
                    }),
                );
            }
            _ => {}
        }
    }
    merged.into_values().collect()
}

pub(super) fn merge_artefact_changes(rows: &[ArtefactChangeRow]) -> Vec<MergedArtefactChange> {
    let mut merged = BTreeMap::new();
    for row in rows {
        let _ = row.generation_seq;
        match row.change_kind.as_str() {
            "added" | "changed" => {
                merged.insert(
                    row.symbol_id.clone(),
                    MergedArtefactChange::Upsert(ChangedArtefact {
                        artefact_id: row.artefact_id.clone(),
                        symbol_id: row.symbol_id.clone(),
                        path: row.path.clone(),
                        canonical_kind: row.canonical_kind.clone(),
                        name: row.name.clone(),
                    }),
                );
            }
            "removed" => {
                merged.insert(
                    row.symbol_id.clone(),
                    MergedArtefactChange::Removed(RemovedArtefact {
                        artefact_id: row.artefact_id.clone(),
                        symbol_id: row.symbol_id.clone(),
                        path: row.path.clone(),
                    }),
                );
            }
            _ => {}
        }
    }
    merged.into_values().collect()
}

fn partition_file_changes(merged: Vec<MergedFileChange>) -> (Vec<ChangedFile>, Vec<RemovedFile>) {
    let mut upserts = Vec::new();
    let mut removals = Vec::new();
    for change in merged {
        match change {
            MergedFileChange::Upsert(file) => upserts.push(file),
            MergedFileChange::Removed(file) => removals.push(file),
        }
    }
    (upserts, removals)
}

fn partition_artefact_changes(
    merged: Vec<MergedArtefactChange>,
) -> (Vec<ChangedArtefact>, Vec<RemovedArtefact>) {
    let mut upserts = Vec::new();
    let mut removals = Vec::new();
    for change in merged {
        match change {
            MergedArtefactChange::Upsert(artefact) => upserts.push(artefact),
            MergedArtefactChange::Removed(artefact) => removals.push(artefact),
        }
    }
    (upserts, removals)
}

pub(super) fn determine_reconcile_mode(
    last_applied_generation_seq: Option<u64>,
    generations: &[GenerationRow],
    merged_file_count: usize,
    merged_artefact_count: usize,
) -> ReconcileMode {
    let Some(last_generation) = generations.last() else {
        return ReconcileMode::MergedDelta;
    };
    let pending_generation_span = last_generation
        .generation_seq
        .saturating_sub(last_applied_generation_seq.unwrap_or(0));
    if last_applied_generation_seq.is_none()
        || generations
            .iter()
            .any(|generation| generation.requires_full_reconcile)
        || pending_generation_span > 64
        || merged_file_count > 2_000
        || merged_artefact_count > 5_000
    {
        ReconcileMode::FullReconcile
    } else {
        ReconcileMode::MergedDelta
    }
}

fn reconcile_mode_label(mode: ReconcileMode) -> &'static str {
    match mode {
        ReconcileMode::MergedDelta => "merged_delta",
        ReconcileMode::FullReconcile => "full_reconcile",
    }
}

pub(super) fn find_current_state_consumer<'a>(
    host: &'a DevqlCapabilityHost,
    run: &CapabilityEventRunRecord,
) -> Option<&'a Arc<dyn CurrentStateConsumer>> {
    let mailbox = host.mailbox_registration(&run.capability_id, &run.consumer_id)?;
    let crate::host::capability_host::CapabilityMailboxHandler::CurrentStateConsumer(handler_id) =
        mailbox.handler
    else {
        return None;
    };
    host.current_state_consumers()
        .iter()
        .find(|registration| {
            registration.capability_id == run.capability_id
                && registration.consumer_id == handler_id
        })
        .map(|registration| &registration.handler)
}

pub(super) fn validate_consumer_result(
    request: &CurrentStateConsumerRequest,
    result: &CurrentStateConsumerResult,
) -> Result<()> {
    if result.applied_to_generation_seq < request.from_generation_seq_exclusive + 1
        || result.applied_to_generation_seq > request.to_generation_seq_inclusive
    {
        anyhow::bail!(
            "consumer applied generation {} outside requested range {}..={}",
            result.applied_to_generation_seq,
            request.from_generation_seq_exclusive + 1,
            request.to_generation_seq_inclusive
        );
    }
    Ok(())
}
