//! Stage 1: semantic feature rows (`symbol_semantics`, `symbol_features`) for the semantic_clones pipeline.

use std::collections::{BTreeSet, HashMap};
use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result};
use serde_json::Value;

use crate::capability_packs::semantic_clones::features as semantic;
use crate::host::checkpoints::strategy::manual_commit::run_git;
use crate::host::devql::{
    RelationalDialect, RelationalStorage, esc_pg, postgres_exec, sqlite_exec_path_allow_create,
};

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

pub(crate) async fn load_semantic_feature_inputs_for_artefacts(
    relational: &RelationalStorage,
    repo_root: &Path,
    artefact_ids: &[String],
) -> Result<Vec<semantic::SemanticFeatureInput>> {
    if artefact_ids.is_empty() {
        return Ok(Vec::new());
    }

    let requested_order = artefact_ids
        .iter()
        .enumerate()
        .map(|(index, artefact_id)| (artefact_id.clone(), index))
        .collect::<HashMap<_, _>>();
    let requested_ids = artefact_ids.iter().cloned().collect::<BTreeSet<_>>();

    let target_rows = relational
        .query_rows(&build_semantic_get_artefacts_by_ids_sql(artefact_ids))
        .await?;
    let target_artefacts = parse_semantic_artefact_rows(target_rows)?;
    hydrate_semantic_feature_inputs(
        relational,
        repo_root,
        target_artefacts,
        &requested_ids,
        &requested_order,
    )
    .await
}

pub(crate) async fn load_semantic_feature_inputs_for_current_repo(
    relational: &RelationalStorage,
    repo_root: &Path,
    repo_id: &str,
) -> Result<Vec<semantic::SemanticFeatureInput>> {
    let target_rows = relational
        .query_rows(&build_current_repo_artefacts_sql(repo_id))
        .await?;
    let target_artefacts = parse_semantic_artefact_rows(target_rows)?;
    let requested_ids = target_artefacts
        .iter()
        .map(|row| row.artefact_id.clone())
        .collect::<BTreeSet<_>>();
    let requested_order = target_artefacts
        .iter()
        .enumerate()
        .map(|(index, row)| (row.artefact_id.clone(), index))
        .collect::<HashMap<_, _>>();

    hydrate_semantic_feature_inputs(
        relational,
        repo_root,
        target_artefacts,
        &requested_ids,
        &requested_order,
    )
    .await
}

async fn hydrate_semantic_feature_inputs(
    relational: &RelationalStorage,
    repo_root: &Path,
    target_artefacts: Vec<semantic::PreStageArtefactRow>,
    requested_ids: &BTreeSet<String>,
    requested_order: &HashMap<String, usize>,
) -> Result<Vec<semantic::SemanticFeatureInput>> {
    let groups = target_artefacts
        .iter()
        .map(|row| (row.repo_id.clone(), row.blob_sha.clone(), row.path.clone()))
        .collect::<BTreeSet<_>>();

    let mut hydrated_inputs = Vec::with_capacity(requested_ids.len());
    for (repo_id, blob_sha, path) in groups {
        let artefacts =
            load_pre_stage_artefacts_for_blob(relational, &repo_id, &blob_sha, &path).await?;
        let dependencies =
            load_pre_stage_dependencies_for_blob(relational, &repo_id, &blob_sha, &path).await?;
        let blob_content = load_blob_content_from_git(repo_root, &blob_sha)
            .with_context(|| format!("loading blob `{blob_sha}` for `{path}`"))?;

        hydrated_inputs.extend(
            semantic::build_semantic_feature_inputs_from_artefacts_with_dependencies(
                &artefacts,
                &dependencies,
                &blob_content,
            )
            .into_iter()
            .filter(|input| requested_ids.contains(&input.artefact_id)),
        );
    }

    hydrated_inputs.sort_by_key(|input| {
        requested_order
            .get(&input.artefact_id)
            .copied()
            .unwrap_or(usize::MAX)
    });
    hydrated_inputs.dedup_by(|left, right| left.artefact_id == right.artefact_id);
    Ok(hydrated_inputs)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SemanticSummarySnapshot {
    pub semantic_features_input_hash: String,
    pub summary: String,
    pub llm_summary: Option<String>,
    pub source_model: Option<String>,
}

impl SemanticSummarySnapshot {
    pub(crate) fn is_llm_enriched(&self) -> bool {
        self.llm_summary
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty())
            || self
                .source_model
                .as_deref()
                .is_some_and(|value| !value.trim().is_empty())
    }
}

