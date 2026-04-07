use anyhow::{Context, Result};

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
