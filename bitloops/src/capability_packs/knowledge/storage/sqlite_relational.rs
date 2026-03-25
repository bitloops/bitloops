use anyhow::{Context, Result};
use rusqlite::{OptionalExtension, params};

use crate::host::devql::knowledge_schema_sql_sqlite;
use crate::storage::SqliteConnectionPool;

use super::models::{KnowledgeItemRow, KnowledgeRelationAssertionRow, KnowledgeSourceRow};
use super::relational_repository::KnowledgeRelationalRepository;

pub struct SqliteKnowledgeRelationalRepository {
    sqlite: SqliteConnectionPool,
}

impl SqliteKnowledgeRelationalRepository {
    pub fn new(sqlite: SqliteConnectionPool) -> Self {
        Self { sqlite }
    }

    pub fn initialise_schema(&self) -> Result<()> {
        self.sqlite
            .execute_batch(knowledge_schema_sql_sqlite())
            .context("initialising SQLite knowledge schema")?;
        self.ensure_relation_target_version_column()
    }

    fn ensure_relation_target_version_column(&self) -> Result<()> {
        self.sqlite.with_connection(|conn| {
            let mut stmt = conn
                .prepare("PRAGMA table_info(knowledge_relation_assertions)")
                .context("reading relation assertion table metadata")?;
            let columns = stmt
                .query_map([], |row| row.get::<_, String>(1))
                .context("querying relation assertion table metadata")?;

            let mut has_target_version_column = false;
            for column in columns {
                if column
                    .map_err(anyhow::Error::from)?
                    .eq("target_knowledge_item_version_id")
                {
                    has_target_version_column = true;
                    break;
                }
            }

            if !has_target_version_column {
                conn.execute(
                    "ALTER TABLE knowledge_relation_assertions \
                     ADD COLUMN target_knowledge_item_version_id TEXT",
                    [],
                )
                .context("adding target_knowledge_item_version_id column to relation assertions")?;
            }

            Ok(())
        })
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
            .with_connection(|conn| insert_relation_assertion_with_conn(conn, relation))
    }

    pub fn find_item(&self, repo_id: &str, source_id: &str) -> Result<Option<KnowledgeItemRow>> {
        self.sqlite.with_connection(|conn| {
            conn.query_row(
                "SELECT knowledge_item_id, repo_id, knowledge_source_id, item_kind, latest_knowledge_item_version_id, provenance_json
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
                        latest_knowledge_item_version_id: row.get(4)?,
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
                "SELECT knowledge_item_id, repo_id, knowledge_source_id, item_kind, latest_knowledge_item_version_id, provenance_json
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
                        latest_knowledge_item_version_id: row.get(4)?,
                        provenance_json: row.get(5)?,
                    })
                },
            )
            .optional()
            .map_err(anyhow::Error::from)
        })
    }

    pub fn find_source_by_id(
        &self,
        knowledge_source_id: &str,
    ) -> Result<Option<KnowledgeSourceRow>> {
        self.sqlite.with_connection(|conn| {
            conn.query_row(
                "SELECT knowledge_source_id, provider, source_kind, canonical_external_id, canonical_url, provenance_json
                 FROM knowledge_sources
                 WHERE knowledge_source_id = ?1
                 LIMIT 1",
                params![knowledge_source_id],
                |row| {
                    Ok(KnowledgeSourceRow {
                        knowledge_source_id: row.get(0)?,
                        provider: row.get(1)?,
                        source_kind: row.get(2)?,
                        canonical_external_id: row.get(3)?,
                        canonical_url: row.get(4)?,
                        provenance_json: row.get(5)?,
                    })
                },
            )
            .optional()
            .map_err(anyhow::Error::from)
        })
    }

    pub fn list_items_for_repo(
        &self,
        repo_id: &str,
        limit: usize,
    ) -> Result<Vec<KnowledgeItemRow>> {
        self.sqlite.with_connection(|conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT knowledge_item_id, repo_id, knowledge_source_id, item_kind, latest_knowledge_item_version_id, provenance_json
                     FROM knowledge_items
                     WHERE repo_id = ?1
                     ORDER BY updated_at DESC
                     LIMIT ?2",
                )
                .context("preparing SQLite knowledge items list query")?;
            let rows = stmt
                .query_map(params![repo_id, limit as i64], |row| {
                    Ok(KnowledgeItemRow {
                        knowledge_item_id: row.get(0)?,
                        repo_id: row.get(1)?,
                        knowledge_source_id: row.get(2)?,
                        item_kind: row.get(3)?,
                        latest_knowledge_item_version_id: row.get(4)?,
                        provenance_json: row.get(5)?,
                    })
                })
                .context("querying SQLite knowledge items list")?;
            let mut out = Vec::new();
            for row in rows {
                out.push(row.map_err(anyhow::Error::from)?);
            }
            Ok(out)
        })
    }
}

