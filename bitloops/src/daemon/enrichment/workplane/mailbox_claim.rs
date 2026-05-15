use std::collections::BTreeMap;
use std::path::PathBuf;

use anyhow::{Context, Result};
use rusqlite::{OptionalExtension, params};
use uuid::Uuid;

use crate::capability_packs::semantic_clones::embeddings::EmbeddingRepresentationKind;
use crate::capability_packs::semantic_clones::types::{
    SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX, SEMANTIC_CLONES_IDENTITY_EMBEDDING_MAILBOX,
    SEMANTIC_CLONES_SUMMARY_EMBEDDING_MAILBOX, SEMANTIC_CLONES_SUMMARY_REFRESH_MAILBOX,
};
use crate::daemon::types::unix_timestamp_now;
use crate::host::devql::esc_pg;
use crate::host::runtime_store::{
    DaemonSqliteRuntimeStore, SemanticEmbeddingMailboxItemRecord, SemanticMailboxItemKind,
    SemanticMailboxItemStatus, SemanticSummaryMailboxItemRecord,
};

use super::super::{EnrichmentControlState, effective_worker_budgets};
use super::mailbox_persistence::{
    load_embedding_mailbox_items_by_ids, load_summary_mailbox_items_by_ids,
};
use super::readiness::{mailbox_claim_readiness, mailbox_readiness_job};
use super::sql::{sql_i64, sql_string_list};

pub(crate) const SEMANTIC_SUMMARY_MAILBOX_BATCH_SIZE: usize = 16;
pub(crate) const SEMANTIC_EMBEDDING_MAILBOX_BATCH_SIZE: usize = 32;
const SEMANTIC_MAILBOX_LEASE_SECS: u64 = 300;
pub(crate) const WORKPLANE_JOB_CLAIM_CANDIDATE_LIMIT: usize = 32;

#[derive(Debug, Clone)]
pub(crate) struct ClaimedSummaryMailboxBatch {
    pub repo_id: String,
    pub repo_root: PathBuf,
    pub config_root: PathBuf,
    pub lease_token: String,
    pub items: Vec<SemanticSummaryMailboxItemRecord>,
}

#[derive(Debug, Clone)]
pub(crate) struct ClaimedEmbeddingMailboxBatch {
    pub repo_id: String,
    pub repo_root: PathBuf,
    pub config_root: PathBuf,
    pub representation_kind: EmbeddingRepresentationKind,
    pub lease_token: String,
    pub items: Vec<SemanticEmbeddingMailboxItemRecord>,
}

#[derive(Debug, Clone)]
struct EmbeddingMailboxRepoCandidate {
    repo_id: String,
    repo_root: PathBuf,
    config_root: PathBuf,
    representation_kind: EmbeddingRepresentationKind,
}

pub(crate) fn claim_summary_mailbox_batch(
    workplane_store: &DaemonSqliteRuntimeStore,
    runtime_store: &DaemonSqliteRuntimeStore,
    control_state: &EnrichmentControlState,
) -> Result<Option<ClaimedSummaryMailboxBatch>> {
    if control_state.paused_semantic {
        return Ok(None);
    }
    workplane_store.with_connection(|conn| {
        let candidates = load_summary_mailbox_repo_candidates(conn, unix_timestamp_now())?;
        let mut readiness_cache = BTreeMap::new();
        for (repo_id, repo_root, config_root) in candidates {
            let job = mailbox_readiness_job(
                &repo_id,
                &repo_root,
                &config_root,
                SEMANTIC_CLONES_SUMMARY_REFRESH_MAILBOX,
            );
            if mailbox_claim_readiness(runtime_store, &mut readiness_cache, &job)?.blocked {
                continue;
            }
            if let Some(batch) = with_immediate_mailbox_claim_transaction(
                conn,
                "starting semantic summary mailbox claim transaction",
                "committing semantic summary mailbox claim transaction",
                || lease_summary_mailbox_batch_for_repo(conn, &repo_id, unix_timestamp_now()),
            )? {
                return Ok(Some(batch));
            }
        }
        Ok(None)
    })
}

