//! Stage-serving queries for `tests()` / `coverage()` DevQL stages (SQLite).

use anyhow::{Context, Result};
use rusqlite::{Connection, OptionalExtension, params};

use crate::models::{
    StageBranchCoverageRecord, StageCoverageMetadataRecord, StageCoveringTestRecord,
    StageLineCoverageRecord,
};

pub(super) fn load_stage_covering_tests(
    conn: &Connection,
    repo_id: &str,
    production_artefact_id: &str,
    min_confidence: Option<f64>,
    linkage_source: Option<&str>,
    limit: usize,
) -> Result<Vec<StageCoveringTestRecord>> {
    let mut sql = String::from(
        "SELECT ts.scenario_id AS test_id, ts.name AS test_name, \
         su.name AS suite_name, ts.path AS file_path, \
         tl.confidence, ts.discovery_source, \
         tl.link_source AS linkage_source, tl.linkage_status \
         FROM test_links tl \
         JOIN test_scenarios ts ON ts.scenario_id = tl.test_scenario_id \
         LEFT JOIN test_suites su ON su.suite_id = ts.suite_id \
         WHERE tl.repo_id = ?1 AND tl.production_artefact_id = ?2",
    );
    let mut param_idx = 3;
    if min_confidence.is_some() {
        sql.push_str(&format!(" AND tl.confidence >= ?{param_idx}"));
        param_idx += 1;
    }
    if linkage_source.is_some() {
        sql.push_str(&format!(" AND tl.link_source = ?{param_idx}"));
    }
    sql.push_str(&format!(
        " ORDER BY tl.confidence DESC, ts.path, ts.name LIMIT {}",
        limit.max(1)
    ));

    let mut stmt = conn
        .prepare(&sql)
        .context("failed preparing stage covering tests query")?;

    let mut params_vec: Vec<Box<dyn rusqlite::types::ToSql>> = vec![
        Box::new(repo_id.to_string()),
        Box::new(production_artefact_id.to_string()),
    ];
    if let Some(mc) = min_confidence {
        params_vec.push(Box::new(mc));
    }
    if let Some(ls) = linkage_source {
        params_vec.push(Box::new(ls.to_string()));
    }
    let param_refs: Vec<&dyn rusqlite::types::ToSql> =
        params_vec.iter().map(|p| p.as_ref()).collect();

    let rows = stmt
        .query_map(param_refs.as_slice(), |row| {
            Ok(StageCoveringTestRecord {
                test_id: row.get(0)?,
                test_name: row.get(1)?,
                suite_name: row.get(2)?,
                file_path: row.get(3)?,
                confidence: row.get(4)?,
                discovery_source: row.get(5)?,
                linkage_source: row.get(6)?,
                linkage_status: row.get(7)?,
            })
        })
        .context("failed querying stage covering tests")?;

    let mut result = Vec::new();
    for row in rows {
        result.push(row.context("failed decoding stage covering test row")?);
    }
    Ok(result)
}

pub(super) fn load_stage_line_coverage(
    conn: &Connection,
    repo_id: &str,
    artefact_id: &str,
    commit_sha: Option<&str>,
) -> Result<Vec<StageLineCoverageRecord>> {
    let mut sql = String::from(
        "SELECT ch.line, MAX(CASE WHEN ch.covered = 1 THEN 1 ELSE 0 END) AS covered_any \
         FROM coverage_hits ch \
         JOIN coverage_captures cc ON cc.capture_id = ch.capture_id \
         WHERE cc.repo_id = ?1 AND ch.production_artefact_id = ?2 AND ch.branch_id = -1",
    );
    if commit_sha.is_some() {
        sql.push_str(" AND cc.commit_sha = ?3");
    }
    sql.push_str(" GROUP BY ch.line ORDER BY ch.line");

    let mut stmt = conn
        .prepare(&sql)
        .context("failed preparing stage line coverage query")?;

    let mut result = Vec::new();
    if let Some(sha) = commit_sha {
        let rows = stmt
            .query_map(params![repo_id, artefact_id, sha], |row| {
                Ok(StageLineCoverageRecord {
                    line: row.get(0)?,
                    covered: row.get::<_, i64>(1)? == 1,
                })
            })
            .context("failed querying stage line coverage")?;
        for row in rows {
            result.push(row.context("failed decoding stage line coverage row")?);
        }
    } else {
        let rows = stmt
            .query_map(params![repo_id, artefact_id], |row| {
                Ok(StageLineCoverageRecord {
                    line: row.get(0)?,
                    covered: row.get::<_, i64>(1)? == 1,
                })
            })
            .context("failed querying stage line coverage")?;
        for row in rows {
            result.push(row.context("failed decoding stage line coverage row")?);
        }
    }
    Ok(result)
}

