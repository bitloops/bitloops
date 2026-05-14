use std::path::Path;

use anyhow::{Context, Result};
use tokio_postgres::Client;

use crate::capability_packs::semantic_clones::schema::{
    semantic_clones_postgres_schema_sql, semantic_clones_sqlite_schema_sql,
};
use crate::host::devql::{RelationalStorage, postgres_exec, sqlite_exec_path_allow_create};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum CloneProjection {
    Historical,
    Current,
}

impl CloneProjection {
    pub(super) fn artefacts_table(self) -> &'static str {
        match self {
            Self::Historical => "artefacts_historical",
            Self::Current => "artefacts_current",
        }
    }

    pub(super) fn semantics_table(self) -> &'static str {
        match self {
            Self::Historical => "symbol_semantics",
            Self::Current => "symbol_semantics_current",
        }
    }

    pub(super) fn features_table(self) -> &'static str {
        match self {
            Self::Historical => "symbol_features",
            Self::Current => "symbol_features_current",
        }
    }

    pub(super) fn embeddings_table(self) -> &'static str {
        match self {
            Self::Historical => "symbol_embeddings",
            Self::Current => "symbol_embeddings_current",
        }
    }

    pub(super) fn clone_edges_table(self) -> &'static str {
        match self {
            Self::Historical => "symbol_clone_edges",
            Self::Current => "symbol_clone_edges_current",
        }
    }

    pub(super) fn dependency_edges_table(self) -> &'static str {
        match self {
            Self::Historical => "artefact_edges",
            Self::Current => "artefact_edges_current",
        }
    }

    pub(super) fn dependency_source_symbol_expr(self) -> &'static str {
        match self {
            Self::Historical => "source.symbol_id",
            Self::Current => "e.from_symbol_id",
        }
    }

    pub(super) fn dependency_source_join(self) -> &'static str {
        match self {
            Self::Historical => {
                "JOIN artefacts_historical source \
ON source.repo_id = e.repo_id AND source.artefact_id = e.from_artefact_id AND source.blob_sha = e.blob_sha"
            }
            Self::Current => "",
        }
    }

    pub(super) fn dependency_target_join(self) -> &'static str {
        match self {
            Self::Historical => {
                "LEFT JOIN artefacts_historical target \
ON target.repo_id = e.repo_id AND target.artefact_id = e.to_artefact_id AND target.blob_sha = e.blob_sha"
            }
            Self::Current => {
                "LEFT JOIN artefacts_current target \
ON target.repo_id = e.repo_id AND target.artefact_id = e.to_artefact_id"
            }
        }
    }

    pub(super) fn dependency_target_ref_expr(self) -> &'static str {
        match self {
            Self::Historical => {
                "COALESCE(target.symbol_fqn, target.path, e.to_symbol_ref, e.to_artefact_id, '')"
            }
            Self::Current => {
                "COALESCE(target.symbol_fqn, target.path, e.to_symbol_ref, e.to_symbol_id, '')"
            }
        }
    }

    pub(super) fn blob_column(self) -> &'static str {
        match self {
            Self::Historical => "blob_sha",
            Self::Current => "content_id",
        }
    }
}

async fn init_sqlite_semantic_clones_schema(sqlite_path: &Path) -> Result<()> {
    sqlite_exec_path_allow_create(sqlite_path, semantic_clones_sqlite_schema_sql())
        .await
        .context("creating SQLite semantic clone tables")?;
    upgrade_sqlite_semantic_clones_schema(sqlite_path).await
}

pub(crate) async fn init_postgres_semantic_clones_schema(pg_client: &Client) -> Result<()> {
    postgres_exec(pg_client, semantic_clones_postgres_schema_sql())
        .await
        .context("creating Postgres semantic clone tables")?;
    postgres_exec(
        pg_client,
        "ALTER TABLE symbol_clone_edges DROP CONSTRAINT IF EXISTS symbol_clone_edges_pkey;",
    )
    .await
    .context("dropping legacy Postgres symbol_clone_edges primary key")?;
    postgres_exec(
        pg_client,
        "ALTER TABLE symbol_clone_edges ADD CONSTRAINT symbol_clone_edges_pkey PRIMARY KEY (repo_id, source_artefact_id, target_artefact_id);",
    )
    .await
    .context("upgrading Postgres symbol_clone_edges primary key")?;
    Ok(())
}

