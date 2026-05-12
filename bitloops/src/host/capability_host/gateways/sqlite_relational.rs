use anyhow::{Context, Result, bail};
use rusqlite::{OptionalExtension, params};

use super::RelationalGateway;
use crate::host::checkpoints::checkpoint_id::is_valid_checkpoint_id;
use crate::models::{
    CurrentCanonicalArtefactRecord, CurrentCanonicalEdgeRecord, CurrentCanonicalFileRecord,
    ProductionArtefact,
};
use crate::storage::SqliteConnectionPool;

pub struct SqliteRelationalGateway {
    sqlite: SqliteConnectionPool,
}

impl SqliteRelationalGateway {
    pub fn new(sqlite: SqliteConnectionPool) -> Self {
        Self { sqlite }
    }

    pub fn resolve_checkpoint_id(&self, repo_id: &str, checkpoint_ref: &str) -> Result<String> {
        let trimmed = checkpoint_ref.trim();
        if trimmed.is_empty() {
            bail!("checkpoint id must not be empty");
        }
        if !is_valid_checkpoint_id(trimmed) {
            bail!(
                "checkpoint id `{trimmed}` is not a valid checkpoint identifier \
                 (expected 12-character lowercase hex)"
            );
        }

        self.sqlite
            .initialise_relational_checkpoint_schema()
            .context("initialising relational checkpoint schema for checkpoint resolution")?;

        let exists = self.sqlite.with_connection(|conn| {
            conn.query_row(
                "SELECT checkpoint_id FROM checkpoints WHERE checkpoint_id = ?1 AND repo_id = ?2 LIMIT 1",
                params![trimmed, repo_id],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(anyhow::Error::from)
        })?;

        exists
            .map(|id| id.trim().to_string())
            .with_context(|| format!("checkpoint `{trimmed}` not found in current repository"))
    }

    pub fn load_repo_id_for_commit(&self, commit_sha: &str) -> Result<String> {
        let commit_sha = commit_sha.to_string();
        self.sqlite.with_connection(|conn| {
            let mut stmt = conn
                .prepare("SELECT repo_id FROM commits WHERE commit_sha = ?1 LIMIT 1")
                .context("failed preparing repo lookup query")?;
            let repo_id: String = stmt
                .query_row(params![commit_sha], |row| row.get(0))
                .with_context(|| {
                    format!(
                        "no production artefacts found for commit {}; materialise production artefacts first (use `bitloops devql tasks enqueue --kind ingest --status` or `bitloops devql tasks enqueue --kind sync --status`)",
                        commit_sha
                    )
                })?;
            Ok(repo_id)
        })
    }

    pub fn load_current_canonical_files(
        &self,
        repo_id: &str,
    ) -> Result<Vec<CurrentCanonicalFileRecord>> {
        let repo_id = repo_id.to_string();
        self.sqlite.with_connection(|conn| {
            let mut stmt = conn
                .prepare(
                    r#"
SELECT
  repo_id,
  path,
  analysis_mode,
  file_role,
  language,
  resolved_language,
  effective_content_id,
  parser_version,
  extractor_version,
  exists_in_head,
  exists_in_index,
  exists_in_worktree
FROM current_file_state
WHERE repo_id = ?1
ORDER BY path ASC
"#,
                )
                .map_err(|err| map_missing_sync_table(err, "current_file_state"))
                .context("failed preparing current canonical file query")?;

            let rows = stmt
                .query_map(params![repo_id], |row| {
                    Ok(CurrentCanonicalFileRecord {
                        repo_id: row.get(0)?,
                        path: row.get(1)?,
                        analysis_mode: row.get(2)?,
                        file_role: row.get(3)?,
                        language: row.get(4)?,
                        resolved_language: row.get(5)?,
                        effective_content_id: row.get(6)?,
                        parser_version: row.get(7)?,
                        extractor_version: row.get(8)?,
                        exists_in_head: sqlite_bool(row.get::<_, i64>(9)?),
                        exists_in_index: sqlite_bool(row.get::<_, i64>(10)?),
                        exists_in_worktree: sqlite_bool(row.get::<_, i64>(11)?),
                    })
                })
                .map_err(|err| map_missing_sync_table(err, "current_file_state"))
                .context("failed querying current canonical files")?;

            let mut files = Vec::new();
            for row in rows {
                files.push(row.context("failed decoding current canonical file row")?);
            }
            Ok(files)
        })
    }

    pub fn load_current_canonical_artefacts(
        &self,
        repo_id: &str,
    ) -> Result<Vec<CurrentCanonicalArtefactRecord>> {
        let repo_id = repo_id.to_string();
        self.sqlite.with_connection(|conn| {
            let mut stmt = conn
                .prepare(
                    r#"
SELECT
  repo_id,
  path,
  content_id,
  symbol_id,
  artefact_id,
  language,
  COALESCE(extraction_fingerprint, ''),
  canonical_kind,
  language_kind,
  symbol_fqn,
  parent_symbol_id,
  parent_artefact_id,
  start_line,
  end_line,
  start_byte,
  end_byte,
  signature,
  modifiers,
  docstring
FROM artefacts_current
WHERE repo_id = ?1
ORDER BY path ASC, start_line ASC, end_line ASC, symbol_id ASC
"#,
                )
                .map_err(|err| map_missing_sync_table(err, "artefacts_current"))
                .context("failed preparing current canonical artefact query")?;

            let rows = stmt
                .query_map(params![repo_id], |row| {
                    Ok(CurrentCanonicalArtefactRecord {
                        repo_id: row.get(0)?,
                        path: row.get(1)?,
                        content_id: row.get(2)?,
                        symbol_id: row.get(3)?,
                        artefact_id: row.get(4)?,
                        language: row.get(5)?,
                        extraction_fingerprint: row.get(6)?,
                        canonical_kind: row.get(7)?,
                        language_kind: row.get(8)?,
                        symbol_fqn: row.get(9)?,
                        parent_symbol_id: row.get(10)?,
                        parent_artefact_id: row.get(11)?,
                        start_line: row.get(12)?,
                        end_line: row.get(13)?,
                        start_byte: row.get(14)?,
                        end_byte: row.get(15)?,
                        signature: row.get(16)?,
                        modifiers: row.get(17)?,
                        docstring: row.get(18)?,
                    })
                })
                .map_err(|err| map_missing_sync_table(err, "artefacts_current"))
                .context("failed querying current canonical artefacts")?;

            let mut artefacts = Vec::new();
            for row in rows {
                artefacts.push(row.context("failed decoding current canonical artefact row")?);
            }
            Ok(artefacts)
        })
    }

    pub fn visit_current_canonical_artefacts(
        &self,
        repo_id: &str,
        visitor: &mut dyn FnMut(CurrentCanonicalArtefactRecord) -> Result<()>,
    ) -> Result<()> {
        let repo_id = repo_id.to_string();
        self.sqlite.with_connection(|conn| {
            let mut stmt = conn
                .prepare(
                    r#"
SELECT
  repo_id,
  path,
  content_id,
  symbol_id,
  artefact_id,
  language,
  COALESCE(extraction_fingerprint, ''),
  canonical_kind,
  language_kind,
  symbol_fqn,
  parent_symbol_id,
  parent_artefact_id,
  start_line,
  end_line,
  start_byte,
  end_byte,
  signature,
  modifiers,
  docstring
FROM artefacts_current
WHERE repo_id = ?1
ORDER BY path ASC, start_line ASC, end_line ASC, symbol_id ASC
"#,
                )
                .map_err(|err| map_missing_sync_table(err, "artefacts_current"))
                .context("failed preparing current canonical artefact visitor query")?;

            let rows = stmt
                .query_map(params![repo_id], |row| {
                    Ok(CurrentCanonicalArtefactRecord {
                        repo_id: row.get(0)?,
                        path: row.get(1)?,
                        content_id: row.get(2)?,
                        symbol_id: row.get(3)?,
                        artefact_id: row.get(4)?,
                        language: row.get(5)?,
                        extraction_fingerprint: row.get(6)?,
                        canonical_kind: row.get(7)?,
                        language_kind: row.get(8)?,
                        symbol_fqn: row.get(9)?,
                        parent_symbol_id: row.get(10)?,
                        parent_artefact_id: row.get(11)?,
                        start_line: row.get(12)?,
                        end_line: row.get(13)?,
                        start_byte: row.get(14)?,
                        end_byte: row.get(15)?,
                        signature: row.get(16)?,
                        modifiers: row.get(17)?,
                        docstring: row.get(18)?,
                    })
                })
                .map_err(|err| map_missing_sync_table(err, "artefacts_current"))
                .context("failed querying current canonical artefacts for visitor")?;

            for row in rows {
                visitor(row.context("failed decoding current canonical artefact row")?)?;
            }
            Ok(())
        })
    }

    pub fn load_current_canonical_edges(
        &self,
        repo_id: &str,
    ) -> Result<Vec<CurrentCanonicalEdgeRecord>> {
        let repo_id = repo_id.to_string();
        self.sqlite.with_connection(|conn| {
            let mut stmt = conn
                .prepare(
                    r#"
SELECT
  repo_id,
  edge_id,
  path,
  content_id,
  from_symbol_id,
  from_artefact_id,
  to_symbol_id,
  to_artefact_id,
  to_symbol_ref,
  edge_kind,
  language,
  start_line,
  end_line,
  metadata
FROM artefact_edges_current
WHERE repo_id = ?1
ORDER BY path ASC, edge_id ASC
"#,
                )
                .map_err(|err| map_missing_sync_table(err, "artefact_edges_current"))
                .context("failed preparing current canonical edge query")?;

            let rows = stmt
                .query_map(params![repo_id], |row| {
                    Ok(CurrentCanonicalEdgeRecord {
                        repo_id: row.get(0)?,
                        edge_id: row.get(1)?,
                        path: row.get(2)?,
                        content_id: row.get(3)?,
                        from_symbol_id: row.get(4)?,
                        from_artefact_id: row.get(5)?,
                        to_symbol_id: row.get(6)?,
                        to_artefact_id: row.get(7)?,
                        to_symbol_ref: row.get(8)?,
                        edge_kind: row.get(9)?,
                        language: row.get(10)?,
                        start_line: row.get(11)?,
                        end_line: row.get(12)?,
                        metadata: row.get(13)?,
                    })
                })
                .map_err(|err| map_missing_sync_table(err, "artefact_edges_current"))
                .context("failed querying current canonical edges")?;

            let mut edges = Vec::new();
            for row in rows {
                edges.push(row.context("failed decoding current canonical edge row")?);
            }
            Ok(edges)
        })
    }

    pub fn visit_current_canonical_edges(
        &self,
        repo_id: &str,
        visitor: &mut dyn FnMut(CurrentCanonicalEdgeRecord) -> Result<()>,
    ) -> Result<()> {
        let repo_id = repo_id.to_string();
        self.sqlite.with_connection(|conn| {
            let mut stmt = conn
                .prepare(
                    r#"
SELECT
  repo_id,
  edge_id,
  path,
  content_id,
  from_symbol_id,
  from_artefact_id,
  to_symbol_id,
  to_artefact_id,
  to_symbol_ref,
  edge_kind,
  language,
  start_line,
  end_line,
  metadata
FROM artefact_edges_current
WHERE repo_id = ?1
ORDER BY path ASC, edge_id ASC
"#,
                )
                .map_err(|err| map_missing_sync_table(err, "artefact_edges_current"))
                .context("failed preparing current canonical edge visitor query")?;

            let rows = stmt
                .query_map(params![repo_id], |row| {
                    Ok(CurrentCanonicalEdgeRecord {
                        repo_id: row.get(0)?,
                        edge_id: row.get(1)?,
                        path: row.get(2)?,
                        content_id: row.get(3)?,
                        from_symbol_id: row.get(4)?,
                        from_artefact_id: row.get(5)?,
                        to_symbol_id: row.get(6)?,
                        to_artefact_id: row.get(7)?,
                        to_symbol_ref: row.get(8)?,
                        edge_kind: row.get(9)?,
                        language: row.get(10)?,
                        start_line: row.get(11)?,
                        end_line: row.get(12)?,
                        metadata: row.get(13)?,
                    })
                })
                .map_err(|err| map_missing_sync_table(err, "artefact_edges_current"))
                .context("failed querying current canonical edges for visitor")?;

            for row in rows {
                visitor(row.context("failed decoding current canonical edge row")?)?;
            }
            Ok(())
        })
    }

    pub fn load_production_artefacts(&self, commit_sha: &str) -> Result<Vec<ProductionArtefact>> {
        let commit_sha = commit_sha.to_string();
        self.sqlite.with_connection(|conn| {
            let mut stmt = conn
                .prepare(
                    r#"
SELECT DISTINCT a.artefact_id, a.symbol_id, COALESCE(a.symbol_fqn, ''), a.path, a.start_line
FROM file_state fs
JOIN artefacts_historical a
  ON a.repo_id = fs.repo_id
 AND a.blob_sha = fs.blob_sha
 AND a.path = fs.path
WHERE fs.commit_sha = ?1
  AND a.canonical_kind IN ('function', 'method', 'class')
ORDER BY a.path ASC, a.start_line ASC
"#,
                )
                .context("failed preparing production artefact query")?;

            let rows = stmt
                .query_map(params![commit_sha], |row| {
                    Ok(ProductionArtefact {
                        artefact_id: row.get(0)?,
                        symbol_id: row.get(1)?,
                        symbol_fqn: row.get::<_, String>(2)?,
                        path: row.get(3)?,
                        start_line: row.get(4)?,
                    })
                })
                .context("failed querying production artefacts")?;

            let mut artefacts = Vec::new();
            for row in rows {
                artefacts.push(row.context("failed decoding production artefact row")?);
            }
            Ok(artefacts)
        })
    }

    pub fn load_current_production_artefacts(
        &self,
        repo_id: &str,
    ) -> Result<Vec<ProductionArtefact>> {
        let repo_id = repo_id.to_string();
        self.sqlite.with_connection(|conn| {
            let mut stmt = conn
                .prepare(
                    r#"
SELECT artefact_id, symbol_id, COALESCE(symbol_fqn, ''), path, start_line
FROM artefacts_current
WHERE repo_id = ?1
  AND canonical_kind IN ('function', 'method', 'class')
ORDER BY path ASC, start_line ASC
"#,
                )
                .context("failed preparing current production artefact query")?;

            let rows = stmt
                .query_map(params![repo_id], |row| {
                    Ok(ProductionArtefact {
                        artefact_id: row.get(0)?,
                        symbol_id: row.get(1)?,
                        symbol_fqn: row.get::<_, String>(2)?,
                        path: row.get(3)?,
                        start_line: row.get(4)?,
                    })
                })
                .context("failed querying current production artefacts")?;

            let mut artefacts = Vec::new();
            for row in rows {
                artefacts.push(row.context("failed decoding current production artefact row")?);
            }
            Ok(artefacts)
        })
    }

    pub fn load_artefacts_for_file_lines(
        &self,
        commit_sha: &str,
        file_path: &str,
    ) -> Result<Vec<(String, i64, i64)>> {
        let commit_sha = commit_sha.to_string();
        let file_path = file_path.to_string();
        self.sqlite.with_connection(|conn| {
            let mut stmt = conn
                .prepare(
                    r#"
SELECT DISTINCT a.artefact_id, a.start_line, a.end_line
FROM file_state fs
JOIN artefacts_historical a
  ON a.repo_id = fs.repo_id
 AND a.blob_sha = fs.blob_sha
 AND a.path = fs.path
WHERE fs.commit_sha = ?1
  AND a.canonical_kind != 'file'
  AND (fs.path = ?2 OR ?2 LIKE '%' || fs.path)
ORDER BY a.path ASC, a.start_line ASC
"#,
                )
                .context("failed preparing artefacts-for-file query")?;

            let rows = stmt
                .query_map(params![commit_sha, file_path], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, i64>(2)?,
                    ))
                })
                .context("failed querying artefacts for file")?;

            let mut result = Vec::new();
            for row in rows {
                result.push(row.context("failed mapping artefact-for-file row")?);
            }
            Ok(result)
        })
    }

    pub fn artefact_exists(&self, repo_id: &str, artefact_id: &str) -> Result<bool> {
        let trimmed = artefact_id.trim();
        if trimmed.is_empty() {
            return Ok(false);
        }
        if !is_valid_artefact_id(trimmed) {
            return Ok(false);
        }

        self.sqlite.with_connection(|conn| {
            let current = conn
                .query_row(
                    "SELECT artefact_id FROM artefacts_current \
                     WHERE repo_id = ?1 AND artefact_id = ?2 LIMIT 1",
                    params![repo_id, trimmed],
                    |row| row.get::<_, String>(0),
                )
                .optional();

            match current {
                Ok(Some(_)) => return Ok(true),
                Ok(None) => {}
                Err(err) if err.to_string().contains("no such table") => {}
                Err(err) => return Err(anyhow::Error::from(err)),
            }

            let historical = conn
                .query_row(
                    "SELECT artefact_id FROM artefacts \
                     WHERE repo_id = ?1 AND artefact_id = ?2 LIMIT 1",
                    params![repo_id, trimmed],
                    |row| row.get::<_, String>(0),
                )
                .optional();

            match historical {
                Ok(result) => Ok(result.is_some()),
                Err(err) if err.to_string().contains("no such table") => Ok(false),
                Err(err) => Err(anyhow::Error::from(err)),
            }
        })
    }
}

