use anyhow::{Context, Result, anyhow, bail};
use rusqlite::{Connection, OpenFlags};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time::Duration;
use tokio::sync::oneshot;

#[derive(Debug)]
struct SqliteWriteRequest {
    statements: Vec<String>,
    response: oneshot::Sender<std::result::Result<(), String>>,
}

#[derive(Debug)]
struct RepoSqliteWriteActor {
    sender: Sender<SqliteWriteRequest>,
}

impl RepoSqliteWriteActor {
    fn shared_for_path(path: &Path) -> Result<Arc<Self>> {
        static REGISTRY: OnceLock<Mutex<HashMap<PathBuf, Arc<RepoSqliteWriteActor>>>> =
            OnceLock::new();
        let registry = REGISTRY.get_or_init(|| Mutex::new(HashMap::new()));
        let canonical_path = canonical_actor_path(path);
        let mut registry = registry
            .lock()
            .map_err(|_| anyhow!("locking SQLite write actor registry"))?;
        if let Some(actor) = registry.get(&canonical_path) {
            return Ok(Arc::clone(actor));
        }

        let actor = Arc::new(Self::spawn(canonical_path.clone())?);
        registry.insert(canonical_path, Arc::clone(&actor));
        Ok(actor)
    }

    fn spawn(path: PathBuf) -> Result<Self> {
        let (sender, receiver) = mpsc::channel::<SqliteWriteRequest>();
        let thread_name = format!("bitloops-sqlite-writer-{}", short_thread_label(&path));
        thread::Builder::new()
            .name(thread_name)
            .spawn(move || writer_loop(path, receiver))
            .context("spawning SQLite write actor thread")?;
        Ok(Self { sender })
    }

    async fn exec(&self, statements: Vec<String>) -> Result<()> {
        if statements
            .iter()
            .all(|statement| statement.trim().is_empty())
        {
            return Ok(());
        }
        let (response_tx, response_rx) = oneshot::channel();
        self.sender
            .send(SqliteWriteRequest {
                statements,
                response: response_tx,
            })
            .map_err(|_| anyhow!("sending work to SQLite write actor"))?;
        match response_rx.await {
            Ok(Ok(())) => Ok(()),
            Ok(Err(err)) => Err(anyhow!(err)),
            Err(_) => Err(anyhow!("SQLite write actor dropped the response channel")),
        }
    }
}

pub(super) async fn sqlite_exec_serialized_path(path: &Path, sql: &str) -> Result<()> {
    RepoSqliteWriteActor::shared_for_path(path)?
        .exec(vec![sql.to_string()])
        .await
}

pub(super) async fn sqlite_exec_serialized_batch_transactional_path(
    path: &Path,
    statements: &[String],
) -> Result<()> {
    RepoSqliteWriteActor::shared_for_path(path)?
        .exec(statements.to_vec())
        .await
}

fn writer_loop(path: PathBuf, receiver: Receiver<SqliteWriteRequest>) {
    let mut connection = open_sqlite_writer_connection(&path)
        .map_err(|err| format!("{err:#}"))
        .ok();
    while let Ok(request) = receiver.recv() {
        let result = match connection.as_mut() {
            Some(connection) => execute_request(connection, &request.statements).map_err(|err| {
                format!("serialised SQLite write for `{}`: {err:#}", path.display())
            }),
            None => Err(format!(
                "opening serialised SQLite writer connection for `{}` failed",
                path.display()
            )),
        };

        let _ = request.response.send(result);
    }
}

fn execute_request(connection: &mut Connection, statements: &[String]) -> Result<()> {
    let tx = connection
        .transaction()
        .context("starting serialised SQLite write transaction")?;
    for statement in statements {
        if statement.trim().is_empty() {
            continue;
        }
        tx.execute_batch(statement)
            .context("executing serialised SQLite write statement")?;
    }
    tx.commit()
        .context("committing serialised SQLite write transaction")?;
    Ok(())
}

