use std::path::Path;

use anyhow::Result;
use serde_json::json;
use tempfile::TempDir;

use super::{
    ArchitectureGraphReplaceRequest, RepoSqliteWriteActor,
    architecture_graph_batch_row_count_for_limit, architecture_graph_write_batch_size,
    build_architecture_graph_edge_insert_sql, build_architecture_graph_node_insert_sql,
    short_thread_label, sqlite_exec_serialized_batch_transactional_path,
    sqlite_exec_serialized_path,
};
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
