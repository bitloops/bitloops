use super::*;

#[test]
fn parse_devql_clones_stage_basic() {
    let parsed = parse_devql_query(
        r#"repo("temp2")->artefacts(kind:"function")->clones(relation_kind:"similar_implementation",min_score:0.6,raw:true)->limit(5)"#,
    )
    .unwrap();

    assert!(parsed.has_clones_stage);
    assert_eq!(
        parsed.clones.relation_kind.as_deref(),
        Some("similar_implementation")
    );
    assert_eq!(parsed.clones.min_score, Some(0.6));
    assert!(parsed.clones.raw);
}

#[test]
fn parse_devql_clones_stage_accepts_raw_false() {
    let parsed = parse_devql_query(
        r#"repo("temp2")->artefacts(kind:"function")->clones(raw:false)->limit(5)"#,
    )
    .unwrap();

    assert!(parsed.has_clones_stage);
    assert!(!parsed.clones.raw);
}

#[test]
fn parse_devql_clones_stage_rejects_invalid_raw_literal() {
    let err = parse_devql_query(
        r#"repo("temp2")->artefacts(kind:"function")->clones(raw:"maybe")->limit(5)"#,
    )
    .unwrap_err();

    assert!(
        err.to_string()
            .contains("invalid boolean value for clones raw")
    );
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
async fn execute_relational_pipeline_reads_commit_asof_clones_from_historical_tables() {
    let cfg = test_cfg();
    let events_cfg = default_events_cfg();
    let temp = tempdir().expect("tempdir");
    let sqlite_path = temp.path().join("relational.sqlite");
    let relational = sqlite_relational_store_with_schema(&sqlite_path).await;

    let conn = rusqlite::Connection::open(&sqlite_path).expect("open sqlite");
    let repo_id = cfg.repo.repo_id.as_str();

    conn.execute(
        "INSERT INTO file_state (repo_id, commit_sha, path, blob_sha) VALUES (?1, ?2, ?3, ?4)",
        rusqlite::params![repo_id, "commit-old", "src/pdf.ts", "blob-1"],
    )
    .expect("insert file_state");
    conn.execute(
        "INSERT INTO artefacts (
            artefact_id, symbol_id, repo_id, blob_sha, path, language, canonical_kind,
            language_kind, symbol_fqn, start_line, end_line, start_byte, end_byte, modifiers,
            content_hash
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
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
            "[]",
            "hash-1",
        ],
    )
    .expect("insert source historical artefact");
    conn.execute(
        "INSERT INTO artefacts (
            artefact_id, symbol_id, repo_id, blob_sha, path, language, canonical_kind,
            language_kind, symbol_fqn, start_line, end_line, start_byte, end_byte, modifiers,
            content_hash
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
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
            "[]",
            "hash-2",
        ],
    )
    .expect("insert target historical artefact");
    conn.execute(
        "INSERT INTO symbol_clone_edges (
            repo_id, source_symbol_id, source_artefact_id, target_symbol_id, target_artefact_id,
            relation_kind, score, semantic_score, lexical_score, structural_score,
            clone_input_hash, explanation_json
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
        rusqlite::params![
            repo_id,
            "sym::invoice_pdf",
            "artefact::invoice_pdf",
            "sym::invoice_doc",
            "artefact::invoice_doc",
            "similar_implementation",
            0.91_f64,
            0.91_f64,
            0.73_f64,
            0.69_f64,
            "clone-hash-old",
            "{}",
        ],
    )
    .expect("insert historical clone edge");

    let parsed = parse_devql_query(
        r#"repo("temp2")->asOf(commit:"commit-old")->file("src/pdf.ts")->artefacts(kind:"function")->clones(min_score:0.5)->limit(10)"#,
    )
    .expect("parse historical clone query");
    let rows = execute_relational_pipeline(&cfg, &events_cfg, &parsed, &relational)
        .await
        .expect("execute historical clone query");

    assert_eq!(rows.len(), 1);
    assert_eq!(
        rows[0]["source_path"],
        Value::String("src/pdf.ts".to_string())
    );
    assert_eq!(
        rows[0]["target_symbol_fqn"],
        Value::String("src/render.ts::renderInvoiceDocument".to_string())
    );
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
            repo_id, path, content_id, symbol_id, artefact_id, language,
            canonical_kind, language_kind, symbol_fqn, start_line, end_line, start_byte,
            end_byte, signature, modifiers, updated_at
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, '[]', ?15)",
        rusqlite::params![
            repo_id,
            "src/pdf.ts",
            "blob-1",
            "sym::invoice_pdf",
            "artefact::invoice_pdf",
            "typescript",
            "function",
            "function_declaration",
            "src/pdf.ts::createInvoicePdf",
            1,
            12,
            0,
            120,
            "function createInvoicePdf(orderId: string, locale: string)",
            "2026-03-26T09:00:00Z",
        ],
    )
    .expect("insert source current artefact");
    conn.execute(
        "INSERT INTO artefacts_current (
            repo_id, path, content_id, symbol_id, artefact_id, language,
            canonical_kind, language_kind, symbol_fqn, start_line, end_line, start_byte,
            end_byte, signature, modifiers, updated_at
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, '[]', ?15)",
        rusqlite::params![
            repo_id,
            "src/render.ts",
            "blob-2",
            "sym::invoice_doc",
            "artefact::invoice_doc",
            "typescript",
            "function",
            "function_declaration",
            "src/render.ts::renderInvoiceDocument",
            1,
            12,
            0,
            120,
            "function renderInvoiceDocument(orderId: string, locale: string)",
            "2026-03-26T09:00:00Z",
        ],
    )
    .expect("insert target current artefact");

    conn.execute(
        "INSERT INTO symbol_semantics_current (artefact_id, repo_id, path, content_id, symbol_id, semantic_features_input_hash, template_summary, summary, confidence)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        rusqlite::params![
            "artefact::invoice_pdf",
            repo_id,
            "src/pdf.ts",
            "blob-1",
            "sym::invoice_pdf",
            "semantic-hash-1",
            "Function create invoice pdf.",
            "Function create invoice pdf. Generates an invoice PDF for an order.",
            0.9_f64,
        ],
    )
    .expect("insert source semantics");
    conn.execute(
        "INSERT INTO symbol_semantics_current (artefact_id, repo_id, path, content_id, symbol_id, semantic_features_input_hash, template_summary, summary, confidence)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        rusqlite::params![
            "artefact::invoice_doc",
            repo_id,
            "src/render.ts",
            "blob-2",
            "sym::invoice_doc",
            "semantic-hash-2",
            "Function render invoice document.",
            "Function render invoice document. Renders the invoice document for an order.",
            0.9_f64,
        ],
    )
    .expect("insert target semantics");

    conn.execute(
        "INSERT INTO symbol_features_current (artefact_id, repo_id, path, content_id, symbol_id, semantic_features_input_hash, normalized_name, normalized_signature, identifier_tokens, normalized_body_tokens, parent_kind, context_tokens)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
        rusqlite::params![
            "artefact::invoice_pdf",
            repo_id,
            "src/pdf.ts",
            "blob-1",
            "sym::invoice_pdf",
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
        "INSERT INTO symbol_features_current (artefact_id, repo_id, path, content_id, symbol_id, semantic_features_input_hash, normalized_name, normalized_signature, identifier_tokens, normalized_body_tokens, parent_kind, context_tokens)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
        rusqlite::params![
            "artefact::invoice_doc",
            repo_id,
            "src/render.ts",
            "blob-2",
            "sym::invoice_doc",
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
        "INSERT INTO symbol_embeddings_current (artefact_id, repo_id, path, content_id, symbol_id, provider, model, dimension, embedding_input_hash, embedding)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        rusqlite::params![
            "artefact::invoice_pdf",
            repo_id,
            "src/pdf.ts",
            "blob-1",
            "sym::invoice_pdf",
            "local",
            "jinaai/jina-embeddings-v2-base-code",
            3,
            "embed-hash-1",
            "[0.91,0.09,0.0]",
        ],
    )
    .expect("insert source embedding");
    conn.execute(
        "INSERT INTO symbol_embeddings_current (artefact_id, repo_id, path, content_id, symbol_id, provider, model, dimension, embedding_input_hash, embedding)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        rusqlite::params![
            "artefact::invoice_doc",
            repo_id,
            "src/render.ts",
            "blob-2",
            "sym::invoice_doc",
            "local",
            "jinaai/jina-embeddings-v2-base-code",
            3,
            "embed-hash-2",
            "[0.89,0.11,0.0]",
        ],
    )
    .expect("insert target embedding");

    let clone_result =
        crate::capability_packs::semantic_clones::pipeline::rebuild_current_symbol_clone_edges(
            &relational,
            repo_id,
        )
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