impl KnowledgeRelationalRepository for SqliteKnowledgeRelationalRepository {
    fn initialise_schema(&self) -> Result<()> {
        SqliteKnowledgeRelationalRepository::initialise_schema(self)
    }

    fn persist_ingestion(
        &self,
        source: &KnowledgeSourceRow,
        item: &KnowledgeItemRow,
    ) -> Result<()> {
        SqliteKnowledgeRelationalRepository::persist_ingestion(self, source, item)
    }

    fn insert_relation_assertion(&self, relation: &KnowledgeRelationAssertionRow) -> Result<()> {
        SqliteKnowledgeRelationalRepository::insert_relation_assertion(self, relation)
    }

    fn find_item(&self, repo_id: &str, source_id: &str) -> Result<Option<KnowledgeItemRow>> {
        SqliteKnowledgeRelationalRepository::find_item(self, repo_id, source_id)
    }

    fn find_item_by_id(
        &self,
        repo_id: &str,
        knowledge_item_id: &str,
    ) -> Result<Option<KnowledgeItemRow>> {
        SqliteKnowledgeRelationalRepository::find_item_by_id(self, repo_id, knowledge_item_id)
    }

    fn find_source_by_id(&self, knowledge_source_id: &str) -> Result<Option<KnowledgeSourceRow>> {
        SqliteKnowledgeRelationalRepository::find_source_by_id(self, knowledge_source_id)
    }

    fn list_items_for_repo(&self, repo_id: &str, limit: usize) -> Result<Vec<KnowledgeItemRow>> {
        SqliteKnowledgeRelationalRepository::list_items_for_repo(self, repo_id, limit)
    }
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
            knowledge_item_id, repo_id, knowledge_source_id, item_kind, latest_knowledge_item_version_id,
            provenance_json, created_at, updated_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, datetime('now'), datetime('now'))
         ON CONFLICT(knowledge_item_id) DO UPDATE SET
            latest_knowledge_item_version_id = excluded.latest_knowledge_item_version_id,
            provenance_json = excluded.provenance_json,
            updated_at = datetime('now')",
        params![
            item.knowledge_item_id.as_str(),
            item.repo_id.as_str(),
            item.knowledge_source_id.as_str(),
            item.item_kind.as_str(),
            item.latest_knowledge_item_version_id.as_str(),
            item.provenance_json.as_str(),
        ],
    )
    .context("upserting SQLite knowledge item")?;
    Ok(())
}

fn insert_relation_assertion_with_conn(
    conn: &rusqlite::Connection,
    relation: &KnowledgeRelationAssertionRow,
) -> Result<()> {
    conn.execute(
        "INSERT OR IGNORE INTO knowledge_relation_assertions (
            relation_assertion_id, repo_id, knowledge_item_id, source_knowledge_item_version_id,
            target_type, target_id, target_knowledge_item_version_id, relation_type,
            association_method, confidence, provenance_json, created_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, datetime('now'))",
        params![
            relation.relation_assertion_id.as_str(),
            relation.repo_id.as_str(),
            relation.knowledge_item_id.as_str(),
            relation.source_knowledge_item_version_id.as_str(),
            relation.target_type.as_str(),
            relation.target_id.as_str(),
            relation.target_knowledge_item_version_id.as_deref(),
            relation.relation_type.as_str(),
            relation.association_method.as_str(),
            relation.confidence,
            relation.provenance_json.as_str(),
        ],
    )
    .context("inserting SQLite knowledge relation assertion")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::SqliteConnectionPool;
    use tempfile::TempDir;

    #[test]
    fn sqlite_relational_store_roundtrip_persists_and_finds_item() {
        let temp = TempDir::new().expect("temp dir");
        let sqlite_path = temp.path().join("knowledge-relational.db");
        let pool = SqliteConnectionPool::connect(sqlite_path).expect("sqlite pool");
        let store = SqliteKnowledgeRelationalRepository::new(pool);
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
            latest_knowledge_item_version_id: "version-1".to_string(),
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
        let store = SqliteKnowledgeRelationalRepository::new(pool);
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
            latest_knowledge_item_version_id: "version-1".to_string(),
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
        let store = SqliteKnowledgeRelationalRepository::new(pool);
        store.initialise_schema().expect("initialise schema");

        let relation = KnowledgeRelationAssertionRow {
            relation_assertion_id: "relation-1".to_string(),
            repo_id: "repo-1".to_string(),
            knowledge_item_id: "item-1".to_string(),
            source_knowledge_item_version_id: "version-1".to_string(),
            target_type: "commit".to_string(),
            target_id: "abc123".to_string(),
            target_knowledge_item_version_id: None,
            relation_type: "associated_with".to_string(),
            association_method: "manual_attachment".to_string(),
            confidence: 1.0,
            provenance_json: "{\"provider\":\"github\"}".to_string(),
        };

        store
            .insert_relation_assertion(&relation)
            .expect("insert relation assertion");
    }
}
