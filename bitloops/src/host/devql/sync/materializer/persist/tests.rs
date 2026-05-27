use std::collections::HashSet;
use std::time::Duration;

use rusqlite::Connection;
use tempfile::TempDir;

use super::{
    load_current_edges_for_local_reconciliation_with_connection,
    load_current_source_facts_for_paths_with_connection,
    load_current_targets_for_paths_for_local_resolution_with_connection,
    reconcile_current_local_edges_for_paths_with_write_lock,
    reconcile_current_local_edges_for_paths_with_write_lock_and_progress,
    repo_wide_targets_for_source_path,
    shared_repo_wide_target_cache_keys_for_touched_unresolved_source_paths,
};
use crate::host::language_adapter::LocalTargetInfo;

fn setup_edges_table(connection: &Connection) {
    connection
        .execute_batch(
            "CREATE TABLE artefact_edges_current (
                repo_id TEXT NOT NULL,
                edge_id TEXT NOT NULL,
                path TEXT NOT NULL,
                content_id TEXT NOT NULL,
                from_symbol_id TEXT NOT NULL,
                from_artefact_id TEXT NOT NULL,
                to_symbol_id TEXT,
                to_artefact_id TEXT,
                to_symbol_ref TEXT,
                edge_kind TEXT NOT NULL,
                language TEXT NOT NULL,
                start_line INTEGER,
                end_line INTEGER,
                metadata TEXT NOT NULL,
                updated_at TEXT
            );",
        )
        .expect("create artefact_edges_current");
}

fn setup_artefacts_table(connection: &Connection) {
    connection
        .execute_batch(
            "CREATE TABLE artefacts_current (
                repo_id TEXT NOT NULL,
                path TEXT NOT NULL,
                content_id TEXT NOT NULL,
                symbol_id TEXT NOT NULL,
                artefact_id TEXT NOT NULL,
                language TEXT NOT NULL,
                canonical_kind TEXT,
                language_kind TEXT,
                symbol_fqn TEXT,
                start_line INTEGER NOT NULL,
                end_line INTEGER NOT NULL,
                start_byte INTEGER NOT NULL,
                end_byte INTEGER NOT NULL,
                signature TEXT,
                modifiers TEXT NOT NULL DEFAULT '[]',
                updated_at TEXT NOT NULL
            );",
        )
        .expect("create artefacts_current");
}

fn insert_edge(
    connection: &Connection,
    edge_id: &str,
    to_symbol_id: Option<&str>,
    to_symbol_ref: Option<&str>,
    language: &str,
) {
    connection
        .execute(
            "INSERT INTO artefact_edges_current
             (repo_id, edge_id, path, content_id, from_symbol_id, from_artefact_id, to_symbol_id, to_artefact_id, to_symbol_ref, edge_kind, language, start_line, end_line, metadata)
             VALUES
             ('repo', ?1, 'src/caller.rs', 'content', 'from-symbol', 'from-artefact', ?2, 'to-artefact', ?3, 'calls', ?4, NULL, NULL, '{}')",
            rusqlite::params![edge_id, to_symbol_id, to_symbol_ref, language],
        )
        .expect("insert current edge");
}

fn insert_target(
    connection: &Connection,
    path: &str,
    symbol_id: &str,
    artefact_id: &str,
    symbol_fqn: &str,
    language: &str,
    language_kind: &str,
) {
    connection
        .execute(
            "INSERT INTO artefacts_current (
                repo_id, path, content_id, symbol_id, artefact_id, language,
                canonical_kind, language_kind, symbol_fqn, start_line, end_line, start_byte,
                end_byte, signature, modifiers, updated_at
            ) VALUES (
                'repo', ?1, 'content', ?2, ?3, ?4,
                'function', ?5, ?6, 1, 1, 0, 1, 'fn demo()', '[]', '2026-04-17T00:00:00Z'
            )",
            rusqlite::params![
                path,
                symbol_id,
                artefact_id,
                language,
                language_kind,
                symbol_fqn
            ],
        )
        .expect("insert current target");
}

