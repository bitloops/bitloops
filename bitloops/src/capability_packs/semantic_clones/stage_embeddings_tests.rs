use super::*;
use crate::host::devql::{sqlite_exec_path_allow_create, sqlite_query_rows_path};
use crate::host::inference::{EmbeddingInputType as HostEmbeddingInputType, EmbeddingService};
use serde_json::json;
use tempfile::tempdir;

const TEST_EMBEDDINGS_DRIVER: &str = crate::host::inference::BITLOOPS_EMBEDDINGS_IPC_DRIVER;
const TEST_EMBEDDINGS_MODEL: &str = "bge-m3";
const ALT_TEST_EMBEDDINGS_MODEL: &str = "bge-large-en-v1.5";

struct TestEmbeddingProvider;

impl EmbeddingService for TestEmbeddingProvider {
    fn provider_name(&self) -> &str {
        TEST_EMBEDDINGS_DRIVER
    }

    fn model_name(&self) -> &str {
        TEST_EMBEDDINGS_MODEL
    }

    fn output_dimension(&self) -> Option<usize> {
        Some(3)
    }

    fn cache_key(&self) -> String {
        format!("provider={TEST_EMBEDDINGS_DRIVER}:model={TEST_EMBEDDINGS_MODEL}")
    }

    fn embed(&self, input: &str, _input_type: HostEmbeddingInputType) -> Result<Vec<f32>> {
        Ok(vec![input.len() as f32, 0.5, 0.25])
    }
}

fn test_setup_fingerprint(provider: &str, model: &str, dimension: usize) -> String {
    embeddings::EmbeddingSetup::new(provider, model, dimension).setup_fingerprint
}

async fn sqlite_relational_with_schema(sql: &str) -> RelationalStorage {
    let temp = tempdir().expect("temp dir");
    let db_path = temp.path().join("semantic-embeddings.sqlite");
    sqlite_exec_path_allow_create(&db_path, sql)
        .await
        .expect("create sqlite schema");
    std::mem::forget(temp);
    RelationalStorage::local_only(db_path)
}

async fn sqlite_relational_with_embedding_state_schema() -> RelationalStorage {
    sqlite_relational_with_schema(&format!(
        "{}\nCREATE TABLE symbol_semantics (artefact_id TEXT PRIMARY KEY, summary TEXT);
CREATE TABLE symbol_features (artefact_id TEXT PRIMARY KEY);
CREATE TABLE artefacts_current (
    repo_id TEXT NOT NULL,
    artefact_id TEXT PRIMARY KEY,
    path TEXT,
    start_line INTEGER,
    symbol_id TEXT,
    canonical_kind TEXT,
    language_kind TEXT
);
CREATE TABLE current_file_state (
    repo_id TEXT NOT NULL,
    path TEXT NOT NULL,
    analysis_mode TEXT NOT NULL,
    PRIMARY KEY (repo_id, path)
);",
        schema::semantic_embeddings_sqlite_schema_sql()
    ))
    .await
}

async fn insert_fully_indexed_current_artefact(
    relational: &RelationalStorage,
    artefact_id: &str,
    representation_kind: embeddings::EmbeddingRepresentationKind,
    provider: &str,
    model: &str,
    dimension: usize,
) {
    insert_fully_indexed_current_artefact_with_stored_representation(
        relational,
        artefact_id,
        &representation_kind.to_string(),
        provider,
        model,
        dimension,
    )
    .await;
}

