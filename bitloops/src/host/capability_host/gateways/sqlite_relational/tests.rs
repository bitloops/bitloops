use anyhow::Result;
use tempfile::TempDir;

use super::SqliteRelationalGateway;
use crate::storage::{SqliteConnectionPool, init::init_database};

#[test]
fn current_canonical_loaders_return_sync_shaped_rows() -> Result<()> {
    let temp = TempDir::new()?;
    let db_path = temp.path().join("runtime.sqlite");
    init_database(&db_path, false, "seed-commit")?;
    let sqlite = SqliteConnectionPool::connect_existing(db_path)?;
    let gateway = SqliteRelationalGateway::new(sqlite.clone());

    sqlite.with_write_connection(|conn| {
        conn.execute(
            "INSERT INTO repositories (repo_id, provider, organization, name, default_branch)
             VALUES (?1, 'local', 'bitloops', 'demo', 'main')",
            rusqlite::params!["repo-1"],
        )?;
        conn.execute(
            "INSERT INTO current_file_state (
                repo_id, path, analysis_mode, file_role, language, resolved_language,
                effective_content_id, effective_source, parser_version, extractor_version,
                exists_in_head, exists_in_index, exists_in_worktree, last_synced_at
            ) VALUES (
                ?1, ?2, 'code', 'source_code', 'typescript', 'typescript',
                'content-a', 'worktree', 'parser-v1', 'extractor-v1',
                1, 0, 1, '2026-04-28T10:00:00Z'
            )",
            rusqlite::params!["repo-1", "packages/api/src/caller.ts"],
        )?;
        conn.execute(
            "INSERT INTO artefacts_current (
                repo_id, path, content_id, symbol_id, artefact_id, language,
                extraction_fingerprint, canonical_kind, language_kind, symbol_fqn,
                parent_symbol_id, parent_artefact_id, start_line, end_line,
                start_byte, end_byte, signature, modifiers, docstring, updated_at
            ) VALUES (
                ?1, ?2, 'content-a', 'sym::caller', 'artefact::caller', 'typescript',
                'fingerprint-a', 'function', 'function_declaration',
                'packages/api/src/caller.ts::caller', NULL, NULL, 4, 8,
                0, 40, NULL, '[]', 'Doc', '2026-04-28T10:00:00Z'
            )",
            rusqlite::params!["repo-1", "packages/api/src/caller.ts"],
        )?;
        conn.execute(
            "INSERT INTO artefact_edges_current (
                repo_id, edge_id, path, content_id, from_symbol_id, from_artefact_id,
                to_symbol_id, to_artefact_id, to_symbol_ref, edge_kind, language,
                start_line, end_line, metadata, updated_at
            ) VALUES (
                ?1, 'edge-1', ?2, 'content-a', 'sym::caller', 'artefact::caller',
                NULL, NULL, 'packages/api/src/target.ts::target', 'calls', 'typescript',
                6, 6, '{\"resolution\":\"fixture\"}', '2026-04-28T10:00:00Z'
            )",
            rusqlite::params!["repo-1", "packages/api/src/caller.ts"],
        )?;
        Ok(())
    })?;

    let files = gateway.load_current_canonical_files("repo-1")?;
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].path, "packages/api/src/caller.ts");
    assert_eq!(files[0].analysis_mode, "code");
    assert!(!files[0].exists_in_index);
    assert!(files[0].exists_in_head);
    assert!(files[0].exists_in_worktree);

    let artefacts = gateway.load_current_canonical_artefacts("repo-1")?;
    assert_eq!(artefacts.len(), 1);
    assert_eq!(artefacts[0].artefact_id, "artefact::caller");
    assert_eq!(artefacts[0].canonical_kind.as_deref(), Some("function"));
    assert_eq!(
        artefacts[0].symbol_fqn.as_deref(),
        Some("packages/api/src/caller.ts::caller")
    );

    let edges = gateway.load_current_canonical_edges("repo-1")?;
    assert_eq!(edges.len(), 1);
    assert_eq!(edges[0].edge_id, "edge-1");
    assert_eq!(edges[0].edge_kind, "calls");
    assert_eq!(
        edges[0].to_symbol_ref.as_deref(),
        Some("packages/api/src/target.ts::target")
    );

    Ok(())
}

