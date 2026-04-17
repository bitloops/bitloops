use std::collections::HashSet;

use rusqlite::Connection;

use super::load_current_edges_for_local_reconciliation_with_connection;

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
