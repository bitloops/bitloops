pub(crate) fn artefacts_upgrade_sql() -> &'static str {
    r#"
ALTER TABLE artefacts ADD COLUMN IF NOT EXISTS signature TEXT;
ALTER TABLE artefacts ADD COLUMN IF NOT EXISTS symbol_id TEXT;
ALTER TABLE artefacts ADD COLUMN IF NOT EXISTS modifiers JSONB DEFAULT '[]'::jsonb;
ALTER TABLE artefacts ADD COLUMN IF NOT EXISTS docstring TEXT;
ALTER TABLE artefacts ADD COLUMN IF NOT EXISTS content_hash TEXT;
ALTER TABLE artefacts ALTER COLUMN canonical_kind DROP NOT NULL;
UPDATE artefacts
SET modifiers = '[]'::jsonb
WHERE modifiers IS NULL;
ALTER TABLE artefacts ALTER COLUMN modifiers SET NOT NULL;

CREATE INDEX IF NOT EXISTS artefacts_symbol_idx
ON artefacts (repo_id, symbol_id)
WHERE symbol_id IS NOT NULL;

CREATE INDEX IF NOT EXISTS artefacts_symbol_content_hash_idx
ON artefacts (repo_id, symbol_id, content_hash)
WHERE symbol_id IS NOT NULL;

CREATE INDEX IF NOT EXISTS artefacts_fqn_content_hash_idx
ON artefacts (repo_id, symbol_fqn, content_hash)
WHERE symbol_fqn IS NOT NULL;
"#
}

pub(crate) fn historical_artefacts_cutover_postgres_sql() -> &'static str {
    r#"
DO $$
BEGIN
    IF EXISTS (
        SELECT 1
        FROM information_schema.columns
        WHERE table_schema = current_schema()
          AND table_name = 'artefacts'
          AND column_name = 'blob_sha'
    ) THEN
        INSERT INTO artefact_snapshots (
            repo_id, blob_sha, path, artefact_id, parent_artefact_id, start_line, end_line, start_byte, end_byte
        )
        SELECT
            a.repo_id,
            a.blob_sha,
            a.path,
            a.artefact_id,
            a.parent_artefact_id,
            a.start_line,
            a.end_line,
            a.start_byte,
            a.end_byte
        FROM artefacts a
        ON CONFLICT (repo_id, blob_sha, artefact_id) DO UPDATE SET
            path = EXCLUDED.path,
            parent_artefact_id = EXCLUDED.parent_artefact_id,
            start_line = EXCLUDED.start_line,
            end_line = EXCLUDED.end_line,
            start_byte = EXCLUDED.start_byte,
            end_byte = EXCLUDED.end_byte;
    END IF;
END $$;

DROP VIEW IF EXISTS artefacts_historical;

ALTER TABLE artefacts DROP COLUMN IF EXISTS blob_sha;
ALTER TABLE artefacts DROP COLUMN IF EXISTS path;
ALTER TABLE artefacts DROP COLUMN IF EXISTS parent_artefact_id;
ALTER TABLE artefacts DROP COLUMN IF EXISTS start_line;
ALTER TABLE artefacts DROP COLUMN IF EXISTS end_line;
ALTER TABLE artefacts DROP COLUMN IF EXISTS start_byte;
ALTER TABLE artefacts DROP COLUMN IF EXISTS end_byte;

DROP INDEX IF EXISTS artefacts_blob_idx;
DROP INDEX IF EXISTS artefacts_path_idx;
DROP INDEX IF EXISTS artefacts_symbol_content_hash_idx;
DROP INDEX IF EXISTS artefacts_fqn_content_hash_idx;

CREATE INDEX IF NOT EXISTS artefacts_symbol_content_hash_idx
ON artefacts (repo_id, symbol_id, content_hash)
WHERE symbol_id IS NOT NULL;

CREATE INDEX IF NOT EXISTS artefacts_fqn_content_hash_idx
ON artefacts (repo_id, symbol_fqn, content_hash)
WHERE symbol_fqn IS NOT NULL;