#[tokio::test]
async fn execute_relational_pipeline_filters_clone_sources_by_exact_snapshot_identity() {
    let cfg = test_cfg();
    let events_cfg = default_events_cfg();
    let temp = tempdir().expect("tempdir");
    let sqlite_path = temp.path().join("relational.sqlite");
    let relational = sqlite_relational_store_with_schema(&sqlite_path).await;

    let conn = rusqlite::Connection::open(&sqlite_path).expect("open sqlite");
    let repo_id = cfg.repo.repo_id.as_str();

    for (symbol_id, artefact_id, path, blob_sha, symbol_fqn) in [
        (
            "sym::matched_source",
            "artefact::matched_source",
            "src/matched.ts",
            "shared-blob",
            "src/matched.ts::matched",
        ),
        (
            "sym::unmatched_source",
            "artefact::unmatched_source",
            "src/unmatched.ts",
            "shared-blob",
            "src/unmatched.ts::unmatched",
        ),
        (
            "sym::target_a",
            "artefact::target_a",
            "src/target-a.ts",
            "target-blob-a",
            "src/target-a.ts::targetA",
        ),
        (
            "sym::target_b",
            "artefact::target_b",
            "src/target-b.ts",
            "target-blob-b",
            "src/target-b.ts::targetB",
        ),
    ] {
        conn.execute(
            "INSERT INTO artefacts_current (
                repo_id, path, content_id, symbol_id, artefact_id, language,
                canonical_kind, language_kind, symbol_fqn, start_line, end_line, start_byte,
                end_byte, signature, modifiers, updated_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, '[]', ?15)",
            rusqlite::params![
                repo_id,
                path,
                blob_sha,
                symbol_id,
                artefact_id,
                "typescript",
                "function",
                "function_declaration",
                symbol_fqn,
                1,
                8,
                0,
                64,
                format!(
                    "function {}",
                    symbol_fqn.rsplit("::").next().unwrap_or("run")
                ),
                "2026-03-26T09:00:00Z",
            ],
        )
        .expect("insert current artefact");
    }

    conn.execute(
        "INSERT INTO checkpoint_files (
            relation_id, repo_id, checkpoint_id, session_id, event_time, agent, branch, strategy,
            commit_sha, change_kind, path_before, path_after, blob_sha_before, blob_sha_after
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 'modify', ?10, ?10, ?11, ?11)",
        rusqlite::params![
            "relation-1",
            repo_id,
            "checkpoint-1",
            "session-1",
            "2026-03-21T10:00:00Z",
            "codex",
            "main",
            "manual",
            "commit-1",
            "src/matched.ts",
            "shared-blob",
        ],
    )
    .expect("insert checkpoint provenance row");

    for (source_symbol_id, source_artefact_id, target_symbol_id, target_artefact_id, score) in [
        (
            "sym::matched_source",
            "artefact::matched_source",
            "sym::target_a",
            "artefact::target_a",
            0.91_f64,
        ),
        (
            "sym::unmatched_source",
            "artefact::unmatched_source",
            "sym::target_b",
            "artefact::target_b",
            0.89_f64,
        ),
    ] {
        conn.execute(
            "INSERT INTO symbol_clone_edges_current (
                repo_id, source_symbol_id, source_artefact_id, target_symbol_id,
                target_artefact_id, relation_kind, score, semantic_score, lexical_score,
                structural_score, clone_input_hash, explanation_json
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            rusqlite::params![
                repo_id,
                source_symbol_id,
                source_artefact_id,
                target_symbol_id,
                target_artefact_id,
                "similar_implementation",
                score,
                score,
                0.6_f64,
                0.5_f64,
                format!("clone-hash-{source_symbol_id}"),
                "{}",
            ],
        )
        .expect("insert clone edge");
    }

    let parsed = parse_devql_query(
        r#"repo("temp2")->artefacts(kind:"function",agent:"codex")->clones(min_score:0.5)->limit(10)"#,
    )
    .expect("parse clone query");
    let rows = execute_relational_pipeline(&cfg, &events_cfg, &parsed, &relational)
        .await
        .expect("execute projection-backed clone query");

    assert_eq!(rows.len(), 1);
    assert_eq!(
        rows[0]["source_path"],
        Value::String("src/matched.ts".to_string())
    );
    assert_eq!(
        rows[0]["target_symbol_fqn"],
        Value::String("src/target-a.ts::targetA".to_string())
    );
}

