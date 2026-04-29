use std::path::Path;

use anyhow::{Context, Result};
use rusqlite::{OptionalExtension, params};

use crate::capability_packs::codecity::types::{
    CodeCityBuildingHealthSummary, CodeCityHealthEvidence, CodeCityHealthMetrics,
    CodeCityWorldPayload,
};
use crate::host::relational_store::DefaultRelationalStore;
use crate::storage::SqliteConnectionPool;

mod phase4;
mod schema;
#[cfg(test)]
mod tests;

pub use schema::codecity_sqlite_schema_sql;

#[derive(Debug, Clone)]
pub struct SqliteCodeCityRepository {
    sqlite: SqliteConnectionPool,
}

impl SqliteCodeCityRepository {
    pub fn open_for_repo_root(repo_root: &Path) -> Result<Self> {
        let relational = DefaultRelationalStore::open_local_for_repo_root(repo_root)
            .context("opening local relational store for CodeCity health")?;
        Ok(Self {
            sqlite: relational.local_sqlite_pool_allow_create()?,
        })
    }

    pub fn from_sqlite(sqlite: SqliteConnectionPool) -> Self {
        Self { sqlite }
    }

    pub fn initialise_schema(&self) -> Result<()> {
        self.sqlite
            .execute_batch(codecity_sqlite_schema_sql())
            .context("initialising CodeCity health schema")
    }

