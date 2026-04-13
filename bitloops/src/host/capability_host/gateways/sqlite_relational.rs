use anyhow::{Context, Result, bail};
use rusqlite::{OptionalExtension, params};

use super::RelationalGateway;
use crate::host::checkpoints::checkpoint_id::is_valid_checkpoint_id;
use crate::models::ProductionArtefact;
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