DROP INDEX IF EXISTS artefact_snapshots_artefact_blob_idx;
DROP INDEX IF EXISTS artefact_snapshots_path_blob_line_idx;
CREATE INDEX IF NOT EXISTS artefact_snapshots_artefact_blob_idx
ON artefact_snapshots (repo_id, artefact_id, blob_sha);
CREATE INDEX IF NOT EXISTS artefact_snapshots_path_blob_line_idx
ON artefact_snapshots (repo_id, path, blob_sha, start_line, end_line);

DROP INDEX IF EXISTS artefact_edges_from_blob_kind_idx;
DROP INDEX IF EXISTS artefact_edges_to_blob_kind_idx;
CREATE INDEX IF NOT EXISTS artefact_edges_from_blob_kind_idx
ON artefact_edges (repo_id, from_artefact_id, blob_sha, edge_kind);
CREATE INDEX IF NOT EXISTS artefact_edges_to_blob_kind_idx
ON artefact_edges (repo_id, to_artefact_id, blob_sha, edge_kind);

DROP INDEX IF EXISTS file_state_path_blob_commit_idx;
CREATE INDEX IF NOT EXISTS file_state_path_blob_commit_idx
ON file_state (repo_id, path, blob_sha, commit_sha);

CREATE OR REPLACE VIEW artefacts_historical AS
SELECT
    a.artefact_id AS artefact_id,
    a.symbol_id AS symbol_id,
    a.repo_id AS repo_id,
    s.blob_sha AS blob_sha,
    s.path AS path,
    a.language AS language,
    a.canonical_kind AS canonical_kind,
    a.language_kind AS language_kind,
    a.symbol_fqn AS symbol_fqn,
    s.parent_artefact_id AS parent_artefact_id,
    s.start_line AS start_line,
    s.end_line AS end_line,
    s.start_byte AS start_byte,
    s.end_byte AS end_byte,
    a.signature AS signature,
    a.modifiers AS modifiers,
    a.docstring AS docstring,
    a.content_hash AS content_hash,
    a.created_at AS created_at
FROM artefact_snapshots s
JOIN artefacts a
  ON a.repo_id = s.repo_id
 AND a.artefact_id = s.artefact_id;
"#
}

pub(crate) fn historical_artefacts_cutover_sqlite_sql() -> &'static str {
    r#"
DROP VIEW IF EXISTS artefacts_historical;

INSERT INTO artefact_snapshots (
    repo_id, blob_sha, path, artefact_id, parent_artefact_id, start_line, end_line, start_byte, end_byte
)
SELECT
    repo_id, blob_sha, path, artefact_id, parent_artefact_id, start_line, end_line, start_byte, end_byte
FROM artefacts
ON CONFLICT(repo_id, blob_sha, artefact_id) DO UPDATE SET
    path = excluded.path,
    parent_artefact_id = excluded.parent_artefact_id,
    start_line = excluded.start_line,
    end_line = excluded.end_line,
    start_byte = excluded.start_byte,
    end_byte = excluded.end_byte;

CREATE TABLE IF NOT EXISTS artefacts_cutover (
    artefact_id TEXT PRIMARY KEY,
    symbol_id TEXT,
    repo_id TEXT NOT NULL,
    language TEXT NOT NULL,
    canonical_kind TEXT,
    language_kind TEXT,
    symbol_fqn TEXT,
    signature TEXT,
    modifiers TEXT NOT NULL DEFAULT '[]',
    docstring TEXT,
    content_hash TEXT,
    created_at TEXT DEFAULT (datetime('now'))
);

INSERT OR REPLACE INTO artefacts_cutover (
    artefact_id, symbol_id, repo_id, language, canonical_kind, language_kind, symbol_fqn,
    signature, modifiers, docstring, content_hash, created_at
)
SELECT
    artefact_id, symbol_id, repo_id, language, canonical_kind, language_kind, symbol_fqn,
    signature, modifiers, docstring, content_hash, created_at
FROM artefacts;

DROP TABLE artefacts;
ALTER TABLE artefacts_cutover RENAME TO artefacts;

