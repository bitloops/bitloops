use anyhow::{Context, Result};

use crate::engine::devql::capability_host::gateways::KnowledgeDocumentGateway;
use crate::engine::devql::knowledge_schema_sql_duckdb;

use super::models::{KnowledgeDocumentVersionRow, ensure_parent_dir};

pub struct DuckdbKnowledgeDocumentStore {
    path: std::path::PathBuf,
}

impl DuckdbKnowledgeDocumentStore {
    pub fn new(path: std::path::PathBuf) -> Self {
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

    pub fn has_knowledge_item_version(
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
                "SELECT knowledge_item_version_id
                 FROM knowledge_document_versions
                 WHERE knowledge_item_id = ? AND content_hash = ?
                 LIMIT 1",
            )
            .context("preparing DuckDB knowledge-item-version lookup")?;
        let mut rows = stmt
            .query(duckdb::params![knowledge_item_id, content_hash])
            .context("querying DuckDB knowledge-item-version lookup")?;
        if let Some(row) = rows
            .next()
            .context("reading DuckDB knowledge-item-version row")?
        {
            let id: String = row
                .get(0)
                .context("reading DuckDB knowledge_item_version_id")?;
            Ok(Some(id))
        } else {
            Ok(None)
        }
    }

    pub fn insert_knowledge_item_version(&self, row: &KnowledgeDocumentVersionRow) -> Result<()> {
        ensure_parent_dir(&self.path)?;
        let conn = duckdb::Connection::open(&self.path).with_context(|| {
            format!(
                "opening DuckDB knowledge database at {}",
                self.path.display()
            )
        })?;
        conn.execute(
            "INSERT INTO knowledge_document_versions (
                knowledge_item_version_id, knowledge_item_id, provider, source_kind, content_hash,
                title, state, author, updated_at, body_preview, normalized_fields_json,
                storage_backend, storage_path, payload_mime_type, payload_size_bytes,
                provenance_json
             ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            duckdb::params![
                row.knowledge_item_version_id.as_str(),
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
        .context("inserting DuckDB knowledge item version")?;
        Ok(())
    }

    pub fn delete_knowledge_item_version(&self, knowledge_item_version_id: &str) -> Result<()> {
        ensure_parent_dir(&self.path)?;
        let conn = duckdb::Connection::open(&self.path).with_context(|| {
            format!(
                "opening DuckDB knowledge database at {}",
                self.path.display()
            )
        })?;
        conn.execute(
            "DELETE FROM knowledge_document_versions WHERE knowledge_item_version_id = ?",
            duckdb::params![knowledge_item_version_id],
        )
        .context("deleting DuckDB knowledge item version")?;
        Ok(())
    }

    pub fn find_knowledge_item_version(
        &self,
        knowledge_item_version_id: &str,
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
                "SELECT knowledge_item_version_id, knowledge_item_id, provider, source_kind, content_hash,
                        title, state, author, updated_at, body_preview, normalized_fields_json,
                        storage_backend, storage_path, payload_mime_type, payload_size_bytes,
                        provenance_json, CAST(created_at AS VARCHAR)
                 FROM knowledge_document_versions
                 WHERE knowledge_item_version_id = ?
                 LIMIT 1",
            )
            .context("preparing DuckDB knowledge-item-version by id lookup")?;
        let mut rows = stmt
            .query(duckdb::params![knowledge_item_version_id])
            .context("querying DuckDB knowledge-item-version by id lookup")?;
        if let Some(row) = rows
            .next()
            .context("reading DuckDB knowledge-item-version row")?
        {
            Ok(Some(KnowledgeDocumentVersionRow {
                knowledge_item_version_id: row
                    .get(0)
                    .context("reading knowledge_item_version_id")?,
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
                created_at: row.get(16).context("reading created_at")?,
            }))
        } else {
            Ok(None)
        }
    }

    pub fn list_versions_for_item(&self, knowledge_item_id: &str) -> Result<Vec<KnowledgeDocumentVersionRow>> {
        ensure_parent_dir(&self.path)?;
        let conn = duckdb::Connection::open(&self.path).with_context(|| {
            format!(
                "opening DuckDB knowledge database at {}",
                self.path.display()
            )
        })?;
        let mut stmt = conn
            .prepare(
                "SELECT knowledge_item_version_id, knowledge_item_id, provider, source_kind, content_hash,
                        title, state, author, updated_at, body_preview, normalized_fields_json,
                        storage_backend, storage_path, payload_mime_type, payload_size_bytes,
                        provenance_json, CAST(created_at AS VARCHAR)
                 FROM knowledge_document_versions
                 WHERE knowledge_item_id = ?
                 ORDER BY created_at DESC",
            )
            .context("preparing DuckDB knowledge-item-version list by item lookup")?;
        let mut rows = stmt
            .query(duckdb::params![knowledge_item_id])
            .context("querying DuckDB knowledge-item-version list by item lookup")?;
        let mut out = Vec::new();
        while let Some(row) = rows
            .next()
            .context("reading DuckDB knowledge-item-version row")?
        {
            out.push(KnowledgeDocumentVersionRow {
                knowledge_item_version_id: row
                    .get(0)
                    .context("reading knowledge_item_version_id")?,
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
                created_at: row.get(16).context("reading created_at")?,
            });
        }
        Ok(out)
    }
}

impl KnowledgeDocumentGateway for DuckdbKnowledgeDocumentStore {
    fn initialise_schema(&self) -> Result<()> {
        DuckdbKnowledgeDocumentStore::initialise_schema(self)
    }

