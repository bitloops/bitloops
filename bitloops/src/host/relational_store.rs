use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde_json::Value;

use crate::config::{
    RelationalBackendConfig, resolve_bound_store_backend_config_for_repo,
    resolve_store_backend_config_for_repo,
};
use crate::host::devql::{DevqlConfig, RelationalDialect, RelationalStorage};
use crate::storage::SqliteConnectionPool;

pub trait RelationalStore: Send + Sync {
    fn sqlite_path(&self) -> &Path;
    fn has_remote(&self) -> bool;
    fn dialect(&self) -> RelationalDialect;
    fn local_sqlite_pool(&self) -> Result<SqliteConnectionPool>;

    fn exec<'a>(
        &'a self,
        sql: &'a str,
    ) -> core::pin::Pin<Box<dyn core::future::Future<Output = Result<()>> + 'a>>;

    fn exec_batch_transactional<'a>(
        &'a self,
        statements: &'a [String],
    ) -> core::pin::Pin<Box<dyn core::future::Future<Output = Result<()>> + 'a>>;

    fn exec_remote_batch_transactional<'a>(
        &'a self,
        statements: &'a [String],
    ) -> core::pin::Pin<Box<dyn core::future::Future<Output = Result<()>> + 'a>>;

    fn query_rows<'a>(
        &'a self,
        sql: &'a str,
    ) -> core::pin::Pin<Box<dyn core::future::Future<Output = Result<Vec<Value>>> + 'a>>;
}

#[derive(Debug)]
pub struct DefaultRelationalStore {
    inner: RelationalStorage,
}

impl DefaultRelationalStore {
    pub async fn connect(
        cfg: &DevqlConfig,
        relational: &RelationalBackendConfig,
        command: &str,
    ) -> Result<Self> {
        Ok(Self {
            inner: RelationalStorage::connect(cfg, relational, command).await?,
        })
    }

    pub fn local_only(path: PathBuf) -> Self {
        Self {
            inner: RelationalStorage::local_only(path),
        }
    }

    pub fn from_inner(inner: RelationalStorage) -> Self {
        Self { inner }
    }

    pub fn into_inner(self) -> RelationalStorage {
        self.inner
    }

    pub fn inner(&self) -> &RelationalStorage {
        &self.inner
    }

    pub fn sqlite_path(&self) -> &Path {
        &self.inner.local.path
    }

    pub fn open_local_for_repo_root(repo_root: &Path) -> Result<Self> {
        Self::open_local_for_roots(repo_root, repo_root)
    }

    pub fn open_local_for_repo_root_preferring_bound_config(repo_root: &Path) -> Result<Self> {
        let backends = resolve_bound_store_backend_config_for_repo(repo_root)
            .or_else(|_| resolve_store_backend_config_for_repo(repo_root))
            .context("resolving backend config for relational store")?;
        Self::open_local_for_backend_config(repo_root, &backends.relational)
    }

    pub fn open_local_for_backend_config(
        repo_root: &Path,
        relational: &RelationalBackendConfig,
    ) -> Result<Self> {
        let path = relational
            .resolve_sqlite_db_path_for_repo(repo_root)
            .context("resolving sqlite path for relational store")?;
        Ok(Self::local_only(path))
    }

    pub fn open_local_for_roots(config_root: &Path, repo_root: &Path) -> Result<Self> {
        let backends = resolve_store_backend_config_for_repo(config_root)
            .context("resolving backend config for relational store")?;
        let path = backends
            .relational
            .resolve_sqlite_db_path_for_repo(repo_root)
            .context("resolving sqlite path for relational store")?;
        Ok(Self::local_only(path))
    }

    pub fn to_local_inner(&self) -> RelationalStorage {
        RelationalStorage::local_only(self.inner.local.path.clone())
    }

    pub fn with_remote_client(&self, client: tokio_postgres::Client) -> RelationalStorage {
        RelationalStorage::with_remote_client(self.inner.local.path.clone(), client)
    }

