//! Stage-serving queries for `tests()` / `coverage()` DevQL stages (Postgres).

use anyhow::{Context, Result};

use crate::models::{
    StageBranchCoverageRecord, StageCoverageMetadataRecord, StageCoveringTestRecord,
    StageLineCoverageRecord,
};

use super::helpers::{get, get_i64};

pub(super) async fn load_stage_covering_tests(
    client: &mut tokio_postgres::Client,
    repo_id: String,
    production_artefact_id: String,
    linkage_source_owned: Option<String>,
    min_confidence: Option<f64>,
    limit: usize,
) -> Result<Vec<StageCoveringTestRecord>> {
    let limit = limit.max(1);
    let mut sql = String::from(
        "SELECT ts.scenario_id AS test_id, ts.name AS test_name, \
         su.name AS suite_name, ts.path AS file_path, \
         tl.confidence, ts.discovery_source, \
         tl.link_source AS linkage_source, tl.linkage_status \
         FROM test_links tl \
         JOIN test_scenarios ts ON ts.scenario_id = tl.test_scenario_id \
         LEFT JOIN test_suites su ON su.suite_id = ts.suite_id \
         WHERE tl.repo_id = $1 AND tl.production_artefact_id = $2",
    );
    let mut next_param = 3usize;
    if min_confidence.is_some() {
        sql.push_str(&format!(" AND tl.confidence >= ${next_param}"));
        next_param += 1;
    }
    if linkage_source_owned.is_some() {
        sql.push_str(&format!(" AND tl.link_source = ${next_param}"));
    }
    sql.push_str(&format!(
        " ORDER BY tl.confidence DESC, ts.path, ts.name LIMIT {limit}"
    ));

    let rows = match (min_confidence, linkage_source_owned.as_deref()) {
        (Some(mc), Some(ls)) => {
            client
                .query(&sql, &[&repo_id, &production_artefact_id, &mc, &ls])
                .await
        }
        (Some(mc), None) => {
            client
                .query(&sql, &[&repo_id, &production_artefact_id, &mc])
                .await
        }
        (None, Some(ls)) => {
            client
                .query(&sql, &[&repo_id, &production_artefact_id, &ls])
                .await
        }
        (None, None) => {
            client
                .query(&sql, &[&repo_id, &production_artefact_id])
                .await
        }
    }
    .context("failed querying stage covering tests")?;

    rows.into_iter()
        .map(|row| {
            Ok(StageCoveringTestRecord {
                test_id: get(&row, 0, "test_id")?,
                test_name: get(&row, 1, "test_name")?,
                suite_name: row.try_get::<_, Option<String>>(2).context("suite_name")?,
                file_path: get(&row, 3, "file_path")?,
                confidence: row.try_get::<_, f64>(4).context("confidence")?,
                discovery_source: get(&row, 5, "discovery_source")?,
                linkage_source: get(&row, 6, "linkage_source")?,
                linkage_status: get(&row, 7, "linkage_status")?,
            })
        })
        .collect()
}

pub(super) async fn load_stage_line_coverage(
    client: &mut tokio_postgres::Client,
    repo_id: String,
    artefact_id: String,
    commit_sha: Option<String>,
) -> Result<Vec<StageLineCoverageRecord>> {
    let sql_no_commit = concat!(
        "SELECT ch.line, MAX(CASE WHEN ch.covered = 1 THEN 1 ELSE 0 END) AS covered_any ",
        "FROM coverage_hits ch ",
        "JOIN coverage_captures cc ON cc.capture_id = ch.capture_id ",
        "WHERE cc.repo_id = $1 AND ch.production_artefact_id = $2 AND ch.branch_id = -1 ",
        "GROUP BY ch.line ORDER BY ch.line",
    );
    let sql_with_commit = concat!(
        "SELECT ch.line, MAX(CASE WHEN ch.covered = 1 THEN 1 ELSE 0 END) AS covered_any ",
        "FROM coverage_hits ch ",
        "JOIN coverage_captures cc ON cc.capture_id = ch.capture_id ",
        "WHERE cc.repo_id = $1 AND ch.production_artefact_id = $2 ",
        "AND ch.branch_id = -1 AND cc.commit_sha = $3 ",
        "GROUP BY ch.line ORDER BY ch.line",
    );

    let rows = if let Some(ref sha) = commit_sha {
        client
            .query(sql_with_commit, &[&repo_id, &artefact_id, sha])
            .await
    } else {
        client.query(sql_no_commit, &[&repo_id, &artefact_id]).await
    }
    .context("failed querying stage line coverage")?;

    rows.into_iter()
        .map(|row| {
            Ok(StageLineCoverageRecord {
                line: get_i64(&row, 0, "line")?,
                covered: get_i64(&row, 1, "covered_any")? == 1,
            })
        })
        .collect()
}