async fn insert_fully_indexed_current_artefact_with_stored_representation(
    relational: &RelationalStorage,
    artefact_id: &str,
    stored_representation_kind: &str,
    provider: &str,
    model: &str,
    dimension: usize,
) {
    let setup_fingerprint = test_setup_fingerprint(provider, model, dimension);
    relational
        .exec(&format!(
            "INSERT INTO artefacts_current (repo_id, artefact_id, path, start_line, symbol_id, canonical_kind, language_kind)
             VALUES ('repo-1', '{artefact_id}', 'src/a.ts', 1, 'sym-{artefact_id}', 'function', 'function')",
            artefact_id = esc_pg(artefact_id),
        ))
        .await
        .expect("insert current artefact");
    relational
        .exec(
            "INSERT INTO current_file_state (repo_id, path, analysis_mode)
             VALUES ('repo-1', 'src/a.ts', 'code')
             ON CONFLICT (repo_id, path) DO UPDATE SET analysis_mode = excluded.analysis_mode",
        )
        .await
        .expect("insert current file state");
    relational
        .exec(&format!(
            "INSERT INTO symbol_semantics (artefact_id, summary)
             VALUES ('{artefact_id}', 'Function summary.')",
            artefact_id = esc_pg(artefact_id),
        ))
        .await
        .expect("insert semantic row");
    relational
        .exec(&format!(
            "INSERT INTO symbol_features (artefact_id)
             VALUES ('{artefact_id}')",
            artefact_id = esc_pg(artefact_id),
        ))
        .await
        .expect("insert feature row");
    relational
        .exec(&format!(
            "INSERT INTO symbol_embeddings (artefact_id, repo_id, blob_sha, representation_kind, setup_fingerprint, provider, model, dimension, embedding_input_hash, embedding)
             VALUES ('{artefact_id}', 'repo-1', 'blob-1', '{representation_kind}', '{setup_fingerprint}', '{provider}', '{model}', {dimension}, 'hash-{artefact_id}', '[0.1,0.2,0.3]')",
            artefact_id = esc_pg(artefact_id),
            representation_kind = esc_pg(stored_representation_kind),
            setup_fingerprint = esc_pg(&setup_fingerprint),
            provider = esc_pg(provider),
            model = esc_pg(model),
            dimension = dimension,
        ))
        .await
        .expect("insert embedding row");
}

#[test]
fn semantic_embedding_schema_includes_vector_table() {
    let schema = schema::semantic_embeddings_postgres_schema_sql();
    assert!(schema.contains("CREATE EXTENSION IF NOT EXISTS vector"));
    assert!(schema.contains("CREATE TABLE IF NOT EXISTS symbol_embeddings"));
    assert!(schema.contains("embedding vector"));
}

#[test]
fn semantic_embedding_sqlite_schema_uses_text_storage() {
    let schema = schema::semantic_embeddings_sqlite_schema_sql();
    assert!(schema.contains("CREATE TABLE IF NOT EXISTS symbol_embeddings"));
    assert!(schema.contains("embedding TEXT NOT NULL"));
    assert!(schema.contains("generated_at DATETIME DEFAULT CURRENT_TIMESTAMP"));
}

#[test]
fn semantic_embedding_state_parser_defaults_and_reads_hash() {
    let empty = parse_symbol_embedding_index_state_rows(&[]);
    assert_eq!(empty, embeddings::SymbolEmbeddingIndexState::default());

    let rows = vec![json!({ "embedding_hash": "hash-1" })];
    let parsed = parse_symbol_embedding_index_state_rows(&rows);
    assert_eq!(parsed.embedding_hash.as_deref(), Some("hash-1"));
}

#[test]
fn semantic_embedding_postgres_persist_sql_contains_vector_literal() {
    let sql = build_postgres_symbol_embedding_persist_sql(&embeddings::SymbolEmbeddingRow {
        artefact_id: "artefact-1".to_string(),
        repo_id: "repo-1".to_string(),
        blob_sha: "blob-1".to_string(),
        representation_kind: embeddings::EmbeddingRepresentationKind::Code,
        setup_fingerprint: test_setup_fingerprint("voyage", "voyage-code-3", 3),
        provider: "voyage".to_string(),
        model: "voyage-code-3".to_string(),
        dimension: 3,
        embedding_input_hash: "hash-1".to_string(),
        embedding: vec![0.1, -0.2, 0.3],
    })
    .expect("persist sql");
    assert!(sql.contains("INSERT INTO symbol_embeddings"));
    assert!(sql.contains("'[0.1,-0.2,0.3]'::vector"));
}

