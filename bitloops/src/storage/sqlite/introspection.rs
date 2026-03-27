use anyhow::{Context, Result};

pub(super) fn sqlite_table_exists(conn: &rusqlite::Connection, table: &str) -> Result<bool> {
    let exists: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
            [table],
            |row| row.get(0),
        )
        .context("checking SQLite table existence")?;
    Ok(exists > 0)
}

pub(super) fn sqlite_table_pk_columns(
    conn: &rusqlite::Connection,
    table: &str,
) -> Result<Vec<String>> {
    let mut stmt = conn
        .prepare(&format!("PRAGMA table_info({table})"))
        .with_context(|| format!("preparing PRAGMA table_info for `{table}`"))?;
    let mut rows = stmt
        .query([])
        .with_context(|| format!("querying PRAGMA table_info for `{table}`"))?;
    let mut pk = Vec::<(i64, String)>::new();
    while let Some(row) = rows.next().context("reading PRAGMA row")? {
        let name: String = row
            .get(1)
            .with_context(|| format!("reading column name from `{table}`"))?;
        let order: i64 = row
            .get(5)
            .with_context(|| format!("reading pk order from `{table}`"))?;
        if order > 0 {
            pk.push((order, name));
        }
    }
    pk.sort_by_key(|(order, _)| *order);
    Ok(pk.into_iter().map(|(_, name)| name).collect())
}

pub(super) fn sqlite_table_has_column(
    conn: &rusqlite::Connection,
    table: &str,
    column: &str,
) -> Result<bool> {
    let mut stmt = conn
        .prepare(&format!("PRAGMA table_info({table})"))
        .with_context(|| format!("preparing PRAGMA table_info for `{table}`"))?;
    let mut rows = stmt
        .query([])
        .with_context(|| format!("querying PRAGMA table_info for `{table}`"))?;
    while let Some(row) = rows.next().context("reading PRAGMA row")? {
        let name: String = row
            .get(1)
            .with_context(|| format!("reading column name from `{table}`"))?;
        if name == column {
            return Ok(true);
        }
    }
    Ok(false)
}
