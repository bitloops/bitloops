#![cfg_attr(not(test), allow(dead_code))]

use anyhow::Result;
use rusqlite::{Connection, params};

use crate::host::devql::RelationalStorage;
use crate::host::devql::db_utils::esc_pg;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct GcResult {
    pub(crate) candidate_count: usize,
    pub(crate) deleted_count: usize,
}

pub(crate) async fn run_gc(
    relational: &RelationalStorage,
    _repo_id: &str,
    ttl_days: u32,
) -> Result<GcResult> {
    let cutoff = gc_cutoff_expr(relational, ttl_days);
    let candidates = load_gc_candidates(relational, &cutoff).await?;
    if candidates.is_empty() {
        return Ok(GcResult {
            candidate_count: 0,
            deleted_count: 0,
        });
    }

    let mut statements = Vec::with_capacity(candidates.len() * 3);
    for candidate in &candidates {
        statements.push(delete_content_cache_edges_sql(candidate));
    }
    for candidate in &candidates {
        statements.push(delete_content_cache_artefacts_sql(candidate));
    }
    for candidate in &candidates {
        statements.push(delete_content_cache_sql(candidate));
    }

    relational.exec_batch_transactional(&statements).await?;

    Ok(GcResult {
        candidate_count: candidates.len(),
        deleted_count: candidates.len(),
    })
}