fn local_target_info(path: &str, symbol_fqn: &str) -> LocalTargetInfo {
    LocalTargetInfo {
        symbol_fqn: symbol_fqn.to_string(),
        symbol_id: format!("{path}::symbol"),
        artefact_id: format!("{path}::artefact"),
        language_kind: "function_declaration".to_string(),
    }
}

#[test]
fn current_edge_reconciliation_waits_for_write_lock_before_reading_state() {
    let temp = TempDir::new().expect("temp dir");
    let sqlite_path = temp.path().join("devql.sqlite");
    let mut connection = Connection::open(&sqlite_path).expect("open sqlite");
    setup_edges_table(&connection);
    setup_artefacts_table(&connection);

    let (locked_tx, locked_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let sqlite_path_for_blocker = sqlite_path.clone();
    let blocker = std::thread::spawn(move || {
        crate::storage::sqlite::with_sqlite_write_lock(&sqlite_path_for_blocker, || {
            locked_tx.send(()).expect("signal lock held");
            release_rx.recv().expect("wait for release signal");
            let connection =
                Connection::open(&sqlite_path_for_blocker).expect("open sqlite in blocker");
            insert_edge(
                &connection,
                "stale-edge",
                Some("old-symbol"),
                Some("src/utils.ts::helper"),
                "typescript",
            );
            insert_target(
                &connection,
                "src/utils.ts",
                "new-symbol",
                "new-artefact",
                "src/utils.ts::helper",
                "typescript",
                "function_declaration",
            );
            Ok(())
        })
        .expect("hold sqlite write lock");
    });
    locked_rx.recv().expect("wait for sqlite lock");

    let sqlite_path_for_reconcile = sqlite_path.clone();
    let (started_tx, started_rx) = std::sync::mpsc::channel();
    let (done_tx, done_rx) = std::sync::mpsc::channel();
    let worker = std::thread::spawn(move || {
        started_tx.send(()).expect("signal reconcile started");
        let result = reconcile_current_local_edges_for_paths_with_write_lock(
            &mut connection,
            &sqlite_path_for_reconcile,
            "repo",
            &["src/utils.ts".to_string()],
        );
        done_tx.send(result).expect("send reconcile result");
    });
    started_rx.recv().expect("wait for reconcile start");
    assert!(
        done_rx.recv_timeout(Duration::from_millis(50)).is_err(),
        "current edge reconciliation should not complete before the held write lock is released"
    );
    release_tx.send(()).expect("release sqlite write lock");
    let outcome = done_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("wait for reconcile result")
        .expect("reconcile current local edges");
    worker.join().expect("join reconcile worker");
    blocker.join().expect("join sqlite lock blocker");
    assert!(
        outcome.affected_rows > 0,
        "reconciliation should see rows inserted while the write lock was held"
    );
}

#[test]
fn load_current_edges_for_local_reconciliation_only_fetches_relevant_rows() {
    let connection = Connection::open_in_memory().expect("open in-memory sqlite");
    setup_edges_table(&connection);

    insert_edge(
        &connection,
        "unresolved-rust",
        None,
        Some("super::helper"),
        "rust",
    );
    insert_edge(
        &connection,
        "resolved-touched-typescript",
        Some("helper-symbol"),
        Some("src/utils.ts::helper"),
        "typescript",
    );
    insert_edge(
        &connection,
        "resolved-untouched-typescript",
        Some("other-symbol"),
        Some("src/other.ts::helper"),
        "typescript",
    );
    insert_edge(
        &connection,
        "resolved-touched-unsupported",
        Some("unsupported-symbol"),
        Some("src/utils.kt::helper"),
        "kotlin",
    );
    insert_edge(
        &connection,
        "unresolved-unsupported",
        None,
        Some("Helper"),
        "swift",
    );
    insert_edge(&connection, "missing-ref", None, None, "rust");

    let touched_paths = HashSet::from(["src/utils.ts".to_string()]);
    let rows = load_current_edges_for_local_reconciliation_with_connection(
        &connection,
        "repo",
        &touched_paths,
    )
    .expect("load current reconciliation edges");

    let mut edge_ids = rows
        .into_iter()
        .map(|edge| edge.edge_id)
        .collect::<Vec<_>>();
    edge_ids.sort();

    assert_eq!(
        edge_ids,
        vec![
            "resolved-touched-typescript".to_string(),
            "unresolved-rust".to_string(),
        ]
    );
}

#[test]
fn load_current_edges_for_local_reconciliation_matches_touched_paths_exactly() {
    let connection = Connection::open_in_memory().expect("open in-memory sqlite");
    setup_edges_table(&connection);

    insert_edge(
        &connection,
        "resolved-exact",
        Some("helper-symbol"),
        Some("src/100%_util.ts::helper"),
        "typescript",
    );
    insert_edge(
        &connection,
        "resolved-accidental-like-match",
        Some("other-symbol"),
        Some("src/100abcxutil.ts::helper"),
        "typescript",
    );

    let touched_paths = HashSet::from(["src/100%_util.ts".to_string()]);
    let rows = load_current_edges_for_local_reconciliation_with_connection(
        &connection,
        "repo",
        &touched_paths,
    )
    .expect("load current reconciliation edges");

    let edge_ids = rows
        .into_iter()
        .map(|edge| edge.edge_id)
        .collect::<Vec<_>>();
    assert_eq!(edge_ids, vec!["resolved-exact".to_string()]);
}

#[test]
fn load_current_targets_for_local_resolution_only_fetches_touched_supported_paths() {
    let connection = Connection::open_in_memory().expect("open in-memory sqlite");
    setup_artefacts_table(&connection);

    insert_target(
        &connection,
        "src/utils.ts",
        "symbol-utils",
        "artefact-utils",
        "src/utils.ts::helper",
        "typescript",
        "function_declaration",
    );
    insert_target(
        &connection,
        "src/other.ts",
        "symbol-other",
        "artefact-other",
        "src/other.ts::helper",
        "typescript",
        "function_declaration",
    );
    insert_target(
        &connection,
        "src/unsupported.kt",
        "symbol-kotlin",
        "artefact-kotlin",
        "src/unsupported.kt::helper",
        "kotlin",
        "function_declaration",
    );

    let touched_paths =
        HashSet::from(["src/utils.ts".to_string(), "src/unsupported.kt".to_string()]);
    let rows = load_current_targets_for_paths_for_local_resolution_with_connection(
        &connection,
        "repo",
        &touched_paths,
    )
    .expect("load scoped current targets");

    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].symbol_fqn, "src/utils.ts::helper");
}

