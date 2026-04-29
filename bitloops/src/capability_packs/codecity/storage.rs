use std::path::Path;

use anyhow::{Context, Result};
use rusqlite::{OptionalExtension, params};
use sha2::{Digest, Sha256};

use crate::capability_packs::codecity::types::{
    CODECITY_DEFAULT_SNAPSHOT_KEY, CodeCityArchitectureDiagnosticsSnapshot,
    CodeCityBuildingHealthSummary, CodeCityHealthEvidence, CodeCityHealthMetrics,
    CodeCitySnapshotState, CodeCitySnapshotStatus, CodeCityWorldPayload,
};
use crate::host::relational_store::DefaultRelationalStore;
use crate::storage::SqliteConnectionPool;

mod architecture_diagnostics;
mod schema;
#[cfg(test)]
mod tests;

pub use schema::codecity_sqlite_schema_sql;

#[derive(Debug, Clone)]
pub struct SqliteCodeCityRepository {
    sqlite: SqliteConnectionPool,
}

#[derive(Debug, Clone)]
pub struct CodeCityStoredSnapshot {
    pub status: CodeCitySnapshotStatus,
    pub world: Option<CodeCityWorldPayload>,
    pub architecture_diagnostics: CodeCityArchitectureDiagnosticsSnapshot,
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
            .context("initialising CodeCity health schema")?;
        self.rebuild_legacy_architecture_diagnostics_tables_if_needed()?;
        self.sqlite
            .execute_batch(codecity_sqlite_schema_sql())
            .context("initialising CodeCity snapshot schema")
    }

    fn rebuild_legacy_architecture_diagnostics_tables_if_needed(&self) -> Result<()> {
        self.sqlite.with_connection(|conn| {
            for table in [
                "codecity_dependency_evidence_current",
                "codecity_file_dependency_arcs_current",
                "codecity_architecture_violations_current",
                "codecity_render_arcs_current",
            ] {
                if table_exists(conn, table)? && !table_has_column(conn, table, "snapshot_key")? {
                    conn.execute(&format!("DROP TABLE {table}"), [])?;
                }
            }
            Ok(())
        })
    }

    pub fn default_snapshot_key(project_path: Option<&str>) -> String {
        snapshot_key_for(project_path)
    }

    pub fn upsert_snapshot_request(
        &self,
        repo_id: &str,
        project_path: Option<&str>,
        config_fingerprint: &str,
        source_generation_seq: Option<u64>,
    ) -> Result<CodeCitySnapshotStatus> {
        let snapshot_key = snapshot_key_for(project_path);
        let now = chrono::Utc::now().to_rfc3339();
        self.sqlite.with_connection(|conn| {
            conn.execute(
                "INSERT INTO codecity_snapshots_current (
                    repo_id, snapshot_key, project_path, config_fingerprint,
                    source_generation_seq, state, stale, updated_at, last_error
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 1, ?7, NULL)
                ON CONFLICT (repo_id, snapshot_key) DO UPDATE SET
                    project_path = excluded.project_path,
                    config_fingerprint = excluded.config_fingerprint,
                    source_generation_seq = excluded.source_generation_seq,
                    state = excluded.state,
                    stale = 1,
                    updated_at = excluded.updated_at,
                    last_error = NULL",
                params![
                    repo_id,
                    &snapshot_key,
                    normalise_project_path(project_path).as_deref(),
                    config_fingerprint,
                    source_generation_seq.map(sql_i64),
                    CodeCitySnapshotState::Queued.as_str(),
                    &now,
                ],
            )
            .context("upserting CodeCity snapshot request")?;
            self.load_snapshot_status(repo_id, &snapshot_key, source_generation_seq)
        })
    }

    pub fn mark_snapshot_running(
        &self,
        repo_id: &str,
        snapshot_key: &str,
        run_id: &str,
        source_generation_seq: u64,
    ) -> Result<()> {
        let now = chrono::Utc::now().to_rfc3339();
        self.sqlite.with_connection(|conn| {
            conn.execute(
                "UPDATE codecity_snapshots_current
                 SET state = ?1, source_generation_seq = ?2, run_id = ?3,
                     stale = CASE
                         WHEN last_success_generation_seq IS NULL THEN 0
                         WHEN last_success_generation_seq < ?2 THEN 1
                         ELSE 0
                     END,
                     updated_at = ?4, last_error = NULL
                 WHERE repo_id = ?5 AND snapshot_key = ?6",
                params![
                    CodeCitySnapshotState::Running.as_str(),
                    sql_i64(source_generation_seq),
                    run_id,
                    now,
                    repo_id,
                    snapshot_key,
                ],
            )?;
            Ok(())
        })
    }

    pub fn mark_snapshot_failed(
        &self,
        repo_id: &str,
        snapshot_key: &str,
        source_generation_seq: u64,
        error: &str,
    ) -> Result<()> {
        let now = chrono::Utc::now().to_rfc3339();
        self.sqlite.with_connection(|conn| {
            conn.execute(
                "UPDATE codecity_snapshots_current
                 SET state = ?1, source_generation_seq = ?2,
                     stale = CASE WHEN world_json IS NULL THEN 0 ELSE 1 END,
                     updated_at = ?3, last_error = ?4
                 WHERE repo_id = ?5 AND snapshot_key = ?6",
                params![
                    CodeCitySnapshotState::Failed.as_str(),
                    sql_i64(source_generation_seq),
                    now,
                    error,
                    repo_id,
                    snapshot_key,
                ],
            )?;
            Ok(())
        })
    }

    pub fn replace_codecity_snapshot(
        &self,
        snapshot_key: &str,
        project_path: Option<&str>,
        source_generation_seq: u64,
        status_run_id: Option<&str>,
        world: &CodeCityWorldPayload,
        snapshot: &CodeCityArchitectureDiagnosticsSnapshot,
    ) -> Result<()> {
        self.replace_architecture_diagnostics_snapshot_for_key(snapshot_key, snapshot)?;
        let now = chrono::Utc::now().to_rfc3339();
        let world_json = serde_json::to_string(world)?;
        let run_id = status_run_id.unwrap_or(snapshot.run_id.as_str());
        self.sqlite.with_connection(|conn| {
            conn.execute(
                "INSERT INTO codecity_snapshots_current (
                    repo_id, snapshot_key, project_path, config_fingerprint,
                    source_generation_seq, last_success_generation_seq, state, stale,
                    run_id, commit_sha, generated_at, updated_at, last_error, world_json
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?5, ?6, 0, ?7, ?8, ?9, ?10, NULL, ?11)
                ON CONFLICT (repo_id, snapshot_key) DO UPDATE SET
                    project_path = excluded.project_path,
                    config_fingerprint = excluded.config_fingerprint,
                    source_generation_seq = excluded.source_generation_seq,
                    last_success_generation_seq = excluded.last_success_generation_seq,
                    state = excluded.state,
                    stale = 0,
                    run_id = excluded.run_id,
                    commit_sha = excluded.commit_sha,
                    generated_at = excluded.generated_at,
                    updated_at = excluded.updated_at,
                    last_error = NULL,
                    world_json = excluded.world_json",
                params![
                    world.repo_id,
                    snapshot_key,
                    normalise_project_path(project_path).as_deref(),
                    world.config_fingerprint,
                    sql_i64(source_generation_seq),
                    CodeCitySnapshotState::Ready.as_str(),
                    run_id,
                    snapshot.commit_sha,
                    world.health.generated_at,
                    now,
                    world_json,
                ],
            )
            .context("replacing CodeCity snapshot metadata")?;
            Ok(())
        })
    }

    pub fn load_codecity_snapshot(
        &self,
        repo_id: &str,
        snapshot_key: &str,
        latest_generation_seq: Option<u64>,
    ) -> Result<Option<CodeCityStoredSnapshot>> {
        let Some(status) = self.load_optional_snapshot_status(repo_id, snapshot_key)? else {
            return Ok(None);
        };
        let status = with_staleness(status, latest_generation_seq);
        let world = self.load_snapshot_world(repo_id, snapshot_key)?;
        let architecture_diagnostics =
            self.load_architecture_diagnostics_snapshot_for_key(repo_id, snapshot_key)?;
        Ok(Some(CodeCityStoredSnapshot {
            status,
            world,
            architecture_diagnostics,
        }))
    }

    pub fn load_snapshot_status(
        &self,
        repo_id: &str,
        snapshot_key: &str,
        latest_generation_seq: Option<u64>,
    ) -> Result<CodeCitySnapshotStatus> {
        let status = self
            .load_optional_snapshot_status(repo_id, snapshot_key)?
            .unwrap_or_else(|| missing_snapshot_status(repo_id, snapshot_key, None, ""));
        Ok(with_staleness(status, latest_generation_seq))
    }

    pub fn load_snapshot_status_or_missing(
        &self,
        repo_id: &str,
        snapshot_key: &str,
        project_path: Option<&str>,
        config_fingerprint: &str,
        latest_generation_seq: Option<u64>,
    ) -> Result<CodeCitySnapshotStatus> {
        let status = self
            .load_optional_snapshot_status(repo_id, snapshot_key)?
            .unwrap_or_else(|| {
                missing_snapshot_status(repo_id, snapshot_key, project_path, config_fingerprint)
            });
        Ok(with_staleness(status, latest_generation_seq))
    }

    pub fn load_snapshot_requests(&self, repo_id: &str) -> Result<Vec<CodeCitySnapshotStatus>> {
        self.sqlite.with_connection(|conn| {
            let mut stmt = conn.prepare(
                "SELECT snapshot_key, project_path, config_fingerprint, source_generation_seq,
                        last_success_generation_seq, state, stale, run_id, commit_sha,
                        generated_at, updated_at, last_error
                 FROM codecity_snapshots_current
                 WHERE repo_id = ?1
                 ORDER BY snapshot_key ASC",
            )?;
            let rows = stmt.query_map(params![repo_id], |row| {
                let state: String = row.get(5)?;
                Ok(CodeCitySnapshotStatus {
                    snapshot_key: row.get(0)?,
                    project_path: row.get(1)?,
                    config_fingerprint: row.get(2)?,
                    source_generation_seq: row
                        .get::<_, Option<i64>>(3)?
                        .and_then(|value| u64::try_from(value).ok()),
                    last_success_generation_seq: row
                        .get::<_, Option<i64>>(4)?
                        .and_then(|value| u64::try_from(value).ok()),
                    state: parse_snapshot_state(&state),
                    stale: row.get::<_, i64>(6)? != 0,
                    run_id: row.get(7)?,
                    commit_sha: row.get(8)?,
                    generated_at: row.get(9)?,
                    updated_at: row.get(10)?,
                    last_error: row.get(11)?,
                    repo_id: repo_id.to_string(),
                })
            })?;
            rows.collect::<std::result::Result<Vec<_>, _>>()
                .map_err(anyhow::Error::from)
        })
    }

    fn load_optional_snapshot_status(
        &self,
        repo_id: &str,
        snapshot_key: &str,
    ) -> Result<Option<CodeCitySnapshotStatus>> {
        self.sqlite.with_connection(|conn| {
            conn.query_row(
                "SELECT project_path, config_fingerprint, source_generation_seq,
                        last_success_generation_seq, state, stale, run_id, commit_sha,
                        generated_at, updated_at, last_error
                 FROM codecity_snapshots_current
                 WHERE repo_id = ?1 AND snapshot_key = ?2",
                params![repo_id, snapshot_key],
                |row| {
                    let state: String = row.get(4)?;
                    Ok(CodeCitySnapshotStatus {
                        state: parse_snapshot_state(&state),
                        stale: row.get::<_, i64>(5)? != 0,
                        repo_id: repo_id.to_string(),
                        project_path: row.get(0)?,
                        snapshot_key: snapshot_key.to_string(),
                        config_fingerprint: row.get(1)?,
                        source_generation_seq: row
                            .get::<_, Option<i64>>(2)?
                            .and_then(|value| u64::try_from(value).ok()),
                        last_success_generation_seq: row
                            .get::<_, Option<i64>>(3)?
                            .and_then(|value| u64::try_from(value).ok()),
                        run_id: row.get(6)?,
                        commit_sha: row.get(7)?,
                        generated_at: row.get(8)?,
                        updated_at: row.get(9)?,
                        last_error: row.get(10)?,
                    })
                },
            )
            .optional()
            .map_err(anyhow::Error::from)
        })
    }

    fn load_snapshot_world(
        &self,
        repo_id: &str,
        snapshot_key: &str,
    ) -> Result<Option<CodeCityWorldPayload>> {
        let raw = self.sqlite.with_connection(|conn| {
            conn.query_row(
                "SELECT world_json FROM codecity_snapshots_current
                 WHERE repo_id = ?1 AND snapshot_key = ?2",
                params![repo_id, snapshot_key],
                |row| row.get::<_, Option<String>>(0),
            )
            .optional()
            .map_err(anyhow::Error::from)
        })?;
        raw.flatten()
            .map(|value| serde_json::from_str(&value).context("decoding persisted CodeCity world"))
            .transpose()
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

fn table_exists(conn: &rusqlite::Connection, table: &str) -> Result<bool> {
    conn.query_row(
        "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1 LIMIT 1",
        params![table],
        |_| Ok(()),
    )
    .optional()
    .map(|value| value.is_some())
    .map_err(anyhow::Error::from)
}

fn table_has_column(conn: &rusqlite::Connection, table: &str, column: &str) -> Result<bool> {
    let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})"))?;
    let columns = stmt.query_map([], |row| row.get::<_, String>(1))?;
    for name in columns {
        if name? == column {
            return Ok(true);
        }
    }
    Ok(false)
}

