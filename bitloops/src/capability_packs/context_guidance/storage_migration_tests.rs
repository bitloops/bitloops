use std::path::{Path, PathBuf};

use super::*;
use crate::capability_packs::context_guidance::migrations;
use crate::host::capability_host::{CapabilityMigrationContext, MigrationRunner};
use crate::host::devql::RepoIdentity;

struct MigrationTestContext {
    repo: RepoIdentity,
    repo_root: PathBuf,
    sqlite: SqliteConnectionPool,
}

impl CapabilityMigrationContext for MigrationTestContext {
    fn repo(&self) -> &RepoIdentity {
        &self.repo
    }

    fn repo_root(&self) -> &Path {
        &self.repo_root
    }

    fn apply_devql_sqlite_ddl(&self, sql: &str) -> Result<()> {
        self.sqlite.execute_batch(sql)
    }

    fn apply_devql_sqlite_migration(
        &self,
        operation: &mut dyn FnMut(&rusqlite::Connection) -> Result<()>,
    ) -> Result<()> {
        self.sqlite.with_write_connection(|conn| operation(conn))
    }
}

#[test]
fn migration_initializes_tables_indexes_and_attribution_columns() {
    let temp = tempfile::NamedTempFile::new().expect("temp db");
    let path = temp.into_temp_path().keep().expect("keep temp db");
    let sqlite = SqliteConnectionPool::connect(path).expect("sqlite");
    let mut ctx = MigrationTestContext {
        repo: RepoIdentity {
            provider: "local".to_string(),
            organization: "bitloops".to_string(),
            name: "repo".to_string(),
            identity: "local/repo".to_string(),
            repo_id: "repo-1".to_string(),
        },
        repo_root: PathBuf::from("."),
        sqlite: sqlite.clone(),
    };

    match migrations::CONTEXT_GUIDANCE_MIGRATIONS[0].run {
        MigrationRunner::Core(run) => run(&mut ctx).expect("migration"),
        MigrationRunner::Knowledge(_) => panic!("context guidance migration must be core"),
    }

    let table_names = [
        "context_guidance_distillation_runs",
        "context_guidance_facts",
        "context_guidance_sources",
        "context_guidance_targets",
    ];
    let index_names = [
        "context_guidance_runs_scope_input_idx",
        "context_guidance_runs_scope_idx",
        "context_guidance_facts_repo_category_idx",
        "context_guidance_facts_run_idx",
        "context_guidance_sources_guidance_idx",
        "context_guidance_sources_history_idx",
        "context_guidance_sources_filter_idx",
        "context_guidance_sources_knowledge_idx",
        "context_guidance_targets_lookup_idx",
        "context_guidance_targets_guidance_idx",
    ];

    sqlite
        .with_connection(|conn| {
            for table in table_names {
                let count: i64 = conn.query_row(
                    "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
                    params![table],
                    |row| row.get(0),
                )?;
                assert_eq!(count, 1, "missing table {table}");
            }
            for index in index_names {
                let count: i64 = conn.query_row(
                    "SELECT COUNT(*) FROM sqlite_master WHERE type = 'index' AND name = ?1",
                    params![index],
                    |row| row.get(0),
                )?;
                assert_eq!(count, 1, "missing index {index}");
            }
            let columns = conn
                .prepare("PRAGMA table_info(context_guidance_distillation_runs)")?
                .query_map([], |row| row.get::<_, String>(1))?
                .collect::<Result<Vec<_>, _>>()?;
            assert!(columns.iter().any(|column| column == "capability_id"));
            assert!(columns.iter().any(|column| column == "capability_version"));
            Ok(())
        })
        .expect("inspect schema");
}