#[test]
fn load_current_edges_for_local_reconciliation_handles_large_touched_path_sets() {
    let connection = Connection::open_in_memory().expect("open in-memory sqlite");
    setup_edges_table(&connection);

    insert_edge(
        &connection,
        "resolved-large-set",
        Some("helper-symbol"),
        Some("src/utils.ts::helper"),
        "typescript",
    );
    insert_edge(
        &connection,
        "resolved-untouched",
        Some("other-symbol"),
        Some("src/other.ts::helper"),
        "typescript",
    );

    let mut touched_paths = (0..1_200)
        .map(|index| format!("src/generated_{index}.ts"))
        .collect::<HashSet<_>>();
    touched_paths.insert("src/utils.ts".to_string());

    let rows = load_current_edges_for_local_reconciliation_with_connection(
        &connection,
        "repo",
        &touched_paths,
    )
    .expect("load current reconciliation edges for a large touched path set");

    let edge_ids = rows
        .into_iter()
        .map(|edge| edge.edge_id)
        .collect::<Vec<_>>();
    assert_eq!(edge_ids, vec!["resolved-large-set".to_string()]);
}

#[test]
fn load_current_source_facts_for_paths_only_fetches_requested_paths() {
    let connection = Connection::open_in_memory().expect("open in-memory sqlite");
    setup_edges_table(&connection);
    setup_artefacts_table(&connection);

    connection
        .execute(
            "INSERT INTO artefact_edges_current
             (repo_id, edge_id, path, content_id, from_symbol_id, from_artefact_id, to_symbol_id, to_artefact_id, to_symbol_ref, edge_kind, language, start_line, end_line, metadata)
             VALUES
             ('repo', 'import-a', 'src/a.rs', 'content', 'from-a', 'artefact-a', NULL, NULL, 'crate::helper', 'imports', 'rust', NULL, NULL, '{}'),
             ('repo', 'import-b', 'src/b.rs', 'content', 'from-b', 'artefact-b', NULL, NULL, 'crate::other', 'imports', 'rust', NULL, NULL, '{}')",
            [],
        )
        .expect("insert import refs");
    insert_target(
        &connection,
        "src/a.rs",
        "package-a",
        "artefact-package-a",
        "src/a.rs::package",
        "rust",
        "package_declaration",
    );
    insert_target(
        &connection,
        "src/a.rs",
        "namespace-a",
        "artefact-namespace-a",
        "src/a.rs::ns::demo",
        "csharp",
        "namespace_declaration",
    );
    insert_target(
        &connection,
        "src/b.rs",
        "package-b",
        "artefact-package-b",
        "src/b.rs::package",
        "rust",
        "package_declaration",
    );

    let source_paths = HashSet::from(["src/a.rs".to_string()]);
    let facts =
        load_current_source_facts_for_paths_with_connection(&connection, "repo", &source_paths)
            .expect("load scoped source facts");

    assert_eq!(facts.len(), 1);
    let source_facts = facts.get("src/a.rs").expect("facts for src/a.rs");
    assert_eq!(source_facts.import_refs, vec!["crate::helper".to_string()]);
    assert_eq!(source_facts.package_refs, vec!["package".to_string()]);
    assert_eq!(source_facts.namespace_refs, vec!["demo".to_string()]);
}