    pub fn initialise_local_relational_checkpoint_schema(&self) -> Result<()> {
        let sqlite = self.local_sqlite_pool()?;
        sqlite
            .initialise_relational_checkpoint_schema()
            .context("initialising local relational checkpoint schema")
    }

    pub fn local_sqlite_pool_allow_create(&self) -> Result<SqliteConnectionPool> {
        SqliteConnectionPool::connect(self.inner.local.path.clone()).with_context(|| {
            format!(
                "opening relational sqlite at {}",
                self.inner.local.path.display()
            )
        })
    }

    pub fn initialise_local_devql_schema(&self) -> Result<()> {
        let sqlite = self.local_sqlite_pool_allow_create()?;
        sqlite
            .initialise_devql_schema()
            .context("initialising local DevQL sqlite schema")
    }

    pub fn execute_local_sqlite_batch_allow_create(&self, sql: &str) -> Result<()> {
        let sqlite = self.local_sqlite_pool_allow_create()?;
        sqlite
            .execute_batch(sql)
            .context("executing local SQLite batch via relational store")
    }

    pub async fn count_distinct_current_symbol_embedding_artefacts(
        &self,
        repo_id: &str,
        representation_kind: &str,
    ) -> Result<u64> {
        let rows = self
            .query_rows(&format!(
                "SELECT COUNT(DISTINCT artefact_id) AS total \
                 FROM symbol_embeddings_current \
                 WHERE repo_id = '{}' AND representation_kind = '{}'",
                escape_sql_string(repo_id),
                escape_sql_string(representation_kind),
            ))
            .await;
        let rows = match rows {
            Ok(rows) => rows,
            Err(err) if missing_current_embedding_table(&err) => return Ok(0),
            Err(err) => return Err(err),
        };

        Ok(rows
            .first()
            .and_then(|row| row.get("total"))
            .and_then(Value::as_u64)
            .unwrap_or_default())
    }
}

fn missing_current_embedding_table(err: &anyhow::Error) -> bool {
    let message = err.to_string();
    message.contains("no such table: symbol_embeddings_current")
        || message.contains("relation \"symbol_embeddings_current\" does not exist")
}

fn escape_sql_string(value: &str) -> String {
    value.replace('\'', "''")
}

impl RelationalStore for DefaultRelationalStore {
    fn sqlite_path(&self) -> &Path {
        &self.inner.local.path
    }

    fn has_remote(&self) -> bool {
        self.inner.remote.is_some()
    }

    fn dialect(&self) -> RelationalDialect {
        self.inner.dialect()
    }

    fn local_sqlite_pool(&self) -> Result<SqliteConnectionPool> {
        SqliteConnectionPool::connect_existing(self.inner.local.path.clone()).with_context(|| {
            format!(
                "opening relational sqlite at {}",
                self.inner.local.path.display()
            )
        })
    }

    fn exec<'a>(
        &'a self,
        sql: &'a str,
    ) -> core::pin::Pin<Box<dyn core::future::Future<Output = Result<()>> + 'a>> {
        Box::pin(async move { self.inner.exec(sql).await })
    }

    fn exec_batch_transactional<'a>(
        &'a self,
        statements: &'a [String],
    ) -> core::pin::Pin<Box<dyn core::future::Future<Output = Result<()>> + 'a>> {
        Box::pin(async move { self.inner.exec_batch_transactional(statements).await })
    }

    fn exec_remote_batch_transactional<'a>(
        &'a self,
        statements: &'a [String],
    ) -> core::pin::Pin<Box<dyn core::future::Future<Output = Result<()>> + 'a>> {
        Box::pin(async move {
            if !self.has_remote() {
                bail!("remote Postgres storage is not configured");
            }
            self.inner.exec_remote_batch_transactional(statements).await
        })
    }

    fn query_rows<'a>(
        &'a self,
        sql: &'a str,
    ) -> core::pin::Pin<Box<dyn core::future::Future<Output = Result<Vec<Value>>> + 'a>> {
        Box::pin(async move { self.inner.query_rows(sql).await })
    }
}
