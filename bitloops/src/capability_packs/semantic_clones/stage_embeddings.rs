//! Stage 2: symbol embedding rows (`symbol_embeddings`) for the semantic_clones pipeline.

use std::collections::{BTreeSet, HashMap};
use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use serde_json::Value;

use crate::adapters::model_providers::embeddings::EmbeddingProvider;
use crate::capability_packs::semantic_clones::embeddings;
use crate::capability_packs::semantic_clones::features as semantic;
use crate::host::devql::{
    RelationalStorage, esc_pg, postgres_exec, sql_string_list_pg, sqlite_exec_path_allow_create,
};

fn semantic_embeddings_postgres_schema_sql() -> &'static str {
    r#"
CREATE EXTENSION IF NOT EXISTS vector;

CREATE TABLE IF NOT EXISTS symbol_embeddings (
    artefact_id TEXT NOT NULL,
    repo_id TEXT NOT NULL,
    blob_sha TEXT NOT NULL,
    representation_kind TEXT NOT NULL DEFAULT 'baseline',
    provider TEXT NOT NULL,
    model TEXT NOT NULL,
    dimension INTEGER NOT NULL CHECK (dimension > 0),
    embedding_input_hash TEXT NOT NULL,
    embedding vector NOT NULL,
    generated_at TIMESTAMPTZ DEFAULT now(),
    PRIMARY KEY (artefact_id, representation_kind)
);

CREATE INDEX IF NOT EXISTS symbol_embeddings_repo_artefact_idx
ON symbol_embeddings (repo_id, artefact_id, representation_kind);

CREATE INDEX IF NOT EXISTS symbol_embeddings_repo_model_idx
ON symbol_embeddings (repo_id, representation_kind, provider, model, dimension, blob_sha);

CREATE TABLE IF NOT EXISTS symbol_embeddings_current (
    artefact_id TEXT NOT NULL,
    repo_id TEXT NOT NULL,
    path TEXT NOT NULL,
    content_id TEXT NOT NULL,
    symbol_id TEXT,
    representation_kind TEXT NOT NULL DEFAULT 'baseline',
    provider TEXT NOT NULL,
    model TEXT NOT NULL,
    dimension INTEGER NOT NULL CHECK (dimension > 0),
    embedding_input_hash TEXT NOT NULL,
    embedding vector NOT NULL,
    generated_at TIMESTAMPTZ DEFAULT now(),
    PRIMARY KEY (artefact_id, representation_kind)
);

CREATE INDEX IF NOT EXISTS symbol_embeddings_current_repo_path_idx
ON symbol_embeddings_current (repo_id, path);

CREATE UNIQUE INDEX IF NOT EXISTS symbol_embeddings_current_repo_artefact_idx
ON symbol_embeddings_current (repo_id, artefact_id, representation_kind);
CREATE TABLE IF NOT EXISTS semantic_clone_embedding_setup_state (
    repo_id TEXT PRIMARY KEY,
    representation_kind TEXT NOT NULL DEFAULT 'baseline',
    provider TEXT NOT NULL,
    model TEXT NOT NULL,
    dimension INTEGER NOT NULL CHECK (dimension > 0),
    setup_fingerprint TEXT NOT NULL,
    updated_at TIMESTAMPTZ DEFAULT now()
);
"#
}

fn semantic_embeddings_sqlite_schema_sql() -> &'static str {
    r#"
CREATE TABLE IF NOT EXISTS symbol_embeddings (
    artefact_id TEXT NOT NULL,
    repo_id TEXT NOT NULL,
    blob_sha TEXT NOT NULL,
    representation_kind TEXT NOT NULL DEFAULT 'baseline',
    provider TEXT NOT NULL,
    model TEXT NOT NULL,
    dimension INTEGER NOT NULL CHECK (dimension > 0),
    embedding_input_hash TEXT NOT NULL,
    embedding TEXT NOT NULL,
    generated_at DATETIME DEFAULT CURRENT_TIMESTAMP,
    PRIMARY KEY (artefact_id, representation_kind)
);

CREATE INDEX IF NOT EXISTS symbol_embeddings_repo_artefact_idx
ON symbol_embeddings (repo_id, artefact_id, representation_kind);

CREATE INDEX IF NOT EXISTS symbol_embeddings_repo_model_idx
ON symbol_embeddings (repo_id, representation_kind, provider, model, dimension, blob_sha);

CREATE TABLE IF NOT EXISTS symbol_embeddings_current (
    artefact_id TEXT NOT NULL,
    repo_id TEXT NOT NULL,
    path TEXT NOT NULL,
    content_id TEXT NOT NULL,
    symbol_id TEXT,
    representation_kind TEXT NOT NULL DEFAULT 'baseline',
    provider TEXT NOT NULL,
    model TEXT NOT NULL,
    dimension INTEGER NOT NULL CHECK (dimension > 0),
    embedding_input_hash TEXT NOT NULL,
    embedding TEXT NOT NULL,
    generated_at DATETIME DEFAULT CURRENT_TIMESTAMP,
    PRIMARY KEY (artefact_id, representation_kind)
);

CREATE INDEX IF NOT EXISTS symbol_embeddings_current_repo_path_idx
ON symbol_embeddings_current (repo_id, path);

CREATE UNIQUE INDEX IF NOT EXISTS symbol_embeddings_current_repo_artefact_idx
ON symbol_embeddings_current (repo_id, artefact_id, representation_kind);
CREATE TABLE IF NOT EXISTS semantic_clone_embedding_setup_state (
    repo_id TEXT PRIMARY KEY,
    representation_kind TEXT NOT NULL DEFAULT 'baseline',
    provider TEXT NOT NULL,
    model TEXT NOT NULL,
    dimension INTEGER NOT NULL CHECK (dimension > 0),
    setup_fingerprint TEXT NOT NULL,
    updated_at DATETIME DEFAULT CURRENT_TIMESTAMP
);
"#
}

fn semantic_embeddings_postgres_upgrade_sql() -> &'static str {
    r#"
