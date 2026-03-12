async fn load_pre_stage_artefacts_for_blob(
    relational_store: &dyn store_contracts::RelationalStore,
    repo_id: &str,
    blob_sha: &str,
    path: &str,
) -> Result<Vec<semantic::PreStageArtefactRow>> {
    let rows = relational_store
        .query_rows(&build_semantic_get_artefacts_sql(repo_id, blob_sha, path))
        .await?;
    parse_semantic_artefact_rows(rows)
}

async fn upsert_semantic_feature_rows(
    relational_store: &dyn store_contracts::RelationalStore,
    inputs: &[semantic::SemanticFeatureInput],
    summary_provider: &dyn semantic::SemanticSummaryProvider,
) -> Result<semantic::SemanticFeatureIngestionStats> {
    let mut stats = semantic::SemanticFeatureIngestionStats::default();

    for input in inputs {
        let rows = semantic::build_semantic_feature_rows(input, summary_provider);
        let state = load_semantic_index_state(relational_store, &input.artefact_id).await?;
        if !semantic::semantic_features_require_reindex(&state, &rows.semantic_features_input_hash)
        {
            stats.skipped += 1;
            continue;
        }

        persist_semantic_feature_rows(relational_store, &rows).await?;
        stats.upserted += 1;
    }

    Ok(stats)
}

async fn load_semantic_index_state(
    relational_store: &dyn store_contracts::RelationalStore,
    artefact_id: &str,
) -> Result<semantic::SemanticFeatureIndexState> {
    let rows = relational_store
        .query_rows(&build_semantic_get_index_state_sql(artefact_id))
        .await?;
    Ok(parse_semantic_index_state_rows(&rows))
}

async fn persist_semantic_feature_rows(
    relational_store: &dyn store_contracts::RelationalStore,
    rows: &semantic::SemanticFeatureRows,
) -> Result<()> {
    relational_store
        .execute(&build_semantic_persist_rows_sql(rows)?)
        .await
}

fn build_semantic_get_artefacts_sql(repo_id: &str, blob_sha: &str, path: &str) -> String {
    format!(
        "SELECT artefact_id, symbol_id, repo_id, blob_sha, path, language, canonical_kind, language_kind, symbol_fqn, parent_artefact_id, start_line, end_line, start_byte, end_byte, signature, content_hash \
FROM artefacts \
WHERE repo_id = '{repo_id}' AND blob_sha = '{blob_sha}' AND path = '{path}' \
ORDER BY coalesce(start_byte, 0), coalesce(start_line, 0), artefact_id",
        repo_id = esc_pg(repo_id),
        blob_sha = esc_pg(blob_sha),
        path = esc_pg(path),
    )
}

fn parse_semantic_artefact_rows(rows: Vec<Value>) -> Result<Vec<semantic::PreStageArtefactRow>> {
    let mut artefacts = Vec::new();
    for row in rows {
        artefacts.push(serde_json::from_value::<semantic::PreStageArtefactRow>(
            row,
        )?);
    }
    Ok(artefacts)
}

fn build_semantic_get_index_state_sql(artefact_id: &str) -> String {
    format!(
        "SELECT \
            (SELECT semantic_features_input_hash FROM symbol_semantics WHERE artefact_id = '{artefact_id}') AS semantics_hash, \
            (SELECT semantic_features_input_hash FROM symbol_features WHERE artefact_id = '{artefact_id}') AS features_hash",
        artefact_id = esc_pg(artefact_id),
    )
}

fn parse_semantic_index_state_rows(rows: &[Value]) -> semantic::SemanticFeatureIndexState {
    let Some(row) = rows.first() else {
        return semantic::SemanticFeatureIndexState::default();
    };

    semantic::SemanticFeatureIndexState {
        semantics_hash: row
            .get("semantics_hash")
            .and_then(Value::as_str)
            .map(str::to_string),
        features_hash: row
            .get("features_hash")
            .and_then(Value::as_str)
            .map(str::to_string),
    }
}