#[test]
fn load_current_artefacts_for_file_lines_reads_artefacts_current_by_suffix() -> Result<()> {
    let temp = TempDir::new()?;
    let db_path = temp.path().join("runtime.sqlite");
    init_database(&db_path, false, "seed-commit")?;
    let sqlite = SqliteConnectionPool::connect_existing(db_path)?;
    let gateway = SqliteRelationalGateway::new(sqlite.clone());

    sqlite.with_write_connection(|conn| {
        conn.execute(
            "INSERT INTO repositories (repo_id, provider, organization, name, default_branch)
             VALUES (?1, 'local', 'bitloops', 'demo', 'main')",
            rusqlite::params!["repo-current"],
        )?;
        conn.execute(
            "INSERT INTO artefacts_current (
                repo_id, path, content_id, symbol_id, artefact_id, language,
                extraction_fingerprint, canonical_kind, language_kind, symbol_fqn,
                parent_symbol_id, parent_artefact_id, start_line, end_line,
                start_byte, end_byte, signature, modifiers, docstring, updated_at
            ) VALUES (
                ?1, 'crates/demo/src/lib.rs', 'content-a', 'symbol::covered', 'artefact::covered', 'rust',
                'fingerprint-a', 'function', 'function_item',
                'crates/demo/src/lib.rs::covered', NULL, NULL, 10, 14,
                0, 40, NULL, '[]', NULL, '2026-05-20T10:00:00Z'
            )",
            rusqlite::params!["repo-current"],
        )?;
        conn.execute(
            "INSERT INTO artefacts_current (
                repo_id, path, content_id, symbol_id, artefact_id, language,
                extraction_fingerprint, canonical_kind, language_kind, symbol_fqn,
                parent_symbol_id, parent_artefact_id, start_line, end_line,
                start_byte, end_byte, signature, modifiers, docstring, updated_at
            ) VALUES (
                ?1, 'crates/demo/src/lib.rs', 'content-a', 'file-symbol', 'file-artefact', 'rust',
                'fingerprint-file', 'file', 'source_file',
                'crates/demo/src/lib.rs', NULL, NULL, 1, 30,
                0, 240, NULL, '[]', NULL, '2026-05-20T10:00:00Z'
            )",
            rusqlite::params!["repo-current"],
        )?;
        conn.execute(
            "INSERT INTO artefacts_current (
                repo_id, path, content_id, symbol_id, artefact_id, language,
                extraction_fingerprint, canonical_kind, language_kind, symbol_fqn,
                parent_symbol_id, parent_artefact_id, start_line, end_line,
                start_byte, end_byte, signature, modifiers, docstring, updated_at
            ) VALUES (
                ?1, 'crates/demo/src/lib_rs', 'content-b', 'symbol::underscore', 'artefact::underscore', 'rust',
                'fingerprint-b', 'function', 'function_item',
                'crates/demo/src/lib_rs::underscore', NULL, NULL, 10, 14,
                0, 40, NULL, '[]', NULL, '2026-05-20T10:00:00Z'
            )",
            rusqlite::params!["repo-current"],
        )?;
        Ok(())
    })?;

    let rows = gateway.load_current_artefacts_for_file_lines(
        "repo-current",
        "/tmp/work/crates/demo/src/lib.rs",
    )?;

    assert_eq!(rows, vec![("symbol::covered".to_string(), 10, 14)]);
    let wildcard_rows = gateway.load_current_artefacts_for_file_lines(
        "repo-current",
        "/tmp/work/crates/demo/src/libXrs",
    )?;
    assert!(
        wildcard_rows.is_empty(),
        "underscore in stored path must not act as a LIKE wildcard"
    );
    Ok(())
}

