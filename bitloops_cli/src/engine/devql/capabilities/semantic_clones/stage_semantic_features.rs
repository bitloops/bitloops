//! Stage 1: semantic feature rows (`symbol_semantics`, `symbol_features`) for the semantic_clones pipeline.

use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result};
use serde_json::Value;

use crate::engine::devql::{
    RelationalDialect, RelationalStorage, esc_pg, postgres_exec, sqlite_exec_path_allow_create,
};
use crate::engine::semantic_features as semantic;

fn semantic_features_postgres_schema_sql() -> &'static str {
    r#"
CREATE TABLE IF NOT EXISTS symbol_semantics (
    artefact_id TEXT PRIMARY KEY,
    repo_id TEXT NOT NULL,
    blob_sha TEXT NOT NULL,
    semantic_features_input_hash TEXT NOT NULL,
    docstring_summary TEXT,
    llm_summary TEXT,
    template_summary TEXT NOT NULL,
    summary TEXT NOT NULL,
    confidence REAL NOT NULL,
    source_model TEXT,
    generated_at TIMESTAMPTZ DEFAULT now()
);

CREATE INDEX IF NOT EXISTS symbol_semantics_repo_blob_idx
ON symbol_semantics (repo_id, blob_sha);

CREATE TABLE IF NOT EXISTS symbol_features (
    artefact_id TEXT PRIMARY KEY,
    repo_id TEXT NOT NULL,
    blob_sha TEXT NOT NULL,
    semantic_features_input_hash TEXT NOT NULL,
    normalized_name TEXT NOT NULL,
    normalized_signature TEXT,
    modifiers JSONB NOT NULL DEFAULT '[]'::jsonb,
    identifier_tokens JSONB NOT NULL DEFAULT '[]'::jsonb,
    normalized_body_tokens JSONB NOT NULL DEFAULT '[]'::jsonb,
    parent_kind TEXT,
    context_tokens JSONB NOT NULL DEFAULT '[]'::jsonb,
    generated_at TIMESTAMPTZ DEFAULT now()
);

CREATE INDEX IF NOT EXISTS symbol_features_repo_blob_idx
ON symbol_features (repo_id, blob_sha);
"#
}

fn semantic_features_sqlite_schema_sql() -> &'static str {
    r#"
CREATE TABLE IF NOT EXISTS symbol_semantics (
    artefact_id TEXT PRIMARY KEY,
    repo_id TEXT NOT NULL,
    blob_sha TEXT NOT NULL,
    semantic_features_input_hash TEXT NOT NULL,
    docstring_summary TEXT,
    llm_summary TEXT,
    template_summary TEXT NOT NULL,
    summary TEXT NOT NULL,
    confidence REAL NOT NULL,
    source_model TEXT,
    generated_at TEXT DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS symbol_semantics_repo_blob_idx
ON symbol_semantics (repo_id, blob_sha);

CREATE TABLE IF NOT EXISTS symbol_features (
    artefact_id TEXT PRIMARY KEY,
    repo_id TEXT NOT NULL,
    blob_sha TEXT NOT NULL,
    semantic_features_input_hash TEXT NOT NULL,
    normalized_name TEXT NOT NULL,
    normalized_signature TEXT,
    modifiers TEXT NOT NULL DEFAULT '[]',
    identifier_tokens TEXT NOT NULL DEFAULT '[]',
    normalized_body_tokens TEXT NOT NULL DEFAULT '[]',
    parent_kind TEXT,
    context_tokens TEXT NOT NULL DEFAULT '[]',
    generated_at TEXT DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS symbol_features_repo_blob_idx
ON symbol_features (repo_id, blob_sha);
"#
}

fn semantic_features_postgres_upgrade_sql() -> &'static str {
    r#"
ALTER TABLE symbol_semantics ADD COLUMN IF NOT EXISTS docstring_summary TEXT;
ALTER TABLE symbol_features ADD COLUMN IF NOT EXISTS modifiers JSONB DEFAULT '[]'::jsonb;

DO $$
BEGIN
    IF EXISTS (
        SELECT 1
        FROM information_schema.columns
        WHERE table_schema = current_schema()
          AND table_name = 'symbol_semantics'
          AND column_name = 'doc_comment_summary'
    ) THEN
        UPDATE symbol_semantics
        SET docstring_summary = doc_comment_summary
        WHERE docstring_summary IS NULL AND doc_comment_summary IS NOT NULL;
    END IF;
END $$;
"#
}

pub(crate) async fn init_postgres_semantic_features_schema(
    pg_client: &tokio_postgres::Client,
) -> Result<()> {
    postgres_exec(pg_client, semantic_features_postgres_schema_sql()).await?;
    postgres_exec(pg_client, semantic_features_postgres_upgrade_sql()).await
}

