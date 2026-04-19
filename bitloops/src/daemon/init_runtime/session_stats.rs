use std::path::Path;

use anyhow::Result;
use rusqlite::{OptionalExtension, params};

use crate::capability_packs::semantic_clones::types::{
    SEMANTIC_CLONES_CLONE_REBUILD_MAILBOX, SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX,
    SEMANTIC_CLONES_SUMMARY_EMBEDDING_MAILBOX, SEMANTIC_CLONES_SUMMARY_REFRESH_MAILBOX,
};
use crate::capability_packs::semantic_clones::workplane::{
    payload_artefact_id, payload_is_repo_backfill, payload_repo_backfill_artefact_ids,
    payload_work_item_count,
};
use crate::host::relational_store::DefaultRelationalStore;
use crate::host::runtime_store::{RepoSqliteRuntimeStore, SemanticMailboxItemKind};

use super::progress::load_summary_freshness_state;
use super::stats::{
    SessionWorkplaneStats, StatusCounts, SummaryFreshnessState, is_init_embeddings_mailbox,
    mailbox_stats_mut, semantic_embedding_mailbox_name_for_representation,
    semantic_embedding_representation_kind_for_mailbox, stats_for_mailbox,
};
use super::workplane::repo_blocked_mailboxes;

pub(crate) fn load_session_workplane_stats(
    repo_root: &Path,
    repo_store: &RepoSqliteRuntimeStore,
    repo_id: &str,
    init_session_id: &str,
) -> Result<SessionWorkplaneStats> {
    let sqlite = repo_store.connect_repo_sqlite()?;
    let summary_freshness = load_summary_freshness_state_for_repo(repo_root, repo_id)
        .unwrap_or_else(|err| {
            log::debug!(
                "failed to load summary freshness state for repo `{repo_id}` at `{}`: {err:#}",
                repo_root.display()
            );
            SummaryFreshnessState::default()
        });
    sqlite.with_connection(|conn| {
        let mut stats = SessionWorkplaneStats::default();

        let mut cursor_stmt = conn.prepare(
            "SELECT status, COUNT(*)
             FROM capability_workplane_cursor_runs
             WHERE repo_id = ?1 AND init_session_id = ?2
             GROUP BY status",
        )?;
        let cursor_rows = cursor_stmt.query_map([repo_id, init_session_id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        })?;
        for row in cursor_rows {
            let (status, count) = row?;
            let count = u64::try_from(count).unwrap_or_default();
            match status.as_str() {
                "queued" => stats.current_state.pending += count,
                "running" => stats.current_state.running += count,
                "completed" => stats.current_state.completed += count,
                "failed" | "cancelled" => stats.current_state.failed += count,
                _ => {}
            }
        }
        stats.failed_current_state_detail = conn
            .query_row(
                "SELECT run_id, error
                 FROM capability_workplane_cursor_runs
                 WHERE repo_id = ?1
                   AND init_session_id = ?2
                   AND status IN ('failed', 'cancelled')
                 ORDER BY updated_at_unix DESC
                 LIMIT 1",
                params![repo_id, init_session_id],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?)),
            )
            .optional()?
            .map(|(run_id, error)| {
                format!(
                    "Applying codebase updates failed for run `{run_id}`{}",
                    error
                        .as_deref()
                        .map(|error| format!(": {error}"))
                        .unwrap_or_default()
                )
            });

        let mut job_stmt = conn.prepare(
            "SELECT mailbox_name, status, payload
             FROM capability_workplane_jobs
             WHERE repo_id = ?1 AND init_session_id = ?2",
        )?;
        let job_rows = job_stmt.query_map([repo_id, init_session_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })?;
        for row in job_rows {
            let (mailbox_name, status, payload_json) = row?;
            let target = mailbox_stats_mut(&mut stats, mailbox_name.as_str());
            let payload = serde_json::from_str::<serde_json::Value>(&payload_json)
                .unwrap_or(serde_json::Value::Null);
            let count = effective_session_work_item_count(
                mailbox_name.as_str(),
                status.as_str(),
                &payload,
                &summary_freshness,
            );
            match status.as_str() {
                "pending" => target.counts.pending += count,
                "running" => target.counts.running += count,
                "completed" => target.counts.completed += count,
                "failed" => target.counts.failed += count,
                _ => {}
            }
        }
        load_semantic_summary_session_mailbox_counts(
            conn,
            &mut stats,
            repo_id,
            init_session_id,
            &summary_freshness,
        )?;
        load_semantic_embedding_session_mailbox_counts(conn, &mut stats, repo_id, init_session_id)?;
        stats.refresh_lane_counts();
        stats.summary_refresh_jobs.latest_error = latest_mailbox_error(
            conn,
            repo_id,
            init_session_id,
            SEMANTIC_CLONES_SUMMARY_REFRESH_MAILBOX,
        )?;
        stats.code_embedding_jobs.latest_error = latest_mailbox_error(
            conn,
            repo_id,
            init_session_id,
            SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX,
        )?;
        stats.summary_embedding_jobs.latest_error = latest_mailbox_error(
            conn,
            repo_id,
            init_session_id,
            SEMANTIC_CLONES_SUMMARY_EMBEDDING_MAILBOX,
        )?;
        stats.clone_rebuild_jobs.latest_error = latest_mailbox_error(
            conn,
            repo_id,
            init_session_id,
            SEMANTIC_CLONES_CLONE_REBUILD_MAILBOX,
        )?;

        for blocked in repo_blocked_mailboxes(repo_store.db_path().to_path_buf(), repo_id)? {
            if blocked.mailbox_name == SEMANTIC_CLONES_SUMMARY_REFRESH_MAILBOX {
                if stats.summary_refresh_jobs.counts.has_pending_or_running() {
                    stats
                        .blocked_summary_reason
                        .get_or_insert(blocked.reason.clone());
                }
                continue;
            }
            if is_init_embeddings_mailbox(blocked.mailbox_name.as_str())
                && stats_for_mailbox(&stats, blocked.mailbox_name.as_str()).has_pending_or_running()
            {
                stats
                    .blocked_embedding_reason
                    .get_or_insert(blocked.reason.clone());
            }
        }

        Ok(stats)
    })
}

