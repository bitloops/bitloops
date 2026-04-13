use anyhow::{Context, Result};

use super::introspection::{sqlite_table_exists, sqlite_table_has_column, sqlite_table_pk_columns};

pub(super) fn migrate_artefacts_current_branch_scope(conn: &rusqlite::Connection) -> Result<()> {
    if !sqlite_table_exists(conn, "artefacts_current")? {
        return Ok(());
    }

    if artefacts_current_matches_sync_shape(conn)? {
        return Ok(());
    }
    let has_branch = sqlite_table_has_column(conn, "artefacts_current", "branch")?;
    let pk_columns = sqlite_table_pk_columns(conn, "artefacts_current")?;
    let needs_rebuild = !has_branch
        || pk_columns
            != [
                "repo_id".to_string(),
                "branch".to_string(),
                "symbol_id".to_string(),
            ];

    if needs_rebuild {
        let can_copy_existing_rows = sqlite_table_has_columns(
            conn,
            "artefacts_current",
            &[
                "repo_id",
                "symbol_id",
                "artefact_id",
                "commit_sha",
                "revision_kind",
                "revision_id",
                "temp_checkpoint_id",
                "blob_sha",
                "path",
                "language",
                "canonical_kind",
                "language_kind",
                "symbol_fqn",
                "parent_symbol_id",
                "parent_artefact_id",
                "start_line",
                "end_line",
                "start_byte",
                "end_byte",
                "signature",
                "modifiers",
                "docstring",
                "content_hash",
                "updated_at",
            ],
        )?;
        if can_copy_existing_rows {
            rebuild_artefacts_current_with_copy(conn)?;
        } else {
            rebuild_artefacts_current_without_copy(conn)?;
        }
    }

    conn.execute_batch(
        r#"
DROP INDEX IF EXISTS artefacts_current_path_idx;
DROP INDEX IF EXISTS artefacts_current_kind_idx;
DROP INDEX IF EXISTS artefacts_current_symbol_fqn_idx;
DROP INDEX IF EXISTS artefacts_current_branch_path_idx;
DROP INDEX IF EXISTS artefacts_current_branch_kind_idx;
DROP INDEX IF EXISTS artefacts_current_branch_fqn_idx;
DROP INDEX IF EXISTS artefacts_current_artefact_idx;

CREATE INDEX IF NOT EXISTS artefacts_current_branch_path_idx
ON artefacts_current (repo_id, branch, path);

CREATE INDEX IF NOT EXISTS artefacts_current_branch_kind_idx
ON artefacts_current (repo_id, branch, canonical_kind);

CREATE INDEX IF NOT EXISTS artefacts_current_artefact_idx
ON artefacts_current (repo_id, branch, artefact_id);

CREATE INDEX IF NOT EXISTS artefacts_current_branch_fqn_idx
ON artefacts_current (repo_id, branch, symbol_fqn);
"#,
    )
    .context("ensuring branch-aware artefacts_current indexes")?;

    Ok(())
}