#[allow(clippy::too_many_arguments)]
fn insert_clone_candidate_fixture(
    conn: &rusqlite::Connection,
    repo_id: &str,
    symbol_id: &str,
    artefact_id: &str,
    path: &str,
    symbol_fqn: &str,
    normalized_name: &str,
    summary: &str,
    provider: &str,
    model: &str,
    dimension: i64,
    embedding: &str,
) {
    conn.execute(
        "INSERT INTO artefacts (artefact_id, symbol_id, repo_id, blob_sha, path, language, canonical_kind, language_kind, symbol_fqn, parent_artefact_id, start_line, end_line, start_byte, end_byte, signature, modifiers, content_hash)
         VALUES (?1, ?2, ?3, ?4, ?5, 'typescript', 'function', 'function_declaration', ?6, NULL, 1, 12, 0, 120, ?7, '[]', ?8)",
        rusqlite::params![
            artefact_id,
            symbol_id,
            repo_id,
            format!("blob-{symbol_id}"),
            path,
            symbol_fqn,
            format!("function {normalized_name}(id: string)"),
            format!("hash-{symbol_id}"),
        ],
    )
    .expect("insert artefact history");

    conn.execute(
        "INSERT INTO artefacts_current (
            repo_id, path, content_id, symbol_id, artefact_id, language,
            canonical_kind, language_kind, symbol_fqn, start_line, end_line, start_byte,
            end_byte, signature, modifiers, updated_at
        ) VALUES (?1, ?2, ?3, ?4, ?5, 'typescript', 'function', 'function_declaration', ?6, 1, 12, 0, 120, ?7, '[]', '2026-03-26T09:00:00Z')",
        rusqlite::params![
            repo_id,
            path,
            format!("blob-{symbol_id}"),
            symbol_id,
            artefact_id,
            symbol_fqn,
            format!("function {normalized_name}(id: string)"),
        ],
    )
    .expect("insert current artefact");

    conn.execute(
        "INSERT INTO symbol_semantics (artefact_id, repo_id, blob_sha, semantic_features_input_hash, template_summary, summary, confidence)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, 0.9)",
        rusqlite::params![
            artefact_id,
            repo_id,
            format!("blob-{symbol_id}"),
            format!("semantic-hash-{symbol_id}"),
            format!("Function {normalized_name}."),
            summary,
        ],
    )
    .expect("insert semantics");

    conn.execute(
        "INSERT INTO symbol_features (artefact_id, repo_id, blob_sha, semantic_features_input_hash, normalized_name, normalized_signature, identifier_tokens, normalized_body_tokens, parent_kind, context_tokens)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, '[\"invoice\",\"document\"]', '[\"render\",\"invoice\",\"document\"]', 'module', '[\"src\",\"billing\"]')",
        rusqlite::params![
            artefact_id,
            repo_id,
            format!("blob-{symbol_id}"),
            format!("semantic-hash-{symbol_id}"),
            normalized_name,
            format!("function {normalized_name}(id: string)"),
        ],
    )
    .expect("insert features");

    conn.execute(
        "INSERT INTO symbol_embeddings (artefact_id, repo_id, blob_sha, provider, model, dimension, embedding_input_hash, embedding)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        rusqlite::params![
            artefact_id,
            repo_id,
            format!("blob-{symbol_id}"),
            provider,
            model,
            dimension,
            format!("embed-hash-{symbol_id}"),
            embedding,
        ],
    )
    .expect("insert embedding");
}