pub(super) async fn load_stage_branch_coverage(
    client: &mut tokio_postgres::Client,
    repo_id: String,
    artefact_id: String,
    commit_sha: Option<String>,
) -> Result<Vec<StageBranchCoverageRecord>> {
    let sql_no_commit = concat!(
        "SELECT ch.line, ch.branch_id, ",
        "MAX(CASE WHEN ch.covered = 1 THEN 1 ELSE 0 END) AS covered_any, ",
        "MAX(ch.hit_count) AS hit_count ",
        "FROM coverage_hits ch ",
        "JOIN coverage_captures cc ON cc.capture_id = ch.capture_id ",
        "WHERE cc.repo_id = $1 AND ch.production_artefact_id = $2 AND ch.branch_id != -1 ",
        "GROUP BY ch.line, ch.branch_id ORDER BY ch.line, ch.branch_id",
    );
    let sql_with_commit = concat!(
        "SELECT ch.line, ch.branch_id, ",
        "MAX(CASE WHEN ch.covered = 1 THEN 1 ELSE 0 END) AS covered_any, ",
        "MAX(ch.hit_count) AS hit_count ",
        "FROM coverage_hits ch ",
        "JOIN coverage_captures cc ON cc.capture_id = ch.capture_id ",
        "WHERE cc.repo_id = $1 AND ch.production_artefact_id = $2 ",
        "AND ch.branch_id != -1 AND cc.commit_sha = $3 ",
        "GROUP BY ch.line, ch.branch_id ORDER BY ch.line, ch.branch_id",
    );

    let rows = if let Some(ref sha) = commit_sha {
        client
            .query(sql_with_commit, &[&repo_id, &artefact_id, sha])
            .await
    } else {
        client.query(sql_no_commit, &[&repo_id, &artefact_id]).await
    }
    .context("failed querying stage branch coverage")?;

    rows.into_iter()
        .map(|row| {
            Ok(StageBranchCoverageRecord {
                line: get_i64(&row, 0, "line")?,
                branch_id: get_i64(&row, 1, "branch_id")?,
                covered: get_i64(&row, 2, "covered_any")? == 1,
                hit_count: get_i64(&row, 3, "hit_count")?,
            })
        })
        .collect()
}

pub(super) async fn load_stage_coverage_metadata(
    client: &mut tokio_postgres::Client,
    repo_id: String,
    commit_sha: Option<String>,
) -> Result<Option<StageCoverageMetadataRecord>> {
    let row_opt = if let Some(ref sha) = commit_sha {
        client
            .query_opt(
                "SELECT cc.format AS coverage_source, cc.branch_truth \
                 FROM coverage_captures cc \
                 WHERE cc.repo_id = $1 AND cc.commit_sha = $2 \
                 LIMIT 1",
                &[&repo_id, sha],
            )
            .await
    } else {
        client
            .query_opt(
                "SELECT cc.format AS coverage_source, cc.branch_truth \
                 FROM coverage_captures cc \
                 WHERE cc.repo_id = $1 \
                 LIMIT 1",
                &[&repo_id],
            )
            .await
    }
    .context("failed querying stage coverage metadata")?;

    match row_opt {
        Some(row) => Ok(Some(StageCoverageMetadataRecord {
            coverage_source: get(&row, 0, "coverage_source")?,
            branch_truth: get_i64(&row, 1, "branch_truth")?,
        })),
        None => Ok(None),
    }
}