pub(super) fn migrate_artefact_edges_current_branch_scope(
    conn: &rusqlite::Connection,
) -> Result<()> {
    if !sqlite_table_exists(conn, "artefact_edges_current")? {
        return Ok(());
    }

    if artefact_edges_current_matches_sync_shape(conn)? {
        return Ok(());
    }
    let has_branch = sqlite_table_has_column(conn, "artefact_edges_current", "branch")?;
    let pk_columns = sqlite_table_pk_columns(conn, "artefact_edges_current")?;
    let needs_rebuild = !has_branch
        || pk_columns
            != [
                "repo_id".to_string(),
                "branch".to_string(),
                "edge_id".to_string(),
            ];

    if needs_rebuild {
        let can_copy_existing_rows = sqlite_table_has_columns(
            conn,
            "artefact_edges_current",
            &[
                "edge_id",
                "repo_id",
                "commit_sha",
                "revision_kind",
                "revision_id",
                "temp_checkpoint_id",
                "blob_sha",
                "path",
                "from_symbol_id",
                "from_artefact_id",
                "to_symbol_id",
                "to_artefact_id",
                "to_symbol_ref",
                "edge_kind",
                "language",
                "start_line",
                "end_line",
                "metadata",
                "updated_at",
            ],
        )?;
        if can_copy_existing_rows {
            rebuild_artefact_edges_current_with_copy(conn)?;
        } else {
            rebuild_artefact_edges_current_without_copy(conn)?;
        }
    }

    conn.execute_batch(
        r#"
DROP INDEX IF EXISTS artefact_edges_current_from_idx;
DROP INDEX IF EXISTS artefact_edges_current_to_idx;
DROP INDEX IF EXISTS artefact_edges_current_branch_from_idx;
DROP INDEX IF EXISTS artefact_edges_current_branch_to_idx;
DROP INDEX IF EXISTS artefact_edges_current_path_idx;
DROP INDEX IF EXISTS artefact_edges_current_kind_idx;
DROP INDEX IF EXISTS artefact_edges_current_symbol_ref_idx;
DROP INDEX IF EXISTS artefact_edges_current_natural_uq;

CREATE INDEX IF NOT EXISTS artefact_edges_current_path_idx
ON artefact_edges_current (repo_id, branch, path);

CREATE INDEX IF NOT EXISTS artefact_edges_current_branch_from_idx
ON artefact_edges_current (repo_id, branch, from_symbol_id, edge_kind);

CREATE INDEX IF NOT EXISTS artefact_edges_current_branch_to_idx
ON artefact_edges_current (repo_id, branch, to_symbol_id, edge_kind);

CREATE INDEX IF NOT EXISTS artefact_edges_current_kind_idx
ON artefact_edges_current (repo_id, branch, edge_kind);

CREATE INDEX IF NOT EXISTS artefact_edges_current_symbol_ref_idx
ON artefact_edges_current (repo_id, branch, to_symbol_ref);

CREATE UNIQUE INDEX IF NOT EXISTS artefact_edges_current_natural_uq
ON artefact_edges_current (
    repo_id,
    branch,
    from_symbol_id,
    edge_kind,
    COALESCE(to_symbol_id, ''),
    COALESCE(to_symbol_ref, ''),
    COALESCE(start_line, -1),
    COALESCE(end_line, -1),
    COALESCE(metadata, '{}')
);
"#,
    )
    .context("ensuring branch-aware artefact_edges_current indexes")?;

    Ok(())
}

