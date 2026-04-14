use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use rusqlite::OptionalExtension;
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::daemon::{
    DaemonRuntimeState, DaemonServiceMetadata, PersistedEmbeddingsBootstrapState,
    PersistedEnrichmentQueueState, SupervisorRuntimeState, SupervisorServiceMetadata,
};
use crate::daemon::{runtime_state_path, service_metadata_path};
use crate::storage::SqliteConnectionPool;
use crate::utils::paths::default_global_runtime_db_path;

use super::types::{
    DaemonSqliteRuntimeStore, PersistedCapabilityEventQueueState, PersistedDevqlTaskQueueState,
    PersistedSyncQueueState,
};

const RUNTIME_DOCUMENTS_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS runtime_documents (
    document_kind TEXT PRIMARY KEY,
    payload TEXT NOT NULL,
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);
"#;

const PACK_RECONCILE_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS pack_reconcile_generations (
    repo_id TEXT NOT NULL,
    generation_seq INTEGER NOT NULL,
    source_task_id TEXT,
    sync_mode TEXT NOT NULL,
    active_branch TEXT,
    head_commit_sha TEXT,
    requires_full_reconcile INTEGER NOT NULL DEFAULT 0,
    created_at_unix INTEGER NOT NULL,
    PRIMARY KEY (repo_id, generation_seq)
);

CREATE INDEX IF NOT EXISTS idx_pack_reconcile_generations_repo_created
ON pack_reconcile_generations (repo_id, created_at_unix DESC);

CREATE TABLE IF NOT EXISTS pack_reconcile_file_changes (
    repo_id TEXT NOT NULL,
    generation_seq INTEGER NOT NULL,
    path TEXT NOT NULL,
    change_kind TEXT NOT NULL,
    language TEXT,
    content_id TEXT
);

CREATE INDEX IF NOT EXISTS idx_pack_reconcile_file_changes_repo_generation
ON pack_reconcile_file_changes (repo_id, generation_seq, path);