DROP INDEX IF EXISTS artefacts_blob_idx;
DROP INDEX IF EXISTS artefacts_path_idx;
DROP INDEX IF EXISTS artefacts_kind_idx;
DROP INDEX IF EXISTS artefacts_symbol_idx;
DROP INDEX IF EXISTS artefacts_symbol_content_hash_idx;
DROP INDEX IF EXISTS artefacts_fqn_content_hash_idx;
CREATE INDEX IF NOT EXISTS artefacts_kind_idx
ON artefacts (repo_id, canonical_kind);
CREATE INDEX IF NOT EXISTS artefacts_symbol_idx
ON artefacts (repo_id, symbol_id);
CREATE INDEX IF NOT EXISTS artefacts_symbol_content_hash_idx
ON artefacts (repo_id, symbol_id, content_hash);
CREATE INDEX IF NOT EXISTS artefacts_fqn_content_hash_idx
ON artefacts (repo_id, symbol_fqn, content_hash);

DROP INDEX IF EXISTS artefact_snapshots_artefact_blob_idx;
DROP INDEX IF EXISTS artefact_snapshots_path_blob_line_idx;
CREATE INDEX IF NOT EXISTS artefact_snapshots_artefact_blob_idx
ON artefact_snapshots (repo_id, artefact_id, blob_sha);
CREATE INDEX IF NOT EXISTS artefact_snapshots_path_blob_line_idx
ON artefact_snapshots (repo_id, path, blob_sha, start_line, end_line);

DROP INDEX IF EXISTS file_state_path_blob_commit_idx;
CREATE INDEX IF NOT EXISTS file_state_path_blob_commit_idx
ON file_state (repo_id, path, blob_sha, commit_sha);

DROP INDEX IF EXISTS artefact_edges_from_blob_kind_idx;
DROP INDEX IF EXISTS artefact_edges_to_blob_kind_idx;
CREATE INDEX IF NOT EXISTS artefact_edges_from_blob_kind_idx
ON artefact_edges (repo_id, from_artefact_id, blob_sha, edge_kind);
CREATE INDEX IF NOT EXISTS artefact_edges_to_blob_kind_idx
ON artefact_edges (repo_id, to_artefact_id, blob_sha, edge_kind);

CREATE VIEW IF NOT EXISTS artefacts_historical AS
SELECT
    a.artefact_id AS artefact_id,
    a.symbol_id AS symbol_id,
    a.repo_id AS repo_id,
    s.blob_sha AS blob_sha,
    s.path AS path,
    a.language AS language,
    a.canonical_kind AS canonical_kind,
    a.language_kind AS language_kind,
    a.symbol_fqn AS symbol_fqn,
    s.parent_artefact_id AS parent_artefact_id,
    s.start_line AS start_line,
    s.end_line AS end_line,
    s.start_byte AS start_byte,
    s.end_byte AS end_byte,
    a.signature AS signature,
    a.modifiers AS modifiers,
    a.docstring AS docstring,
    a.content_hash AS content_hash,
    a.created_at AS created_at
FROM artefact_snapshots s
JOIN artefacts a
  ON a.repo_id = s.repo_id
 AND a.artefact_id = s.artefact_id;
"#
}

pub(crate) fn artefact_edges_hardening_sql() -> &'static str {
    r#"
ALTER TABLE artefact_edges ADD COLUMN IF NOT EXISTS metadata JSONB DEFAULT '{}'::jsonb;

DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1
        FROM pg_constraint
        WHERE conname = 'artefact_edges_target_chk'
    ) THEN
        ALTER TABLE artefact_edges
        ADD CONSTRAINT artefact_edges_target_chk
        CHECK (to_artefact_id IS NOT NULL OR to_symbol_ref IS NOT NULL);
    END IF;
END $$;

DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1
        FROM pg_constraint
        WHERE conname = 'artefact_edges_line_range_chk'
    ) THEN
        ALTER TABLE artefact_edges
        ADD CONSTRAINT artefact_edges_line_range_chk
        CHECK (
            (start_line IS NULL AND end_line IS NULL)
            OR (start_line IS NOT NULL AND end_line IS NOT NULL AND start_line > 0 AND end_line >= start_line)
        );
    END IF;