impl RelationalGateway for SqliteRelationalGateway {
    fn resolve_checkpoint_id(&self, repo_id: &str, checkpoint_ref: &str) -> Result<String> {
        SqliteRelationalGateway::resolve_checkpoint_id(self, repo_id, checkpoint_ref)
    }

    fn artefact_exists(&self, repo_id: &str, artefact_id: &str) -> Result<bool> {
        SqliteRelationalGateway::artefact_exists(self, repo_id, artefact_id)
    }

    fn load_repo_id_for_commit(&self, commit_sha: &str) -> Result<String> {
        SqliteRelationalGateway::load_repo_id_for_commit(self, commit_sha)
    }

    fn load_current_canonical_files(
        &self,
        repo_id: &str,
    ) -> Result<Vec<CurrentCanonicalFileRecord>> {
        SqliteRelationalGateway::load_current_canonical_files(self, repo_id)
    }

    fn load_current_canonical_artefacts(
        &self,
        repo_id: &str,
    ) -> Result<Vec<CurrentCanonicalArtefactRecord>> {
        SqliteRelationalGateway::load_current_canonical_artefacts(self, repo_id)
    }

    fn visit_current_canonical_artefacts(
        &self,
        repo_id: &str,
        visitor: &mut dyn FnMut(CurrentCanonicalArtefactRecord) -> Result<()>,
    ) -> Result<()> {
        SqliteRelationalGateway::visit_current_canonical_artefacts(self, repo_id, visitor)
    }

