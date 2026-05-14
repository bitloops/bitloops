use super::*;
use crate::capability_packs::architecture_graph::storage::ArchitectureGraphFacts;
use crate::config::{
    resolve_bound_daemon_config_root_for_repo, resolve_bound_store_backend_config_for_repo,
};
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

#[derive(Debug, Clone)]
pub struct RepoIdentity {
    pub(crate) provider: String,
    pub(crate) organization: String,
    pub(crate) name: String,
    pub(crate) identity: String,
    pub(crate) repo_id: String,
}

#[derive(Debug, Clone)]
pub struct DevqlConfig {
    pub(crate) daemon_config_root: PathBuf,
    pub(crate) repo_root: PathBuf,
    pub(crate) repo: RepoIdentity,
    pub(crate) pg_dsn: Option<String>,
    pub(crate) clickhouse_url: String,
    pub(crate) clickhouse_user: Option<String>,
    pub(crate) clickhouse_password: Option<String>,
    pub(crate) clickhouse_database: String,
}

impl DevqlConfig {
    pub fn from_env(repo_root: PathBuf, repo: RepoIdentity) -> Result<Self> {
        let daemon_config_root = resolve_bound_daemon_config_root_for_repo(&repo_root)?;
        let backend_cfg = resolve_bound_store_backend_config_for_repo(&repo_root)
            .context("resolving backend config for DevQL runtime")?;
        Ok(Self {
            daemon_config_root,
            repo_root,
            repo,
            pg_dsn: backend_cfg.relational.postgres_dsn,
            clickhouse_url: backend_cfg
                .events
                .clickhouse_url
                .unwrap_or_else(|| "http://localhost:8123".to_string()),
            clickhouse_user: backend_cfg.events.clickhouse_user,
            clickhouse_password: backend_cfg.events.clickhouse_password,
            clickhouse_database: backend_cfg
                .events
                .clickhouse_database
                .unwrap_or_else(|| "default".to_string()),
        })
    }

    pub fn from_roots(
        daemon_config_root: PathBuf,
        repo_root: PathBuf,
        repo: RepoIdentity,
    ) -> Result<Self> {
        let backend_cfg = resolve_store_backend_config_for_repo(&daemon_config_root)
            .context("resolving backend config for DevQL runtime")?;
        Ok(Self {
            daemon_config_root,
            repo_root,
            repo,
            pg_dsn: backend_cfg.relational.postgres_dsn,
            clickhouse_url: backend_cfg
                .events
                .clickhouse_url
                .unwrap_or_else(|| "http://localhost:8123".to_string()),
            clickhouse_user: backend_cfg.events.clickhouse_user,
            clickhouse_password: backend_cfg.events.clickhouse_password,
            clickhouse_database: backend_cfg
                .events
                .clickhouse_database
                .unwrap_or_else(|| "default".to_string()),
        })
    }

