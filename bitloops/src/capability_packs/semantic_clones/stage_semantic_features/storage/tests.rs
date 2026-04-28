use super::*;
use crate::capability_packs::semantic_clones::features as semantic;
use crate::host::devql::RelationalDialect;
use serde_json::json;

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
            modifiers: vec!["public".to_string(), "async".to_string()],
            body: "return repo.findById(id);".to_string(),
            docstring: Some("Fetches O'Brien by id.".to_string()),
            parent_kind: Some("class".to_string()),
            dependency_signals: vec!["calls:user_repo::find_by_id".to_string()],
            content_hash: Some("hash-1".to_string()),
        },
        &semantic::NoopSemanticSummaryProvider,
    )
}

#[test]
fn semantic_feature_persistence_schema_includes_stage1_tables() {
    let schema = semantic_features_postgres_schema_sql();
    assert!(schema.contains("CREATE TABLE IF NOT EXISTS symbol_semantics"));
    assert!(schema.contains("CREATE TABLE IF NOT EXISTS symbol_features"));
    assert!(schema.contains("docstring_summary TEXT"));
    assert!(schema.contains("modifiers JSONB"));
}

#[test]
fn semantic_feature_persistence_upgrade_sql_backfills_legacy_doc_comment_summary() {
    let sql = semantic_features_postgres_upgrade_sql();
    assert!(sql.contains("ADD COLUMN IF NOT EXISTS docstring_summary TEXT"));
    assert!(sql.contains("ADD COLUMN IF NOT EXISTS modifiers JSONB"));
    assert!(sql.contains("doc_comment_summary"));
}

#[test]
fn semantic_feature_persistence_builds_get_artefacts_sql_with_escaped_values() {
    let sql = build_semantic_get_artefacts_sql("repo'1", "blob'1", "src/o'brien.ts");
    assert!(sql.contains("repo_id = 'repo''1'"));
    assert!(sql.contains("blob_sha = 'blob''1'"));
    assert!(sql.contains("path = 'src/o''brien.ts'"));
    assert!(sql.contains("signature, modifiers, docstring, content_hash"));
}

#[test]
fn semantic_feature_persistence_builds_get_dependencies_sql_with_escaped_values() {
    let sql = build_semantic_get_dependencies_sql("repo'1", "blob'1", "src/o'brien.ts");
    assert!(sql.contains("repo_id = 'repo''1'"));
    assert!(sql.contains("blob_sha = 'blob''1'"));
    assert!(sql.contains("source.path = 'src/o''brien.ts'"));
    assert!(sql.contains("FROM artefact_edges e"));
}

#[test]
fn semantic_feature_persistence_builds_get_artefacts_by_ids_sql_with_escaped_values() {
    let sql = build_semantic_get_artefacts_by_ids_sql(&[
        "artefact'1".to_string(),
        "artefact-2".to_string(),
    ]);
    assert!(sql.contains("WHERE artefact_id IN ('artefact''1', 'artefact-2')"));
    assert!(sql.contains("ORDER BY repo_id, blob_sha, path"));
}

#[test]
fn semantic_feature_persistence_builds_current_repo_artefacts_sql_without_id_in_clause() {
    let sql = build_current_repo_artefacts_sql("repo'1");
    assert!(sql.contains("FROM artefacts_current current"));
    assert!(sql.contains("LEFT JOIN artefacts a ON a.repo_id = current.repo_id"));
    assert!(sql.contains("WHERE current.repo_id = 'repo''1'"));
    assert!(!sql.contains("WHERE artefact_id IN"));
}

#[test]
fn semantic_feature_persistence_builds_current_repo_artefacts_by_ids_sql_with_escaped_values() {
    let sql = build_current_repo_artefacts_by_ids_sql(
        "repo'1",
        &["artefact'1".to_string(), "artefact-2".to_string()],
    );
    assert!(sql.contains("FROM artefacts_current current"));
    assert!(sql.contains("state.analysis_mode = 'code'"));
    assert!(sql.contains("current.repo_id = 'repo''1'"));
    assert!(sql.contains("current.artefact_id IN ('artefact''1', 'artefact-2')"));
}