pub(crate) fn claim_embedding_mailbox_batch(
    workplane_store: &DaemonSqliteRuntimeStore,
    runtime_store: &DaemonSqliteRuntimeStore,
    control_state: &EnrichmentControlState,
) -> Result<Option<ClaimedEmbeddingMailboxBatch>> {
    if control_state.paused_embeddings {
        return Ok(None);
    }
    workplane_store.with_connection(|conn| {
        let candidates = load_embedding_mailbox_repo_candidates(conn, unix_timestamp_now())?;
        let candidates =
            prioritize_embedding_mailbox_repo_candidates(conn, workplane_store, candidates)?;
        let mut readiness_cache = BTreeMap::new();
        for candidate in candidates {
            let EmbeddingMailboxRepoCandidate {
                repo_id,
                repo_root,
                config_root,
                representation_kind,
            } = candidate;
            let mailbox_name = match representation_kind {
                EmbeddingRepresentationKind::Summary => SEMANTIC_CLONES_SUMMARY_EMBEDDING_MAILBOX,
                EmbeddingRepresentationKind::Identity => SEMANTIC_CLONES_IDENTITY_EMBEDDING_MAILBOX,
                EmbeddingRepresentationKind::Code => SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX,
            };
            let job = mailbox_readiness_job(&repo_id, &repo_root, &config_root, mailbox_name);
            if mailbox_claim_readiness(runtime_store, &mut readiness_cache, &job)?.blocked {
                continue;
            }
            if let Some(batch) = with_immediate_mailbox_claim_transaction(
                conn,
                "starting semantic embedding mailbox claim transaction",
                "committing semantic embedding mailbox claim transaction",
                || {
                    lease_embedding_mailbox_batch_for_repo(
                        conn,
                        &repo_id,
                        representation_kind,
                        unix_timestamp_now(),
                    )
                },
            )? {
                return Ok(Some(batch));
            }
        }
        Ok(None)
    })
}

fn with_immediate_mailbox_claim_transaction<T>(
    conn: &rusqlite::Connection,
    start_context: &str,
    commit_context: &str,
    action: impl FnOnce() -> Result<T>,
) -> Result<T> {
    conn.execute_batch("BEGIN IMMEDIATE TRANSACTION;")
        .with_context(|| start_context.to_string())?;
    let result = action();
    match result {
        Ok(value) => {
            conn.execute_batch("COMMIT;")
                .with_context(|| commit_context.to_string())?;
            Ok(value)
        }
        Err(err) => {
            let _ = conn.execute_batch("ROLLBACK;");
            Err(err)
        }
    }
}

fn load_summary_mailbox_repo_candidates(
    conn: &rusqlite::Connection,
    now: u64,
) -> Result<Vec<(String, PathBuf, PathBuf)>> {
    let limit = i64::try_from(WORKPLANE_JOB_CLAIM_CANDIDATE_LIMIT)
        .context("converting summary mailbox claim candidate limit")?;
    let mut stmt = conn.prepare(
        "SELECT repo_id, repo_root, config_root, MIN(submitted_at_unix) AS oldest_submitted
         FROM semantic_summary_mailbox_items
         WHERE status = ?1
           AND available_at_unix <= ?2
         GROUP BY repo_id, repo_root, config_root
         ORDER BY oldest_submitted ASC
         LIMIT ?3",
    )?;
    let rows = stmt.query_map(
        params![
            SemanticMailboxItemStatus::Pending.as_str(),
            sql_i64(now)?,
            limit,
        ],
        |row| {
            Ok((
                row.get::<_, String>(0)?,
                PathBuf::from(row.get::<_, String>(1)?),
                PathBuf::from(row.get::<_, String>(2)?),
            ))
        },
    )?;
    let mut values = Vec::new();
    for row in rows {
        values.push(row?);
    }
    Ok(values)
}