#[test]
fn current_canonical_visitors_preserve_loader_order() -> Result<()> {
    let temp = TempDir::new()?;
    let db_path = temp.path().join("runtime.sqlite");
    init_database(&db_path, false, "seed-commit")?;
    let sqlite = SqliteConnectionPool::connect_existing(db_path)?;
    let gateway = SqliteRelationalGateway::new(sqlite.clone());

    sqlite.with_write_connection(|conn| {
        conn.execute(
            "INSERT INTO repositories (repo_id, provider, organization, name, default_branch)
             VALUES (?1, 'local', 'bitloops', 'demo', 'main')",
            rusqlite::params!["repo-1"],
        )?;
        conn.execute(
            "INSERT INTO artefacts_current (
                repo_id, path, content_id, symbol_id, artefact_id, language,
                extraction_fingerprint, canonical_kind, language_kind, symbol_fqn,
                parent_symbol_id, parent_artefact_id, start_line, end_line,
                start_byte, end_byte, signature, modifiers, docstring, updated_at
            ) VALUES (
                ?1, 'src/a.rs', 'content-a', 'sym::a', 'artefact::a', 'rust',
                'fingerprint-a', 'function', 'function_item',
                'src/a.rs::a', NULL, NULL, 1, 2,
                0, 12, NULL, '[]', NULL, '2026-04-28T10:00:00Z'
            )",
            rusqlite::params!["repo-1"],
        )?;
        conn.execute(
            "INSERT INTO artefacts_current (
                repo_id, path, content_id, symbol_id, artefact_id, language,
                extraction_fingerprint, canonical_kind, language_kind, symbol_fqn,
                parent_symbol_id, parent_artefact_id, start_line, end_line,
                start_byte, end_byte, signature, modifiers, docstring, updated_at
            ) VALUES (
                ?1, 'src/b.rs', 'content-b', 'sym::b', 'artefact::b', 'rust',
                'fingerprint-b', 'function', 'function_item',
                'src/b.rs::b', NULL, NULL, 3, 4,
                12, 24, NULL, '[]', NULL, '2026-04-28T10:00:00Z'
            )",
            rusqlite::params!["repo-1"],
        )?;
        conn.execute(
            "INSERT INTO artefact_edges_current (
                repo_id, edge_id, path, content_id, from_symbol_id, from_artefact_id,
                to_symbol_id, to_artefact_id, to_symbol_ref, edge_kind, language,
                start_line, end_line, metadata, updated_at
            ) VALUES (
                ?1, 'edge-a', 'src/a.rs', 'content-a', 'sym::a', 'artefact::a',
                'sym::b', 'artefact::b', NULL, 'calls', 'rust',
                1, 1, '{}', '2026-04-28T10:00:00Z'
            )",
            rusqlite::params!["repo-1"],
        )?;
        conn.execute(
            "INSERT INTO artefact_edges_current (
                repo_id, edge_id, path, content_id, from_symbol_id, from_artefact_id,
                to_symbol_id, to_artefact_id, to_symbol_ref, edge_kind, language,
                start_line, end_line, metadata, updated_at
            ) VALUES (
                ?1, 'edge-b', 'src/b.rs', 'content-b', 'sym::b', 'artefact::b',
                NULL, NULL, 'external::c', 'calls', 'rust',
                3, 3, '{}', '2026-04-28T10:00:00Z'
            )",
            rusqlite::params!["repo-1"],
        )?;
        Ok(())
    })?;

    let mut artefact_ids = Vec::new();
    gateway.visit_current_canonical_artefacts("repo-1", &mut |artefact| {
        artefact_ids.push(artefact.artefact_id);
        Ok(())
    })?;
    assert_eq!(artefact_ids, vec!["artefact::a", "artefact::b"]);

    let mut edge_ids = Vec::new();
    gateway.visit_current_canonical_edges("repo-1", &mut |edge| {
        edge_ids.push(edge.edge_id);
        Ok(())
    })?;
    assert_eq!(edge_ids, vec!["edge-a", "edge-b"]);

    Ok(())
}

#[test]
fn current_canonical_visitors_propagate_visitor_errors() -> Result<()> {
    let temp = TempDir::new()?;
    let db_path = temp.path().join("runtime.sqlite");
    init_database(&db_path, false, "seed-commit")?;
    let sqlite = SqliteConnectionPool::connect_existing(db_path)?;
    let gateway = SqliteRelationalGateway::new(sqlite.clone());

    sqlite.with_write_connection(|conn| {
        conn.execute(
            "INSERT INTO repositories (repo_id, provider, organization, name, default_branch)
             VALUES (?1, 'local', 'bitloops', 'demo', 'main')",
            rusqlite::params!["repo-1"],
        )?;
        conn.execute(
            "INSERT INTO artefact_edges_current (
                repo_id, edge_id, path, content_id, from_symbol_id, from_artefact_id,
                to_symbol_id, to_artefact_id, to_symbol_ref, edge_kind, language,
                start_line, end_line, metadata, updated_at
            ) VALUES (
                ?1, 'edge-a', 'src/a.rs', 'content-a', 'sym::a', 'artefact::a',
                NULL, NULL, 'external::b', 'calls', 'rust',
                1, 1, '{}', '2026-04-28T10:00:00Z'
            )",
            rusqlite::params!["repo-1"],
        )?;
        Ok(())
    })?;

    let error = gateway
        .visit_current_canonical_edges("repo-1", &mut |_edge| Err(anyhow::anyhow!("stop here")))
        .expect_err("visitor failure should bubble up");
    assert!(error.to_string().contains("stop here"));

    Ok(())
}
