use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use rusqlite::OptionalExtension;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::config::resolve_store_backend_config_for_repo;
use crate::daemon::{
    DaemonRuntimeState, DaemonServiceMetadata, PersistedEnrichmentQueueState,
    SupervisorRuntimeState, SupervisorServiceMetadata, SyncTaskRecord,
};
use crate::daemon::{runtime_state_path, service_metadata_path};
use crate::host::checkpoints::session::DbSessionBackend;
use crate::host::checkpoints::transcript::metadata::{
    SessionMetadataBundle, build_context_markdown, build_session_metadata_bundle,
    extract_prompts_from_transcript_bytes, extract_summary_from_transcript_bytes,
};
use crate::host::interactions::db_store::{
    SqliteInteractionSpool, legacy_interaction_spool_db_path,
};
use crate::storage::SqliteConnectionPool;
use crate::utils::paths;
use crate::utils::paths::{
    LEGACY_BITLOOPS_METADATA_DIR, TRANSCRIPT_FILE_NAME, default_global_runtime_db_path,
    default_repo_runtime_db_path,
};

const RUNTIME_DOCUMENTS_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS runtime_documents (
    document_kind TEXT PRIMARY KEY,
    payload TEXT NOT NULL,
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);
"#;

pub trait RuntimeStore: Send + Sync {
    type RepoStore;
    type DaemonStore;

    fn repo_store(&self, repo_root: &Path) -> Result<Self::RepoStore>;
    fn daemon_store(&self) -> Result<Self::DaemonStore>;
}

#[derive(Debug, Clone, Default)]
pub struct SqliteRuntimeStore;

#[derive(Debug, Clone)]
pub struct RepoSqliteRuntimeStore {
    repo_root: PathBuf,
    repo_id: String,
    db_path: PathBuf,
}

#[derive(Debug, Clone)]
pub struct DaemonSqliteRuntimeStore {
    db_path: PathBuf,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PersistedSyncQueueState {
    pub version: u8,
    pub tasks: Vec<SyncTaskRecord>,
    pub last_action: Option<String>,
    pub updated_at_unix: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RuntimeMetadataBlobType {
    Transcript,
    Prompts,
    Summary,
    Context,
    TaskCheckpoint,
    SubagentTranscript,
    IncrementalCheckpoint,
    Prompt,
}

impl RuntimeMetadataBlobType {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Transcript => "transcript",
            Self::Prompts => "prompts",
            Self::Summary => "summary",
            Self::Context => "context",
            Self::TaskCheckpoint => "task_checkpoint",
            Self::SubagentTranscript => "subagent_transcript",
            Self::IncrementalCheckpoint => "incremental_checkpoint",
            Self::Prompt => "prompt",
        }
    }

