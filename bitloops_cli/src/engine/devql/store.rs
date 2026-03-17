use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use async_trait::async_trait;
use rusqlite::types::ValueRef;
use serde_json::{Map, Value};
use crate::store_config::RelationalBackendConfig;

pub(crate) const SCOPE_COMMITTED: &str = "committed";
pub(crate) const SCOPE_VISIBLE: &str = "visible";
pub(crate) const REVISION_COMMIT: &str = "commit";
pub(crate) const REVISION_TEMPORARY: &str = "temporary";

// ---------------------------------------------------------------------------
// Trait
// ---------------------------------------------------------------------------

#[async_trait]
pub(crate) trait DevqlRelationalBackend: Send + Sync {
    async fn init_schema(&self) -> Result<()>;

    async fn upsert_file_state(&self, sql: &str) -> Result<()>;
    async fn upsert_artefact(&self, sql: &str) -> Result<()>;
    async fn upsert_edge(&self, sql: &str) -> Result<()>;

    async fn upsert_current_file_state(&self, sql: &str) -> Result<()>;
    async fn upsert_current_artefact(&self, sql: &str) -> Result<()>;
    async fn upsert_current_edge(&self, sql: &str) -> Result<()>;

    async fn delete_temporary_rows(&self, session_id: &str, repo_id: &str) -> Result<()>;

    async fn query_current(&self, scope: &str, sql: &str) -> Result<Vec<Value>>;
    async fn query_historical(
        &self,
        revision_kind: &str,
        revision_id: &str,
        sql: &str,
    ) -> Result<Vec<Value>>;
}

// ---------------------------------------------------------------------------
// Backend set (local SQLite + optional remote Postgres)
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub(crate) struct RelationalStorage {
    pub(crate) local: Arc<dyn DevqlRelationalBackend>,
    pub(crate) remote: Option<Arc<dyn DevqlRelationalBackend>>,
}

impl RelationalStorage {
    pub(crate) async fn init_schema(&self) -> Result<()> {
        self.local.init_schema().await?;
        if let Some(remote) = &self.remote {
            remote.init_schema().await?;
        }
        Ok(())
    }

    pub(crate) async fn upsert_file_state(&self, sql: &str) -> Result<()> {
        self.local.upsert_file_state(sql).await?;
        if let Some(remote) = &self.remote {
            remote.upsert_file_state(sql).await?;
        }
        Ok(())
    }

    pub(crate) async fn upsert_artefact(&self, sql: &str) -> Result<()> {
        self.local.upsert_artefact(sql).await?;
        if let Some(remote) = &self.remote {
            remote.upsert_artefact(sql).await?;
        }
        Ok(())
    }

    pub(crate) async fn upsert_edge(&self, sql: &str) -> Result<()> {
        self.local.upsert_edge(sql).await?;
        if let Some(remote) = &self.remote {
            remote.upsert_edge(sql).await?;
        }
        Ok(())
    }

    pub(crate) async fn upsert_current_file_state(&self, sql: &str) -> Result<()> {
        self.local.upsert_current_file_state(sql).await?;
        if let Some(remote) = &self.remote {
            remote.upsert_current_file_state(sql).await?;
        }
        Ok(())
    }

    pub(crate) async fn upsert_current_artefact(&self, sql: &str) -> Result<()> {
        self.local.upsert_current_artefact(sql).await?;
        if let Some(remote) = &self.remote {
            remote.upsert_current_artefact(sql).await?;
        }
        Ok(())
    }

    pub(crate) async fn upsert_current_edge(&self, sql: &str) -> Result<()> {
        self.local.upsert_current_edge(sql).await?;
        if let Some(remote) = &self.remote {
            remote.upsert_current_edge(sql).await?;
        }
        Ok(())
    }

    pub(crate) async fn delete_temporary_rows(&self, session_id: &str, repo_id: &str) -> Result<()> {
        self.local.delete_temporary_rows(session_id, repo_id).await?;
        Ok(())
    }

    pub(crate) async fn query_current(&self, scope: &str, sql: &str) -> Result<Vec<Value>> {
        if scope == SCOPE_VISIBLE {
            return self.local.query_current(scope, sql).await;
        }
        if let Some(remote) = &self.remote {
            return remote.query_current(scope, sql).await;
        }
        self.local.query_current(scope, sql).await
    }

