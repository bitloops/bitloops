use std::collections::HashSet;
use std::path::{Component, Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use anyhow::{Context, Result, anyhow};

use crate::host::interactions::db_store::SqliteInteractionSpool;
use crate::storage::SqliteConnectionPool;

pub(crate) fn initialise_repo_runtime_schema(sqlite: &SqliteConnectionPool) -> Result<()> {
    sqlite
        .execute_batch(crate::host::devql::checkpoint_runtime_schema_sql_sqlite())
        .context("initialising runtime checkpoint schema")?;
    let spool = SqliteInteractionSpool::new(sqlite.clone(), "__runtime-bootstrap__".to_string())
        .context("initialising interaction spool schema in runtime db")?;
    drop(spool);
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

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct SchemaInitKey {
    sqlite_path: PathBuf,
    family: &'static str,
}

pub(crate) fn ensure_sqlite_schema_once<F>(
    sqlite_path: &Path,
    family: &'static str,
    init: F,
) -> Result<()>
where
    F: FnOnce(PathBuf) -> Result<()>,
{
    static REGISTRY: OnceLock<Mutex<HashSet<SchemaInitKey>>> = OnceLock::new();
    let key = schema_init_key(sqlite_path, family)?;
    let registry = REGISTRY.get_or_init(|| Mutex::new(HashSet::new()));
    let mut registry = registry
        .lock()
        .map_err(|_| anyhow!("locking SQLite schema initialisation registry"))?;
    if key.sqlite_path.is_file() && registry.contains(&key) {
        return Ok(());
    }
    init(key.sqlite_path.clone())
        .with_context(|| format!("initialising SQLite schema family `{family}`"))?;
    registry.insert(key);
    Ok(())
}

fn schema_init_key(sqlite_path: &Path, family: &'static str) -> Result<SchemaInitKey> {
    let absolute = std::path::absolute(sqlite_path)
        .with_context(|| format!("resolving absolute SQLite path {}", sqlite_path.display()))?;
    Ok(SchemaInitKey {
        sqlite_path: normalize_sqlite_path(&absolute),
        family,
    })
}

fn normalize_sqlite_path(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if !normalized.pop() {
                    normalized.push(component.as_os_str());
                }
            }
            Component::Normal(_) | Component::RootDir | Component::Prefix(_) => {
                normalized.push(component.as_os_str());
            }
        }
    }
    normalized
}
