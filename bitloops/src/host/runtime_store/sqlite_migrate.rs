use anyhow::{Context, Result};

use crate::host::interactions::db_store::initialise_interaction_spool_schema;
use crate::storage::SqliteConnectionPool;

pub(crate) fn initialise_repo_runtime_schema(sqlite: &SqliteConnectionPool) -> Result<()> {
    sqlite
        .execute_batch(crate::host::devql::checkpoint_runtime_schema_sql_sqlite())
        .context("initialising runtime checkpoint schema")?;
    initialise_interaction_spool_schema(sqlite)
        .context("initialising interaction spool schema in runtime db")?;
    sqlite
        .execute_batch(crate::host::devql::producer_spool_schema_sql_sqlite())
        .context("initialising DevQL producer spool schema in runtime db")?;
    sqlite
        .execute_batch(super::repo_workplane::REPO_WORKPLANE_SCHEMA)
        .context("initialising capability workplane schema in runtime db")?;
    super::repo_workplane::ensure_repo_workplane_schema_upgrades(sqlite)
        .context("upgrading capability workplane schema in runtime db")?;
    Ok(())
}