END $$;

CREATE INDEX IF NOT EXISTS artefact_edges_blob_idx
ON artefact_edges (repo_id, blob_sha);

CREATE INDEX IF NOT EXISTS artefact_edges_from_idx
ON artefact_edges (repo_id, from_artefact_id, edge_kind);

CREATE INDEX IF NOT EXISTS artefact_edges_to_idx
ON artefact_edges (repo_id, to_artefact_id, edge_kind);

CREATE INDEX IF NOT EXISTS artefact_edges_kind_idx
ON artefact_edges (repo_id, edge_kind);

CREATE INDEX IF NOT EXISTS artefact_edges_symbol_ref_idx
ON artefact_edges (repo_id, edge_kind, to_symbol_ref)
WHERE to_symbol_ref IS NOT NULL;

CREATE UNIQUE INDEX IF NOT EXISTS artefact_edges_natural_uq
ON artefact_edges (
    repo_id,
    blob_sha,
    from_artefact_id,
    edge_kind,
    COALESCE(to_artefact_id, ''),
    COALESCE(to_symbol_ref, ''),
    COALESCE(start_line, -1),
    COALESCE(end_line, -1)
);
"#
}

#[allow(dead_code)]
pub(crate) fn current_state_hardening_sql() -> &'static str {
    r#"
CREATE TABLE IF NOT EXISTS current_file_state (
    repo_id TEXT NOT NULL,
    path TEXT NOT NULL,
    commit_sha TEXT NOT NULL,
    blob_sha TEXT NOT NULL,
    committed_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ DEFAULT now(),
    PRIMARY KEY (repo_id, path)
);

ALTER TABLE current_file_state ADD COLUMN IF NOT EXISTS commit_sha TEXT;
ALTER TABLE current_file_state ADD COLUMN IF NOT EXISTS blob_sha TEXT;
ALTER TABLE current_file_state ADD COLUMN IF NOT EXISTS committed_at TIMESTAMPTZ;
ALTER TABLE current_file_state ADD COLUMN IF NOT EXISTS updated_at TIMESTAMPTZ DEFAULT now();

CREATE TABLE IF NOT EXISTS artefacts_current (
    repo_id TEXT NOT NULL,
    branch TEXT NOT NULL DEFAULT 'main',
    symbol_id TEXT NOT NULL,
    artefact_id TEXT NOT NULL,
    commit_sha TEXT NOT NULL,
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
    content_hash TEXT,
    updated_at TIMESTAMPTZ DEFAULT now(),
    PRIMARY KEY (repo_id, branch, symbol_id)
);

