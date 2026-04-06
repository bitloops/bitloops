use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::config::resolve_store_backend_config_for_repo;
use crate::host::interactions::db_store::SqliteInteractionSpool;
use crate::storage::SqliteConnectionPool;

pub(crate) fn initialise_repo_runtime_schema(sqlite: &SqliteConnectionPool) -> Result<()> {
    sqlite
        .execute_batch(crate::host::devql::checkpoint_runtime_schema_sql_sqlite())
        .context("initialising runtime checkpoint schema")?;
    let spool = SqliteInteractionSpool::new(sqlite.clone(), "__runtime-bootstrap__".to_string())
        .context("initialising interaction spool schema in runtime db")?;
    drop(spool);
    Ok(())
}

pub(crate) fn legacy_relational_sqlite_path(repo_root: &Path) -> Result<PathBuf> {
    let cfg = resolve_store_backend_config_for_repo(repo_root)
        .context("resolving backend config for legacy relational runtime migration")?;
    cfg.relational
        .resolve_sqlite_db_path_for_repo(repo_root)
        .context("resolving legacy relational sqlite path")
}

pub(crate) fn all_tables_empty(conn: &rusqlite::Connection, tables: &[&str]) -> Result<bool> {
    for table in tables {
        if !table_exists(conn, table)? {
            continue;
        }
        let count: i64 = conn
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                row.get(0)
            })
            .with_context(|| format!("counting rows in `{table}`"))?;
        if count > 0 {
            return Ok(false);
        }
    }
    Ok(true)
}

fn table_exists(conn: &rusqlite::Connection, table: &str) -> Result<bool> {
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
            rusqlite::params![table],
            |row| row.get(0),
        )
        .with_context(|| format!("checking table `{table}`"))?;
    Ok(count > 0)
}

pub(crate) fn attach_if_needed(
    conn: &rusqlite::Connection,
    path: &Path,
    alias: &str,
) -> Result<()> {
    conn.execute(
        &format!("ATTACH DATABASE ?1 AS {alias}"),
        rusqlite::params![path.display().to_string()],
    )
    .with_context(|| format!("attaching database {} as {alias}", path.display()))?;
    Ok(())
}

pub(crate) fn detach_if_needed(conn: &rusqlite::Connection, alias: &str) -> Result<()> {
    conn.execute_batch(&format!("DETACH DATABASE {alias}"))
        .with_context(|| format!("detaching database alias `{alias}`"))?;
    Ok(())
}

pub(crate) fn table_has_rows_in_attached_db(
    conn: &rusqlite::Connection,
    alias: &str,
    table: &str,
) -> Result<bool> {
    let count: i64 = conn
        .query_row(
            &format!(
                "SELECT COUNT(*) FROM {alias}.sqlite_master WHERE type = 'table' AND name = ?1"
            ),
            rusqlite::params![table],
            |row| row.get(0),
        )
        .with_context(|| format!("checking attached table `{alias}.{table}`"))?;
    if count == 0 {
        return Ok(false);
    }
    let row_count: i64 = conn
        .query_row(
            &format!("SELECT COUNT(*) FROM {alias}.{table}"),
            [],
            |row| row.get(0),
        )
        .with_context(|| format!("counting rows in `{alias}.{table}`"))?;
    Ok(row_count > 0)
}

pub(crate) fn execute_copy_if_legacy_table_exists(
    conn: &rusqlite::Connection,
    alias: &str,
    table: &str,
    sql: &str,
) -> Result<()> {
    if table_has_rows_in_attached_db(conn, alias, table)? {
        conn.execute_batch(sql)
            .with_context(|| format!("copying `{alias}.{table}` into runtime store"))?;
    }
    Ok(())
}