    pub(crate) async fn query_historical(
        &self,
        revision_kind: &str,
        revision_id: &str,
        sql: &str,
    ) -> Result<Vec<Value>> {
        if revision_kind == REVISION_TEMPORARY {
            return self
                .local
                .query_historical(revision_kind, revision_id, sql)
                .await;
        }
        if let Some(remote) = &self.remote {
            return remote
                .query_historical(revision_kind, revision_id, sql)
                .await;
        }
        self.local
            .query_historical(revision_kind, revision_id, sql)
            .await
    }
}

pub(crate) fn create_devql_backends(cfg: &RelationalBackendConfig) -> Result<RelationalStorage> {
    let sqlite_path = cfg
        .resolve_sqlite_db_path()
        .map_err(|err| anyhow::anyhow!("resolving DevQL SQLite path: {err:#}"))?;

    let local: Arc<dyn DevqlRelationalBackend> = Arc::new(SqliteDevqlBackend::new(sqlite_path));

    let remote: Option<Arc<dyn DevqlRelationalBackend>> = cfg
        .postgres_dsn
        .as_deref()
        .map(|dsn| {
            if dsn.trim().is_empty() {
                bail!("configured Postgres DSN is empty");
            }
            Ok(Arc::new(PostgresDevqlBackend::new(dsn.to_string())) as Arc<dyn DevqlRelationalBackend>)
        })
        .transpose()?;

    Ok(RelationalStorage { local, remote })
}

// ---------------------------------------------------------------------------
// SQLite implementation
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub(crate) struct SqliteDevqlBackend {
    db_path: PathBuf,
}

impl SqliteDevqlBackend {
    pub(crate) fn new(db_path: PathBuf) -> Self {
        Self { db_path }
    }

    fn execute_batch(&self, sql: &str) -> Result<()> {
        let sql = self.translate_sql(sql);
        let sqlite = crate::engine::db::SqliteConnectionPool::connect(self.db_path.clone())?;
        sqlite.with_connection(|conn| {
            conn.execute_batch(&sql)
                .context("executing SQLite statements")?;
            Ok(())
        })
    }

    fn query_rows(&self, sql: &str) -> Result<Vec<Value>> {
        let sql = self.translate_sql(sql);
        let sqlite = crate::engine::db::SqliteConnectionPool::connect(self.db_path.clone())?;
        sqlite.with_connection(|conn| {
            let mut stmt = conn.prepare(&sql).context("preparing SQLite query")?;
            let col_count = stmt.column_count();
            let col_names = (0..col_count)
                .map(|idx| stmt.column_name(idx).unwrap_or("col").to_string())
                .collect::<Vec<_>>();
            let mut rows = stmt.query([]).context("executing SQLite query")?;
            let mut out = Vec::new();

            while let Some(row) = rows.next().context("reading SQLite row")? {
                let mut map = Map::new();
                for (idx, name) in col_names.iter().enumerate() {
                    let raw = row.get_ref(idx).context("reading SQLite column")?;
                    let value = match raw {
                        ValueRef::Null => Value::Null,
                        ValueRef::Integer(v) => Value::Number(v.into()),
                        ValueRef::Real(v) => serde_json::Number::from_f64(v)
                            .map(Value::Number)
                            .unwrap_or(Value::Null),
                        ValueRef::Text(bytes) => {
                            Value::String(String::from_utf8_lossy(bytes).to_string())
                        }
                        ValueRef::Blob(bytes) => Value::String(format!("{bytes:?}")),
                    };
                    map.insert(name.clone(), value);
                }
                out.push(Value::Object(map));
            }

            Ok(out)
        })
    }

