use tempfile::tempdir;

use crate::capability_packs::semantic_clones::embeddings;
use crate::capability_packs::semantic_clones::schema::{
    semantic_clones_postgres_schema_sql, semantic_clones_sqlite_schema_sql,
};
use crate::capability_packs::semantic_clones::scoring::{CloneScoringOptions, SymbolCloneEdgeRow};
use crate::capability_packs::semantic_clones::{
    init_sqlite_semantic_embeddings_schema, init_sqlite_semantic_features_schema,
};
use crate::host::devql::{
    RelationalDialect, RelationalPrimaryBackend, RelationalStorage, devql_schema_sql_sqlite,
    sqlite_exec_path_allow_create,
};
use serde_json::json;

use super::orchestrator::{rebuild_symbol_clone_edges, rebuild_symbol_clone_edges_with_options};
use super::persistence::{
    build_persist_symbol_clone_edge_statement, replace_repo_symbol_clone_edges_for_projection,
};
use super::queries::build_symbol_clone_candidate_lookup_sql;
use super::schema::CloneProjection;
use super::state::choose_current_projection_embedding_state;

fn sample_symbol_clone_edge_row() -> SymbolCloneEdgeRow {
    SymbolCloneEdgeRow {
        repo_id: "repo-1".to_string(),
        source_symbol_id: "source-symbol".to_string(),
        source_artefact_id: "source-artefact".to_string(),
        target_symbol_id: "target-symbol".to_string(),
        target_artefact_id: "target-artefact".to_string(),
        relation_kind: "similar_implementation".to_string(),
        score: 0.91,
        semantic_score: 0.9,
        lexical_score: 0.8,
        structural_score: 0.7,
        clone_input_hash: "clone-input-hash".to_string(),
        explanation_json: json!({"reason": "matched behavior"}),
    }
}

#[test]
fn semantic_clone_schema_includes_clone_edge_table() {
    let pg = semantic_clones_postgres_schema_sql();
    let sqlite = semantic_clones_sqlite_schema_sql();

    assert!(pg.contains("CREATE TABLE IF NOT EXISTS symbol_clone_edges"));
    assert!(sqlite.contains("CREATE TABLE IF NOT EXISTS symbol_clone_edges"));
    assert!(pg.contains("CREATE TABLE IF NOT EXISTS symbol_clone_edges_current"));
    assert!(sqlite.contains("CREATE TABLE IF NOT EXISTS symbol_clone_edges_current"));
    assert!(pg.contains("PRIMARY KEY (repo_id, source_artefact_id, target_artefact_id)"));
    assert!(pg.contains("PRIMARY KEY (repo_id, source_symbol_id, target_symbol_id)"));
}

#[test]
fn semantic_clone_candidate_lookup_sql_loads_all_indexed_candidates() {
    let sql = build_symbol_clone_candidate_lookup_sql(
        "repo'1",
        CloneProjection::Current,
        &embeddings::ActiveEmbeddingRepresentationState::new(
            embeddings::EmbeddingRepresentationKind::Code,
            embeddings::EmbeddingSetup::new(
                crate::host::inference::BITLOOPS_EMBEDDINGS_IPC_DRIVER,
                "bge-m3",
                3,
            ),
        ),
    );

    assert!(sql.contains("FROM symbol_embeddings_current e"));
    assert!(sql.contains("LEFT JOIN symbol_semantics_current ss"));
    assert!(sql.contains("JOIN artefacts_current a"));
    assert!(sql.contains("e.provider AS embedding_provider"));
    assert!(sql.contains("e.model AS embedding_model"));
    assert!(sql.contains("repo''1"));
    assert!(sql.contains("representation_rank = 1"));
    assert!(sql.contains("setup_fingerprint"));
}