fn rebuild_artefacts_current_with_copy(conn: &rusqlite::Connection) -> Result<()> {
    conn.execute_batch(
        r#"
DROP TABLE IF EXISTS artefacts_current__branch_migration;
CREATE TABLE artefacts_current__branch_migration (
    repo_id TEXT NOT NULL,
    branch TEXT NOT NULL DEFAULT 'main',
    symbol_id TEXT NOT NULL,
    artefact_id TEXT NOT NULL,
    commit_sha TEXT NOT NULL,
    revision_kind TEXT NOT NULL DEFAULT 'commit',
    revision_id TEXT NOT NULL DEFAULT '',
    temp_checkpoint_id INTEGER,
    blob_sha TEXT NOT NULL,
    path TEXT NOT NULL,
    language TEXT NOT NULL,
    canonical_kind TEXT,
    language_kind TEXT,
    symbol_fqn TEXT,
    parent_symbol_id TEXT,
    parent_artefact_id TEXT,
    start_line INTEGER NOT NULL,
    end_line INTEGER NOT NULL,
    start_byte INTEGER NOT NULL,
    end_byte INTEGER NOT NULL,
    signature TEXT,
    modifiers TEXT NOT NULL DEFAULT '[]',
    docstring TEXT,
    content_hash TEXT,
    updated_at TEXT DEFAULT (datetime('now')),
    PRIMARY KEY (repo_id, branch, symbol_id)
);
INSERT INTO artefacts_current__branch_migration (
    repo_id, branch, symbol_id, artefact_id, commit_sha, revision_kind, revision_id, temp_checkpoint_id,
    blob_sha, path, language, canonical_kind, language_kind, symbol_fqn, parent_symbol_id,
    parent_artefact_id, start_line, end_line, start_byte, end_byte, signature, modifiers,
    docstring, content_hash, updated_at
)
SELECT
    ac.repo_id,
    COALESCE(NULLIF((SELECT r.default_branch FROM repositories r WHERE r.repo_id = ac.repo_id LIMIT 1), ''), 'main') AS branch,
    ac.symbol_id,
    ac.artefact_id,
    ac.commit_sha,
    ac.revision_kind,
    ac.revision_id,
    ac.temp_checkpoint_id,
    ac.blob_sha,
    ac.path,
    ac.language,
    ac.canonical_kind,
    ac.language_kind,
    ac.symbol_fqn,
    ac.parent_symbol_id,
    ac.parent_artefact_id,
    ac.start_line,
    ac.end_line,
    ac.start_byte,
    ac.end_byte,
    ac.signature,
    ac.modifiers,
    ac.docstring,
    ac.content_hash,
    ac.updated_at
FROM artefacts_current ac;
DROP TABLE artefacts_current;
ALTER TABLE artefacts_current__branch_migration RENAME TO artefacts_current;
"#,
    )
    .context("rebuilding artefacts_current with branch-scoped primary key")
}

fn rebuild_artefacts_current_without_copy(conn: &rusqlite::Connection) -> Result<()> {
    conn.execute_batch(
        r#"
DROP TABLE IF EXISTS artefacts_current__branch_migration;
CREATE TABLE artefacts_current__branch_migration (
    repo_id TEXT NOT NULL,
    branch TEXT NOT NULL DEFAULT 'main',
    symbol_id TEXT NOT NULL,
    artefact_id TEXT NOT NULL,
    commit_sha TEXT NOT NULL,
    revision_kind TEXT NOT NULL DEFAULT 'commit',
    revision_id TEXT NOT NULL DEFAULT '',
    temp_checkpoint_id INTEGER,
    blob_sha TEXT NOT NULL,
    path TEXT NOT NULL,
    language TEXT NOT NULL,
    canonical_kind TEXT,
    language_kind TEXT,
    symbol_fqn TEXT,
    parent_symbol_id TEXT,
    parent_artefact_id TEXT,
    start_line INTEGER NOT NULL,
    end_line INTEGER NOT NULL,
    start_byte INTEGER NOT NULL,
    end_byte INTEGER NOT NULL,
    signature TEXT,
    modifiers TEXT NOT NULL DEFAULT '[]',
    docstring TEXT,
    content_hash TEXT,
    updated_at TEXT DEFAULT (datetime('now')),
    PRIMARY KEY (repo_id, branch, symbol_id)
);
DROP TABLE artefacts_current;
ALTER TABLE artefacts_current__branch_migration RENAME TO artefacts_current;
"#,
    )
    .context("rebuilding malformed artefacts_current with branch-scoped primary key")
}