pub(super) fn load_stage_branch_coverage(
    conn: &Connection,
    repo_id: &str,
    artefact_id: &str,
    commit_sha: Option<&str>,
) -> Result<Vec<StageBranchCoverageRecord>> {
    let mut sql = String::from(
        "SELECT ch.line, ch.branch_id, \
         MAX(CASE WHEN ch.covered = 1 THEN 1 ELSE 0 END) AS covered_any, \
         MAX(ch.hit_count) AS hit_count \
         FROM coverage_hits ch \
         JOIN coverage_captures cc ON cc.capture_id = ch.capture_id \
         WHERE cc.repo_id = ?1 AND ch.production_artefact_id = ?2 AND ch.branch_id != -1",
    );
    if commit_sha.is_some() {
        sql.push_str(" AND cc.commit_sha = ?3");
    }
    sql.push_str(" GROUP BY ch.line, ch.branch_id ORDER BY ch.line, ch.branch_id");

    let mut stmt = conn
        .prepare(&sql)
        .context("failed preparing stage branch coverage query")?;

    let mut result = Vec::new();
    if let Some(sha) = commit_sha {
        let rows = stmt
            .query_map(params![repo_id, artefact_id, sha], |row| {
                Ok(StageBranchCoverageRecord {
                    line: row.get(0)?,
                    branch_id: row.get(1)?,
                    covered: row.get::<_, i64>(2)? == 1,
                    hit_count: row.get(3)?,
                })
            })
            .context("failed querying stage branch coverage")?;
        for row in rows {
            result.push(row.context("failed decoding stage branch coverage row")?);
        }
    } else {
        let rows = stmt
            .query_map(params![repo_id, artefact_id], |row| {
                Ok(StageBranchCoverageRecord {
                    line: row.get(0)?,
                    branch_id: row.get(1)?,
                    covered: row.get::<_, i64>(2)? == 1,
                    hit_count: row.get(3)?,
                })
            })
            .context("failed querying stage branch coverage")?;
        for row in rows {
            result.push(row.context("failed decoding stage branch coverage row")?);
        }
    }
    Ok(result)
}

pub(super) fn load_stage_coverage_metadata(
    conn: &Connection,
    repo_id: &str,
    commit_sha: Option<&str>,
) -> Result<Option<StageCoverageMetadataRecord>> {
    let mut sql = String::from(
        "SELECT cc.format AS coverage_source, cc.branch_truth \
         FROM coverage_captures cc \
         WHERE cc.repo_id = ?1",
    );
    if commit_sha.is_some() {
        sql.push_str(" AND cc.commit_sha = ?2");
    }
    sql.push_str(" LIMIT 1");

    let mut stmt = conn
        .prepare(&sql)
        .context("failed preparing stage coverage metadata query")?;

    let result = if let Some(sha) = commit_sha {
        stmt.query_row(params![repo_id, sha], |row| {
            Ok(StageCoverageMetadataRecord {
                coverage_source: row.get(0)?,
                branch_truth: row.get(1)?,
            })
        })
        .optional()
        .context("failed querying stage coverage metadata")?
    } else {
        stmt.query_row(params![repo_id], |row| {
            Ok(StageCoverageMetadataRecord {
                coverage_source: row.get(0)?,
                branch_truth: row.get(1)?,
            })
        })
        .optional()
        .context("failed querying stage coverage metadata")?
    };

    Ok(result)
}
