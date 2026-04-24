use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use rusqlite::Connection;

pub mod schema;
pub mod seed;

pub fn init_database(db_path: &Path, seed: bool, commit_sha: &str) -> Result<()> {
    ensure_parent_dir_exists(db_path)?;

    let mut conn = Connection::open(db_path).with_context(|| {
        format!(
            "failed to open or create sqlite database at {}",
            db_path.display()
        )
    })?;

    configure_sqlite_connection(&conn).context("failed to configure sqlite connection")?;

    conn.execute_batch(schema::SCHEMA_SQL)
        .context("failed to create schema")?;
    conn.execute_batch(
        crate::capability_packs::semantic_clones::semantic_features_sqlite_schema_sql(),
    )
    .context("failed to create semantic feature schema")?;

    if seed {
        let seeded = seed::seed_database(&mut conn, commit_sha)?;
        println!(
            "seeded {} production artefacts for commit {}",
            seeded.artefacts, commit_sha
        );
    }

    println!("database initialized at {}", db_path.display());
    Ok(())
}

pub fn open_existing_database(db_path: &Path) -> Result<Connection> {
    if !db_path.exists() {
        anyhow::bail!("Database not found. Run init-fixture-db.sh first.");
    }

    let conn = Connection::open(db_path)
        .with_context(|| format!("failed to open sqlite database at {}", db_path.display()))?;
    configure_sqlite_connection(&conn).context("failed to configure sqlite connection")?;
    Ok(conn)
}

fn configure_sqlite_connection(conn: &Connection) -> Result<()> {
    conn.busy_timeout(std::time::Duration::from_secs(30))
        .context("failed to set sqlite busy timeout")?;
    conn.execute_batch(
        "PRAGMA foreign_keys = ON; PRAGMA journal_mode = WAL; PRAGMA synchronous = NORMAL;",
    )
    .context("failed to configure sqlite pragmas")?;
    Ok(())
}

fn ensure_parent_dir_exists(db_path: &Path) -> Result<()> {
    if let Some(parent) = db_path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent).with_context(|| {
            format!(
                "failed to create parent directory for db path {}",
                db_path.display()
            )
        })?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn init_database_enables_wal_mode() {
        let dir = tempdir().expect("tempdir");
        let db_path = dir.path().join("fixture.sqlite");

        init_database(&db_path, false, "seed-commit").expect("init db");

        let conn = Connection::open(&db_path).expect("open db");
        let journal_mode: String = conn
            .query_row("PRAGMA journal_mode;", [], |row| row.get(0))
            .expect("read journal mode");

        assert_eq!(journal_mode.to_ascii_lowercase(), "wal");
    }
}
