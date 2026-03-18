use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use rusqlite::{OptionalExtension, params};
use serde_json::Value;
use sha2::Digest;

use crate::engine::blob::{BlobStore, create_blob_store_with_backend_for_repo};
use crate::engine::db::SqliteConnectionPool;
use crate::engine::devql::{
    deterministic_uuid, knowledge_schema_sql_duckdb, knowledge_schema_sql_sqlite,
};
use crate::store_config::StoreBackendConfig;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KnowledgePayloadRef {
    pub storage_backend: String,
    pub storage_path: String,
    pub mime_type: String,
    pub size_bytes: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KnowledgeSourceRow {
    pub knowledge_source_id: String,
    pub provider: String,
    pub source_kind: String,
    pub canonical_external_id: String,
    pub canonical_url: String,
    pub provenance_json: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KnowledgeItemRow {
    pub knowledge_item_id: String,
    pub repo_id: String,
    pub knowledge_source_id: String,
    pub item_kind: String,
    pub latest_document_version_id: String,
    pub provenance_json: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct KnowledgeRelationAssertionRow {
    pub relation_assertion_id: String,
    pub repo_id: String,
    pub knowledge_item_id: String,
    pub source_document_version_id: String,
    pub target_type: String,
    pub target_id: String,
    pub relation_type: String,
    pub association_method: String,
    pub confidence: f64,
    pub provenance_json: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KnowledgeDocumentVersionRow {
    pub document_version_id: String,
    pub knowledge_item_id: String,
    pub provider: String,
    pub source_kind: String,
    pub content_hash: String,
    pub title: String,
    pub state: Option<String>,
    pub author: Option<String>,
    pub updated_at: Option<String>,
    pub body_preview: Option<String>,
    pub normalized_fields_json: String,
    pub storage_backend: String,
    pub storage_path: String,
    pub payload_mime_type: String,
    pub payload_size_bytes: i64,
    pub provenance_json: String,
}

pub struct SqliteKnowledgeRelationalStore {
    sqlite: SqliteConnectionPool,
}

impl SqliteKnowledgeRelationalStore {
    pub fn new(sqlite: SqliteConnectionPool) -> Self {
        Self { sqlite }
    }

    pub fn initialise_schema(&self) -> Result<()> {
        self.sqlite
            .execute_batch(knowledge_schema_sql_sqlite())
            .context("initialising SQLite knowledge schema")
    }

    pub fn persist_ingestion(
        &self,
        source: &KnowledgeSourceRow,
        item: &KnowledgeItemRow,
    ) -> Result<()> {
        self.sqlite.with_connection(|conn| {
            conn.execute_batch("BEGIN IMMEDIATE TRANSACTION")
                .context("starting SQLite knowledge transaction")?;

            let result = (|| -> Result<()> {
                upsert_source(conn, source)?;
                upsert_item(conn, item)?;
                Ok(())
            })();

            match result {
                Ok(()) => {
                    conn.execute_batch("COMMIT")
                        .context("committing SQLite knowledge transaction")?;
                    Ok(())
                }
                Err(err) => {
                    let _ = conn.execute_batch("ROLLBACK");
                    Err(err)
                }
            }
        })
    }

    pub fn insert_relation_assertion(
        &self,
        relation: &KnowledgeRelationAssertionRow,
    ) -> Result<()> {
        self.sqlite
            .with_connection(|conn| insert_relation_assertion(conn, relation))
    }

    pub fn find_item(&self, repo_id: &str, source_id: &str) -> Result<Option<KnowledgeItemRow>> {
        self.sqlite.with_connection(|conn| {
            conn.query_row(
                "SELECT knowledge_item_id, repo_id, knowledge_source_id, item_kind, latest_document_version_id, provenance_json
                 FROM knowledge_items
                 WHERE repo_id = ?1 AND knowledge_source_id = ?2
                 LIMIT 1",
                params![repo_id, source_id],
                |row| {
                    Ok(KnowledgeItemRow {
                        knowledge_item_id: row.get(0)?,
                        repo_id: row.get(1)?,
                        knowledge_source_id: row.get(2)?,
                        item_kind: row.get(3)?,
                        latest_document_version_id: row.get(4)?,
                        provenance_json: row.get(5)?,
                    })
                },
            )
            .optional()
            .map_err(anyhow::Error::from)
        })
    }

    pub fn find_item_by_id(
        &self,
        repo_id: &str,
        knowledge_item_id: &str,
    ) -> Result<Option<KnowledgeItemRow>> {
        self.sqlite.with_connection(|conn| {
            conn.query_row(
                "SELECT knowledge_item_id, repo_id, knowledge_source_id, item_kind, latest_document_version_id, provenance_json
                 FROM knowledge_items
                 WHERE repo_id = ?1 AND knowledge_item_id = ?2
                 LIMIT 1",
                params![repo_id, knowledge_item_id],
                |row| {
                    Ok(KnowledgeItemRow {
                        knowledge_item_id: row.get(0)?,
                        repo_id: row.get(1)?,
                        knowledge_source_id: row.get(2)?,
                        item_kind: row.get(3)?,
                        latest_document_version_id: row.get(4)?,
                        provenance_json: row.get(5)?,
                    })
                },
            )
            .optional()
            .map_err(anyhow::Error::from)
        })
    }
}

pub struct DuckdbKnowledgeDocumentStore {
    path: PathBuf,
}

impl DuckdbKnowledgeDocumentStore {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    pub fn initialise_schema(&self) -> Result<()> {
        ensure_parent_dir(&self.path)?;
        let conn = duckdb::Connection::open(&self.path).with_context(|| {
            format!(
                "opening DuckDB knowledge database at {}",
                self.path.display()
            )
        })?;
        conn.execute_batch(knowledge_schema_sql_duckdb())
            .context("initialising DuckDB knowledge schema")
    }

    pub fn has_document_version(
        &self,
        knowledge_item_id: &str,
        content_hash: &str,
    ) -> Result<Option<String>> {
        ensure_parent_dir(&self.path)?;
        let conn = duckdb::Connection::open(&self.path).with_context(|| {
            format!(
                "opening DuckDB knowledge database at {}",
                self.path.display()
            )
        })?;
        let mut stmt = conn
            .prepare(
                "SELECT document_version_id
                 FROM knowledge_document_versions
                 WHERE knowledge_item_id = ? AND content_hash = ?
                 LIMIT 1",
            )
            .context("preparing DuckDB document-version lookup")?;
        let mut rows = stmt
            .query(duckdb::params![knowledge_item_id, content_hash])
            .context("querying DuckDB document-version lookup")?;
        if let Some(row) = rows.next().context("reading DuckDB document-version row")? {
            let id: String = row.get(0).context("reading DuckDB document_version_id")?;
            Ok(Some(id))
        } else {
            Ok(None)
        }
    }

    pub fn insert_document_version(&self, row: &KnowledgeDocumentVersionRow) -> Result<()> {
        ensure_parent_dir(&self.path)?;
        let conn = duckdb::Connection::open(&self.path).with_context(|| {
            format!(
                "opening DuckDB knowledge database at {}",
                self.path.display()
            )
        })?;
        conn.execute(
            "INSERT INTO knowledge_document_versions (
                document_version_id, knowledge_item_id, provider, source_kind, content_hash,
                title, state, author, updated_at, body_preview, normalized_fields_json,
                storage_backend, storage_path, payload_mime_type, payload_size_bytes,
                provenance_json
             ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            duckdb::params![
                row.document_version_id.as_str(),
                row.knowledge_item_id.as_str(),
                row.provider.as_str(),
                row.source_kind.as_str(),
                row.content_hash.as_str(),
                row.title.as_str(),
                row.state.as_deref(),
                row.author.as_deref(),
                row.updated_at.as_deref(),
                row.body_preview.as_deref(),
                row.normalized_fields_json.as_str(),
                row.storage_backend.as_str(),
                row.storage_path.as_str(),
                row.payload_mime_type.as_str(),
                row.payload_size_bytes,
                row.provenance_json.as_str(),
            ],
        )
        .context("inserting DuckDB knowledge document version")?;
        Ok(())
    }

    pub fn delete_document_version(&self, document_version_id: &str) -> Result<()> {
        ensure_parent_dir(&self.path)?;
        let conn = duckdb::Connection::open(&self.path).with_context(|| {
            format!(
                "opening DuckDB knowledge database at {}",
                self.path.display()
            )
        })?;
        conn.execute(
            "DELETE FROM knowledge_document_versions WHERE document_version_id = ?",
            duckdb::params![document_version_id],
        )
        .context("deleting DuckDB knowledge document version")?;
        Ok(())
    }

    pub fn find_document_version(
        &self,
        document_version_id: &str,
    ) -> Result<Option<KnowledgeDocumentVersionRow>> {
        ensure_parent_dir(&self.path)?;
        let conn = duckdb::Connection::open(&self.path).with_context(|| {
            format!(
                "opening DuckDB knowledge database at {}",
                self.path.display()
            )
        })?;
        let mut stmt = conn
            .prepare(
                "SELECT document_version_id, knowledge_item_id, provider, source_kind, content_hash,
                        title, state, author, updated_at, body_preview, normalized_fields_json,
                        storage_backend, storage_path, payload_mime_type, payload_size_bytes,
                        provenance_json
                 FROM knowledge_document_versions
                 WHERE document_version_id = ?
                 LIMIT 1",
            )
            .context("preparing DuckDB document-version by id lookup")?;
        let mut rows = stmt
            .query(duckdb::params![document_version_id])
            .context("querying DuckDB document-version by id lookup")?;
        if let Some(row) = rows.next().context("reading DuckDB document-version row")? {
            Ok(Some(KnowledgeDocumentVersionRow {
                document_version_id: row.get(0).context("reading document_version_id")?,
                knowledge_item_id: row.get(1).context("reading knowledge_item_id")?,
                provider: row.get(2).context("reading provider")?,
                source_kind: row.get(3).context("reading source_kind")?,
                content_hash: row.get(4).context("reading content_hash")?,
                title: row.get(5).context("reading title")?,
                state: row.get(6).context("reading state")?,
                author: row.get(7).context("reading author")?,
                updated_at: row.get(8).context("reading updated_at")?,
                body_preview: row.get(9).context("reading body_preview")?,
                normalized_fields_json: row.get(10).context("reading normalized_fields_json")?,
                storage_backend: row.get(11).context("reading storage_backend")?,
                storage_path: row.get(12).context("reading storage_path")?,
                payload_mime_type: row.get(13).context("reading payload_mime_type")?,
                payload_size_bytes: row.get(14).context("reading payload_size_bytes")?,
                provenance_json: row.get(15).context("reading provenance_json")?,
            }))
        } else {
            Ok(None)
        }
    }
}

pub struct BlobKnowledgePayloadStore {
    store: Box<dyn BlobStore>,
    backend: String,
}

impl BlobKnowledgePayloadStore {
    pub fn from_backend_config(repo_root: &Path, cfg: &StoreBackendConfig) -> Result<Self> {
        let resolved = create_blob_store_with_backend_for_repo(&cfg.blobs, repo_root)
            .context("initialising knowledge payload blob store")?;
        Ok(Self {
            store: resolved.store,
            backend: resolved.backend.to_string(),
        })
    }

    pub fn write_payload(
        &self,
        repo_id: &str,
        knowledge_item_id: &str,
        document_version_id: &str,
        bytes: &[u8],
    ) -> Result<KnowledgePayloadRef> {
        let storage_path = knowledge_payload_key(repo_id, knowledge_item_id, document_version_id);
        self.store
            .write(&storage_path, bytes)
            .context("writing knowledge payload blob")?;
        Ok(KnowledgePayloadRef {
            storage_backend: self.backend.clone(),
            storage_path,
            mime_type: "application/json".to_string(),
            size_bytes: bytes.len() as i64,
        })
    }

    pub fn delete_payload(&self, payload: &KnowledgePayloadRef) -> Result<()> {
        self.store
            .delete(&payload.storage_path)
            .context("deleting knowledge payload blob")
    }

    pub fn payload_exists(&self, storage_path: &str) -> Result<bool> {
        self.store
            .exists(storage_path)
            .context("checking knowledge payload blob existence")
    }
}

pub fn knowledge_payload_key(
    repo_id: &str,
    knowledge_item_id: &str,
    document_version_id: &str,
) -> String {
    format!("knowledge/{repo_id}/{knowledge_item_id}/{document_version_id}/payload.json")
}

pub fn knowledge_source_id(canonical_external_id: &str) -> String {
    deterministic_uuid(&format!("knowledge-source://{canonical_external_id}"))
}

pub fn knowledge_item_id(repo_id: &str, knowledge_source_id: &str) -> String {
    deterministic_uuid(&format!("knowledge-item://{repo_id}/{knowledge_source_id}"))
}

pub fn document_version_id(knowledge_item_id: &str, content_hash: &str) -> String {
    deterministic_uuid(&format!(
        "knowledge-version://{knowledge_item_id}/{content_hash}"
    ))
}

pub fn relation_assertion_id(
    knowledge_item_id: &str,
    source_document_version_id: &str,
    target_type: &str,
    target_id: &str,
    association_method: &str,
) -> String {
    deterministic_uuid(&format!(
        "knowledge-relation://{knowledge_item_id}/{source_document_version_id}/{target_type}/{target_id}/{association_method}"
    ))
}

pub fn serialize_payload(payload: &Value) -> Result<Vec<u8>> {
    serde_json::to_vec(payload).context("serialising knowledge payload JSON")
}

pub fn content_hash(bytes: &[u8]) -> String {
    let digest = sha2::Sha256::digest(bytes);
    format!("{digest:x}")
}

fn upsert_source(conn: &rusqlite::Connection, source: &KnowledgeSourceRow) -> Result<()> {
    conn.execute(
        "INSERT INTO knowledge_sources (
            knowledge_source_id, provider, source_kind, canonical_external_id, canonical_url,
            provenance_json, created_at, updated_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, datetime('now'), datetime('now'))
         ON CONFLICT(knowledge_source_id) DO UPDATE SET
            provider = excluded.provider,
            source_kind = excluded.source_kind,
            canonical_external_id = excluded.canonical_external_id,
            canonical_url = excluded.canonical_url,
            provenance_json = excluded.provenance_json,
            updated_at = datetime('now')",
        params![
            source.knowledge_source_id.as_str(),
            source.provider.as_str(),
            source.source_kind.as_str(),
            source.canonical_external_id.as_str(),
            source.canonical_url.as_str(),
            source.provenance_json.as_str(),
        ],
    )
    .context("upserting SQLite knowledge source")?;
    Ok(())
}

fn upsert_item(conn: &rusqlite::Connection, item: &KnowledgeItemRow) -> Result<()> {
    conn.execute(
        "INSERT INTO knowledge_items (
            knowledge_item_id, repo_id, knowledge_source_id, item_kind, latest_document_version_id,
            provenance_json, created_at, updated_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, datetime('now'), datetime('now'))
         ON CONFLICT(knowledge_item_id) DO UPDATE SET
            latest_document_version_id = excluded.latest_document_version_id,
            provenance_json = excluded.provenance_json,
            updated_at = datetime('now')",
        params![
            item.knowledge_item_id.as_str(),
            item.repo_id.as_str(),
            item.knowledge_source_id.as_str(),
            item.item_kind.as_str(),
            item.latest_document_version_id.as_str(),
            item.provenance_json.as_str(),
        ],
    )
    .context("upserting SQLite knowledge item")?;
    Ok(())
}

fn insert_relation_assertion(
    conn: &rusqlite::Connection,
    relation: &KnowledgeRelationAssertionRow,
) -> Result<()> {
    conn.execute(
        "INSERT OR IGNORE INTO knowledge_relation_assertions (
            relation_assertion_id, repo_id, knowledge_item_id, source_document_version_id,
            target_type, target_id, relation_type, association_method, confidence, provenance_json,
            created_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, datetime('now'))",
        params![
            relation.relation_assertion_id.as_str(),
            relation.repo_id.as_str(),
            relation.knowledge_item_id.as_str(),
            relation.source_document_version_id.as_str(),
            relation.target_type.as_str(),
            relation.target_id.as_str(),
            relation.relation_type.as_str(),
            relation.association_method.as_str(),
            relation.confidence,
            relation.provenance_json.as_str(),
        ],
    )
    .context("inserting SQLite knowledge relation assertion")?;
    Ok(())
}

fn ensure_parent_dir(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("creating parent directory {}", parent.display()))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::db::SqliteConnectionPool;
    use crate::store_config::{
        BlobStorageConfig, BlobStorageProvider, EventsBackendConfig, EventsProvider,
        RelationalBackendConfig, RelationalProvider, StoreBackendConfig,
    };
    use tempfile::TempDir;

    #[test]
    fn payload_key_is_stable() {
        assert_eq!(
            knowledge_payload_key("repo-1", "item-1", "version-1"),
            "knowledge/repo-1/item-1/version-1/payload.json"
        );
    }

    #[test]
    fn blob_payload_store_uses_local_backend() {
        let temp = TempDir::new().expect("temp dir");
        let repo_root = temp.path().join("repo");
        fs::create_dir_all(&repo_root).expect("repo root");
        let backends = StoreBackendConfig {
            relational: RelationalBackendConfig {
                provider: RelationalProvider::Sqlite,
                sqlite_path: Some(
                    temp.path()
                        .join("relational.db")
                        .to_string_lossy()
                        .to_string(),
                ),
                postgres_dsn: None,
            },
            events: EventsBackendConfig {
                provider: EventsProvider::DuckDb,
                duckdb_path: Some(
                    temp.path()
                        .join("events.duckdb")
                        .to_string_lossy()
                        .to_string(),
                ),
                clickhouse_url: None,
                clickhouse_user: None,
                clickhouse_password: None,
                clickhouse_database: None,
            },
            blobs: BlobStorageConfig {
                provider: BlobStorageProvider::Local,
                local_path: Some(temp.path().join("blobs").to_string_lossy().to_string()),
                s3_bucket: None,
                s3_region: None,
                s3_access_key_id: None,
                s3_secret_access_key: None,
                gcs_bucket: None,
                gcs_credentials_path: None,
            },
        };
        let store =
            BlobKnowledgePayloadStore::from_backend_config(&repo_root, &backends).expect("store");
        let payload = store
            .write_payload("repo-1", "item-1", "version-1", b"{\"ok\":true}")
            .expect("write payload");

        assert!(store.payload_exists(&payload.storage_path).expect("exists"));

        store.delete_payload(&payload).expect("delete payload");
        assert!(
            !store
                .payload_exists(&payload.storage_path)
                .expect("exists after delete")
        );
    }

    #[test]
    fn sqlite_relational_store_roundtrip_persists_and_finds_item() {
        let temp = TempDir::new().expect("temp dir");
        let sqlite_path = temp.path().join("knowledge-relational.db");
        let pool = SqliteConnectionPool::connect(sqlite_path).expect("sqlite pool");
        let store = SqliteKnowledgeRelationalStore::new(pool);
        store.initialise_schema().expect("initialise schema");

        let source = KnowledgeSourceRow {
            knowledge_source_id: "source-1".to_string(),
            provider: "github".to_string(),
            source_kind: "github_issue".to_string(),
            canonical_external_id: "github://bitloops/bitloops/issues/42".to_string(),
            canonical_url: "https://github.com/bitloops/bitloops/issues/42".to_string(),
            provenance_json: "{\"provider\":\"github\"}".to_string(),
        };
        let item = KnowledgeItemRow {
            knowledge_item_id: "item-1".to_string(),
            repo_id: "repo-1".to_string(),
            knowledge_source_id: source.knowledge_source_id.clone(),
            item_kind: "github_issue".to_string(),
            latest_document_version_id: "version-1".to_string(),
            provenance_json: source.provenance_json.clone(),
        };
        store
            .persist_ingestion(&source, &item)
            .expect("persist ingestion");

        let found = store
            .find_item(&item.repo_id, &source.knowledge_source_id)
            .expect("find item")
            .expect("item row");

        assert_eq!(found, item);
    }

    #[test]
    fn sqlite_relational_store_finds_item_by_id() {
        let temp = TempDir::new().expect("temp dir");
        let sqlite_path = temp.path().join("knowledge-relational.db");
        let pool = SqliteConnectionPool::connect(sqlite_path).expect("sqlite pool");
        let store = SqliteKnowledgeRelationalStore::new(pool);
        store.initialise_schema().expect("initialise schema");

        let source = KnowledgeSourceRow {
            knowledge_source_id: "source-1".to_string(),
            provider: "github".to_string(),
            source_kind: "github_issue".to_string(),
            canonical_external_id: "github://bitloops/bitloops/issues/42".to_string(),
            canonical_url: "https://github.com/bitloops/bitloops/issues/42".to_string(),
            provenance_json: "{\"provider\":\"github\"}".to_string(),
        };
        let item = KnowledgeItemRow {
            knowledge_item_id: "item-1".to_string(),
            repo_id: "repo-1".to_string(),
            knowledge_source_id: source.knowledge_source_id.clone(),
            item_kind: "github_issue".to_string(),
            latest_document_version_id: "version-1".to_string(),
            provenance_json: source.provenance_json.clone(),
        };
        store
            .persist_ingestion(&source, &item)
            .expect("persist ingestion");

        let found = store
            .find_item_by_id(&item.repo_id, &item.knowledge_item_id)
            .expect("find item by id")
            .expect("item row");

        assert_eq!(found, item);
    }

    #[test]
    fn sqlite_relational_store_inserts_relation_assertion() {
        let temp = TempDir::new().expect("temp dir");
        let sqlite_path = temp.path().join("knowledge-relational.db");
        let pool = SqliteConnectionPool::connect(sqlite_path).expect("sqlite pool");
        let store = SqliteKnowledgeRelationalStore::new(pool);
        store.initialise_schema().expect("initialise schema");

        let relation = KnowledgeRelationAssertionRow {
            relation_assertion_id: "relation-1".to_string(),
            repo_id: "repo-1".to_string(),
            knowledge_item_id: "item-1".to_string(),
            source_document_version_id: "version-1".to_string(),
            target_type: "commit".to_string(),
            target_id: "abc123".to_string(),
            relation_type: "associated_with".to_string(),
            association_method: "manual_attachment".to_string(),
            confidence: 1.0,
            provenance_json: "{\"provider\":\"github\"}".to_string(),
        };

        store
            .insert_relation_assertion(&relation)
            .expect("insert relation assertion");
    }

    #[test]
    fn duckdb_document_store_roundtrip_inserts_looks_up_and_deletes_version() {
        let temp = TempDir::new().expect("temp dir");
        let path = temp.path().join("knowledge-documents.duckdb");
        let store = DuckdbKnowledgeDocumentStore::new(path);
        store.initialise_schema().expect("initialise schema");
        assert!(
            store
                .has_document_version("item-1", "hash-1")
                .expect("lookup before insert")
                .is_none()
        );

        let row = KnowledgeDocumentVersionRow {
            document_version_id: "version-1".to_string(),
            knowledge_item_id: "item-1".to_string(),
            provider: "github".to_string(),
            source_kind: "github_issue".to_string(),
            content_hash: "hash-1".to_string(),
            title: "Issue title".to_string(),
            state: Some("open".to_string()),
            author: Some("spiros".to_string()),
            updated_at: Some("2026-03-16T10:00:00Z".to_string()),
            body_preview: Some("Issue body".to_string()),
            normalized_fields_json: "{\"title\":\"Issue title\"}".to_string(),
            storage_backend: "local".to_string(),
            storage_path: "knowledge/repo-1/item-1/version-1/payload.json".to_string(),
            payload_mime_type: "application/json".to_string(),
            payload_size_bytes: 32,
            provenance_json: "{\"provider\":\"github\"}".to_string(),
        };

        store
            .insert_document_version(&row)
            .expect("insert document version");
        assert_eq!(
            store
                .has_document_version(&row.knowledge_item_id, &row.content_hash)
                .expect("lookup after insert"),
            Some(row.document_version_id.clone())
        );

        store
            .delete_document_version(&row.document_version_id)
            .expect("delete document version");
        assert!(
            store
                .has_document_version(&row.knowledge_item_id, &row.content_hash)
                .expect("lookup after delete")
                .is_none()
        );
    }

    #[test]
    fn duckdb_document_store_finds_document_version_by_id() {
        let temp = TempDir::new().expect("temp dir");
        let path = temp.path().join("knowledge-documents.duckdb");
        let store = DuckdbKnowledgeDocumentStore::new(path);
        store.initialise_schema().expect("initialise schema");

        let row = KnowledgeDocumentVersionRow {
            document_version_id: "version-1".to_string(),
            knowledge_item_id: "item-1".to_string(),
            provider: "github".to_string(),
            source_kind: "github_issue".to_string(),
            content_hash: "hash-1".to_string(),
            title: "Issue title".to_string(),
            state: Some("open".to_string()),
            author: Some("spiros".to_string()),
            updated_at: Some("2026-03-16T10:00:00Z".to_string()),
            body_preview: Some("Issue body".to_string()),
            normalized_fields_json: "{\"title\":\"Issue title\"}".to_string(),
            storage_backend: "local".to_string(),
            storage_path: "knowledge/repo-1/item-1/version-1/payload.json".to_string(),
            payload_mime_type: "application/json".to_string(),
            payload_size_bytes: 32,
            provenance_json: "{\"provider\":\"github\"}".to_string(),
        };

        store
            .insert_document_version(&row)
            .expect("insert document version");

        let found = store
            .find_document_version(&row.document_version_id)
            .expect("find document version by id")
            .expect("document version row");

        assert_eq!(found, row);
    }
}