ALTER TABLE symbol_embeddings
    ADD COLUMN IF NOT EXISTS representation_kind TEXT NOT NULL DEFAULT 'baseline';
ALTER TABLE symbol_embeddings_current
    ADD COLUMN IF NOT EXISTS representation_kind TEXT NOT NULL DEFAULT 'baseline';
ALTER TABLE semantic_clone_embedding_setup_state
    ADD COLUMN IF NOT EXISTS representation_kind TEXT NOT NULL DEFAULT 'baseline';

DROP INDEX IF EXISTS symbol_embeddings_repo_artefact_idx;
CREATE INDEX IF NOT EXISTS symbol_embeddings_repo_artefact_idx
ON symbol_embeddings (repo_id, artefact_id, representation_kind);

DROP INDEX IF EXISTS symbol_embeddings_repo_model_idx;
CREATE INDEX IF NOT EXISTS symbol_embeddings_repo_model_idx
ON symbol_embeddings (repo_id, representation_kind, provider, model, dimension, blob_sha);

DROP INDEX IF EXISTS symbol_embeddings_current_repo_artefact_idx;
CREATE UNIQUE INDEX IF NOT EXISTS symbol_embeddings_current_repo_artefact_idx
ON symbol_embeddings_current (repo_id, artefact_id, representation_kind);

DO $$
BEGIN
    IF EXISTS (
        SELECT 1
        FROM pg_constraint
        WHERE conrelid = 'symbol_embeddings'::regclass
          AND conname = 'symbol_embeddings_pkey'
    ) THEN
        ALTER TABLE symbol_embeddings DROP CONSTRAINT symbol_embeddings_pkey;
    END IF;
EXCEPTION WHEN undefined_table THEN
    NULL;
END $$;

DO $$
BEGIN
    ALTER TABLE symbol_embeddings
        ADD CONSTRAINT symbol_embeddings_pkey PRIMARY KEY (artefact_id, representation_kind);
EXCEPTION WHEN duplicate_table THEN
    NULL;
WHEN duplicate_object THEN
    NULL;
END $$;

DO $$
BEGIN
    IF EXISTS (
        SELECT 1
        FROM pg_constraint
        WHERE conrelid = 'symbol_embeddings_current'::regclass
          AND conname = 'symbol_embeddings_current_pkey'
    ) THEN
        ALTER TABLE symbol_embeddings_current DROP CONSTRAINT symbol_embeddings_current_pkey;
    END IF;
EXCEPTION WHEN undefined_table THEN
    NULL;
END $$;

DO $$
BEGIN
    ALTER TABLE symbol_embeddings_current
        ADD CONSTRAINT symbol_embeddings_current_pkey PRIMARY KEY (artefact_id, representation_kind);
EXCEPTION WHEN duplicate_table THEN
    NULL;
WHEN duplicate_object THEN
    NULL;
END $$;
"#
}