    fn has_knowledge_item_version(
        &self,
        knowledge_item_id: &str,
        content_hash: &str,
    ) -> Result<Option<String>> {
        DuckdbKnowledgeDocumentStore::has_knowledge_item_version(self, knowledge_item_id, content_hash)
    }

    fn insert_knowledge_item_version(&self, row: &KnowledgeDocumentVersionRow) -> Result<()> {
        DuckdbKnowledgeDocumentStore::insert_knowledge_item_version(self, row)
    }

    fn delete_knowledge_item_version(&self, knowledge_item_version_id: &str) -> Result<()> {
        DuckdbKnowledgeDocumentStore::delete_knowledge_item_version(self, knowledge_item_version_id)
    }

    fn find_knowledge_item_version(
        &self,
        knowledge_item_version_id: &str,
    ) -> Result<Option<KnowledgeDocumentVersionRow>> {
        DuckdbKnowledgeDocumentStore::find_knowledge_item_version(self, knowledge_item_version_id)
    }

    fn list_versions_for_item(&self, knowledge_item_id: &str) -> Result<Vec<KnowledgeDocumentVersionRow>> {
        DuckdbKnowledgeDocumentStore::list_versions_for_item(self, knowledge_item_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn duckdb_document_store_roundtrip_inserts_looks_up_and_deletes_version() {
        let temp = TempDir::new().expect("temp dir");
        let path = temp.path().join("knowledge-documents.duckdb");
        let store = DuckdbKnowledgeDocumentStore::new(path);
        store.initialise_schema().expect("initialise schema");
        assert!(
            store
                .has_knowledge_item_version("item-1", "hash-1")
                .expect("lookup before insert")
                .is_none()
        );

        let row = KnowledgeDocumentVersionRow {
            knowledge_item_version_id: "version-1".to_string(),
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
            created_at: None,
        };

        store
            .insert_knowledge_item_version(&row)
            .expect("insert knowledge item version");
        assert_eq!(
            store
                .has_knowledge_item_version(&row.knowledge_item_id, &row.content_hash)
                .expect("lookup after insert"),
            Some(row.knowledge_item_version_id.clone())
        );

        store
            .delete_knowledge_item_version(&row.knowledge_item_version_id)
            .expect("delete knowledge item version");
        assert!(
            store
                .has_knowledge_item_version(&row.knowledge_item_id, &row.content_hash)
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
            knowledge_item_version_id: "version-1".to_string(),
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
            created_at: None,
        };

        store
            .insert_knowledge_item_version(&row)
            .expect("insert knowledge item version");

        let found = store
            .find_knowledge_item_version(&row.knowledge_item_version_id)
            .expect("find knowledge item version by id")
            .expect("knowledge item version row");

        assert_eq!(found, row);
    }
}