    fn load_current_canonical_edges(
        &self,
        repo_id: &str,
    ) -> Result<Vec<CurrentCanonicalEdgeRecord>> {
        SqliteRelationalGateway::load_current_canonical_edges(self, repo_id)
    }

    fn visit_current_canonical_edges(
        &self,
        repo_id: &str,
        visitor: &mut dyn FnMut(CurrentCanonicalEdgeRecord) -> Result<()>,
    ) -> Result<()> {
        SqliteRelationalGateway::visit_current_canonical_edges(self, repo_id, visitor)
    }

    fn load_current_production_artefacts(&self, repo_id: &str) -> Result<Vec<ProductionArtefact>> {
        SqliteRelationalGateway::load_current_production_artefacts(self, repo_id)
    }

    fn load_production_artefacts(&self, commit_sha: &str) -> Result<Vec<ProductionArtefact>> {
        SqliteRelationalGateway::load_production_artefacts(self, commit_sha)
    }

    fn load_artefacts_for_file_lines(
        &self,
        commit_sha: &str,
        file_path: &str,
    ) -> Result<Vec<(String, i64, i64)>> {
        SqliteRelationalGateway::load_artefacts_for_file_lines(self, commit_sha, file_path)
    }
}

fn sqlite_bool(value: i64) -> bool {
    value != 0
}

