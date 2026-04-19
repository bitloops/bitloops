use std::collections::HashSet;

use rusqlite::Connection;

use super::{
    load_current_edges_for_local_reconciliation_with_connection,
    load_current_source_facts_for_paths_with_connection,
    load_current_targets_for_paths_for_local_resolution_with_connection,
};

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
                metadata TEXT NOT NULL
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