#[test]
fn lifecycle_migration_adds_status_fingerprint_and_compaction_tables() {
    let temp = tempfile::NamedTempFile::new().expect("temp db");
    let path = temp.into_temp_path().keep().expect("keep temp db");
    let sqlite = SqliteConnectionPool::connect(path).expect("sqlite");
    let mut ctx = MigrationTestContext {
        repo: RepoIdentity {
            provider: "local".to_string(),
            organization: "bitloops".to_string(),
            name: "repo".to_string(),
            identity: "local/repo".to_string(),
            repo_id: "repo-1".to_string(),
        },
        repo_root: PathBuf::from("."),
        sqlite: sqlite.clone(),
    };

    for migration in migrations::CONTEXT_GUIDANCE_MIGRATIONS {
        match migration.run {
            MigrationRunner::Core(run) => run(&mut ctx).expect("migration"),
            MigrationRunner::Knowledge(_) => panic!("context guidance migration must be core"),
        }
    }

    let columns = sqlite
        .with_connection(|conn| {
            let mut stmt = conn
                .prepare("PRAGMA table_info(context_guidance_facts)")
                .expect("table info");
            let rows = stmt
                .query_map([], |row| row.get::<_, String>(1))
                .expect("query columns");
            let mut values = Vec::new();
            for row in rows {
                values.push(row.expect("column"));
            }
            Ok::<_, anyhow::Error>(values)
        })
        .expect("columns");

    assert!(columns.contains(&"lifecycle_status".to_string()));
    assert!(columns.contains(&"fact_fingerprint".to_string()));
    assert!(columns.contains(&"value_score".to_string()));
    assert!(columns.contains(&"superseded_by_guidance_id".to_string()));

    for table in [
        "context_guidance_compaction_runs",
        "context_guidance_compaction_members",
        "context_guidance_target_summaries",
    ] {
        let exists = sqlite
            .with_connection(|conn| {
                conn.query_row(
                    "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
                    rusqlite::params![table],
                    |row| row.get::<_, i64>(0),
                )
                .map_err(anyhow::Error::from)
            })
            .expect("table lookup");
        assert_eq!(exists, 1, "missing table {table}");
    }
}

#[test]
fn migrations_upgrade_pre_lifecycle_facts_table_without_index_failure() {
    let temp = tempfile::NamedTempFile::new().expect("temp db");
    let path = temp.into_temp_path().keep().expect("keep temp db");
    let sqlite = SqliteConnectionPool::connect(path).expect("sqlite");
    sqlite
        .execute_batch(
            "CREATE TABLE context_guidance_facts (
                guidance_id TEXT PRIMARY KEY,
                run_id TEXT NOT NULL,
                repo_id TEXT NOT NULL,
                active INTEGER NOT NULL DEFAULT 1,
                category TEXT NOT NULL,
                kind TEXT NOT NULL,
                guidance TEXT NOT NULL,
                evidence_excerpt TEXT NOT NULL,
                confidence TEXT NOT NULL,
                generated_at TEXT DEFAULT (datetime('now')),
                updated_at TEXT DEFAULT (datetime('now'))
            );
            INSERT INTO context_guidance_facts (
                guidance_id,
                run_id,
                repo_id,
                active,
                category,
                kind,
                guidance,
                evidence_excerpt,
                confidence
            ) VALUES (
                'guidance-1',
                'run-1',
                'repo-1',
                1,
                'DECISION',
                'old-row',
                'Keep this row.',
                'Existing guidance evidence.',
                'HIGH'
            );",
        )
        .expect("old schema");
    let mut ctx = MigrationTestContext {
        repo: RepoIdentity {
            provider: "local".to_string(),
            organization: "bitloops".to_string(),
            name: "repo".to_string(),
            identity: "local/repo".to_string(),
            repo_id: "repo-1".to_string(),
        },
        repo_root: PathBuf::from("."),
        sqlite: sqlite.clone(),
    };

    for migration in migrations::CONTEXT_GUIDANCE_MIGRATIONS {
        match migration.run {
            MigrationRunner::Core(run) => run(&mut ctx).expect("migration"),
            MigrationRunner::Knowledge(_) => panic!("context guidance migration must be core"),
        }
    }

    let row = sqlite
        .with_connection(|conn| {
            conn.query_row(
                "SELECT lifecycle_status, fact_fingerprint, value_score
                 FROM context_guidance_facts WHERE guidance_id = 'guidance-1'",
                [],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, f64>(2)?,
                    ))
                },
            )
            .map_err(anyhow::Error::from)
        })
        .expect("migrated fact metadata");

    assert_eq!(row.0, "active");
    assert_eq!(row.1, "");
    assert_eq!(row.2, 0.0);
}

