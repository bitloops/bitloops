use tempfile::tempdir;

use crate::capability_packs::semantic_clones::embeddings;
use crate::capability_packs::semantic_clones::schema::{
    semantic_clones_postgres_schema_sql, semantic_clones_sqlite_schema_sql,
};
use crate::capability_packs::semantic_clones::scoring::CloneScoringOptions;
use crate::capability_packs::semantic_clones::{
    init_sqlite_semantic_embeddings_schema, init_sqlite_semantic_features_schema,
};
use crate::host::devql::{
    RelationalStorage, devql_schema_sql_sqlite, sqlite_exec_path_allow_create,
};

use super::orchestrator::{rebuild_symbol_clone_edges, rebuild_symbol_clone_edges_with_options};
use super::queries::build_symbol_clone_candidate_lookup_sql;
use super::schema::CloneProjection;
use super::state::choose_current_projection_embedding_state;

#[test]
fn semantic_clone_schema_includes_clone_edge_table() {
    let pg = semantic_clones_postgres_schema_sql();
    let sqlite = semantic_clones_sqlite_schema_sql();

    assert!(pg.contains("CREATE TABLE IF NOT EXISTS symbol_clone_edges"));
    assert!(sqlite.contains("CREATE TABLE IF NOT EXISTS symbol_clone_edges"));
    assert!(pg.contains("CREATE TABLE IF NOT EXISTS symbol_clone_edges_current"));
    assert!(sqlite.contains("CREATE TABLE IF NOT EXISTS symbol_clone_edges_current"));
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
async fn rebuild_wrapper_syncs_empty_historical_result_into_current_projection() {
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
    assert_eq!(current_count, 0);
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