#[tokio::test]
async fn rebuild_symbol_clone_edges_uses_only_active_embedding_setup_candidates() {
    let cfg = test_cfg();
    let temp = tempdir().expect("tempdir");
    let sqlite_path = temp.path().join("relational.sqlite");
    let relational = sqlite_relational_store_with_schema(&sqlite_path).await;
    crate::capability_packs::semantic_clones::ensure_semantic_embeddings_schema(&relational)
        .await
        .expect("ensure semantic embedding schema");

    let conn = rusqlite::Connection::open(&sqlite_path).expect("open sqlite");
    let repo_id = cfg.repo.repo_id.as_str();
    insert_clone_candidate_fixture(
        &conn,
        repo_id,
        "sym::source",
        "artefact::source",
        "src/source.ts",
        "src/source.ts::renderInvoice",
        "render_invoice",
        "Function render invoice. Renders the invoice document.",
        "local_fastembed",
        "jinaai/jina-embeddings-v2-base-code",
        3,
        "[0.91,0.09,0.0]",
    );
    insert_clone_candidate_fixture(
        &conn,
        repo_id,
        "sym::target_current",
        "artefact::target_current",
        "src/target-current.ts",
        "src/target-current.ts::renderInvoiceDocument",
        "render_invoice_document",
        "Function render invoice document. Renders the invoice document for an order.",
        "local_fastembed",
        "jinaai/jina-embeddings-v2-base-code",
        3,
        "[0.89,0.11,0.0]",
    );
    insert_clone_candidate_fixture(
        &conn,
        repo_id,
        "sym::target_stale",
        "artefact::target_stale",
        "src/target-stale.ts",
        "src/target-stale.ts::renderInvoiceDraft",
        "render_invoice_draft",
        "Function render invoice draft. Renders the invoice document for an order.",
        "voyage",
        "voyage-code-3",
        3,
        "[0.92,0.08,0.0]",
    );
    conn.execute(
        "INSERT INTO semantic_clone_embedding_setup_state (repo_id, provider, model, dimension, setup_fingerprint)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        rusqlite::params![
            repo_id,
            "local_fastembed",
            "jinaai/jina-embeddings-v2-base-code",
            3_i64,
            crate::capability_packs::semantic_clones::embeddings::EmbeddingSetup::new(
                "local_fastembed",
                "jinaai/jina-embeddings-v2-base-code",
                3,
            )
            .setup_fingerprint,
        ],
    )
    .expect("insert active setup");

    let clone_result = rebuild_symbol_clone_edges(&relational, repo_id)
        .await
        .expect("rebuild clone edges");

    assert!(
        clone_result
            .edges
            .iter()
            .any(|edge| edge.target_symbol_id == "sym::target_current")
    );
    assert!(
        clone_result
            .edges
            .iter()
            .all(|edge| edge.target_symbol_id != "sym::target_stale")
    );
}