async fn upgrade_sqlite_semantic_embeddings_schema(sqlite_path: &Path) -> Result<()> {
    let db_path = sqlite_path.to_path_buf();
    tokio::task::spawn_blocking(move || -> Result<()> {
        let conn = rusqlite::Connection::open(&db_path)
            .with_context(|| format!("opening SQLite database at {}", db_path.display()))?;

        if sqlite_table_has_column(&conn, "symbol_embeddings", "artefact_id")?
            && !sqlite_table_has_column(&conn, "symbol_embeddings", "representation_kind")?
        {
            conn.execute(
                "ALTER TABLE symbol_embeddings RENAME TO symbol_embeddings_legacy",
                [],
            )
            .context("renaming legacy symbol_embeddings table")?;
            conn.execute_batch(semantic_embeddings_sqlite_schema_sql())
                .context("creating upgraded semantic embedding tables")?;
            conn.execute(
                "INSERT INTO symbol_embeddings (
                    artefact_id,
                    repo_id,
                    blob_sha,
                    representation_kind,
                    provider,
                    model,
                    dimension,
                    embedding_input_hash,
                    embedding,
                    generated_at
                )
                SELECT
                    artefact_id,
                    repo_id,
                    blob_sha,
                    'baseline',
                    provider,
                    model,
                    dimension,
                    embedding_input_hash,
                    embedding,
                    generated_at
                FROM symbol_embeddings_legacy",
                [],
            )
            .context("copying legacy symbol_embeddings rows into upgraded table")?;
            conn.execute("DROP TABLE symbol_embeddings_legacy", [])
                .context("dropping legacy symbol_embeddings table")?;
        }

        if sqlite_table_has_column(&conn, "symbol_embeddings_current", "artefact_id")?
            && !sqlite_table_has_column(&conn, "symbol_embeddings_current", "representation_kind")?
        {
            conn.execute(
                "ALTER TABLE symbol_embeddings_current RENAME TO symbol_embeddings_current_legacy",
                [],
            )
            .context("renaming legacy symbol_embeddings_current table")?;
            conn.execute_batch(semantic_embeddings_sqlite_schema_sql())
                .context("creating upgraded current semantic embedding tables")?;
            conn.execute(
                "INSERT INTO symbol_embeddings_current (
                    artefact_id,
                    repo_id,
                    path,
                    content_id,
                    symbol_id,
                    representation_kind,
                    provider,
                    model,
                    dimension,
                    embedding_input_hash,
                    embedding,
                    generated_at
                )
                SELECT
                    artefact_id,
                    repo_id,
                    path,
                    content_id,
                    symbol_id,
                    'baseline',
                    provider,
                    model,
                    dimension,
                    embedding_input_hash,
                    embedding,
                    generated_at
                FROM symbol_embeddings_current_legacy",
                [],
            )
            .context("copying legacy symbol_embeddings_current rows into upgraded table")?;
            conn.execute("DROP TABLE symbol_embeddings_current_legacy", [])
                .context("dropping legacy symbol_embeddings_current table")?;
        }

        if sqlite_table_has_column(&conn, "semantic_clone_embedding_setup_state", "repo_id")?
            && !sqlite_table_has_column(
                &conn,
                "semantic_clone_embedding_setup_state",
                "representation_kind",
            )?
        {
            conn.execute(
                "ALTER TABLE semantic_clone_embedding_setup_state
                 ADD COLUMN representation_kind TEXT NOT NULL DEFAULT 'baseline'",
                [],
            )
            .context("adding representation_kind to semantic_clone_embedding_setup_state")?;
        }

        Ok(())
    })
    .await
    .context("joining SQLite semantic embedding upgrade task")?
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RepoEmbeddingSyncAction {
    Incremental,
    AdoptExisting,
    RefreshCurrentRepo,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct CurrentRepoEmbeddingRefreshResult {
    pub embedding_stats: embeddings::SymbolEmbeddingIngestionStats,
    pub clone_build: crate::capability_packs::semantic_clones::scoring::SymbolCloneBuildResult,
}

pub(crate) async fn init_sqlite_semantic_embeddings_schema(sqlite_path: &Path) -> Result<()> {
    sqlite_exec_path_allow_create(sqlite_path, semantic_embeddings_sqlite_schema_sql())
        .await
        .context("creating SQLite semantic embedding tables")?;
    upgrade_sqlite_semantic_embeddings_schema(sqlite_path).await?;
    Ok(())
}

pub(crate) async fn init_postgres_semantic_embeddings_schema(
    pg_client: &tokio_postgres::Client,
) -> Result<()> {
    postgres_exec(pg_client, semantic_embeddings_postgres_schema_sql())
        .await
        .context("creating Postgres semantic embedding tables")?;
    postgres_exec(pg_client, semantic_embeddings_postgres_upgrade_sql())
        .await
        .context("upgrading Postgres semantic embedding tables")?;
    Ok(())
}

pub(crate) async fn upsert_symbol_embedding_rows(
    relational: &RelationalStorage,
    inputs: &[semantic::SemanticFeatureInput],
    representation_kind: embeddings::EmbeddingRepresentationKind,
    embedding_provider: Arc<dyn EmbeddingProvider>,
) -> Result<embeddings::SymbolEmbeddingIngestionStats> {
    let mut stats = embeddings::SymbolEmbeddingIngestionStats::default();
    if inputs.is_empty() {
        return Ok(stats);
    }

    ensure_semantic_embeddings_schema(relational).await?;

    let artefact_ids = inputs
        .iter()
        .map(|input| input.artefact_id.clone())
        .collect::<Vec<_>>();
    let summary_by_artefact_id =
        load_semantic_summary_map(relational, &artefact_ids, representation_kind).await?;
    let embedding_inputs = embeddings::build_symbol_embedding_inputs(
        inputs,
        representation_kind,
        &summary_by_artefact_id,
    );
    stats.eligible = embedding_inputs.len();

    for input in embedding_inputs {
        let next_input_hash =
            embeddings::build_symbol_embedding_input_hash(&input, embedding_provider.as_ref());
        let state = load_symbol_embedding_index_state(
            relational,
            &input.artefact_id,
            input.representation_kind,
        )
        .await?;
        if !embeddings::symbol_embeddings_require_reindex(&state, &next_input_hash) {
            stats.skipped += 1;
            continue;
        }

        let input = input.clone();
        let embedding_provider = Arc::clone(&embedding_provider);
        let row = tokio::task::spawn_blocking(move || {
            embeddings::build_symbol_embedding_row(&input, embedding_provider.as_ref())
        })
        .await
        .context("building semantic embedding row on blocking worker")??;
        persist_symbol_embedding_row(relational, &row).await?;
        stats.upserted += 1;
    }

    Ok(stats)
}

#[allow(dead_code)]
pub(crate) async fn upsert_current_symbol_embedding_rows(
    relational: &RelationalStorage,
    path: &str,
    content_id: &str,
    inputs: &[semantic::SemanticFeatureInput],
    representation_kind: embeddings::EmbeddingRepresentationKind,
    embedding_provider: Arc<dyn EmbeddingProvider>,
) -> Result<embeddings::SymbolEmbeddingIngestionStats> {
    let mut stats = embeddings::SymbolEmbeddingIngestionStats::default();
    let Some(first) = inputs.first() else {
        return Ok(stats);
    };

    ensure_semantic_embeddings_schema(relational).await?;
    let setup = embeddings::resolve_embedding_setup(embedding_provider.as_ref())?;

    let artefact_ids = inputs
        .iter()
        .map(|input| input.artefact_id.clone())
        .collect::<Vec<_>>();
    let summary_by_artefact_id =
        load_current_semantic_summary_map(relational, &artefact_ids, representation_kind).await?;
    let input_by_artefact_id = inputs
        .iter()
        .map(|input| (input.artefact_id.clone(), input))
        .collect::<HashMap<_, _>>();
    let embedding_inputs = embeddings::build_symbol_embedding_inputs(
        inputs,
        representation_kind,
        &summary_by_artefact_id,
    );
    stats.eligible = embedding_inputs.len();
    delete_stale_current_symbol_embedding_rows_for_path(
        relational,
        &first.repo_id,
        path,
        representation_kind,
        &setup,
        &embedding_inputs
            .iter()
            .map(|input| input.artefact_id.clone())
            .collect::<Vec<_>>(),
    )
    .await?;

    for input in embedding_inputs {
        let input_metadata = input_by_artefact_id
            .get(&input.artefact_id)
            .copied()
            .ok_or_else(|| {
                anyhow::anyhow!("missing current semantic input for `{}`", input.artefact_id)
            })?;
        let next_input_hash =
            embeddings::build_symbol_embedding_input_hash(&input, embedding_provider.as_ref());
        let state = load_current_symbol_embedding_index_state(
            relational,
            &input.artefact_id,
            input.representation_kind,
        )
        .await?;
        if !embeddings::symbol_embeddings_require_reindex(&state, &next_input_hash) {
            stats.skipped += 1;
            continue;
        }
        let input = input.clone();
        let embedding_provider = Arc::clone(&embedding_provider);
        let row = tokio::task::spawn_blocking(move || {
            embeddings::build_symbol_embedding_row(&input, embedding_provider.as_ref())
        })
        .await
        .context("building current semantic embedding row on blocking worker")??;
        persist_current_symbol_embedding_row(relational, input_metadata, path, content_id, &row)
            .await?;
        stats.upserted += 1;
    }

    Ok(stats)
}

pub(crate) async fn ensure_semantic_embeddings_schema(
    relational: &RelationalStorage,
) -> Result<()> {
    init_sqlite_semantic_embeddings_schema(relational.sqlite_path()).await?;
    if let Some(remote_client) = relational.remote_client() {
        init_postgres_semantic_embeddings_schema(remote_client).await?;
    }
    Ok(())
}

pub(crate) async fn clear_repo_symbol_embedding_rows(
    relational: &RelationalStorage,
    repo_id: &str,
) -> Result<()> {
    ensure_semantic_embeddings_schema(relational).await?;
    relational
        .exec_batch_transactional(&[
            format!(
                "DELETE FROM symbol_embeddings WHERE repo_id = '{}'",
                esc_pg(repo_id),
            ),
            format!(
                "DELETE FROM symbol_embeddings_current WHERE repo_id = '{}'",
                esc_pg(repo_id),
            ),
        ])
        .await
}

#[allow(dead_code)]
pub(crate) async fn clear_current_symbol_embedding_rows_for_path(
    relational: &RelationalStorage,
    repo_id: &str,
    path: &str,
) -> Result<()> {
    ensure_semantic_embeddings_schema(relational).await?;
    let sql = format!(
        "DELETE FROM symbol_embeddings_current WHERE repo_id = '{}' AND path = '{}'",
        esc_pg(repo_id),
        esc_pg(path),
    );
    relational.exec(&sql).await
}

pub(crate) async fn clear_repo_active_embedding_setup(
    relational: &RelationalStorage,
    repo_id: &str,
) -> Result<()> {
    ensure_semantic_embeddings_schema(relational).await?;
    let sql = format!(
        "DELETE FROM semantic_clone_embedding_setup_state WHERE repo_id = '{}'",
        esc_pg(repo_id),
    );
    relational.exec(&sql).await
}

pub(crate) async fn load_active_embedding_setup(
    relational: &RelationalStorage,
    repo_id: &str,
) -> Result<Option<embeddings::ActiveEmbeddingRepresentationState>> {
    ensure_semantic_embeddings_schema(relational).await?;
    let rows = relational
        .query_rows(&build_active_embedding_setup_lookup_sql(repo_id))
        .await?;
    Ok(parse_active_embedding_state_rows(&rows).into_iter().next())
}

pub(crate) async fn persist_active_embedding_setup(
    relational: &RelationalStorage,
    repo_id: &str,
    active_state: &embeddings::ActiveEmbeddingRepresentationState,
) -> Result<()> {
    ensure_semantic_embeddings_schema(relational).await?;
    let sql = build_active_embedding_setup_persist_sql(repo_id, active_state);
    relational.exec(&sql).await
}

pub(crate) async fn determine_repo_embedding_sync_action(
    relational: &RelationalStorage,
    repo_id: &str,
    representation_kind: embeddings::EmbeddingRepresentationKind,
    setup: &embeddings::EmbeddingSetup,
) -> Result<RepoEmbeddingSyncAction> {
    if let Some(active) = load_active_embedding_setup(relational, repo_id).await? {
        return Ok(
            if active.representation_kind == representation_kind && active.setup == *setup {
                RepoEmbeddingSyncAction::Incremental
            } else {
                RepoEmbeddingSyncAction::RefreshCurrentRepo
            },
        );
    }

    let current_states =
        load_current_repo_embedding_states(relational, repo_id, Some(representation_kind)).await?;
    Ok(
        if current_states.len() == 1 && current_states[0].setup == *setup {
            RepoEmbeddingSyncAction::AdoptExisting
        } else {
            RepoEmbeddingSyncAction::RefreshCurrentRepo
        },
    )
}

pub(crate) async fn refresh_current_repo_symbol_embeddings_and_clone_edges(
    relational: &RelationalStorage,
    repo_root: &Path,
    repo_id: &str,
    representation_kind: embeddings::EmbeddingRepresentationKind,
    embedding_provider: Arc<dyn EmbeddingProvider>,
) -> Result<CurrentRepoEmbeddingRefreshResult> {
    ensure_semantic_embeddings_schema(relational).await?;
    let setup = embeddings::resolve_embedding_setup(embedding_provider.as_ref())?;
    let current_inputs =
        super::load_semantic_feature_inputs_for_current_repo(relational, repo_root, repo_id)
            .await?;
    let embedding_stats = upsert_symbol_embedding_rows(
        relational,
        &current_inputs,
        representation_kind,
        embedding_provider,
    )
    .await?;
    if embedding_stats.eligible == 0 {
        return Ok(CurrentRepoEmbeddingRefreshResult {
            embedding_stats,
            clone_build: Default::default(),
        });
    }
    persist_active_embedding_setup(
        relational,
        repo_id,
        &embeddings::ActiveEmbeddingRepresentationState::new(representation_kind, setup),
    )
    .await?;
    let clone_build =
        crate::capability_packs::semantic_clones::pipeline::rebuild_symbol_clone_edges(
            relational, repo_id,
        )
        .await?;

    Ok(CurrentRepoEmbeddingRefreshResult {
        embedding_stats,
        clone_build,
    })
}

async fn load_symbol_embedding_index_state(
    relational: &RelationalStorage,
    artefact_id: &str,
    representation_kind: embeddings::EmbeddingRepresentationKind,
) -> Result<embeddings::SymbolEmbeddingIndexState> {
    let rows = relational
        .query_rows(&build_symbol_embedding_index_state_sql(
            artefact_id,
            "symbol_embeddings",
            representation_kind,
        ))
        .await?;
    Ok(parse_symbol_embedding_index_state_rows(&rows))
}

async fn load_current_symbol_embedding_index_state(
    relational: &RelationalStorage,
    artefact_id: &str,
    representation_kind: embeddings::EmbeddingRepresentationKind,
) -> Result<embeddings::SymbolEmbeddingIndexState> {
    let rows = relational
        .query_rows(&build_symbol_embedding_index_state_sql(
            artefact_id,
            "symbol_embeddings_current",
            representation_kind,
        ))
        .await?;
    Ok(parse_symbol_embedding_index_state_rows(&rows))
}

async fn load_semantic_summary_map(
    relational: &RelationalStorage,
    artefact_ids: &[String],
    representation_kind: embeddings::EmbeddingRepresentationKind,
) -> Result<HashMap<String, String>> {
    load_semantic_summary_map_from_table(
        relational,
        artefact_ids,
        "symbol_semantics",
        representation_kind,
    )
    .await
}

#[allow(dead_code)]
async fn load_current_semantic_summary_map(
    relational: &RelationalStorage,
    artefact_ids: &[String],
    representation_kind: embeddings::EmbeddingRepresentationKind,
) -> Result<HashMap<String, String>> {
    load_semantic_summary_map_from_table(
        relational,
        artefact_ids,
        "symbol_semantics_current",
        representation_kind,
    )
    .await
}

async fn load_semantic_summary_map_from_table(
    relational: &RelationalStorage,
    artefact_ids: &[String],
    table: &str,
    representation_kind: embeddings::EmbeddingRepresentationKind,
) -> Result<HashMap<String, String>> {
    if artefact_ids.is_empty() {
        return Ok(HashMap::new());
    }

    let rows = relational
        .query_rows(&build_semantic_summary_lookup_sql(artefact_ids, table))
        .await?;
    let mut out = HashMap::with_capacity(rows.len());
    for row in rows {
        let Some(artefact_id) = row.get("artefact_id").and_then(Value::as_str) else {
            continue;
        };
        if let Some(summary) = resolve_embedding_summary(&row, representation_kind) {
            out.insert(artefact_id.to_string(), summary);
        }
    }
    Ok(out)
}

async fn persist_symbol_embedding_row(
    relational: &RelationalStorage,
    row: &embeddings::SymbolEmbeddingRow,
) -> Result<()> {
    let sql = build_sqlite_symbol_embedding_persist_sql(row)?;
    relational.exec(&sql).await
}

#[allow(dead_code)]
async fn persist_current_symbol_embedding_row(
    relational: &RelationalStorage,
    input: &semantic::SemanticFeatureInput,
    path: &str,
    content_id: &str,
    row: &embeddings::SymbolEmbeddingRow,
) -> Result<()> {
    let sql = build_current_symbol_embedding_persist_sql(input, path, content_id, row)?;
    relational.exec(&sql).await
}

fn build_symbol_embedding_index_state_sql(
    artefact_id: &str,
    table: &str,
    representation_kind: embeddings::EmbeddingRepresentationKind,
) -> String {
    format!(
        "SELECT embedding_input_hash AS embedding_hash \
FROM {table} \
WHERE artefact_id = '{artefact_id}' AND representation_kind = '{representation_kind}'",
        table = table,
        artefact_id = esc_pg(artefact_id),
        representation_kind = esc_pg(&representation_kind.to_string()),
    )
}

async fn delete_stale_current_symbol_embedding_rows_for_path(
    relational: &RelationalStorage,
    repo_id: &str,
    path: &str,
    representation_kind: embeddings::EmbeddingRepresentationKind,
    setup: &embeddings::EmbeddingSetup,
    keep_artefact_ids: &[String],
) -> Result<()> {
    let extra_delete_clause = if keep_artefact_ids.is_empty() {
        " OR 1 = 1".to_string()
    } else {
        format!(
            " OR artefact_id NOT IN ({})",
            sql_string_list_pg(keep_artefact_ids)
        )
    };
    let sql = format!(
        "DELETE FROM symbol_embeddings_current \
WHERE repo_id = '{repo_id}' AND path = '{path}' AND representation_kind = '{representation_kind}' \
  AND (provider <> '{provider}' OR model <> '{model}' OR dimension <> {dimension}{extra_delete_clause})",
        repo_id = esc_pg(repo_id),
        path = esc_pg(path),
        representation_kind = esc_pg(&representation_kind.to_string()),
        provider = esc_pg(&setup.provider),
        model = esc_pg(&setup.model),
        dimension = setup.dimension,
        extra_delete_clause = extra_delete_clause,
    );
    relational.exec(&sql).await
}

fn build_active_embedding_setup_lookup_sql(repo_id: &str) -> String {
    format!(
        "SELECT representation_kind, provider, model, dimension \
FROM semantic_clone_embedding_setup_state \
WHERE repo_id = '{}'",
        esc_pg(repo_id),
    )
}

fn build_current_repo_embedding_states_sql(
    repo_id: &str,
    representation_kind: Option<embeddings::EmbeddingRepresentationKind>,
) -> String {
    let representation_filter = representation_kind
        .map(|kind| {
            format!(
                "AND e.representation_kind = '{}'",
                esc_pg(&kind.to_string())
            )
        })
        .unwrap_or_default();
    format!(
        "SELECT representation_kind, provider, model, dimension \
FROM ( \
    SELECT e.representation_kind AS representation_kind, e.provider AS provider, e.model AS model, e.dimension AS dimension \
    FROM artefacts_current a \
    JOIN symbol_embeddings_current e ON e.repo_id = a.repo_id AND e.artefact_id = a.artefact_id \
    WHERE a.repo_id = '{repo_id}' {representation_filter} \
    UNION \
    SELECT e.representation_kind AS representation_kind, e.provider AS provider, e.model AS model, e.dimension AS dimension \
    FROM artefacts_current a \
    JOIN symbol_embeddings e ON e.repo_id = a.repo_id AND e.artefact_id = a.artefact_id \
    WHERE a.repo_id = '{repo_id}' {representation_filter} \
) setups \
ORDER BY representation_kind, provider, model, dimension",
        repo_id = esc_pg(repo_id),
        representation_filter = representation_filter,
    )
}

fn build_active_embedding_setup_persist_sql(
    repo_id: &str,
    active_state: &embeddings::ActiveEmbeddingRepresentationState,
) -> String {
    let setup = &active_state.setup;
    format!(
        "INSERT INTO semantic_clone_embedding_setup_state (repo_id, representation_kind, provider, model, dimension, setup_fingerprint) \
VALUES ('{repo_id}', '{representation_kind}', '{provider}', '{model}', {dimension}, '{setup_fingerprint}') \
ON CONFLICT (repo_id) DO UPDATE SET representation_kind = excluded.representation_kind, provider = excluded.provider, model = excluded.model, dimension = excluded.dimension, setup_fingerprint = excluded.setup_fingerprint, updated_at = CURRENT_TIMESTAMP",
        repo_id = esc_pg(repo_id),
        representation_kind = esc_pg(&active_state.representation_kind.to_string()),
        provider = esc_pg(&setup.provider),
        model = esc_pg(&setup.model),
        dimension = setup.dimension,
        setup_fingerprint = esc_pg(&setup.setup_fingerprint),
    )
}

fn parse_symbol_embedding_index_state_rows(
    rows: &[Value],
) -> embeddings::SymbolEmbeddingIndexState {
    let Some(row) = rows.first() else {
        return embeddings::SymbolEmbeddingIndexState::default();
    };

    embeddings::SymbolEmbeddingIndexState {
        embedding_hash: row
            .get("embedding_hash")
            .and_then(Value::as_str)
            .map(str::to_string),
    }
}

fn parse_active_embedding_state_rows(
    rows: &[Value],
) -> Vec<embeddings::ActiveEmbeddingRepresentationState> {
    let mut states = BTreeSet::new();
    for row in rows {
        let Some(representation_kind) = row
            .get("representation_kind")
            .and_then(Value::as_str)
            .and_then(parse_representation_kind)
        else {
            continue;
        };
        let Some(provider) = row.get("provider").and_then(Value::as_str) else {
            continue;
        };
        let Some(model) = row.get("model").and_then(Value::as_str) else {
            continue;
        };
        let Some(dimension) = row
            .get("dimension")
            .and_then(value_as_positive_usize)
            .filter(|value| *value > 0)
        else {
            continue;
        };
        states.insert((
            representation_kind,
            provider.to_string(),
            model.to_string(),
            dimension,
        ));
    }

    states
        .into_iter()
        .map(|(representation_kind, provider, model, dimension)| {
            embeddings::ActiveEmbeddingRepresentationState::new(
                representation_kind,
                embeddings::EmbeddingSetup::new(provider, model, dimension),
            )
        })
        .collect()
}

fn value_as_positive_usize(value: &Value) -> Option<usize> {
    if let Some(value) = value.as_u64() {
        return usize::try_from(value).ok();
    }
    if let Some(value) = value.as_i64() {
        return usize::try_from(value).ok();
    }
    value.as_str()?.trim().parse::<usize>().ok()
}

fn parse_representation_kind(raw: &str) -> Option<embeddings::EmbeddingRepresentationKind> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "baseline" => Some(embeddings::EmbeddingRepresentationKind::Baseline),
        "enriched" => Some(embeddings::EmbeddingRepresentationKind::Enriched),
        _ => None,
    }
}