pub(crate) fn run_gc_with_connection(
    conn: &mut Connection,
    ttl_days: u32,
) -> Result<(GcResult, usize)> {
    let candidates = load_gc_candidates_with_connection(conn, ttl_days)?;
    if candidates.is_empty() {
        return Ok((
            GcResult {
                candidate_count: 0,
                deleted_count: 0,
            },
            0,
        ));
    }

    let tx = conn.transaction()?;
    let mut rows_written = 0usize;
    {
        let mut delete_edges = tx.prepare(
            "DELETE FROM content_cache_edges WHERE content_id = ?1 AND language = ?2 AND parser_version = ?3 AND extractor_version = ?4",
        )?;
        let mut delete_artefacts = tx.prepare(
            "DELETE FROM content_cache_artefacts WHERE content_id = ?1 AND language = ?2 AND parser_version = ?3 AND extractor_version = ?4",
        )?;
        let mut delete_cache = tx.prepare(
            "DELETE FROM content_cache WHERE content_id = ?1 AND language = ?2 AND parser_version = ?3 AND extractor_version = ?4",
        )?;
        for candidate in &candidates {
            rows_written += delete_edges.execute(params![
                candidate.content_id,
                candidate.language,
                candidate.parser_version,
                candidate.extractor_version,
            ])?;
            rows_written += delete_artefacts.execute(params![
                candidate.content_id,
                candidate.language,
                candidate.parser_version,
                candidate.extractor_version,
            ])?;
            rows_written += delete_cache.execute(params![
                candidate.content_id,
                candidate.language,
                candidate.parser_version,
                candidate.extractor_version,
            ])?;
        }
    }
    tx.commit()?;

    Ok((
        GcResult {
            candidate_count: candidates.len(),
            deleted_count: candidates.len(),
        },
        rows_written,
    ))
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct GcCandidate {
    content_id: String,
    language: String,
    parser_version: String,
    extractor_version: String,
}

async fn load_gc_candidates(
    relational: &RelationalStorage,
    cutoff: &str,
) -> Result<Vec<GcCandidate>> {
    let sql = format!(
        "SELECT c.content_id, c.language, c.parser_version, c.extractor_version \
FROM content_cache c \
WHERE c.retention_class = 'worktree_only' \
AND c.last_accessed_at < {} \
AND NOT EXISTS ( \
    SELECT 1 FROM current_file_state s \
    WHERE s.effective_content_id = c.content_id \
      AND s.language = c.language \
      AND s.parser_version = c.parser_version \
      AND s.extractor_version = c.extractor_version \
) \
ORDER BY c.content_id, c.language, c.parser_version, c.extractor_version",
        cutoff,
    );
    let rows = relational.query_rows(&sql).await?;
    let candidates = rows
        .into_iter()
        .filter_map(|row| row.as_object().cloned())
        .filter_map(|row| gc_candidate_from_row(&row))
        .collect::<Vec<_>>();
    Ok(candidates)
}

fn load_gc_candidates_with_connection(
    conn: &Connection,
    ttl_days: u32,
) -> Result<Vec<GcCandidate>> {
    let sql = format!(
        "SELECT c.content_id, c.language, c.parser_version, c.extractor_version \
FROM content_cache c \
WHERE c.retention_class = 'worktree_only' \
AND c.last_accessed_at < datetime('now', '-{} days') \
AND NOT EXISTS ( \
    SELECT 1 FROM current_file_state s \
    WHERE s.effective_content_id = c.content_id \
      AND s.language = c.language \
      AND s.parser_version = c.parser_version \
      AND s.extractor_version = c.extractor_version \
) \
ORDER BY c.content_id, c.language, c.parser_version, c.extractor_version",
        ttl_days,
    );
    let mut stmt = conn.prepare(&sql)?;
    stmt.query_map([], |row| {
        Ok(GcCandidate {
            content_id: row.get(0)?,
            language: row.get(1)?,
            parser_version: row.get(2)?,
            extractor_version: row.get(3)?,
        })
    })?
    .collect::<std::result::Result<Vec<_>, _>>()
    .map_err(Into::into)
}

fn gc_candidate_from_row(row: &serde_json::Map<String, serde_json::Value>) -> Option<GcCandidate> {
    Some(GcCandidate {
        content_id: row.get("content_id")?.as_str()?.to_string(),
        language: row.get("language")?.as_str()?.to_string(),
        parser_version: row.get("parser_version")?.as_str()?.to_string(),
        extractor_version: row.get("extractor_version")?.as_str()?.to_string(),
    })
}

fn delete_content_cache_edges_sql(candidate: &GcCandidate) -> String {
    format!(
        "DELETE FROM content_cache_edges \
WHERE content_id = '{}' AND language = '{}' AND parser_version = '{}' AND extractor_version = '{}'",
        esc_pg(&candidate.content_id),
        esc_pg(&candidate.language),
        esc_pg(&candidate.parser_version),
        esc_pg(&candidate.extractor_version),
    )
}

fn delete_content_cache_artefacts_sql(candidate: &GcCandidate) -> String {
    format!(
        "DELETE FROM content_cache_artefacts \
WHERE content_id = '{}' AND language = '{}' AND parser_version = '{}' AND extractor_version = '{}'",
        esc_pg(&candidate.content_id),
        esc_pg(&candidate.language),
        esc_pg(&candidate.parser_version),
        esc_pg(&candidate.extractor_version),
    )
}

fn delete_content_cache_sql(candidate: &GcCandidate) -> String {
    format!(
        "DELETE FROM content_cache \
WHERE content_id = '{}' AND language = '{}' AND parser_version = '{}' AND extractor_version = '{}'",
        esc_pg(&candidate.content_id),
        esc_pg(&candidate.language),
        esc_pg(&candidate.parser_version),
        esc_pg(&candidate.extractor_version),
    )
}

fn gc_cutoff_expr(relational: &RelationalStorage, ttl_days: u32) -> String {
    match relational.dialect() {
        crate::host::devql::RelationalDialect::Sqlite => {
            format!("datetime('now', '-{} days')", ttl_days)
        }
        crate::host::devql::RelationalDialect::Postgres => {
            format!("now() - interval '{} days'", ttl_days)
        }
    }
}

pub(crate) const DEFAULT_GC_TTL_DAYS: u32 = 7;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::devql::RelationalStorage;
    use crate::host::devql::sync::content_cache::{
        CachedArtefact, CachedEdge, CachedExtraction, store_cached_content,
    };
    use crate::host::devql::sync::types::EffectiveSource;
    use serde_json::json;
    use tempfile::tempdir;

    fn test_extraction(content_id: &str) -> CachedExtraction {
        CachedExtraction {
            content_id: content_id.to_string(),
            language: "rust".to_string(),
            parser_version: "parser-v1".to_string(),
            extractor_version: "extractor-v1".to_string(),
            parse_status: "ok".to_string(),
            artefacts: vec![CachedArtefact {
                artifact_key: format!("file::{content_id}"),
                canonical_kind: Some("file".to_string()),
                language_kind: "file".to_string(),
                name: "src/lib.rs".to_string(),
                parent_artifact_key: None,
                start_line: 1,
                end_line: 1,
                start_byte: 0,
                end_byte: 1,
                signature: "file".to_string(),
                modifiers: vec![],
                docstring: None,
                metadata: json!({}),
            }],
            edges: vec![CachedEdge {
                edge_key: format!("edge::{content_id}"),
                from_artifact_key: format!("file::{content_id}"),
                to_artifact_key: None,
                to_symbol_ref: Some("ref".to_string()),
                edge_kind: "calls".to_string(),
                start_line: Some(1),
                end_line: Some(1),
                metadata: json!({}),
            }],
        }
    }

    async fn insert_test_cache_entry(
        relational: &RelationalStorage,
        content_id: &str,
        retention_class: &str,
        last_accessed_at: &str,
    ) {
        let extraction = test_extraction(content_id);
        store_cached_content(relational, &extraction, retention_class)
            .await
            .expect("store test cache entry");
        let sql = format!(
            "UPDATE content_cache SET last_accessed_at = '{}' WHERE content_id = '{}' AND language = '{}' AND parser_version = '{}' AND extractor_version = '{}'",
            esc_pg(last_accessed_at),
            esc_pg(content_id),
            esc_pg(&extraction.language),
            esc_pg(&extraction.parser_version),
            esc_pg(&extraction.extractor_version),
        );
        relational.exec(&sql).await.expect("set last_accessed_at");
    }

    async fn insert_test_file_state(
        relational: &RelationalStorage,
        repo_id: &str,
        content_id: &str,
    ) {
        let repo_name = repo_id.replace('/', "-");
        let repo_sql = format!(
            "INSERT INTO repositories (repo_id, provider, organization, name, default_branch) \
VALUES ('{}', 'test', 'test', '{}', 'main') \
ON CONFLICT (repo_id) DO UPDATE SET \
  provider = excluded.provider, \
  organization = excluded.organization, \
  name = excluded.name, \
  default_branch = excluded.default_branch",
            esc_pg(repo_id),
            esc_pg(&repo_name),
        );
        relational
            .exec(&repo_sql)
            .await
            .expect("insert repositories row");
        let sql = format!(
            "INSERT INTO current_file_state (repo_id, path, language, head_content_id, index_content_id, worktree_content_id, effective_content_id, effective_source, parser_version, extractor_version, exists_in_head, exists_in_index, exists_in_worktree, last_synced_at) \
VALUES ('{}', 'src/a.rs', 'rust', NULL, NULL, NULL, '{}', '{}', 'parser-v1', 'extractor-v1', 0, 0, 0, '2026-03-01T00:00:00Z')",
            esc_pg(repo_id),
            esc_pg(content_id),
            esc_pg(EffectiveSource::Head.as_str()),
        );
        relational
            .exec(&sql)
            .await
            .expect("insert current_file_state");
    }

    async fn lookup_cache_entry(
        relational: &RelationalStorage,
        content_id: &str,
    ) -> Option<serde_json::Value> {
        let sql = format!(
            "SELECT content_id, retention_class FROM content_cache WHERE content_id = '{}' LIMIT 1",
            esc_pg(content_id),
        );
        relational
            .query_rows(&sql)
            .await
            .expect("query content_cache")
            .into_iter()
            .next()
    }

    async fn count_cache_dependents(
        relational: &RelationalStorage,
        table: &str,
        content_id: &str,
    ) -> i64 {
        let sql = format!(
            "SELECT COUNT(*) AS count FROM {table} WHERE content_id = '{}'",
            esc_pg(content_id),
        );
        relational
            .query_rows(&sql)
            .await
            .expect("count cache dependents")
            .first()
            .and_then(serde_json::Value::as_object)
            .and_then(|row| row.get("count"))
            .and_then(serde_json::Value::as_i64)
            .unwrap_or_default()
    }

    #[tokio::test]
    async fn gc_removes_unreferenced_worktree_only_past_ttl() {
        let temp = tempdir().expect("temp dir");
        let sqlite_path = temp.path().join("devql.sqlite");
        crate::host::devql::init_sqlite_schema(&sqlite_path)
            .await
            .expect("initialise sqlite relational schema");
        let relational = RelationalStorage::local_only(sqlite_path);
        let repo_id = "test-repo";

        insert_test_cache_entry(
            &relational,
            "old_hash",
            "worktree_only",
            "2026-03-01 00:00:00",
        )
        .await;
        insert_test_cache_entry(&relational, "git_hash", "git_backed", "2026-03-01 00:00:00").await;
        insert_test_cache_entry(
            &relational,
            "ref_hash",
            "worktree_only",
            "2026-03-01 00:00:00",
        )
        .await;
        insert_test_file_state(&relational, repo_id, "ref_hash").await;

        let result = run_gc(&relational, repo_id, 7).await.expect("run gc");

        assert_eq!(result.candidate_count, 1);
        assert_eq!(result.deleted_count, 1);
        assert!(lookup_cache_entry(&relational, "old_hash").await.is_none());
        assert_eq!(
            count_cache_dependents(&relational, "content_cache_artefacts", "old_hash").await,
            0
        );
        assert_eq!(
            count_cache_dependents(&relational, "content_cache_edges", "old_hash").await,
            0
        );
        assert!(lookup_cache_entry(&relational, "git_hash").await.is_some());
        assert!(lookup_cache_entry(&relational, "ref_hash").await.is_some());
    }

    #[tokio::test]
    async fn gc_preserves_cache_referenced_by_other_repos() {
        let temp = tempdir().expect("temp dir");
        let sqlite_path = temp.path().join("devql.sqlite");
        crate::host::devql::init_sqlite_schema(&sqlite_path)
            .await
            .expect("initialise sqlite relational schema");
        let relational = RelationalStorage::local_only(sqlite_path);
        let repo_a = "repo-a";
        let repo_b = "repo-b";
        let shared_content_id = "shared_hash";

        insert_test_cache_entry(
            &relational,
            shared_content_id,
            "worktree_only",
            "2026-03-01 00:00:00",
        )
        .await;
        insert_test_file_state(&relational, repo_a, shared_content_id).await;
        insert_test_file_state(&relational, repo_b, shared_content_id).await;

        let result = run_gc(&relational, repo_a, 7).await.expect("run gc");

        assert_eq!(result.candidate_count, 0);
        assert_eq!(result.deleted_count, 0);
        assert!(
            lookup_cache_entry(&relational, shared_content_id)
                .await
                .is_some()
        );
        assert_eq!(
            count_cache_dependents(&relational, "content_cache_artefacts", shared_content_id).await,
            1
        );
        assert_eq!(
            count_cache_dependents(&relational, "content_cache_edges", shared_content_id).await,
            1
        );
    }
}
