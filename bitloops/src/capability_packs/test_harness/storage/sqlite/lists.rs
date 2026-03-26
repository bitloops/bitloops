use anyhow::{Context, Result};
use rusqlite::{Connection, params};

use crate::models::ListedArtefactRecord;

pub(super) fn load_listed_production_artefacts(
    conn: &Connection,
    commit_sha: &str,
    kind: Option<&str>,
) -> Result<Vec<ListedArtefactRecord>> {
    let mut output = Vec::new();

    if let Some(kind) = kind {
        let mut stmt = conn
            .prepare(
                r#"
SELECT DISTINCT
  a.artefact_id,
  a.symbol_fqn,
  LOWER(COALESCE(a.canonical_kind, COALESCE(a.language_kind, 'unknown'))) AS kind,
  a.path,
  a.start_line,
  a.end_line
FROM file_state fs
JOIN artefacts a
  ON a.repo_id = fs.repo_id
 AND a.blob_sha = fs.blob_sha
 AND a.path = fs.path
WHERE fs.commit_sha = ?1
  AND LOWER(COALESCE(a.canonical_kind, COALESCE(a.language_kind, 'unknown'))) = LOWER(?2)
ORDER BY a.path ASC, a.start_line ASC
"#,
            )
            .context("failed preparing list query")?;
        let rows = stmt
            .query_map(params![commit_sha, kind], |row| {
                Ok(ListedArtefactRecord {
                    artefact_id: row.get(0)?,
                    symbol_fqn: row.get(1)?,
                    kind: row.get(2)?,
                    file_path: row.get(3)?,
                    start_line: row.get(4)?,
                    end_line: row.get(5)?,
                })
            })
            .context("failed querying production artefacts")?;
        for row in rows {
            output.push(row.context("failed decoding production list row")?);
        }
    } else {
        let mut stmt = conn
            .prepare(
                r#"
SELECT DISTINCT
  a.artefact_id,
  a.symbol_fqn,
  LOWER(COALESCE(a.canonical_kind, COALESCE(a.language_kind, 'unknown'))) AS kind,
  a.path,
  a.start_line,
  a.end_line
FROM file_state fs
JOIN artefacts a
  ON a.repo_id = fs.repo_id
 AND a.blob_sha = fs.blob_sha
 AND a.path = fs.path
WHERE fs.commit_sha = ?1
ORDER BY a.path ASC, a.start_line ASC
"#,
            )
            .context("failed preparing list query")?;
        let rows = stmt
            .query_map(params![commit_sha], |row| {
                Ok(ListedArtefactRecord {
                    artefact_id: row.get(0)?,
                    symbol_fqn: row.get(1)?,
                    kind: row.get(2)?,
                    file_path: row.get(3)?,
                    start_line: row.get(4)?,
                    end_line: row.get(5)?,
                })
            })
            .context("failed querying production artefacts")?;
        for row in rows {
            output.push(row.context("failed decoding production list row")?);
        }
    }

    Ok(output)
}

pub(super) fn load_listed_test_suites(
    conn: &Connection,
    commit_sha: &str,
) -> Result<Vec<ListedArtefactRecord>> {
    let mut stmt = conn
        .prepare(
            r#"
SELECT artefact_id, symbol_fqn, path, start_line, end_line
FROM test_artefacts_current
WHERE commit_sha = ?1
  AND canonical_kind = 'test_suite'
ORDER BY path ASC, start_line ASC
"#,
        )
        .context("failed preparing test suite list query")?;
    let rows = stmt
        .query_map(params![commit_sha], |row| {
            Ok(ListedArtefactRecord {
                artefact_id: row.get(0)?,
                symbol_fqn: row.get(1)?,
                kind: "test_suite".to_string(),
                file_path: row.get(2)?,
                start_line: row.get(3)?,
                end_line: row.get(4)?,
            })
        })
        .context("failed querying test suites")?;
    let mut output = Vec::new();
    for row in rows {
        output.push(row.context("failed decoding test suite list row")?);
    }
    Ok(output)
}

pub(super) fn load_listed_test_scenarios(
    conn: &Connection,
    commit_sha: &str,
) -> Result<Vec<ListedArtefactRecord>> {
    let mut stmt = conn
        .prepare(
            r#"
SELECT artefact_id, symbol_fqn, path, start_line, end_line
FROM test_artefacts_current
WHERE commit_sha = ?1
  AND canonical_kind = 'test_scenario'
ORDER BY path ASC, start_line ASC
"#,
        )
        .context("failed preparing test scenario list query")?;
    let rows = stmt
        .query_map(params![commit_sha], |row| {
            Ok(ListedArtefactRecord {
                artefact_id: row.get(0)?,
                symbol_fqn: row.get(1)?,
                kind: "test_scenario".to_string(),
                file_path: row.get(2)?,
                start_line: row.get(3)?,
                end_line: row.get(4)?,
            })
        })
        .context("failed querying test scenarios")?;
    let mut output = Vec::new();
    for row in rows {
        output.push(row.context("failed decoding test scenario list row")?);
    }
    Ok(output)
}