fn build_semantic_summary_lookup_sql(artefact_ids: &[String], table: &str) -> String {
    format!(
        "SELECT artefact_id, docstring_summary, llm_summary, template_summary, summary, source_model \
FROM {table} \
WHERE artefact_id IN ({})",
        sql_string_list_pg(artefact_ids),
        table = table,
    )
}

fn resolve_embedding_summary(
    row: &Value,
    representation_kind: embeddings::EmbeddingRepresentationKind,
) -> Option<String> {
    let template_summary = row
        .get("template_summary")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())?;
    let docstring_summary = row
        .get("docstring_summary")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let canonical_summary = row
        .get("summary")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let llm_summary = row
        .get("llm_summary")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let has_llm_enrichment = llm_summary.is_some()
        || row
            .get("source_model")
            .and_then(Value::as_str)
            .is_some_and(|value| !value.trim().is_empty());

    match representation_kind {
        embeddings::EmbeddingRepresentationKind::Baseline => Some(
            semantic::synthesize_deterministic_summary(template_summary, docstring_summary),
        ),
        embeddings::EmbeddingRepresentationKind::Enriched if has_llm_enrichment => {
            canonical_summary.map(str::to_string)
        }
        embeddings::EmbeddingRepresentationKind::Enriched => None,
    }
}

