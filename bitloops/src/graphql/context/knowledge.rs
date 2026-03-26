use std::collections::HashMap;
use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use chrono::{DateTime, FixedOffset, NaiveDateTime};
use serde_json::Value;

use super::DevqlGraphqlContext;
use crate::capability_packs::knowledge::KnowledgePayloadEnvelope;
use crate::graphql::ResolverScope;
use crate::graphql::types::{
    DateTimeScalar, KnowledgeItem, KnowledgeProvider, KnowledgeRelation, KnowledgeSourceKind,
    KnowledgeTargetType, KnowledgeVersion,
};
use crate::host::devql::{
    duckdb_query_rows_path, esc_pg, knowledge_schema_sql_sqlite, sqlite_query_rows_path,
};

impl DevqlGraphqlContext {
    pub(crate) async fn find_knowledge_item_by_id(
        &self,
        knowledge_item_id: &str,
    ) -> Result<Option<KnowledgeItem>> {
        let sqlite_path = self.ensure_knowledge_sqlite_schema().await?;
        let sql = format!(
            "SELECT i.knowledge_item_id, i.knowledge_source_id, i.latest_knowledge_item_version_id, \
s.provider, s.source_kind, s.canonical_external_id, s.canonical_url \
FROM knowledge_items i \
JOIN knowledge_sources s ON s.knowledge_source_id = i.knowledge_source_id \
WHERE i.repo_id = '{}' AND i.knowledge_item_id = '{}' \
LIMIT 1",
            esc_pg(self.repo_identity.repo_id.as_str()),
            esc_pg(knowledge_item_id)
        );

        let rows = sqlite_query_rows_path(&sqlite_path, &sql).await?;
        rows.into_iter()
            .next()
            .map(knowledge_item_from_row)
            .transpose()
    }

    pub(crate) async fn list_knowledge_items(
        &self,
        provider: Option<KnowledgeProvider>,
        _scope: &ResolverScope,
    ) -> Result<Vec<KnowledgeItem>> {
        let sqlite_path = self.ensure_knowledge_sqlite_schema().await?;
        let mut conditions = vec![format!(
            "i.repo_id = '{}'",
            esc_pg(self.repo_identity.repo_id.as_str())
        )];
        if let Some(provider) = provider {
            conditions.push(format!(
                "s.provider = '{}'",
                esc_pg(provider.as_storage_value())
            ));
        }

        let sql = format!(
            "SELECT i.knowledge_item_id, i.knowledge_source_id, i.latest_knowledge_item_version_id, \
s.provider, s.source_kind, s.canonical_external_id, s.canonical_url \
FROM knowledge_items i \
JOIN knowledge_sources s ON s.knowledge_source_id = i.knowledge_source_id \
WHERE {} \
ORDER BY i.updated_at DESC, i.knowledge_item_id DESC",
            conditions.join(" AND ")
        );

        let rows = sqlite_query_rows_path(&sqlite_path, &sql).await?;
        rows.into_iter().map(knowledge_item_from_row).collect()
    }

    pub(crate) async fn list_knowledge_relations(
        &self,
        knowledge_item_id: &str,
    ) -> Result<Vec<KnowledgeRelation>> {
        let sqlite_path = self.ensure_knowledge_sqlite_schema().await?;
        let sql = format!(
            "SELECT relation_assertion_id, source_knowledge_item_version_id, target_type, target_id, \
target_knowledge_item_version_id, relation_type, association_method, confidence, provenance_json \
FROM knowledge_relation_assertions \
WHERE repo_id = '{}' AND knowledge_item_id = '{}' \
ORDER BY created_at DESC, relation_assertion_id DESC",
            esc_pg(self.repo_identity.repo_id.as_str()),
            esc_pg(knowledge_item_id)
        );

        let rows = sqlite_query_rows_path(&sqlite_path, &sql).await?;
        rows.into_iter().map(knowledge_relation_from_row).collect()
    }