pub(crate) async fn init_sqlite_semantic_features_schema(sqlite_path: &Path) -> Result<()> {
    sqlite_exec_path_allow_create(sqlite_path, semantic_features_sqlite_schema_sql()).await?;
    upgrade_sqlite_semantic_features_schema(sqlite_path).await
}

async fn upgrade_sqlite_semantic_features_schema(sqlite_path: &Path) -> Result<()> {
    let db_path = sqlite_path.to_path_buf();
    tokio::task::spawn_blocking(move || -> Result<()> {
        let conn = rusqlite::Connection::open_with_flags(
            &db_path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE | rusqlite::OpenFlags::SQLITE_OPEN_CREATE,
        )
        .with_context(|| format!("opening SQLite database at {}", db_path.display()))?;

        if !sqlite_table_has_column(&conn, "symbol_semantics", "docstring_summary")? {
            conn.execute(
                "ALTER TABLE symbol_semantics ADD COLUMN docstring_summary TEXT",
                [],
            )
            .context("adding symbol_semantics.docstring_summary column")?;
        }

        if sqlite_table_has_column(&conn, "symbol_semantics", "doc_comment_summary")?
            && sqlite_table_has_column(&conn, "symbol_semantics", "docstring_summary")?
        {
            conn.execute(
                "UPDATE symbol_semantics \
SET docstring_summary = doc_comment_summary \
WHERE docstring_summary IS NULL AND doc_comment_summary IS NOT NULL",
                [],
            )
            .context("backfilling legacy symbol_semantics.doc_comment_summary values")?;
        }

        if !sqlite_table_has_column(&conn, "symbol_features", "modifiers")? {
            conn.execute(
                "ALTER TABLE symbol_features ADD COLUMN modifiers TEXT NOT NULL DEFAULT '[]'",
                [],
            )
            .context("adding symbol_features.modifiers column")?;
        }

        Ok(())
    })
    .await
    .context("joining SQLite semantic feature upgrade task")?
}

fn sqlite_table_has_column(
    conn: &rusqlite::Connection,
    table_name: &str,
    column_name: &str,
) -> Result<bool> {
    let mut stmt = conn
        .prepare(&format!("PRAGMA table_info({table_name})"))
        .with_context(|| format!("preparing PRAGMA table_info({table_name})"))?;
    let mut rows = stmt
        .query([])
        .with_context(|| format!("querying PRAGMA table_info({table_name})"))?;
    while let Some(row) = rows
        .next()
        .with_context(|| format!("iterating PRAGMA table_info({table_name})"))?
    {
        let name: String = row
            .get(1)
            .with_context(|| format!("reading column name from PRAGMA table_info({table_name})"))?;
        if name == column_name {
            return Ok(true);
        }
    }
    Ok(false)
}

pub(crate) async fn load_pre_stage_artefacts_for_blob(
    relational: &RelationalStorage,
    repo_id: &str,
    blob_sha: &str,
    path: &str,
) -> Result<Vec<semantic::PreStageArtefactRow>> {
    let rows = relational
        .query_rows(&build_semantic_get_artefacts_sql(repo_id, blob_sha, path))
        .await?;
    parse_semantic_artefact_rows(rows)
}

pub(crate) async fn load_pre_stage_dependencies_for_blob(
    relational: &RelationalStorage,
    repo_id: &str,
    blob_sha: &str,
    path: &str,
) -> Result<Vec<semantic::PreStageDependencyRow>> {
    let rows = relational
        .query_rows(&build_semantic_get_dependencies_sql(
            repo_id, blob_sha, path,
        ))
        .await?;
    parse_semantic_dependency_rows(rows)
}

pub(crate) async fn upsert_semantic_feature_rows(
    relational: &RelationalStorage,
    inputs: &[semantic::SemanticFeatureInput],
    summary_provider: Arc<dyn semantic::SemanticSummaryProvider>,
) -> Result<semantic::SemanticFeatureIngestionStats> {
    let mut stats = semantic::SemanticFeatureIngestionStats::default();

    for input in inputs {
        let next_input_hash =
            semantic::build_semantic_feature_input_hash(input, summary_provider.as_ref());
        let state = load_semantic_index_state(relational, &input.artefact_id).await?;
        if !semantic::semantic_features_require_reindex(&state, &next_input_hash) {
            stats.skipped += 1;
            continue;
        }

        let input = input.clone();
        let summary_provider = Arc::clone(&summary_provider);
        let rows = tokio::task::spawn_blocking(move || {
            semantic::build_semantic_feature_rows(&input, summary_provider.as_ref())
        })
        .await
        .context("building semantic feature rows on blocking worker")?;
        persist_semantic_feature_rows(relational, &rows).await?;
        stats.upserted += 1;
    }

    Ok(stats)
}