    pub const fn default_file_name(self) -> &'static str {
        match self {
            Self::Transcript => "full.jsonl",
            Self::Prompts => "prompt.txt",
            Self::Summary => "summary.txt",
            Self::Context => "context.md",
            Self::TaskCheckpoint => "checkpoint.json",
            Self::SubagentTranscript => "agent.jsonl",
            Self::IncrementalCheckpoint => "incremental-checkpoint.json",
            Self::Prompt => "prompt.txt",
        }
    }

    fn from_str(value: &str) -> Option<Self> {
        match value {
            "transcript" => Some(Self::Transcript),
            "prompts" => Some(Self::Prompts),
            "summary" => Some(Self::Summary),
            "context" => Some(Self::Context),
            "task_checkpoint" => Some(Self::TaskCheckpoint),
            "subagent_transcript" => Some(Self::SubagentTranscript),
            "incremental_checkpoint" => Some(Self::IncrementalCheckpoint),
            "prompt" => Some(Self::Prompt),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionMetadataSnapshot {
    pub snapshot_id: String,
    pub session_id: String,
    pub turn_id: String,
    pub transcript_identifier: String,
    pub transcript_path: String,
    pub bundle: SessionMetadataBundle,
}

impl SessionMetadataSnapshot {
    pub fn new(session_id: impl Into<String>, bundle: SessionMetadataBundle) -> Self {
        Self {
            snapshot_id: Uuid::new_v4().simple().to_string(),
            session_id: session_id.into(),
            turn_id: String::new(),
            transcript_identifier: String::new(),
            transcript_path: String::new(),
            bundle,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskCheckpointArtefact {
    pub artefact_id: String,
    pub session_id: String,
    pub tool_use_id: String,
    pub agent_id: String,
    pub checkpoint_uuid: String,
    pub kind: RuntimeMetadataBlobType,
    pub incremental_sequence: Option<u32>,
    pub incremental_type: String,
    pub is_incremental: bool,
    pub payload: Vec<u8>,
}

impl TaskCheckpointArtefact {
    pub fn new(
        session_id: impl Into<String>,
        tool_use_id: impl Into<String>,
        kind: RuntimeMetadataBlobType,
        payload: Vec<u8>,
    ) -> Self {
        Self {
            artefact_id: Uuid::new_v4().simple().to_string(),
            session_id: session_id.into(),
            tool_use_id: tool_use_id.into(),
            agent_id: String::new(),
            checkpoint_uuid: String::new(),
            kind,
            incremental_sequence: None,
            incremental_type: String::new(),
            is_incremental: matches!(kind, RuntimeMetadataBlobType::IncrementalCheckpoint),
            payload,
        }
    }
}

impl Default for PersistedSyncQueueState {
    fn default() -> Self {
        Self {
            version: 1,
            tasks: Vec::new(),
            last_action: Some("initialized".to_string()),
            updated_at_unix: 0,
        }
    }
}

impl SqliteRuntimeStore {
    pub fn new() -> Self {
        Self
    }
}

impl RuntimeStore for SqliteRuntimeStore {
    type RepoStore = RepoSqliteRuntimeStore;
    type DaemonStore = DaemonSqliteRuntimeStore;

    fn repo_store(&self, repo_root: &Path) -> Result<Self::RepoStore> {
        RepoSqliteRuntimeStore::open(repo_root)
    }

    fn daemon_store(&self) -> Result<Self::DaemonStore> {
        DaemonSqliteRuntimeStore::open()
    }
}

impl RepoSqliteRuntimeStore {
    pub fn open(repo_root: &Path) -> Result<Self> {
        let repo = crate::host::devql::resolve_repo_identity(repo_root)
            .context("resolving repo identity for runtime store")?;
        let db_path = default_repo_runtime_db_path(repo_root);
        let sqlite = SqliteConnectionPool::connect(db_path.clone())
            .with_context(|| format!("opening repo runtime database {}", db_path.display()))?;
        initialise_repo_runtime_schema(&sqlite)?;
        let store = Self {
            repo_root: repo_root.to_path_buf(),
            repo_id: repo.repo_id,
            db_path,
        };
        store.import_legacy_checkpoint_runtime_if_needed()?;
        store.import_legacy_interaction_spool_if_needed()?;
        store.import_legacy_checkpoint_metadata_if_needed()?;
        Ok(store)
    }

    pub fn db_path(&self) -> &Path {
        &self.db_path
    }

    pub fn repo_id(&self) -> &str {
        &self.repo_id
    }

    pub fn session_backend(&self) -> Result<DbSessionBackend> {
        DbSessionBackend::from_sqlite_path(self.repo_id.clone(), self.db_path.clone())
    }

    pub fn interaction_spool(&self) -> Result<SqliteInteractionSpool> {
        let sqlite = SqliteConnectionPool::connect(self.db_path.clone())
            .with_context(|| format!("opening repo runtime database {}", self.db_path.display()))?;
        SqliteInteractionSpool::new(sqlite, self.repo_id.clone())
    }

    pub fn save_session_metadata_snapshot(&self, snapshot: &SessionMetadataSnapshot) -> Result<()> {
        let sqlite = SqliteConnectionPool::connect(self.db_path.clone())
            .with_context(|| format!("opening repo runtime database {}", self.db_path.display()))?;
        let prompt_text = snapshot.bundle.prompt_text();
        let entries = [
            (
                RuntimeMetadataBlobType::Transcript,
                snapshot.bundle.transcript.clone(),
            ),
            (
                RuntimeMetadataBlobType::Prompts,
                prompt_text.as_bytes().to_vec(),
            ),
            (
                RuntimeMetadataBlobType::Summary,
                snapshot.bundle.summary.as_bytes().to_vec(),
            ),
            (
                RuntimeMetadataBlobType::Context,
                snapshot.bundle.context.clone(),
            ),
        ];

        for (blob_type, payload) in entries {
            if payload.is_empty() {
                continue;
            }
            let (storage_backend, storage_path, content_hash, size_bytes) = self
                .write_runtime_blob(
                    &session_snapshot_blob_key(
                        &self.repo_id,
                        &snapshot.session_id,
                        &snapshot.snapshot_id,
                        blob_type,
                    ),
                    &payload,
                )?;
            sqlite.with_connection(|conn| {
                conn.execute(
                    "INSERT INTO session_metadata_snapshots (
                        snapshot_id, session_id, repo_id, turn_id, transcript_identifier,
                        transcript_path, blob_type, storage_backend, storage_path,
                        content_hash, size_bytes, created_at
                    ) VALUES (
                        ?1, ?2, ?3, ?4, ?5,
                        ?6, ?7, ?8, ?9,
                        ?10, ?11, datetime('now')
                    )
                    ON CONFLICT(repo_id, snapshot_id, blob_type) DO UPDATE SET
                        session_id = excluded.session_id,
                        turn_id = excluded.turn_id,
                        transcript_identifier = excluded.transcript_identifier,
                        transcript_path = excluded.transcript_path,
                        storage_backend = excluded.storage_backend,
                        storage_path = excluded.storage_path,
                        content_hash = excluded.content_hash,
                        size_bytes = excluded.size_bytes",
                    rusqlite::params![
                        snapshot.snapshot_id,
                        snapshot.session_id,
                        self.repo_id.as_str(),
                        snapshot.turn_id,
                        snapshot.transcript_identifier,
                        snapshot.transcript_path,
                        blob_type.as_str(),
                        storage_backend,
                        storage_path,
                        content_hash,
                        size_bytes,
                    ],
                )
                .context("upserting session_metadata_snapshots row")?;
                Ok(())
            })?;
        }

        Ok(())
    }

    pub fn load_latest_session_metadata_snapshot(
        &self,
        session_id: &str,
    ) -> Result<Option<SessionMetadataSnapshot>> {
        let sqlite = SqliteConnectionPool::connect(self.db_path.clone())
            .with_context(|| format!("opening repo runtime database {}", self.db_path.display()))?;
        let header = sqlite.with_connection(|conn| {
            conn.query_row(
                "SELECT snapshot_id,
                        COALESCE(MAX(turn_id), ''),
                        COALESCE(MAX(transcript_identifier), ''),
                        COALESCE(MAX(transcript_path), '')
                 FROM session_metadata_snapshots
                 WHERE repo_id = ?1 AND session_id = ?2
                 GROUP BY snapshot_id
                 ORDER BY MAX(created_at) DESC, snapshot_id DESC
                 LIMIT 1",
                rusqlite::params![self.repo_id.as_str(), session_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                    ))
                },
            )
            .optional()
            .map_err(anyhow::Error::from)
        })?;
        let Some((snapshot_id, turn_id, transcript_identifier, transcript_path)) = header else {
            return Ok(None);
        };

        let blob_store = self.open_repo_blob_store()?;
        let rows = sqlite.with_connection(|conn| {
            let mut stmt = conn.prepare(
                "SELECT blob_type, storage_path
                 FROM session_metadata_snapshots
                 WHERE repo_id = ?1 AND session_id = ?2 AND snapshot_id = ?3",
            )?;
            let mut rows = stmt.query(rusqlite::params![
                self.repo_id.as_str(),
                session_id,
                snapshot_id.as_str()
            ])?;
            let mut values = Vec::new();
            while let Some(row) = rows.next()? {
                values.push((row.get::<_, String>(0)?, row.get::<_, String>(1)?));
            }
            Ok::<_, anyhow::Error>(values)
        })?;

        let mut bundle = SessionMetadataBundle::default();
        for (blob_type, storage_path) in rows {
            let payload = blob_store
                .store
                .read(&storage_path)
                .with_context(|| format!("reading runtime metadata blob `{storage_path}`"))?;
            match RuntimeMetadataBlobType::from_str(&blob_type) {
                Some(RuntimeMetadataBlobType::Transcript) => bundle.transcript = payload,
                Some(RuntimeMetadataBlobType::Prompts) => {
                    bundle.prompts = String::from_utf8_lossy(&payload)
                        .split("\n\n---\n\n")
                        .map(str::trim)
                        .filter(|value| !value.is_empty())
                        .map(str::to_string)
                        .collect();
                }
                Some(RuntimeMetadataBlobType::Summary) => {
                    bundle.summary = String::from_utf8_lossy(&payload).to_string();
                }
                Some(RuntimeMetadataBlobType::Context) => bundle.context = payload,
                _ => {}
            }
        }

        Ok(Some(SessionMetadataSnapshot {
            snapshot_id,
            session_id: session_id.to_string(),
            turn_id,
            transcript_identifier,
            transcript_path,
            bundle,
        }))
    }

    pub fn save_task_checkpoint_artefact(&self, artefact: &TaskCheckpointArtefact) -> Result<()> {
        let sqlite = SqliteConnectionPool::connect(self.db_path.clone())
            .with_context(|| format!("opening repo runtime database {}", self.db_path.display()))?;
        let (storage_backend, storage_path, content_hash, size_bytes) = self.write_runtime_blob(
            &task_artefact_blob_key(&self.repo_id, artefact),
            &artefact.payload,
        )?;

        sqlite.with_connection(|conn| {
            conn.execute(
                "INSERT INTO task_checkpoint_artefacts (
                    artefact_id, session_id, repo_id, tool_use_id, agent_id,
                    checkpoint_uuid, artefact_kind, incremental_sequence, incremental_type,
                    is_incremental, storage_backend, storage_path, content_hash, size_bytes,
                    created_at
                ) VALUES (
                    ?1, ?2, ?3, ?4, ?5,
                    ?6, ?7, ?8, ?9,
                    ?10, ?11, ?12, ?13, ?14,
                    datetime('now')
                )",
                rusqlite::params![
                    artefact.artefact_id,
                    artefact.session_id,
                    self.repo_id.as_str(),
                    artefact.tool_use_id,
                    artefact.agent_id,
                    artefact.checkpoint_uuid,
                    artefact.kind.as_str(),
                    artefact.incremental_sequence.map(i64::from),
                    artefact.incremental_type,
                    if artefact.is_incremental {
                        1_i64
                    } else {
                        0_i64
                    },
                    storage_backend,
                    storage_path,
                    content_hash,
                    size_bytes,
                ],
            )
            .context("inserting task_checkpoint_artefacts row")?;
            Ok(())
        })
    }

    pub fn load_task_checkpoint_artefacts(
        &self,
        session_id: &str,
        tool_use_id: &str,
    ) -> Result<Vec<TaskCheckpointArtefact>> {
        let sqlite = SqliteConnectionPool::connect(self.db_path.clone())
            .with_context(|| format!("opening repo runtime database {}", self.db_path.display()))?;
        let blob_store = self.open_repo_blob_store()?;
        let rows = sqlite.with_connection(|conn| {
            let mut stmt = conn.prepare(
                "SELECT artefact_id, agent_id, checkpoint_uuid, artefact_kind,
                        incremental_sequence, incremental_type, is_incremental, storage_path
                 FROM task_checkpoint_artefacts
                 WHERE repo_id = ?1 AND session_id = ?2 AND tool_use_id = ?3
                 ORDER BY created_at ASC, artefact_id ASC",
            )?;
            let mut rows = stmt.query(rusqlite::params![
                self.repo_id.as_str(),
                session_id,
                tool_use_id
            ])?;
            let mut values = Vec::new();
            while let Some(row) = rows.next()? {
                values.push((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, Option<i64>>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, i64>(6)?,
                    row.get::<_, String>(7)?,
                ));
            }
            Ok::<_, anyhow::Error>(values)
        })?;

        let mut artefacts = Vec::new();
        for (
            artefact_id,
            agent_id,
            checkpoint_uuid,
            kind_raw,
            incremental_sequence,
            incremental_type,
            is_incremental,
            storage_path,
        ) in rows
        {
            let Some(kind) = RuntimeMetadataBlobType::from_str(&kind_raw) else {
                continue;
            };
            let payload = blob_store
                .store
                .read(&storage_path)
                .with_context(|| format!("reading runtime task blob `{storage_path}`"))?;
            artefacts.push(TaskCheckpointArtefact {
                artefact_id,
                session_id: session_id.to_string(),
                tool_use_id: tool_use_id.to_string(),
                agent_id,
                checkpoint_uuid,
                kind,
                incremental_sequence: incremental_sequence
                    .and_then(|value| u32::try_from(value).ok()),
                incremental_type,
                is_incremental: is_incremental != 0,
                payload,
            });
        }

        Ok(artefacts)
    }

    pub fn next_task_incremental_sequence(
        &self,
        session_id: &str,
        tool_use_id: &str,
    ) -> Result<u32> {
        let sqlite = SqliteConnectionPool::connect(self.db_path.clone())
            .with_context(|| format!("opening repo runtime database {}", self.db_path.display()))?;
        let max_sequence = sqlite.with_connection(|conn| {
            conn.query_row(
                "SELECT MAX(incremental_sequence)
                 FROM task_checkpoint_artefacts
                 WHERE repo_id = ?1
                   AND session_id = ?2
                   AND tool_use_id = ?3
                   AND is_incremental = 1
                   AND artefact_kind = ?4",
                rusqlite::params![
                    self.repo_id.as_str(),
                    session_id,
                    tool_use_id,
                    RuntimeMetadataBlobType::IncrementalCheckpoint.as_str(),
                ],
                |row| row.get::<_, Option<i64>>(0),
            )
            .map_err(anyhow::Error::from)
        })?;
        Ok(max_sequence
            .and_then(|value| u32::try_from(value).ok())
            .map(|value| value.saturating_add(1))
            .unwrap_or(1))
    }

    fn open_repo_blob_store(&self) -> Result<crate::storage::blob::ResolvedBlobStore> {
        let cfg = resolve_store_backend_config_for_repo(&self.repo_root)
            .context("resolving backend config for repo runtime metadata")?;
        crate::storage::blob::create_blob_store_with_backend_for_repo(&cfg.blobs, &self.repo_root)
            .context("initialising blob storage for repo runtime metadata")
    }

    fn write_runtime_blob(
        &self,
        key: &str,
        payload: &[u8],
    ) -> Result<(String, String, String, i64)> {
        let resolved = self.open_repo_blob_store()?;
        resolved
            .store
            .write(key, payload)
            .with_context(|| format!("writing runtime metadata blob `{key}`"))?;
        Ok((
            resolved.backend.to_string(),
            key.to_string(),
            format!("sha256:{}", sha256_hex(payload)),
            payload.len() as i64,
        ))
    }

    fn import_legacy_checkpoint_runtime_if_needed(&self) -> Result<()> {
        let sqlite = SqliteConnectionPool::connect(self.db_path.clone())
            .with_context(|| format!("opening repo runtime database {}", self.db_path.display()))?;
        let destination_empty = sqlite.with_connection(|conn| {
            all_tables_empty(
                conn,
                &[
                    "sessions",
                    "temporary_checkpoints",
                    "pre_prompt_states",
                    "pre_task_markers",
                ],
            )
        })?;
        if !destination_empty {
            return Ok(());
        }

        let legacy_path = legacy_relational_sqlite_path(&self.repo_root)?;
        if !legacy_path.is_file() || legacy_path == self.db_path {
            return Ok(());
        }

        sqlite.with_connection(|conn| {
            attach_if_needed(conn, &legacy_path, "legacy_runtime")?;
            let legacy_tables = [
                "sessions",
                "temporary_checkpoints",
                "pre_prompt_states",
                "pre_task_markers",
            ];
            let any_legacy_rows = legacy_tables.iter().try_fold(false, |found, table| {
                if found {
                    return Ok(true);
                }
                table_has_rows_in_attached_db(conn, "legacy_runtime", table)
            })?;
            if !any_legacy_rows {
                detach_if_needed(conn, "legacy_runtime")?;
                return Ok(());
            }

            conn.execute_batch("BEGIN IMMEDIATE TRANSACTION;")
                .context("starting runtime checkpoint import transaction")?;
            let result = (|| {
                execute_copy_if_legacy_table_exists(
                    conn,
                    "legacy_runtime",
                    "sessions",
                    "INSERT OR IGNORE INTO sessions SELECT * FROM legacy_runtime.sessions",
                )?;
                execute_copy_if_legacy_table_exists(
                    conn,
                    "legacy_runtime",
                    "temporary_checkpoints",
                    "INSERT OR IGNORE INTO temporary_checkpoints SELECT * FROM legacy_runtime.temporary_checkpoints",
                )?;
                execute_copy_if_legacy_table_exists(
                    conn,
                    "legacy_runtime",
                    "pre_prompt_states",
                    "INSERT OR IGNORE INTO pre_prompt_states SELECT * FROM legacy_runtime.pre_prompt_states",
                )?;
                execute_copy_if_legacy_table_exists(
                    conn,
                    "legacy_runtime",
                    "pre_task_markers",
                    "INSERT OR IGNORE INTO pre_task_markers SELECT * FROM legacy_runtime.pre_task_markers",
                )?;
                conn.execute_batch("COMMIT;")
                    .context("committing runtime checkpoint import transaction")?;
                Ok(())
            })();
            if result.is_err() {
                let _ = conn.execute_batch("ROLLBACK;");
            }
            detach_if_needed(conn, "legacy_runtime")?;
            result
        })
    }

    fn import_legacy_interaction_spool_if_needed(&self) -> Result<()> {
        let sqlite = SqliteConnectionPool::connect(self.db_path.clone())
            .with_context(|| format!("opening repo runtime database {}", self.db_path.display()))?;
        let destination_empty = sqlite.with_connection(|conn| {
            all_tables_empty(
                conn,
                &[
                    "interaction_sessions",
                    "interaction_turns",
                    "interaction_events",
                    "interaction_spool_queue",
                ],
            )
        })?;
        if !destination_empty {
            return Ok(());
        }

        let legacy_path = legacy_interaction_spool_db_path(&self.repo_root)
            .context("resolving legacy interaction spool path")?;
        if !legacy_path.is_file() || legacy_path == self.db_path {
            return Ok(());
        }

        sqlite.with_connection(|conn| {
            attach_if_needed(conn, &legacy_path, "legacy_spool")?;
            let legacy_tables = [
                "interaction_sessions",
                "interaction_turns",
                "interaction_events",
                "interaction_spool_queue",
            ];
            let any_legacy_rows = legacy_tables.iter().try_fold(false, |found, table| {
                if found {
                    return Ok(true);
                }
                table_has_rows_in_attached_db(conn, "legacy_spool", table)
            })?;
            if !any_legacy_rows {
                detach_if_needed(conn, "legacy_spool")?;
                return Ok(());
            }

            conn.execute_batch("BEGIN IMMEDIATE TRANSACTION;")
                .context("starting interaction spool import transaction")?;
            let result = (|| {
                execute_copy_if_legacy_table_exists(
                    conn,
                    "legacy_spool",
                    "interaction_sessions",
                    "INSERT OR IGNORE INTO interaction_sessions SELECT * FROM legacy_spool.interaction_sessions",
                )?;
                execute_copy_if_legacy_table_exists(
                    conn,
                    "legacy_spool",
                    "interaction_turns",
                    "INSERT OR IGNORE INTO interaction_turns SELECT * FROM legacy_spool.interaction_turns",
                )?;
                execute_copy_if_legacy_table_exists(
                    conn,
                    "legacy_spool",
                    "interaction_events",
                    "INSERT OR IGNORE INTO interaction_events SELECT * FROM legacy_spool.interaction_events",
                )?;
                execute_copy_if_legacy_table_exists(
                    conn,
                    "legacy_spool",
                    "interaction_spool_queue",
                    "INSERT OR IGNORE INTO interaction_spool_queue SELECT * FROM legacy_spool.interaction_spool_queue",
                )?;
                conn.execute_batch("COMMIT;")
                    .context("committing interaction spool import transaction")?;
                Ok(())
            })();
            if result.is_err() {
                let _ = conn.execute_batch("ROLLBACK;");
            }
            detach_if_needed(conn, "legacy_spool")?;
            result
        })
    }

    fn import_legacy_checkpoint_metadata_if_needed(&self) -> Result<()> {
        let legacy_root = self.repo_root.join(LEGACY_BITLOOPS_METADATA_DIR);
        if !legacy_root.is_dir() {
            return Ok(());
        }

        for entry in fs::read_dir(&legacy_root)
            .with_context(|| format!("reading legacy metadata root {}", legacy_root.display()))?
        {
            let entry = match entry {
                Ok(entry) => entry,
                Err(err) => {
                    eprintln!("[bitloops] Warning: failed reading legacy metadata entry: {err}");
                    continue;
                }
            };
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            if !file_type.is_dir() {
                continue;
            }
            let session_id = entry.file_name().to_string_lossy().to_string();
            let session_dir = entry.path();
            let removable = self.import_legacy_session_metadata_dir(&session_id, &session_dir);
            match removable {
                Ok(true) => {
                    let _ = fs::remove_dir_all(&session_dir);
                }
                Ok(false) => {}
                Err(err) => {
                    eprintln!(
                        "[bitloops] Warning: failed importing legacy metadata for session `{session_id}`: {err:#}"
                    );
                }
            }
        }

        if fs::read_dir(&legacy_root)
            .ok()
            .and_then(|mut entries| entries.next())
            .is_none()
        {
            let _ = fs::remove_dir_all(&legacy_root);
        }

        Ok(())
    }

    fn import_legacy_session_metadata_dir(
        &self,
        session_id: &str,
        session_dir: &Path,
    ) -> Result<bool> {
        let mut removable = true;
        let transcript_path = session_dir.join(paths::TRANSCRIPT_FILE_NAME);
        let prompt_path = session_dir.join(paths::PROMPT_FILE_NAME);
        let summary_path = session_dir.join(paths::SUMMARY_FILE_NAME);
        let context_path = session_dir.join(paths::CONTEXT_FILE_NAME);

        if transcript_path.exists()
            || prompt_path.exists()
            || summary_path.exists()
            || context_path.exists()
        {
            let transcript = fs::read(&transcript_path).unwrap_or_default();
            let prompt_text = fs::read_to_string(&prompt_path).unwrap_or_default();
            let summary_text = fs::read_to_string(&summary_path).unwrap_or_default();
            let context = fs::read(&context_path).unwrap_or_default();
            let prompts_from_prompt_file = prompt_text
                .split("\n\n---\n\n")
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
                .collect::<Vec<_>>();
            let derived_prompts = if !prompts_from_prompt_file.is_empty() {
                prompts_from_prompt_file
            } else {
                extract_prompts_from_transcript_bytes(&transcript)
            };
            let commit_message = derived_prompts.last().cloned().unwrap_or_default();
            let mut bundle = if !transcript.is_empty() {
                build_session_metadata_bundle(session_id, &commit_message, &transcript)?
            } else {
                SessionMetadataBundle::default()
            };
            if !derived_prompts.is_empty() {
                bundle.prompts = derived_prompts;
            }
            if !summary_text.trim().is_empty() {
                bundle.summary = summary_text;
            } else if bundle.summary.trim().is_empty() {
                bundle.summary = extract_summary_from_transcript_bytes(&transcript);
            }
            if !context.is_empty() {
                bundle.context = context;
            } else if bundle.context.is_empty() {
                bundle.context = build_context_markdown(
                    session_id,
                    &commit_message,
                    &bundle.prompts,
                    &bundle.summary,
                )
                .into_bytes();
            }
            if bundle.transcript.is_empty() {
                bundle.transcript = transcript;
            }

            if !bundle.transcript.is_empty()
                || !bundle.prompts.is_empty()
                || !bundle.summary.trim().is_empty()
                || !bundle.context.is_empty()
            {
                let mut snapshot = SessionMetadataSnapshot::new(session_id.to_string(), bundle);
                snapshot.snapshot_id = format!(
                    "legacy-{}",
                    &sha256_hex(session_dir.to_string_lossy().as_bytes())[..16]
                );
                snapshot.transcript_path = transcript_path.to_string_lossy().to_string();
                self.save_session_metadata_snapshot(&snapshot)?;
            }
        }

        let tasks_dir = session_dir.join("tasks");
        if tasks_dir.exists() {
            for task_entry in fs::read_dir(&tasks_dir)
                .with_context(|| format!("reading legacy tasks dir {}", tasks_dir.display()))?
            {
                let task_entry = match task_entry {
                    Ok(entry) => entry,
                    Err(err) => {
                        eprintln!("[bitloops] Warning: failed reading legacy task entry: {err}");
                        removable = false;
                        continue;
                    }
                };
                let Ok(task_file_type) = task_entry.file_type() else {
                    removable = false;
                    continue;
                };
                if !task_file_type.is_dir() {
                    removable = false;
                    continue;
                }
                if !self.import_legacy_task_metadata_dir(
                    session_id,
                    &task_entry.file_name().to_string_lossy(),
                    &task_entry.path(),
                )? {
                    removable = false;
                }
            }
        }

        Ok(removable)
    }

    fn import_legacy_task_metadata_dir(
        &self,
        session_id: &str,
        tool_use_id: &str,
        task_dir: &Path,
    ) -> Result<bool> {
        let mut removable = true;
        for entry in fs::read_dir(task_dir)
            .with_context(|| format!("reading legacy task metadata dir {}", task_dir.display()))?
        {
            let entry = match entry {
                Ok(entry) => entry,
                Err(err) => {
                    eprintln!(
                        "[bitloops] Warning: failed reading legacy task metadata file: {err}"
                    );
                    removable = false;
                    continue;
                }
            };
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();
            let Ok(file_type) = entry.file_type() else {
                removable = false;
                continue;
            };
            if file_type.is_dir() {
                if name == "checkpoints" {
                    for checkpoint_entry in fs::read_dir(&path).with_context(|| {
                        format!("reading legacy incremental checkpoints {}", path.display())
                    })? {
                        let checkpoint_entry = match checkpoint_entry {
                            Ok(entry) => entry,
                            Err(err) => {
                                eprintln!(
                                    "[bitloops] Warning: failed reading legacy incremental checkpoint: {err}"
                                );
                                removable = false;
                                continue;
                            }
                        };
                        let checkpoint_path = checkpoint_entry.path();
                        let checkpoint_name =
                            checkpoint_entry.file_name().to_string_lossy().to_string();
                        if checkpoint_path.extension().and_then(|ext| ext.to_str()) != Some("json")
                        {
                            removable = false;
                            continue;
                        }
                        let payload = match fs::read(&checkpoint_path) {
                            Ok(payload) => payload,
                            Err(err) => {
                                eprintln!(
                                    "[bitloops] Warning: failed reading legacy incremental checkpoint {}: {err}",
                                    checkpoint_path.display()
                                );
                                removable = false;
                                continue;
                            }
                        };
                        let mut artefact = TaskCheckpointArtefact::new(
                            session_id.to_string(),
                            tool_use_id.to_string(),
                            RuntimeMetadataBlobType::IncrementalCheckpoint,
                            payload,
                        );
                        artefact.artefact_id = format!(
                            "legacy-{}",
                            &sha256_hex(checkpoint_path.to_string_lossy().as_bytes())[..16]
                        );
                        artefact.incremental_sequence =
                            parse_incremental_sequence_from_name(&checkpoint_name);
                        artefact.is_incremental = true;
                        self.save_task_checkpoint_artefact(&artefact)?;
                    }
                } else {
                    removable = false;
                }
                continue;
            }

            let kind = if name == paths::CHECKPOINT_FILE_NAME {
                Some(RuntimeMetadataBlobType::TaskCheckpoint)
            } else if name == paths::PROMPT_FILE_NAME {
                Some(RuntimeMetadataBlobType::Prompt)
            } else if name.starts_with("agent-") && name.ends_with(".jsonl") {
                Some(RuntimeMetadataBlobType::SubagentTranscript)
            } else {
                None
            };
            let Some(kind) = kind else {
                removable = false;
                continue;
            };

            let payload = match fs::read(&path) {
                Ok(payload) => payload,
                Err(err) => {
                    eprintln!(
                        "[bitloops] Warning: failed reading legacy task metadata {}: {err}",
                        path.display()
                    );
                    removable = false;
                    continue;
                }
            };
            let mut artefact = TaskCheckpointArtefact::new(
                session_id.to_string(),
                tool_use_id.to_string(),
                kind,
                payload,
            );
            artefact.artefact_id = format!(
                "legacy-{}",
                &sha256_hex(path.to_string_lossy().as_bytes())[..16]
            );
            if kind == RuntimeMetadataBlobType::SubagentTranscript {
                artefact.agent_id = name
                    .trim_start_matches("agent-")
                    .trim_end_matches(".jsonl")
                    .to_string();
            }
            self.save_task_checkpoint_artefact(&artefact)?;
        }

        Ok(removable)
    }
}

impl DaemonSqliteRuntimeStore {
    pub fn open() -> Result<Self> {
        Self::open_at(default_global_runtime_db_path())
    }

    pub fn open_at(db_path: PathBuf) -> Result<Self> {
        let sqlite = SqliteConnectionPool::connect(db_path.clone())
            .with_context(|| format!("opening daemon runtime database {}", db_path.display()))?;
        sqlite
            .execute_batch(RUNTIME_DOCUMENTS_SCHEMA)
            .context("initialising daemon runtime documents schema")?;
        Ok(Self { db_path })
    }

    pub fn db_path(&self) -> &Path {
        &self.db_path
    }

    pub fn runtime_state_exists(&self) -> Result<bool> {
        self.document_exists(document_key_runtime_state())
    }

    pub fn sync_state_exists(&self) -> Result<bool> {
        self.document_exists(document_key_sync_state())
    }

    pub fn enrichment_state_exists(&self) -> Result<bool> {
        self.document_exists(document_key_enrichment_state())
    }

    pub fn load_runtime_state(&self) -> Result<Option<DaemonRuntimeState>> {
        self.load_document(
            document_key_runtime_state(),
            Some(runtime_state_path(Path::new("."))),
        )
    }

    pub fn save_runtime_state(&self, state: &DaemonRuntimeState) -> Result<()> {
        self.save_document(document_key_runtime_state(), state)
    }

    pub fn delete_runtime_state(&self) -> Result<()> {
        self.delete_document(document_key_runtime_state())
    }

    pub fn load_service_metadata(&self) -> Result<Option<DaemonServiceMetadata>> {
        self.load_document(
            document_key_service_metadata(),
            Some(service_metadata_path(Path::new("."))),
        )
    }

    pub fn save_service_metadata(&self, state: &DaemonServiceMetadata) -> Result<()> {
        self.save_document(document_key_service_metadata(), state)
    }

    pub fn delete_service_metadata(&self) -> Result<()> {
        self.delete_document(document_key_service_metadata())
    }

    pub fn load_supervisor_runtime_state(&self) -> Result<Option<SupervisorRuntimeState>> {
        self.load_document(
            document_key_supervisor_runtime_state(),
            Some(legacy_supervisor_runtime_state_path()),
        )
    }

    pub fn save_supervisor_runtime_state(&self, state: &SupervisorRuntimeState) -> Result<()> {
        self.save_document(document_key_supervisor_runtime_state(), state)
    }

    pub fn delete_supervisor_runtime_state(&self) -> Result<()> {
        self.delete_document(document_key_supervisor_runtime_state())
    }

    pub fn load_supervisor_service_metadata(&self) -> Result<Option<SupervisorServiceMetadata>> {
        self.load_document(
            document_key_supervisor_service_metadata(),
            Some(legacy_supervisor_service_metadata_path()),
        )
    }

    pub fn save_supervisor_service_metadata(
        &self,
        state: &SupervisorServiceMetadata,
    ) -> Result<()> {
        self.save_document(document_key_supervisor_service_metadata(), state)
    }

    pub fn delete_supervisor_service_metadata(&self) -> Result<()> {
        self.delete_document(document_key_supervisor_service_metadata())
    }

    pub fn load_sync_queue_state(&self) -> Result<Option<PersistedSyncQueueState>> {
        self.load_document(document_key_sync_state(), Some(sync_state_legacy_path()))
    }

    pub fn mutate_sync_queue_state<T>(
        &self,
        mutate: impl FnOnce(&mut PersistedSyncQueueState) -> Result<T>,
    ) -> Result<T> {
        self.mutate_document(
            document_key_sync_state(),
            Some(sync_state_legacy_path()),
            PersistedSyncQueueState::default,
            mutate,
        )
    }

    pub fn load_enrichment_queue_state(&self) -> Result<Option<PersistedEnrichmentQueueState>> {
        self.load_document(
            document_key_enrichment_state(),
            Some(enrichment_state_legacy_path()),
        )
    }

    pub fn save_enrichment_queue_state(&self, state: &PersistedEnrichmentQueueState) -> Result<()> {
        self.save_document(document_key_enrichment_state(), state)
    }

    fn document_exists(&self, kind: &'static str) -> Result<bool> {
        let sqlite = SqliteConnectionPool::connect(self.db_path.clone()).with_context(|| {
            format!("opening daemon runtime database {}", self.db_path.display())
        })?;
        sqlite.with_connection(|conn| {
            conn.query_row(
                "SELECT 1 FROM runtime_documents WHERE document_kind = ?1 LIMIT 1",
                rusqlite::params![kind],
                |row| row.get::<_, i64>(0),
            )
            .optional()
            .map(|row| row.is_some())
            .map_err(anyhow::Error::from)
        })
    }

    fn load_document<T>(
        &self,
        kind: &'static str,
        legacy_path: Option<PathBuf>,
    ) -> Result<Option<T>>
    where
        T: DeserializeOwned,
    {
        let sqlite = SqliteConnectionPool::connect(self.db_path.clone()).with_context(|| {
            format!("opening daemon runtime database {}", self.db_path.display())
        })?;
        sqlite.with_connection(|conn| {
            import_legacy_document_if_needed(conn, kind, legacy_path.as_deref())?;
            let payload = load_document_payload(conn, kind)?;
            payload
                .map(|payload| {
                    serde_json::from_str::<T>(&payload)
                        .with_context(|| format!("parsing runtime document `{kind}`"))
                })
                .transpose()
        })
    }

    fn save_document<T>(&self, kind: &'static str, value: &T) -> Result<()>
    where
        T: Serialize,
    {
        let sqlite = SqliteConnectionPool::connect(self.db_path.clone()).with_context(|| {
            format!("opening daemon runtime database {}", self.db_path.display())
        })?;
        sqlite.with_connection(|conn| {
            store_document_payload(conn, kind, &serde_json::to_string(value)?)?;
            Ok(())
        })
    }

    fn delete_document(&self, kind: &'static str) -> Result<()> {
        let sqlite = SqliteConnectionPool::connect(self.db_path.clone()).with_context(|| {
            format!("opening daemon runtime database {}", self.db_path.display())
        })?;
        sqlite.with_connection(|conn| {
            conn.execute(
                "DELETE FROM runtime_documents WHERE document_kind = ?1",
                rusqlite::params![kind],
            )
            .context("deleting runtime document")?;
            Ok(())
        })
    }

    fn mutate_document<TDoc, TResult>(
        &self,
        kind: &'static str,
        legacy_path: Option<PathBuf>,
        default: impl FnOnce() -> TDoc,
        mutate: impl FnOnce(&mut TDoc) -> Result<TResult>,
    ) -> Result<TResult>
    where
        TDoc: Serialize + DeserializeOwned,
    {
        let sqlite = SqliteConnectionPool::connect(self.db_path.clone()).with_context(|| {
            format!("opening daemon runtime database {}", self.db_path.display())
        })?;
        sqlite.with_connection(|conn| {
            conn.execute_batch("BEGIN IMMEDIATE TRANSACTION;")
                .context("starting runtime document transaction")?;
            let result = (|| {
                import_legacy_document_if_needed(conn, kind, legacy_path.as_deref())?;
                let mut value = load_document_payload(conn, kind)?
                    .map(|payload| {
                        serde_json::from_str::<TDoc>(&payload)
                            .with_context(|| format!("parsing runtime document `{kind}`"))
                    })
                    .transpose()?
                    .unwrap_or_else(default);
                let output = mutate(&mut value)?;
                store_document_payload(conn, kind, &serde_json::to_string(&value)?)?;
                conn.execute_batch("COMMIT;")
                    .context("committing runtime document transaction")?;
                Ok(output)
            })();
            if result.is_err() {
                let _ = conn.execute_batch("ROLLBACK;");
            }
            result
        })
    }
}

fn session_snapshot_blob_key(
    repo_id: &str,
    session_id: &str,
    snapshot_id: &str,
    blob_type: RuntimeMetadataBlobType,
) -> String {
    format!(
        "runtime/{repo_id}/session-metadata/{session_id}/{snapshot_id}/{}",
        blob_type.default_file_name()
    )
}

fn task_artefact_blob_key(repo_id: &str, artefact: &TaskCheckpointArtefact) -> String {
    let file_name = match artefact.kind {
        RuntimeMetadataBlobType::TaskCheckpoint => paths::CHECKPOINT_FILE_NAME.to_string(),
        RuntimeMetadataBlobType::Prompt => paths::PROMPT_FILE_NAME.to_string(),
        RuntimeMetadataBlobType::SubagentTranscript => {
            if artefact.agent_id.trim().is_empty() {
                "agent.jsonl".to_string()
            } else {
                format!("agent-{}.jsonl", artefact.agent_id)
            }
        }
        RuntimeMetadataBlobType::IncrementalCheckpoint => {
            if let Some(sequence) = artefact.incremental_sequence {
                format!("{sequence:03}-{}.json", artefact.tool_use_id)
            } else {
                "incremental-checkpoint.json".to_string()
            }
        }
        RuntimeMetadataBlobType::Transcript => TRANSCRIPT_FILE_NAME.to_string(),
        RuntimeMetadataBlobType::Prompts => paths::PROMPT_FILE_NAME.to_string(),
        RuntimeMetadataBlobType::Summary => paths::SUMMARY_FILE_NAME.to_string(),
        RuntimeMetadataBlobType::Context => paths::CONTEXT_FILE_NAME.to_string(),
    };

    format!(
        "runtime/{repo_id}/task-checkpoint-artefacts/{}/{}/{}/{}",
        artefact.session_id, artefact.tool_use_id, artefact.artefact_id, file_name
    )
}

fn parse_incremental_sequence_from_name(name: &str) -> Option<u32> {
    name.split('-').next()?.parse::<u32>().ok()
}

fn sha256_hex(data: &[u8]) -> String {
    let digest = Sha256::digest(data);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn initialise_repo_runtime_schema(sqlite: &SqliteConnectionPool) -> Result<()> {
    sqlite
        .execute_batch(crate::host::devql::checkpoint_runtime_schema_sql_sqlite())
        .context("initialising runtime checkpoint schema")?;
    let spool = SqliteInteractionSpool::new(sqlite.clone(), "__runtime-bootstrap__".to_string())
        .context("initialising interaction spool schema in runtime db")?;
    drop(spool);
    Ok(())
}

fn legacy_relational_sqlite_path(repo_root: &Path) -> Result<PathBuf> {
    let cfg = resolve_store_backend_config_for_repo(repo_root)
        .context("resolving backend config for legacy relational runtime migration")?;
    cfg.relational
        .resolve_sqlite_db_path_for_repo(repo_root)
        .context("resolving legacy relational sqlite path")
}

fn all_tables_empty(conn: &rusqlite::Connection, tables: &[&str]) -> Result<bool> {
    for table in tables {
        if !table_exists(conn, table)? {
            continue;
        }
        let count: i64 = conn
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                row.get(0)
            })
            .with_context(|| format!("counting rows in `{table}`"))?;
        if count > 0 {
            return Ok(false);
        }
    }
    Ok(true)
}

fn table_exists(conn: &rusqlite::Connection, table: &str) -> Result<bool> {
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
            rusqlite::params![table],
            |row| row.get(0),
        )
        .with_context(|| format!("checking table `{table}`"))?;
    Ok(count > 0)
}