    fn translate_sql(&self, sql: &str) -> String {
        let mut out = sql.to_string();
        out = out.replace("::jsonb", "");
        out = out.replace("::BIGINT", "");
        out = out.replace("now()", "datetime('now')");
        out = out.replace("EXTRACT(EPOCH FROM committed_at)", "strftime('%s', committed_at)");

        // Postgres to_timestamp(unix) -> SQLite datetime(unixepoch)
        while let Some(start) = out.find("to_timestamp(") {
            let rest = &out[start + "to_timestamp(".len()..];
            let Some(end_rel) = rest.find(')') else {
                break;
            };
            let expr = rest[..end_rel].trim();
            let replacement = format!("datetime({expr}, 'unixepoch')");
            out.replace_range(start..start + "to_timestamp(".len() + end_rel + 1, &replacement);
        }

        out
    }
}

#[async_trait]
impl DevqlRelationalBackend for SqliteDevqlBackend {
    async fn init_schema(&self) -> Result<()> {
        self.execute_batch(crate::engine::devql::devql_schema_sql_sqlite())
            .context("initialising SQLite DevQL schema")
    }

    async fn upsert_file_state(&self, sql: &str) -> Result<()> {
        self.execute_batch(sql)
    }

    async fn upsert_artefact(&self, sql: &str) -> Result<()> {
        self.execute_batch(sql)
    }

    async fn upsert_edge(&self, sql: &str) -> Result<()> {
        self.execute_batch(sql)
    }

    async fn upsert_current_file_state(&self, sql: &str) -> Result<()> {
        self.execute_batch(sql)
    }

    async fn upsert_current_artefact(&self, sql: &str) -> Result<()> {
        self.execute_batch(sql)
    }

    async fn upsert_current_edge(&self, sql: &str) -> Result<()> {
        self.execute_batch(sql)
    }

