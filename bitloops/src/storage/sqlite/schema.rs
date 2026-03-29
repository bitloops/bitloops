use anyhow::{Context, Result};

use super::SqliteConnectionPool;
use super::current_state::{
    migrate_artefact_edges_current_branch_scope, migrate_artefacts_current_branch_scope,
};
use super::introspection::sqlite_table_exists;

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
        self.execute_batch(crate::host::devql::checkpoint_schema_sql_sqlite())
            .context("initialising SQLite checkpoint schema")?;
        self.with_connection(|conn| {
            match conn.execute_batch("ALTER TABLE sessions ADD COLUMN ended_at TEXT;") {
                Ok(()) => Ok(()),
                Err(err) if err.to_string().contains("duplicate column name: ended_at") => Ok(()),
                Err(err) => Err(err).context("executing SQLite ended_at migration"),
            }
        })
        .context("migrating SQLite checkpoint schema for sessions.ended_at")
    }

    pub fn initialise_devql_schema(&self) -> Result<()> {
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
        self.migrate_workspace_revisions_uniqueness()
            .context("migrating SQLite workspace_revisions uniqueness")
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
