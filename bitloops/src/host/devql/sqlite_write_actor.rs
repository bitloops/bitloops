use anyhow::{Context, Result, anyhow, bail};
use rusqlite::{Connection, OpenFlags, limits::Limit, params_from_iter, types::Value as SqlValue};
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time::Duration;
use tokio::sync::oneshot;

use crate::capability_packs::architecture_graph::storage::ArchitectureGraphFacts;

const ARCHITECTURE_GRAPH_WRITE_BATCH_SIZE: usize = 250;
const ARCHITECTURE_GRAPH_NODE_INSERT_WIDTH: usize = 14;
const ARCHITECTURE_GRAPH_EDGE_INSERT_WIDTH: usize = 11;
const SQLITE_FALLBACK_VARIABLE_LIMIT: usize = 999;

#[derive(Debug)]
enum SqliteWriteOperation {
    Statements(Vec<String>),
    ReplaceArchitectureGraph(ArchitectureGraphReplaceRequest),
}

#[derive(Debug)]
struct SqliteWriteRequest {
    operation: SqliteWriteOperation,
    response: oneshot::Sender<std::result::Result<(), String>>,
}

#[derive(Debug)]
struct ArchitectureGraphReplaceRequest {
    repo_id: String,
    facts: ArchitectureGraphFacts,
    generation_seq: u64,
    warnings: Vec<String>,
    metrics: Value,
    #[cfg(test)]
    fail_after_writes: Option<usize>,
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
                operation: SqliteWriteOperation::Statements(statements),
                response: response_tx,
            })
            .map_err(|_| anyhow!("sending work to SQLite write actor"))?;
        match response_rx.await {
            Ok(Ok(())) => Ok(()),
            Ok(Err(err)) => Err(anyhow!(err)),
            Err(_) => Err(anyhow!("SQLite write actor dropped the response channel")),
        }
    }

    async fn replace_architecture_graph(
        &self,
        request: ArchitectureGraphReplaceRequest,
    ) -> Result<()> {
        let (response_tx, response_rx) = oneshot::channel();
        self.sender
            .send(SqliteWriteRequest {
                operation: SqliteWriteOperation::ReplaceArchitectureGraph(request),
                response: response_tx,
            })
            .map_err(|_| anyhow!("sending architecture graph work to SQLite write actor"))?;
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

pub(super) async fn sqlite_replace_architecture_graph_current_path(
    path: &Path,
    repo_id: &str,
    facts: ArchitectureGraphFacts,
    generation_seq: u64,
    warnings: &[String],
    metrics: Value,
) -> Result<()> {
    RepoSqliteWriteActor::shared_for_path(path)?
        .replace_architecture_graph(ArchitectureGraphReplaceRequest {
            repo_id: repo_id.to_string(),
            facts,
            generation_seq,
            warnings: warnings.to_vec(),
            metrics,
            #[cfg(test)]
            fail_after_writes: None,
        })
        .await
}

fn writer_loop(path: PathBuf, receiver: Receiver<SqliteWriteRequest>) {
    let mut connection = open_sqlite_writer_connection(&path)
        .map_err(|err| format!("{err:#}"))
        .ok();
    while let Ok(request) = receiver.recv() {
        let result = match connection.as_mut() {
            Some(connection) => crate::storage::sqlite::with_sqlite_write_lock(&path, || {
                execute_request(connection, &request.operation)
            })
            .map_err(|err| format!("serialised SQLite write for `{}`: {err:#}", path.display())),
            None => Err(format!(
                "opening serialised SQLite writer connection for `{}` failed",
                path.display()
            )),
        };

        let _ = request.response.send(result);
    }
}

fn execute_request(connection: &mut Connection, operation: &SqliteWriteOperation) -> Result<()> {
    match operation {
        SqliteWriteOperation::Statements(statements) => {
            execute_statement_batch(connection, statements)
        }
        SqliteWriteOperation::ReplaceArchitectureGraph(request) => {
            execute_architecture_graph_replace(connection, request)
        }
    }
}

fn execute_statement_batch(connection: &mut Connection, statements: &[String]) -> Result<()> {
    let tx = connection
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
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

fn execute_architecture_graph_replace(
    connection: &mut Connection,
    request: &ArchitectureGraphReplaceRequest,
) -> Result<()> {
    validate_architecture_graph_repo_scope(request)?;
    let node_batch_rows =
        architecture_graph_batch_row_count(connection, ARCHITECTURE_GRAPH_NODE_INSERT_WIDTH);
    let edge_batch_rows =
        architecture_graph_batch_row_count(connection, ARCHITECTURE_GRAPH_EDGE_INSERT_WIDTH);
    let tx = connection
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .context("starting architecture graph replacement transaction")?;
    tx.execute(
        "DELETE FROM architecture_graph_edges_current WHERE repo_id = ?1",
        rusqlite::params![request.repo_id],
    )
    .context("deleting current architecture graph edges")?;
    tx.execute(
        "DELETE FROM architecture_graph_nodes_current WHERE repo_id = ?1",
        rusqlite::params![request.repo_id],
    )
    .context("deleting current architecture graph nodes")?;

    let mut writes = 0usize;
    execute_architecture_graph_node_batches(&tx, request, node_batch_rows, &mut writes)?;
    execute_architecture_graph_edge_batches(&tx, request, edge_batch_rows, &mut writes)?;

    tx.execute(
        "INSERT INTO architecture_graph_runs_current (
            repo_id, last_generation_seq, status, warnings_json, metrics_json, updated_at
         ) VALUES (?1, ?2, 'ready', ?3, ?4, datetime('now'))
         ON CONFLICT(repo_id) DO UPDATE SET
            last_generation_seq = excluded.last_generation_seq,
            status = excluded.status,
            warnings_json = excluded.warnings_json,
            metrics_json = excluded.metrics_json,
            updated_at = excluded.updated_at",
        rusqlite::params![
            request.repo_id,
            i64::try_from(request.generation_seq)
                .context("converting architecture graph generation sequence for SQLite")?,
            serde_json::to_string(&request.warnings)
                .context("serialising architecture graph warnings")?,
            json_string(&request.metrics)?,
        ],
    )
    .context("upserting architecture graph run metadata")?;

    tx.commit()
        .context("committing architecture graph replacement transaction")?;
    Ok(())
}

fn validate_architecture_graph_repo_scope(request: &ArchitectureGraphReplaceRequest) -> Result<()> {
    for node in &request.facts.nodes {
        if node.repo_id != request.repo_id {
            bail!(
                "architecture graph node `{}` repo_id `{}` did not match replacement repo `{}`",
                node.node_id,
                node.repo_id,
                request.repo_id
            );
        }
    }
    for edge in &request.facts.edges {
        if edge.repo_id != request.repo_id {
            bail!(
                "architecture graph edge `{}` repo_id `{}` did not match replacement repo `{}`",
                edge.edge_id,
                edge.repo_id,
                request.repo_id
            );
        }
    }
    Ok(())
}

fn json_string(value: &Value) -> Result<String> {
    serde_json::to_string(value).context("serialising architecture graph JSON payload")
}

fn opt_generation(value: Option<u64>) -> Result<Option<i64>> {
    value
        .map(|generation| {
            i64::try_from(generation)
                .context("converting architecture graph generation sequence for SQLite")
        })
        .transpose()
}

fn maybe_fail_architecture_graph_write(
    request: &ArchitectureGraphReplaceRequest,
    writes: usize,
) -> Result<()> {
    #[cfg(test)]
    if request
        .fail_after_writes
        .is_some_and(|limit| writes >= limit)
    {
        bail!("injected architecture graph write failure after {writes} writes");
    }
    let _ = (request, writes);
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
    conn.execute_batch("PRAGMA foreign_keys = ON; PRAGMA synchronous = NORMAL;")
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

fn architecture_graph_batch_row_count(connection: &Connection, columns_per_row: usize) -> usize {
    let variable_limit = connection
        .limit(Limit::SQLITE_LIMIT_VARIABLE_NUMBER)
        .ok()
        .and_then(|limit| usize::try_from(limit).ok())
        .unwrap_or(SQLITE_FALLBACK_VARIABLE_LIMIT);
    architecture_graph_batch_row_count_for_limit(
        architecture_graph_write_batch_size(),
        columns_per_row,
        variable_limit,
    )
}

fn architecture_graph_batch_row_count_for_limit(
    max_batch_rows: usize,
    columns_per_row: usize,
    variable_limit: usize,
) -> usize {
    assert!(
        columns_per_row > 0,
        "columns_per_row must be greater than zero"
    );
    let max_rows_by_limit = variable_limit / columns_per_row;
    max_batch_rows.min(max_rows_by_limit.max(1)).max(1)
}

fn architecture_graph_write_batch_size() -> usize {
    ARCHITECTURE_GRAPH_WRITE_BATCH_SIZE
}

fn build_architecture_graph_node_insert_sql(row_count: usize) -> String {
    build_architecture_graph_insert_sql(
        "architecture_graph_nodes_current",
        "repo_id, node_id, node_kind, label, artefact_id, symbol_id, path, entry_kind, \
         source_kind, confidence, provenance_json, evidence_json, properties_json, \
         last_observed_generation, updated_at",
        ARCHITECTURE_GRAPH_NODE_INSERT_WIDTH,
        row_count,
    )
}

fn build_architecture_graph_edge_insert_sql(row_count: usize) -> String {
    build_architecture_graph_insert_sql(
        "architecture_graph_edges_current",
        "repo_id, edge_id, edge_kind, from_node_id, to_node_id, source_kind, confidence, \
         provenance_json, evidence_json, properties_json, last_observed_generation, updated_at",
        ARCHITECTURE_GRAPH_EDGE_INSERT_WIDTH,
        row_count,
    )
}

fn build_architecture_graph_insert_sql(
    table: &str,
    columns_sql: &str,
    parameter_count: usize,
    row_count: usize,
) -> String {
    assert!(row_count > 0, "row_count must be greater than zero");
    let placeholders = std::iter::repeat_n("?", parameter_count)
        .collect::<Vec<_>>()
        .join(", ");
    let row_sql = format!("({placeholders}, datetime('now'))");
    let values_sql = std::iter::repeat_n(row_sql, row_count)
        .collect::<Vec<_>>()
        .join(", ");
    format!("INSERT INTO {table} ({columns_sql}) VALUES {values_sql}")
}

fn execute_architecture_graph_node_batches(
    tx: &rusqlite::Transaction<'_>,
    request: &ArchitectureGraphReplaceRequest,
    batch_rows: usize,
    writes: &mut usize,
) -> Result<()> {
    let full_batch_sql = build_architecture_graph_node_insert_sql(batch_rows);
    let mut tail_batch_sql = None::<String>;

    for batch in request.facts.nodes.chunks(batch_rows) {
        let sql = if batch.len() == batch_rows {
            full_batch_sql.as_str()
        } else {
            tail_batch_sql
                .get_or_insert_with(|| build_architecture_graph_node_insert_sql(batch.len()))
                .as_str()
        };
        let mut params = Vec::with_capacity(batch.len() * ARCHITECTURE_GRAPH_NODE_INSERT_WIDTH);
        for node in batch {
            append_architecture_graph_node_params(&mut params, &request.repo_id, node)?;
            *writes += 1;
            maybe_fail_architecture_graph_write(request, *writes)?;
        }
        tx.execute(sql, params_from_iter(params))
            .context("inserting architecture graph node batch")?;
    }

    Ok(())
}

fn execute_architecture_graph_edge_batches(
    tx: &rusqlite::Transaction<'_>,
    request: &ArchitectureGraphReplaceRequest,
    batch_rows: usize,
    writes: &mut usize,
) -> Result<()> {
    let full_batch_sql = build_architecture_graph_edge_insert_sql(batch_rows);
    let mut tail_batch_sql = None::<String>;

    for batch in request.facts.edges.chunks(batch_rows) {
        let sql = if batch.len() == batch_rows {
            full_batch_sql.as_str()
        } else {
            tail_batch_sql
                .get_or_insert_with(|| build_architecture_graph_edge_insert_sql(batch.len()))
                .as_str()
        };
        let mut params = Vec::with_capacity(batch.len() * ARCHITECTURE_GRAPH_EDGE_INSERT_WIDTH);
        for edge in batch {
            append_architecture_graph_edge_params(&mut params, &request.repo_id, edge)?;
            *writes += 1;
            maybe_fail_architecture_graph_write(request, *writes)?;
        }
        tx.execute(sql, params_from_iter(params))
            .context("inserting architecture graph edge batch")?;
    }

    Ok(())
}

fn append_architecture_graph_node_params(
    params: &mut Vec<SqlValue>,
    repo_id: &str,
    node: &crate::capability_packs::architecture_graph::storage::ArchitectureGraphNodeFact,
) -> Result<()> {
    params.push(SqlValue::Text(repo_id.to_string()));
    params.push(SqlValue::Text(node.node_id.clone()));
    params.push(SqlValue::Text(node.node_kind.clone()));
    params.push(SqlValue::Text(node.label.clone()));
    push_optional_text(params, &node.artefact_id);
    push_optional_text(params, &node.symbol_id);
    push_optional_text(params, &node.path);
    push_optional_text(params, &node.entry_kind);
    params.push(SqlValue::Text(node.source_kind.clone()));
    params.push(SqlValue::Real(node.confidence));
    params.push(SqlValue::Text(json_string(&node.provenance)?));
    params.push(SqlValue::Text(json_string(&node.evidence)?));
    params.push(SqlValue::Text(json_string(&node.properties)?));
    push_optional_i64(params, opt_generation(node.last_observed_generation)?);
    Ok(())
}

fn append_architecture_graph_edge_params(
    params: &mut Vec<SqlValue>,
    repo_id: &str,
    edge: &crate::capability_packs::architecture_graph::storage::ArchitectureGraphEdgeFact,
) -> Result<()> {
    params.push(SqlValue::Text(repo_id.to_string()));
    params.push(SqlValue::Text(edge.edge_id.clone()));
    params.push(SqlValue::Text(edge.edge_kind.clone()));
    params.push(SqlValue::Text(edge.from_node_id.clone()));
    params.push(SqlValue::Text(edge.to_node_id.clone()));
    params.push(SqlValue::Text(edge.source_kind.clone()));
    params.push(SqlValue::Real(edge.confidence));
    params.push(SqlValue::Text(json_string(&edge.provenance)?));
    params.push(SqlValue::Text(json_string(&edge.evidence)?));
    params.push(SqlValue::Text(json_string(&edge.properties)?));
    push_optional_i64(params, opt_generation(edge.last_observed_generation)?);
    Ok(())
}

fn push_optional_text(params: &mut Vec<SqlValue>, value: &Option<String>) {
    match value {
        Some(value) => params.push(SqlValue::Text(value.clone())),
        None => params.push(SqlValue::Null),
    }
}

fn push_optional_i64(params: &mut Vec<SqlValue>, value: Option<i64>) {
    match value {
        Some(value) => params.push(SqlValue::Integer(value)),
        None => params.push(SqlValue::Null),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ArchitectureGraphReplaceRequest, RepoSqliteWriteActor,
        architecture_graph_batch_row_count_for_limit, architecture_graph_write_batch_size,
        build_architecture_graph_edge_insert_sql, build_architecture_graph_node_insert_sql,
        short_thread_label, sqlite_exec_serialized_batch_transactional_path,
        sqlite_exec_serialized_path,
    };
    use anyhow::Result;
    use serde_json::json;
    use std::path::Path;
    use tempfile::TempDir;

    use crate::capability_packs::architecture_graph::schema::architecture_graph_sqlite_schema_sql;
    use crate::capability_packs::architecture_graph::storage::{
        ArchitectureGraphEdgeFact, ArchitectureGraphFacts, ArchitectureGraphNodeFact,
    };

    fn create_sample_db() -> Result<(TempDir, std::path::PathBuf)> {
        let temp = TempDir::new().expect("temp dir");
        let db_path = temp.path().join("runtime.sqlite");
        let conn = rusqlite::Connection::open(&db_path).expect("open sqlite");
        conn.execute_batch("CREATE TABLE sample (value INTEGER NOT NULL);")
            .expect("create table");
        Ok((temp, db_path))
    }

    fn create_architecture_graph_db() -> Result<(TempDir, std::path::PathBuf)> {
        let temp = TempDir::new().expect("temp dir");
        let db_path = temp.path().join("runtime.sqlite");
        let conn = rusqlite::Connection::open(&db_path).expect("open sqlite");
        conn.execute_batch(architecture_graph_sqlite_schema_sql())
            .expect("create architecture graph schema");
        Ok((temp, db_path))
    }

    fn sample_architecture_facts(repo_id: &str) -> ArchitectureGraphFacts {
        ArchitectureGraphFacts {
            nodes: vec![ArchitectureGraphNodeFact {
                repo_id: repo_id.to_string(),
                node_id: "node-1".to_string(),
                node_kind: "NODE".to_string(),
                label: "Caller".to_string(),
                artefact_id: Some("artefact::caller".to_string()),
                symbol_id: Some("symbol::caller".to_string()),
                path: Some("src/caller.rs".to_string()),
                entry_kind: None,
                source_kind: "COMPUTED".to_string(),
                confidence: 0.9,
                provenance: json!({ "source": "test" }),
                evidence: json!([{ "path": "src/caller.rs" }]),
                properties: json!({ "language": "rust" }),
                last_observed_generation: Some(7),
            }],
            edges: vec![ArchitectureGraphEdgeFact {
                repo_id: repo_id.to_string(),
                edge_id: "edge-1".to_string(),
                edge_kind: "DEPENDS_ON".to_string(),
                from_node_id: "node-1".to_string(),
                to_node_id: "node-2".to_string(),
                source_kind: "COMPUTED".to_string(),
                confidence: 0.8,
                provenance: json!({ "source": "test" }),
                evidence: json!([{ "path": "src/caller.rs" }]),
                properties: json!({ "edge_kind": "calls" }),
                last_observed_generation: Some(7),
            }],
        }
    }

    fn sample_architecture_facts_with_counts(
        repo_id: &str,
        node_count: usize,
        edge_count: usize,
    ) -> ArchitectureGraphFacts {
        let nodes = (0..node_count)
            .map(|index| ArchitectureGraphNodeFact {
                repo_id: repo_id.to_string(),
                node_id: format!("node-{index}"),
                node_kind: "NODE".to_string(),
                label: format!("Node {index}"),
                artefact_id: Some(format!("artefact::{index}")),
                symbol_id: Some(format!("symbol::{index}")),
                path: Some(format!("src/node_{index}.rs")),
                entry_kind: None,
                source_kind: "COMPUTED".to_string(),
                confidence: 0.9,
                provenance: json!({ "source": "test", "index": index }),
                evidence: json!([{ "path": format!("src/node_{index}.rs") }]),
                properties: json!({ "language": "rust" }),
                last_observed_generation: Some(7),
            })
            .collect();
        let edges = (0..edge_count)
            .map(|index| ArchitectureGraphEdgeFact {
                repo_id: repo_id.to_string(),
                edge_id: format!("edge-{index}"),
                edge_kind: "DEPENDS_ON".to_string(),
                from_node_id: format!("node-{index}"),
                to_node_id: format!("node-{}", (index + 1) % node_count.max(1)),
                source_kind: "COMPUTED".to_string(),
                confidence: 0.8,
                provenance: json!({ "source": "test", "index": index }),
                evidence: json!([{ "path": format!("src/node_{index}.rs") }]),
                properties: json!({ "edge_kind": "calls" }),
                last_observed_generation: Some(7),
            })
            .collect();
        ArchitectureGraphFacts { nodes, edges }
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

    #[test]
    fn architecture_graph_write_batch_size_is_lowered() {
        assert_eq!(architecture_graph_write_batch_size(), 250);
    }

    #[test]
    fn architecture_graph_batch_row_count_respects_sqlite_variable_limit() {
        assert_eq!(
            architecture_graph_batch_row_count_for_limit(250, 14, 999),
            71
        );
        assert_eq!(
            architecture_graph_batch_row_count_for_limit(250, 11, 999),
            90
        );
        assert_eq!(architecture_graph_batch_row_count_for_limit(250, 14, 10), 1);
    }

    #[test]
    fn architecture_graph_node_insert_sql_batches_multiple_rows() {
        let sql = build_architecture_graph_node_insert_sql(2);
        assert!(
            sql.contains("VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, datetime('now')), (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, datetime('now'))"),
            "expected multi-row node insert SQL, got {sql}"
        );
    }

    #[test]
    fn architecture_graph_edge_insert_sql_batches_multiple_rows() {
        let sql = build_architecture_graph_edge_insert_sql(3);
        assert!(
            sql.contains("VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, datetime('now')), (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, datetime('now')), (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, datetime('now'))"),
            "expected multi-row edge insert SQL, got {sql}"
        );
    }

    #[tokio::test]
    async fn serialised_writer_replaces_architecture_graph_atomically() -> Result<()> {
        let (_temp, db_path) = create_architecture_graph_db()?;
        let conn = rusqlite::Connection::open(&db_path).expect("open sqlite");
        conn.execute(
            "INSERT INTO architecture_graph_nodes_current (
                repo_id, node_id, node_kind, label, source_kind, confidence,
                provenance_json, evidence_json, properties_json, updated_at
             ) VALUES (?1, 'stale-node', 'NODE', 'Stale', 'COMPUTED', 0.5, '{}', '[]', '{}', datetime('now'))",
            rusqlite::params!["repo-1"],
        )
        .expect("insert stale node");
        drop(conn);

        RepoSqliteWriteActor::shared_for_path(&db_path)?
            .replace_architecture_graph(ArchitectureGraphReplaceRequest {
                repo_id: "repo-1".to_string(),
                facts: sample_architecture_facts("repo-1"),
                generation_seq: 7,
                warnings: vec!["warning-a".to_string()],
                metrics: json!({ "nodes": 1, "edges": 1 }),
                fail_after_writes: None,
            })
            .await?;

        let conn = rusqlite::Connection::open(&db_path).expect("re-open sqlite");
        let node_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM architecture_graph_nodes_current WHERE repo_id = 'repo-1'",
                [],
                |row| row.get(0),
            )
            .expect("count nodes");
        let edge_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM architecture_graph_edges_current WHERE repo_id = 'repo-1'",
                [],
                |row| row.get(0),
            )
            .expect("count edges");
        let run_row: (i64, String, String) = conn
            .query_row(
                "SELECT last_generation_seq, warnings_json, metrics_json
                 FROM architecture_graph_runs_current WHERE repo_id = 'repo-1'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .expect("load run row");

        assert_eq!(node_count, 1);
        assert_eq!(edge_count, 1);
        assert_eq!(run_row.0, 7);
        assert_eq!(run_row.1, "[\"warning-a\"]");
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&run_row.2).expect("parse metrics JSON"),
            json!({ "nodes": 1, "edges": 1 })
        );

        Ok(())
    }

    #[tokio::test]
    async fn serialised_writer_replaces_large_architecture_graph_across_batches() -> Result<()> {
        let (_temp, db_path) = create_architecture_graph_db()?;
        let facts = sample_architecture_facts_with_counts(
            "repo-1",
            architecture_graph_write_batch_size() + 5,
            architecture_graph_write_batch_size() + 7,
        );

        RepoSqliteWriteActor::shared_for_path(&db_path)?
            .replace_architecture_graph(ArchitectureGraphReplaceRequest {
                repo_id: "repo-1".to_string(),
                generation_seq: 7,
                warnings: Vec::new(),
                metrics: json!({
                    "nodes": facts.nodes.len(),
                    "edges": facts.edges.len(),
                }),
                facts,
                fail_after_writes: None,
            })
            .await?;

        let conn = rusqlite::Connection::open(&db_path).expect("re-open sqlite");
        let node_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM architecture_graph_nodes_current WHERE repo_id = 'repo-1'",
                [],
                |row| row.get(0),
            )
            .expect("count nodes");
        let edge_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM architecture_graph_edges_current WHERE repo_id = 'repo-1'",
                [],
                |row| row.get(0),
            )
            .expect("count edges");

        assert_eq!(
            usize::try_from(node_count).expect("node count fits"),
            architecture_graph_write_batch_size() + 5
        );
        assert_eq!(
            usize::try_from(edge_count).expect("edge count fits"),
            architecture_graph_write_batch_size() + 7
        );

        Ok(())
    }

    #[tokio::test]
    async fn serialised_writer_rolls_back_failed_architecture_graph_replacement() -> Result<()> {
        let (_temp, db_path) = create_architecture_graph_db()?;
        let conn = rusqlite::Connection::open(&db_path).expect("open sqlite");
        conn.execute(
            "INSERT INTO architecture_graph_nodes_current (
                repo_id, node_id, node_kind, label, source_kind, confidence,
                provenance_json, evidence_json, properties_json, updated_at
             ) VALUES (?1, 'stable-node', 'NODE', 'Stable', 'COMPUTED', 0.5, '{}', '[]', '{}', datetime('now'))",
            rusqlite::params!["repo-1"],
        )
        .expect("insert stable node");
        drop(conn);

        let err = RepoSqliteWriteActor::shared_for_path(&db_path)?
            .replace_architecture_graph(ArchitectureGraphReplaceRequest {
                repo_id: "repo-1".to_string(),
                facts: sample_architecture_facts("repo-1"),
                generation_seq: 8,
                warnings: Vec::new(),
                metrics: json!({ "nodes": 1 }),
                fail_after_writes: Some(1),
            })
            .await
            .expect_err("replacement should fail");
        assert!(
            err.to_string()
                .contains("injected architecture graph write failure"),
            "unexpected error: {err:#}"
        );

        let conn = rusqlite::Connection::open(&db_path).expect("re-open sqlite");
        let labels = conn
            .prepare(
                "SELECT label FROM architecture_graph_nodes_current
                 WHERE repo_id = 'repo-1' ORDER BY node_id ASC",
            )
            .expect("prepare label query")
            .query_map([], |row| row.get::<_, String>(0))
            .expect("query labels")
            .collect::<std::result::Result<Vec<_>, _>>()
            .expect("collect labels");
        let run_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM architecture_graph_runs_current WHERE repo_id = 'repo-1'",
                [],
                |row| row.get(0),
            )
            .expect("count run rows");

        assert_eq!(labels, vec!["Stable"]);
        assert_eq!(run_count, 0);

        Ok(())
    }

    #[tokio::test]
    async fn serialised_writer_rejects_node_repo_scope_mismatches() -> Result<()> {
        let (_temp, db_path) = create_architecture_graph_db()?;
        let conn = rusqlite::Connection::open(&db_path).expect("open sqlite");
        conn.execute(
            "INSERT INTO architecture_graph_nodes_current (
                repo_id, node_id, node_kind, label, source_kind, confidence,
                provenance_json, evidence_json, properties_json, updated_at
             ) VALUES (?1, 'stable-node', 'NODE', 'Stable', 'COMPUTED', 0.5, '{}', '[]', '{}', datetime('now'))",
            rusqlite::params!["repo-1"],
        )
        .expect("insert stable node");
        drop(conn);

        let mut facts = sample_architecture_facts("repo-1");
        facts.nodes[0].repo_id = "repo-2".to_string();

        let err = RepoSqliteWriteActor::shared_for_path(&db_path)?
            .replace_architecture_graph(ArchitectureGraphReplaceRequest {
                repo_id: "repo-1".to_string(),
                facts,
                generation_seq: 8,
                warnings: Vec::new(),
                metrics: json!({ "nodes": 1 }),
                fail_after_writes: None,
            })
            .await
            .expect_err("replacement should fail");
        assert!(
            err.to_string().contains("did not match replacement repo"),
            "unexpected error: {err:#}"
        );

        let conn = rusqlite::Connection::open(&db_path).expect("re-open sqlite");
        let labels = conn
            .prepare(
                "SELECT label FROM architecture_graph_nodes_current
                 WHERE repo_id = 'repo-1' ORDER BY node_id ASC",
            )
            .expect("prepare label query")
            .query_map([], |row| row.get::<_, String>(0))
            .expect("query labels")
            .collect::<std::result::Result<Vec<_>, _>>()
            .expect("collect labels");

        assert_eq!(labels, vec!["Stable"]);

        Ok(())
    }

    #[tokio::test]
    async fn serialised_writer_rejects_edge_repo_scope_mismatches() -> Result<()> {
        let (_temp, db_path) = create_architecture_graph_db()?;
        let conn = rusqlite::Connection::open(&db_path).expect("open sqlite");
        conn.execute(
            "INSERT INTO architecture_graph_nodes_current (
                repo_id, node_id, node_kind, label, source_kind, confidence,
                provenance_json, evidence_json, properties_json, updated_at
             ) VALUES (?1, 'stable-node', 'NODE', 'Stable', 'COMPUTED', 0.5, '{}', '[]', '{}', datetime('now'))",
            rusqlite::params!["repo-1"],
        )
        .expect("insert stable node");
        drop(conn);

        let mut facts = sample_architecture_facts("repo-1");
        facts.edges[0].repo_id = "repo-2".to_string();

        let err = RepoSqliteWriteActor::shared_for_path(&db_path)?
            .replace_architecture_graph(ArchitectureGraphReplaceRequest {
                repo_id: "repo-1".to_string(),
                facts,
                generation_seq: 8,
                warnings: Vec::new(),
                metrics: json!({ "edges": 1 }),
                fail_after_writes: None,
            })
            .await
            .expect_err("replacement should fail");
        assert!(
            err.to_string().contains("did not match replacement repo"),
            "unexpected error: {err:#}"
        );

        let conn = rusqlite::Connection::open(&db_path).expect("re-open sqlite");
        let labels = conn
            .prepare(
                "SELECT label FROM architecture_graph_nodes_current
                 WHERE repo_id = 'repo-1' ORDER BY node_id ASC",
            )
            .expect("prepare label query")
            .query_map([], |row| row.get::<_, String>(0))
            .expect("query labels")
            .collect::<std::result::Result<Vec<_>, _>>()
            .expect("collect labels");

        assert_eq!(labels, vec!["Stable"]);

        Ok(())
    }
}