pub(crate) async fn load_current_repo_embedding_states(
    relational: &RelationalStorage,
    repo_id: &str,
    representation_kind: Option<embeddings::EmbeddingRepresentationKind>,
) -> Result<Vec<embeddings::ActiveEmbeddingRepresentationState>> {
    let rows = relational
        .query_rows(&build_current_repo_embedding_states_sql(
            repo_id,
            representation_kind,
        ))
        .await?;
    Ok(parse_active_embedding_state_rows(&rows))
}

#[cfg(test)]
fn build_postgres_symbol_embedding_persist_sql(
    row: &embeddings::SymbolEmbeddingRow,
) -> Result<String> {
    let embedding_expr = sql_vector_string(&row.embedding)?;
    Ok(format!(
        "INSERT INTO symbol_embeddings (artefact_id, repo_id, blob_sha, representation_kind, provider, model, dimension, embedding_input_hash, embedding) \
VALUES ('{artefact_id}', '{repo_id}', '{blob_sha}', '{representation_kind}', '{provider}', '{model}', {dimension}, '{embedding_input_hash}', {embedding}) \
ON CONFLICT (artefact_id, representation_kind) DO UPDATE SET repo_id = EXCLUDED.repo_id, blob_sha = EXCLUDED.blob_sha, provider = EXCLUDED.provider, model = EXCLUDED.model, dimension = EXCLUDED.dimension, embedding_input_hash = EXCLUDED.embedding_input_hash, embedding = EXCLUDED.embedding, generated_at = now()",
        artefact_id = esc_pg(&row.artefact_id),
        repo_id = esc_pg(&row.repo_id),
        blob_sha = esc_pg(&row.blob_sha),
        representation_kind = esc_pg(&row.representation_kind.to_string()),
        provider = esc_pg(&row.provider),
        model = esc_pg(&row.model),
        dimension = row.dimension,
        embedding_input_hash = esc_pg(&row.embedding_input_hash),
        embedding = embedding_expr,
    ))
}