    async fn delete_temporary_rows(&self, session_id: &str, repo_id: &str) -> Result<()> {
        let session_sql = session_id.replace('\'', "''");
        let repo_sql = repo_id.replace('\'', "''");
        let sql = format!(
            "DELETE FROM file_state WHERE repo_id = '{repo_sql}' AND revision_kind = 'temporary';\n\
             DELETE FROM artefacts WHERE repo_id = '{repo_sql}' AND revision_kind = 'temporary';\n\
             DELETE FROM artefact_edges WHERE repo_id = '{repo_sql}' AND revision_kind = 'temporary';\n\
             DELETE FROM current_file_state WHERE repo_id = '{repo_sql}' AND current_scope = 'visible' AND revision_kind = 'temporary';\n\
             DELETE FROM artefacts_current WHERE repo_id = '{repo_sql}' AND current_scope = 'visible' AND revision_kind = 'temporary';\n\
             DELETE FROM artefact_edges_current WHERE repo_id = '{repo_sql}' AND current_scope = 'visible' AND revision_kind = 'temporary';\n\
             DELETE FROM temporary_checkpoints WHERE repo_id = '{repo_sql}' AND session_id = '{session_sql}';\n\
             INSERT INTO current_file_state (repo_id, path, current_scope, revision_kind, revision_id, tree_hash, commit_sha, base_commit_sha, blob_sha, committed_at, updated_at)\n\
               SELECT repo_id, path, 'visible', revision_kind, revision_id, tree_hash, commit_sha, base_commit_sha, blob_sha, committed_at, updated_at\n\
               FROM current_file_state WHERE repo_id = '{repo_sql}' AND current_scope = 'committed'\n\
               ON CONFLICT (repo_id, path, current_scope) DO UPDATE SET revision_kind = excluded.revision_kind, revision_id = excluded.revision_id, tree_hash = excluded.tree_hash, commit_sha = excluded.commit_sha, base_commit_sha = excluded.base_commit_sha, blob_sha = excluded.blob_sha, committed_at = excluded.committed_at, updated_at = excluded.updated_at;\n\
             INSERT INTO artefacts_current (repo_id, symbol_id, current_scope, revision_kind, revision_id, tree_hash, commit_sha, base_commit_sha, artefact_id, blob_sha, path, language, canonical_kind, language_kind, symbol_fqn, parent_symbol_id, parent_artefact_id, start_line, end_line, start_byte, end_byte, signature, modifiers, docstring, content_hash, updated_at)\n\
               SELECT repo_id, symbol_id, 'visible', revision_kind, revision_id, tree_hash, commit_sha, base_commit_sha, artefact_id, blob_sha, path, language, canonical_kind, language_kind, symbol_fqn, parent_symbol_id, parent_artefact_id, start_line, end_line, start_byte, end_byte, signature, modifiers, docstring, content_hash, updated_at\n\
               FROM artefacts_current WHERE repo_id = '{repo_sql}' AND current_scope = 'committed'\n\
               ON CONFLICT (repo_id, symbol_id, current_scope) DO UPDATE SET revision_kind = excluded.revision_kind, revision_id = excluded.revision_id, tree_hash = excluded.tree_hash, commit_sha = excluded.commit_sha, base_commit_sha = excluded.base_commit_sha, artefact_id = excluded.artefact_id, blob_sha = excluded.blob_sha, path = excluded.path, language = excluded.language, canonical_kind = excluded.canonical_kind, language_kind = excluded.language_kind, symbol_fqn = excluded.symbol_fqn, parent_symbol_id = excluded.parent_symbol_id, parent_artefact_id = excluded.parent_artefact_id, start_line = excluded.start_line, end_line = excluded.end_line, start_byte = excluded.start_byte, end_byte = excluded.end_byte, signature = excluded.signature, modifiers = excluded.modifiers, docstring = excluded.docstring, content_hash = excluded.content_hash, updated_at = excluded.updated_at;\n\
             INSERT INTO artefact_edges_current (edge_id, repo_id, current_scope, revision_kind, revision_id, tree_hash, commit_sha, base_commit_sha, blob_sha, path, from_symbol_id, from_artefact_id, to_symbol_id, to_artefact_id, to_symbol_ref, edge_kind, language, start_line, end_line, metadata, updated_at)\n\
               SELECT edge_id, repo_id, 'visible', revision_kind, revision_id, tree_hash, commit_sha, base_commit_sha, blob_sha, path, from_symbol_id, from_artefact_id, to_symbol_id, to_artefact_id, to_symbol_ref, edge_kind, language, start_line, end_line, metadata, updated_at\n\
               FROM artefact_edges_current WHERE repo_id = '{repo_sql}' AND current_scope = 'committed'\n\
               ON CONFLICT (edge_id, current_scope) DO UPDATE SET revision_kind = excluded.revision_kind, revision_id = excluded.revision_id, tree_hash = excluded.tree_hash, commit_sha = excluded.commit_sha, base_commit_sha = excluded.base_commit_sha, blob_sha = excluded.blob_sha, path = excluded.path, from_symbol_id = excluded.from_symbol_id, from_artefact_id = excluded.from_artefact_id, to_symbol_id = excluded.to_symbol_id, to_artefact_id = excluded.to_artefact_id, to_symbol_ref = excluded.to_symbol_ref, edge_kind = excluded.edge_kind, language = excluded.language, start_line = excluded.start_line, end_line = excluded.end_line, metadata = excluded.metadata, updated_at = excluded.updated_at;"
        );
        self.execute_batch(&sql)
    }

    async fn query_current(&self, scope: &str, sql: &str) -> Result<Vec<Value>> {
        if scope != SCOPE_VISIBLE && scope != SCOPE_COMMITTED {
            anyhow::bail!("unsupported current scope `{scope}`");
        }
        self.query_rows(sql)
    }

    async fn query_historical(
        &self,
        revision_kind: &str,
        _revision_id: &str,
        sql: &str,
    ) -> Result<Vec<Value>> {
        if revision_kind != REVISION_TEMPORARY && revision_kind != REVISION_COMMIT {
            anyhow::bail!("unsupported revision kind `{revision_kind}`");
        }
        self.query_rows(sql)
    }
}

// ---------------------------------------------------------------------------
// Postgres implementation
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub(crate) struct PostgresDevqlBackend {
    dsn: String,
}

impl PostgresDevqlBackend {
    pub(crate) fn new(dsn: String) -> Self {
        Self { dsn }
    }

    async fn connect(&self) -> Result<tokio_postgres::Client> {
        crate::engine::db::postgres::connect_postgres_client(&self.dsn).await
    }

    async fn execute_batch(&self, sql: &str) -> Result<()> {
        use std::time::Duration;
        let client = self.connect().await?;
        tokio::time::timeout(Duration::from_secs(30), client.batch_execute(sql))
            .await
            .context("Postgres statement timeout after 30s")?
            .context("executing Postgres DevQL statements")?;
        Ok(())
    }