    pub(crate) async fn find_knowledge_relation_by_id(
        &self,
        relation_assertion_id: &str,
    ) -> Result<Option<KnowledgeRelation>> {
        let sqlite_path = self.ensure_knowledge_sqlite_schema().await?;
        let sql = format!(
            "SELECT relation_assertion_id, source_knowledge_item_version_id, target_type, target_id, \
target_knowledge_item_version_id, relation_type, association_method, confidence, provenance_json \
FROM knowledge_relation_assertions \
WHERE repo_id = '{}' AND relation_assertion_id = '{}' \
LIMIT 1",
            esc_pg(self.repo_identity.repo_id.as_str()),
            esc_pg(relation_assertion_id)
        );

        let rows = sqlite_query_rows_path(&sqlite_path, &sql).await?;
        rows.into_iter()
            .next()
            .map(knowledge_relation_from_row)
            .transpose()
    }

    pub(crate) async fn load_knowledge_versions_by_item_ids(
        &self,
        knowledge_item_ids: &[String],
    ) -> Result<HashMap<String, Vec<KnowledgeVersion>>> {
        if knowledge_item_ids.is_empty() {
            return Ok(HashMap::new());
        }

        let duckdb_path = self.knowledge_duckdb_path()?;
        let ids = knowledge_item_ids
            .iter()
            .map(|id| format!("'{}'", esc_pg(id)))
            .collect::<Vec<_>>()
            .join(", ");
        let sql = format!(
            "SELECT knowledge_item_version_id, knowledge_item_id, content_hash, title, state, author, \
updated_at, body_preview, normalized_fields_json, provenance_json, storage_path, CAST(created_at AS VARCHAR) AS created_at \
FROM knowledge_document_versions \
WHERE knowledge_item_id IN ({ids}) \
ORDER BY knowledge_item_id ASC, created_at DESC, knowledge_item_version_id DESC"
        );

        let rows = duckdb_query_rows_path(&duckdb_path, &sql).await?;
        let mut versions_by_item = knowledge_item_ids
            .iter()
            .cloned()
            .map(|id| (id, Vec::new()))
            .collect::<HashMap<_, _>>();

        for row in rows {
            let version = knowledge_version_from_row(row)?;
            versions_by_item
                .entry(version.knowledge_item_id.as_ref().to_string())
                .or_default()
                .push(version);
        }

        Ok(versions_by_item)
    }

    pub(crate) fn load_knowledge_payload(
        &self,
        storage_path: &str,
    ) -> Result<Option<KnowledgePayloadEnvelope>> {
        let blob_store = self
            .blob_store
            .as_ref()
            .context("blob store unavailable for GraphQL knowledge payload reads")?;

        if !blob_store
            .exists(storage_path)
            .with_context(|| format!("checking knowledge payload blob `{storage_path}`"))?
        {
            return Ok(None);
        }

        let bytes = blob_store
            .read(storage_path)
            .with_context(|| format!("reading knowledge payload blob `{storage_path}`"))?;
        let payload = serde_json::from_slice(&bytes)
            .with_context(|| format!("deserialising knowledge payload blob `{storage_path}`"))?;
        Ok(Some(payload))
    }

    fn knowledge_duckdb_path(&self) -> Result<PathBuf> {
        Ok(self
            .backend_config
            .as_ref()
            .context("store backend configuration unavailable")?
            .events
            .resolve_duckdb_db_path_for_repo(&self.repo_root))
    }

    async fn ensure_knowledge_sqlite_schema(&self) -> Result<PathBuf> {
        let sqlite_path = self.devql_sqlite_path()?;
        let db_path = sqlite_path.clone();
        tokio::task::spawn_blocking(move || -> Result<()> {
            if !db_path.is_file() {
                bail!(
                    "SQLite database file not found at {}. Run `bitloops init` to create and initialise stores.",
                    db_path.display()
                );
            }

            let conn = rusqlite::Connection::open_with_flags(
                &db_path,
                rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE,
            )
            .with_context(|| format!("opening SQLite database at {}", db_path.display()))?;
            conn.execute_batch(knowledge_schema_sql_sqlite())
                .context("initialising SQLite knowledge schema")?;
            Ok(())
        })
        .await
        .context("joining SQLite knowledge schema task")??;

        Ok(sqlite_path)
    }
}

