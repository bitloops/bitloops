use anyhow::{Context, Result};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use super::SqliteConnectionPool;
use super::current_state::{
    migrate_artefact_edges_current_branch_scope, migrate_artefacts_current_branch_scope,
};
use super::introspection::{sqlite_table_exists, sqlite_table_has_column};

const DEVQL_LEGACY_BOOTSTRAP_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS repositories (
    repo_id TEXT PRIMARY KEY,
    provider TEXT NOT NULL,
    organization TEXT NOT NULL,
    name TEXT NOT NULL,
    default_branch TEXT,
    created_at TEXT DEFAULT (datetime('now'))
);
"#;

impl SqliteConnectionPool {
    pub fn initialise_checkpoint_schema(&self) -> Result<()> {
        self.initialise_runtime_checkpoint_schema()
            .context("initialising SQLite runtime checkpoint schema")?;
        self.initialise_relational_checkpoint_schema()
            .context("initialising SQLite relational checkpoint schema")
    }

    pub fn initialise_runtime_checkpoint_schema(&self) -> Result<()> {
        let schema_lock = sqlite_schema_lock_for(self.db_path());
        let _guard = schema_lock
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());

        self.execute_batch(crate::host::devql::checkpoint_runtime_schema_sql_sqlite())
            .context("initialising SQLite runtime checkpoint schema")?;
        self.with_connection(|conn| {
            match conn.execute_batch("ALTER TABLE sessions ADD COLUMN ended_at TEXT;") {
                Ok(()) => Ok(()),
                Err(err) if err.to_string().contains("duplicate column name: ended_at") => Ok(()),
                Err(err) => Err(err).context("executing SQLite ended_at migration"),
            }
        })
        .context("migrating SQLite checkpoint schema for sessions.ended_at")?;
        self.with_connection(|conn| {
            if !sqlite_table_exists(conn, "repo_watcher_registrations")?
                || sqlite_table_has_column(conn, "repo_watcher_registrations", "state")?
            {
                return Ok(());
            }

            conn.execute_batch(
                "ALTER TABLE repo_watcher_registrations
                 ADD COLUMN state TEXT NOT NULL DEFAULT 'ready';",
            )
            .context("executing SQLite repo_watcher_registrations.state migration")
        })
        .context("migrating SQLite checkpoint schema for repo_watcher_registrations.state")
    }

    pub fn initialise_relational_checkpoint_schema(&self) -> Result<()> {
        let schema_lock = sqlite_schema_lock_for(self.db_path());
        let _guard = schema_lock
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());

        self.execute_batch(crate::host::devql::checkpoint_relational_schema_sql_sqlite())
            .context("initialising SQLite relational checkpoint schema")
    }

    pub fn initialise_devql_schema(&self) -> Result<()> {
        let schema_lock = sqlite_schema_lock_for(self.db_path());
        let _guard = schema_lock
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());

        self.migrate_workspace_revisions_uniqueness()
            .context("migrating SQLite workspace_revisions uniqueness (pre-schema)")?;
        self.execute_batch(DEVQL_LEGACY_BOOTSTRAP_SQL)
            .context("bootstrapping SQLite DevQL catalog tables")?;
        self.migrate_devql_checkpoint_columns()
            .context("migrating SQLite DevQL checkpoint columns")?;
        self.migrate_devql_branch_scope_current_tables()
            .context("migrating SQLite DevQL current-state branch scope")?;
        self.execute_batch(crate::host::devql::devql_schema_sql_sqlite())
            .context("initialising SQLite DevQL schema")?;
        self.migrate_historical_artefacts_cutover()
            .context("migrating SQLite historical artefacts cutover")?;
        self.migrate_workspace_revisions_uniqueness()
            .context("migrating SQLite workspace_revisions uniqueness")
    }

    fn migrate_historical_artefacts_cutover(&self) -> Result<()> {
        let needs_cutover = self
            .with_connection(sqlite_artefacts_historical_needs_cutover)
            .context("inspecting SQLite historical artefacts schema shape")?;
        if needs_cutover {
            self.execute_batch(crate::host::devql::historical_artefacts_cutover_sqlite_sql())
                .context("applying SQLite historical artefacts one-shot cutover")?;
        }
        Ok(())
    }

    fn migrate_devql_checkpoint_columns(&self) -> Result<()> {
        let migrations = [
            (
                "artefacts_current",
                "revision_kind",
                "ALTER TABLE artefacts_current ADD COLUMN revision_kind TEXT NOT NULL DEFAULT 'commit';",
            ),
            (
                "artefacts_current",
                "revision_id",
                "ALTER TABLE artefacts_current ADD COLUMN revision_id TEXT NOT NULL DEFAULT '';",
            ),
            (
                "artefacts_current",
                "temp_checkpoint_id",
                "ALTER TABLE artefacts_current ADD COLUMN temp_checkpoint_id INTEGER;",
            ),
            (
                "artefact_edges_current",
                "revision_kind",
                "ALTER TABLE artefact_edges_current ADD COLUMN revision_kind TEXT NOT NULL DEFAULT 'commit';",
            ),
            (
                "artefact_edges_current",
                "revision_id",
                "ALTER TABLE artefact_edges_current ADD COLUMN revision_id TEXT NOT NULL DEFAULT '';",
            ),
            (
                "artefact_edges_current",
                "temp_checkpoint_id",
                "ALTER TABLE artefact_edges_current ADD COLUMN temp_checkpoint_id INTEGER;",
            ),
        ];
        self.with_connection(|conn| {
            for (table, column, sql) in migrations {
                if !sqlite_table_exists(conn, table)? {
                    continue;
                }
                let sync_shape_matches = match table {
                    "artefacts_current" => {
                        super::current_state::artefacts_current_matches_sync_shape(conn)?
                    }
                    "artefact_edges_current" => {
                        super::current_state::artefact_edges_current_matches_sync_shape(conn)?
                    }
                    _ => false,
                };
                if sync_shape_matches {
                    continue;
                }
                match conn.execute_batch(sql) {
                    Ok(()) => {}
                    Err(err)
                        if err
                            .to_string()
                            .contains(&format!("duplicate column name: {column}")) => {}
                    Err(err) => {
                        return Err(err).with_context(|| format!("adding column {column}"));
                    }
                }
            }
            Ok(())
        })
    }

    fn migrate_workspace_revisions_uniqueness(&self) -> Result<()> {
        self.with_connection(|conn| {
            if !sqlite_table_exists(conn, "workspace_revisions")? {
                return Ok(());
            }
            conn.execute_batch(
                r#"
DELETE FROM workspace_revisions
WHERE id NOT IN (
    SELECT MAX(id)
    FROM workspace_revisions
    GROUP BY repo_id, tree_hash
);
DROP INDEX IF EXISTS workspace_revisions_tree_idx;
CREATE UNIQUE INDEX IF NOT EXISTS workspace_revisions_repo_tree_unique_idx
ON workspace_revisions (repo_id, tree_hash);
"#,
            )
            .context("hardening workspace_revisions uniqueness")?;
            Ok(())
        })
    }

    fn migrate_devql_branch_scope_current_tables(&self) -> Result<()> {
        self.with_connection(|conn| {
            migrate_artefacts_current_branch_scope(conn)?;
            migrate_artefact_edges_current_branch_scope(conn)?;
            Ok(())
        })
    }
}

fn sqlite_artefacts_historical_needs_cutover(conn: &rusqlite::Connection) -> Result<bool> {
    if !sqlite_table_exists(conn, "artefacts")? {
        return Ok(false);
    }
    for column in [
        "blob_sha",
        "path",
        "parent_artefact_id",
        "start_line",
        "end_line",
        "start_byte",
        "end_byte",
    ] {
        if sqlite_table_has_column(conn, "artefacts", column)? {
            return Ok(true);
        }
    }
    Ok(false)
}

fn sqlite_schema_lock_for(db_path: &Path) -> Arc<Mutex<()>> {
    static LOCKS: OnceLock<Mutex<HashMap<PathBuf, Arc<Mutex<()>>>>> = OnceLock::new();
    let canonical = db_path
        .canonicalize()
        .unwrap_or_else(|_| db_path.to_path_buf());
    let mut locks = LOCKS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    Arc::clone(
        locks
            .entry(canonical)
            .or_insert_with(|| Arc::new(Mutex::new(()))),
    )
}