pub(crate) async fn load_semantic_summary_snapshot(
    relational: &RelationalStorage,
    artefact_id: &str,
) -> Result<Option<SemanticSummarySnapshot>> {
    let rows = relational
        .query_rows(&build_semantic_get_summary_sql(artefact_id))
        .await?;
    let Some(row) = rows.first() else {
        return Ok(None);
    };

    let Some(input_hash) = row
        .get("semantic_features_input_hash")
        .and_then(Value::as_str)
        .map(str::to_string)
    else {
        return Ok(None);
    };
    let Some(summary) = row
        .get("summary")
        .and_then(Value::as_str)
        .map(str::to_string)
    else {
        return Ok(None);
    };
    let llm_summary = row
        .get("llm_summary")
        .and_then(Value::as_str)
        .map(str::to_string);
    let source_model = row
        .get("source_model")
        .and_then(Value::as_str)
        .map(str::to_string);

    Ok(Some(SemanticSummarySnapshot {
        semantic_features_input_hash: input_hash,
        summary,
        llm_summary,
        source_model,
    }))
}

pub(crate) async fn persist_semantic_summary_row(
    relational: &RelationalStorage,
    semantics: &semantic::SymbolSemanticsRow,
    semantic_features_input_hash: &str,
) -> Result<()> {
    relational
        .exec(&build_semantic_persist_summary_sql(
            semantics,
            semantic_features_input_hash,
            relational.dialect(),
        )?)
        .await
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
FROM artefacts_historical \
WHERE repo_id = '{repo_id}' AND blob_sha = '{blob_sha}' AND path = '{path}' \
ORDER BY coalesce(start_byte, 0), coalesce(start_line, 0), artefact_id",
        repo_id = esc_pg(repo_id),
        blob_sha = esc_pg(blob_sha),
        path = esc_pg(path),
    )
}

fn build_current_repo_artefacts_sql(repo_id: &str) -> String {
    format!(
        "SELECT current.artefact_id, current.symbol_id, current.repo_id, current.content_id AS blob_sha, current.path, current.language, \
COALESCE(current.canonical_kind, COALESCE(current.language_kind, 'symbol')) AS canonical_kind, \
COALESCE(current.language_kind, COALESCE(current.canonical_kind, 'symbol')) AS language_kind, \
COALESCE(current.symbol_fqn, current.path) AS symbol_fqn, current.parent_artefact_id, current.start_line, current.end_line, current.start_byte, current.end_byte, current.signature, current.modifiers, current.docstring, a.content_hash \
FROM artefacts_current current \
LEFT JOIN artefacts a ON a.repo_id = current.repo_id AND a.artefact_id = current.artefact_id \
WHERE current.repo_id = '{repo_id}' \
ORDER BY current.path, current.start_line, current.symbol_id, coalesce(current.start_byte, 0), current.artefact_id",
        repo_id = esc_pg(repo_id),
    )
}