#[tokio::test]
async fn rebuild_wrapper_matches_default_options_on_empty_snapshot() {
    let tmp = tempdir().expect("temp dir");
    let sqlite_path = tmp.path().join("devql.sqlite");

    sqlite_exec_path_allow_create(&sqlite_path, devql_schema_sql_sqlite())
        .await
        .expect("core devql schema");
    init_sqlite_semantic_features_schema(&sqlite_path)
        .await
        .expect("semantic feature schema");
    init_sqlite_semantic_embeddings_schema(&sqlite_path)
        .await
        .expect("semantic embedding schema");
    sqlite_exec_path_allow_create(&sqlite_path, semantic_clones_sqlite_schema_sql())
        .await
        .expect("semantic clone schema");

    let relational = RelationalStorage::local_only(sqlite_path.clone());
    let wrapper = rebuild_symbol_clone_edges(&relational, "repo-1")
        .await
        .expect("wrapper rebuild");
    let explicit = rebuild_symbol_clone_edges_with_options(
        &relational,
        "repo-1",
        CloneScoringOptions::default(),
    )
    .await
    .expect("explicit rebuild");
    assert_eq!(wrapper, explicit);
    assert!(wrapper.edges.is_empty());

    let conn = rusqlite::Connection::open(&sqlite_path).expect("open sqlite");
    let mut stmt = conn
        .prepare("PRAGMA table_info(symbol_clone_edges)")
        .expect("prepare pragma table info");
    let columns = stmt
        .query_map([], |row| row.get::<_, String>(1))
        .expect("query table columns")
        .collect::<std::result::Result<Vec<_>, _>>()
        .expect("read table columns");
    assert_eq!(
        columns,
        vec![
            "repo_id",
            "source_symbol_id",
            "source_artefact_id",
            "target_symbol_id",
            "target_artefact_id",
            "relation_kind",
            "score",
            "semantic_score",
            "lexical_score",
            "structural_score",
            "clone_input_hash",
            "explanation_json",
            "generated_at",
        ]
    );
}

#[tokio::test]
async fn historical_rebuild_leaves_current_projection_unchanged() {
    let tmp = tempdir().expect("temp dir");
    let sqlite_path = tmp.path().join("devql.sqlite");

    sqlite_exec_path_allow_create(&sqlite_path, devql_schema_sql_sqlite())
        .await
        .expect("core devql schema");
    init_sqlite_semantic_features_schema(&sqlite_path)
        .await
        .expect("semantic feature schema");
    init_sqlite_semantic_embeddings_schema(&sqlite_path)
        .await
        .expect("semantic embedding schema");
    sqlite_exec_path_allow_create(&sqlite_path, semantic_clones_sqlite_schema_sql())
        .await
        .expect("semantic clone schema");

    let conn = rusqlite::Connection::open(&sqlite_path).expect("open sqlite");
    conn.execute(
        "INSERT INTO symbol_clone_edges_current (
            repo_id, source_symbol_id, source_artefact_id, target_symbol_id, target_artefact_id,
            relation_kind, score, semantic_score, lexical_score, structural_score,
            clone_input_hash, explanation_json
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
        rusqlite::params![
            "repo-1",
            "source",
            "artefact-source",
            "target",
            "artefact-target",
            "similar_implementation",
            0.91_f64,
            0.9_f64,
            0.8_f64,
            0.7_f64,
            "input-hash",
            r#"{"reason":"stale"}"#,
        ],
    )
    .expect("insert stale current clone edge");
    drop(conn);

    let relational = RelationalStorage::local_only(sqlite_path.clone());
    let build = rebuild_symbol_clone_edges(&relational, "repo-1")
        .await
        .expect("wrapper rebuild");
    assert!(build.edges.is_empty());

    let conn = rusqlite::Connection::open(&sqlite_path).expect("open sqlite");
    let current_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM symbol_clone_edges_current WHERE repo_id = ?1",
            ["repo-1"],
            |row| row.get(0),
        )
        .expect("count current clone edges");
    assert_eq!(current_count, 1);
}

#[test]
fn current_projection_returns_none_when_multiple_code_setups_exist() {
    let chosen = choose_current_projection_embedding_state(&[
        embeddings::ActiveEmbeddingRepresentationState::new(
            embeddings::EmbeddingRepresentationKind::Code,
            embeddings::EmbeddingSetup::new(
                crate::host::inference::BITLOOPS_EMBEDDINGS_IPC_DRIVER,
                "bge-m3",
                3,
            ),
        ),
        embeddings::ActiveEmbeddingRepresentationState::new(
            embeddings::EmbeddingRepresentationKind::Code,
            embeddings::EmbeddingSetup::new(
                crate::host::inference::BITLOOPS_EMBEDDINGS_IPC_DRIVER,
                "bge-large-en-v1.5",
                3,
            ),
        ),
    ])
    .expect("choose current projection setup");

    assert!(chosen.is_none());
}