fn build_semantic_persist_rows_sql(rows: &semantic::SemanticFeatureRows) -> Result<String> {
    let semantics = &rows.semantics;
    let features = &rows.features;

    let doc_comment_summary_expr = match semantics.doc_comment_summary.as_deref() {
        Some(value) => format!("'{}'", esc_pg(value)),
        None => "NULL".to_string(),
    };
    let llm_summary_expr = match semantics.llm_summary.as_deref() {
        Some(value) => format!("'{}'", esc_pg(value)),
        None => "NULL".to_string(),
    };
    let source_model_expr = match semantics.source_model.as_deref() {
        Some(value) => format!("'{}'", esc_pg(value)),
        None => "NULL".to_string(),
    };
    let normalized_signature_expr = match features.normalized_signature.as_deref() {
        Some(value) => format!("'{}'", esc_pg(value)),
        None => "NULL".to_string(),
    };
    let parent_kind_expr = match features.parent_kind.as_deref() {
        Some(value) => format!("'{}'", esc_pg(value)),
        None => "NULL".to_string(),
    };
    let parent_symbol_expr = match features.parent_symbol.as_deref() {
        Some(value) => format!("'{}'", esc_pg(value)),
        None => "NULL".to_string(),
    };
    let identifier_tokens_expr = format!(
        "'{}'",
        esc_pg(&serde_json::to_string(&features.identifier_tokens)?)
    );
    let body_tokens_expr = format!(
        "'{}'",
        esc_pg(&serde_json::to_string(&features.normalized_body_tokens)?)
    );
    let local_relationships_expr = format!(
        "'{}'",
        esc_pg(&serde_json::to_string(&features.local_relationships)?)
    );
    let context_tokens_expr = format!(
        "'{}'",
        esc_pg(&serde_json::to_string(&features.context_tokens)?)
    );

    Ok(format!(
        "INSERT INTO symbol_semantics (artefact_id, repo_id, blob_sha, semantic_features_input_hash, doc_comment_summary, llm_summary, template_summary, summary, confidence, source_model) \
VALUES ('{artefact_id}', '{repo_id}', '{blob_sha}', '{input_hash}', {doc_comment_summary}, {llm_summary}, '{template_summary}', '{summary}', {confidence:.4}, {source_model}) \
ON CONFLICT (artefact_id) DO UPDATE SET repo_id = EXCLUDED.repo_id, blob_sha = EXCLUDED.blob_sha, semantic_features_input_hash = EXCLUDED.semantic_features_input_hash, doc_comment_summary = EXCLUDED.doc_comment_summary, llm_summary = EXCLUDED.llm_summary, template_summary = EXCLUDED.template_summary, summary = EXCLUDED.summary, confidence = EXCLUDED.confidence, source_model = EXCLUDED.source_model, generated_at = CURRENT_TIMESTAMP; \
INSERT INTO symbol_features (artefact_id, repo_id, blob_sha, semantic_features_input_hash, normalized_name, normalized_signature, identifier_tokens, normalized_body_tokens, parent_kind, parent_symbol, local_relationships, context_tokens) \
VALUES ('{features_artefact_id}', '{features_repo_id}', '{features_blob_sha}', '{features_input_hash}', '{normalized_name}', {normalized_signature}, {identifier_tokens}, {body_tokens}, {parent_kind}, {parent_symbol}, {local_relationships}, {context_tokens}) \
ON CONFLICT (artefact_id) DO UPDATE SET repo_id = EXCLUDED.repo_id, blob_sha = EXCLUDED.blob_sha, semantic_features_input_hash = EXCLUDED.semantic_features_input_hash, normalized_name = EXCLUDED.normalized_name, normalized_signature = EXCLUDED.normalized_signature, identifier_tokens = EXCLUDED.identifier_tokens, normalized_body_tokens = EXCLUDED.normalized_body_tokens, parent_kind = EXCLUDED.parent_kind, parent_symbol = EXCLUDED.parent_symbol, local_relationships = EXCLUDED.local_relationships, context_tokens = EXCLUDED.context_tokens, generated_at = CURRENT_TIMESTAMP",
        artefact_id = esc_pg(&semantics.artefact_id),
        repo_id = esc_pg(&semantics.repo_id),
        blob_sha = esc_pg(&semantics.blob_sha),
        input_hash = esc_pg(&rows.semantic_features_input_hash),
        doc_comment_summary = doc_comment_summary_expr,
        llm_summary = llm_summary_expr,
        template_summary = esc_pg(&semantics.template_summary),
        summary = esc_pg(&semantics.summary),
        confidence = semantics.confidence,
        source_model = source_model_expr,
        features_artefact_id = esc_pg(&features.artefact_id),
        features_repo_id = esc_pg(&features.repo_id),
        features_blob_sha = esc_pg(&features.blob_sha),
        features_input_hash = esc_pg(&rows.semantic_features_input_hash),
        normalized_name = esc_pg(&features.normalized_name),
        normalized_signature = normalized_signature_expr,
        identifier_tokens = identifier_tokens_expr,
        body_tokens = body_tokens_expr,
        parent_kind = parent_kind_expr,
        parent_symbol = parent_symbol_expr,
        local_relationships = local_relationships_expr,
        context_tokens = context_tokens_expr,
    ))
}