CREATE TABLE IF NOT EXISTS pack_reconcile_artefact_changes (
    repo_id TEXT NOT NULL,
    generation_seq INTEGER NOT NULL,
    symbol_id TEXT NOT NULL,
    change_kind TEXT NOT NULL,
    artefact_id TEXT NOT NULL,
    path TEXT NOT NULL,
    canonical_kind TEXT,
    name TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_pack_reconcile_artefact_changes_repo_generation
ON pack_reconcile_artefact_changes (repo_id, generation_seq, symbol_id);

CREATE TABLE IF NOT EXISTS pack_reconcile_consumers (
    repo_id TEXT NOT NULL,
    consumer_id TEXT NOT NULL,
    capability_id TEXT NOT NULL,
    last_applied_generation_seq INTEGER,
    last_error TEXT,
    updated_at_unix INTEGER NOT NULL,
    PRIMARY KEY (repo_id, consumer_id)
);

CREATE INDEX IF NOT EXISTS idx_pack_reconcile_consumers_repo_capability
ON pack_reconcile_consumers (repo_id, capability_id);

CREATE TABLE IF NOT EXISTS pack_reconcile_runs (
    run_id TEXT PRIMARY KEY,
    repo_id TEXT NOT NULL,
    repo_root TEXT NOT NULL,
    consumer_id TEXT NOT NULL,
    capability_id TEXT NOT NULL,
    from_generation_seq INTEGER NOT NULL,
    to_generation_seq INTEGER NOT NULL,
    reconcile_mode TEXT NOT NULL,
    status TEXT NOT NULL,
    attempts INTEGER NOT NULL,
    submitted_at_unix INTEGER NOT NULL,
    started_at_unix INTEGER,
    updated_at_unix INTEGER NOT NULL,
    completed_at_unix INTEGER,
    error TEXT
);

CREATE INDEX IF NOT EXISTS idx_pack_reconcile_runs_repo_consumer_status
ON pack_reconcile_runs (repo_id, consumer_id, status, submitted_at_unix);
"#;

fn initialise_runtime_schema(sqlite: &SqliteConnectionPool) -> Result<()> {
    sqlite
        .execute_batch(RUNTIME_DOCUMENTS_SCHEMA)
        .context("initialising daemon runtime documents schema")?;
    sqlite
        .execute_batch(PACK_RECONCILE_SCHEMA)
        .context("initialising current-state consumer runtime schema")?;
    sqlite
        .execute_batch(super::repo_workplane::REPO_WORKPLANE_SCHEMA)
        .context("initialising capability workplane schema in daemon runtime db")?;
    Ok(())
}

impl DaemonSqliteRuntimeStore {
    pub fn open() -> Result<Self> {
        Self::open_at(default_global_runtime_db_path())
    }

    pub fn open_at(db_path: PathBuf) -> Result<Self> {
        let sqlite = SqliteConnectionPool::connect(db_path.clone())
            .with_context(|| format!("opening daemon runtime database {}", db_path.display()))?;
        initialise_runtime_schema(&sqlite)?;
        Ok(Self { db_path })
    }

    pub fn db_path(&self) -> &Path {
        &self.db_path
    }

    fn open_sqlite_with_runtime_schema(&self) -> Result<SqliteConnectionPool> {
        let sqlite = SqliteConnectionPool::connect(self.db_path.clone()).with_context(|| {
            format!("opening daemon runtime database {}", self.db_path.display())
        })?;
        initialise_runtime_schema(&sqlite)?;
        Ok(sqlite)
    }

    pub fn with_connection<T>(
        &self,
        operation: impl FnOnce(&rusqlite::Connection) -> Result<T>,
    ) -> Result<T> {
        let sqlite = self.open_sqlite_with_runtime_schema()?;
        sqlite.with_connection(operation)
    }

    pub fn runtime_state_exists(&self) -> Result<bool> {
        self.document_exists(document_key_runtime_state())
    }

    pub fn devql_task_state_exists(&self) -> Result<bool> {
        self.document_exists(document_key_devql_task_state())
    }

    pub fn sync_state_exists(&self) -> Result<bool> {
        self.document_exists(document_key_sync_state())
    }

    pub fn enrichment_state_exists(&self) -> Result<bool> {
        self.document_exists(document_key_enrichment_state())
    }

    pub fn embeddings_bootstrap_state_exists(&self) -> Result<bool> {
        self.document_exists(document_key_embeddings_bootstrap_state())
    }

    pub fn capability_event_state_exists(&self) -> Result<bool> {
        self.document_exists(document_key_capability_event_state())
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
        let mut state: Option<PersistedSyncQueueState> =
            self.load_document(document_key_sync_state(), Some(sync_state_legacy_path()))?;
        if let Some(state) = state.as_mut() {
            state.normalise_legacy_values();
        }
        Ok(state)
    }

    pub fn load_devql_task_queue_state(&self) -> Result<Option<PersistedDevqlTaskQueueState>> {
        let mut state: Option<PersistedDevqlTaskQueueState> =
            self.load_document(document_key_devql_task_state(), None)?;
        if let Some(state) = state.as_mut() {
            state.normalise_legacy_values();
            return Ok(Some(state.clone()));
        }

        let Some(legacy_state) = self.load_sync_queue_state()? else {
            return Ok(None);
        };
        let migrated = migrate_legacy_sync_queue_state(legacy_state);
        self.save_document(document_key_devql_task_state(), &migrated)?;
        Ok(Some(migrated))
    }

    pub fn mutate_devql_task_queue_state<T>(
        &self,
        mutate: impl FnOnce(&mut PersistedDevqlTaskQueueState) -> Result<T>,
    ) -> Result<T> {
        if !self.devql_task_state_exists()?
            && let Some(legacy_state) = self.load_sync_queue_state()?
        {
            self.save_document(
                document_key_devql_task_state(),
                &migrate_legacy_sync_queue_state(legacy_state),
            )?;
        }

        self.mutate_document(
            document_key_devql_task_state(),
            None,
            PersistedDevqlTaskQueueState::default,
            |state| {
                state.normalise_legacy_values();
                mutate(state)
            },
        )
    }

    pub fn mutate_sync_queue_state<T>(
        &self,
        mutate: impl FnOnce(&mut PersistedSyncQueueState) -> Result<T>,
    ) -> Result<T> {
        self.mutate_document(
            document_key_sync_state(),
            Some(sync_state_legacy_path()),
            PersistedSyncQueueState::default,
            |state| {
                state.normalise_legacy_values();
                mutate(state)
            },
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

    pub fn load_embeddings_bootstrap_state(
        &self,
    ) -> Result<Option<PersistedEmbeddingsBootstrapState>> {
        self.load_document(document_key_embeddings_bootstrap_state(), None)
    }

    pub fn save_embeddings_bootstrap_state(
        &self,
        state: &PersistedEmbeddingsBootstrapState,
    ) -> Result<()> {
        self.save_document(document_key_embeddings_bootstrap_state(), state)
    }

    pub fn mutate_embeddings_bootstrap_state<T>(
        &self,
        mutate: impl FnOnce(&mut PersistedEmbeddingsBootstrapState) -> Result<T>,
    ) -> Result<T> {
        self.mutate_document(
            document_key_embeddings_bootstrap_state(),
            None,
            PersistedEmbeddingsBootstrapState::default,
            mutate,
        )
    }

    pub fn load_capability_event_queue_state(
        &self,
    ) -> Result<Option<PersistedCapabilityEventQueueState>> {
        let state: Option<PersistedCapabilityEventQueueState> = self.load_document(
            document_key_capability_event_state(),
            Some(capability_event_state_legacy_path()),
        )?;
        Ok(state)
    }

    pub fn mutate_capability_event_queue_state<T>(
        &self,
        mutate: impl FnOnce(&mut PersistedCapabilityEventQueueState) -> Result<T>,
    ) -> Result<T> {
        self.mutate_document(
            document_key_capability_event_state(),
            Some(capability_event_state_legacy_path()),
            PersistedCapabilityEventQueueState::default,
            mutate,
        )
    }

    fn document_exists(&self, kind: &'static str) -> Result<bool> {
        let sqlite = self.open_sqlite_with_runtime_schema()?;
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
        let sqlite = self.open_sqlite_with_runtime_schema()?;
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
        let sqlite = self.open_sqlite_with_runtime_schema()?;
        sqlite.with_connection(|conn| {
            store_document_payload(conn, kind, &serde_json::to_string(value)?)?;
            Ok(())
        })
    }

    fn delete_document(&self, kind: &'static str) -> Result<()> {
        let sqlite = self.open_sqlite_with_runtime_schema()?;
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
        let sqlite = self.open_sqlite_with_runtime_schema()?;
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

fn document_key_devql_task_state() -> &'static str {
    "devql_task_queue_state"
}

fn document_key_enrichment_state() -> &'static str {
    "enrichment_queue_state"
}

fn document_key_embeddings_bootstrap_state() -> &'static str {
    "embeddings_bootstrap_state"
}

fn document_key_capability_event_state() -> &'static str {
    "capability_event_queue_state"
}

fn daemon_state_root() -> PathBuf {
    crate::utils::platform_dirs::bitloops_state_dir()
        .unwrap_or_else(|_| std::env::temp_dir().join("bitloops").join("state"))
        .join("daemon")
}

fn sync_state_legacy_path() -> PathBuf {
    daemon_state_root().join(crate::daemon::SYNC_STATE_FILE_NAME)
}

fn migrate_legacy_sync_queue_state(
    legacy: PersistedSyncQueueState,
) -> PersistedDevqlTaskQueueState {
    PersistedDevqlTaskQueueState {
        version: legacy.version,
        tasks: legacy
            .tasks
            .into_iter()
            .map(|task| crate::daemon::DevqlTaskRecord {
                task_id: task.task_id,
                repo_id: task.repo_id,
                repo_name: task.repo_name,
                repo_provider: task.repo_provider,
                repo_organisation: task.repo_organisation,
                repo_identity: task.repo_identity,
                daemon_config_root: task.daemon_config_root,
                repo_root: task.repo_root,
                kind: crate::daemon::DevqlTaskKind::Sync,
                source: task.source,
                spec: crate::daemon::DevqlTaskSpec::Sync(crate::daemon::SyncTaskSpec {
                    mode: task.mode,
                }),
                status: task.status,
                submitted_at_unix: task.submitted_at_unix,
                started_at_unix: task.started_at_unix,
                updated_at_unix: task.updated_at_unix,
                completed_at_unix: task.completed_at_unix,
                queue_position: task.queue_position,
                tasks_ahead: task.tasks_ahead,
                progress: crate::daemon::DevqlTaskProgress::Sync(task.progress),
                error: task.error,
                result: task
                    .summary
                    .map(|summary| crate::daemon::DevqlTaskResult::Sync(Box::new(summary))),
            })
            .collect(),
        repo_controls: Default::default(),
        last_action: legacy.last_action,
        updated_at_unix: legacy.updated_at_unix,
    }
}

fn enrichment_state_legacy_path() -> PathBuf {
    daemon_state_root().join(crate::daemon::ENRICHMENT_STATE_FILE_NAME)
}

fn capability_event_state_legacy_path() -> PathBuf {
    daemon_state_root().join("capability-event-queue.json")
}

fn legacy_supervisor_runtime_state_path() -> PathBuf {
    daemon_state_root().join(crate::daemon::SUPERVISOR_RUNTIME_STATE_FILE_NAME)
}

fn legacy_supervisor_service_metadata_path() -> PathBuf {
    daemon_state_root().join("supervisor-service.json")
}
