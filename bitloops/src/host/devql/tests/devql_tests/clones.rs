use super::*;

#[test]
fn parse_devql_clones_stage_basic() {
    let parsed = parse_devql_query(
        r#"repo("temp2")->artefacts(kind:"function")->clones(relation_kind:"similar_implementation",min_score:0.6)->limit(5)"#,
    )
    .unwrap();

    assert!(parsed.has_clones_stage);
    assert_eq!(
        parsed.clones.relation_kind.as_deref(),
        Some("similar_implementation")
    );
    assert_eq!(parsed.clones.min_score, Some(0.6));
}

#[tokio::test]
async fn execute_devql_query_rejects_clones_without_artefacts_stage() {
    let cfg = test_cfg();
    let events_cfg = default_events_cfg();
    let parsed = parse_devql_query(r#"repo("temp2")->clones()->limit(5)"#).unwrap();
    let err = execute_devql_query(&cfg, &parsed, &events_cfg, None)
        .await
        .unwrap_err();

    assert!(
        err.to_string()
            .contains("clones() requires an artefacts() stage")
    );
}

#[tokio::test]
async fn execute_devql_query_rejects_clones_with_asof() {
    let cfg = test_cfg();
    let events_cfg = default_events_cfg();
    let temp = tempdir().expect("tempdir");
    let parsed = parse_devql_query(
        r#"repo("temp2")->asOf(commit:"abc123")->artefacts(kind:"function")->clones()->limit(5)"#,
    )
    .unwrap();
    let relational = sqlite_relational_store_with_schema(&temp.path().join("db.sqlite")).await;
    let err = execute_devql_query(&cfg, &parsed, &events_cfg, Some(&relational))
        .await
        .unwrap_err();

    assert!(
        err.to_string()
            .contains("clones() does not yet support asOf")
    );
}

#[tokio::test]
async fn sqlite_symbol_clone_edges_table_exists_after_semantic_clones_ddl() {
    let temp = tempdir().expect("temp dir");
    let db_path = temp.path().join("devql.sqlite");

    init_sqlite_schema(&db_path)
        .await
        .expect("initialise sqlite relational schema");
    apply_symbol_clone_edges_sqlite_schema(&db_path);

    let conn = rusqlite::Connection::open(&db_path).expect("open sqlite db");
    let mut stmt = conn
        .prepare(
            "SELECT name FROM sqlite_master WHERE type = 'table' AND name = 'symbol_clone_edges'",
        )
        .expect("prepare sqlite master query");
    let table_name: String = stmt
        .query_row([], |row| row.get(0))
        .expect("symbol_clone_edges table");

    assert_eq!(table_name, "symbol_clone_edges");
}

#[tokio::test]
async fn execute_relational_pipeline_reads_clones_from_sqlite_relational_store() {
    let cfg = test_cfg();
    let events_cfg = default_events_cfg();
    let temp = tempdir().expect("tempdir");
    let sqlite_path = temp.path().join("relational.sqlite");
    let relational = sqlite_relational_store_with_schema(&sqlite_path).await;

    let conn = rusqlite::Connection::open(&sqlite_path).expect("open sqlite");
    let repo_id = cfg.repo.repo_id.as_str();
    conn.execute(
        "INSERT INTO artefacts (artefact_id, symbol_id, repo_id, blob_sha, path, language, canonical_kind, language_kind, symbol_fqn, parent_artefact_id, start_line, end_line, start_byte, end_byte, signature, modifiers, content_hash)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, NULL, ?10, ?11, ?12, ?13, ?14, '[]', ?15)",
        rusqlite::params![
            "artefact::invoice_pdf",
            "sym::invoice_pdf",
            repo_id,
            "blob-1",
            "src/pdf.ts",
            "typescript",
            "function",
            "function_declaration",
            "src/pdf.ts::createInvoicePdf",
            1,
            12,
            0,
            120,
            "function createInvoicePdf(orderId: string, locale: string)",
            "hash-1",
        ],
    )
    .expect("insert artefact history source");
    conn.execute(
        "INSERT INTO artefacts (artefact_id, symbol_id, repo_id, blob_sha, path, language, canonical_kind, language_kind, symbol_fqn, parent_artefact_id, start_line, end_line, start_byte, end_byte, signature, modifiers, content_hash)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, NULL, ?10, ?11, ?12, ?13, ?14, '[]', ?15)",
        rusqlite::params![
            "artefact::invoice_doc",
            "sym::invoice_doc",
            repo_id,
            "blob-2",
            "src/render.ts",
            "typescript",
            "function",
            "function_declaration",
            "src/render.ts::renderInvoiceDocument",
            1,
            12,
            0,
            120,
            "function renderInvoiceDocument(orderId: string, locale: string)",
            "hash-2",
        ],
    )
    .expect("insert artefact history target");

    conn.execute(
        "INSERT INTO artefacts_current (
            repo_id, symbol_id, artefact_id, commit_sha, blob_sha, path, language,
            canonical_kind, language_kind, symbol_fqn, start_line, end_line, start_byte,
            end_byte, signature, modifiers, content_hash
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, '[]', ?16)",
        rusqlite::params![
            repo_id,
            "sym::invoice_pdf",
            "artefact::invoice_pdf",
            "commit-1",
            "blob-1",
            "src/pdf.ts",
            "typescript",
            "function",
            "function_declaration",
            "src/pdf.ts::createInvoicePdf",
            1,
            12,
            0,
            120,
            "function createInvoicePdf(orderId: string, locale: string)",
            "hash-1",
        ],
    )
    .expect("insert source current artefact");
    conn.execute(
        "INSERT INTO artefacts_current (
            repo_id, symbol_id, artefact_id, commit_sha, blob_sha, path, language,
            canonical_kind, language_kind, symbol_fqn, start_line, end_line, start_byte,
            end_byte, signature, modifiers, content_hash
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, '[]', ?16)",
        rusqlite::params![
            repo_id,
            "sym::invoice_doc",
            "artefact::invoice_doc",
            "commit-1",
            "blob-2",
            "src/render.ts",
            "typescript",
            "function",
            "function_declaration",
            "src/render.ts::renderInvoiceDocument",
            1,
            12,
            0,
            120,
            "function renderInvoiceDocument(orderId: string, locale: string)",
            "hash-2",
        ],
    )
    .expect("insert target current artefact");

    conn.execute(
        "INSERT INTO symbol_semantics (artefact_id, repo_id, blob_sha, semantic_features_input_hash, template_summary, summary, confidence)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        rusqlite::params![
            "artefact::invoice_pdf",
            repo_id,
            "blob-1",
            "semantic-hash-1",
            "Function create invoice pdf.",
            "Function create invoice pdf. Generates an invoice PDF for an order.",
            0.9_f64,
        ],
    )
    .expect("insert source semantics");
    conn.execute(
        "INSERT INTO symbol_semantics (artefact_id, repo_id, blob_sha, semantic_features_input_hash, template_summary, summary, confidence)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        rusqlite::params![
            "artefact::invoice_doc",
            repo_id,
            "blob-2",
            "semantic-hash-2",
            "Function render invoice document.",
            "Function render invoice document. Renders the invoice document for an order.",
            0.9_f64,
        ],
    )
    .expect("insert target semantics");

    conn.execute(
        "INSERT INTO symbol_features (artefact_id, repo_id, blob_sha, semantic_features_input_hash, normalized_name, normalized_signature, identifier_tokens, normalized_body_tokens, parent_kind, context_tokens)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        rusqlite::params![
            "artefact::invoice_pdf",
            repo_id,
            "blob-1",
            "semantic-hash-1",
            "create_invoice_pdf",
            "function createInvoicePdf(orderId: string, locale: string)",
            "[\"create\",\"invoice\",\"pdf\",\"order\",\"locale\"]",
            "[\"generate\",\"invoice\",\"pdf\",\"order\",\"locale\"]",
            "module",
            "[\"src\",\"pdf\",\"invoice\"]",
        ],
    )
    .expect("insert source features");
    conn.execute(
        "INSERT INTO symbol_features (artefact_id, repo_id, blob_sha, semantic_features_input_hash, normalized_name, normalized_signature, identifier_tokens, normalized_body_tokens, parent_kind, context_tokens)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        rusqlite::params![
            "artefact::invoice_doc",
            repo_id,
            "blob-2",
            "semantic-hash-2",
            "render_invoice_document",
            "function renderInvoiceDocument(orderId: string, locale: string)",
            "[\"render\",\"invoice\",\"document\",\"order\",\"locale\"]",
            "[\"render\",\"invoice\",\"document\",\"order\",\"locale\"]",
            "module",
            "[\"src\",\"render\",\"invoice\"]",
        ],
    )
    .expect("insert target features");

    conn.execute(
        "INSERT INTO symbol_embeddings (artefact_id, repo_id, blob_sha, provider, model, dimension, embedding_input_hash, embedding)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        rusqlite::params![
            "artefact::invoice_pdf",
            repo_id,
            "blob-1",
            "local",
            "jinaai/jina-embeddings-v2-base-code",
            3,
            "embed-hash-1",
            "[0.91,0.09,0.0]",
        ],
    )
    .expect("insert source embedding");
    conn.execute(
        "INSERT INTO symbol_embeddings (artefact_id, repo_id, blob_sha, provider, model, dimension, embedding_input_hash, embedding)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        rusqlite::params![
            "artefact::invoice_doc",
            repo_id,
            "blob-2",
            "local",
            "jinaai/jina-embeddings-v2-base-code",
            3,
            "embed-hash-2",
            "[0.89,0.11,0.0]",
        ],
    )
    .expect("insert target embedding");

    let clone_result = rebuild_symbol_clone_edges(&relational, repo_id)
        .await
        .expect("rebuild clone edges");
    assert!(clone_result.edges.len() >= 2);

    let parsed = parse_devql_query(
        r#"repo("temp2")->artefacts(kind:"function",symbol_fqn:"src/pdf.ts::createInvoicePdf")->clones(min_score:0.5)->limit(10)"#,
    )
    .expect("parse clone query");
    let rows = execute_relational_pipeline(&cfg, &events_cfg, &parsed, &relational)
        .await
        .expect("execute clone query");

    assert_eq!(rows.len(), 1);
    assert_eq!(
        rows[0]["target_symbol_fqn"],
        Value::String("src/render.ts::renderInvoiceDocument".to_string())
    );
    assert_eq!(
        rows[0]["relation_kind"],
        Value::String("similar_implementation".to_string())
    );
    assert!(rows[0]["explanation_json"].is_object());
}