#[tokio::test]
async fn remote_shared_clone_edge_persistence_routes_historical_rows_off_local_sqlite() {
    let tmp = tempdir().expect("temp dir");
    let sqlite_path = tmp.path().join("devql.sqlite");
    let relational = RelationalStorage::primary_backend_for_tests(
        sqlite_path.clone(),
        RelationalPrimaryBackend::Postgres,
    );

    let err = replace_repo_symbol_clone_edges_for_projection(
        &relational,
        "repo-1",
        CloneProjection::Historical,
        &[sample_symbol_clone_edge_row()],
    )
    .await
    .expect_err("historical rows should route to remote shared authority");
    assert!(
        err.to_string()
            .contains("remote Postgres storage is not configured"),
        "expected remote routing error, got: {err:#}"
    );

    let conn = rusqlite::Connection::open(&sqlite_path).expect("open sqlite");
    let historical_tables: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'symbol_clone_edges'",
            [],
            |row| row.get(0),
        )
        .expect("count local historical clone-edge tables");
    let current_tables: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'symbol_clone_edges_current'",
            [],
            |row| row.get(0),
        )
        .expect("count local current clone-edge tables");
    assert_eq!(historical_tables, 0);
    assert_eq!(current_tables, 1);
}

#[tokio::test]
async fn remote_shared_clone_edge_persistence_keeps_current_rows_local() {
    let tmp = tempdir().expect("temp dir");
    let sqlite_path = tmp.path().join("devql.sqlite");
    let relational = RelationalStorage::primary_backend_for_tests(
        sqlite_path.clone(),
        RelationalPrimaryBackend::Postgres,
    );

    replace_repo_symbol_clone_edges_for_projection(
        &relational,
        "repo-1",
        CloneProjection::Current,
        &[sample_symbol_clone_edge_row()],
    )
    .await
    .expect("current rows should stay local");

    let conn = rusqlite::Connection::open(&sqlite_path).expect("open sqlite");
    let current_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM symbol_clone_edges_current WHERE repo_id = ?1",
            ["repo-1"],
            |row| row.get(0),
        )
        .expect("count current clone-edge rows");
    let historical_tables: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'symbol_clone_edges'",
            [],
            |row| row.get(0),
        )
        .expect("count local historical clone-edge tables");
    assert_eq!(current_count, 1);
    assert_eq!(historical_tables, 0);
}

#[tokio::test]
async fn local_only_clone_edge_persistence_keeps_historical_and_current_rows_local() {
    let tmp = tempdir().expect("temp dir");
    let sqlite_path = tmp.path().join("devql.sqlite");
    let relational = RelationalStorage::local_only(sqlite_path.clone());

    replace_repo_symbol_clone_edges_for_projection(
        &relational,
        "repo-1",
        CloneProjection::Historical,
        &[sample_symbol_clone_edge_row()],
    )
    .await
    .expect("historical rows should stay local in sqlite-only mode");
    replace_repo_symbol_clone_edges_for_projection(
        &relational,
        "repo-1",
        CloneProjection::Current,
        &[sample_symbol_clone_edge_row()],
    )
    .await
    .expect("current rows should stay local in sqlite-only mode");

    let conn = rusqlite::Connection::open(&sqlite_path).expect("open sqlite");
    let historical_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM symbol_clone_edges WHERE repo_id = ?1",
            ["repo-1"],
            |row| row.get(0),
        )
        .expect("count historical clone-edge rows");
    let current_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM symbol_clone_edges_current WHERE repo_id = ?1",
            ["repo-1"],
            |row| row.get(0),
        )
        .expect("count current clone-edge rows");
    assert_eq!(historical_count, 1);
    assert_eq!(current_count, 1);
}

#[test]
fn clone_edge_sql_uses_projection_owned_dialect() {
    let row = sample_symbol_clone_edge_row();
    let historical_sql = build_persist_symbol_clone_edge_statement(
        RelationalDialect::Postgres,
        CloneProjection::Historical,
        std::slice::from_ref(&row),
    );
    let current_sql = build_persist_symbol_clone_edge_statement(
        RelationalDialect::Sqlite,
        CloneProjection::Current,
        &[row],
    );

    assert!(historical_sql.contains("now()"));
    assert!(historical_sql.contains("::jsonb"));
    assert!(current_sql.contains("datetime('now')"));
    assert!(!current_sql.contains("::jsonb"));
}