#[tokio::test]
async fn rebuild_symbol_clone_edges_bootstraps_single_current_embedding_setup() {
    let cfg = test_cfg();
    let temp = tempdir().expect("tempdir");
    let sqlite_path = temp.path().join("relational.sqlite");
    let relational = sqlite_relational_store_with_schema(&sqlite_path).await;
    crate::capability_packs::semantic_clones::ensure_semantic_embeddings_schema(&relational)
        .await
        .expect("ensure semantic embedding schema");

    let conn = rusqlite::Connection::open(&sqlite_path).expect("open sqlite");
    let repo_id = cfg.repo.repo_id.as_str();
    insert_clone_candidate_fixture(
        &conn,
        repo_id,
        "sym::source",
        "artefact::source",
        "src/source.ts",
        "src/source.ts::renderInvoice",
        "render_invoice",
        "Function render invoice. Renders the invoice document.",
        "local_fastembed",
        "jinaai/jina-embeddings-v2-base-code",
        3,
        "[0.91,0.09,0.0]",
    );
    insert_clone_candidate_fixture(
        &conn,
        repo_id,
        "sym::target",
        "artefact::target",
        "src/target.ts",
        "src/target.ts::renderInvoiceDocument",
        "render_invoice_document",
        "Function render invoice document. Renders the invoice document for an order.",
        "local_fastembed",
        "jinaai/jina-embeddings-v2-base-code",
        3,
        "[0.89,0.11,0.0]",
    );

    let clone_result = rebuild_symbol_clone_edges(&relational, repo_id)
        .await
        .expect("rebuild clone edges");

    assert!(!clone_result.edges.is_empty());

    let persisted_setup = relational
        .query_rows("SELECT provider, model, dimension FROM semantic_clone_embedding_setup_state")
        .await
        .expect("read persisted setup");
    assert_eq!(persisted_setup.len(), 1);
    assert_eq!(
        persisted_setup[0]["provider"],
        Value::String("local_fastembed".to_string())
    );
}