fn load_embedding_mailbox_repo_candidates(
    conn: &rusqlite::Connection,
    now: u64,
) -> Result<Vec<EmbeddingMailboxRepoCandidate>> {
    let limit = i64::try_from(WORKPLANE_JOB_CLAIM_CANDIDATE_LIMIT)
        .context("converting embedding mailbox claim candidate limit")?;
    let mut stmt = conn.prepare(
        "SELECT repo_id, repo_root, config_root, representation_kind, MIN(submitted_at_unix) AS oldest_submitted
         FROM semantic_embedding_mailbox_items
         WHERE status = ?1
           AND available_at_unix <= ?2
         GROUP BY repo_id, repo_root, config_root, representation_kind
         ORDER BY oldest_submitted ASC
         LIMIT ?3",
    )?;
    let rows = stmt.query_map(
        params![
            SemanticMailboxItemStatus::Pending.as_str(),
            sql_i64(now)?,
            limit,
        ],
        |row| {
            let representation_kind = match row.get::<_, String>(3)?.as_str() {
                "summary" => EmbeddingRepresentationKind::Summary,
                "identity" | "locator" => EmbeddingRepresentationKind::Identity,
                _ => EmbeddingRepresentationKind::Code,
            };
            Ok(EmbeddingMailboxRepoCandidate {
                repo_id: row.get::<_, String>(0)?,
                repo_root: PathBuf::from(row.get::<_, String>(1)?),
                config_root: PathBuf::from(row.get::<_, String>(2)?),
                representation_kind,
            })
        },
    )?;
    let mut values = Vec::new();
    for row in rows {
        values.push(row?);
    }
    Ok(values)
}

fn prioritize_embedding_mailbox_repo_candidates(
    conn: &rusqlite::Connection,
    workplane_store: &DaemonSqliteRuntimeStore,
    values: Vec<EmbeddingMailboxRepoCandidate>,
) -> Result<Vec<EmbeddingMailboxRepoCandidate>> {
    if values.is_empty() {
        return Ok(values);
    }
    let has_summary_candidates = values
        .iter()
        .any(|candidate| candidate.representation_kind == EmbeddingRepresentationKind::Summary);
    let has_summary_overlap_candidates = values.iter().any(|candidate| {
        candidate.representation_kind == EmbeddingRepresentationKind::Summary
            && repo_has_active_summary_refresh_work(conn, &candidate.repo_id).unwrap_or(false)
    });
    let has_non_summary_candidates = values
        .iter()
        .any(|candidate| candidate.representation_kind != EmbeddingRepresentationKind::Summary);
    let can_prioritize_summary = if has_summary_candidates && has_non_summary_candidates {
        let fallback_config_root = values
            .first()
            .map(|candidate| candidate.config_root.as_path())
            .expect("checked non-empty candidates");
        let summary_priority_worker_limit = summary_embedding_priority_worker_limit(
            workplane_store,
            fallback_config_root,
            has_summary_overlap_candidates,
        )?;
        let leased_summary_batches =
            leased_embedding_mailbox_batch_count(conn, EmbeddingRepresentationKind::Summary)?;
        leased_summary_batches < summary_priority_worker_limit
    } else {
        false
    };
    let preferred_non_summary_kind = preferred_non_summary_representation_kind(conn, &values)?;

    let mut prioritized_summary = Vec::new();
    let mut preferred_non_summary = Vec::new();
    let mut fallback = Vec::new();
    let mut deferred_summary = Vec::new();
    for candidate in values {
        let is_summary_candidate =
            candidate.representation_kind == EmbeddingRepresentationKind::Summary;
        if is_summary_candidate {
            if can_prioritize_summary {
                prioritized_summary.push(candidate);
            } else {
                deferred_summary.push(candidate);
            }
        } else if preferred_non_summary_kind
            .is_some_and(|kind| candidate.representation_kind == kind)
        {
            preferred_non_summary.push(candidate);
        } else {
            fallback.push(candidate);
        }
    }

    prioritized_summary.extend(preferred_non_summary);
    prioritized_summary.extend(fallback);
    prioritized_summary.extend(deferred_summary);
    Ok(prioritized_summary)
}