#[test]
fn semantic_feature_persistence_builds_conditional_current_persist_sql_with_repo_scoped_target_filters()
 {
    let rows = sample_semantic_rows();
    let sql = build_conditional_current_semantic_persist_rows_sql(
        &rows,
        &semantic::SemanticFeatureInput {
            artefact_id: "historical-1".to_string(),
            symbol_id: Some("historical-symbol".to_string()),
            repo_id: "repo'1".to_string(),
            blob_sha: "content'1".to_string(),
            path: "src/o'brien.ts".to_string(),
            language: "typescript".to_string(),
            canonical_kind: "Method".to_string(),
            language_kind: "method".to_string(),
            symbol_fqn: "src/o'brien.ts::UserService::getById".to_string(),
            name: "getById".to_string(),
            signature: Some("async getById(id: string): Promise<User | null>".to_string()),
            modifiers: vec!["public".to_string(), "async".to_string()],
            body: "return repo.findById(id);".to_string(),
            docstring: Some("Fetches O'Brien by id.".to_string()),
            parent_kind: Some("class".to_string()),
            dependency_signals: vec!["calls:user_repo::find_by_id".to_string()],
            content_hash: Some("hash-1".to_string()),
        },
        RelationalDialect::Sqlite,
    )
    .expect("conditional current persist SQL");

    assert!(sql.contains("FROM artefacts_current current"));
    assert!(sql.contains(
        "JOIN current_file_state state ON state.repo_id = current.repo_id AND state.path = current.path"
    ));
    assert!(sql.contains("current.repo_id = 'repo''1'"));
    assert!(sql.contains("current.path = 'src/o''brien.ts'"));
    assert!(sql.contains("current.content_id = 'content''1'"));
    assert!(sql.contains(
        "LOWER(COALESCE(current.canonical_kind, COALESCE(current.language_kind, 'symbol'))) = 'method'"
    ));
    assert!(sql.contains(
        "COALESCE(current.symbol_fqn, current.path) = 'src/o''brien.ts::UserService::getById'"
    ));
    assert!(sql.contains("state.analysis_mode = 'code'"));
}

#[test]
fn semantic_feature_persistence_parses_index_state_rows_and_defaults() {
    let empty = parse_semantic_index_state_rows(&[]);
    assert_eq!(empty, semantic::SemanticFeatureIndexState::default());

    let rows = vec![json!({
        "semantics_hash": "hash-a",
        "features_hash": "hash-b",
        "semantics_llm_enriched": 1,
    })];
    let parsed = parse_semantic_index_state_rows(&rows);
    assert_eq!(parsed.semantics_hash.as_deref(), Some("hash-a"));
    assert_eq!(parsed.features_hash.as_deref(), Some("hash-b"));
    assert!(parsed.semantics_llm_enriched);
}

#[test]
fn semantic_feature_persistence_builds_postgres_persist_sql() {
    let sql = build_semantic_persist_rows_sql(&sample_semantic_rows(), RelationalDialect::Postgres)
        .expect("persist SQL");
    assert!(sql.contains("INSERT INTO symbol_semantics"));
    assert!(sql.contains("INSERT INTO symbol_features"));
    assert!(sql.contains("docstring_summary"));
    assert!(sql.contains("modifiers"));
    assert!(sql.contains("Fetches O''Brien by id."));
    assert!(sql.contains("::jsonb"));
}

#[test]
fn semantic_feature_persistence_builds_sqlite_persist_sql() {
    let sql = build_semantic_persist_rows_sql(&sample_semantic_rows(), RelationalDialect::Sqlite)
        .expect("persist SQL");
    assert!(sql.contains("INSERT INTO symbol_semantics"));
    assert!(sql.contains("INSERT INTO symbol_features"));
    assert!(sql.contains("modifiers"));
    assert!(sql.contains("generated_at = datetime('now')"));
    assert!(!sql.contains("::jsonb"));
}

#[test]
fn semantic_feature_persistence_builds_summary_snapshot_sql_with_enrichment_markers() {
    let sql = build_semantic_get_summary_sql("artefact'1");
    assert!(sql.contains("semantic_features_input_hash"));
    assert!(sql.contains("summary"));
    assert!(sql.contains("llm_summary"));
    assert!(sql.contains("source_model"));
    assert!(sql.contains("artefact_id = 'artefact''1'"));
}

#[test]
fn semantic_feature_persistence_parses_modifiers_from_json_string_rows() {
    let parsed = parse_semantic_artefact_rows(vec![json!({
        "artefact_id": "artefact-1",
        "symbol_id": "symbol-1",
        "repo_id": "repo-1",
        "blob_sha": "blob-1",
        "path": "src/services/user.ts",
        "language": "typescript",
        "canonical_kind": "method",
        "language_kind": "method",
        "symbol_fqn": "src/services/user.ts::UserService::getById",
        "modifiers": "[\"public\",\"async\"]"
    })])
    .expect("artefact rows should parse");

    assert_eq!(
        parsed[0].modifiers,
        vec!["public".to_string(), "async".to_string()]
    );
}

#[test]
fn semantic_feature_persistence_parses_dependency_rows() {
    let parsed = parse_semantic_dependency_rows(vec![json!({
        "from_artefact_id": "artefact-1",
        "edge_kind": "calls",
        "target_ref": "src/services/user.ts::UserRepo::findById"
    })])
    .expect("dependency rows should parse");

    assert_eq!(parsed.len(), 1);
    assert_eq!(parsed[0].edge_kind, "calls");
}
