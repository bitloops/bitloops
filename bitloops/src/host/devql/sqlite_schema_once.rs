use std::collections::HashMap;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use anyhow::{Context, Result, anyhow};
use tokio::sync::OnceCell;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct SchemaInitKey {
    sqlite_path: PathBuf,
    family: &'static str,
}

impl SchemaInitKey {
    fn new(sqlite_path: &Path, family: &'static str) -> Self {
        Self {
            sqlite_path: sqlite_path
                .canonicalize()
                .unwrap_or_else(|_| sqlite_path.to_path_buf()),
            family,
        }
    }
}

pub(crate) async fn ensure_sqlite_schema_once<F, Fut>(
    sqlite_path: &Path,
    family: &'static str,
    init: F,
) -> Result<()>
where
    F: FnOnce(PathBuf) -> Fut,
    Fut: Future<Output = Result<()>>,
{
    let key = SchemaInitKey::new(sqlite_path, family);
    let once = shared_schema_once_cell(&key)?;
    once.get_or_try_init(|| async move {
        init(key.sqlite_path.clone())
            .await
            .with_context(|| format!("initialising SQLite schema family `{family}`"))
    })
    .await?;
    Ok(())
}

fn shared_schema_once_cell(key: &SchemaInitKey) -> Result<Arc<OnceCell<()>>> {
    static REGISTRY: OnceLock<Mutex<HashMap<SchemaInitKey, Arc<OnceCell<()>>>>> = OnceLock::new();
    let registry = REGISTRY.get_or_init(|| Mutex::new(HashMap::new()));
    let mut registry = registry
        .lock()
        .map_err(|_| anyhow!("locking SQLite schema initialisation registry"))?;
    Ok(Arc::clone(
        registry
            .entry(key.clone())
            .or_insert_with(|| Arc::new(OnceCell::new())),
    ))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    use anyhow::{Result, anyhow};
    use tempfile::TempDir;
    use tokio::time::sleep;

    use super::ensure_sqlite_schema_once;

    fn sample_sqlite_path() -> (TempDir, std::path::PathBuf) {
        let temp = TempDir::new().expect("temp dir");
        let sqlite_path = temp.path().join("runtime.sqlite");
        std::fs::write(&sqlite_path, []).expect("create sqlite path placeholder");
        (temp, sqlite_path)
    }

    #[tokio::test]
    async fn schema_once_runs_only_once_per_path_and_family() -> Result<()> {
        let (_temp, sqlite_path) = sample_sqlite_path();
        let init_calls = Arc::new(AtomicUsize::new(0));

        let tasks = (0..8)
            .map(|_| {
                let sqlite_path = sqlite_path.clone();
                let init_calls = Arc::clone(&init_calls);
                tokio::spawn(async move {
                    ensure_sqlite_schema_once(
                        &sqlite_path,
                        "schema-once-concurrent",
                        move |_| async move {
                            init_calls.fetch_add(1, Ordering::SeqCst);
                            sleep(Duration::from_millis(20)).await;
                            Ok(())
                        },
                    )
                    .await
                })
            })
            .collect::<Vec<_>>();

        for task in tasks {
            task.await.expect("join schema once task")?;
        }

        assert_eq!(init_calls.load(Ordering::SeqCst), 1);
        Ok(())
    }

    #[tokio::test]
    async fn schema_once_retries_after_failure_and_then_caches_success() -> Result<()> {
        let (_temp, sqlite_path) = sample_sqlite_path();
        let init_calls = Arc::new(AtomicUsize::new(0));

        let first_err = ensure_sqlite_schema_once(&sqlite_path, "schema-once-retry", {
            let init_calls = Arc::clone(&init_calls);
            move |_| async move {
                init_calls.fetch_add(1, Ordering::SeqCst);
                Err(anyhow!("initial failure"))
            }
        })
        .await
        .expect_err("first initialisation should fail");
        assert!(
            format!("{first_err:#}").contains("initial failure"),
            "expected original failure, got {first_err:#}"
        );

        ensure_sqlite_schema_once(&sqlite_path, "schema-once-retry", {
            let init_calls = Arc::clone(&init_calls);
            move |_| async move {
                init_calls.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }
        })
        .await?;

        ensure_sqlite_schema_once(&sqlite_path, "schema-once-retry", {
            let init_calls = Arc::clone(&init_calls);
            move |_| async move {
                init_calls.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }
        })
        .await?;

        assert_eq!(init_calls.load(Ordering::SeqCst), 2);
        Ok(())
    }

    #[tokio::test]
    async fn schema_once_keeps_families_independent_for_same_path() -> Result<()> {
        let (_temp, sqlite_path) = sample_sqlite_path();
        let init_calls = Arc::new(AtomicUsize::new(0));

        ensure_sqlite_schema_once(&sqlite_path, "schema-family-a", {
            let init_calls = Arc::clone(&init_calls);
            move |_| async move {
                init_calls.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }
        })
        .await?;
        ensure_sqlite_schema_once(&sqlite_path, "schema-family-b", {
            let init_calls = Arc::clone(&init_calls);
            move |_| async move {
                init_calls.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }
        })
        .await?;
        ensure_sqlite_schema_once(&sqlite_path, "schema-family-a", {
            let init_calls = Arc::clone(&init_calls);
            move |_| async move {
                init_calls.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }
        })
        .await?;

        assert_eq!(init_calls.load(Ordering::SeqCst), 2);
        Ok(())
    }
}