ALTER TABLE artefacts_current ADD COLUMN IF NOT EXISTS branch TEXT;
ALTER TABLE artefacts_current ADD COLUMN IF NOT EXISTS artefact_id TEXT;
ALTER TABLE artefacts_current ADD COLUMN IF NOT EXISTS commit_sha TEXT;
ALTER TABLE artefacts_current ADD COLUMN IF NOT EXISTS blob_sha TEXT;
ALTER TABLE artefacts_current ADD COLUMN IF NOT EXISTS path TEXT;
ALTER TABLE artefacts_current ADD COLUMN IF NOT EXISTS language TEXT;
ALTER TABLE artefacts_current ADD COLUMN IF NOT EXISTS canonical_kind TEXT;
ALTER TABLE artefacts_current ADD COLUMN IF NOT EXISTS language_kind TEXT;
ALTER TABLE artefacts_current ADD COLUMN IF NOT EXISTS symbol_fqn TEXT;
ALTER TABLE artefacts_current ADD COLUMN IF NOT EXISTS parent_symbol_id TEXT;
ALTER TABLE artefacts_current ADD COLUMN IF NOT EXISTS parent_artefact_id TEXT;
ALTER TABLE artefacts_current ADD COLUMN IF NOT EXISTS start_line INTEGER;
ALTER TABLE artefacts_current ADD COLUMN IF NOT EXISTS end_line INTEGER;
ALTER TABLE artefacts_current ADD COLUMN IF NOT EXISTS start_byte INTEGER;
ALTER TABLE artefacts_current ADD COLUMN IF NOT EXISTS end_byte INTEGER;
ALTER TABLE artefacts_current ADD COLUMN IF NOT EXISTS signature TEXT;
ALTER TABLE artefacts_current ADD COLUMN IF NOT EXISTS modifiers JSONB DEFAULT '[]'::jsonb;
ALTER TABLE artefacts_current ADD COLUMN IF NOT EXISTS docstring TEXT;
ALTER TABLE artefacts_current ADD COLUMN IF NOT EXISTS content_hash TEXT;
ALTER TABLE artefacts_current ADD COLUMN IF NOT EXISTS updated_at TIMESTAMPTZ DEFAULT now();
ALTER TABLE artefacts_current ADD COLUMN IF NOT EXISTS revision_kind TEXT NOT NULL DEFAULT 'commit';
ALTER TABLE artefacts_current ADD COLUMN IF NOT EXISTS revision_id TEXT NOT NULL DEFAULT '';
ALTER TABLE artefacts_current ADD COLUMN IF NOT EXISTS temp_checkpoint_id BIGINT;
ALTER TABLE artefacts_current ALTER COLUMN branch SET DEFAULT 'main';
ALTER TABLE artefacts_current ALTER COLUMN branch SET NOT NULL;
ALTER TABLE artefacts_current ALTER COLUMN canonical_kind DROP NOT NULL;
ALTER TABLE artefacts_current ALTER COLUMN modifiers SET NOT NULL;
ALTER TABLE artefacts_current DROP CONSTRAINT IF EXISTS artefacts_current_pkey;
ALTER TABLE artefacts_current ADD PRIMARY KEY (repo_id, branch, symbol_id);

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

CREATE TABLE IF NOT EXISTS artefact_edges_current (
    edge_id TEXT NOT NULL,
    repo_id TEXT NOT NULL,
    branch TEXT NOT NULL DEFAULT 'main',
    commit_sha TEXT NOT NULL,
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
    metadata JSONB DEFAULT '{}'::jsonb,
    updated_at TIMESTAMPTZ DEFAULT now(),
    PRIMARY KEY (repo_id, branch, edge_id)
);

ALTER TABLE artefact_edges_current ADD COLUMN IF NOT EXISTS branch TEXT;
ALTER TABLE artefact_edges_current ADD COLUMN IF NOT EXISTS commit_sha TEXT;
ALTER TABLE artefact_edges_current ADD COLUMN IF NOT EXISTS blob_sha TEXT;
ALTER TABLE artefact_edges_current ADD COLUMN IF NOT EXISTS path TEXT;
ALTER TABLE artefact_edges_current ADD COLUMN IF NOT EXISTS from_symbol_id TEXT;
ALTER TABLE artefact_edges_current ADD COLUMN IF NOT EXISTS from_artefact_id TEXT;
ALTER TABLE artefact_edges_current ADD COLUMN IF NOT EXISTS to_symbol_id TEXT;
ALTER TABLE artefact_edges_current ADD COLUMN IF NOT EXISTS to_artefact_id TEXT;
ALTER TABLE artefact_edges_current ADD COLUMN IF NOT EXISTS to_symbol_ref TEXT;
ALTER TABLE artefact_edges_current ADD COLUMN IF NOT EXISTS edge_kind TEXT;
ALTER TABLE artefact_edges_current ADD COLUMN IF NOT EXISTS language TEXT;
ALTER TABLE artefact_edges_current ADD COLUMN IF NOT EXISTS start_line INTEGER;
ALTER TABLE artefact_edges_current ADD COLUMN IF NOT EXISTS end_line INTEGER;
ALTER TABLE artefact_edges_current ADD COLUMN IF NOT EXISTS metadata JSONB DEFAULT '{}'::jsonb;
ALTER TABLE artefact_edges_current ADD COLUMN IF NOT EXISTS updated_at TIMESTAMPTZ DEFAULT now();
ALTER TABLE artefact_edges_current ADD COLUMN IF NOT EXISTS revision_kind TEXT NOT NULL DEFAULT 'commit';
ALTER TABLE artefact_edges_current ADD COLUMN IF NOT EXISTS revision_id TEXT NOT NULL DEFAULT '';
ALTER TABLE artefact_edges_current ADD COLUMN IF NOT EXISTS temp_checkpoint_id BIGINT;
ALTER TABLE artefact_edges_current ALTER COLUMN branch SET DEFAULT 'main';
ALTER TABLE artefact_edges_current ALTER COLUMN branch SET NOT NULL;
ALTER TABLE artefact_edges_current DROP CONSTRAINT IF EXISTS artefact_edges_current_pkey;
ALTER TABLE artefact_edges_current ADD PRIMARY KEY (repo_id, branch, edge_id);

DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1
        FROM pg_constraint
        WHERE conname = 'artefact_edges_current_target_chk'
    ) THEN
        ALTER TABLE artefact_edges_current
        ADD CONSTRAINT artefact_edges_current_target_chk
        CHECK (to_symbol_id IS NOT NULL OR to_symbol_ref IS NOT NULL);
    END IF;
END $$;

DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1
        FROM pg_constraint
        WHERE conname = 'artefact_edges_current_line_range_chk'
    ) THEN
        ALTER TABLE artefact_edges_current
        ADD CONSTRAINT artefact_edges_current_line_range_chk
        CHECK (
            (start_line IS NULL AND end_line IS NULL)
            OR (start_line IS NOT NULL AND end_line IS NOT NULL AND start_line > 0 AND end_line >= start_line)
        );
    END IF;
END $$;

DROP INDEX IF EXISTS artefact_edges_current_path_idx;
DROP INDEX IF EXISTS artefact_edges_current_from_idx;
DROP INDEX IF EXISTS artefact_edges_current_to_idx;
DROP INDEX IF EXISTS artefact_edges_current_branch_from_idx;
DROP INDEX IF EXISTS artefact_edges_current_branch_to_idx;
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
ON artefact_edges_current (repo_id, branch, to_symbol_ref)
WHERE to_symbol_ref IS NOT NULL;

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
    md5(metadata::text)
);
"#
}

pub(crate) fn test_links_upgrade_sql() -> &'static str {
    r#"
ALTER TABLE test_links ADD COLUMN IF NOT EXISTS confidence DOUBLE PRECISION NOT NULL DEFAULT 0.6;
ALTER TABLE test_links ADD COLUMN IF NOT EXISTS linkage_status TEXT NOT NULL DEFAULT 'resolved';
"#
}

pub(crate) fn workspace_revisions_sql() -> &'static str {
    r#"
CREATE TABLE IF NOT EXISTS workspace_revisions (
    id         BIGSERIAL PRIMARY KEY,
    repo_id    TEXT      NOT NULL,
    tree_hash  TEXT      NOT NULL,
    created_at TIMESTAMPTZ DEFAULT now()
);

CREATE INDEX IF NOT EXISTS workspace_revisions_repo_idx
ON workspace_revisions (repo_id);

CREATE UNIQUE INDEX IF NOT EXISTS workspace_revisions_repo_tree_unique_idx
ON workspace_revisions (repo_id, tree_hash);
"#
}

pub(crate) fn edge_model_cleanup_postgres_sql() -> &'static str {
    r#"
UPDATE artefact_edges
SET edge_kind = 'extends'
WHERE edge_kind = 'inherits';

UPDATE artefact_edges
SET metadata = CASE
    WHEN edge_kind IN ('extends', 'implements') THEN '{}'::jsonb
    ELSE metadata - 'inherit_form' - 'relation'
END
WHERE metadata IS NOT NULL;

UPDATE artefact_edges
SET metadata = jsonb_set(metadata, '{import_form}', '\"binding\"'::jsonb)
WHERE metadata ->> 'import_form' IN ('module', 'use');
"#
}