fn rebuild_artefact_edges_current_with_copy(conn: &rusqlite::Connection) -> Result<()> {
    conn.execute_batch(
        r#"
DROP TABLE IF EXISTS artefact_edges_current__branch_migration;
CREATE TABLE artefact_edges_current__branch_migration (
    edge_id TEXT NOT NULL,
    repo_id TEXT NOT NULL,
    branch TEXT NOT NULL DEFAULT 'main',
    commit_sha TEXT NOT NULL,
    revision_kind TEXT NOT NULL DEFAULT 'commit',
    revision_id TEXT NOT NULL DEFAULT '',
    temp_checkpoint_id INTEGER,
    blob_sha TEXT NOT NULL,
    path TEXT NOT NULL,
    from_symbol_id TEXT NOT NULL,
    from_artefact_id TEXT NOT NULL,
    to_symbol_id TEXT,
    to_artefact_id TEXT,
    to_symbol_ref TEXT,
    edge_kind TEXT NOT NULL,
    language TEXT NOT NULL,
    start_line INTEGER,
    end_line INTEGER,
    metadata TEXT DEFAULT '{}',
    updated_at TEXT DEFAULT (datetime('now')),
    CHECK (to_symbol_id IS NOT NULL OR to_symbol_ref IS NOT NULL),
    CHECK (
        (start_line IS NULL AND end_line IS NULL)
        OR (start_line IS NOT NULL AND end_line IS NOT NULL AND start_line > 0 AND end_line >= start_line)
    ),
    PRIMARY KEY (repo_id, branch, edge_id)
);
INSERT INTO artefact_edges_current__branch_migration (
    edge_id, repo_id, branch, commit_sha, revision_kind, revision_id, temp_checkpoint_id, blob_sha,
    path, from_symbol_id, from_artefact_id, to_symbol_id, to_artefact_id, to_symbol_ref,
    edge_kind, language, start_line, end_line, metadata, updated_at
)
SELECT
    ec.edge_id,
    ec.repo_id,
    COALESCE(NULLIF((SELECT r.default_branch FROM repositories r WHERE r.repo_id = ec.repo_id LIMIT 1), ''), 'main') AS branch,
    ec.commit_sha,
    ec.revision_kind,
    ec.revision_id,
    ec.temp_checkpoint_id,
    ec.blob_sha,
    ec.path,
    ec.from_symbol_id,
    ec.from_artefact_id,
    ec.to_symbol_id,
    ec.to_artefact_id,
    ec.to_symbol_ref,
    ec.edge_kind,
    ec.language,
    ec.start_line,
    ec.end_line,
    ec.metadata,
    ec.updated_at
FROM artefact_edges_current ec;
DROP TABLE artefact_edges_current;
ALTER TABLE artefact_edges_current__branch_migration RENAME TO artefact_edges_current;
"#,
    )
    .context("rebuilding artefact_edges_current with branch-scoped primary key")
}

fn rebuild_artefact_edges_current_without_copy(conn: &rusqlite::Connection) -> Result<()> {
    conn.execute_batch(
        r#"
DROP TABLE IF EXISTS artefact_edges_current__branch_migration;
CREATE TABLE artefact_edges_current__branch_migration (
    edge_id TEXT NOT NULL,
    repo_id TEXT NOT NULL,
    branch TEXT NOT NULL DEFAULT 'main',
    commit_sha TEXT NOT NULL,
    revision_kind TEXT NOT NULL DEFAULT 'commit',
    revision_id TEXT NOT NULL DEFAULT '',
    temp_checkpoint_id INTEGER,
    blob_sha TEXT NOT NULL,
    path TEXT NOT NULL,
    from_symbol_id TEXT NOT NULL,
    from_artefact_id TEXT NOT NULL,
    to_symbol_id TEXT,
    to_artefact_id TEXT,
    to_symbol_ref TEXT,
    edge_kind TEXT NOT NULL,
    language TEXT NOT NULL,
    start_line INTEGER,
    end_line INTEGER,
    metadata TEXT DEFAULT '{}',
    updated_at TEXT DEFAULT (datetime('now')),
    CHECK (to_symbol_id IS NOT NULL OR to_symbol_ref IS NOT NULL),
    CHECK (
        (start_line IS NULL AND end_line IS NULL)
        OR (start_line IS NOT NULL AND end_line IS NOT NULL AND start_line > 0 AND end_line >= start_line)
    ),
    PRIMARY KEY (repo_id, branch, edge_id)
);
DROP TABLE artefact_edges_current;
ALTER TABLE artefact_edges_current__branch_migration RENAME TO artefact_edges_current;
"#,
    )
    .context("rebuilding malformed artefact_edges_current with branch-scoped primary key")
}

