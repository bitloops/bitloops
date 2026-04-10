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
    Ok(())
}

pub(crate) async fn init_postgres_semantic_clones_schema(pg_client: &Client) -> Result<()> {
    postgres_exec(pg_client, semantic_clones_postgres_schema_sql())
        .await
        .context("creating Postgres semantic clone tables")?;
    Ok(())
}

pub(super) async fn ensure_semantic_clones_schema(relational: &RelationalStorage) -> Result<()> {
    init_sqlite_semantic_clones_schema(relational.sqlite_path()).await?;
    if let Some(remote_client) = relational.remote_client() {
        init_postgres_semantic_clones_schema(remote_client).await?;
    }
    Ok(())
}