fn build_sqlite_symbol_embedding_persist_sql(
    row: &embeddings::SymbolEmbeddingRow,
) -> Result<String> {
    let embedding_json = sql_json_string(&row.embedding)?;
    Ok(format!(
        "INSERT INTO symbol_embeddings (artefact_id, repo_id, blob_sha, representation_kind, provider, model, dimension, embedding_input_hash, embedding) \
VALUES ('{artefact_id}', '{repo_id}', '{blob_sha}', '{representation_kind}', '{provider}', '{model}', {dimension}, '{embedding_input_hash}', '{embedding}') \
ON CONFLICT (artefact_id, representation_kind) DO UPDATE SET repo_id = excluded.repo_id, blob_sha = excluded.blob_sha, provider = excluded.provider, model = excluded.model, dimension = excluded.dimension, embedding_input_hash = excluded.embedding_input_hash, embedding = excluded.embedding, generated_at = CURRENT_TIMESTAMP",
        artefact_id = esc_pg(&row.artefact_id),
        repo_id = esc_pg(&row.repo_id),
        blob_sha = esc_pg(&row.blob_sha),
        representation_kind = esc_pg(&row.representation_kind.to_string()),
        provider = esc_pg(&row.provider),
        model = esc_pg(&row.model),
        dimension = row.dimension,
        embedding_input_hash = esc_pg(&row.embedding_input_hash),
        embedding = embedding_json,
    ))
}