pub(crate) fn edge_model_cleanup_sqlite_sql() -> &'static str {
    r#"
UPDATE artefact_edges
SET edge_kind = 'extends'
WHERE edge_kind = 'inherits';

UPDATE artefact_edges
SET metadata = CASE
    WHEN edge_kind IN ('extends', 'implements') THEN '{}'
    ELSE json_remove(json_remove(COALESCE(metadata, '{}'), '$.inherit_form'), '$.relation')
END
WHERE metadata IS NOT NULL;

UPDATE artefact_edges
SET metadata = json_set(COALESCE(metadata, '{}'), '$.import_form', 'binding')
WHERE json_extract(metadata, '$.import_form') IN ('module', 'use');
"#
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn current_state_hardening_sql_includes_checkpoint_column_migrations_for_artefacts_current() {
        let sql = current_state_hardening_sql();
        assert!(
            sql.contains("ALTER TABLE artefacts_current ADD COLUMN IF NOT EXISTS revision_kind"),
            "migration must add revision_kind to artefacts_current"
        );
        assert!(
            sql.contains("ALTER TABLE artefacts_current ADD COLUMN IF NOT EXISTS revision_id"),
            "migration must add revision_id to artefacts_current"
        );
        assert!(
            sql.contains(
                "ALTER TABLE artefacts_current ADD COLUMN IF NOT EXISTS temp_checkpoint_id"
            ),
            "migration must add temp_checkpoint_id to artefacts_current"
        );
    }

    #[test]
    fn current_state_hardening_sql_includes_checkpoint_column_migrations_for_artefact_edges_current()
     {
        let sql = current_state_hardening_sql();
        assert!(
            sql.contains(
                "ALTER TABLE artefact_edges_current ADD COLUMN IF NOT EXISTS revision_kind"
            ),
            "migration must add revision_kind to artefact_edges_current"
        );
        assert!(
            sql.contains("ALTER TABLE artefact_edges_current ADD COLUMN IF NOT EXISTS revision_id"),
            "migration must add revision_id to artefact_edges_current"
        );
        assert!(
            sql.contains(
                "ALTER TABLE artefact_edges_current ADD COLUMN IF NOT EXISTS temp_checkpoint_id"
            ),
            "migration must add temp_checkpoint_id to artefact_edges_current"
        );
    }

    #[test]
    fn edge_model_cleanup_postgres_sql_does_not_mutate_current_state_table() {
        let sql = edge_model_cleanup_postgres_sql();
        assert!(
            !sql.contains("UPDATE artefact_edges_current"),
            "cleanup should not write to artefact_edges_current"
        );
    }

    #[test]
    fn edge_model_cleanup_sqlite_sql_does_not_mutate_current_state_table() {
        let sql = edge_model_cleanup_sqlite_sql();
        assert!(
            !sql.contains("UPDATE artefact_edges_current"),
            "cleanup should not write to artefact_edges_current"
        );
    }

    #[test]
    fn historical_artefacts_cutover_postgres_sql_drops_legacy_columns_and_rebuilds_view() {
        let sql = historical_artefacts_cutover_postgres_sql();
        assert!(sql.contains("ALTER TABLE artefacts DROP COLUMN IF EXISTS blob_sha"));
        assert!(sql.contains("INSERT INTO artefact_snapshots"));
        assert!(sql.contains("CREATE OR REPLACE VIEW artefacts_historical AS"));
        assert!(sql.contains("FROM artefact_snapshots s"));
    }

    #[test]
    fn historical_artefacts_cutover_sqlite_sql_rebuilds_artefacts_table_shape() {
        let sql = historical_artefacts_cutover_sqlite_sql();
        assert!(sql.contains("CREATE TABLE IF NOT EXISTS artefacts_cutover"));
        assert!(sql.contains("DROP TABLE artefacts;"));
        assert!(sql.contains("ALTER TABLE artefacts_cutover RENAME TO artefacts;"));
        assert!(sql.contains("CREATE VIEW IF NOT EXISTS artefacts_historical AS"));
    }
}
