pub(crate) fn artefacts_upgrade_sql() -> &'static str {
    r#"
ALTER TABLE artefacts ADD COLUMN IF NOT EXISTS start_byte INTEGER;
ALTER TABLE artefacts ADD COLUMN IF NOT EXISTS end_byte INTEGER;
ALTER TABLE artefacts ADD COLUMN IF NOT EXISTS signature TEXT;
ALTER TABLE artefacts ADD COLUMN IF NOT EXISTS symbol_id TEXT;
ALTER TABLE artefacts ADD COLUMN IF NOT EXISTS modifiers JSONB DEFAULT '[]'::jsonb;
ALTER TABLE artefacts ADD COLUMN IF NOT EXISTS docstring TEXT;
ALTER TABLE artefacts ALTER COLUMN canonical_kind DROP NOT NULL;
UPDATE artefacts
SET start_byte = 0
WHERE start_byte IS NULL;
UPDATE artefacts
SET end_byte = 0
WHERE end_byte IS NULL;
UPDATE artefacts
SET modifiers = '[]'::jsonb
WHERE modifiers IS NULL;
ALTER TABLE artefacts ALTER COLUMN start_byte SET NOT NULL;
ALTER TABLE artefacts ALTER COLUMN end_byte SET NOT NULL;
ALTER TABLE artefacts ALTER COLUMN modifiers SET NOT NULL;

CREATE INDEX IF NOT EXISTS artefacts_symbol_idx
ON artefacts (repo_id, symbol_id)
WHERE symbol_id IS NOT NULL;
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
    PRIMARY KEY (repo_id, symbol_id)
);

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
ALTER TABLE artefacts_current ALTER COLUMN canonical_kind DROP NOT NULL;
UPDATE artefacts_current
SET modifiers = '[]'::jsonb
WHERE modifiers IS NULL;
ALTER TABLE artefacts_current ALTER COLUMN modifiers SET NOT NULL;

CREATE INDEX IF NOT EXISTS artefacts_current_path_idx
ON artefacts_current (repo_id, path);

CREATE INDEX IF NOT EXISTS artefacts_current_kind_idx
ON artefacts_current (repo_id, canonical_kind);

CREATE INDEX IF NOT EXISTS artefacts_current_artefact_idx
ON artefacts_current (repo_id, artefact_id);

CREATE INDEX IF NOT EXISTS artefacts_current_symbol_fqn_idx
ON artefacts_current (repo_id, symbol_fqn);

CREATE TABLE IF NOT EXISTS artefact_edges_current (
    edge_id TEXT PRIMARY KEY,
    repo_id TEXT NOT NULL,
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
    updated_at TIMESTAMPTZ DEFAULT now()
);

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

CREATE INDEX IF NOT EXISTS artefact_edges_current_path_idx
ON artefact_edges_current (repo_id, path);

CREATE INDEX IF NOT EXISTS artefact_edges_current_from_idx
ON artefact_edges_current (repo_id, from_symbol_id, edge_kind);

CREATE INDEX IF NOT EXISTS artefact_edges_current_to_idx
ON artefact_edges_current (repo_id, to_symbol_id, edge_kind);

CREATE INDEX IF NOT EXISTS artefact_edges_current_kind_idx
ON artefact_edges_current (repo_id, edge_kind);

CREATE INDEX IF NOT EXISTS artefact_edges_current_symbol_ref_idx
ON artefact_edges_current (repo_id, to_symbol_ref)
WHERE to_symbol_ref IS NOT NULL;

CREATE UNIQUE INDEX IF NOT EXISTS artefact_edges_current_natural_uq
ON artefact_edges_current (
    repo_id,
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

CREATE INDEX IF NOT EXISTS workspace_revisions_tree_idx
ON workspace_revisions (repo_id, tree_hash);
"#
}

pub(crate) fn edge_model_cleanup_postgres_sql() -> &'static str {
    r#"
UPDATE artefact_edges
SET edge_kind = 'extends'
WHERE edge_kind = 'inherits';

UPDATE artefact_edges_current
SET edge_kind = 'extends'
WHERE edge_kind = 'inherits';

UPDATE artefact_edges
SET metadata = CASE
    WHEN edge_kind IN ('extends', 'implements') THEN '{}'::jsonb
    ELSE metadata - 'inherit_form' - 'relation'
END
WHERE metadata IS NOT NULL;

UPDATE artefact_edges_current
SET metadata = CASE
    WHEN edge_kind IN ('extends', 'implements') THEN '{}'::jsonb
    ELSE metadata - 'inherit_form' - 'relation'
END
WHERE metadata IS NOT NULL;

UPDATE artefact_edges
SET metadata = jsonb_set(metadata, '{import_form}', '\"binding\"'::jsonb)
WHERE metadata ->> 'import_form' IN ('module', 'use');

UPDATE artefact_edges_current
SET metadata = jsonb_set(metadata, '{import_form}', '\"binding\"'::jsonb)
WHERE metadata ->> 'import_form' IN ('module', 'use');
"#
}

pub(crate) fn edge_model_cleanup_sqlite_sql() -> &'static str {
    r#"
UPDATE artefact_edges
SET edge_kind = 'extends'
WHERE edge_kind = 'inherits';

UPDATE artefact_edges_current
SET edge_kind = 'extends'
WHERE edge_kind = 'inherits';

UPDATE artefact_edges
SET metadata = CASE
    WHEN edge_kind IN ('extends', 'implements') THEN '{}'
    ELSE json_remove(json_remove(COALESCE(metadata, '{}'), '$.inherit_form'), '$.relation')
END
WHERE metadata IS NOT NULL;

UPDATE artefact_edges_current
SET metadata = CASE
    WHEN edge_kind IN ('extends', 'implements') THEN '{}'
    ELSE json_remove(json_remove(COALESCE(metadata, '{}'), '$.inherit_form'), '$.relation')
END
WHERE metadata IS NOT NULL;

UPDATE artefact_edges
SET metadata = json_set(COALESCE(metadata, '{}'), '$.import_form', 'binding')
WHERE json_extract(metadata, '$.import_form') IN ('module', 'use');

UPDATE artefact_edges_current
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
}
