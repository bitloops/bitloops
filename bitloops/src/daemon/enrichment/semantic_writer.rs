use anyhow::{Context, Result, anyhow};
use rusqlite::{Connection, OpenFlags, OptionalExtension, Transaction};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time::Duration;
use tokio::sync::oneshot;

use crate::host::devql::esc_pg;
use crate::host::runtime_store::{
    CapabilityWorkplaneJobInsert, SemanticEmbeddingMailboxItemInsert,
    SemanticSummaryMailboxItemInsert,
};

#[derive(Debug, Clone)]
pub(crate) struct SemanticBatchRepoContext {
    pub repo_id: String,
    pub repo_root: PathBuf,
    pub config_root: PathBuf,
}

#[derive(Debug, Clone)]
pub(crate) struct CommitSummaryBatchRequest {
    pub repo: SemanticBatchRepoContext,
    pub lease_token: String,
    pub semantic_statements: Vec<String>,
    pub embedding_follow_ups: Vec<SemanticEmbeddingMailboxItemInsert>,
    pub replacement_backfill_item: Option<SemanticSummaryMailboxItemInsert>,
    pub acked_item_ids: Vec<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct CommitEmbeddingBatchRequest {
    pub repo: SemanticBatchRepoContext,
    pub lease_token: String,
    pub embedding_statements: Vec<String>,
    pub setup_statements: Vec<String>,
    pub clone_rebuild_signal: Option<CapabilityWorkplaneJobInsert>,
    pub replacement_backfill_item: Option<SemanticEmbeddingMailboxItemInsert>,
    pub acked_item_ids: Vec<String>,
}

#[derive(Debug)]
enum SemanticWriterRequest {
    Summary {
        request: CommitSummaryBatchRequest,
        response: oneshot::Sender<std::result::Result<(), String>>,
    },
    Embedding {
        request: CommitEmbeddingBatchRequest,
        response: oneshot::Sender<std::result::Result<(), String>>,
    },
}

#[derive(Debug)]
struct RepoSemanticWriterActor {
    sender: Sender<SemanticWriterRequest>,
}

impl RepoSemanticWriterActor {
    fn shared(
        runtime_db_path: &Path,
        relational_db_path: &Path,
        repo_id: &str,
    ) -> Result<Arc<Self>> {
        static REGISTRY: OnceLock<Mutex<HashMap<String, Arc<RepoSemanticWriterActor>>>> =
            OnceLock::new();
        let registry = REGISTRY.get_or_init(|| Mutex::new(HashMap::new()));
        let key = format!(
            "{}::{}::{repo_id}",
            runtime_db_path.display(),
            relational_db_path.display()
        );
        let mut registry = registry
            .lock()
            .map_err(|_| anyhow!("locking semantic writer actor registry"))?;
        if let Some(actor) = registry.get(&key) {
            return Ok(Arc::clone(actor));
        }

        let actor = Arc::new(Self::spawn(
            runtime_db_path.to_path_buf(),
            relational_db_path.to_path_buf(),
            repo_id.to_string(),
        )?);
        registry.insert(key, Arc::clone(&actor));
        Ok(actor)
    }

    fn spawn(
        runtime_db_path: PathBuf,
        relational_db_path: PathBuf,
        repo_id: String,
    ) -> Result<Self> {
        let (sender, receiver) = mpsc::channel::<SemanticWriterRequest>();
        let thread_name = format!("bitloops-semantic-writer-{repo_id}");
        thread::Builder::new()
            .name(thread_name)
            .spawn(move || writer_loop(runtime_db_path, relational_db_path, receiver))
            .context("spawning semantic writer actor thread")?;
        Ok(Self { sender })
    }

