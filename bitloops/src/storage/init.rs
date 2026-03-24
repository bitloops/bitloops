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

    conn.execute_batch("PRAGMA foreign_keys = ON;")
        .context("failed to enable foreign keys")?;

    conn.execute_batch(schema::SCHEMA_SQL)
        .context("failed to create schema")?;

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
    conn.execute_batch("PRAGMA foreign_keys = ON;")
        .context("failed to enable foreign keys")?;
    Ok(conn)
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