fn attach_if_needed(conn: &rusqlite::Connection, path: &Path, alias: &str) -> Result<()> {
    conn.execute(
        &format!("ATTACH DATABASE ?1 AS {alias}"),
        rusqlite::params![path.display().to_string()],
    )
    .with_context(|| format!("attaching database {} as {alias}", path.display()))?;
    Ok(())
}

fn detach_if_needed(conn: &rusqlite::Connection, alias: &str) -> Result<()> {
    conn.execute_batch(&format!("DETACH DATABASE {alias}"))
        .with_context(|| format!("detaching database alias `{alias}`"))?;
    Ok(())
}

fn table_has_rows_in_attached_db(
    conn: &rusqlite::Connection,
    alias: &str,
    table: &str,
) -> Result<bool> {
    let count: i64 = conn
        .query_row(
            &format!(
                "SELECT COUNT(*) FROM {alias}.sqlite_master WHERE type = 'table' AND name = ?1"
            ),
            rusqlite::params![table],
            |row| row.get(0),
        )
        .with_context(|| format!("checking attached table `{alias}.{table}`"))?;
    if count == 0 {
        return Ok(false);
    }
    let row_count: i64 = conn
        .query_row(
            &format!("SELECT COUNT(*) FROM {alias}.{table}"),
            [],
            |row| row.get(0),
        )
        .with_context(|| format!("counting rows in `{alias}.{table}`"))?;
    Ok(row_count > 0)
}