pub(super) fn artefacts_current_matches_sync_shape(conn: &rusqlite::Connection) -> Result<bool> {
    let expected_columns = [
        "repo_id",
        "path",
        "content_id",
        "symbol_id",
        "artefact_id",
        "language",
        "extraction_fingerprint",
        "canonical_kind",
        "language_kind",
        "symbol_fqn",
        "parent_symbol_id",
        "parent_artefact_id",
        "start_line",
        "end_line",
        "start_byte",
        "end_byte",
        "signature",
        "modifiers",
        "docstring",
        "updated_at",
    ];
    let expected_pk = vec![
        "repo_id".to_string(),
        "path".to_string(),
        "symbol_id".to_string(),
    ];
    Ok(
        sqlite_table_columns(conn, "artefacts_current")? == expected_columns
            && sqlite_table_pk_columns(conn, "artefacts_current")? == expected_pk,
    )
}

pub(super) fn artefact_edges_current_matches_sync_shape(
    conn: &rusqlite::Connection,
) -> Result<bool> {
    let expected_columns = [
        "repo_id",
        "edge_id",
        "path",
        "content_id",
        "from_symbol_id",
        "from_artefact_id",
        "to_symbol_id",
        "to_artefact_id",
        "to_symbol_ref",
        "edge_kind",
        "language",
        "start_line",
        "end_line",
        "metadata",
        "updated_at",
    ];
    let expected_pk = vec!["repo_id".to_string(), "edge_id".to_string()];
    Ok(
        sqlite_table_columns(conn, "artefact_edges_current")? == expected_columns
            && sqlite_table_pk_columns(conn, "artefact_edges_current")? == expected_pk,
    )
}

fn sqlite_table_columns(conn: &rusqlite::Connection, table: &str) -> Result<Vec<String>> {
    let mut stmt = conn
        .prepare(&format!("PRAGMA table_info({table})"))
        .with_context(|| format!("preparing PRAGMA table_info for `{table}`"))?;
    let mut rows = stmt
        .query([])
        .with_context(|| format!("querying PRAGMA table_info for `{table}`"))?;
    let mut columns = Vec::new();
    while let Some(row) = rows.next().context("reading PRAGMA row")? {
        let name: String = row
            .get(1)
            .with_context(|| format!("reading column name from `{table}`"))?;
        columns.push(name);
    }
    Ok(columns)
}