fn build_semantic_get_dependencies_sql(repo_id: &str, blob_sha: &str, path: &str) -> String {
    format!(
        "SELECT e.from_artefact_id, LOWER(e.edge_kind) AS edge_kind, \
COALESCE(target.symbol_fqn, target.path, e.to_symbol_ref, e.to_artefact_id, '') AS target_ref \
FROM artefact_edges e \
JOIN artefacts_historical source ON source.repo_id = e.repo_id AND source.artefact_id = e.from_artefact_id AND source.blob_sha = e.blob_sha \
LEFT JOIN artefacts_historical target ON target.repo_id = e.repo_id AND target.artefact_id = e.to_artefact_id AND target.blob_sha = e.blob_sha \
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

fn build_semantic_get_artefacts_by_ids_sql(artefact_ids: &[String]) -> String {
    let artefact_ids = artefact_ids
        .iter()
        .map(|artefact_id| format!("'{}'", esc_pg(artefact_id)))
        .collect::<Vec<_>>()
        .join(", ");

    format!(
        "SELECT artefact_id, symbol_id, repo_id, blob_sha, path, language, \
COALESCE(canonical_kind, COALESCE(language_kind, 'symbol')) AS canonical_kind, \
COALESCE(language_kind, COALESCE(canonical_kind, 'symbol')) AS language_kind, \
COALESCE(symbol_fqn, path) AS symbol_fqn, parent_artefact_id, start_line, end_line, start_byte, end_byte, signature, modifiers, docstring, content_hash \
FROM artefacts_historical \
WHERE artefact_id IN ({artefact_ids}) \
ORDER BY repo_id, blob_sha, path, coalesce(start_byte, 0), coalesce(start_line, 0), artefact_id",
    )
}

fn build_semantic_get_summary_sql(artefact_id: &str) -> String {
    format!(
        "SELECT semantic_features_input_hash, summary, llm_summary, source_model \
FROM symbol_semantics \
WHERE artefact_id = '{artefact_id}'",
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

    let normalized_signature_expr = sql_optional_string(features.normalized_signature.as_deref());
    let parent_kind_expr = sql_optional_string(features.parent_kind.as_deref());
    let modifiers_expr = sql_json_string_for_dialect(&features.modifiers, dialect)?;
    let identifier_tokens_expr = sql_json_string_for_dialect(&features.identifier_tokens, dialect)?;
    let body_tokens_expr = sql_json_string_for_dialect(&features.normalized_body_tokens, dialect)?;
    let context_tokens_expr = sql_json_string_for_dialect(&features.context_tokens, dialect)?;
    let generated_at_sql = semantic_generated_at_now_sql(dialect);

    Ok(format!(
        "{persist_summary_sql}; \
INSERT INTO symbol_features (artefact_id, repo_id, blob_sha, semantic_features_input_hash, normalized_name, normalized_signature, modifiers, identifier_tokens, normalized_body_tokens, parent_kind, context_tokens) \
VALUES ('{features_artefact_id}', '{features_repo_id}', '{features_blob_sha}', '{features_input_hash}', '{normalized_name}', {normalized_signature}, {modifiers}, {identifier_tokens}, {body_tokens}, {parent_kind}, {context_tokens}) \
ON CONFLICT (artefact_id) DO UPDATE SET repo_id = EXCLUDED.repo_id, blob_sha = EXCLUDED.blob_sha, semantic_features_input_hash = EXCLUDED.semantic_features_input_hash, normalized_name = EXCLUDED.normalized_name, normalized_signature = EXCLUDED.normalized_signature, modifiers = EXCLUDED.modifiers, identifier_tokens = EXCLUDED.identifier_tokens, normalized_body_tokens = EXCLUDED.normalized_body_tokens, parent_kind = EXCLUDED.parent_kind, context_tokens = EXCLUDED.context_tokens, generated_at = {generated_at}",
        persist_summary_sql = build_semantic_persist_summary_sql(
            semantics,
            &rows.semantic_features_input_hash,
            dialect,
        )?,
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

fn build_semantic_persist_summary_sql(
    semantics: &semantic::SymbolSemanticsRow,
    semantic_features_input_hash: &str,
    dialect: RelationalDialect,
) -> Result<String> {
    let docstring_summary_expr = sql_optional_string(semantics.docstring_summary.as_deref());
    let llm_summary_expr = sql_optional_string(semantics.llm_summary.as_deref());
    let source_model_expr = sql_optional_string(semantics.source_model.as_deref());
    let generated_at_sql = semantic_generated_at_now_sql(dialect);

    Ok(format!(
        "INSERT INTO symbol_semantics (artefact_id, repo_id, blob_sha, semantic_features_input_hash, docstring_summary, llm_summary, template_summary, summary, confidence, source_model) \
VALUES ('{artefact_id}', '{repo_id}', '{blob_sha}', '{input_hash}', {docstring_summary}, {llm_summary}, '{template_summary}', '{summary}', {confidence:.4}, {source_model}) \
ON CONFLICT (artefact_id) DO UPDATE SET repo_id = EXCLUDED.repo_id, blob_sha = EXCLUDED.blob_sha, semantic_features_input_hash = EXCLUDED.semantic_features_input_hash, docstring_summary = EXCLUDED.docstring_summary, llm_summary = EXCLUDED.llm_summary, template_summary = EXCLUDED.template_summary, summary = EXCLUDED.summary, confidence = EXCLUDED.confidence, source_model = EXCLUDED.source_model, generated_at = {generated_at}",
        artefact_id = esc_pg(&semantics.artefact_id),
        repo_id = esc_pg(&semantics.repo_id),
        blob_sha = esc_pg(&semantics.blob_sha),
        input_hash = esc_pg(semantic_features_input_hash),
        docstring_summary = docstring_summary_expr,
        llm_summary = llm_summary_expr,
        template_summary = esc_pg(&semantics.template_summary),
        summary = esc_pg(&semantics.summary),
        confidence = semantics.confidence,
        source_model = source_model_expr,
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

fn load_blob_content_from_git(repo_root: &Path, blob_sha: &str) -> Result<String> {
    run_git(repo_root, &["cat-file", "-p", blob_sha])
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

    #[test]
    fn semantic_summary_snapshot_marks_llm_enrichment_from_summary_or_model() {
        assert!(
            SemanticSummarySnapshot {
                semantic_features_input_hash: "hash-1".to_string(),
                summary: "Function load user.".to_string(),
                llm_summary: Some("Loads a user by id.".to_string()),
                source_model: None,
            }
            .is_llm_enriched()
        );

        assert!(
            SemanticSummarySnapshot {
                semantic_features_input_hash: "hash-1".to_string(),
                summary: "Function load user.".to_string(),
                llm_summary: None,
                source_model: Some("openai:gpt-test".to_string()),
            }
            .is_llm_enriched()
        );

        assert!(
            !SemanticSummarySnapshot {
                semantic_features_input_hash: "hash-1".to_string(),
                summary: "Function load user.".to_string(),
                llm_summary: None,
                source_model: None,
            }
            .is_llm_enriched()
        );
    }
}