#[test]
fn lifecycle_migration_preserves_existing_lifecycle_metadata() {
    let temp = tempfile::NamedTempFile::new().expect("temp db");
    let path = temp.into_temp_path().keep().expect("keep temp db");
    let sqlite = SqliteConnectionPool::connect(path).expect("sqlite");
    sqlite
        .execute_batch(context_guidance_sqlite_schema_sql())
        .expect("schema");
    sqlite
        .execute_batch(
            "INSERT INTO context_guidance_facts (
                guidance_id,
                run_id,
                repo_id,
                active,
                category,
                kind,
                guidance,
                evidence_excerpt,
                confidence,
                lifecycle_status,
                fact_fingerprint,
                value_score,
                superseded_by_guidance_id,
                lifecycle_reason
            ) VALUES (
                'guidance-1',
                'run-1',
                'repo-1',
                0,
                'DECISION',
                'duplicate',
                'Use the retained fact.',
                'Duplicate of prior guidance.',
                'HIGH',
                'superseded',
                'fingerprint-1',
                0.91,
                'guidance-0',
                'merged by compaction'
            );",
        )
        .expect("insert fact");
    let mut ctx = MigrationTestContext {
        repo: RepoIdentity {
            provider: "local".to_string(),
            organization: "bitloops".to_string(),
            name: "repo".to_string(),
            identity: "local/repo".to_string(),
            repo_id: "repo-1".to_string(),
        },
        repo_root: PathBuf::from("."),
        sqlite: sqlite.clone(),
    };

    match migrations::CONTEXT_GUIDANCE_MIGRATIONS[1].run {
        MigrationRunner::Core(run) => run(&mut ctx).expect("migration"),
        MigrationRunner::Knowledge(_) => panic!("context guidance migration must be core"),
    }

    let row = sqlite
        .with_connection(|conn| {
            conn.query_row(
                "SELECT lifecycle_status, fact_fingerprint, value_score,
                    superseded_by_guidance_id, lifecycle_reason
                 FROM context_guidance_facts WHERE guidance_id = 'guidance-1'",
                [],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, f64>(2)?,
                        row.get::<_, Option<String>>(3)?,
                        row.get::<_, String>(4)?,
                    ))
                },
            )
            .map_err(anyhow::Error::from)
        })
        .expect("fact metadata");

    assert_eq!(row.0, "superseded");
    assert_eq!(row.1, "fingerprint-1");
    assert_eq!(row.2, 0.91);
    assert_eq!(row.3.as_deref(), Some("guidance-0"));
    assert_eq!(row.4, "merged by compaction");
}

#[test]
fn lifecycle_migration_recovers_from_renamed_backup_table() {
    let temp = tempfile::NamedTempFile::new().expect("temp db");
    let path = temp.into_temp_path().keep().expect("keep temp db");
    let sqlite = SqliteConnectionPool::connect(path).expect("sqlite");
    sqlite
        .execute_batch(
            "CREATE TABLE context_guidance_facts_old_v021 (
                guidance_id TEXT PRIMARY KEY,
                run_id TEXT NOT NULL,
                repo_id TEXT NOT NULL,
                active INTEGER NOT NULL DEFAULT 1,
                category TEXT NOT NULL,
                kind TEXT NOT NULL,
                guidance TEXT NOT NULL,
                evidence_excerpt TEXT NOT NULL,
                confidence TEXT NOT NULL,
                generated_at TEXT DEFAULT (datetime('now')),
                updated_at TEXT DEFAULT (datetime('now'))
            );
            INSERT INTO context_guidance_facts_old_v021 (
                guidance_id,
                run_id,
                repo_id,
                active,
                category,
                kind,
                guidance,
                evidence_excerpt,
                confidence
            ) VALUES (
                'guidance-1',
                'run-1',
                'repo-1',
                1,
                'DECISION',
                'old-row',
                'Keep this row.',
                'Existing guidance evidence.',
                'HIGH'
            );",
        )
        .expect("backup state");
    let mut ctx = MigrationTestContext {
        repo: RepoIdentity {
            provider: "local".to_string(),
            organization: "bitloops".to_string(),
            name: "repo".to_string(),
            identity: "local/repo".to_string(),
            repo_id: "repo-1".to_string(),
        },
        repo_root: PathBuf::from("."),
        sqlite: sqlite.clone(),
    };

    match migrations::CONTEXT_GUIDANCE_MIGRATIONS[1].run {
        MigrationRunner::Core(run) => run(&mut ctx).expect("migration"),
        MigrationRunner::Knowledge(_) => panic!("context guidance migration must be core"),
    }

    let (guidance_count, backup_count): (i64, i64) = sqlite
        .with_connection(|conn| {
            let guidance_count = conn.query_row(
                "SELECT COUNT(*) FROM context_guidance_facts WHERE guidance_id = 'guidance-1'",
                [],
                |row| row.get::<_, i64>(0),
            )?;
            let backup_count = conn.query_row(
                "SELECT COUNT(*) FROM sqlite_master
                 WHERE type = 'table' AND name = 'context_guidance_facts_old_v021'",
                [],
                |row| row.get::<_, i64>(0),
            )?;
            Ok((guidance_count, backup_count))
        })
        .expect("migration state");

    assert_eq!(guidance_count, 1);
    assert_eq!(backup_count, 0);
}