#[allow(dead_code)]
fn build_current_symbol_embedding_persist_sql(
    input: &semantic::SemanticFeatureInput,
    path: &str,
    content_id: &str,
    row: &embeddings::SymbolEmbeddingRow,
) -> Result<String> {
    let embedding_json = sql_json_string(&row.embedding)?;
    let symbol_id_sql = input
        .symbol_id
        .as_deref()
        .map(|value| format!("'{}'", esc_pg(value)))
        .unwrap_or_else(|| "NULL".to_string());
    Ok(format!(
        "INSERT INTO symbol_embeddings_current (artefact_id, repo_id, path, content_id, symbol_id, representation_kind, provider, model, dimension, embedding_input_hash, embedding) \
VALUES ('{artefact_id}', '{repo_id}', '{path}', '{content_id}', {symbol_id}, '{representation_kind}', '{provider}', '{model}', {dimension}, '{embedding_input_hash}', '{embedding}') \
ON CONFLICT (artefact_id, representation_kind) DO UPDATE SET repo_id = excluded.repo_id, path = excluded.path, content_id = excluded.content_id, symbol_id = excluded.symbol_id, provider = excluded.provider, model = excluded.model, dimension = excluded.dimension, embedding_input_hash = excluded.embedding_input_hash, embedding = excluded.embedding, generated_at = CURRENT_TIMESTAMP",
        artefact_id = esc_pg(&row.artefact_id),
        repo_id = esc_pg(&row.repo_id),
        path = esc_pg(path),
        content_id = esc_pg(content_id),
        symbol_id = symbol_id_sql,
        representation_kind = esc_pg(&row.representation_kind.to_string()),
        provider = esc_pg(&row.provider),
        model = esc_pg(&row.model),
        dimension = row.dimension,
        embedding_input_hash = esc_pg(&row.embedding_input_hash),
        embedding = embedding_json,
    ))
}

#[cfg(test)]
fn sql_vector_string(values: &[f32]) -> Result<String> {
    let json = sql_json_string(values)?;
    Ok(format!("'{json}'::vector"))
}

fn sql_json_string(values: &[f32]) -> Result<String> {
    if values.is_empty() {
        bail!("cannot persist empty embedding vector");
    }

    for value in values {
        if !value.is_finite() {
            bail!("cannot persist embedding vector containing non-finite values");
        }
    }

    Ok(esc_pg(&serde_json::to_string(values)?))
}

#[cfg(test)]
mod semantic_embedding_persistence_tests {
    use super::*;
    use crate::adapters::model_providers::embeddings::{EmbeddingInputType, EmbeddingProvider};
    use crate::host::devql::sqlite_query_rows_path;
    use serde_json::json;
    use tempfile::tempdir;

    struct TestEmbeddingProvider;

    impl EmbeddingProvider for TestEmbeddingProvider {
        fn provider_name(&self) -> &str {
            "local_fastembed"
        }

        fn model_name(&self) -> &str {
            "jinaai/jina-embeddings-v2-base-code"
        }

        fn output_dimension(&self) -> Option<usize> {
            Some(3)
        }

        fn cache_key(&self) -> String {
            "provider=local_fastembed:model=jinaai/jina-embeddings-v2-base-code".to_string()
        }