    async fn commit_summary(&self, request: CommitSummaryBatchRequest) -> Result<()> {
        let (response_tx, response_rx) = oneshot::channel();
        self.sender
            .send(SemanticWriterRequest::Summary {
                request,
                response: response_tx,
            })
            .map_err(|_| anyhow!("sending summary batch to semantic writer"))?;
        match response_rx.await {
            Ok(Ok(())) => Ok(()),
            Ok(Err(err)) => Err(anyhow!(err)),
            Err(_) => Err(anyhow!(
                "semantic writer dropped the summary response channel"
            )),
        }
    }

    async fn commit_embedding(&self, request: CommitEmbeddingBatchRequest) -> Result<()> {
        let (response_tx, response_rx) = oneshot::channel();
        self.sender
            .send(SemanticWriterRequest::Embedding {
                request,
                response: response_tx,
            })
            .map_err(|_| anyhow!("sending embedding batch to semantic writer"))?;
        match response_rx.await {
            Ok(Ok(())) => Ok(()),
            Ok(Err(err)) => Err(anyhow!(err)),
            Err(_) => Err(anyhow!(
                "semantic writer dropped the embedding response channel"
            )),
        }
    }
}

pub(crate) async fn commit_summary_batch(
    runtime_db_path: &Path,
    relational_db_path: &Path,
    request: CommitSummaryBatchRequest,
) -> Result<()> {
    RepoSemanticWriterActor::shared(runtime_db_path, relational_db_path, &request.repo.repo_id)?
        .commit_summary(request)
        .await
}

pub(crate) async fn commit_embedding_batch(
    runtime_db_path: &Path,
    relational_db_path: &Path,
    request: CommitEmbeddingBatchRequest,
) -> Result<()> {
    RepoSemanticWriterActor::shared(runtime_db_path, relational_db_path, &request.repo.repo_id)?
        .commit_embedding(request)
        .await
}

fn writer_loop(
    runtime_db_path: PathBuf,
    relational_db_path: PathBuf,
    receiver: Receiver<SemanticWriterRequest>,
) {
    let mut connection = open_semantic_writer_connection(&runtime_db_path, &relational_db_path)
        .map_err(|err| format!("{err:#}"))
        .ok();
    while let Ok(request) = receiver.recv() {
        match (&mut connection, request) {
            (Some(connection), SemanticWriterRequest::Summary { request, response }) => {
                let result = execute_summary_commit(connection, &request).map_err(|err| {
                    format!(
                        "committing semantic summary batch for repo `{}`: {err:#}",
                        request.repo.repo_id
                    )
                });
                let _ = response.send(result);
            }
            (Some(connection), SemanticWriterRequest::Embedding { request, response }) => {
                let result = execute_embedding_commit(connection, &request).map_err(|err| {
                    format!(
                        "committing semantic embedding batch for repo `{}`: {err:#}",
                        request.repo.repo_id
                    )
                });
                let _ = response.send(result);
            }
            (None, SemanticWriterRequest::Summary { response, .. }) => {
                let _ = response.send(Err("opening semantic writer connection failed".to_string()));
            }
            (None, SemanticWriterRequest::Embedding { response, .. }) => {
                let _ = response.send(Err("opening semantic writer connection failed".to_string()));
            }
        }
    }
}

fn execute_summary_commit(
    connection: &mut Connection,
    request: &CommitSummaryBatchRequest,
) -> Result<()> {
    let tx = connection
        .transaction()
        .context("starting semantic summary batch transaction")?;
    for statement in &request.semantic_statements {
        if statement.trim().is_empty() {
            continue;
        }
        tx.execute_batch(statement)
            .context("executing semantic summary SQL")?;
    }
    for item in &request.embedding_follow_ups {
        upsert_runtime_embedding_mailbox_item(&tx, &request.repo, item)?;
    }
    if let Some(item) = request.replacement_backfill_item.as_ref() {
        insert_runtime_summary_mailbox_item(&tx, &request.repo, item)?;
    }
    delete_runtime_summary_mailbox_items(&tx, &request.lease_token, &request.acked_item_ids)?;
    tx.commit()
        .context("committing semantic summary batch transaction")?;
    Ok(())
}

fn execute_embedding_commit(
    connection: &mut Connection,
    request: &CommitEmbeddingBatchRequest,
) -> Result<()> {
    let tx = connection
        .transaction()
        .context("starting semantic embedding batch transaction")?;
    for statement in request
        .embedding_statements
        .iter()
        .chain(request.setup_statements.iter())
    {
        if statement.trim().is_empty() {
            continue;
        }
        tx.execute_batch(statement)
            .context("executing semantic embedding SQL")?;
    }
    if let Some(signal) = request.clone_rebuild_signal.as_ref() {
        upsert_runtime_clone_rebuild_signal(&tx, &request.repo, signal)?;
    }
    if let Some(item) = request.replacement_backfill_item.as_ref() {
        insert_runtime_embedding_mailbox_item(&tx, &request.repo, item)?;
    }
    delete_runtime_embedding_mailbox_items(&tx, &request.lease_token, &request.acked_item_ids)?;
    tx.commit()
        .context("committing semantic embedding batch transaction")?;
    Ok(())
}

fn open_semantic_writer_connection(
    runtime_db_path: &Path,
    relational_db_path: &Path,
) -> Result<Connection> {
    if !runtime_db_path.is_file() {
        anyhow::bail!(
            "runtime SQLite database not found at {}",
            runtime_db_path.display()
        );
    }
    if !relational_db_path.is_file() {
        anyhow::bail!(
            "relational SQLite database not found at {}",
            relational_db_path.display()
        );
    }
    let conn = Connection::open_with_flags(relational_db_path, OpenFlags::SQLITE_OPEN_READ_WRITE)
        .with_context(|| {
        format!(
            "opening SQLite database at {}",
            relational_db_path.display()
        )
    })?;
    conn.busy_timeout(Duration::from_secs(30))
        .context("setting semantic writer busy timeout")?;
    conn.execute_batch(
        "PRAGMA foreign_keys = ON; PRAGMA journal_mode = WAL; PRAGMA synchronous = NORMAL;",
    )
    .context("configuring semantic writer connection")?;
    conn.execute(
        "ATTACH DATABASE ?1 AS runtime_store",
        [runtime_db_path.to_string_lossy().to_string()],
    )
    .context("attaching runtime SQLite database to semantic writer connection")?;
    Ok(conn)
}

fn upsert_runtime_embedding_mailbox_item(
    tx: &Transaction<'_>,
    repo: &SemanticBatchRepoContext,
    item: &SemanticEmbeddingMailboxItemInsert,
) -> Result<()> {
    let existing = tx
        .query_row(
            "SELECT item_id, status
             FROM runtime_store.semantic_embedding_mailbox_items
             WHERE repo_id = ?1
               AND representation_kind = ?2
               AND dedupe_key = ?3
               AND status IN ('pending', 'leased', 'failed')
             ORDER BY CASE status
                          WHEN 'leased' THEN 0
                          WHEN 'pending' THEN 1
                          ELSE 2
                      END,
                      submitted_at_unix ASC
             LIMIT 1",
            rusqlite::params![
                repo.repo_id,
                item.representation_kind,
                item.dedupe_key.as_deref()
            ],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()?;
    let now = unix_timestamp_now();
    if let Some((item_id, status)) = existing {
        if status == "pending" {
            tx.execute(
                "UPDATE runtime_store.semantic_embedding_mailbox_items
                 SET init_session_id = COALESCE(init_session_id, ?1),
                     payload_json = ?2,
                     available_at_unix = ?3,
                     updated_at_unix = ?4,
                     last_error = NULL
                 WHERE item_id = ?5",
                rusqlite::params![
                    item.init_session_id.as_deref(),
                    item.payload_json.as_ref().map(serde_json::Value::to_string),
                    sql_i64(now)?,
                    sql_i64(now)?,
                    item_id,
                ],
            )
            .context("refreshing pending runtime embedding mailbox item")?;
        }
        return Ok(());
    }

    insert_runtime_embedding_mailbox_item(tx, repo, item)
}

fn insert_runtime_embedding_mailbox_item(
    tx: &Transaction<'_>,
    repo: &SemanticBatchRepoContext,
    item: &SemanticEmbeddingMailboxItemInsert,
) -> Result<()> {
    let now = unix_timestamp_now();
    let item_id = format!("semantic-embedding-mailbox-item-{}", uuid::Uuid::new_v4());
    tx.execute(
        "INSERT INTO runtime_store.semantic_embedding_mailbox_items (
            item_id, repo_id, repo_root, config_root, init_session_id, representation_kind,
            item_kind, artefact_id, payload_json, dedupe_key, status, attempts,
            available_at_unix, submitted_at_unix, leased_at_unix, lease_expires_at_unix,
            lease_token, updated_at_unix, last_error
         ) VALUES (
            ?1, ?2, ?3, ?4, ?5, ?6,
            ?7, ?8, ?9, ?10, 'pending', 0,
            ?11, ?12, NULL, NULL,
            NULL, ?13, NULL
         )",
        rusqlite::params![
            item_id,
            repo.repo_id,
            repo.repo_root.to_string_lossy().to_string(),
            repo.config_root.to_string_lossy().to_string(),
            item.init_session_id.as_deref(),
            item.representation_kind.as_str(),
            item.item_kind.as_str(),
            item.artefact_id.as_deref(),
            item.payload_json.as_ref().map(serde_json::Value::to_string),
            item.dedupe_key.as_deref(),
            sql_i64(now)?,
            sql_i64(now)?,
            sql_i64(now)?,
        ],
    )
    .context("inserting runtime embedding mailbox item")?;
    Ok(())
}

fn insert_runtime_summary_mailbox_item(
    tx: &Transaction<'_>,
    repo: &SemanticBatchRepoContext,
    item: &SemanticSummaryMailboxItemInsert,
) -> Result<()> {
    let now = unix_timestamp_now();
    let item_id = format!("semantic-summary-mailbox-item-{}", uuid::Uuid::new_v4());
    tx.execute(
        "INSERT INTO runtime_store.semantic_summary_mailbox_items (
            item_id, repo_id, repo_root, config_root, init_session_id, item_kind,
            artefact_id, payload_json, dedupe_key, status, attempts, available_at_unix,
            submitted_at_unix, leased_at_unix, lease_expires_at_unix, lease_token,
            updated_at_unix, last_error
         ) VALUES (
            ?1, ?2, ?3, ?4, ?5, ?6,
            ?7, ?8, ?9, 'pending', 0, ?10,
            ?11, NULL, NULL, NULL,
            ?12, NULL
         )",
        rusqlite::params![
            item_id,
            repo.repo_id,
            repo.repo_root.to_string_lossy().to_string(),
            repo.config_root.to_string_lossy().to_string(),
            item.init_session_id.as_deref(),
            item.item_kind.as_str(),
            item.artefact_id.as_deref(),
            item.payload_json.as_ref().map(serde_json::Value::to_string),
            item.dedupe_key.as_deref(),
            sql_i64(now)?,
            sql_i64(now)?,
            sql_i64(now)?,
        ],
    )
    .context("inserting runtime summary mailbox item")?;
    Ok(())
}