fn sql_i64(value: u64) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

pub fn normalise_project_path(project_path: Option<&str>) -> Option<String> {
    project_path
        .map(str::trim)
        .filter(|value| !value.is_empty() && *value != ".")
        .map(|value| value.trim_matches('/').to_string())
        .filter(|value| !value.is_empty())
}

pub fn snapshot_key_for(project_path: Option<&str>) -> String {
    let Some(project_path) = normalise_project_path(project_path) else {
        return CODECITY_DEFAULT_SNAPSHOT_KEY.to_string();
    };
    let mut hasher = Sha256::new();
    hasher.update(project_path.as_bytes());
    let digest = hex::encode(hasher.finalize());
    format!("project:{}", &digest[..16])
}

pub fn missing_snapshot_status(
    repo_id: &str,
    snapshot_key: &str,
    project_path: Option<&str>,
    config_fingerprint: &str,
) -> CodeCitySnapshotStatus {
    CodeCitySnapshotStatus {
        state: CodeCitySnapshotState::Missing,
        stale: false,
        repo_id: repo_id.to_string(),
        project_path: normalise_project_path(project_path),
        snapshot_key: snapshot_key.to_string(),
        config_fingerprint: config_fingerprint.to_string(),
        ..CodeCitySnapshotStatus::default()
    }
}

pub fn with_staleness(
    mut status: CodeCitySnapshotStatus,
    latest_generation_seq: Option<u64>,
) -> CodeCitySnapshotStatus {
    if let (Some(latest), Some(applied)) =
        (latest_generation_seq, status.last_success_generation_seq)
    {
        status.stale = applied < latest;
    }
    status
}

fn parse_snapshot_state(value: &str) -> CodeCitySnapshotState {
    match value {
        "queued" => CodeCitySnapshotState::Queued,
        "running" => CodeCitySnapshotState::Running,
        "ready" => CodeCitySnapshotState::Ready,
        "failed" => CodeCitySnapshotState::Failed,
        _ => CodeCitySnapshotState::Missing,
    }
}