fn open_sqlite_writer_connection(path: &Path) -> Result<Connection> {
    if !path.is_file() {
        bail!(
            "SQLite database file not found at {}. Run `bitloops init` to create and initialise stores.",
            path.display()
        );
    }
    crate::sqlite_vec_auto_extension::register_sqlite_vec_auto_extension()
        .context("registering sqlite-vec auto-extension for serialised SQLite writer")?;
    let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_WRITE)
        .with_context(|| format!("opening SQLite database at {}", path.display()))?;
    conn.busy_timeout(Duration::from_secs(30))
        .context("setting SQLite busy timeout for serialised writer")?;
    conn.execute_batch(
        "PRAGMA foreign_keys = ON; PRAGMA journal_mode = WAL; PRAGMA synchronous = NORMAL;",
    )
    .context("configuring serialised SQLite writer connection")?;
    Ok(conn)
}

fn short_thread_label(path: &Path) -> String {
    let stem = path
        .file_stem()
        .and_then(|value| value.to_str())
        .filter(|value| !value.is_empty())
        .unwrap_or("db");
    let mut hash = 1469598103934665603_u64;
    for byte in path.to_string_lossy().as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(1099511628211);
    }
    format!("{stem}-{hash:08x}")
}

fn canonical_actor_path(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::{
        short_thread_label, sqlite_exec_serialized_batch_transactional_path,
        sqlite_exec_serialized_path,
    };
    use anyhow::Result;
    use std::path::Path;
    use tempfile::TempDir;

    fn create_sample_db() -> Result<(TempDir, std::path::PathBuf)> {
        let temp = TempDir::new().expect("temp dir");
        let db_path = temp.path().join("runtime.sqlite");
        let conn = rusqlite::Connection::open(&db_path).expect("open sqlite");
        conn.execute_batch("CREATE TABLE sample (value INTEGER NOT NULL);")
            .expect("create table");
        Ok((temp, db_path))
    }

    #[tokio::test]
    async fn serialised_writer_applies_concurrent_requests_on_one_connection() -> Result<()> {
        let (_temp, db_path) = create_sample_db()?;

        let writes = (0..16_u64)
            .map(|value| {
                let db_path = db_path.clone();
                tokio::spawn(async move {
                    sqlite_exec_serialized_path(
                        &db_path,
                        &format!("INSERT INTO sample (value) VALUES ({value});"),
                    )
                    .await
                })
            })
            .collect::<Vec<_>>();

        for write in writes {
            write.await.expect("join sqlite writer task")?;
        }

        let conn = rusqlite::Connection::open(&db_path).expect("re-open sqlite");
        let count = conn
            .query_row("SELECT COUNT(*) FROM sample", [], |row| {
                row.get::<_, i64>(0)
            })
            .expect("count rows");
        assert_eq!(count, 16);
        Ok(())
    }

    #[tokio::test]
    async fn serialised_writer_rolls_back_failed_batches() -> Result<()> {
        let (_temp, db_path) = create_sample_db()?;

        let err = sqlite_exec_serialized_batch_transactional_path(
            &db_path,
            &[
                "INSERT INTO sample (value) VALUES (1);".to_string(),
                "INSERT INTO missing_table (value) VALUES (2);".to_string(),
            ],
        )
        .await
        .expect_err("batch should fail");
        assert!(
            err.to_string().contains("missing_table"),
            "expected missing table error, got {err:#}"
        );

        let conn = rusqlite::Connection::open(&db_path).expect("re-open sqlite");
        let count = conn
            .query_row("SELECT COUNT(*) FROM sample", [], |row| {
                row.get::<_, i64>(0)
            })
            .expect("count rows");
        assert_eq!(count, 0);

        sqlite_exec_serialized_path(&db_path, "INSERT INTO sample (value) VALUES (3);").await?;
        let conn = rusqlite::Connection::open(&db_path).expect("re-open sqlite after recovery");
        let count = conn
            .query_row("SELECT COUNT(*) FROM sample", [], |row| {
                row.get::<_, i64>(0)
            })
            .expect("count rows after recovery");
        assert_eq!(count, 1);
        Ok(())
    }

    #[test]
    fn thread_label_uses_stem_and_hash() {
        let label = short_thread_label(Path::new("/tmp/repos/runtime.sqlite"));
        assert!(label.starts_with("runtime-"));
    }
}