#[tokio::test]
async fn rebuild_symbol_clone_edges_refuses_to_bootstrap_mixed_current_embedding_setups() {
    let cfg = test_cfg();
    let temp = tempdir().expect("tempdir");
    let sqlite_path = temp.path().join("relational.sqlite");
    let relational = sqlite_relational_store_with_schema(&sqlite_path).await;
    crate::capability_packs::semantic_clones::ensure_semantic_embeddings_schema(&relational)
        .await
        .expect("ensure semantic embedding schema");

    let conn = rusqlite::Connection::open(&sqlite_path).expect("open sqlite");
    let repo_id = cfg.repo.repo_id.as_str();
    insert_clone_candidate_fixture(
        &conn,
        repo_id,
        "sym::source",
        "artefact::source",
        "src/source.ts",
        "src/source.ts::renderInvoice",
        "render_invoice",
        "Function render invoice. Renders the invoice document.",
        "local_fastembed",
        "jinaai/jina-embeddings-v2-base-code",
        3,
        "[0.91,0.09,0.0]",
    );
    insert_clone_candidate_fixture(
        &conn,
        repo_id,
        "sym::target_current",
        "artefact::target_current",
        "src/target-current.ts",
        "src/target-current.ts::renderInvoiceDocument",
        "render_invoice_document",
        "Function render invoice document. Renders the invoice document for an order.",
        "local_fastembed",
        "jinaai/jina-embeddings-v2-base-code",
        3,
        "[0.89,0.11,0.0]",
    );
    insert_clone_candidate_fixture(
        &conn,
        repo_id,
        "sym::target_stale",
        "artefact::target_stale",
        "src/target-stale.ts",
        "src/target-stale.ts::renderInvoiceDraft",
        "render_invoice_draft",
        "Function render invoice draft. Renders the invoice document for an order.",
        "voyage",
        "voyage-code-3",
        1024,
        "[0.92,0.08,0.0]",
    );

    let clone_result = rebuild_symbol_clone_edges(&relational, repo_id)
        .await
        .expect("rebuild clone edges");

    assert!(clone_result.edges.is_empty());

    let persisted_setup = relational
        .query_rows("SELECT provider, model, dimension FROM semantic_clone_embedding_setup_state")
        .await
        .expect("read persisted setup");
    assert!(persisted_setup.is_empty());

    let stored_edges = relational
        .query_rows("SELECT source_symbol_id, target_symbol_id FROM symbol_clone_edges")
        .await
        .expect("read stored clone edges");
    assert!(stored_edges.is_empty());
}