fn summary_embedding_priority_worker_limit(
    workplane_store: &DaemonSqliteRuntimeStore,
    fallback_config_root: &std::path::Path,
    has_active_summary_overlap: bool,
) -> Result<usize> {
    let embeddings = effective_worker_budgets(workplane_store, fallback_config_root)?.embeddings;
    if embeddings <= 1 {
        return Ok(0);
    }
    if has_active_summary_overlap {
        return Ok(embeddings / 2);
    }
    Ok(1)
}

fn preferred_non_summary_representation_kind(
    conn: &rusqlite::Connection,
    values: &[EmbeddingMailboxRepoCandidate],
) -> Result<Option<EmbeddingRepresentationKind>> {
    let has_code = values
        .iter()
        .any(|candidate| candidate.representation_kind == EmbeddingRepresentationKind::Code);
    let has_identity = values
        .iter()
        .any(|candidate| candidate.representation_kind == EmbeddingRepresentationKind::Identity);
    if !has_code || !has_identity {
        return Ok(None);
    }

    let leased_code_batches =
        leased_embedding_mailbox_batch_count(conn, EmbeddingRepresentationKind::Code)?;
    let leased_identity_batches =
        leased_embedding_mailbox_batch_count(conn, EmbeddingRepresentationKind::Identity)?;
    Ok(match leased_code_batches.cmp(&leased_identity_batches) {
        std::cmp::Ordering::Greater => Some(EmbeddingRepresentationKind::Identity),
        std::cmp::Ordering::Less => Some(EmbeddingRepresentationKind::Code),
        std::cmp::Ordering::Equal => None,
    })
}

fn leased_embedding_mailbox_batch_count(
    conn: &rusqlite::Connection,
    representation_kind: EmbeddingRepresentationKind,
) -> Result<usize> {
    let count = conn.query_row(
        "SELECT COUNT(DISTINCT COALESCE(lease_token, item_id))
         FROM semantic_embedding_mailbox_items
         WHERE representation_kind = ?1
           AND status = ?2",
        params![
            representation_kind.to_string(),
            SemanticMailboxItemStatus::Leased.as_str(),
        ],
        |row| row.get::<_, i64>(0),
    )?;
    Ok(usize::try_from(count).unwrap_or_default())
}

fn repo_has_active_summary_refresh_work(
    conn: &rusqlite::Connection,
    repo_id: &str,
) -> Result<bool> {
    Ok(summary_mailbox_work_is_active(conn, repo_id)?
        || summary_workplane_jobs_are_active(conn, repo_id)?)
}

fn summary_mailbox_work_is_active(conn: &rusqlite::Connection, repo_id: &str) -> Result<bool> {
    Ok(conn
        .query_row(
            "SELECT 1
             FROM semantic_summary_mailbox_items
             WHERE repo_id = ?1
               AND status IN (?2, ?3)
             LIMIT 1",
            params![
                repo_id,
                SemanticMailboxItemStatus::Pending.as_str(),
                SemanticMailboxItemStatus::Leased.as_str(),
            ],
            |_| Ok(()),
        )
        .optional()?
        .is_some())
}

fn summary_workplane_jobs_are_active(conn: &rusqlite::Connection, repo_id: &str) -> Result<bool> {
    Ok(conn
        .query_row(
            "SELECT 1
             FROM capability_workplane_jobs
             WHERE repo_id = ?1
               AND mailbox_name = ?2
               AND status IN (?3, ?4)
             LIMIT 1",
            params![
                repo_id,
                SEMANTIC_CLONES_SUMMARY_REFRESH_MAILBOX,
                crate::host::runtime_store::WorkplaneJobStatus::Pending.as_str(),
                crate::host::runtime_store::WorkplaneJobStatus::Running.as_str(),
            ],
            |_| Ok(()),
        )
        .optional()?
        .is_some())
}