fn latest_mailbox_error(
    conn: &rusqlite::Connection,
    repo_id: &str,
    init_session_id: &str,
    mailbox_name: &str,
) -> rusqlite::Result<Option<String>> {
    let legacy = conn
        .query_row(
            "SELECT last_error, updated_at_unix
             FROM capability_workplane_jobs
             WHERE repo_id = ?1
               AND init_session_id = ?2
               AND status = 'failed'
               AND mailbox_name = ?3
             ORDER BY updated_at_unix DESC
             LIMIT 1",
            params![repo_id, init_session_id, mailbox_name],
            |row| Ok((row.get::<_, i64>(1)?, row.get::<_, Option<String>>(0)?)),
        )
        .optional()?;
    let semantic = latest_semantic_mailbox_error(conn, repo_id, init_session_id, mailbox_name)?;
    Ok(match (legacy, semantic) {
        (Some((legacy_updated_at, legacy_error)), Some((semantic_updated_at, semantic_error))) => {
            if semantic_updated_at >= legacy_updated_at {
                semantic_error
            } else {
                legacy_error
            }
        }
        (Some((_, error)), None) | (None, Some((_, error))) => error,
        (None, None) => None,
    })
}

fn latest_semantic_mailbox_error(
    conn: &rusqlite::Connection,
    repo_id: &str,
    init_session_id: &str,
    mailbox_name: &str,
) -> rusqlite::Result<Option<(i64, Option<String>)>> {
    match mailbox_name {
        SEMANTIC_CLONES_SUMMARY_REFRESH_MAILBOX => conn
            .query_row(
                "SELECT last_error, updated_at_unix
                 FROM semantic_summary_mailbox_items
                 WHERE repo_id = ?1
                   AND init_session_id = ?2
                   AND status = 'failed'
                 ORDER BY updated_at_unix DESC
                 LIMIT 1",
                params![repo_id, init_session_id],
                |row| Ok((row.get::<_, i64>(1)?, row.get::<_, Option<String>>(0)?)),
            )
            .optional(),
        SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX | SEMANTIC_CLONES_SUMMARY_EMBEDDING_MAILBOX => {
            let representation_kind =
                semantic_embedding_representation_kind_for_mailbox(mailbox_name);
            conn.query_row(
                "SELECT last_error, updated_at_unix
                 FROM semantic_embedding_mailbox_items
                 WHERE repo_id = ?1
                   AND init_session_id = ?2
                   AND representation_kind = ?3
                   AND status = 'failed'
                 ORDER BY updated_at_unix DESC
                 LIMIT 1",
                params![repo_id, init_session_id, representation_kind],
                |row| Ok((row.get::<_, i64>(1)?, row.get::<_, Option<String>>(0)?)),
            )
            .optional()
        }
        _ => Ok(None),
    }
}

pub(crate) fn load_semantic_summary_session_mailbox_counts(
    conn: &rusqlite::Connection,
    stats: &mut SessionWorkplaneStats,
    repo_id: &str,
    init_session_id: &str,
    summary_freshness: &SummaryFreshnessState,
) -> rusqlite::Result<()> {
    let mut stmt = conn.prepare(
        "SELECT status, item_kind, artefact_id, payload_json
         FROM semantic_summary_mailbox_items
         WHERE repo_id = ?1 AND init_session_id = ?2",
    )?;
    let rows = stmt.query_map(params![repo_id, init_session_id], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, Option<String>>(2)?,
            row.get::<_, Option<String>>(3)?,
        ))
    })?;
    for row in rows {
        let (status, item_kind, artefact_id, payload_json) = row?;
        let count = effective_session_summary_mailbox_item_count(
            status.as_str(),
            item_kind.as_str(),
            artefact_id.as_deref(),
            payload_json.as_deref(),
            summary_freshness,
        );
        record_session_mailbox_count(
            &mut stats.summary_refresh_jobs.counts,
            status.as_str(),
            count,
        );
    }
    Ok(())
}