#[test]
fn current_edge_reconciliation_chunks_large_rewrite_sets() {
    let temp = TempDir::new().expect("temp dir");
    let sqlite_path = temp.path().join("devql.sqlite");
    let mut connection = Connection::open(&sqlite_path).expect("open sqlite");
    setup_edges_table(&connection);
    setup_artefacts_table(&connection);

    insert_target(
        &connection,
        "src/utils.ts",
        "helper-symbol",
        "helper-artefact",
        "src/utils.ts::helper",
        "typescript",
        "function_declaration",
    );
    for index in 0..251 {
        connection
            .execute(
                "INSERT INTO artefact_edges_current
                 (repo_id, edge_id, path, content_id, from_symbol_id, from_artefact_id, to_symbol_id, to_artefact_id, to_symbol_ref, edge_kind, language, start_line, end_line, metadata)
                 VALUES
                 ('repo', ?1, ?2, 'content', ?3, ?4, ?5, ?6, 'src/utils.ts::helper', 'calls', 'typescript', ?7, ?7, '{}')",
                rusqlite::params![
                    format!("edge-{index}"),
                    format!("src/caller_{index}.ts"),
                    format!("from-symbol-{index}"),
                    format!("from-artefact-{index}"),
                    format!("stale-symbol-{index}"),
                    format!("stale-artefact-{index}"),
                    index as i64,
                ],
            )
            .expect("insert stale resolved current edge");
    }

    let touched_paths = vec!["src/utils.ts".to_string()];
    let outcome = reconcile_current_local_edges_for_paths_with_write_lock(
        &mut connection,
        &sqlite_path,
        "repo",
        &touched_paths,
    )
    .expect("reconcile chunked current local edges");

    assert_eq!(
        outcome.sqlite_phase_metrics.phase_name,
        Some("current_edge_reconcile")
    );
    assert!(
        outcome.sqlite_phase_metrics.transaction_count > 1,
        "large current-edge rewrites should span multiple transactions"
    );
    let reconciled_count: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM artefact_edges_current WHERE repo_id = 'repo' AND to_symbol_id = 'helper-symbol'",
            [],
            |row| row.get(0),
        )
        .expect("count reconciled rows");
    assert_eq!(reconciled_count, 251);
}

