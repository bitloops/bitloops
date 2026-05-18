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

use super::super::EnrichmentControlState;
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

pub(crate) fn claim_summary_mailbox_batch(
    workplane_store: &DaemonSqliteRuntimeStore,
    runtime_store: &DaemonSqliteRuntimeStore,
    control_state: &EnrichmentControlState,
) -> Result<Option<ClaimedSummaryMailboxBatch>> {
    if control_state.paused_semantic {
        return Ok(None);
    }
    workplane_store.with_write_connection(|conn| {
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
    workplane_store.with_write_connection(|conn| {
        let candidates = load_embedding_mailbox_repo_candidates(conn, unix_timestamp_now())?;
        let mut readiness_cache = BTreeMap::new();
        for (repo_id, repo_root, config_root, representation_kind) in candidates {
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
) -> Result<Vec<(String, PathBuf, PathBuf, EmbeddingRepresentationKind)>> {
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
            Ok((
                row.get::<_, String>(0)?,
                PathBuf::from(row.get::<_, String>(1)?),
                PathBuf::from(row.get::<_, String>(2)?),
                representation_kind,
            ))
        },
    )?;
    let mut values = Vec::new();
    for row in rows {
        values.push(row?);
    }
    Ok(values)
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