fn lease_summary_mailbox_batch_for_repo(
    conn: &rusqlite::Connection,
    repo_id: &str,
    now: u64,
) -> Result<Option<ClaimedSummaryMailboxBatch>> {
    let Some((repo_root, config_root, oldest_kind)) = conn
        .query_row(
            "SELECT repo_root, config_root, item_kind
             FROM semantic_summary_mailbox_items
             WHERE repo_id = ?1
               AND status = ?2
               AND available_at_unix <= ?3
             ORDER BY submitted_at_unix ASC, item_id ASC
             LIMIT 1",
            params![
                repo_id,
                SemanticMailboxItemStatus::Pending.as_str(),
                sql_i64(now)?,
            ],
            |row| {
                Ok((
                    PathBuf::from(row.get::<_, String>(0)?),
                    PathBuf::from(row.get::<_, String>(1)?),
                    SemanticMailboxItemKind::parse(&row.get::<_, String>(2)?),
                ))
            },
        )
        .optional()?
    else {
        return Ok(None);
    };

    let lease_token = format!("semantic-summary-lease-{}", Uuid::new_v4());
    let selected_ids = load_selected_summary_item_ids(conn, repo_id, oldest_kind, now)?;
    if selected_ids.is_empty() {
        return Ok(None);
    }

    conn.execute(
        &format!(
            "UPDATE semantic_summary_mailbox_items
             SET status = '{leased}',
                 attempts = attempts + 1,
                 leased_at_unix = {leased_at},
                 lease_expires_at_unix = {lease_expires_at},
                 lease_token = '{lease_token}',
                 updated_at_unix = {updated_at}
             WHERE status = '{pending}'
               AND item_id IN ({item_ids})",
            leased = SemanticMailboxItemStatus::Leased.as_str(),
            leased_at = sql_i64(now)?,
            lease_expires_at = sql_i64(now.saturating_add(SEMANTIC_MAILBOX_LEASE_SECS))?,
            lease_token = esc_pg(&lease_token),
            updated_at = sql_i64(now)?,
            pending = SemanticMailboxItemStatus::Pending.as_str(),
            item_ids = sql_string_list(&selected_ids),
        ),
        [],
    )
    .context("leasing semantic summary mailbox items")?;
    let items = load_summary_mailbox_items_by_ids(conn, &selected_ids)?;
    Ok(Some(ClaimedSummaryMailboxBatch {
        repo_id: repo_id.to_string(),
        repo_root,
        config_root,
        lease_token,
        items,
    }))
}