    pub(super) fn clickhouse_endpoint(&self) -> String {
        let base = self.clickhouse_url.trim_end_matches('/');
        format!("{base}/?database={}", self.clickhouse_database)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelationalDialect {
    Postgres,
    Sqlite,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelationalPrimaryBackend {
    Postgres,
    Sqlite,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelationalStorageRole {
    CurrentProjection,
    SharedRelational,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelationalRoleBackend {
    LocalSqlite,
    Postgres,
}

#[derive(Debug)]
pub struct SqliteStorage {
    pub path: PathBuf,
}

#[derive(Debug)]
pub struct PostgresStorage {
    pub client: tokio_postgres::Client,
}

#[derive(Debug)]
pub struct RelationalStorage {
    pub local: SqliteStorage,
    pub remote: Option<PostgresStorage>,
    remote_dsn: Option<String>,
    primary_backend: RelationalPrimaryBackend,
}

fn relational_authority_registry() -> &'static Mutex<HashMap<PathBuf, bool>> {
    static REGISTRY: OnceLock<Mutex<HashMap<PathBuf, bool>>> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

fn register_shared_relational_authority(path: &Path, shared_remote: bool) {
    if let Ok(mut registry) = relational_authority_registry().lock() {
        registry.insert(path.to_path_buf(), shared_remote);
    }
}

pub(crate) fn sqlite_path_uses_remote_shared_relational_authority(path: &Path) -> bool {
    relational_authority_registry()
        .lock()
        .ok()
        .and_then(|registry| registry.get(path).copied())
        .unwrap_or(false)
}

impl RelationalStorage {
    pub(crate) async fn connect(
        cfg: &DevqlConfig,
        relational: &RelationalBackendConfig,
        command: &str,
    ) -> Result<Self> {
        let sqlite_path = relational
            .resolve_sqlite_db_path_for_repo(&cfg.repo_root)
            .with_context(|| format!("resolving SQLite path for `{command}`"))?;
        let remote_dsn = relational
            .postgres_dsn
            .as_deref()
            .or(cfg.pg_dsn.as_deref())
            .map(str::trim)
            .filter(|dsn| !dsn.is_empty());
        let remote = if let Some(dsn) = remote_dsn {
            let client = connect_postgres_client(dsn).await?;
            Some(PostgresStorage { client })
        } else {
            None
        };

        Ok(Self {
            local: SqliteStorage { path: sqlite_path },
            remote,
            remote_dsn: remote_dsn.map(ToOwned::to_owned),
            primary_backend: if remote_dsn.is_some() {
                RelationalPrimaryBackend::Postgres
            } else {
                RelationalPrimaryBackend::Sqlite
            },
        })
        .map(|storage| {
            register_shared_relational_authority(
                &storage.local.path,
                storage.primary_backend == RelationalPrimaryBackend::Postgres,
            );
            storage
        })
    }

    pub fn local_only(path: PathBuf) -> Self {
        let storage = Self {
            local: SqliteStorage { path },
            remote: None,
            remote_dsn: None,
            primary_backend: RelationalPrimaryBackend::Sqlite,
        };
        register_shared_relational_authority(&storage.local.path, false);
        storage
    }

    pub fn configured_primary(path: PathBuf, postgres_dsn: Option<String>) -> Self {
        let remote_dsn = postgres_dsn
            .map(|dsn| dsn.trim().to_string())
            .filter(|dsn| !dsn.is_empty());
        let primary_backend = if remote_dsn.is_some() {
            RelationalPrimaryBackend::Postgres
        } else {
            RelationalPrimaryBackend::Sqlite
        };

        let storage = Self {
            local: SqliteStorage { path },
            remote: None,
            remote_dsn,
            primary_backend,
        };
        register_shared_relational_authority(
            &storage.local.path,
            storage.primary_backend == RelationalPrimaryBackend::Postgres,
        );
        storage
    }

    pub fn with_remote_client(path: PathBuf, client: tokio_postgres::Client) -> Self {
        let storage = Self {
            local: SqliteStorage { path },
            remote: Some(PostgresStorage { client }),
            remote_dsn: None,
            primary_backend: RelationalPrimaryBackend::Postgres,
        };
        register_shared_relational_authority(&storage.local.path, true);
        storage
    }

    pub fn dialect(&self) -> RelationalDialect {
        RelationalDialect::Sqlite
    }

    pub fn primary_backend(&self) -> RelationalPrimaryBackend {
        self.primary_backend
    }

    pub fn sqlite_path(&self) -> &Path {
        &self.local.path
    }

    pub fn remote_client(&self) -> Option<&tokio_postgres::Client> {
        self.remote.as_ref().map(|remote| &remote.client)
    }

    pub fn remote_dsn(&self) -> Option<&str> {
        self.remote_dsn.as_deref()
    }

    pub fn backend_for_role(&self, role: RelationalStorageRole) -> RelationalRoleBackend {
        match role {
            RelationalStorageRole::CurrentProjection => RelationalRoleBackend::LocalSqlite,
            RelationalStorageRole::SharedRelational => match self.primary_backend() {
                RelationalPrimaryBackend::Sqlite => RelationalRoleBackend::LocalSqlite,
                RelationalPrimaryBackend::Postgres => RelationalRoleBackend::Postgres,
            },
        }
    }

    pub fn dialect_for_role(&self, role: RelationalStorageRole) -> RelationalDialect {
        match self.backend_for_role(role) {
            RelationalRoleBackend::LocalSqlite => RelationalDialect::Sqlite,
            RelationalRoleBackend::Postgres => RelationalDialect::Postgres,
        }
    }

    pub fn has_remote_shared_relational_authority(&self) -> bool {
        self.backend_for_role(RelationalStorageRole::SharedRelational)
            == RelationalRoleBackend::Postgres
    }

    pub async fn exec(&self, sql: &str) -> Result<()> {
        sqlite_exec_path(self.sqlite_path(), sql).await
    }

    pub async fn exec_batch_transactional(&self, statements: &[String]) -> Result<()> {
        sqlite_exec_batch_transactional_path(self.sqlite_path(), statements).await
    }

    pub async fn exec_batch_transactional_for_role(
        &self,
        role: RelationalStorageRole,
        statements: &[String],
    ) -> Result<()> {
        match self.backend_for_role(role) {
            RelationalRoleBackend::LocalSqlite => self.exec_batch_transactional(statements).await,
            RelationalRoleBackend::Postgres => {
                self.exec_remote_batch_transactional(statements).await
            }
        }
    }

    pub async fn exec_for_role(&self, role: RelationalStorageRole, sql: &str) -> Result<()> {
        self.exec_batch_transactional_for_role(role, &[sql.to_string()])
            .await
    }

    pub async fn exec_serialized(&self, sql: &str) -> Result<()> {
        super::sqlite_write_actor::sqlite_exec_serialized_path(self.sqlite_path(), sql).await
    }

    pub async fn exec_serialized_batch_transactional(&self, statements: &[String]) -> Result<()> {
        super::sqlite_write_actor::sqlite_exec_serialized_batch_transactional_path(
            self.sqlite_path(),
            statements,
        )
        .await
    }

    pub async fn replace_architecture_graph_current(
        &self,
        repo_id: &str,
        facts: ArchitectureGraphFacts,
        generation_seq: u64,
        warnings: &[String],
        metrics: Value,
    ) -> Result<()> {
        super::sqlite_write_actor::sqlite_replace_architecture_graph_current_path(
            self.sqlite_path(),
            repo_id,
            facts,
            generation_seq,
            warnings,
            metrics,
        )
        .await
    }

    pub async fn exec_remote_batch_transactional(&self, statements: &[String]) -> Result<()> {
        if let Some(remote_client) = self.remote_client() {
            return postgres_exec_batch_transactional(remote_client, statements).await;
        }
        bail!("remote Postgres storage is not configured")
    }

    pub async fn exec_primary_batch_transactional(&self, statements: &[String]) -> Result<()> {
        match self.primary_backend() {
            RelationalPrimaryBackend::Sqlite => self.exec_batch_transactional(statements).await,
            RelationalPrimaryBackend::Postgres => {
                self.exec_remote_batch_transactional(statements).await
            }
        }
    }

    pub async fn query_rows(&self, sql: &str) -> Result<Vec<Value>> {
        sqlite_query_rows_path(self.sqlite_path(), sql).await
    }

    pub async fn query_rows_for_role(
        &self,
        role: RelationalStorageRole,
        sql: &str,
    ) -> Result<Vec<Value>> {
        match self.backend_for_role(role) {
            RelationalRoleBackend::LocalSqlite => self.query_rows(sql).await,
            RelationalRoleBackend::Postgres => self.query_rows_remote(sql).await,
        }
    }

    pub async fn query_rows_remote(&self, sql: &str) -> Result<Vec<Value>> {
        if let Some(remote_client) = self.remote_client() {
            return pg_query_rows(remote_client, sql).await;
        }
        bail!("remote Postgres storage is not configured")
    }

    pub async fn query_rows_primary(&self, sql: &str) -> Result<Vec<Value>> {
        match self.primary_backend() {
            RelationalPrimaryBackend::Sqlite => self.query_rows(sql).await,
            RelationalPrimaryBackend::Postgres => self.query_rows_remote(sql).await,
        }
    }

    #[cfg(test)]
    pub(crate) fn primary_backend_for_tests(
        path: PathBuf,
        primary_backend: RelationalPrimaryBackend,
    ) -> Self {
        let storage = Self {
            local: SqliteStorage { path },
            remote: None,
            remote_dsn: None,
            primary_backend,
        };
        register_shared_relational_authority(
            &storage.local.path,
            storage.primary_backend == RelationalPrimaryBackend::Postgres,
        );
        storage
    }

    #[cfg(test)]
    pub(crate) fn primary_backend_with_dsn_for_tests(
        path: PathBuf,
        primary_backend: RelationalPrimaryBackend,
        remote_dsn: Option<String>,
    ) -> Self {
        let storage = Self {
            local: SqliteStorage { path },
            remote: None,
            remote_dsn,
            primary_backend,
        };
        register_shared_relational_authority(
            &storage.local.path,
            storage.primary_backend == RelationalPrimaryBackend::Postgres,
        );
        storage
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    fn sample_cfg(repo_root: PathBuf) -> DevqlConfig {
        DevqlConfig {
            daemon_config_root: repo_root.clone(),
            repo_root,
            repo: RepoIdentity {
                provider: "git".to_string(),
                organization: "bitloops".to_string(),
                name: "bitloops".to_string(),
                identity: "git/bitloops/bitloops".to_string(),
                repo_id: "repo-1".to_string(),
            },
            pg_dsn: None,
            clickhouse_url: "http://localhost:8123".to_string(),
            clickhouse_user: None,
            clickhouse_password: None,
            clickhouse_database: "default".to_string(),
        }
    }

    #[tokio::test]
    async fn connect_always_builds_local_sqlite_storage() {
        let temp = tempfile::tempdir().expect("create temp dir");
        let sqlite_path = temp.path().join("stores").join("relational.sqlite");
        let cfg = sample_cfg(temp.path().to_path_buf());
        let backends = RelationalBackendConfig {
            sqlite_path: Some(sqlite_path.to_string_lossy().to_string()),
            postgres_dsn: None,
        };

        let relational = RelationalStorage::connect(&cfg, &backends, "devql test")
            .await
            .expect("connect relational storage");

        assert_eq!(relational.sqlite_path(), sqlite_path.as_path());
        assert!(relational.remote.is_none());
        assert_eq!(relational.dialect(), RelationalDialect::Sqlite);
        assert_eq!(
            relational.primary_backend(),
            RelationalPrimaryBackend::Sqlite
        );
    }

    #[tokio::test]
    async fn connect_fails_fast_when_postgres_dsn_is_invalid() {
        let temp = tempfile::tempdir().expect("create temp dir");
        let sqlite_path = temp.path().join("stores").join("relational.sqlite");
        let cfg = sample_cfg(temp.path().to_path_buf());
        let backends = RelationalBackendConfig {
            sqlite_path: Some(sqlite_path.to_string_lossy().to_string()),
            postgres_dsn: Some("postgres://not a valid dsn".to_string()),
        };

        let err = RelationalStorage::connect(&cfg, &backends, "devql test")
            .await
            .expect_err("invalid DSN should fail");
        assert!(
            err.to_string().contains("parsing Postgres DSN")
                || err.to_string().contains("connecting to Postgres"),
            "expected DSN connection setup to fail, got: {err:#}"
        );
    }

    #[test]
    fn primary_backend_for_tests_can_represent_postgres_without_mutating_local_dialect() {
        let temp = tempfile::tempdir().expect("create temp dir");
        let relational = RelationalStorage::primary_backend_for_tests(
            temp.path().join("stores").join("relational.sqlite"),
            RelationalPrimaryBackend::Postgres,
        );

        assert_eq!(
            relational.primary_backend(),
            RelationalPrimaryBackend::Postgres
        );
        assert_eq!(relational.dialect(), RelationalDialect::Sqlite);
    }

    #[test]
    fn explicit_role_backends_split_current_and_shared_authority() {
        let temp = tempfile::tempdir().expect("create temp dir");
        let sqlite_only = RelationalStorage::primary_backend_for_tests(
            temp.path().join("stores").join("sqlite-only.sqlite"),
            RelationalPrimaryBackend::Sqlite,
        );
        assert_eq!(
            sqlite_only.backend_for_role(RelationalStorageRole::CurrentProjection),
            RelationalRoleBackend::LocalSqlite
        );
        assert_eq!(
            sqlite_only.backend_for_role(RelationalStorageRole::SharedRelational),
            RelationalRoleBackend::LocalSqlite
        );

        let remote_shared = RelationalStorage::primary_backend_for_tests(
            temp.path().join("stores").join("shared-remote.sqlite"),
            RelationalPrimaryBackend::Postgres,
        );
        assert_eq!(
            remote_shared.backend_for_role(RelationalStorageRole::CurrentProjection),
            RelationalRoleBackend::LocalSqlite
        );
        assert_eq!(
            remote_shared.backend_for_role(RelationalStorageRole::SharedRelational),
            RelationalRoleBackend::Postgres
        );
        assert_eq!(
            remote_shared.dialect_for_role(RelationalStorageRole::CurrentProjection),
            RelationalDialect::Sqlite
        );
        assert_eq!(
            remote_shared.dialect_for_role(RelationalStorageRole::SharedRelational),
            RelationalDialect::Postgres
        );
    }

    #[tokio::test]
    async fn role_queries_keep_current_projection_local_when_shared_authority_is_remote() {
        let temp = tempfile::tempdir().expect("create temp dir");
        let sqlite_path = temp.path().join("stores").join("shared-remote.sqlite");
        crate::host::devql::sqlite_exec_path_allow_create(
            &sqlite_path,
            "CREATE TABLE local_probe(value INTEGER);
             INSERT INTO local_probe(value) VALUES (7);",
        )
        .await
        .expect("seed local sqlite probe");

        let relational = RelationalStorage::primary_backend_for_tests(
            sqlite_path,
            RelationalPrimaryBackend::Postgres,
        );

        let current_rows = relational
            .query_rows_for_role(
                RelationalStorageRole::CurrentProjection,
                "SELECT value FROM local_probe",
            )
            .await
            .expect("query current/projection rows from local sqlite");
        assert_eq!(
            current_rows
                .first()
                .and_then(|row| row.get("value"))
                .and_then(Value::as_i64),
            Some(7)
        );

        let err = relational
            .query_rows_for_role(
                RelationalStorageRole::SharedRelational,
                "SELECT value FROM local_probe",
            )
            .await
            .expect_err("shared/historical query should route remote when configured");
        assert!(
            err.to_string()
                .contains("remote Postgres storage is not configured"),
            "expected remote routing failure, got: {err:#}"
        );
    }

    #[tokio::test]
    async fn role_writes_keep_current_projection_local_when_shared_authority_is_remote() {
        let temp = tempfile::tempdir().expect("create temp dir");
        let sqlite_path = temp.path().join("stores").join("shared-remote.sqlite");
        crate::host::devql::sqlite_exec_path_allow_create(
            &sqlite_path,
            "CREATE TABLE local_probe(value INTEGER);",
        )
        .await
        .expect("create local sqlite probe");

        let relational = RelationalStorage::primary_backend_for_tests(
            sqlite_path,
            RelationalPrimaryBackend::Postgres,
        );

        relational
            .exec_batch_transactional_for_role(
                RelationalStorageRole::CurrentProjection,
                &["INSERT INTO local_probe(value) VALUES (11)".to_string()],
            )
            .await
            .expect("current/projection write should stay local");

        let err = relational
            .exec_batch_transactional_for_role(
                RelationalStorageRole::SharedRelational,
                &["INSERT INTO local_probe(value) VALUES (13)".to_string()],
            )
            .await
            .expect_err("shared/historical write should route remote when configured");
        assert!(
            err.to_string()
                .contains("remote Postgres storage is not configured"),
            "expected remote routing failure, got: {err:#}"
        );

        let persisted_rows = relational
            .query_rows("SELECT value FROM local_probe ORDER BY value")
            .await
            .expect("query persisted local rows");
        let persisted_values = persisted_rows
            .iter()
            .filter_map(|row| row.get("value").and_then(Value::as_i64))
            .collect::<Vec<_>>();
        assert_eq!(persisted_values, vec![11]);
    }

    #[tokio::test]
    async fn local_only_keeps_current_projection_and_shared_authority_local() {
        let temp = tempfile::tempdir().expect("create temp dir");
        let sqlite_path = temp.path().join("stores").join("local-only.sqlite");
        crate::host::devql::sqlite_exec_path_allow_create(
            &sqlite_path,
            "CREATE TABLE local_probe(value INTEGER);",
        )
        .await
        .expect("create local sqlite probe");

        let relational = RelationalStorage::local_only(sqlite_path);

        relational
            .exec_batch_transactional_for_role(
                RelationalStorageRole::CurrentProjection,
                &["INSERT INTO local_probe(value) VALUES (17)".to_string()],
            )
            .await
            .expect("current/projection write should stay local");
        relational
            .exec_batch_transactional_for_role(
                RelationalStorageRole::SharedRelational,
                &["INSERT INTO local_probe(value) VALUES (19)".to_string()],
            )
            .await
            .expect("shared/historical write should stay local when sqlite-only");

        let shared_rows = relational
            .query_rows_for_role(
                RelationalStorageRole::SharedRelational,
                "SELECT value FROM local_probe ORDER BY value",
            )
            .await
            .expect("shared/historical query should stay local when sqlite-only");
        let shared_values = shared_rows
            .iter()
            .filter_map(|row| row.get("value").and_then(Value::as_i64))
            .collect::<Vec<_>>();
        assert_eq!(shared_values, vec![17, 19]);
    }
}
