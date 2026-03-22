use anyhow::{Context, Result};

use crate::config::StoreBackendConfig;

use super::postgres::PostgresSyncConnection;
use super::sqlite::SqliteConnectionPool;

#[derive(Debug, Clone)]
pub struct CheckpointDbConnections {
    sqlite: SqliteConnectionPool,
    postgres: Option<PostgresSyncConnection>,
}

impl CheckpointDbConnections {
    pub fn connect_from_store_config(cfg: &StoreBackendConfig) -> Result<Self> {
        let sqlite_path = cfg
            .relational
            .resolve_sqlite_db_path()
            .context("resolving SQLite path for checkpoint storage")?;
        let sqlite = SqliteConnectionPool::connect_existing(sqlite_path)?;
        let postgres = cfg
            .relational
            .postgres_dsn
            .as_deref()
            .map(PostgresSyncConnection::connect)
            .transpose()?;

        Ok(Self { sqlite, postgres })
    }

    pub fn initialise_checkpoint_schema(&self) -> Result<()> {
        self.sqlite.initialise_checkpoint_schema()?;
        self.sqlite.initialise_devql_schema()?;
        if let Some(postgres) = &self.postgres {
            postgres.initialise_checkpoint_schema()?;
        }
        Ok(())
    }

    pub fn sqlite(&self) -> &SqliteConnectionPool {
        &self.sqlite
    }

    pub fn postgres(&self) -> Option<&PostgresSyncConnection> {
        self.postgres.as_ref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{StoreFileConfig, resolve_store_backend_config_for_tests};
    use anyhow::Result;
    use tempfile::TempDir;

    #[test]
    fn checkpoint_db_connections_initialise_sqlite_schema_even_without_postgres() -> Result<()> {
        let temp = TempDir::new().context("creating temp dir for sqlite")?;
        let sqlite_path = temp.path().join("db").join("relational.sqlite");
        std::fs::create_dir_all(
            sqlite_path
                .parent()
                .expect("sqlite path should have parent directory"),
        )
        .context("creating sqlite parent directory for test")?;
        let _ = rusqlite::Connection::open(&sqlite_path)
            .context("creating sqlite file for connect_existing test")?;
        let file_cfg = StoreFileConfig {
            sqlite_path: Some(sqlite_path.to_string_lossy().to_string()),
            ..Default::default()
        };

        let cfg = resolve_store_backend_config_for_tests(file_cfg)?;
        let connections = CheckpointDbConnections::connect_from_store_config(&cfg)?;
        connections.initialise_checkpoint_schema()?;

        let sessions_table_exists = connections.sqlite().with_connection(|conn| {
            let count: i64 = conn.query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'sessions'",
                [],
                |row| row.get(0),
            )?;
            Ok(count == 1)
        })?;
        let checkpoint_blobs_table_exists = connections.sqlite().with_connection(|conn| {
            let count: i64 = conn.query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'checkpoint_blobs'",
                [],
                |row| row.get(0),
            )?;
            Ok(count == 1)
        })?;

        assert!(sessions_table_exists);
        assert!(checkpoint_blobs_table_exists);
        assert!(connections.postgres().is_none());
        Ok(())
    }

    #[test]
    fn checkpoint_db_connections_enable_optional_postgres_when_dsn_present() -> Result<()> {
        let temp = TempDir::new().context("creating temp dir for sqlite")?;
        let sqlite_path = temp.path().join("relational.sqlite");
        let _ = rusqlite::Connection::open(&sqlite_path)
            .context("creating sqlite file for connect_existing test")?;
        let file_cfg = StoreFileConfig {
            relational_provider: Some("postgres".to_string()),
            sqlite_path: Some(sqlite_path.to_string_lossy().to_string()),
            pg_dsn: Some("postgres://bitloops:bitloops@localhost:5432/bitloops".to_string()),
            ..Default::default()
        };

        let cfg = resolve_store_backend_config_for_tests(file_cfg)?;
        let connections = CheckpointDbConnections::connect_from_store_config(&cfg)?;

        assert!(connections.postgres().is_some());
        Ok(())
    }
}