#[test]
fn current_edge_reconciliation_reports_source_path_batch_progress() {
    let temp = TempDir::new().expect("temp dir");
    let sqlite_path = temp.path().join("devql.sqlite");
    let mut connection = Connection::open(&sqlite_path).expect("open sqlite");
    setup_edges_table(&connection);
    setup_artefacts_table(&connection);

    insert_target(
        &connection,
        "src/utils.ts",
        "helper-symbol",
        "helper-artefact",
        "src/utils.ts::helper",
        "typescript",
        "function_declaration",
    );
    for index in 0..251 {
        connection
            .execute(
                "INSERT INTO artefact_edges_current
                 (repo_id, edge_id, path, content_id, from_symbol_id, from_artefact_id, to_symbol_id, to_artefact_id, to_symbol_ref, edge_kind, language, start_line, end_line, metadata)
                 VALUES
                 ('repo', ?1, ?2, 'content', ?3, ?4, ?5, ?6, 'src/utils.ts::helper', 'calls', 'typescript', ?7, ?7, '{}')",
                rusqlite::params![
                    format!("edge-{index}"),
                    format!("src/caller_{index}.ts"),
                    format!("from-symbol-{index}"),
                    format!("from-artefact-{index}"),
                    format!("stale-symbol-{index}"),
                    format!("stale-artefact-{index}"),
                    index as i64,
                ],
            )
            .expect("insert stale resolved current edge");
    }

    let mut progress_updates = Vec::new();
    let touched_paths = vec!["src/utils.ts".to_string()];
    let outcome = reconcile_current_local_edges_for_paths_with_write_lock_and_progress(
        &mut connection,
        &sqlite_path,
        "repo",
        &touched_paths,
        |update| progress_updates.push(update),
    )
    .expect("reconcile chunked current local edges with progress");

    assert!(outcome.affected_rows > 0);
    assert!(
        progress_updates.len() > 1,
        "251 unique source paths should report multiple reconcile batches, got {progress_updates:?}"
    );
    assert_eq!(
        progress_updates
            .last()
            .map(|update| update.source_paths_total),
        Some(251)
    );
    assert_eq!(
        progress_updates
            .last()
            .map(|update| update.source_paths_completed),
        Some(251)
    );
}

#[test]
fn shared_repo_wide_target_cache_keys_group_compatible_languages_once() {
    let touched_paths = HashSet::from([
        "src/a.ts".to_string(),
        "src/b.js".to_string(),
        "pkg/c.py".to_string(),
    ]);
    let current_edges = vec![
        crate::host::devql::sync::materializer::persist::CurrentEdgeRecord {
            edge_id: "edge-ts".to_string(),
            path: "src/a.ts".to_string(),
            content_id: "content".to_string(),
            from_symbol_id: "from-ts".to_string(),
            from_artefact_id: "from-ts-artefact".to_string(),
            to_symbol_id: None,
            to_artefact_id: None,
            to_symbol_ref: Some("./dep::helper".to_string()),
            edge_kind: "calls".to_string(),
            language: "typescript".to_string(),
            start_line: None,
            end_line: None,
            metadata_json: "{}".to_string(),
        },
        crate::host::devql::sync::materializer::persist::CurrentEdgeRecord {
            edge_id: "edge-js".to_string(),
            path: "src/b.js".to_string(),
            content_id: "content".to_string(),
            from_symbol_id: "from-js".to_string(),
            from_artefact_id: "from-js-artefact".to_string(),
            to_symbol_id: None,
            to_artefact_id: None,
            to_symbol_ref: Some("./dep::helper".to_string()),
            edge_kind: "calls".to_string(),
            language: "javascript".to_string(),
            start_line: None,
            end_line: None,
            metadata_json: "{}".to_string(),
        },
        crate::host::devql::sync::materializer::persist::CurrentEdgeRecord {
            edge_id: "edge-py".to_string(),
            path: "pkg/c.py".to_string(),
            content_id: "content".to_string(),
            from_symbol_id: "from-py".to_string(),
            from_artefact_id: "from-py-artefact".to_string(),
            to_symbol_id: None,
            to_artefact_id: None,
            to_symbol_ref: Some("pkg.dep::helper".to_string()),
            edge_kind: "calls".to_string(),
            language: "python".to_string(),
            start_line: None,
            end_line: None,
            metadata_json: "{}".to_string(),
        },
    ];

    let cache_keys = shared_repo_wide_target_cache_keys_for_touched_unresolved_source_paths(
        &current_edges,
        &touched_paths,
    );

    assert_eq!(cache_keys.len(), 2);
    assert!(cache_keys.contains("javascript|typescript"));
    assert!(cache_keys.contains("python"));
}

