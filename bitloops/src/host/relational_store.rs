use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde_json::Value;

use crate::config::{
    RelationalBackendConfig, resolve_bound_store_backend_config_for_repo,
    resolve_store_backend_config_for_repo,
};
use crate::host::devql::{
    DevqlConfig, RelationalDialect, RelationalPrimaryBackend, RelationalRoleBackend,
    RelationalStorage, RelationalStorageRole, sqlite_value_to_json,
};
use crate::storage::{PostgresSyncConnection, SqliteConnectionPool};

pub trait RelationalStore: Send + Sync {
    fn sqlite_path(&self) -> &Path;
    fn has_remote(&self) -> bool;
    fn dialect(&self) -> RelationalDialect;
    fn local_sqlite_pool(&self) -> Result<SqliteConnectionPool>;
    fn backend_for_role(&self, role: RelationalStorageRole) -> RelationalRoleBackend;
    fn dialect_for_role(&self, role: RelationalStorageRole) -> RelationalDialect;

    fn exec<'a>(
        &'a self,
        sql: &'a str,
    ) -> core::pin::Pin<Box<dyn core::future::Future<Output = Result<()>> + Send + 'a>>;

    fn exec_batch_transactional<'a>(
        &'a self,
        statements: &'a [String],
    ) -> core::pin::Pin<Box<dyn core::future::Future<Output = Result<()>> + Send + 'a>>;

    fn exec_remote_batch_transactional<'a>(
        &'a self,
        statements: &'a [String],
    ) -> core::pin::Pin<Box<dyn core::future::Future<Output = Result<()>> + Send + 'a>>;

    fn query_rows<'a>(
        &'a self,
        sql: &'a str,
    ) -> core::pin::Pin<Box<dyn core::future::Future<Output = Result<Vec<Value>>> + Send + 'a>>;

    fn exec_batch_transactional_for_role<'a>(
        &'a self,
        role: RelationalStorageRole,
        statements: &'a [String],
    ) -> core::pin::Pin<Box<dyn core::future::Future<Output = Result<()>> + Send + 'a>>;

    fn query_rows_for_role<'a>(
        &'a self,
        role: RelationalStorageRole,
        sql: &'a str,
    ) -> core::pin::Pin<Box<dyn core::future::Future<Output = Result<Vec<Value>>> + Send + 'a>>;
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

    pub fn backend_for_role(&self, role: RelationalStorageRole) -> RelationalRoleBackend {
        self.inner.backend_for_role(role)
    }

    pub fn dialect_for_role(&self, role: RelationalStorageRole) -> RelationalDialect {
        self.inner.dialect_for_role(role)
    }

    pub fn has_remote_shared_relational_authority(&self) -> bool {
        self.inner.has_remote_shared_relational_authority()
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

    pub fn open_primary_for_repo_root_preferring_bound_config(repo_root: &Path) -> Result<Self> {
        let backends = resolve_bound_store_backend_config_for_repo(repo_root)
            .or_else(|_| resolve_store_backend_config_for_repo(repo_root))
            .context("resolving backend config for relational store")?;
        Self::open_primary_for_backend_config(repo_root, &backends.relational)
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

    pub fn open_primary_for_backend_config(
        repo_root: &Path,
        relational: &RelationalBackendConfig,
    ) -> Result<Self> {
        let path = relational
            .resolve_sqlite_db_path_for_repo(repo_root)
            .context("resolving sqlite path for relational store")?;
        Ok(Self::from_inner(RelationalStorage::configured_primary(
            path,
            relational.postgres_dsn.clone(),
        )))
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

    pub fn query_rows_primary_blocking(&self, sql: &str) -> Result<Vec<Value>> {
        match self.inner.primary_backend() {
            RelationalPrimaryBackend::Sqlite => query_local_sqlite_rows_blocking(
                self.sqlite_path(),
                sql,
                "querying primary relational SQLite rows",
            ),
            RelationalPrimaryBackend::Postgres => {
                let dsn = self.inner.remote_dsn().ok_or_else(|| {
                    anyhow::anyhow!("remote Postgres primary backend is configured without a DSN")
                })?;
                PostgresSyncConnection::connect(dsn)?
                    .query_rows(sql)
                    .context("querying primary relational Postgres rows")
            }
        }
    }

    pub fn query_rows_for_role_blocking(
        &self,
        role: RelationalStorageRole,
        sql: &str,
    ) -> Result<Vec<Value>> {
        match self.backend_for_role(role) {
            RelationalRoleBackend::LocalSqlite => query_local_sqlite_rows_blocking(
                self.sqlite_path(),
                sql,
                "querying local relational SQLite rows for explicit role",
            ),
            RelationalRoleBackend::Postgres => {
                let dsn = self.inner.remote_dsn().ok_or_else(|| {
                    anyhow::anyhow!(
                        "remote Postgres shared relational backend is configured without a DSN"
                    )
                })?;
                PostgresSyncConnection::connect(dsn)?
                    .query_rows(sql)
                    .context("querying shared relational Postgres rows for explicit role")
            }
        }
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

fn query_local_sqlite_rows_blocking(
    path: &Path,
    sql: &str,
    context_label: &str,
) -> Result<Vec<Value>> {
    let sqlite = SqliteConnectionPool::connect_existing(path.to_path_buf())
        .with_context(|| format!("opening relational sqlite at {}", path.display()))?;
    sqlite.with_connection(|conn| {
        let mut stmt = conn.prepare(sql).with_context(|| context_label.to_string())?;
        let column_names = stmt
            .column_names()
            .iter()
            .map(|name| (*name).to_string())
            .collect::<Vec<_>>();
        let mut rows = stmt
            .query([])
            .with_context(|| format!("{context_label}: executing query"))?;
        let mut out = Vec::new();

        while let Some(row) = rows
            .next()
            .with_context(|| format!("{context_label}: iterating rows"))?
        {
            let mut object = serde_json::Map::new();
            for (index, column_name) in column_names.iter().enumerate() {
                let value = row.get_ref(index).with_context(|| {
                    format!(
                        "{context_label}: reading SQLite value for column index {index} (`{column_name}`)"
                    )
                })?;
                object.insert(column_name.clone(), sqlite_value_to_json(value));
            }
            out.push(Value::Object(object));
        }

        Ok(out)
    })
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

    fn backend_for_role(&self, role: RelationalStorageRole) -> RelationalRoleBackend {
        self.inner.backend_for_role(role)
    }

    fn dialect_for_role(&self, role: RelationalStorageRole) -> RelationalDialect {
        self.inner.dialect_for_role(role)
    }

    fn exec<'a>(
        &'a self,
        sql: &'a str,
    ) -> core::pin::Pin<Box<dyn core::future::Future<Output = Result<()>> + Send + 'a>> {
        Box::pin(async move { self.inner.exec(sql).await })
    }

    fn exec_batch_transactional<'a>(
        &'a self,
        statements: &'a [String],
    ) -> core::pin::Pin<Box<dyn core::future::Future<Output = Result<()>> + Send + 'a>> {
        Box::pin(async move { self.inner.exec_batch_transactional(statements).await })
    }

    fn exec_remote_batch_transactional<'a>(
        &'a self,
        statements: &'a [String],
    ) -> core::pin::Pin<Box<dyn core::future::Future<Output = Result<()>> + Send + 'a>> {
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
    ) -> core::pin::Pin<Box<dyn core::future::Future<Output = Result<Vec<Value>>> + Send + 'a>>
    {
        Box::pin(async move { self.inner.query_rows(sql).await })
    }

    fn exec_batch_transactional_for_role<'a>(
        &'a self,
        role: RelationalStorageRole,
        statements: &'a [String],
    ) -> core::pin::Pin<Box<dyn core::future::Future<Output = Result<()>> + Send + 'a>> {
        Box::pin(async move {
            self.inner
                .exec_batch_transactional_for_role(role, statements)
                .await
        })
    }

    fn query_rows_for_role<'a>(
        &'a self,
        role: RelationalStorageRole,
        sql: &'a str,
    ) -> core::pin::Pin<Box<dyn core::future::Future<Output = Result<Vec<Value>>> + Send + 'a>>
    {
        Box::pin(async move { self.inner.query_rows_for_role(role, sql).await })
    }
}