fn execute_copy_if_legacy_table_exists(
    conn: &rusqlite::Connection,
    alias: &str,
    table: &str,
    sql: &str,
) -> Result<()> {
    if table_has_rows_in_attached_db(conn, alias, table)? {
        conn.execute_batch(sql)
            .with_context(|| format!("copying `{alias}.{table}` into runtime store"))?;
    }
    Ok(())
}

fn load_document_payload(conn: &rusqlite::Connection, kind: &str) -> Result<Option<String>> {
    conn.query_row(
        "SELECT payload FROM runtime_documents WHERE document_kind = ?1 LIMIT 1",
        rusqlite::params![kind],
        |row| row.get::<_, String>(0),
    )
    .optional()
    .map_err(anyhow::Error::from)
}

fn store_document_payload(conn: &rusqlite::Connection, kind: &str, payload: &str) -> Result<()> {
    conn.execute(
        "INSERT INTO runtime_documents (document_kind, payload, updated_at)
         VALUES (?1, ?2, datetime('now'))
         ON CONFLICT(document_kind) DO UPDATE SET
            payload = excluded.payload,
            updated_at = excluded.updated_at",
        rusqlite::params![kind, payload],
    )
    .context("storing runtime document")?;
    Ok(())
}

fn import_legacy_document_if_needed(
    conn: &rusqlite::Connection,
    kind: &str,
    legacy_path: Option<&Path>,
) -> Result<()> {
    if load_document_payload(conn, kind)?.is_some() {
        return Ok(());
    }
    let Some(path) = legacy_path else {
        return Ok(());
    };
    let payload = match std::fs::read_to_string(path) {
        Ok(payload) => payload,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(err) => {
            return Err(err).with_context(|| format!("reading legacy document {}", path.display()));
        }
    };
    store_document_payload(conn, kind, &payload)
}

