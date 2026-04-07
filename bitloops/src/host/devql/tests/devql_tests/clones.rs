use super::*;

#[test]
fn parse_devql_clones_stage_basic() {
    let parsed = parse_devql_query(
        r#"repo("temp2")->artefacts(kind:"function")->clones(relation_kind:"similar_implementation",min_score:0.6,neighbors:9)->limit(5)"#,
    )
    .unwrap();

    assert!(parsed.has_clones_stage);
    assert_eq!(
        parsed.clones.relation_kind.as_deref(),
        Some("similar_implementation")
    );
    assert_eq!(parsed.clones.min_score, Some(0.6));
    assert_eq!(parsed.clones.neighbors, Some(9));
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
async fn execute_relational_pipeline_rejects_neighbors_without_single_source() {
    let cfg = test_cfg();
    let events_cfg = default_events_cfg();
    let temp = tempdir().expect("tempdir");
    let sqlite_path = temp.path().join("relational.sqlite");
    let relational = sqlite_relational_store_with_schema(&sqlite_path).await;

    let parsed = parse_devql_query(
        r#"repo("temp2")->artefacts(kind:"function")->clones(neighbors:5)->limit(10)"#,
    )
    .expect("parse clone query");
    let err = execute_relational_pipeline(&cfg, &events_cfg, &parsed, &relational)
        .await
        .expect_err("neighbors override should require one source");
    assert_eq!(
        err.to_string(),
        "clones(neighbors:...) requires the source artefact set to resolve to exactly one source symbol"
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

    let parsed_neighbors = parse_devql_query(
        r#"repo("temp2")->artefacts(kind:"function",symbol_fqn:"src/pdf.ts::createInvoicePdf")->clones(neighbors:5,min_score:0.5)->limit(10)"#,
    )
    .expect("parse clone neighbors query");
    let on_demand_rows =
        execute_relational_pipeline(&cfg, &events_cfg, &parsed_neighbors, &relational)
            .await
            .expect("execute on-demand neighbors query");
    assert_eq!(on_demand_rows.len(), 1);
    assert_eq!(
        on_demand_rows[0]["target_symbol_fqn"],
        Value::String("src/render.ts::renderInvoiceDocument".to_string())
    );
    assert_eq!(
        on_demand_rows[0]["relation_kind"],
        Value::String("similar_implementation".to_string())
    );
    assert!(on_demand_rows[0]["score"].is_number());
    assert!(on_demand_rows[0]["semantic_score"].is_number());
    assert!(on_demand_rows[0]["lexical_score"].is_number());
    assert!(on_demand_rows[0]["structural_score"].is_number());
    assert_eq!(
        on_demand_rows[0]["source_symbol_fqn"],
        Value::String("src/pdf.ts::createInvoicePdf".to_string())
    );
    assert!(on_demand_rows[0]["explanation_json"].is_object());

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

#[tokio::test]
async fn execute_relational_pipeline_clones_without_neighbors_reads_persisted_edges_without_scoring_inputs()
 {
    let cfg = test_cfg();
    let events_cfg = default_events_cfg();
    let temp = tempdir().expect("tempdir");
    let sqlite_path = temp.path().join("relational.sqlite");
    let relational = sqlite_relational_store_with_schema(&sqlite_path).await;
    let conn = rusqlite::Connection::open(&sqlite_path).expect("open sqlite");
    let repo_id = cfg.repo.repo_id.as_str();

    for (symbol_id, artefact_id, path, symbol_fqn) in [
        (
            "sym::persisted_source",
            "artefact::persisted_source",
            "src/source.ts",
            "src/source.ts::persistedSource",
        ),
        (
            "sym::persisted_target",
            "artefact::persisted_target",
            "src/target.ts",
            "src/target.ts::persistedTarget",
        ),
    ] {
        conn.execute(
            "INSERT INTO artefacts_current (
                repo_id, path, content_id, symbol_id, artefact_id, language,
                canonical_kind, language_kind, symbol_fqn, start_line, end_line, start_byte,
                end_byte, signature, modifiers, updated_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 1, 8, 0, 80, ?10, '[]', ?11)",
            rusqlite::params![
                repo_id,
                path,
                format!("blob-{symbol_id}"),
                symbol_id,
                artefact_id,
                "typescript",
                "function",
                "function_declaration",
                symbol_fqn,
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
        "INSERT INTO symbol_clone_edges (
            repo_id, source_symbol_id, source_artefact_id, target_symbol_id, target_artefact_id,
            relation_kind, score, semantic_score, lexical_score, structural_score,
            clone_input_hash, explanation_json
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
        rusqlite::params![
            repo_id,
            "sym::persisted_source",
            "artefact::persisted_source",
            "sym::persisted_target",
            "artefact::persisted_target",
            "similar_implementation",
            0.77_f64,
            0.80_f64,
            0.63_f64,
            0.55_f64,
            "clone-hash-persisted",
            r#"{"source":"persisted_only"}"#,
        ],
    )
    .expect("insert persisted clone edge");

    let parsed = parse_devql_query(
        r#"repo("temp2")->artefacts(kind:"function",symbol_fqn:"src/source.ts::persistedSource")->clones(min_score:0.7)->limit(10)"#,
    )
    .expect("parse persisted clone query");
    let rows = execute_relational_pipeline(&cfg, &events_cfg, &parsed, &relational)
        .await
        .expect("execute persisted clone query");

    assert_eq!(rows.len(), 1);
    assert_eq!(
        rows[0]["source_symbol_fqn"],
        Value::String("src/source.ts::persistedSource".to_string())
    );
    assert_eq!(
        rows[0]["target_symbol_fqn"],
        Value::String("src/target.ts::persistedTarget".to_string())
    );
    assert_eq!(
        rows[0]["relation_kind"],
        Value::String("similar_implementation".to_string())
    );
    assert!(rows[0]["score"].is_number());
    assert!(rows[0]["semantic_score"].is_number());
    assert!(rows[0]["lexical_score"].is_number());
    assert!(rows[0]["structural_score"].is_number());
    assert!(rows[0]["explanation_json"].is_object());
    assert!(rows[0]["target_summary"].is_null());
}

#[tokio::test]
async fn execute_relational_pipeline_neighbors_and_persisted_paths_share_ordering_contract() {
    let cfg = test_cfg();
    let events_cfg = default_events_cfg();
    let temp = tempdir().expect("tempdir");
    let sqlite_path = temp.path().join("relational.sqlite");
    let relational = sqlite_relational_store_with_schema(&sqlite_path).await;
    let conn = rusqlite::Connection::open(&sqlite_path).expect("open sqlite");
    let repo_id = cfg.repo.repo_id.as_str();

    for (
        symbol_id,
        artefact_id,
        path,
        symbol_fqn,
        normalized_name,
        normalized_signature,
        summary,
        embedding,
    ) in [
        (
            "sym::source",
            "artefact::source",
            "src/source.ts",
            "src/source.ts::process",
            "process",
            "function process(orderId: string)",
            "Processes an order payload.",
            "[0.98,0.02,0.0]",
        ),
        (
            "sym::target_a",
            "artefact::target_a",
            "src/target-a.ts",
            "src/target-a.ts::process",
            "process",
            "function process(orderId: string)",
            "Processes an order payload copy.",
            "[0.75,0.25,0.0]",
        ),
        (
            "sym::target_b",
            "artefact::target_b",
            "src/target-b.ts",
            "src/target-b.ts::process",
            "process",
            "function process(orderId: string)",
            "Processes an order payload copy.",
            "[0.74,0.26,0.0]",
        ),
    ] {
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
                normalized_signature,
            ],
        )
        .expect("insert current artefact");

        conn.execute(
            "INSERT INTO symbol_semantics (
                artefact_id, repo_id, blob_sha, semantic_features_input_hash,
                template_summary, summary, confidence
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            rusqlite::params![
                artefact_id,
                repo_id,
                format!("blob-{symbol_id}"),
                format!("semantic-hash-{symbol_id}"),
                summary,
                summary,
                0.92_f64,
            ],
        )
        .expect("insert semantic summary");

        conn.execute(
            "INSERT INTO symbol_features (
                artefact_id, repo_id, blob_sha, semantic_features_input_hash,
                normalized_name, normalized_signature, modifiers, identifier_tokens,
                normalized_body_tokens, parent_kind, context_tokens
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, '[]', ?7, ?8, ?9, ?10)",
            rusqlite::params![
                artefact_id,
                repo_id,
                format!("blob-{symbol_id}"),
                format!("semantic-hash-{symbol_id}"),
                normalized_name,
                normalized_signature,
                r#"["process","order","payload"]"#,
                r#"["validate","order","payload","return"]"#,
                "module",
                r#"["src","orders"]"#,
            ],
        )
        .expect("insert semantic features");

        conn.execute(
            "INSERT INTO symbol_embeddings (
                artefact_id, repo_id, blob_sha, provider, model, dimension, embedding_input_hash, embedding
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            rusqlite::params![
                artefact_id,
                repo_id,
                format!("blob-{symbol_id}"),
                "local",
                "jinaai/jina-embeddings-v2-base-code",
                3,
                format!("embed-hash-{symbol_id}"),
                embedding,
            ],
        )
        .expect("insert embedding");
    }

    let parsed_neighbors = parse_devql_query(
        r#"repo("temp2")->artefacts(kind:"function",symbol_fqn:"src/source.ts::process")->clones(neighbors:5,min_score:0.4)->limit(10)"#,
    )
    .expect("parse on-demand clone query");
    let on_demand_rows =
        execute_relational_pipeline(&cfg, &events_cfg, &parsed_neighbors, &relational)
            .await
            .expect("execute on-demand clone query");

    assert_eq!(on_demand_rows.len(), 2);
    for row in &on_demand_rows {
        assert!(row["relation_kind"].is_string());
        assert!(row["score"].is_number());
        assert!(row["semantic_score"].is_number());
        assert!(row["lexical_score"].is_number());
        assert!(row["structural_score"].is_number());
        assert!(row["explanation_json"].is_object());
    }

    for pair in on_demand_rows.windows(2) {
        let left_score = pair[0]["score"].as_f64().expect("left score");
        let right_score = pair[1]["score"].as_f64().expect("right score");
        assert!(left_score >= right_score);
        if (left_score - right_score).abs() < f64::EPSILON {
            let left_target = pair[0]["target_symbol_fqn"]
                .as_str()
                .expect("left target symbol fqn");
            let right_target = pair[1]["target_symbol_fqn"]
                .as_str()
                .expect("right target symbol fqn");
            assert!(left_target <= right_target);
        }
    }

    rebuild_symbol_clone_edges(&relational, repo_id)
        .await
        .expect("rebuild persisted clone edges");
    let parsed_persisted = parse_devql_query(
        r#"repo("temp2")->artefacts(kind:"function",symbol_fqn:"src/source.ts::process")->clones(min_score:0.4)->limit(10)"#,
    )
    .expect("parse persisted clone query");
    let persisted_rows =
        execute_relational_pipeline(&cfg, &events_cfg, &parsed_persisted, &relational)
            .await
            .expect("execute persisted clone query");

    assert_eq!(persisted_rows.len(), 2);
    let on_demand_targets = on_demand_rows
        .iter()
        .map(|row| {
            row["target_symbol_fqn"]
                .as_str()
                .expect("on-demand target symbol fqn")
                .to_string()
        })
        .collect::<Vec<_>>();
    let persisted_targets = persisted_rows
        .iter()
        .map(|row| {
            row["target_symbol_fqn"]
                .as_str()
                .expect("persisted target symbol fqn")
                .to_string()
        })
        .collect::<Vec<_>>();
    assert_eq!(on_demand_targets, persisted_targets);
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
            "INSERT INTO symbol_clone_edges (
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