fn sqlite_table_has_columns(
    conn: &rusqlite::Connection,
    table: &str,
    columns: &[&str],
) -> Result<bool> {
    for column in columns {
        if !sqlite_table_has_column(conn, table, column)? {
            return Ok(false);
        }
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::sqlite::introspection::{sqlite_table_has_column, sqlite_table_pk_columns};
    use tempfile::tempdir;

    #[test]
    fn migrate_current_state_branch_scope_skips_exact_sync_shape() -> Result<()> {
        let temp = tempdir().context("creating temp dir")?;
        let db_path = temp.path().join("current-state.sqlite");
        let conn = rusqlite::Connection::open(db_path).context("opening sqlite")?;

        conn.execute_batch(
            r#"
CREATE TABLE artefacts_current (
    repo_id TEXT NOT NULL,
    path TEXT NOT NULL,
    content_id TEXT NOT NULL,
    symbol_id TEXT NOT NULL,
    artefact_id TEXT NOT NULL,
    language TEXT NOT NULL,
    extraction_fingerprint TEXT NOT NULL DEFAULT '',
    canonical_kind TEXT,
    language_kind TEXT,
    symbol_fqn TEXT,
    parent_symbol_id TEXT,
    parent_artefact_id TEXT,
    start_line INTEGER NOT NULL,
    end_line INTEGER NOT NULL,
    start_byte INTEGER NOT NULL,
    end_byte INTEGER NOT NULL,
    signature TEXT,
    modifiers TEXT NOT NULL DEFAULT '[]',
    docstring TEXT,
    updated_at TEXT NOT NULL,
    PRIMARY KEY (repo_id, path, symbol_id)
);

CREATE TABLE artefact_edges_current (
    repo_id TEXT NOT NULL,
    edge_id TEXT NOT NULL,
    path TEXT NOT NULL,
    content_id TEXT NOT NULL,
    from_symbol_id TEXT NOT NULL,
    from_artefact_id TEXT NOT NULL,
    to_symbol_id TEXT,
    to_artefact_id TEXT,
    to_symbol_ref TEXT,
    edge_kind TEXT NOT NULL,
    language TEXT NOT NULL,
    start_line INTEGER,
    end_line INTEGER,
    metadata TEXT NOT NULL DEFAULT '{}',
    updated_at TEXT NOT NULL,
    PRIMARY KEY (repo_id, edge_id)
);
"#,
        )
        .context("creating sync-shaped current-state tables")?;

        migrate_artefacts_current_branch_scope(&conn)?;
        migrate_artefact_edges_current_branch_scope(&conn)?;

        assert!(!sqlite_table_has_column(
            &conn,
            "artefacts_current",
            "branch"
        )?);
        assert!(!sqlite_table_has_column(
            &conn,
            "artefacts_current",
            "commit_sha"
        )?);
        assert!(!sqlite_table_has_column(
            &conn,
            "artefact_edges_current",
            "branch"
        )?);
        assert!(!sqlite_table_has_column(
            &conn,
            "artefact_edges_current",
            "commit_sha"
        )?);

        assert_eq!(
            sqlite_table_pk_columns(&conn, "artefacts_current")?,
            vec![
                "repo_id".to_string(),
                "path".to_string(),
                "symbol_id".to_string(),
            ]
        );
        assert_eq!(
            sqlite_table_pk_columns(&conn, "artefact_edges_current")?,
            vec!["repo_id".to_string(), "edge_id".to_string()]
        );

        Ok(())
    }

    #[test]
    fn migrate_current_state_branch_scope_rebuilds_malformed_tables_without_copy() -> Result<()> {
        let temp = tempdir().context("creating temp dir")?;
        let db_path = temp.path().join("current-state-malformed.sqlite");
        let conn = rusqlite::Connection::open(db_path).context("opening sqlite")?;

        conn.execute_batch(
            r#"
CREATE TABLE artefacts_current (
    repo_id TEXT NOT NULL,
    path TEXT NOT NULL,
    symbol_id TEXT NOT NULL,
    PRIMARY KEY (repo_id, path, symbol_id)
);
CREATE TABLE artefact_edges_current (
    repo_id TEXT NOT NULL,
    edge_id TEXT NOT NULL,
    path TEXT NOT NULL,
    PRIMARY KEY (repo_id, edge_id)
);
"#,
        )
        .context("creating malformed current-state tables")?;

        migrate_artefacts_current_branch_scope(&conn)?;
        migrate_artefact_edges_current_branch_scope(&conn)?;

        assert!(sqlite_table_has_column(
            &conn,
            "artefacts_current",
            "branch"
        )?);
        assert!(sqlite_table_has_column(
            &conn,
            "artefacts_current",
            "commit_sha"
        )?);
        assert!(sqlite_table_has_column(
            &conn,
            "artefact_edges_current",
            "branch"
        )?);
        assert!(sqlite_table_has_column(
            &conn,
            "artefact_edges_current",
            "commit_sha"
        )?);

        assert_eq!(
            sqlite_table_pk_columns(&conn, "artefacts_current")?,
            vec![
                "repo_id".to_string(),
                "branch".to_string(),
                "symbol_id".to_string(),
            ]
        );
        assert_eq!(
            sqlite_table_pk_columns(&conn, "artefact_edges_current")?,
            vec![
                "repo_id".to_string(),
                "branch".to_string(),
                "edge_id".to_string(),
            ]
        );

        Ok(())
    }
}