        fn embed(&self, input: &str, _input_type: EmbeddingInputType) -> Result<Vec<f32>> {
            Ok(vec![input.len() as f32, 0.5, 0.25])
        }
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
            "{}\nCREATE TABLE artefacts_current (repo_id TEXT NOT NULL, artefact_id TEXT PRIMARY KEY, path TEXT, start_line INTEGER, symbol_id TEXT);",
            semantic_embeddings_sqlite_schema_sql()
        ))
        .await
    }

    #[test]
    fn semantic_embedding_schema_includes_vector_table() {
        let schema = semantic_embeddings_postgres_schema_sql();
        assert!(schema.contains("CREATE EXTENSION IF NOT EXISTS vector"));
        assert!(schema.contains("CREATE TABLE IF NOT EXISTS symbol_embeddings"));
        assert!(schema.contains("embedding vector"));
    }

    #[test]
    fn semantic_embedding_sqlite_schema_uses_text_storage() {
        let schema = semantic_embeddings_sqlite_schema_sql();
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
            representation_kind: embeddings::EmbeddingRepresentationKind::Baseline,
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
            representation_kind: embeddings::EmbeddingRepresentationKind::Baseline,
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

        let invalid_err =
            sql_json_string(&[0.1, f32::NAN]).expect_err("non-finite vectors must fail");
        assert!(invalid_err.to_string().contains("non-finite values"));
    }

    #[test]
    fn semantic_embedding_index_state_sql_filters_by_artefact_id() {
        let sql = build_symbol_embedding_index_state_sql(
            "artefact-'1",
            "symbol_embeddings",
            embeddings::EmbeddingRepresentationKind::Baseline,
        );
        assert!(sql.contains("FROM symbol_embeddings"));
        assert!(sql.contains("WHERE artefact_id = 'artefact-''1'"));
        assert!(sql.contains("representation_kind = 'baseline'"));
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
        let relational = sqlite_relational_with_schema(
            "CREATE TABLE symbol_embeddings (
                artefact_id TEXT NOT NULL,
                repo_id TEXT NOT NULL,
                representation_kind TEXT NOT NULL,
                provider TEXT NOT NULL,
                model TEXT NOT NULL,
                dimension INTEGER NOT NULL,
                embedding_input_hash TEXT NOT NULL,
                PRIMARY KEY (artefact_id, representation_kind)
            );
            INSERT INTO symbol_embeddings (
                artefact_id, repo_id, representation_kind, provider, model, dimension, embedding_input_hash
            ) VALUES (
                'artefact-1', 'repo-1', 'baseline', 'voyage', 'voyage-code-3', 1024, 'hash-1'
            );",
        )
        .await;

        let state = load_symbol_embedding_index_state(
            &relational,
            "artefact-1",
            embeddings::EmbeddingRepresentationKind::Baseline,
        )
        .await
        .expect("load embedding state");

        assert_eq!(state.embedding_hash.as_deref(), Some("hash-1"));
    }

    #[tokio::test]
    async fn current_embedding_upsert_reuses_matching_rows_and_keeps_enriched_variant() {
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
            semantic_embeddings_sqlite_schema_sql()
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
        let provider: Arc<dyn EmbeddingProvider> = Arc::new(TestEmbeddingProvider);

        let baseline_first = upsert_current_symbol_embedding_rows(
            &relational,
            "src/a.ts",
            "blob-1",
            &inputs,
            embeddings::EmbeddingRepresentationKind::Baseline,
            Arc::clone(&provider),
        )
        .await
        .expect("upsert baseline current embeddings");
        let baseline_second = upsert_current_symbol_embedding_rows(
            &relational,
            "src/a.ts",
            "blob-1",
            &inputs,
            embeddings::EmbeddingRepresentationKind::Baseline,
            Arc::clone(&provider),
        )
        .await
        .expect("reuse baseline current embeddings");
        let enriched = upsert_current_symbol_embedding_rows(
            &relational,
            "src/a.ts",
            "blob-1",
            &inputs,
            embeddings::EmbeddingRepresentationKind::Enriched,
            provider,
        )
        .await
        .expect("upsert enriched current embeddings");

        assert_eq!(baseline_first.upserted, 2);
        assert_eq!(baseline_second.skipped, 2);
        assert_eq!(enriched.eligible, 1);
        assert_eq!(enriched.upserted, 1);

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
                ("artefact-1".to_string(), "baseline".to_string()),
                ("artefact-1".to_string(), "enriched".to_string()),
                ("artefact-2".to_string(), "baseline".to_string()),
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
            embeddings::EmbeddingRepresentationKind::Baseline,
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
        assert!(!summary_map.contains_key("artefact-2"));
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
        relational
            .exec(
                "INSERT INTO artefacts_current (repo_id, artefact_id, path, start_line, symbol_id)
                 VALUES ('repo-1', 'artefact-1', 'src/a.ts', 1, 'sym-1')",
            )
            .await
            .expect("insert current artefact");
        relational
            .exec(
                "INSERT INTO symbol_embeddings (artefact_id, repo_id, blob_sha, representation_kind, provider, model, dimension, embedding_input_hash, embedding)
                 VALUES ('artefact-1', 'repo-1', 'blob-1', 'baseline', 'local_fastembed', 'jinaai/jina-embeddings-v2-base-code', 3, 'hash-1', '[0.1,0.2,0.3]')",
            )
            .await
            .expect("insert embedding row");

        let action = determine_repo_embedding_sync_action(
            &relational,
            "repo-1",
            embeddings::EmbeddingRepresentationKind::Baseline,
            &embeddings::EmbeddingSetup::new(
                "local_fastembed",
                "jinaai/jina-embeddings-v2-base-code",
                3,
            ),
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
                embeddings::EmbeddingRepresentationKind::Baseline,
                embeddings::EmbeddingSetup::new(
                    "local_fastembed",
                    "jinaai/jina-embeddings-v2-base-code",
                    3,
                ),
            ),
        )
        .await
        .expect("persist active setup");

        let action = determine_repo_embedding_sync_action(
            &relational,
            "repo-1",
            embeddings::EmbeddingRepresentationKind::Baseline,
            &embeddings::EmbeddingSetup::new("voyage", "voyage-code-3", 1024),
        )
        .await
        .expect("sync action");

        assert_eq!(action, RepoEmbeddingSyncAction::RefreshCurrentRepo);
    }
}