fn document_key_runtime_state() -> &'static str {
    "daemon_runtime_state"
}

fn document_key_service_metadata() -> &'static str {
    "daemon_service_metadata"
}

fn document_key_supervisor_runtime_state() -> &'static str {
    "supervisor_runtime_state"
}

fn document_key_supervisor_service_metadata() -> &'static str {
    "supervisor_service_metadata"
}

fn document_key_sync_state() -> &'static str {
    "sync_queue_state"
}

fn document_key_enrichment_state() -> &'static str {
    "enrichment_queue_state"
}

fn daemon_state_root() -> PathBuf {
    crate::utils::platform_dirs::bitloops_state_dir()
        .unwrap_or_else(|_| std::env::temp_dir().join("bitloops").join("state"))
        .join("daemon")
}

fn sync_state_legacy_path() -> PathBuf {
    daemon_state_root().join(crate::daemon::SYNC_STATE_FILE_NAME)
}

fn enrichment_state_legacy_path() -> PathBuf {
    daemon_state_root().join(crate::daemon::ENRICHMENT_STATE_FILE_NAME)
}

fn legacy_supervisor_runtime_state_path() -> PathBuf {
    daemon_state_root().join(crate::daemon::SUPERVISOR_RUNTIME_STATE_FILE_NAME)
}

