use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use rusqlite::OptionalExtension;
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::config::resolve_store_backend_config_for_repo;
use crate::daemon::{
    DaemonRuntimeState, DaemonServiceMetadata, PersistedEnrichmentQueueState,
    SupervisorRuntimeState, SupervisorServiceMetadata, SyncTaskRecord,
};
use crate::daemon::{runtime_state_path, service_metadata_path};
use crate::host::checkpoints::session::DbSessionBackend;
use crate::host::interactions::db_store::{
    SqliteInteractionSpool, legacy_interaction_spool_db_path,
};
use crate::storage::SqliteConnectionPool;
use crate::utils::paths::{default_global_runtime_db_path, default_repo_runtime_db_path};

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

    fn collect_rust_files(root: &Path, out: &mut Vec<PathBuf>) {
        let entries = fs::read_dir(root).expect("read source directory");
        for entry in entries {
            let entry = entry.expect("read source directory entry");
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
                assert!(
                    !production_contents.contains(banned_import),
                    "legacy daemon JSON helpers are forbidden outside the runtime store: {}",
                    relative
                );
            }

            if production_contents.contains("resolve_temporary_checkpoint_sqlite_path(")
                && !allowed_temporary_path_shims.contains(&relative.as_str())
            {
                panic!(
                    "runtime checkpoint path shim must stay local to the runtime-store compatibility layer: {}",
                    relative
                );
            }

            if production_contents.contains("interaction_spool_db_path(")
                && !allowed_spool_path_shims.contains(&relative.as_str())
            {
                panic!(
                    "interaction spool path shim must stay local to the runtime-store compatibility layer: {}",
                    relative
                );
            }

            for banned_pattern in banned_direct_sqlite_patterns {
                assert!(
                    !production_contents.contains(banned_pattern),
                    "direct SQLite access must flow through RuntimeStore or RelationalStore: {}",
                    relative
                );
            }

            if !allowed_relational_internal_modules.contains(&relative.as_str()) {
                for banned_pattern in banned_relational_internal_patterns {
                    assert!(
                        !production_contents.contains(banned_pattern),
                        "RelationalStorage internals must stay local to store implementation layers: {}",
                        relative
                    );
                }
            }
        }
    }
}