    pub fn try_apply_current_snapshot(&self, world: &mut CodeCityWorldPayload) -> Result<bool> {
        let floor_count = world
            .buildings
            .iter()
            .map(|building| building.floors.len())
            .sum::<usize>();
        if floor_count == 0 {
            return Ok(false);
        }

        let repo_id = world.repo_id.clone();
        let fingerprint = world.config_fingerprint.clone();
        let commit_sha = world.commit_sha.clone();
        let run_health = self.sqlite.with_connection(|conn| {
            conn.query_row(
                "SELECT health_summary_json FROM codecity_health_runs_current \
                 WHERE repo_id = ?1 AND config_fingerprint = ?2 AND \
                 COALESCE(commit_sha, '') = COALESCE(?3, '')",
                params![repo_id, fingerprint, commit_sha],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(anyhow::Error::from)
        })?;
        let Some(run_health) = run_health else {
            return Ok(false);
        };

        let floor_rows = self.load_floor_rows(
            &world.repo_id,
            &world.config_fingerprint,
            world.commit_sha.as_deref(),
        )?;
        if floor_rows.len() != floor_count {
            return Ok(false);
        }
        let file_rows = self.load_file_rows(
            &world.repo_id,
            &world.config_fingerprint,
            world.commit_sha.as_deref(),
        )?;

        for building in &mut world.buildings {
            if let Some(row) = file_rows.iter().find(|row| row.path == building.path) {
                building.health_risk = row.health_risk;
                building.health_status = row.health_status.clone();
                building.health_confidence = row.health_confidence;
                building.colour = row.colour.clone();
                building.health_summary = row.summary.clone();
            }
            for floor in &mut building.floors {
                if let Some(row) = floor_rows
                    .iter()
                    .find(|row| row.path == building.path && row.floor_index == floor.floor_index)
                {
                    floor.health_risk = row.health_risk;
                    floor.health_status = row.health_status.clone();
                    floor.health_confidence = row.health_confidence;
                    floor.colour = row.colour.clone();
                    floor.health_metrics = row.metrics.clone();
                    floor.health_evidence = row.evidence.clone();
                }
            }
        }

        world.health = serde_json::from_str(&run_health)
            .context("decoding persisted CodeCity health overview")?;
        world.summary.coverage_available = world.health.coverage_available;
        world.summary.git_history_available = world.health.git_history_available;
        world.summary.unhealthy_floor_count = world
            .buildings
            .iter()
            .flat_map(|building| building.floors.iter())
            .filter(|floor| floor.health_risk.is_some_and(|risk| risk >= 0.7))
            .count();
        world.summary.insufficient_health_data_count = world
            .buildings
            .iter()
            .flat_map(|building| building.floors.iter())
            .filter(|floor| floor.health_status == "insufficient_data")
            .count();
        Ok(true)
    }

    pub fn replace_current_snapshot(&self, world: &CodeCityWorldPayload) -> Result<()> {
        let updated_at = world
            .health
            .generated_at
            .clone()
            .unwrap_or_else(|| chrono::Utc::now().to_rfc3339());
        let commit_sha = world.commit_sha.as_deref();
        self.sqlite.with_connection(|conn| {
            conn.execute(
                "DELETE FROM codecity_floor_health_current WHERE repo_id = ?1",
                params![world.repo_id],
            )?;
            conn.execute(
                "DELETE FROM codecity_file_health_current WHERE repo_id = ?1",
                params![world.repo_id],
            )?;
            conn.execute(
                "DELETE FROM codecity_health_runs_current WHERE repo_id = ?1",
                params![world.repo_id],
            )?;

            for building in &world.buildings {
                conn.execute(
                    "INSERT INTO codecity_file_health_current (
                        repo_id, path, commit_sha, config_fingerprint, health_risk,
                        health_status, health_confidence, colour, floor_count,
                        high_risk_floor_count, insufficient_data_floor_count,
                        average_floor_risk, max_floor_risk, missing_signals_json,
                        summary_json, updated_at
                    ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)",
                    params![
                        world.repo_id,
                        building.path,
                        commit_sha,
                        world.config_fingerprint,
                        building.health_risk,
                        building.health_status,
                        building.health_confidence,
                        building.colour,
                        building.health_summary.floor_count as i64,
                        building.health_summary.high_risk_floor_count as i64,
                        building.health_summary.insufficient_data_floor_count as i64,
                        building.health_summary.average_risk,
                        building.health_summary.max_risk,
                        serde_json::to_string(&building.health_summary.missing_signals)?,
                        serde_json::to_string(&building.health_summary)?,
                        updated_at,
                    ],
                )?;

                for floor in &building.floors {
                    conn.execute(
                        "INSERT INTO codecity_floor_health_current (
                            repo_id, path, floor_index, artefact_id, symbol_id, commit_sha,
                            config_fingerprint, health_risk, health_status, health_confidence,
                            colour, churn, complexity, bug_count, coverage,
                            author_concentration, distinct_authors, commits_touching,
                            bug_fix_commits, covered_lines, total_coverable_lines,
                            complexity_source, coverage_source, git_history_source,
                            missing_signals_json, updated_at
                        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25, ?26)",
                        params![
                            world.repo_id,
                            building.path,
                            floor.floor_index as i64,
                            floor.artefact_id,
                            floor.symbol_id,
                            commit_sha,
                            world.config_fingerprint,
                            floor.health_risk,
                            floor.health_status,
                            floor.health_confidence,
                            floor.colour,
                            floor.health_metrics.churn as i64,
                            floor.health_metrics.complexity,
                            floor.health_metrics.bug_count as i64,
                            floor.health_metrics.coverage,
                            floor.health_metrics.author_concentration,
                            floor.health_evidence.distinct_authors as i64,
                            floor.health_evidence.commits_touching as i64,
                            floor.health_evidence.bug_fix_commits as i64,
                            floor.health_evidence.covered_lines.map(|value| value as i64),
                            floor.health_evidence.total_coverable_lines.map(|value| value as i64),
                            floor.health_evidence.complexity_source,
                            floor.health_evidence.coverage_source,
                            floor.health_evidence.git_history_source,
                            serde_json::to_string(&floor.health_evidence.missing_signals)?,
                            updated_at,
                        ],
                    )?;
                }
            }

            conn.execute(
                "INSERT INTO codecity_health_runs_current (
                    repo_id, commit_sha, config_fingerprint, health_status,
                    health_generated_at, health_summary_json, updated_at
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    world.repo_id,
                    commit_sha,
                    world.config_fingerprint,
                    world.health.status,
                    world.health.generated_at,
                    serde_json::to_string(&world.health)?,
                    updated_at,
                ],
            )?;

            Ok(())
        })
    }

    fn load_floor_rows(
        &self,
        repo_id: &str,
        fingerprint: &str,
        commit_sha: Option<&str>,
    ) -> Result<Vec<PersistedFloorHealth>> {
        self.sqlite.with_connection(|conn| {
            let mut stmt = conn.prepare(
                "SELECT path, floor_index, health_risk, health_status, health_confidence, colour,
                        churn, complexity, bug_count, coverage, author_concentration,
                        distinct_authors, commits_touching, bug_fix_commits, covered_lines,
                        total_coverable_lines, complexity_source, coverage_source,
                        git_history_source, missing_signals_json
                 FROM codecity_floor_health_current
                 WHERE repo_id = ?1 AND config_fingerprint = ?2
                   AND COALESCE(commit_sha, '') = COALESCE(?3, '')
                 ORDER BY path ASC, floor_index ASC",
            )?;
            let rows = stmt.query_map(params![repo_id, fingerprint, commit_sha], |row| {
                let missing: String = row.get(19)?;
                Ok(PersistedFloorHealth {
                    path: row.get(0)?,
                    floor_index: row.get::<_, i64>(1)? as usize,
                    health_risk: row.get(2)?,
                    health_status: row.get(3)?,
                    health_confidence: row.get(4)?,
                    colour: row.get(5)?,
                    metrics: CodeCityHealthMetrics {
                        churn: row.get::<_, i64>(6)? as u64,
                        complexity: row.get(7)?,
                        bug_count: row.get::<_, i64>(8)? as u64,
                        coverage: row.get(9)?,
                        author_concentration: row.get(10)?,
                    },
                    evidence: CodeCityHealthEvidence {
                        distinct_authors: row.get::<_, i64>(11)? as u64,
                        commits_touching: row.get::<_, i64>(12)? as u64,
                        bug_fix_commits: row.get::<_, i64>(13)? as u64,
                        covered_lines: row.get::<_, Option<i64>>(14)?.map(|value| value as u64),
                        total_coverable_lines: row
                            .get::<_, Option<i64>>(15)?
                            .map(|value| value as u64),
                        complexity_source: row.get(16)?,
                        coverage_source: row.get(17)?,
                        git_history_source: row.get(18)?,
                        missing_signals: serde_json::from_str(&missing).unwrap_or_default(),
                    },
                })
            })?;

            rows.collect::<std::result::Result<Vec<_>, _>>()
                .map_err(anyhow::Error::from)
        })
    }

    fn load_file_rows(
        &self,
        repo_id: &str,
        fingerprint: &str,
        commit_sha: Option<&str>,
    ) -> Result<Vec<PersistedFileHealth>> {
        self.sqlite.with_connection(|conn| {
            let mut stmt = conn.prepare(
                "SELECT path, health_risk, health_status, health_confidence, colour, summary_json
                 FROM codecity_file_health_current
                 WHERE repo_id = ?1 AND config_fingerprint = ?2
                   AND COALESCE(commit_sha, '') = COALESCE(?3, '')
                 ORDER BY path ASC",
            )?;
            let rows = stmt.query_map(params![repo_id, fingerprint, commit_sha], |row| {
                let summary_json: String = row.get(5)?;
                Ok(PersistedFileHealth {
                    path: row.get(0)?,
                    health_risk: row.get(1)?,
                    health_status: row.get(2)?,
                    health_confidence: row.get(3)?,
                    colour: row.get(4)?,
                    summary: serde_json::from_str::<CodeCityBuildingHealthSummary>(&summary_json)
                        .unwrap_or_default(),
                })
            })?;

            rows.collect::<std::result::Result<Vec<_>, _>>()
                .map_err(anyhow::Error::from)
        })
    }
}

#[derive(Debug, Clone)]
struct PersistedFloorHealth {
    path: String,
    floor_index: usize,
    health_risk: Option<f64>,
    health_status: String,
    health_confidence: f64,
    colour: String,
    metrics: CodeCityHealthMetrics,
    evidence: CodeCityHealthEvidence,
}

#[derive(Debug, Clone)]
struct PersistedFileHealth {
    path: String,
    health_risk: Option<f64>,
    health_status: String,
    health_confidence: f64,
    colour: String,
    summary: CodeCityBuildingHealthSummary,
}