#[test]
fn repo_wide_targets_for_source_path_excludes_same_path_targets() {
    let repo_wide_targets = vec![
        local_target_info("pkg/a.py", "pkg/a.py::helper"),
        local_target_info("pkg/dep.py", "pkg/dep.py::helper"),
    ];

    let filtered = repo_wide_targets_for_source_path(&repo_wide_targets, "pkg/a.py");

    assert_eq!(filtered.len(), 1);
    assert_eq!(filtered[0].symbol_fqn, "pkg/dep.py::helper");
}

#[test]
fn current_edge_reconciliation_uses_shared_repo_cache_without_self_targets() {
    let temp = TempDir::new().expect("temp dir");
    let sqlite_path = temp.path().join("devql.sqlite");
    let mut connection = Connection::open(&sqlite_path).expect("open sqlite");
    setup_edges_table(&connection);
    setup_artefacts_table(&connection);

    insert_target(
        &connection,
        "pkg/a.py",
        "self-symbol",
        "self-artefact",
        "pkg/a.py::helper",
        "python",
        "function_declaration",
    );
    insert_target(
        &connection,
        "pkg/dep.py",
        "dep-symbol",
        "dep-artefact",
        "pkg/dep.py::helper",
        "python",
        "function_declaration",
    );
    connection
        .execute(
            "INSERT INTO artefact_edges_current
             (repo_id, edge_id, path, content_id, from_symbol_id, from_artefact_id, to_symbol_id, to_artefact_id, to_symbol_ref, edge_kind, language, start_line, end_line, metadata)
             VALUES
             ('repo', 'dep-edge', 'pkg/a.py', 'content', 'from-a', 'artefact-a', NULL, NULL, 'pkg.dep::helper', 'calls', 'python', NULL, NULL, '{}'),
             ('repo', 'self-edge', 'pkg/a.py', 'content', 'from-a', 'artefact-a', NULL, NULL, 'pkg.a::helper', 'calls', 'python', NULL, NULL, '{}')",
            [],
        )
        .expect("insert unresolved current edges");

    let outcome = reconcile_current_local_edges_for_paths_with_write_lock(
        &mut connection,
        &sqlite_path,
        "repo",
        &["pkg/a.py".to_string()],
    )
    .expect("reconcile current local edges");

    assert!(outcome.affected_rows > 0);

    let dep_edge: (Option<String>, Option<String>) = connection
        .query_row(
            "SELECT to_symbol_id, to_symbol_ref
             FROM artefact_edges_current
             WHERE repo_id = 'repo' AND edge_id != 'dep-edge' AND path = 'pkg/a.py' AND to_symbol_ref = 'pkg/dep.py::helper'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("load resolved dep edge");
    assert_eq!(dep_edge.0.as_deref(), Some("dep-symbol"));
    assert_eq!(dep_edge.1.as_deref(), Some("pkg/dep.py::helper"));

    let unresolved_self_edges: i64 = connection
        .query_row(
            "SELECT COUNT(*)
             FROM artefact_edges_current
             WHERE repo_id = 'repo'
               AND path = 'pkg/a.py'
               AND to_symbol_ref = 'pkg.a::helper'
               AND to_symbol_id IS NULL",
            [],
            |row| row.get(0),
        )
        .expect("count unresolved self edges");
    assert_eq!(unresolved_self_edges, 1);
}