#[cfg(test)]
mod semantic_feature_relational_tests {
    use super::*;

    fn sample_semantic_rows() -> semantic::SemanticFeatureRows {
        semantic::build_semantic_feature_rows(
            &semantic::SemanticFeatureInput {
                artefact_id: "artefact'1".to_string(),
                symbol_id: Some("symbol-1".to_string()),
                repo_id: "repo-1".to_string(),
                blob_sha: "blob-1".to_string(),
                path: "src/services/user.ts".to_string(),
                language: "typescript".to_string(),
                canonical_kind: "method".to_string(),
                language_kind: "method".to_string(),
                symbol_fqn: "src/services/user.ts::UserService::getById".to_string(),
                name: "getById".to_string(),
                signature: Some("async getById(id: string): Promise<User | null>".to_string()),
                body: "return repo.findById(id);".to_string(),
                doc_comment: Some("Fetches O'Brien by id.".to_string()),
                parent_kind: Some("class".to_string()),
                parent_symbol: Some("src/services/user.ts::UserService".to_string()),
                local_relationships: vec!["contains:method".to_string()],
                context_hints: vec!["src/services/user.ts".to_string()],
                content_hash: Some("hash-1".to_string()),
            },
            &semantic::NoopSemanticSummaryProvider,
        )
    }

    #[test]
    fn semantic_feature_relational_builds_get_artefacts_sql_with_escaped_values() {
        let sql = build_semantic_get_artefacts_sql("repo'1", "blob'1", "src/o'brien.ts");
        assert!(sql.contains("repo_id = 'repo''1'"));
        assert!(sql.contains("blob_sha = 'blob''1'"));
        assert!(sql.contains("path = 'src/o''brien.ts'"));
    }

    #[test]
    fn semantic_feature_relational_parses_index_state_rows_and_defaults() {
        let empty = parse_semantic_index_state_rows(&[]);
        assert_eq!(empty, semantic::SemanticFeatureIndexState::default());

        let rows = vec![json!({
            "semantics_hash": "hash-a",
            "features_hash": "hash-b",
        })];
        let parsed = parse_semantic_index_state_rows(&rows);
        assert_eq!(parsed.semantics_hash.as_deref(), Some("hash-a"));
        assert_eq!(parsed.features_hash.as_deref(), Some("hash-b"));
    }

    #[test]
    fn semantic_feature_relational_builds_provider_neutral_persist_sql() {
        let sql = build_semantic_persist_rows_sql(&sample_semantic_rows()).expect("persist SQL");
        assert!(sql.contains("INSERT INTO symbol_semantics"));
        assert!(sql.contains("INSERT INTO symbol_features"));
        assert!(sql.contains("doc_comment_summary"));
        assert!(sql.contains("Fetches O''Brien by id."));
        assert!(sql.contains("'[\"contains:method\"]'"));
        assert!(!sql.contains("::jsonb"));
    }
}