fn lease_embedding_mailbox_batch_for_repo(
    conn: &rusqlite::Connection,
    repo_id: &str,
    representation_kind: EmbeddingRepresentationKind,
    now: u64,
) -> Result<Option<ClaimedEmbeddingMailboxBatch>> {
    let Some((repo_root, config_root, oldest_kind)) = conn
        .query_row(
            "SELECT repo_root, config_root, item_kind
             FROM semantic_embedding_mailbox_items
             WHERE repo_id = ?1
               AND representation_kind = ?2
               AND status = ?3
               AND available_at_unix <= ?4
             ORDER BY submitted_at_unix ASC, item_id ASC
             LIMIT 1",
            params![
                repo_id,
                representation_kind.to_string(),
                SemanticMailboxItemStatus::Pending.as_str(),
                sql_i64(now)?,
            ],
            |row| {
                Ok((
                    PathBuf::from(row.get::<_, String>(0)?),
                    PathBuf::from(row.get::<_, String>(1)?),
                    SemanticMailboxItemKind::parse(&row.get::<_, String>(2)?),
                ))
            },
        )
        .optional()?
    else {
        return Ok(None);
    };

    let lease_token = format!("semantic-embedding-lease-{}", Uuid::new_v4());
    let selected_ids =
        load_selected_embedding_item_ids(conn, repo_id, representation_kind, oldest_kind, now)?;
    if selected_ids.is_empty() {
        return Ok(None);
    }

    conn.execute(
        &format!(
            "UPDATE semantic_embedding_mailbox_items
             SET status = '{leased}',
                 attempts = attempts + 1,
                 leased_at_unix = {leased_at},
                 lease_expires_at_unix = {lease_expires_at},
                 lease_token = '{lease_token}',
                 updated_at_unix = {updated_at}
             WHERE status = '{pending}'
               AND item_id IN ({item_ids})",
            leased = SemanticMailboxItemStatus::Leased.as_str(),
            leased_at = sql_i64(now)?,
            lease_expires_at = sql_i64(now.saturating_add(SEMANTIC_MAILBOX_LEASE_SECS))?,
            lease_token = esc_pg(&lease_token),
            updated_at = sql_i64(now)?,
            pending = SemanticMailboxItemStatus::Pending.as_str(),
            item_ids = sql_string_list(&selected_ids),
        ),
        [],
    )
    .context("leasing semantic embedding mailbox items")?;
    let items = load_embedding_mailbox_items_by_ids(conn, &selected_ids)?;
    Ok(Some(ClaimedEmbeddingMailboxBatch {
        repo_id: repo_id.to_string(),
        repo_root,
        config_root,
        representation_kind,
        lease_token,
        items,
    }))
}

fn load_selected_summary_item_ids(
    conn: &rusqlite::Connection,
    repo_id: &str,
    item_kind: SemanticMailboxItemKind,
    now: u64,
) -> Result<Vec<String>> {
    let limit = if item_kind == SemanticMailboxItemKind::RepoBackfill {
        1
    } else {
        SEMANTIC_SUMMARY_MAILBOX_BATCH_SIZE
    };
    let mut stmt = conn.prepare(
        "SELECT item_id
         FROM semantic_summary_mailbox_items
         WHERE repo_id = ?1
           AND item_kind = ?2
           AND status = ?3
           AND available_at_unix <= ?4
         ORDER BY submitted_at_unix ASC, item_id ASC
         LIMIT ?5",
    )?;
    let rows = stmt.query_map(
        params![
            repo_id,
            item_kind.as_str(),
            SemanticMailboxItemStatus::Pending.as_str(),
            sql_i64(now)?,
            i64::try_from(limit)?,
        ],
        |row| row.get::<_, String>(0),
    )?;
    let mut values = Vec::new();
    for row in rows {
        values.push(row?);
    }
    Ok(values)
}

fn load_selected_embedding_item_ids(
    conn: &rusqlite::Connection,
    repo_id: &str,
    representation_kind: EmbeddingRepresentationKind,
    item_kind: SemanticMailboxItemKind,
    now: u64,
) -> Result<Vec<String>> {
    let limit = if item_kind == SemanticMailboxItemKind::RepoBackfill {
        1
    } else {
        SEMANTIC_EMBEDDING_MAILBOX_BATCH_SIZE
    };
    let mut stmt = conn.prepare(
        "SELECT item_id
         FROM semantic_embedding_mailbox_items
         WHERE repo_id = ?1
           AND representation_kind = ?2
           AND item_kind = ?3
           AND status = ?4
           AND available_at_unix <= ?5
         ORDER BY submitted_at_unix ASC, item_id ASC
         LIMIT ?6",
    )?;
    let rows = stmt.query_map(
        params![
            repo_id,
            representation_kind.to_string(),
            item_kind.as_str(),
            SemanticMailboxItemStatus::Pending.as_str(),
            sql_i64(now)?,
            i64::try_from(limit)?,
        ],
        |row| row.get::<_, String>(0),
    )?;
    let mut values = Vec::new();
    for row in rows {
        values.push(row?);
    }
    Ok(values)
}