#[test]
fn semantic_embedding_sqlite_persist_sql_contains_json_literal() {
    let sql = build_sqlite_symbol_embedding_persist_sql(&embeddings::SymbolEmbeddingRow {
        artefact_id: "artefact-1".to_string(),
        repo_id: "repo-1".to_string(),
        blob_sha: "blob-1".to_string(),
        representation_kind: embeddings::EmbeddingRepresentationKind::Code,
        setup_fingerprint: test_setup_fingerprint(
            "local",
            "jinaai/jina-embeddings-v2-base-code",
            3,
        ),
        provider: "local".to_string(),
        model: "jinaai/jina-embeddings-v2-base-code".to_string(),
        dimension: 3,
        embedding_input_hash: "hash-1".to_string(),
        embedding: vec![0.1, -0.2, 0.3],
    })
    .expect("persist sql");
    assert!(sql.contains("INSERT INTO symbol_embeddings"));
    assert!(sql.contains("'[0.1,-0.2,0.3]'"));
    assert!(!sql.contains("::vector"));
    assert!(sql.contains("generated_at = CURRENT_TIMESTAMP"));
}

#[test]
fn semantic_embedding_vector_sql_contains_vector_cast() {
    let sql = sql_vector_string(&[0.1, -0.2, 0.3]).expect("vector sql");
    assert_eq!(sql, "'[0.1,-0.2,0.3]'::vector");
}

#[test]
fn semantic_embedding_json_sql_contains_json_literal() {
    let sql = sql_json_string(&[0.1, -0.2, 0.3]).expect("json sql");
    assert_eq!(sql, "[0.1,-0.2,0.3]");
}

#[test]
fn semantic_embedding_vector_sql_rejects_empty_or_non_finite_vectors() {
    let empty_err = sql_vector_string(&[]).expect_err("empty vectors must fail");
    assert!(empty_err.to_string().contains("empty embedding vector"));

    let invalid_err =
        sql_vector_string(&[0.1, f32::NAN]).expect_err("non-finite vectors must fail");
    assert!(invalid_err.to_string().contains("non-finite values"));
}

#[test]
fn semantic_embedding_json_sql_rejects_empty_or_non_finite_vectors() {
    let empty_err = sql_json_string(&[]).expect_err("empty vectors must fail");
    assert!(empty_err.to_string().contains("empty embedding vector"));

    let invalid_err = sql_json_string(&[0.1, f32::NAN]).expect_err("non-finite vectors must fail");
    assert!(invalid_err.to_string().contains("non-finite values"));
}

#[test]
fn semantic_embedding_index_state_sql_filters_by_artefact_id() {
    let sql = build_symbol_embedding_index_state_sql(
        "artefact-'1",
        "symbol_embeddings",
        embeddings::EmbeddingRepresentationKind::Code,
        "provider=voyage|model=voyage-code-3|dimension=1024",
    );
    assert!(sql.contains("FROM symbol_embeddings"));
    assert!(sql.contains("WHERE artefact_id = 'artefact-''1'"));
    assert!(sql.contains("representation_kind = 'code'"));
    assert!(
        sql.contains("setup_fingerprint = 'provider=voyage|model=voyage-code-3|dimension=1024'")
    );
}

#[test]
fn semantic_embedding_summary_lookup_sql_uses_all_ids() {
    let sql = build_semantic_summary_lookup_sql(
        &["artefact-1".to_string(), "artefact-2".to_string()],
        "symbol_semantics_current",
    );
    assert!(sql.contains("FROM symbol_semantics_current"));
    assert!(sql.contains("'artefact-1'"));
    assert!(sql.contains("'artefact-2'"));
}

#[tokio::test]
async fn semantic_embedding_loads_index_state_from_relational_storage() {
    let setup_fingerprint = test_setup_fingerprint("voyage", "voyage-code-3", 1024);
    let relational = sqlite_relational_with_schema(&format!(
        "{schema}
        INSERT INTO symbol_embeddings (
            artefact_id, repo_id, blob_sha, representation_kind, setup_fingerprint, provider, model, dimension, embedding_input_hash, embedding
        ) VALUES (
            'artefact-1', 'repo-1', 'blob-1', 'code', '{setup_fingerprint}', 'voyage', 'voyage-code-3', 1024, 'hash-1', '[0.1,0.2,0.3]'
        );",
        schema = schema::semantic_embeddings_sqlite_schema_sql(),
        setup_fingerprint = setup_fingerprint,
    ))
    .await;

    let state = load_symbol_embedding_index_state(
        &relational,
        "artefact-1",
        embeddings::EmbeddingRepresentationKind::Code,
        &setup_fingerprint,
    )
    .await
    .expect("load embedding state");

    assert_eq!(state.embedding_hash.as_deref(), Some("hash-1"));
}