fn map_missing_sync_table(err: rusqlite::Error, table_name: &str) -> anyhow::Error {
    if err.to_string().contains("no such table") {
        anyhow::anyhow!(
            "required DevQL sync table `{table_name}` is unavailable; run DevQL sync first."
        )
    } else {
        anyhow::Error::from(err)
    }
}

fn is_valid_artefact_id(id: &str) -> bool {
    let parts: Vec<&str> = id.split('-').collect();
    if parts.len() != 5 {
        return false;
    }
    let expected_lengths = [8, 4, 4, 4, 12];
    parts
        .iter()
        .zip(expected_lengths.iter())
        .all(|(part, &len)| {
            part.len() == len
                && part
                    .chars()
                    .all(|ch| ch.is_ascii_hexdigit() && !ch.is_ascii_uppercase())
        })
}

#[cfg(test)]
mod tests {
    use anyhow::Result;
    use tempfile::TempDir;

    use super::SqliteRelationalGateway;
    use crate::storage::{SqliteConnectionPool, init::init_database};

    #[test]
    fn current_canonical_loaders_return_sync_shaped_rows() -> Result<()> {
        let temp = TempDir::new()?;
        let db_path = temp.path().join("runtime.sqlite");
        init_database(&db_path, false, "seed-commit")?;
        let sqlite = SqliteConnectionPool::connect_existing(db_path)?;
        let gateway = SqliteRelationalGateway::new(sqlite.clone());

        sqlite.with_connection(|conn| {
            conn.execute(
                "INSERT INTO repositories (repo_id, provider, organization, name, default_branch)
                 VALUES (?1, 'local', 'bitloops', 'demo', 'main')",
                rusqlite::params!["repo-1"],
            )?;
            conn.execute(
                "INSERT INTO current_file_state (
                    repo_id, path, analysis_mode, file_role, language, resolved_language,
                    effective_content_id, effective_source, parser_version, extractor_version,
                    exists_in_head, exists_in_index, exists_in_worktree, last_synced_at
                ) VALUES (
                    ?1, ?2, 'code', 'source_code', 'typescript', 'typescript',
                    'content-a', 'worktree', 'parser-v1', 'extractor-v1',
                    1, 0, 1, '2026-04-28T10:00:00Z'
                )",
                rusqlite::params!["repo-1", "packages/api/src/caller.ts"],
            )?;
            conn.execute(
                "INSERT INTO artefacts_current (
                    repo_id, path, content_id, symbol_id, artefact_id, language,
                    extraction_fingerprint, canonical_kind, language_kind, symbol_fqn,
                    parent_symbol_id, parent_artefact_id, start_line, end_line,
                    start_byte, end_byte, signature, modifiers, docstring, updated_at
                ) VALUES (
                    ?1, ?2, 'content-a', 'sym::caller', 'artefact::caller', 'typescript',
                    'fingerprint-a', 'function', 'function_declaration',
                    'packages/api/src/caller.ts::caller', NULL, NULL, 4, 8,
                    0, 40, NULL, '[]', 'Doc', '2026-04-28T10:00:00Z'
                )",
                rusqlite::params!["repo-1", "packages/api/src/caller.ts"],
            )?;
            conn.execute(
                "INSERT INTO artefact_edges_current (
                    repo_id, edge_id, path, content_id, from_symbol_id, from_artefact_id,
                    to_symbol_id, to_artefact_id, to_symbol_ref, edge_kind, language,
                    start_line, end_line, metadata, updated_at
                ) VALUES (
                    ?1, 'edge-1', ?2, 'content-a', 'sym::caller', 'artefact::caller',
                    NULL, NULL, 'packages/api/src/target.ts::target', 'calls', 'typescript',
                    6, 6, '{\"resolution\":\"fixture\"}', '2026-04-28T10:00:00Z'
                )",
                rusqlite::params!["repo-1", "packages/api/src/caller.ts"],
            )?;
            Ok(())
        })?;

        let files = gateway.load_current_canonical_files("repo-1")?;
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, "packages/api/src/caller.ts");
        assert_eq!(files[0].analysis_mode, "code");
        assert!(!files[0].exists_in_index);
        assert!(files[0].exists_in_head);
        assert!(files[0].exists_in_worktree);

        let artefacts = gateway.load_current_canonical_artefacts("repo-1")?;
        assert_eq!(artefacts.len(), 1);
        assert_eq!(artefacts[0].artefact_id, "artefact::caller");
        assert_eq!(artefacts[0].canonical_kind.as_deref(), Some("function"));
        assert_eq!(
            artefacts[0].symbol_fqn.as_deref(),
            Some("packages/api/src/caller.ts::caller")
        );

        let edges = gateway.load_current_canonical_edges("repo-1")?;
        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].edge_id, "edge-1");
        assert_eq!(edges[0].edge_kind, "calls");
        assert_eq!(
            edges[0].to_symbol_ref.as_deref(),
            Some("packages/api/src/target.ts::target")
        );

        Ok(())
    }

    #[test]
    fn current_canonical_visitors_preserve_loader_order() -> Result<()> {
        let temp = TempDir::new()?;
        let db_path = temp.path().join("runtime.sqlite");
        init_database(&db_path, false, "seed-commit")?;
        let sqlite = SqliteConnectionPool::connect_existing(db_path)?;
        let gateway = SqliteRelationalGateway::new(sqlite.clone());

        sqlite.with_connection(|conn| {
            conn.execute(
                "INSERT INTO repositories (repo_id, provider, organization, name, default_branch)
                 VALUES (?1, 'local', 'bitloops', 'demo', 'main')",
                rusqlite::params!["repo-1"],
            )?;
            conn.execute(
                "INSERT INTO artefacts_current (
                    repo_id, path, content_id, symbol_id, artefact_id, language,
                    extraction_fingerprint, canonical_kind, language_kind, symbol_fqn,
                    parent_symbol_id, parent_artefact_id, start_line, end_line,
                    start_byte, end_byte, signature, modifiers, docstring, updated_at
                ) VALUES (
                    ?1, 'src/a.rs', 'content-a', 'sym::a', 'artefact::a', 'rust',
                    'fingerprint-a', 'function', 'function_item',
                    'src/a.rs::a', NULL, NULL, 1, 2,
                    0, 12, NULL, '[]', NULL, '2026-04-28T10:00:00Z'
                )",
                rusqlite::params!["repo-1"],
            )?;
            conn.execute(
                "INSERT INTO artefacts_current (
                    repo_id, path, content_id, symbol_id, artefact_id, language,
                    extraction_fingerprint, canonical_kind, language_kind, symbol_fqn,
                    parent_symbol_id, parent_artefact_id, start_line, end_line,
                    start_byte, end_byte, signature, modifiers, docstring, updated_at
                ) VALUES (
                    ?1, 'src/b.rs', 'content-b', 'sym::b', 'artefact::b', 'rust',
                    'fingerprint-b', 'function', 'function_item',
                    'src/b.rs::b', NULL, NULL, 3, 4,
                    12, 24, NULL, '[]', NULL, '2026-04-28T10:00:00Z'
                )",
                rusqlite::params!["repo-1"],
            )?;
            conn.execute(
                "INSERT INTO artefact_edges_current (
                    repo_id, edge_id, path, content_id, from_symbol_id, from_artefact_id,
                    to_symbol_id, to_artefact_id, to_symbol_ref, edge_kind, language,
                    start_line, end_line, metadata, updated_at
                ) VALUES (
                    ?1, 'edge-a', 'src/a.rs', 'content-a', 'sym::a', 'artefact::a',
                    'sym::b', 'artefact::b', NULL, 'calls', 'rust',
                    1, 1, '{}', '2026-04-28T10:00:00Z'
                )",
                rusqlite::params!["repo-1"],
            )?;
            conn.execute(
                "INSERT INTO artefact_edges_current (
                    repo_id, edge_id, path, content_id, from_symbol_id, from_artefact_id,
                    to_symbol_id, to_artefact_id, to_symbol_ref, edge_kind, language,
                    start_line, end_line, metadata, updated_at
                ) VALUES (
                    ?1, 'edge-b', 'src/b.rs', 'content-b', 'sym::b', 'artefact::b',
                    NULL, NULL, 'external::c', 'calls', 'rust',
                    3, 3, '{}', '2026-04-28T10:00:00Z'
                )",
                rusqlite::params!["repo-1"],
            )?;
            Ok(())
        })?;

        let mut artefact_ids = Vec::new();
        gateway.visit_current_canonical_artefacts("repo-1", &mut |artefact| {
            artefact_ids.push(artefact.artefact_id);
            Ok(())
        })?;
        assert_eq!(artefact_ids, vec!["artefact::a", "artefact::b"]);

        let mut edge_ids = Vec::new();
        gateway.visit_current_canonical_edges("repo-1", &mut |edge| {
            edge_ids.push(edge.edge_id);
            Ok(())
        })?;
        assert_eq!(edge_ids, vec!["edge-a", "edge-b"]);

        Ok(())
    }

    #[test]
    fn current_canonical_visitors_propagate_visitor_errors() -> Result<()> {
        let temp = TempDir::new()?;
        let db_path = temp.path().join("runtime.sqlite");
        init_database(&db_path, false, "seed-commit")?;
        let sqlite = SqliteConnectionPool::connect_existing(db_path)?;
        let gateway = SqliteRelationalGateway::new(sqlite.clone());

        sqlite.with_connection(|conn| {
            conn.execute(
                "INSERT INTO repositories (repo_id, provider, organization, name, default_branch)
                 VALUES (?1, 'local', 'bitloops', 'demo', 'main')",
                rusqlite::params!["repo-1"],
            )?;
            conn.execute(
                "INSERT INTO artefact_edges_current (
                    repo_id, edge_id, path, content_id, from_symbol_id, from_artefact_id,
                    to_symbol_id, to_artefact_id, to_symbol_ref, edge_kind, language,
                    start_line, end_line, metadata, updated_at
                ) VALUES (
                    ?1, 'edge-a', 'src/a.rs', 'content-a', 'sym::a', 'artefact::a',
                    NULL, NULL, 'external::b', 'calls', 'rust',
                    1, 1, '{}', '2026-04-28T10:00:00Z'
                )",
                rusqlite::params!["repo-1"],
            )?;
            Ok(())
        })?;

        let error = gateway
            .visit_current_canonical_edges("repo-1", &mut |_edge| Err(anyhow::anyhow!("stop here")))
            .expect_err("visitor failure should bubble up");
        assert!(error.to_string().contains("stop here"));

        Ok(())
    }
}