async fn load_semantic_index_state(
    relational: &RelationalStorage,
    artefact_id: &str,
) -> Result<semantic::SemanticFeatureIndexState> {
    let rows = relational
        .query_rows(&build_semantic_get_index_state_sql(artefact_id))
        .await?;
    Ok(parse_semantic_index_state_rows(&rows))
}

async fn persist_semantic_feature_rows(
    relational: &RelationalStorage,
    rows: &semantic::SemanticFeatureRows,
) -> Result<()> {
    relational
        .exec(&build_semantic_persist_rows_sql(
            rows,
            relational.dialect(),
        )?)
        .await
}

fn build_semantic_get_artefacts_sql(repo_id: &str, blob_sha: &str, path: &str) -> String {
    format!(
        "SELECT artefact_id, symbol_id, repo_id, blob_sha, path, language, \
COALESCE(canonical_kind, COALESCE(language_kind, 'symbol')) AS canonical_kind, \
COALESCE(language_kind, COALESCE(canonical_kind, 'symbol')) AS language_kind, \
COALESCE(symbol_fqn, path) AS symbol_fqn, parent_artefact_id, start_line, end_line, start_byte, end_byte, signature, modifiers, docstring, content_hash \
FROM artefacts \
WHERE repo_id = '{repo_id}' AND blob_sha = '{blob_sha}' AND path = '{path}' \
ORDER BY coalesce(start_byte, 0), coalesce(start_line, 0), artefact_id",
        repo_id = esc_pg(repo_id),
        blob_sha = esc_pg(blob_sha),
        path = esc_pg(path),
    )
}

fn build_semantic_get_dependencies_sql(repo_id: &str, blob_sha: &str, path: &str) -> String {
    format!(
        "SELECT e.from_artefact_id, LOWER(e.edge_kind) AS edge_kind, \
COALESCE(target.symbol_fqn, target.path, e.to_symbol_ref, e.to_artefact_id, '') AS target_ref \
FROM artefact_edges e \
JOIN artefacts source ON source.repo_id = e.repo_id AND source.artefact_id = e.from_artefact_id \
LEFT JOIN artefacts target ON target.repo_id = e.repo_id AND target.artefact_id = e.to_artefact_id \
WHERE e.repo_id = '{repo_id}' AND e.blob_sha = '{blob_sha}' AND source.path = '{path}' \
ORDER BY e.from_artefact_id, e.edge_kind, target_ref",
        repo_id = esc_pg(repo_id),
        blob_sha = esc_pg(blob_sha),
        path = esc_pg(path),
    )
}

fn parse_semantic_artefact_rows(rows: Vec<Value>) -> Result<Vec<semantic::PreStageArtefactRow>> {
    let mut artefacts = Vec::with_capacity(rows.len());
    for row in rows {
        let modifiers = parse_semantic_json_string_array(row.get("modifiers"));
        let mut normalized_row = row;
        if let Value::Object(ref mut object) = normalized_row {
            object.insert(
                "modifiers".to_string(),
                Value::Array(modifiers.iter().cloned().map(Value::String).collect()),
            );
        }
        let mut artefact = serde_json::from_value::<semantic::PreStageArtefactRow>(normalized_row)?;
        artefact.modifiers = modifiers;
        artefacts.push(artefact);
    }
    Ok(artefacts)
}

fn parse_semantic_dependency_rows(
    rows: Vec<Value>,
) -> Result<Vec<semantic::PreStageDependencyRow>> {
    let mut dependencies = Vec::with_capacity(rows.len());
    for row in rows {
        let dependency = serde_json::from_value::<semantic::PreStageDependencyRow>(row)?;
        if dependency.target_ref.trim().is_empty() {
            continue;
        }
        dependencies.push(dependency);
    }
    Ok(dependencies)
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

fn semantic_generated_at_now_sql(dialect: RelationalDialect) -> &'static str {
    match dialect {
        RelationalDialect::Postgres => "now()",
        RelationalDialect::Sqlite => "datetime('now')",
    }
}