pub(crate) fn load_semantic_embedding_session_mailbox_counts(
    conn: &rusqlite::Connection,
    stats: &mut SessionWorkplaneStats,
    repo_id: &str,
    init_session_id: &str,
) -> rusqlite::Result<()> {
    let mut stmt = conn.prepare(
        "SELECT representation_kind, status, item_kind, payload_json
         FROM semantic_embedding_mailbox_items
         WHERE repo_id = ?1 AND init_session_id = ?2",
    )?;
    let rows = stmt.query_map(params![repo_id, init_session_id], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, Option<String>>(3)?,
        ))
    })?;
    for row in rows {
        let (representation_kind, status, item_kind, payload_json) = row?;
        let mailbox_name =
            semantic_embedding_mailbox_name_for_representation(representation_kind.as_str());
        let count =
            semantic_mailbox_item_work_item_count(item_kind.as_str(), payload_json.as_deref());
        let target = mailbox_stats_mut(stats, mailbox_name);
        record_session_mailbox_count(&mut target.counts, status.as_str(), count);
    }
    Ok(())
}

fn record_session_mailbox_count(counts: &mut StatusCounts, status: &str, count: u64) {
    match status {
        "pending" => counts.pending += count,
        "running" | "leased" => counts.running += count,
        "completed" => counts.completed += count,
        "failed" => counts.failed += count,
        _ => {}
    }
}

fn effective_session_summary_mailbox_item_count(
    status: &str,
    item_kind: &str,
    artefact_id: Option<&str>,
    payload_json: Option<&str>,
    summary_freshness: &SummaryFreshnessState,
) -> u64 {
    if !matches!(status, "pending" | "leased" | "failed") {
        return semantic_mailbox_item_work_item_count(item_kind, payload_json);
    }

    match SemanticMailboxItemKind::parse(item_kind) {
        SemanticMailboxItemKind::RepoBackfill => {
            let payload = parse_semantic_mailbox_payload_json(payload_json);
            payload
                .as_ref()
                .and_then(payload_repo_backfill_artefact_ids)
                .map(|artefact_ids| {
                    summary_freshness.outstanding_work_item_count_for_artefacts(&artefact_ids)
                })
                .unwrap_or_else(|| summary_freshness.outstanding_work_item_count())
        }
        SemanticMailboxItemKind::Artefact => artefact_id
            .map(|artefact_id| u64::from(summary_freshness.artefact_needs_refresh(artefact_id)))
            .unwrap_or(1),
    }
}

fn semantic_mailbox_item_work_item_count(item_kind: &str, payload_json: Option<&str>) -> u64 {
    match SemanticMailboxItemKind::parse(item_kind) {
        SemanticMailboxItemKind::RepoBackfill => {
            let payload = parse_semantic_mailbox_payload_json(payload_json);
            payload
                .as_ref()
                .and_then(payload_repo_backfill_artefact_ids)
                .map(|artefact_ids| artefact_ids.len() as u64)
                .unwrap_or(1)
        }
        SemanticMailboxItemKind::Artefact => 1,
    }
}

fn parse_semantic_mailbox_payload_json(payload_json: Option<&str>) -> Option<serde_json::Value> {
    payload_json.and_then(|payload_json| serde_json::from_str(payload_json).ok())
}

fn load_summary_freshness_state_for_repo(
    repo_root: &Path,
    repo_id: &str,
) -> Result<SummaryFreshnessState> {
    let relational =
        DefaultRelationalStore::open_local_for_repo_root_preferring_bound_config(repo_root)?;
    load_summary_freshness_state(&relational, repo_id)
}

fn effective_session_work_item_count(
    mailbox_name: &str,
    status: &str,
    payload: &serde_json::Value,
    summary_freshness: &SummaryFreshnessState,
) -> u64 {
    if mailbox_name != SEMANTIC_CLONES_SUMMARY_REFRESH_MAILBOX {
        return payload_work_item_count(payload, mailbox_name);
    }
    match status {
        "pending" | "running" | "failed" => {
            summary_effective_work_item_count(payload, summary_freshness)
        }
        _ => payload_work_item_count(payload, mailbox_name),
    }
}

pub(crate) fn summary_effective_work_item_count(
    payload: &serde_json::Value,
    summary_freshness: &SummaryFreshnessState,
) -> u64 {
    if payload_is_repo_backfill(payload) {
        return payload_repo_backfill_artefact_ids(payload)
            .map(|artefact_ids| {
                summary_freshness.outstanding_work_item_count_for_artefacts(&artefact_ids)
            })
            .unwrap_or_else(|| summary_freshness.outstanding_work_item_count());
    }
    payload_artefact_id(payload)
        .map(|artefact_id| u64::from(summary_freshness.artefact_needs_refresh(&artefact_id)))
        .unwrap_or_else(|| {
            payload_work_item_count(payload, SEMANTIC_CLONES_SUMMARY_REFRESH_MAILBOX)
        })
}