fn knowledge_item_from_row(row: Value) -> Result<KnowledgeItem> {
    let provider_raw = required_string(&row, "provider")?;
    let source_kind_raw = required_string(&row, "source_kind")?;
    let provider =
        KnowledgeProvider::from_storage_value(&provider_raw).map_err(|err| anyhow::anyhow!(err))?;
    let source_kind = KnowledgeSourceKind::from_storage_value(&source_kind_raw)
        .map_err(|err| anyhow::anyhow!(err))?;

    Ok(KnowledgeItem {
        id: required_string(&row, "knowledge_item_id")?.into(),
        source_id: required_string(&row, "knowledge_source_id")?.into(),
        provider,
        source_kind,
        canonical_external_id: required_string(&row, "canonical_external_id")?,
        external_url: required_string(&row, "canonical_url")?,
        latest_knowledge_item_version_id: required_string(
            &row,
            "latest_knowledge_item_version_id",
        )?,
    })
}

fn knowledge_relation_from_row(row: Value) -> Result<KnowledgeRelation> {
    let target_type_raw = required_string(&row, "target_type")?;
    let target_type = KnowledgeTargetType::from_storage_value(&target_type_raw)
        .map_err(|err| anyhow::anyhow!(err))?;

    Ok(KnowledgeRelation {
        id: required_string(&row, "relation_assertion_id")?.into(),
        source_version_id: required_string(&row, "source_knowledge_item_version_id")?.into(),
        target_type,
        target_id: required_string(&row, "target_id")?,
        target_version_id: optional_string(&row, "target_knowledge_item_version_id")
            .map(Into::into),
        relation_type: required_string(&row, "relation_type")?,
        association_method: required_string(&row, "association_method")?,
        confidence: optional_f64(&row, "confidence"),
        provenance: async_graphql::types::Json(parsed_json_column(&row, "provenance_json")?),
    })
}

fn knowledge_version_from_row(row: Value) -> Result<KnowledgeVersion> {
    Ok(KnowledgeVersion {
        id: required_string(&row, "knowledge_item_version_id")?.into(),
        knowledge_item_id: required_string(&row, "knowledge_item_id")?.into(),
        content_hash: required_string(&row, "content_hash")?,
        title: required_string(&row, "title")?,
        state: optional_string(&row, "state"),
        author: optional_string(&row, "author"),
        updated_at: optional_string(&row, "updated_at")
            .as_deref()
            .map(parse_storage_datetime)
            .transpose()?,
        body_preview: optional_string(&row, "body_preview"),
        normalized_fields: async_graphql::types::Json(parsed_json_column(
            &row,
            "normalized_fields_json",
        )?),
        provenance: async_graphql::types::Json(parsed_json_column(&row, "provenance_json")?),
        created_at: parse_storage_datetime(&required_string(&row, "created_at")?)?,
        storage_path: required_string(&row, "storage_path")?,
    })
}

fn parse_storage_datetime(value: &str) -> Result<DateTimeScalar> {
    if let Ok(timestamp) = DateTimeScalar::from_rfc3339(value.to_string()) {
        return Ok(timestamp);
    }

    let parsed = NaiveDateTime::parse_from_str(value, "%Y-%m-%d %H:%M:%S%.f")
        .or_else(|_| NaiveDateTime::parse_from_str(value, "%Y-%m-%d %H:%M:%S"))
        .with_context(|| format!("parsing storage timestamp `{value}`"))?;
    let zero_offset = FixedOffset::east_opt(0).expect("zero offset is valid");
    let normalised =
        DateTime::<FixedOffset>::from_naive_utc_and_offset(parsed, zero_offset).to_rfc3339();
    DateTimeScalar::from_rfc3339(normalised)
        .with_context(|| format!("normalising storage timestamp `{value}`"))
}

fn parsed_json_column(row: &Value, key: &str) -> Result<Value> {
    let raw = required_string(row, key)?;
    serde_json::from_str(&raw).with_context(|| format!("parsing JSON column `{key}`"))
}

fn required_string(row: &Value, key: &str) -> Result<String> {
    row.get(key)
        .and_then(Value::as_str)
        .map(str::to_string)
        .with_context(|| format!("missing `{key}`"))
}

fn optional_string(row: &Value, key: &str) -> Option<String> {
    row.get(key).and_then(Value::as_str).map(str::to_string)
}

fn optional_f64(row: &Value, key: &str) -> Option<f64> {
    row.get(key).and_then(|value| {
        value
            .as_f64()
            .or_else(|| value.as_i64().map(|value| value as f64))
            .or_else(|| value.as_str().and_then(|value| value.parse::<f64>().ok()))
    })
}