pub(super) async fn ensure_semantic_clones_schema(relational: &RelationalStorage) -> Result<()> {
    crate::host::devql::ensure_sqlite_schema_once(
        relational.sqlite_path(),
        "semantic_clones_sqlite",
        |sqlite_path| async move { init_sqlite_semantic_clones_schema(&sqlite_path).await },
    )
    .await?;
    if let Some(remote_client) = relational.remote_client() {
        crate::host::devql::ensure_sqlite_schema_once(
            relational.sqlite_path(),
            "semantic_clones_postgres",
            |_| async move { init_postgres_semantic_clones_schema(remote_client).await },
        )
        .await?;
    }
    Ok(())
}

async fn upgrade_sqlite_semantic_clones_schema(sqlite_path: &Path) -> Result<()> {
    let db_path = sqlite_path.to_path_buf();
    tokio::task::spawn_blocking(move || -> Result<()> {
        crate::storage::sqlite::with_sqlite_write_lock(&db_path, || {
            let conn = rusqlite::Connection::open(&db_path)
                .with_context(|| format!("opening SQLite database at {}", db_path.display()))?;
            conn.busy_timeout(std::time::Duration::from_secs(30))
                .context("setting SQLite busy timeout for semantic clone schema upgrade")?;
            conn.execute_batch(
                "PRAGMA foreign_keys = ON; PRAGMA journal_mode = WAL; PRAGMA synchronous = NORMAL;",
            )
            .context("configuring SQLite semantic clone schema connection")?;

            let current_pk = sqlite_table_primary_key_columns(&conn, "symbol_clone_edges")?;
            let expected_pk = vec![
                "repo_id".to_string(),
                "source_artefact_id".to_string(),
                "target_artefact_id".to_string(),
            ];
            if current_pk != expected_pk {
                migrate_sqlite_historical_clone_edges_table(&conn)?;
            }

            Ok(())
        })
    })
    .await
    .context("running SQLite semantic clone schema upgrade on blocking worker")?
}

fn migrate_sqlite_historical_clone_edges_table(conn: &rusqlite::Connection) -> Result<()> {
    conn.execute(
        "ALTER TABLE symbol_clone_edges RENAME TO symbol_clone_edges_legacy",
        [],
    )
    .context("renaming legacy symbol_clone_edges table")?;
    conn.execute_batch(semantic_clones_sqlite_schema_sql())
        .context("creating upgraded SQLite semantic clone tables")?;
    conn.execute(
        "INSERT OR REPLACE INTO symbol_clone_edges (
            repo_id,
            source_symbol_id,
            source_artefact_id,
            target_symbol_id,
            target_artefact_id,
            relation_kind,
            score,
            semantic_score,
            lexical_score,
            structural_score,
            clone_input_hash,
            explanation_json,
            generated_at
        )
        SELECT
            repo_id,
            source_symbol_id,
            source_artefact_id,
            target_symbol_id,
            target_artefact_id,
            relation_kind,
            score,
            semantic_score,
            lexical_score,
            structural_score,
            clone_input_hash,
            explanation_json,
            generated_at
        FROM symbol_clone_edges_legacy
        ORDER BY generated_at, rowid",
        [],
    )
    .context("copying legacy symbol_clone_edges rows into upgraded table")?;
    conn.execute("DROP TABLE symbol_clone_edges_legacy", [])
        .context("dropping legacy symbol_clone_edges table")?;
    Ok(())
}

fn sqlite_table_primary_key_columns(
    conn: &rusqlite::Connection,
    table_name: &str,
) -> Result<Vec<String>> {
    let mut stmt = conn
        .prepare(&format!("PRAGMA table_info({table_name})"))
        .with_context(|| format!("preparing PRAGMA table_info({table_name})"))?;
    let columns = stmt
        .query_map([], |row| {
            Ok((row.get::<_, String>(1)?, row.get::<_, i64>(5)?))
        })
        .with_context(|| format!("querying PRAGMA table_info({table_name})"))?
        .collect::<std::result::Result<Vec<_>, _>>()
        .with_context(|| format!("iterating PRAGMA table_info({table_name})"))?;

    let mut primary_key_columns = columns
        .into_iter()
        .filter(|(_, position)| *position > 0)
        .collect::<Vec<_>>();
    primary_key_columns.sort_by_key(|(_, position)| *position);
    Ok(primary_key_columns
        .into_iter()
        .map(|(name, _)| name)
        .collect())
}