    async fn query_rows(&self, sql: &str) -> Result<Vec<Value>> {
        use std::time::Duration;
        let client = self.connect().await?;
        let wrapped = format!(
            "SELECT coalesce(json_agg(t), '[]'::json)::text FROM ({}) t",
            sql.trim().trim_end_matches(';')
        );
        let row = tokio::time::timeout(Duration::from_secs(30), client.query_one(&wrapped, &[]))
            .await
            .context("Postgres query timeout after 30s")?
            .context("executing Postgres DevQL query")?;
        let raw: String = row
            .try_get(0)
            .context("reading Postgres DevQL JSON query result")?;
        let parsed: Value = serde_json::from_str(raw.trim())
            .with_context(|| format!("parsing Postgres DevQL JSON payload failed: {raw}"))?;
        match parsed {
            Value::Array(rows) => Ok(rows),
            Value::Object(_) => Ok(vec![parsed]),
            Value::Null => Ok(vec![]),
            other => bail!("unexpected Postgres JSON payload type: {other}"),
        }
    }

    fn reject_temporary_or_visible(sql: &str) -> Result<()> {
        let lowered = sql.to_ascii_lowercase();
        if lowered.contains("revision_kind") && lowered.contains("'temporary'") {
            bail!("Postgres DevQL backend rejects temporary revision rows");
        }
        if lowered.contains("current_scope") && lowered.contains("'visible'") {
            bail!("Postgres DevQL backend rejects visible current-scope rows");
        }
        Ok(())
    }
}

#[async_trait]
impl DevqlRelationalBackend for PostgresDevqlBackend {
    async fn init_schema(&self) -> Result<()> {
        self.execute_batch(crate::engine::devql::postgres_schema_sql())
            .await
            .context("creating Postgres DevQL tables")?;
        self.execute_batch(crate::engine::devql::artefacts_upgrade_sql())
            .await
            .context("updating Postgres artefacts columns for byte offsets/signature")?;
        self.execute_batch(crate::engine::devql::artefact_edges_hardening_sql())
            .await
            .context("updating Postgres artefact_edges constraints/indexes")?;
        self.execute_batch(crate::engine::devql::current_state_hardening_sql())
            .await
            .context("updating Postgres current-state DevQL tables")
    }

    async fn upsert_file_state(&self, sql: &str) -> Result<()> {
        Self::reject_temporary_or_visible(sql)?;
        self.execute_batch(sql).await
    }

    async fn upsert_artefact(&self, sql: &str) -> Result<()> {
        Self::reject_temporary_or_visible(sql)?;
        self.execute_batch(sql).await
    }

    async fn upsert_edge(&self, sql: &str) -> Result<()> {
        Self::reject_temporary_or_visible(sql)?;
        self.execute_batch(sql).await
    }

    async fn upsert_current_file_state(&self, sql: &str) -> Result<()> {
        Self::reject_temporary_or_visible(sql)?;
        self.execute_batch(sql).await
    }

    async fn upsert_current_artefact(&self, sql: &str) -> Result<()> {
        Self::reject_temporary_or_visible(sql)?;
        self.execute_batch(sql).await
    }

    async fn upsert_current_edge(&self, sql: &str) -> Result<()> {
        Self::reject_temporary_or_visible(sql)?;
        self.execute_batch(sql).await
    }

    async fn delete_temporary_rows(&self, _session_id: &str, _repo_id: &str) -> Result<()> {
        Ok(())
    }

    async fn query_current(&self, scope: &str, sql: &str) -> Result<Vec<Value>> {
        if scope != SCOPE_COMMITTED {
            bail!("Postgres current queries only support committed scope");
        }
        self.query_rows(sql).await
    }

    async fn query_historical(
        &self,
        revision_kind: &str,
        _revision_id: &str,
        sql: &str,
    ) -> Result<Vec<Value>> {
        if revision_kind != REVISION_COMMIT {
            bail!("Postgres historical queries only support committed revisions");
        }
        self.query_rows(sql).await
    }
}