#[tokio::test]
async fn current_embedding_upsert_reuses_matching_rows_and_keeps_summary_variant() {
    let relational = sqlite_relational_with_schema(&format!(
        "{}\nCREATE TABLE symbol_semantics_current (
            artefact_id TEXT PRIMARY KEY,
            repo_id TEXT NOT NULL,
            path TEXT NOT NULL,
            content_id TEXT NOT NULL,
            symbol_id TEXT,
            semantic_features_input_hash TEXT NOT NULL,
            docstring_summary TEXT,
            llm_summary TEXT,
            template_summary TEXT NOT NULL,
            summary TEXT NOT NULL,
            confidence REAL NOT NULL,
            source_model TEXT
        );",
        schema::semantic_embeddings_sqlite_schema_sql()
    ))
    .await;
    relational
        .exec(
            "INSERT INTO symbol_semantics_current (
                artefact_id, repo_id, path, content_id, symbol_id, semantic_features_input_hash,
                docstring_summary, llm_summary, template_summary, summary, confidence, source_model
            ) VALUES
                ('artefact-1', 'repo-1', 'src/a.ts', 'blob-1', 'sym-1', 'semantic-hash-1', NULL, 'Loads invoice data.', 'Function load invoice.', 'Loads invoice data.', 0.9, 'test-model'),
                ('artefact-2', 'repo-1', 'src/a.ts', 'blob-1', 'sym-2', 'semantic-hash-2', NULL, NULL, 'Function save invoice.', 'Function save invoice.', 0.9, NULL)",
        )
        .await
        .expect("insert current semantics");

    let inputs = vec![
        semantic::SemanticFeatureInput {
            artefact_id: "artefact-1".to_string(),
            symbol_id: Some("sym-1".to_string()),
            repo_id: "repo-1".to_string(),
            blob_sha: "blob-1".to_string(),
            path: "src/a.ts".to_string(),
            language: "typescript".to_string(),
            canonical_kind: "function".to_string(),
            language_kind: "function_declaration".to_string(),
            symbol_fqn: "src/a.ts::loadInvoice".to_string(),
            name: "loadInvoice".to_string(),
            signature: Some("function loadInvoice(id: string)".to_string()),
            modifiers: Vec::new(),
            body: "return loadInvoiceData(id);".to_string(),
            docstring: None,
            parent_kind: None,
            dependency_signals: vec!["loadInvoiceData".to_string()],
            content_hash: Some("blob-1".to_string()),
        },
        semantic::SemanticFeatureInput {
            artefact_id: "artefact-2".to_string(),
            symbol_id: Some("sym-2".to_string()),
            repo_id: "repo-1".to_string(),
            blob_sha: "blob-1".to_string(),
            path: "src/a.ts".to_string(),
            language: "typescript".to_string(),
            canonical_kind: "function".to_string(),
            language_kind: "function_declaration".to_string(),
            symbol_fqn: "src/a.ts::saveInvoice".to_string(),
            name: "saveInvoice".to_string(),
            signature: Some("function saveInvoice(id: string)".to_string()),
            modifiers: Vec::new(),
            body: "return persistInvoice(id);".to_string(),
            docstring: None,
            parent_kind: None,
            dependency_signals: vec!["persistInvoice".to_string()],
            content_hash: Some("blob-1".to_string()),
        },
    ];
    let provider: Arc<dyn EmbeddingService> = Arc::new(TestEmbeddingProvider);

    let code_first = upsert_current_symbol_embedding_rows(
        &relational,
        "src/a.ts",
        "blob-1",
        &inputs,
        embeddings::EmbeddingRepresentationKind::Code,
        Arc::clone(&provider),
    )
    .await
    .expect("upsert code current embeddings");
    let code_second = upsert_current_symbol_embedding_rows(
        &relational,
        "src/a.ts",
        "blob-1",
        &inputs,
        embeddings::EmbeddingRepresentationKind::Code,
        Arc::clone(&provider),
    )
    .await
    .expect("reuse code current embeddings");
    let summary = upsert_current_symbol_embedding_rows(
        &relational,
        "src/a.ts",
        "blob-1",
        &inputs,
        embeddings::EmbeddingRepresentationKind::Summary,
        provider,
    )
    .await
    .expect("upsert summary current embeddings");

    assert_eq!(code_first.upserted, 2);
    assert_eq!(code_second.skipped, 2);
    assert_eq!(summary.eligible, 2);
    assert_eq!(summary.upserted, 2);

    let rows = relational
        .query_rows(
            "SELECT artefact_id, representation_kind
             FROM symbol_embeddings_current
             WHERE repo_id = 'repo-1'
             ORDER BY artefact_id, representation_kind",
        )
        .await
        .expect("read current embedding rows");
    let rendered = rows
        .into_iter()
        .map(|row| {
            (
                row["artefact_id"].as_str().unwrap_or_default().to_string(),
                row["representation_kind"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string(),
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(
        rendered,
        vec![
            ("artefact-1".to_string(), "code".to_string()),
            ("artefact-1".to_string(), "summary".to_string()),
            ("artefact-2".to_string(), "code".to_string()),
            ("artefact-2".to_string(), "summary".to_string()),
        ]
    );
}

#[tokio::test]
async fn historical_embedding_upsert_persists_code_and_summary_variants() {
    let relational = sqlite_relational_with_schema(&format!(
        "{}\nCREATE TABLE symbol_semantics (
                artefact_id TEXT PRIMARY KEY,
                repo_id TEXT NOT NULL,
                blob_sha TEXT NOT NULL,
                symbol_id TEXT,
                semantic_features_input_hash TEXT NOT NULL,
                docstring_summary TEXT,
                llm_summary TEXT,
                template_summary TEXT NOT NULL,
                summary TEXT NOT NULL,
                confidence REAL NOT NULL,
                source_model TEXT
            );",
        schema::semantic_embeddings_sqlite_schema_sql()
    ))
    .await;
    relational
        .exec(
            "INSERT INTO symbol_semantics (
                artefact_id, repo_id, blob_sha, symbol_id, semantic_features_input_hash,
                docstring_summary, llm_summary, template_summary, summary, confidence, source_model
            ) VALUES
                ('artefact-1', 'repo-1', 'blob-1', 'sym-1', 'semantic-hash-1', NULL, 'Loads invoice data.', 'Function load invoice.', 'Loads invoice data.', 0.9, 'test-model'),
                ('artefact-2', 'repo-1', 'blob-1', 'sym-2', 'semantic-hash-2', NULL, NULL, 'Function save invoice.', 'Function save invoice.', 0.9, NULL)",
        )
        .await
        .expect("insert historical semantics");

    let inputs = vec![
        semantic::SemanticFeatureInput {
            artefact_id: "artefact-1".to_string(),
            symbol_id: Some("sym-1".to_string()),
            repo_id: "repo-1".to_string(),
            blob_sha: "blob-1".to_string(),
            path: "src/a.ts".to_string(),
            language: "typescript".to_string(),
            canonical_kind: "function".to_string(),
            language_kind: "function_declaration".to_string(),
            symbol_fqn: "src/a.ts::loadInvoice".to_string(),
            name: "loadInvoice".to_string(),
            signature: Some("function loadInvoice(id: string)".to_string()),
            modifiers: Vec::new(),
            body: "return loadInvoiceData(id);".to_string(),
            docstring: None,
            parent_kind: None,
            dependency_signals: vec!["loadInvoiceData".to_string()],
            content_hash: Some("blob-1".to_string()),
        },
        semantic::SemanticFeatureInput {
            artefact_id: "artefact-2".to_string(),
            symbol_id: Some("sym-2".to_string()),
            repo_id: "repo-1".to_string(),
            blob_sha: "blob-1".to_string(),
            path: "src/a.ts".to_string(),
            language: "typescript".to_string(),
            canonical_kind: "function".to_string(),
            language_kind: "function_declaration".to_string(),
            symbol_fqn: "src/a.ts::saveInvoice".to_string(),
            name: "saveInvoice".to_string(),
            signature: Some("function saveInvoice(id: string)".to_string()),
            modifiers: Vec::new(),
            body: "return persistInvoice(id);".to_string(),
            docstring: None,
            parent_kind: None,
            dependency_signals: vec!["persistInvoice".to_string()],
            content_hash: Some("blob-1".to_string()),
        },
    ];
    let provider: Arc<dyn EmbeddingService> = Arc::new(TestEmbeddingProvider);

    let code = upsert_symbol_embedding_rows(
        &relational,
        &inputs,
        embeddings::EmbeddingRepresentationKind::Code,
        Arc::clone(&provider),
    )
    .await
    .expect("upsert historical code embeddings");
    let summary = upsert_symbol_embedding_rows(
        &relational,
        &inputs,
        embeddings::EmbeddingRepresentationKind::Summary,
        provider,
    )
    .await
    .expect("upsert historical summary embeddings");

    assert_eq!(code.upserted, 2);
    assert_eq!(summary.upserted, 2);

    let rows = relational
        .query_rows(
            "SELECT artefact_id, representation_kind
             FROM symbol_embeddings
             WHERE repo_id = 'repo-1'
             ORDER BY artefact_id, representation_kind",
        )
        .await
        .expect("read historical embedding rows");
    let rendered = rows
        .into_iter()
        .map(|row| {
            (
                row["artefact_id"].as_str().unwrap_or_default().to_string(),
                row["representation_kind"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string(),
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(
        rendered,
        vec![
            ("artefact-1".to_string(), "code".to_string()),
            ("artefact-1".to_string(), "summary".to_string()),
            ("artefact-2".to_string(), "code".to_string()),
            ("artefact-2".to_string(), "summary".to_string()),
        ]
    );
}

#[tokio::test]
async fn semantic_embedding_loads_summary_map_from_relational_storage() {
    let relational = sqlite_relational_with_schema(
        "CREATE TABLE symbol_semantics (
            artefact_id TEXT PRIMARY KEY,
            docstring_summary TEXT,
            llm_summary TEXT,
            template_summary TEXT,
            summary TEXT,
            source_model TEXT
        );
        INSERT INTO symbol_semantics (
            artefact_id, docstring_summary, llm_summary, template_summary, summary, source_model
        ) VALUES
            ('artefact-1', NULL, NULL, 'summarizes function 1', 'summarizes function 1', NULL),
            ('artefact-2', NULL, NULL, 'template summary 2', '', NULL),
            ('artefact-3', 'summarizes function 3', NULL, 'template summary 3', 'template summary 3 summarizes function 3', NULL);",
    )
    .await;

    let summary_map = load_semantic_summary_map(
        &relational,
        &[
            "artefact-1".to_string(),
            "artefact-2".to_string(),
            "artefact-3".to_string(),
        ],
        embeddings::EmbeddingRepresentationKind::Code,
    )
    .await
    .expect("load summary map");

    assert_eq!(
        summary_map.get("artefact-1").map(String::as_str),
        Some("summarizes function 1")
    );
    assert_eq!(
        summary_map.get("artefact-3").map(String::as_str),
        Some("template summary 3. summarizes function 3.")
    );
    assert_eq!(
        summary_map.get("artefact-2").map(String::as_str),
        Some("template summary 2")
    );
}

#[tokio::test]
async fn semantic_embedding_schema_ensure_creates_sqlite_table() {
    let temp = tempdir().expect("temp dir");
    let db_path = temp.path().join("semantic-embeddings.sqlite");
    let relational = RelationalStorage::local_only(db_path.clone());

    ensure_semantic_embeddings_schema(&relational)
        .await
        .expect("ensure sqlite embedding schema");

    let rows = sqlite_query_rows_path(
        &db_path,
        "SELECT name FROM sqlite_master WHERE type = 'table' AND name = 'symbol_embeddings'",
    )
    .await
    .expect("query sqlite master");

    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].get("name"), Some(&json!("symbol_embeddings")));
}

#[tokio::test]
async fn semantic_embedding_sync_action_adopts_existing_single_setup() {
    let relational = sqlite_relational_with_embedding_state_schema().await;
    insert_fully_indexed_current_artefact(
        &relational,
        "artefact-1",
        embeddings::EmbeddingRepresentationKind::Code,
        TEST_EMBEDDINGS_DRIVER,
        TEST_EMBEDDINGS_MODEL,
        3,
    )
    .await;

    let action = determine_repo_embedding_sync_action(
        &relational,
        "repo-1",
        embeddings::EmbeddingRepresentationKind::Code,
        &embeddings::EmbeddingSetup::new(TEST_EMBEDDINGS_DRIVER, TEST_EMBEDDINGS_MODEL, 3),
    )
    .await
    .expect("sync action");

    assert_eq!(action, RepoEmbeddingSyncAction::AdoptExisting);
}

#[tokio::test]
async fn semantic_embedding_sync_action_refreshes_when_current_repo_coverage_is_partial() {
    let relational = sqlite_relational_with_embedding_state_schema().await;
    insert_fully_indexed_current_artefact(
        &relational,
        "artefact-1",
        embeddings::EmbeddingRepresentationKind::Code,
        TEST_EMBEDDINGS_DRIVER,
        TEST_EMBEDDINGS_MODEL,
        3,
    )
    .await;
    relational
        .exec(
            "INSERT INTO artefacts_current (repo_id, artefact_id, path, start_line, symbol_id, canonical_kind, language_kind)
             VALUES ('repo-1', 'artefact-2', 'src/b.ts', 2, 'sym-2', 'function', 'function')",
        )
        .await
        .expect("insert uncovered current artefact");
    relational
        .exec(
            "INSERT INTO current_file_state (repo_id, path, analysis_mode)
             VALUES ('repo-1', 'src/b.ts', 'code')",
        )
        .await
        .expect("insert uncovered current file state");

    let action = determine_repo_embedding_sync_action(
        &relational,
        "repo-1",
        embeddings::EmbeddingRepresentationKind::Code,
        &embeddings::EmbeddingSetup::new(TEST_EMBEDDINGS_DRIVER, TEST_EMBEDDINGS_MODEL, 3),
    )
    .await
    .expect("sync action");

    assert_eq!(action, RepoEmbeddingSyncAction::RefreshCurrentRepo);
}

#[tokio::test]
async fn semantic_embedding_sync_action_adopts_existing_when_legacy_representation_exists() {
    let relational = sqlite_relational_with_embedding_state_schema().await;
    insert_fully_indexed_current_artefact_with_stored_representation(
        &relational,
        "artefact-1",
        "enriched",
        TEST_EMBEDDINGS_DRIVER,
        TEST_EMBEDDINGS_MODEL,
        3,
    )
    .await;

    let action = determine_repo_embedding_sync_action(
        &relational,
        "repo-1",
        embeddings::EmbeddingRepresentationKind::Code,
        &embeddings::EmbeddingSetup::new(TEST_EMBEDDINGS_DRIVER, TEST_EMBEDDINGS_MODEL, 3),
    )
    .await
    .expect("sync action");

    assert_eq!(action, RepoEmbeddingSyncAction::AdoptExisting);
}

#[tokio::test]
async fn semantic_embedding_sync_action_refreshes_when_active_setup_changes() {
    let relational = sqlite_relational_with_embedding_state_schema().await;
    persist_active_embedding_setup(
        &relational,
        "repo-1",
        &embeddings::ActiveEmbeddingRepresentationState::new(
            embeddings::EmbeddingRepresentationKind::Code,
            embeddings::EmbeddingSetup::new(TEST_EMBEDDINGS_DRIVER, TEST_EMBEDDINGS_MODEL, 3),
        ),
    )
    .await
    .expect("persist active setup");

    let action = determine_repo_embedding_sync_action(
        &relational,
        "repo-1",
        embeddings::EmbeddingRepresentationKind::Code,
        &embeddings::EmbeddingSetup::new(TEST_EMBEDDINGS_DRIVER, ALT_TEST_EMBEDDINGS_MODEL, 1024),
    )
    .await
    .expect("sync action");

    assert_eq!(action, RepoEmbeddingSyncAction::RefreshCurrentRepo);
}