fn legacy_supervisor_service_metadata_path() -> PathBuf {
    daemon_state_root().join("supervisor-service.json")
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};

    use crate::host::interactions::db_store::legacy_interaction_spool_db_path;
    use crate::host::interactions::store::InteractionSpool;
    use crate::host::interactions::types::InteractionSession;
    use crate::storage::SqliteConnectionPool;
    use crate::test_support::git_fixtures::init_test_repo;
    use crate::test_support::process_state::with_env_var;
    use tempfile::TempDir;

    use super::*;

    #[test]
    fn repo_runtime_store_uses_repo_scoped_runtime_sqlite_path() {
        let dir = TempDir::new().expect("tempdir");
        init_test_repo(dir.path(), "main", "Bitloops Test", "bitloops@example.com");
        let expected = dir
            .path()
            .join(".bitloops")
            .join("stores")
            .join("runtime")
            .join("runtime.sqlite");
        let actual = RepoSqliteRuntimeStore::open(dir.path())
            .expect("open runtime store")
            .db_path
            .clone();
        assert_eq!(actual, expected);
    }

    #[test]
    fn daemon_runtime_store_persists_sync_state_in_sqlite() {
        let state_dir = TempDir::new().expect("tempdir");
        with_env_var(
            "BITLOOPS_TEST_STATE_DIR_OVERRIDE",
            Some(state_dir.path().to_string_lossy().as_ref()),
            || {
                let store = DaemonSqliteRuntimeStore::open().expect("open daemon runtime store");
                let output = store
                    .mutate_sync_queue_state(|state| {
                        state.version = 7;
                        Ok(state.version)
                    })
                    .expect("mutate sync queue state");
                assert_eq!(output, 7);
                let loaded = store
                    .load_sync_queue_state()
                    .expect("load sync queue state")
                    .expect("state exists");
                assert_eq!(loaded.version, 7);
            },
        );
    }

    #[test]
    fn persisted_sync_queue_state_default_preserves_legacy_values() {
        let default = PersistedSyncQueueState::default();
        assert_eq!(default.version, 1);
        assert!(default.tasks.is_empty());
        assert_eq!(default.last_action.as_deref(), Some("initialized"));
        assert_eq!(default.updated_at_unix, 0);
    }

    #[test]
    fn daemon_runtime_store_uses_legacy_sync_defaults_when_state_is_missing() {
        let state_dir = TempDir::new().expect("tempdir");
        with_env_var(
            "BITLOOPS_TEST_STATE_DIR_OVERRIDE",
            Some(state_dir.path().to_string_lossy().as_ref()),
            || {
                let store = DaemonSqliteRuntimeStore::open().expect("open daemon runtime store");
                let observed = store
                    .mutate_sync_queue_state(|state| Ok((state.version, state.last_action.clone())))
                    .expect("load default sync queue state");
                assert_eq!(observed.0, 1);
                assert_eq!(observed.1.as_deref(), Some("initialized"));
            },
        );
    }

    #[test]
    fn repo_runtime_store_imports_legacy_interaction_spool_from_standalone_sqlite() {
        let dir = TempDir::new().expect("tempdir");
        init_test_repo(dir.path(), "main", "Bitloops Test", "bitloops@example.com");
        let repo = crate::host::devql::resolve_repo_identity(dir.path()).expect("resolve repo");

        let legacy_path =
            legacy_interaction_spool_db_path(dir.path()).expect("resolve legacy spool path");
        fs::create_dir_all(legacy_path.parent().expect("legacy spool parent"))
            .expect("create legacy spool directory");

        let sqlite = SqliteConnectionPool::connect(legacy_path).expect("open legacy spool sqlite");
        let legacy_spool =
            SqliteInteractionSpool::new(sqlite, repo.repo_id.clone()).expect("open legacy spool");
        legacy_spool
            .record_session(&InteractionSession {
                session_id: "session-1".into(),
                repo_id: repo.repo_id,
                agent_type: "codex".into(),
                model: "gpt-5.4".into(),
                first_prompt: "hello".into(),
                transcript_path: "/tmp/transcript.jsonl".into(),
                worktree_path: dir.path().display().to_string(),
                worktree_id: "main".into(),
                started_at: "2026-04-06T10:00:00Z".into(),
                last_event_at: "2026-04-06T10:00:00Z".into(),
                updated_at: "2026-04-06T10:00:00Z".into(),
                ..InteractionSession::default()
            })
            .expect("record session in legacy spool");

        let store = RepoSqliteRuntimeStore::open(dir.path()).expect("open repo runtime store");
        let sessions = store
            .interaction_spool()
            .expect("open runtime interaction spool")
            .list_sessions(None, 10)
            .expect("list imported sessions");

        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].session_id, "session-1");
    }

    #[test]
    fn repo_runtime_store_imports_legacy_checkpoint_metadata_and_removes_files() {
        let dir = TempDir::new().expect("tempdir");
        init_test_repo(dir.path(), "main", "Bitloops Test", "bitloops@example.com");

        let session_dir = dir
            .path()
            .join(".bitloops")
            .join("metadata")
            .join("session-legacy");
        let task_dir = session_dir.join("tasks").join("toolu_legacy");
        let incremental_dir = task_dir.join("checkpoints");
        fs::create_dir_all(&incremental_dir).expect("create legacy metadata directories");

        fs::write(
            session_dir.join(paths::TRANSCRIPT_FILE_NAME),
            r#"{"type":"user","message":{"content":"Create foo"}}
{"type":"assistant","message":{"content":"Done"}}"#,
        )
        .expect("write legacy transcript");
        fs::write(session_dir.join(paths::PROMPT_FILE_NAME), "Create foo")
            .expect("write legacy prompt");
        fs::write(session_dir.join(paths::SUMMARY_FILE_NAME), "Done")
            .expect("write legacy summary");
        fs::write(
            session_dir.join(paths::CONTEXT_FILE_NAME),
            "# Session Context\n\nLegacy context",
        )
        .expect("write legacy context");
        fs::write(
            task_dir.join(paths::CHECKPOINT_FILE_NAME),
            r#"{"checkpoint_uuid":"legacy-checkpoint"}"#,
        )
        .expect("write legacy task checkpoint");
        fs::write(
            task_dir.join("agent-agent-1.jsonl"),
            r#"{"type":"assistant","message":{"content":"subagent"}}"#,
        )
        .expect("write legacy subagent transcript");
        fs::write(
            incremental_dir.join("003-toolu_legacy.json"),
            r#"{"type":"TodoWrite","data":{"todo":"document storage"}}"#,
        )
        .expect("write legacy incremental checkpoint");

        let store = RepoSqliteRuntimeStore::open(dir.path()).expect("open repo runtime store");
        let snapshot = store
            .load_latest_session_metadata_snapshot("session-legacy")
            .expect("load imported metadata snapshot")
            .expect("legacy metadata snapshot should be imported");
        assert_eq!(snapshot.bundle.prompts, vec!["Create foo".to_string()]);
        assert_eq!(snapshot.bundle.summary, "Done");
        assert!(
            String::from_utf8_lossy(&snapshot.bundle.context).contains("Legacy context"),
            "legacy context should be preserved during import"
        );

        let artefacts = store
            .load_task_checkpoint_artefacts("session-legacy", "toolu_legacy")
            .expect("load imported task artefacts");
        assert!(
            artefacts
                .iter()
                .any(|artefact| artefact.kind == RuntimeMetadataBlobType::TaskCheckpoint),
            "task checkpoint artefact should be imported"
        );
        assert!(
            artefacts
                .iter()
                .any(|artefact| artefact.kind == RuntimeMetadataBlobType::SubagentTranscript),
            "subagent transcript artefact should be imported"
        );
        assert!(
            artefacts.iter().any(|artefact| {
                artefact.kind == RuntimeMetadataBlobType::IncrementalCheckpoint
                    && artefact.incremental_sequence == Some(3)
            }),
            "incremental checkpoint artefact should be imported with its sequence"
        );

        assert!(
            !session_dir.exists(),
            "legacy metadata directory should be removed after successful import"
        );
    }

    fn collect_rust_files(root: &Path, out: &mut Vec<PathBuf>) {
        let mut entries = fs::read_dir(root)
            .expect("read source directory")
            .collect::<Result<Vec<_>, _>>()
            .expect("read source directory entries");
        entries.sort_by_key(|entry| entry.path());
        for entry in entries {
            let path = entry.path();
            if path.is_dir() {
                collect_rust_files(&path, out);
            } else if path.extension().and_then(|ext| ext.to_str()) == Some("rs") {
                out.push(path);
            }
        }
    }

    fn is_test_like(relative_path: &str) -> bool {
        relative_path.contains("/tests/")
            || relative_path.ends_with("/tests.rs")
            || relative_path.contains("_tests/")
            || relative_path.ends_with("_tests.rs")
            || relative_path.ends_with("_test.rs")
    }

    fn strip_inline_test_module(contents: &str) -> &str {
        contents
            .rfind("\n#[cfg(test)]")
            .map(|index| &contents[..index])
            .or_else(|| {
                contents
                    .rfind("\r\n#[cfg(test)]")
                    .map(|index| &contents[..index])
            })
            .unwrap_or(contents)
    }

    #[test]
    fn runtime_and_relational_store_boundaries_are_not_bypassed() {
        let src_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut files = Vec::new();
        collect_rust_files(&src_root, &mut files);
        files.sort();

        let allowed_temporary_path_shims =
            ["host/checkpoints/strategy/manual_commit/checkpoint_io/temporary.rs"];
        let allowed_spool_path_shims = [
            "host/interactions/db_store.rs",
            "host/checkpoints/lifecycle/adapters.rs",
            "host/checkpoints/strategy/manual_commit_tests/post_commit/helpers.rs",
        ];
        let banned_daemon_json_imports = [
            "super::super::state_store::{read_json, write_json}",
            "crate::daemon::state_store::{read_json, write_json}",
        ];
        let allowed_direct_sqlite_modules = [
            "host/runtime_store.rs",
            "host/relational_store.rs",
            "host/devql/types.rs",
            "host/devql/db_utils.rs",
            "host/devql/connection_status.rs",
            "host/devql/ingestion/schema/relational_initialisation.rs",
            "host/checkpoints/session/db_backend.rs",
            "host/checkpoints/strategy/manual_commit/checkpoint_io/temporary.rs",
            "host/interactions/db_store.rs",
            "api/db/sqlite.rs",
            "capability_packs/semantic_clones/stage_semantic_features.rs",
            "capability_packs/knowledge/storage/sqlite_relational.rs",
        ];
        let skipped_prefixes = ["config/", "storage/", "test_support/", "utils/"];
        let banned_direct_sqlite_patterns = [
            "resolve_sqlite_db_path_for_repo(",
            ".resolve_sqlite_db_path_for_repo(",
            "default_relational_db_path(",
            "SqliteConnectionPool::connect(",
            "SqliteConnectionPool::connect_existing(",
            "rusqlite::Connection::open_with_flags(",
        ];
        let allowed_relational_internal_modules =
            ["host/relational_store.rs", "host/devql/types.rs"];
        let banned_relational_internal_patterns = [".local.path", "RelationalStorage::local_only("];
        let mut violations = Vec::new();

        for file in files {
            let relative = file
                .strip_prefix(&src_root)
                .expect("strip source root prefix")
                .to_string_lossy()
                .replace('\\', "/");
            if relative == "host/runtime_store.rs"
                || skipped_prefixes
                    .iter()
                    .any(|prefix| relative.starts_with(prefix))
            {
                continue;
            }
            if allowed_direct_sqlite_modules.contains(&relative.as_str()) || is_test_like(&relative)
            {
                continue;
            }
            let contents = fs::read_to_string(&file).expect("read source file");
            let production_contents = strip_inline_test_module(&contents);

            for banned_import in banned_daemon_json_imports {
                if production_contents.contains(banned_import) {
                    violations.push(format!(
                        "legacy daemon JSON helpers are forbidden outside the runtime store: {}",
                        relative
                    ));
                }
            }

            if production_contents.contains("resolve_temporary_checkpoint_sqlite_path(")
                && !allowed_temporary_path_shims.contains(&relative.as_str())
            {
                violations.push(format!(
                    "runtime checkpoint path shim must stay local to the runtime-store compatibility layer: {}",
                    relative
                ));
            }

            if production_contents.contains("interaction_spool_db_path(")
                && !allowed_spool_path_shims.contains(&relative.as_str())
            {
                violations.push(format!(
                    "interaction spool path shim must stay local to the runtime-store compatibility layer: {}",
                    relative
                ));
            }

            for banned_pattern in banned_direct_sqlite_patterns {
                if production_contents.contains(banned_pattern) {
                    violations.push(format!(
                        "direct SQLite access must flow through RuntimeStore or RelationalStore: {} (matched `{}`)",
                        relative, banned_pattern
                    ));
                }
            }

            if !allowed_relational_internal_modules.contains(&relative.as_str()) {
                for banned_pattern in banned_relational_internal_patterns {
                    if production_contents.contains(banned_pattern) {
                        violations.push(format!(
                            "RelationalStorage internals must stay local to store implementation layers: {} (matched `{}`)",
                            relative, banned_pattern
                        ));
                    }
                }
            }
        }

        if !violations.is_empty() {
            violations.sort();
            panic!(
                "Runtime/Relational store boundary violations:\n{}",
                violations.join("\n")
            );
        }
    }
}