fn delete_runtime_summary_mailbox_items(
    tx: &Transaction<'_>,
    lease_token: &str,
    item_ids: &[String],
) -> Result<()> {
    if item_ids.is_empty() {
        return Ok(());
    }
    tx.execute_batch(&format!(
        "DELETE FROM runtime_store.semantic_summary_mailbox_items
         WHERE lease_token = '{lease_token}'
           AND item_id IN ({item_ids})",
        lease_token = esc_pg(lease_token),
        item_ids = sql_string_list(item_ids),
    ))
    .context("deleting acknowledged semantic summary mailbox items")?;
    Ok(())
}

fn delete_runtime_embedding_mailbox_items(
    tx: &Transaction<'_>,
    lease_token: &str,
    item_ids: &[String],
) -> Result<()> {
    if item_ids.is_empty() {
        return Ok(());
    }
    tx.execute_batch(&format!(
        "DELETE FROM runtime_store.semantic_embedding_mailbox_items
         WHERE lease_token = '{lease_token}'
           AND item_id IN ({item_ids})",
        lease_token = esc_pg(lease_token),
        item_ids = sql_string_list(item_ids),
    ))
    .context("deleting acknowledged semantic embedding mailbox items")?;
    Ok(())
}

fn upsert_runtime_clone_rebuild_signal(
    tx: &Transaction<'_>,
    repo: &SemanticBatchRepoContext,
    signal: &CapabilityWorkplaneJobInsert,
) -> Result<()> {
    let existing = tx
        .query_row(
            "SELECT job_id, status
             FROM runtime_store.capability_workplane_jobs
             WHERE repo_id = ?1
               AND capability_id = ?2
               AND mailbox_name = ?3
               AND dedupe_key = ?4
               AND status IN ('pending', 'running')
             ORDER BY CASE status WHEN 'running' THEN 0 ELSE 1 END, submitted_at_unix ASC
             LIMIT 1",
            rusqlite::params![
                repo.repo_id,
                crate::capability_packs::semantic_clones::SEMANTIC_CLONES_CAPABILITY_ID,
                signal.mailbox_name.as_str(),
                signal.dedupe_key.as_deref(),
            ],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()?;
    let now = unix_timestamp_now();
    if let Some((job_id, status)) = existing {
        if status == "pending" {
            tx.execute(
                "UPDATE runtime_store.capability_workplane_jobs
                 SET payload = ?1, updated_at_unix = ?2, available_at_unix = ?3, last_error = NULL
                 WHERE job_id = ?4",
                rusqlite::params![
                    signal.payload.to_string(),
                    sql_i64(now)?,
                    sql_i64(now)?,
                    job_id,
                ],
            )
            .context("refreshing pending clone rebuild signal")?;
        }
        return Ok(());
    }

    let job_id = format!("workplane-job-{}", uuid::Uuid::new_v4());
    tx.execute(
        "INSERT INTO runtime_store.capability_workplane_jobs (
            job_id, repo_id, repo_root, config_root, capability_id, mailbox_name,
            init_session_id, dedupe_key, payload, status, attempts, available_at_unix,
            submitted_at_unix, started_at_unix, updated_at_unix, completed_at_unix,
            lease_owner, lease_expires_at_unix, last_error
         ) VALUES (
            ?1, ?2, ?3, ?4, ?5, ?6,
            NULL, ?7, ?8, 'pending', 0, ?9,
            ?10, NULL, ?11, NULL,
            NULL, NULL, NULL
         )",
        rusqlite::params![
            job_id,
            repo.repo_id,
            repo.repo_root.to_string_lossy().to_string(),
            repo.config_root.to_string_lossy().to_string(),
            crate::capability_packs::semantic_clones::SEMANTIC_CLONES_CAPABILITY_ID,
            signal.mailbox_name.as_str(),
            signal.dedupe_key.as_deref(),
            signal.payload.to_string(),
            sql_i64(now)?,
            sql_i64(now)?,
            sql_i64(now)?,
        ],
    )
    .context("inserting clone rebuild signal")?;
    Ok(())
}

fn sql_string_list(values: &[String]) -> String {
    values
        .iter()
        .map(|value| format!("'{}'", esc_pg(value)))
        .collect::<Vec<_>>()
        .join(", ")
}

fn sql_i64(value: u64) -> Result<i64> {
    i64::try_from(value).context("converting semantic writer integer to SQLite i64")
}

fn unix_timestamp_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}
