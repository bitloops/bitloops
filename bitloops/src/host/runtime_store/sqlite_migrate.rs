use std::collections::HashSet;
use std::path::{Component, Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use anyhow::{Context, Result, anyhow};

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
    let canonical = canonical_sqlite_path(&absolute)
        .with_context(|| format!("canonicalising SQLite path {}", absolute.display()))?;
    Ok(SchemaInitKey {
        sqlite_path: normalize_sqlite_path(&canonical),
        family,
    })
}

fn canonical_sqlite_path(path: &Path) -> Result<PathBuf> {
    match path.canonicalize() {
        Ok(canonical) => return Ok(canonical),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
        Err(err) => return Err(err).context("canonicalising existing SQLite path"),
    }

    let mut missing_suffix = Vec::new();
    let mut current = path;
    while !current.exists() {
        let Some(name) = current.file_name() else {
            return Ok(path.to_path_buf());
        };
        missing_suffix.push(name.to_os_string());
        let Some(parent) = current.parent() else {
            return Ok(path.to_path_buf());
        };
        current = parent;
    }

    let mut canonical = current
        .canonicalize()
        .context("canonicalising nearest existing SQLite ancestor")?;
    for segment in missing_suffix.iter().rev() {
        canonical.push(segment);
    }
    Ok(canonical)
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

#[cfg(test)]
mod tests {
    use super::*;

    use std::fs;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[cfg(unix)]
    #[test]
    fn ensure_sqlite_schema_once_dedupes_alias_and_canonical_paths_before_file_exists() {
        use std::os::unix::fs::symlink;

        let temp = tempfile::TempDir::new().expect("temp dir");
        let real_dir = temp.path().join("real");
        fs::create_dir_all(&real_dir).expect("create real dir");
        let alias_dir = temp.path().join("alias");
        symlink(&real_dir, &alias_dir).expect("symlink alias dir");

        let real_db_path = real_dir.join("runtime.sqlite");
        let alias_db_path = alias_dir.join("runtime.sqlite");
        let init_calls = AtomicUsize::new(0);

        ensure_sqlite_schema_once(&alias_db_path, "test-family", |sqlite_path| {
            init_calls.fetch_add(1, Ordering::SeqCst);
            fs::write(sqlite_path, []).context("create sqlite file for alias init")
        })
        .expect("initialise schema via alias path");

        ensure_sqlite_schema_once(&real_db_path, "test-family", |_sqlite_path| {
            init_calls.fetch_add(1, Ordering::SeqCst);
            Ok(())
        })
        .expect("reuse schema init for canonical path");

        assert_eq!(
            init_calls.load(Ordering::SeqCst),
            1,
            "schema initialisation should only run once for aliased and canonical DB paths"
        );
    }
}