fn build_semantic_persist_rows_sql(
    rows: &semantic::SemanticFeatureRows,
    dialect: RelationalDialect,
) -> Result<String> {
    let semantics = &rows.semantics;
    let features = &rows.features;

    let docstring_summary_expr = sql_optional_string(semantics.docstring_summary.as_deref());
    let llm_summary_expr = sql_optional_string(semantics.llm_summary.as_deref());
    let source_model_expr = sql_optional_string(semantics.source_model.as_deref());
    let normalized_signature_expr = sql_optional_string(features.normalized_signature.as_deref());
    let parent_kind_expr = sql_optional_string(features.parent_kind.as_deref());
    let modifiers_expr = sql_json_string_for_dialect(&features.modifiers, dialect)?;
    let identifier_tokens_expr = sql_json_string_for_dialect(&features.identifier_tokens, dialect)?;
    let body_tokens_expr = sql_json_string_for_dialect(&features.normalized_body_tokens, dialect)?;
    let context_tokens_expr = sql_json_string_for_dialect(&features.context_tokens, dialect)?;
    let generated_at_sql = semantic_generated_at_now_sql(dialect);

    Ok(format!(
        "INSERT INTO symbol_semantics (artefact_id, repo_id, blob_sha, semantic_features_input_hash, docstring_summary, llm_summary, template_summary, summary, confidence, source_model) \
VALUES ('{artefact_id}', '{repo_id}', '{blob_sha}', '{input_hash}', {docstring_summary}, {llm_summary}, '{template_summary}', '{summary}', {confidence:.4}, {source_model}) \
ON CONFLICT (artefact_id) DO UPDATE SET repo_id = EXCLUDED.repo_id, blob_sha = EXCLUDED.blob_sha, semantic_features_input_hash = EXCLUDED.semantic_features_input_hash, docstring_summary = EXCLUDED.docstring_summary, llm_summary = EXCLUDED.llm_summary, template_summary = EXCLUDED.template_summary, summary = EXCLUDED.summary, confidence = EXCLUDED.confidence, source_model = EXCLUDED.source_model, generated_at = {generated_at}; \
INSERT INTO symbol_features (artefact_id, repo_id, blob_sha, semantic_features_input_hash, normalized_name, normalized_signature, modifiers, identifier_tokens, normalized_body_tokens, parent_kind, context_tokens) \
VALUES ('{features_artefact_id}', '{features_repo_id}', '{features_blob_sha}', '{features_input_hash}', '{normalized_name}', {normalized_signature}, {modifiers}, {identifier_tokens}, {body_tokens}, {parent_kind}, {context_tokens}) \
ON CONFLICT (artefact_id) DO UPDATE SET repo_id = EXCLUDED.repo_id, blob_sha = EXCLUDED.blob_sha, semantic_features_input_hash = EXCLUDED.semantic_features_input_hash, normalized_name = EXCLUDED.normalized_name, normalized_signature = EXCLUDED.normalized_signature, modifiers = EXCLUDED.modifiers, identifier_tokens = EXCLUDED.identifier_tokens, normalized_body_tokens = EXCLUDED.normalized_body_tokens, parent_kind = EXCLUDED.parent_kind, context_tokens = EXCLUDED.context_tokens, generated_at = {generated_at}",
        artefact_id = esc_pg(&semantics.artefact_id),
        repo_id = esc_pg(&semantics.repo_id),
        blob_sha = esc_pg(&semantics.blob_sha),
        input_hash = esc_pg(&rows.semantic_features_input_hash),
        docstring_summary = docstring_summary_expr,
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
        modifiers = modifiers_expr,
        identifier_tokens = identifier_tokens_expr,
        body_tokens = body_tokens_expr,
        parent_kind = parent_kind_expr,
        context_tokens = context_tokens_expr,
        generated_at = generated_at_sql,
    ))
}

fn sql_string(value: &str) -> String {
    format!("'{}'", esc_pg(value))
}

fn sql_optional_string(value: Option<&str>) -> String {
    value.map(sql_string).unwrap_or_else(|| "NULL".to_string())
}

fn sql_json_string_for_dialect<T: serde::Serialize>(
    value: &T,
    dialect: RelationalDialect,
) -> Result<String> {
    let json = esc_pg(&serde_json::to_string(value)?);
    Ok(match dialect {
        RelationalDialect::Postgres => format!("'{json}'::jsonb"),
        RelationalDialect::Sqlite => format!("'{json}'"),
    })
}

fn parse_semantic_json_string_array(value: Option<&Value>) -> Vec<String> {
    match value {
        Some(Value::Array(values)) => values
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect(),
        Some(Value::String(raw)) => serde_json::from_str::<Vec<String>>(raw).unwrap_or_default(),
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod semantic_feature_persistence_tests {
    use super::*;
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
    fn semantic_feature_persistence_parses_index_state_rows_and_defaults() {
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
    fn semantic_feature_persistence_builds_postgres_persist_sql() {
        let sql =
            build_semantic_persist_rows_sql(&sample_semantic_rows(), RelationalDialect::Postgres)
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
        let sql =
            build_semantic_persist_rows_sql(&sample_semantic_rows(), RelationalDialect::Sqlite)
                .expect("persist SQL");
        assert!(sql.contains("INSERT INTO symbol_semantics"));
        assert!(sql.contains("INSERT INTO symbol_features"));
        assert!(sql.contains("modifiers"));
        assert!(sql.contains("generated_at = datetime('now')"));
        assert!(!sql.contains("::jsonb"));
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
}
