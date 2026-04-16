use rusqlite::Connection;
use tempfile::tempdir;

use super::fixtures::{
    desired_file_state, expected_symbol_id_by_fqn, seed_sync_repository_catalog_row,
    sqlite_relational_store_with_sync_schema, sync_test_cfg,
};

async fn materialize_cached_path(
    cfg: &crate::host::devql::DevqlConfig,
    relational: &crate::host::devql::RelationalStorage,
    path: &str,
    language: &str,
    content: &str,
    parser_version: &str,
    extractor_version: &str,
) {
    let content_id =
        crate::host::devql::sync::content_identity::compute_blob_oid(content.as_bytes());
    let desired = desired_file_state(path, language, &content_id);
    let extraction = crate::host::devql::sync::extraction::extract_to_cache_format(
        cfg,
        crate::host::devql::sync::extraction::CacheExtractionRequest {
            path,
            language,
            content_id: &content_id,
            extraction_fingerprint: &desired.extraction_fingerprint,
            parser_version,
            extractor_version,
            content,
        },
    )
    .expect("extract content into cache format")
    .expect("cache extraction should be supported");

    crate::host::devql::sync::materializer::materialize_path(
        cfg,
        relational,
        &desired,
        &extraction,
        parser_version,
        extractor_version,
    )
    .await
    .expect("materialize cached path");
}

async fn materialize_cached_rust_path(
    cfg: &crate::host::devql::DevqlConfig,
    relational: &crate::host::devql::RelationalStorage,
    path: &str,
    content: &str,
    parser_version: &str,
    extractor_version: &str,
) {
    materialize_cached_path(
        cfg,
        relational,
        path,
        "rust",
        content,
        parser_version,
        extractor_version,
    )
    .await;
}

#[tokio::test]
async fn materialize_writes_artefacts_current_with_correct_symbol_id() {
    let cfg = sync_test_cfg();
    let temp = tempdir().expect("temp dir");
    let sqlite_path = temp.path().join("devql.sqlite");
    let relational = sqlite_relational_store_with_sync_schema(&sqlite_path).await;
    seed_sync_repository_catalog_row(&relational, &cfg).await;
    let path = "src/sample.ts";
    let content = r#"import { remoteFoo } from "./remote";

class Service {
  run(): number {
    return localHelper() + remoteFoo();
  }
}

function localHelper(): number {
  return 1;
}
"#;
    let content_id =
        crate::host::devql::sync::content_identity::compute_blob_oid(content.as_bytes());
    let desired = desired_file_state(path, "typescript", &content_id);
    let extraction = crate::host::devql::sync::extraction::extract_to_cache_format(
        &cfg,
        crate::host::devql::sync::extraction::CacheExtractionRequest {
            path,
            language: "typescript",
            content_id: &content_id,
            extraction_fingerprint: &desired.extraction_fingerprint,
            parser_version: "tree-sitter-ts@1",
            extractor_version: "ts-language-pack@1",
            content,
        },
    )
    .expect("extract TypeScript content into cache format")
    .expect("TypeScript cache extraction should be supported");
    let rev = crate::host::devql::FileRevision {
        commit_sha: &content_id,
        revision: crate::host::devql::TemporalRevisionRef {
            kind: crate::host::devql::TemporalRevisionKind::Temporary,
            id: &content_id,
            temp_checkpoint_id: None,
        },
        commit_unix: 0,
        path,
        blob_sha: &content_id,
    };
    let (items, _, _) = crate::host::devql::extract_language_pack_artefacts_and_edges(
        &cfg,
        &rev,
        "typescript",
        content,
    )
    .expect("extract expected TypeScript artefacts")
    .expect("TypeScript artefacts should be supported");
    let expected_symbol_ids = expected_symbol_id_by_fqn(&items, path);

    crate::host::devql::sync::materializer::materialize_path(
        &cfg,
        &relational,
        &desired,
        &extraction,
        "tree-sitter-ts@1",
        "ts-language-pack@1",
    )
    .await
    .expect("materialize cached path");

    let db = Connection::open(&sqlite_path).expect("open sqlite db");
    let mut stmt = db
        .prepare(
            "SELECT symbol_fqn, symbol_id, artefact_id \
             FROM artefacts_current \
             WHERE repo_id = ?1 AND path = ?2 \
             ORDER BY symbol_fqn",
        )
        .expect("prepare current artefacts query");
    let rows = stmt
        .query_map([cfg.repo.repo_id.as_str(), path], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .expect("query current artefacts")
        .collect::<Result<Vec<_>, _>>()
        .expect("collect current artefacts");

    let mut expected = expected_symbol_ids
        .into_iter()
        .map(|(symbol_fqn, symbol_id)| {
            let artefact_id = crate::host::devql::revision_artefact_id(
                &cfg.repo.repo_id,
                &content_id,
                &symbol_id,
            );
            (symbol_fqn, symbol_id, artefact_id)
        })
        .collect::<Vec<_>>();
    expected.sort_by(|lhs, rhs| lhs.0.cmp(&rhs.0));

    assert_eq!(rows, expected);
}

#[tokio::test]
async fn materialize_then_re_materialize_is_idempotent() {
    let cfg = sync_test_cfg();
    let temp = tempdir().expect("temp dir");
    let sqlite_path = temp.path().join("devql.sqlite");
    let relational = sqlite_relational_store_with_sync_schema(&sqlite_path).await;
    seed_sync_repository_catalog_row(&relational, &cfg).await;
    let path = "src/sample.ts";
    let content = r#"import { remoteFoo } from "./remote";

class Service {
  run(): number {
    return localHelper() + remoteFoo();
  }
}

function localHelper(): number {
  return 1;
}
"#;
    let content_id =
        crate::host::devql::sync::content_identity::compute_blob_oid(content.as_bytes());
    let desired = desired_file_state(path, "typescript", &content_id);
    let extraction = crate::host::devql::sync::extraction::extract_to_cache_format(
        &cfg,
        crate::host::devql::sync::extraction::CacheExtractionRequest {
            path,
            language: "typescript",
            content_id: &content_id,
            extraction_fingerprint: &desired.extraction_fingerprint,
            parser_version: "tree-sitter-ts@1",
            extractor_version: "ts-language-pack@1",
            content,
        },
    )
    .expect("extract TypeScript content into cache format")
    .expect("TypeScript cache extraction should be supported");
    let rev = crate::host::devql::FileRevision {
        commit_sha: &content_id,
        revision: crate::host::devql::TemporalRevisionRef {
            kind: crate::host::devql::TemporalRevisionKind::Temporary,
            id: &content_id,
            temp_checkpoint_id: None,
        },
        commit_unix: 0,
        path,
        blob_sha: &content_id,
    };
    let (items, _, _) = crate::host::devql::extract_language_pack_artefacts_and_edges(
        &cfg,
        &rev,
        "typescript",
        content,
    )
    .expect("extract expected TypeScript artefacts")
    .expect("TypeScript artefacts should be supported");
    let expected_symbol_ids = expected_symbol_id_by_fqn(&items, path);
    let helper_symbol_id = expected_symbol_ids
        .get(&format!("{path}::localHelper"))
        .cloned()
        .expect("expected localHelper symbol id");
    let helper_artefact_id =
        crate::host::devql::revision_artefact_id(&cfg.repo.repo_id, &content_id, &helper_symbol_id);

    let load_artefacts = |db: &Connection| {
        let mut stmt = db
            .prepare(
                "SELECT symbol_fqn, symbol_id, artefact_id, parent_symbol_id, parent_artefact_id \
                 FROM artefacts_current \
                 WHERE repo_id = ?1 AND path = ?2 \
                 ORDER BY symbol_fqn",
            )
            .expect("prepare artefacts_current query");
        stmt.query_map([cfg.repo.repo_id.as_str(), path], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, Option<String>>(4)?,
            ))
        })
        .expect("query artefacts_current rows")
        .collect::<Result<Vec<_>, _>>()
        .expect("collect artefacts_current rows")
    };
    let load_edges = |db: &Connection| {
        let mut stmt = db
            .prepare(
                "SELECT edge_id, from_symbol_id, to_symbol_id, to_artefact_id, to_symbol_ref, edge_kind \
                 FROM artefact_edges_current \
                 WHERE repo_id = ?1 AND path = ?2 \
                 ORDER BY edge_id",
            )
            .expect("prepare artefact_edges_current query");
        stmt.query_map([cfg.repo.repo_id.as_str(), path], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, Option<String>>(4)?,
                row.get::<_, String>(5)?,
            ))
        })
        .expect("query artefact_edges_current rows")
        .collect::<Result<Vec<_>, _>>()
        .expect("collect artefact_edges_current rows")
    };
    let load_current_state = |db: &Connection| {
        db.query_row(
            "SELECT language, head_content_id, index_content_id, worktree_content_id, effective_content_id, effective_source, parser_version, extractor_version, exists_in_head, exists_in_index, exists_in_worktree \
             FROM current_file_state \
             WHERE repo_id = ?1 AND path = ?2",
            [cfg.repo.repo_id.as_str(), path],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, String>(7)?,
                    row.get::<_, i64>(8)?,
                    row.get::<_, i64>(9)?,
                    row.get::<_, i64>(10)?,
                ))
            },
        )
        .expect("load current_file_state row")
    };

    crate::host::devql::sync::materializer::materialize_path(
        &cfg,
        &relational,
        &desired,
        &extraction,
        "tree-sitter-ts@1",
        "ts-language-pack@1",
    )
    .await
    .expect("materialize cached path first time");

    let db = Connection::open(&sqlite_path).expect("open sqlite db");
    let first_artefacts = load_artefacts(&db);
    let first_edges = load_edges(&db);
    let first_state = load_current_state(&db);

    crate::host::devql::sync::materializer::materialize_path(
        &cfg,
        &relational,
        &desired,
        &extraction,
        "tree-sitter-ts@1",
        "ts-language-pack@1",
    )
    .await
    .expect("materialize cached path second time");

    let second_artefacts = load_artefacts(&db);
    let second_edges = load_edges(&db);
    let second_state = load_current_state(&db);

    assert_eq!(first_artefacts, second_artefacts);
    assert_eq!(first_edges, second_edges);
    assert_eq!(first_state, second_state);
    assert_eq!(first_artefacts.len(), extraction.artefacts.len());
    assert_eq!(first_edges.len(), extraction.edges.len());
    assert_eq!(
        first_state,
        (
            "typescript".to_string(),
            Some(content_id.clone()),
            Some(content_id.clone()),
            Some(content_id.clone()),
            content_id.clone(),
            "head".to_string(),
            "tree-sitter-ts@1".to_string(),
            "ts-language-pack@1".to_string(),
            1,
            1,
            1,
        )
    );
    assert!(
        first_edges.iter().any(|edge| {
            edge.2.as_deref() == Some(helper_symbol_id.as_str())
                && edge.3.as_deref() == Some(helper_artefact_id.as_str())
                && edge.5 == "calls"
        }),
        "same-file call edge should resolve through cached artifact_key mapping"
    );
}

#[tokio::test]
async fn materialize_reuses_cached_extraction_at_new_path_with_path_sensitive_identity() {
    let cfg = sync_test_cfg();
    let temp = tempdir().expect("temp dir");
    let sqlite_path = temp.path().join("devql.sqlite");
    let relational = sqlite_relational_store_with_sync_schema(&sqlite_path).await;
    seed_sync_repository_catalog_row(&relational, &cfg).await;
    let original_path = "src/sample.ts";
    let materialized_path = "nested/other.ts";
    let content = r#"import { remoteFoo } from "./remote";

class Service {
  run(): number {
    return localHelper() + remoteFoo();
  }
}

function localHelper(): number {
  return 1;
}
"#;
    let content_id =
        crate::host::devql::sync::content_identity::compute_blob_oid(content.as_bytes());
    let desired = desired_file_state(materialized_path, "typescript", &content_id);
    let extraction = crate::host::devql::sync::extraction::extract_to_cache_format(
        &cfg,
        crate::host::devql::sync::extraction::CacheExtractionRequest {
            path: original_path,
            language: "typescript",
            content_id: &content_id,
            extraction_fingerprint: &desired.extraction_fingerprint,
            parser_version: "tree-sitter-ts@1",
            extractor_version: "ts-language-pack@1",
            content,
        },
    )
    .expect("extract original TypeScript content into cache format")
    .expect("original TypeScript cache extraction should be supported");
    let rev = crate::host::devql::FileRevision {
        commit_sha: &content_id,
        revision: crate::host::devql::TemporalRevisionRef {
            kind: crate::host::devql::TemporalRevisionKind::Temporary,
            id: &content_id,
            temp_checkpoint_id: None,
        },
        commit_unix: 0,
        path: materialized_path,
        blob_sha: &content_id,
    };
    let (items, _, _) = crate::host::devql::extract_language_pack_artefacts_and_edges(
        &cfg,
        &rev,
        "typescript",
        content,
    )
    .expect("extract expected TypeScript artefacts for new path")
    .expect("TypeScript artefacts for new path should be supported");
    let expected_symbol_ids = expected_symbol_id_by_fqn(&items, materialized_path);

    crate::host::devql::sync::materializer::materialize_path(
        &cfg,
        &relational,
        &desired,
        &extraction,
        "tree-sitter-ts@1",
        "ts-language-pack@1",
    )
    .await
    .expect("materialize cached extraction at new path");

    let db = Connection::open(&sqlite_path).expect("open sqlite db");
    let mut stmt = db
        .prepare(
            "SELECT symbol_fqn, symbol_id, artefact_id \
             FROM artefacts_current \
             WHERE repo_id = ?1 AND path = ?2 \
             ORDER BY symbol_fqn",
        )
        .expect("prepare current artefacts query");
    let rows = stmt
        .query_map([cfg.repo.repo_id.as_str(), materialized_path], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .expect("query current artefacts")
        .collect::<Result<Vec<_>, _>>()
        .expect("collect current artefacts");

    let mut expected = expected_symbol_ids
        .into_iter()
        .map(|(symbol_fqn, symbol_id)| {
            let artefact_id = crate::host::devql::revision_artefact_id(
                &cfg.repo.repo_id,
                &content_id,
                &symbol_id,
            );
            (symbol_fqn, symbol_id, artefact_id)
        })
        .collect::<Vec<_>>();
    expected.sort_by(|lhs, rhs| lhs.0.cmp(&rhs.0));

    assert!(
        rows.iter()
            .all(|(symbol_fqn, _, _)| symbol_fqn.starts_with(materialized_path)),
        "all symbol_fqn values should be re-derived from the materialization path"
    );
    assert!(
        rows.iter()
            .all(|(symbol_fqn, _, _)| !symbol_fqn.starts_with(original_path)),
        "stored symbol_fqn values should not retain the cached source path"
    );
    assert_eq!(rows, expected);
}

#[tokio::test]
async fn remove_path_deletes_all_rows() {
    let cfg = sync_test_cfg();
    let temp = tempdir().expect("temp dir");
    let sqlite_path = temp.path().join("devql.sqlite");
    let relational = sqlite_relational_store_with_sync_schema(&sqlite_path).await;
    seed_sync_repository_catalog_row(&relational, &cfg).await;
    let path = "src/sample.ts";
    let content = r#"import { remoteFoo } from "./remote";

class Service {
  run(): number {
    return localHelper() + remoteFoo();
  }
}

function localHelper(): number {
  return 1;
}
"#;
    let content_id =
        crate::host::devql::sync::content_identity::compute_blob_oid(content.as_bytes());
    let desired = desired_file_state(path, "typescript", &content_id);
    let extraction = crate::host::devql::sync::extraction::extract_to_cache_format(
        &cfg,
        crate::host::devql::sync::extraction::CacheExtractionRequest {
            path,
            language: "typescript",
            content_id: &content_id,
            extraction_fingerprint: &desired.extraction_fingerprint,
            parser_version: "tree-sitter-ts@1",
            extractor_version: "ts-language-pack@1",
            content,
        },
    )
    .expect("extract TypeScript content into cache format")
    .expect("TypeScript cache extraction should be supported");

    crate::host::devql::sync::materializer::materialize_path(
        &cfg,
        &relational,
        &desired,
        &extraction,
        "tree-sitter-ts@1",
        "ts-language-pack@1",
    )
    .await
    .expect("materialize cached path");
    crate::host::devql::sync::materializer::remove_path(&cfg, &relational, path)
        .await
        .expect("remove materialized path");

    let db = Connection::open(&sqlite_path).expect("open sqlite db");
    let artefact_count: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM artefacts_current WHERE repo_id = ?1 AND path = ?2",
            [cfg.repo.repo_id.as_str(), path],
            |row| row.get(0),
        )
        .expect("count artefacts_current rows");
    let edge_count: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM artefact_edges_current WHERE repo_id = ?1 AND path = ?2",
            [cfg.repo.repo_id.as_str(), path],
            |row| row.get(0),
        )
        .expect("count artefact_edges_current rows");
    let current_file_state_count: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM current_file_state WHERE repo_id = ?1 AND path = ?2",
            [cfg.repo.repo_id.as_str(), path],
            |row| row.get(0),
        )
        .expect("count current_file_state rows");

    assert_eq!(artefact_count, 0);
    assert_eq!(edge_count, 0);
    assert_eq!(current_file_state_count, 0);
}

#[tokio::test]
async fn materialize_resolves_rust_explicit_local_call_targets_across_files() {
    let cfg = sync_test_cfg();
    let temp = tempdir().expect("temp dir");
    let sqlite_path = temp.path().join("devql.sqlite");
    let relational = sqlite_relational_store_with_sync_schema(&sqlite_path).await;
    seed_sync_repository_catalog_row(&relational, &cfg).await;

    let parser_version = "tree-sitter-rust@1";
    let extractor_version = "rust-language-pack@1";
    let helper_path = "crates/ruff_linter/src/rules/pyflakes/fixes.rs";
    let caller_path = "crates/ruff_linter/src/rules/pyflakes/rules/strings.rs";
    let helper_content = r#"pub(crate) fn remove_unused_positional_arguments_from_format_call() {}
"#;
    let caller_content = r#"use super::super::fixes::remove_unused_positional_arguments_from_format_call;

fn string_dot_format_extra_positional_arguments() {
    remove_unused_positional_arguments_from_format_call();
}
"#;

    materialize_cached_rust_path(
        &cfg,
        &relational,
        helper_path,
        helper_content,
        parser_version,
        extractor_version,
    )
    .await;
    materialize_cached_rust_path(
        &cfg,
        &relational,
        caller_path,
        caller_content,
        parser_version,
        extractor_version,
    )
    .await;

    let db = Connection::open(&sqlite_path).expect("open sqlite db");
    let helper_symbol_fqn =
        format!("{helper_path}::remove_unused_positional_arguments_from_format_call");
    let helper_row: (String, String) = db
        .query_row(
            "SELECT symbol_id, artefact_id FROM artefacts_current WHERE repo_id = ?1 AND symbol_fqn = ?2",
            [cfg.repo.repo_id.as_str(), helper_symbol_fqn.as_str()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("load helper artefact row");
    let call_edge: (Option<String>, Option<String>, Option<String>) = db
        .query_row(
            "SELECT to_symbol_id, to_artefact_id, to_symbol_ref \
             FROM artefact_edges_current \
             WHERE repo_id = ?1 AND path = ?2 AND edge_kind = 'calls'",
            [cfg.repo.repo_id.as_str(), caller_path],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .expect("load caller call edge");

    assert_eq!(call_edge.0.as_deref(), Some(helper_row.0.as_str()));
    assert_eq!(call_edge.1.as_deref(), Some(helper_row.1.as_str()));
    assert_eq!(call_edge.2.as_deref(), Some(helper_symbol_fqn.as_str()));

    let mut stmt = db
        .prepare(
            "SELECT af.symbol_fqn \
             FROM artefact_edges_current e \
             JOIN artefacts_current af \
               ON af.repo_id = e.repo_id AND af.artefact_id = e.from_artefact_id \
             JOIN artefacts_current at \
               ON at.repo_id = e.repo_id AND at.artefact_id = e.to_artefact_id \
             WHERE e.repo_id = ?1 AND e.edge_kind = 'calls' AND at.symbol_fqn = ?2 \
             ORDER BY af.symbol_fqn",
        )
        .expect("prepare inbound call query");
    let inbound_callers = stmt
        .query_map(
            [cfg.repo.repo_id.as_str(), helper_symbol_fqn.as_str()],
            |row| row.get::<_, String>(0),
        )
        .expect("query inbound call rows")
        .collect::<Result<Vec<_>, _>>()
        .expect("collect inbound call rows");

    assert_eq!(
        inbound_callers,
        vec![format!(
            "{caller_path}::string_dot_format_extra_positional_arguments"
        )]
    );
}

#[tokio::test]
async fn materialize_resolves_rust_explicit_local_export_targets_across_files() {
    let cfg = sync_test_cfg();
    let temp = tempdir().expect("temp dir");
    let sqlite_path = temp.path().join("devql.sqlite");
    let relational = sqlite_relational_store_with_sync_schema(&sqlite_path).await;
    seed_sync_repository_catalog_row(&relational, &cfg).await;

    let parser_version = "tree-sitter-rust@1";
    let extractor_version = "rust-language-pack@1";
    let support_path = "src/support.rs";
    let lib_path = "src/lib.rs";
    let support_content = r#"pub struct Thing;
"#;
    let lib_content = r#"pub use crate::support::Thing;
"#;

    materialize_cached_rust_path(
        &cfg,
        &relational,
        support_path,
        support_content,
        parser_version,
        extractor_version,
    )
    .await;
    materialize_cached_rust_path(
        &cfg,
        &relational,
        lib_path,
        lib_content,
        parser_version,
        extractor_version,
    )
    .await;

    let db = Connection::open(&sqlite_path).expect("open sqlite db");
    let support_symbol_fqn = format!("{support_path}::Thing");
    let support_row: (String, String) = db
        .query_row(
            "SELECT symbol_id, artefact_id FROM artefacts_current WHERE repo_id = ?1 AND symbol_fqn = ?2",
            [cfg.repo.repo_id.as_str(), support_symbol_fqn.as_str()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("load exported support artefact row");
    let export_edge: (Option<String>, Option<String>, Option<String>) = db
        .query_row(
            "SELECT to_symbol_id, to_artefact_id, to_symbol_ref \
             FROM artefact_edges_current \
             WHERE repo_id = ?1 AND path = ?2 AND edge_kind = 'exports'",
            [cfg.repo.repo_id.as_str(), lib_path],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .expect("load export edge");

    assert_eq!(export_edge.0.as_deref(), Some(support_row.0.as_str()));
    assert_eq!(export_edge.1.as_deref(), Some(support_row.1.as_str()));
    assert_eq!(export_edge.2.as_deref(), Some(support_symbol_fqn.as_str()));
}

#[tokio::test]
async fn materialize_resolves_typescript_relative_import_call_targets_across_files() {
    let cfg = sync_test_cfg();
    let temp = tempdir().expect("temp dir");
    let sqlite_path = temp.path().join("devql.sqlite");
    let relational = sqlite_relational_store_with_sync_schema(&sqlite_path).await;
    seed_sync_repository_catalog_row(&relational, &cfg).await;

    let parser_version = "tree-sitter-ts@1";
    let extractor_version = "ts-language-pack@1";
    let helper_path = "src/utils.ts";
    let caller_path = "src/caller.ts";
    let helper_content = r#"export function helper(): number {
  return 1;
}
"#;
    let caller_content = r#"import { helper } from "./utils";

export function caller(): number {
  return helper();
}
"#;

    materialize_cached_path(
        &cfg,
        &relational,
        helper_path,
        "typescript",
        helper_content,
        parser_version,
        extractor_version,
    )
    .await;
    materialize_cached_path(
        &cfg,
        &relational,
        caller_path,
        "typescript",
        caller_content,
        parser_version,
        extractor_version,
    )
    .await;

    let db = Connection::open(&sqlite_path).expect("open sqlite db");
    let helper_symbol_fqn = format!("{helper_path}::helper");
    let helper_row: (String, String) = db
        .query_row(
            "SELECT symbol_id, artefact_id FROM artefacts_current WHERE repo_id = ?1 AND symbol_fqn = ?2",
            [cfg.repo.repo_id.as_str(), helper_symbol_fqn.as_str()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("load helper artefact row");
    let call_edge: (Option<String>, Option<String>, Option<String>) = db
        .query_row(
            "SELECT to_symbol_id, to_artefact_id, to_symbol_ref \
             FROM artefact_edges_current \
             WHERE repo_id = ?1 AND path = ?2 AND edge_kind = 'calls'",
            [cfg.repo.repo_id.as_str(), caller_path],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .expect("load caller call edge");

    assert_eq!(call_edge.0.as_deref(), Some(helper_row.0.as_str()));
    assert_eq!(call_edge.1.as_deref(), Some(helper_row.1.as_str()));
    assert_eq!(call_edge.2.as_deref(), Some(helper_symbol_fqn.as_str()));

    let mut stmt = db
        .prepare(
            "SELECT af.symbol_fqn \
             FROM artefact_edges_current e \
             JOIN artefacts_current af \
               ON af.repo_id = e.repo_id AND af.artefact_id = e.from_artefact_id \
             JOIN artefacts_current at \
               ON at.repo_id = e.repo_id AND at.artefact_id = e.to_artefact_id \
             WHERE e.repo_id = ?1 AND e.edge_kind = 'calls' AND at.symbol_fqn = ?2 \
             ORDER BY af.symbol_fqn",
        )
        .expect("prepare inbound call query");
    let inbound_callers = stmt
        .query_map(
            [cfg.repo.repo_id.as_str(), helper_symbol_fqn.as_str()],
            |row| row.get::<_, String>(0),
        )
        .expect("query inbound call rows")
        .collect::<Result<Vec<_>, _>>()
        .expect("collect inbound call rows");

    assert_eq!(inbound_callers, vec![format!("{caller_path}::caller")]);
}

#[tokio::test]
async fn materialize_resolves_typescript_relative_import_edges_to_file_targets() {
    let cfg = sync_test_cfg();
    let temp = tempdir().expect("temp dir");
    let sqlite_path = temp.path().join("devql.sqlite");
    let relational = sqlite_relational_store_with_sync_schema(&sqlite_path).await;
    seed_sync_repository_catalog_row(&relational, &cfg).await;

    let parser_version = "tree-sitter-ts@1";
    let extractor_version = "ts-language-pack@1";
    let caller_path = "src/caller.ts";
    let helper_path = "src/utils.ts";
    let caller_content = r#"import { helper } from "./utils";

export function caller(): number {
  return helper();
}
"#;
    let helper_content = r#"export function helper(): number {
  return 1;
}
"#;

    materialize_cached_path(
        &cfg,
        &relational,
        helper_path,
        "typescript",
        helper_content,
        parser_version,
        extractor_version,
    )
    .await;
    materialize_cached_path(
        &cfg,
        &relational,
        caller_path,
        "typescript",
        caller_content,
        parser_version,
        extractor_version,
    )
    .await;

    let db = Connection::open(&sqlite_path).expect("open sqlite db");
    let helper_row: (String, String) = db
        .query_row(
            "SELECT symbol_id, artefact_id FROM artefacts_current WHERE repo_id = ?1 AND symbol_fqn = ?2",
            [cfg.repo.repo_id.as_str(), helper_path],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("load imported file artefact row");
    let import_edge: (Option<String>, Option<String>, Option<String>) = db
        .query_row(
            "SELECT to_symbol_id, to_artefact_id, to_symbol_ref \
             FROM artefact_edges_current \
             WHERE repo_id = ?1 AND path = ?2 AND edge_kind = 'imports'",
            [cfg.repo.repo_id.as_str(), caller_path],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .expect("load file import edge");

    assert_eq!(import_edge.0.as_deref(), Some(helper_row.0.as_str()));
    assert_eq!(import_edge.1.as_deref(), Some(helper_row.1.as_str()));
    assert_eq!(import_edge.2.as_deref(), Some(helper_path));
}

#[tokio::test]
async fn materialize_reconciles_previously_unresolved_typescript_edge_when_target_is_added_later() {
    let cfg = sync_test_cfg();
    let temp = tempdir().expect("temp dir");
    let sqlite_path = temp.path().join("devql.sqlite");
    let relational = sqlite_relational_store_with_sync_schema(&sqlite_path).await;
    seed_sync_repository_catalog_row(&relational, &cfg).await;

    let parser_version = "tree-sitter-ts@1";
    let extractor_version = "ts-language-pack@1";
    let caller_path = "src/caller.ts";
    let helper_path = "src/utils.ts";
    let caller_content = r#"import { helper } from "./utils";

export function caller(): number {
  return helper();
}
"#;
    let helper_content = r#"export function helper(): number {
  return 1;
}
"#;

    materialize_cached_path(
        &cfg,
        &relational,
        caller_path,
        "typescript",
        caller_content,
        parser_version,
        extractor_version,
    )
    .await;
    materialize_cached_path(
        &cfg,
        &relational,
        helper_path,
        "typescript",
        helper_content,
        parser_version,
        extractor_version,
    )
    .await;

    let db = Connection::open(&sqlite_path).expect("open sqlite db");
    let helper_symbol_fqn = format!("{helper_path}::helper");
    let helper_row: (String, String) = db
        .query_row(
            "SELECT symbol_id, artefact_id FROM artefacts_current WHERE repo_id = ?1 AND symbol_fqn = ?2",
            [cfg.repo.repo_id.as_str(), helper_symbol_fqn.as_str()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("load helper artefact row");
    let call_edge: (Option<String>, Option<String>, Option<String>) = db
        .query_row(
            "SELECT to_symbol_id, to_artefact_id, to_symbol_ref \
             FROM artefact_edges_current \
             WHERE repo_id = ?1 AND path = ?2 AND edge_kind = 'calls'",
            [cfg.repo.repo_id.as_str(), caller_path],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .expect("load caller call edge after helper materialization");

    assert_eq!(call_edge.0.as_deref(), Some(helper_row.0.as_str()));
    assert_eq!(call_edge.1.as_deref(), Some(helper_row.1.as_str()));
    assert_eq!(call_edge.2.as_deref(), Some(helper_symbol_fqn.as_str()));
}

#[tokio::test]
async fn materialize_resolves_python_import_edges_to_module_files() {
    let cfg = sync_test_cfg();
    let temp = tempdir().expect("temp dir");
    let sqlite_path = temp.path().join("devql.sqlite");
    let relational = sqlite_relational_store_with_sync_schema(&sqlite_path).await;
    seed_sync_repository_catalog_row(&relational, &cfg).await;

    let parser_version = "tree-sitter-python@1";
    let extractor_version = "python-language-pack@1";
    let helper_path = "pkg/helpers.py";
    let caller_path = "pkg/main.py";
    let helper_content = "def helper():\n    return 1\n";
    let caller_content = "from pkg.helpers import helper\n\ndef caller():\n    return helper()\n";

    materialize_cached_path(
        &cfg,
        &relational,
        helper_path,
        "python",
        helper_content,
        parser_version,
        extractor_version,
    )
    .await;
    materialize_cached_path(
        &cfg,
        &relational,
        caller_path,
        "python",
        caller_content,
        parser_version,
        extractor_version,
    )
    .await;

    let db = Connection::open(&sqlite_path).expect("open sqlite db");
    let helper_row: (String, String) = db
        .query_row(
            "SELECT symbol_id, artefact_id FROM artefacts_current WHERE repo_id = ?1 AND symbol_fqn = ?2",
            [cfg.repo.repo_id.as_str(), helper_path],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("load imported module artefact row");
    let import_edge: (Option<String>, Option<String>, Option<String>) = db
        .query_row(
            "SELECT to_symbol_id, to_artefact_id, to_symbol_ref \
             FROM artefact_edges_current \
             WHERE repo_id = ?1 AND path = ?2 AND edge_kind = 'imports'",
            [cfg.repo.repo_id.as_str(), caller_path],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .expect("load module import edge");

    assert_eq!(import_edge.0.as_deref(), Some(helper_row.0.as_str()));
    assert_eq!(import_edge.1.as_deref(), Some(helper_row.1.as_str()));
    assert_eq!(import_edge.2.as_deref(), Some(helper_path));
}

#[tokio::test]
async fn materialize_resolves_python_imported_call_targets_across_files() {
    let cfg = sync_test_cfg();
    let temp = tempdir().expect("temp dir");
    let sqlite_path = temp.path().join("devql.sqlite");
    let relational = sqlite_relational_store_with_sync_schema(&sqlite_path).await;
    seed_sync_repository_catalog_row(&relational, &cfg).await;

    let parser_version = "tree-sitter-python@1";
    let extractor_version = "python-language-pack@1";
    let helper_path = "pkg/helpers.py";
    let caller_path = "pkg/main.py";
    let helper_content = "def helper():\n    return 1\n";
    let caller_content = "from pkg.helpers import helper\n\ndef caller():\n    return helper()\n";

    materialize_cached_path(
        &cfg,
        &relational,
        helper_path,
        "python",
        helper_content,
        parser_version,
        extractor_version,
    )
    .await;
    materialize_cached_path(
        &cfg,
        &relational,
        caller_path,
        "python",
        caller_content,
        parser_version,
        extractor_version,
    )
    .await;

    let db = Connection::open(&sqlite_path).expect("open sqlite db");
    let helper_symbol_fqn = format!("{helper_path}::helper");
    let helper_row: (String, String) = db
        .query_row(
            "SELECT symbol_id, artefact_id FROM artefacts_current WHERE repo_id = ?1 AND symbol_fqn = ?2",
            [cfg.repo.repo_id.as_str(), helper_symbol_fqn.as_str()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("load helper artefact row");
    let call_edge: (Option<String>, Option<String>, Option<String>) = db
        .query_row(
            "SELECT to_symbol_id, to_artefact_id, to_symbol_ref \
             FROM artefact_edges_current \
             WHERE repo_id = ?1 AND path = ?2 AND edge_kind = 'calls'",
            [cfg.repo.repo_id.as_str(), caller_path],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .expect("load caller call edge");

    assert_eq!(call_edge.0.as_deref(), Some(helper_row.0.as_str()));
    assert_eq!(call_edge.1.as_deref(), Some(helper_row.1.as_str()));
    assert_eq!(call_edge.2.as_deref(), Some(helper_symbol_fqn.as_str()));
}

#[tokio::test]
async fn materialize_resolves_java_import_edges_to_local_types() {
    let cfg = sync_test_cfg();
    let temp = tempdir().expect("temp dir");
    let sqlite_path = temp.path().join("devql.sqlite");
    let relational = sqlite_relational_store_with_sync_schema(&sqlite_path).await;
    seed_sync_repository_catalog_row(&relational, &cfg).await;

    let parser_version = "tree-sitter-java@1";
    let extractor_version = "java-language-pack@1";
    let helper_path = "src/com/acme/Util.java";
    let caller_path = "src/com/acme/Greeter.java";
    let helper_content = r#"package com.acme;

class Util {
    static void helper() {}
}
"#;
    let caller_content = r#"package com.acme;

import com.acme.Util;

class Greeter {
    void greet() {
        Util.helper();
    }
}
"#;

    materialize_cached_path(
        &cfg,
        &relational,
        helper_path,
        "java",
        helper_content,
        parser_version,
        extractor_version,
    )
    .await;
    materialize_cached_path(
        &cfg,
        &relational,
        caller_path,
        "java",
        caller_content,
        parser_version,
        extractor_version,
    )
    .await;

    let db = Connection::open(&sqlite_path).expect("open sqlite db");
    let target_symbol_fqn = format!("{helper_path}::Util");
    let target_row: (String, String) = db
        .query_row(
            "SELECT symbol_id, artefact_id FROM artefacts_current WHERE repo_id = ?1 AND symbol_fqn = ?2",
            [cfg.repo.repo_id.as_str(), target_symbol_fqn.as_str()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("load imported type artefact row");
    let import_edge: (Option<String>, Option<String>, Option<String>) = db
        .query_row(
            "SELECT to_symbol_id, to_artefact_id, to_symbol_ref \
             FROM artefact_edges_current \
             WHERE repo_id = ?1 AND path = ?2 AND edge_kind = 'imports'",
            [cfg.repo.repo_id.as_str(), caller_path],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .expect("load type import edge");

    assert_eq!(import_edge.0.as_deref(), Some(target_row.0.as_str()));
    assert_eq!(import_edge.1.as_deref(), Some(target_row.1.as_str()));
    assert_eq!(import_edge.2.as_deref(), Some(target_symbol_fqn.as_str()));
}

#[tokio::test]
async fn materialize_resolves_go_same_package_call_targets_across_files() {
    let cfg = sync_test_cfg();
    let temp = tempdir().expect("temp dir");
    let sqlite_path = temp.path().join("devql.sqlite");
    let relational = sqlite_relational_store_with_sync_schema(&sqlite_path).await;
    seed_sync_repository_catalog_row(&relational, &cfg).await;

    let parser_version = "tree-sitter-go@1";
    let extractor_version = "go-language-pack@1";
    let helper_path = "service/helper.go";
    let caller_path = "service/run.go";
    let helper_content = "package service\n\nfunc helper() {}\n";
    let caller_content = "package service\n\nfunc run() {\n\thelper()\n}\n";

    materialize_cached_path(
        &cfg,
        &relational,
        helper_path,
        "go",
        helper_content,
        parser_version,
        extractor_version,
    )
    .await;
    materialize_cached_path(
        &cfg,
        &relational,
        caller_path,
        "go",
        caller_content,
        parser_version,
        extractor_version,
    )
    .await;

    let db = Connection::open(&sqlite_path).expect("open sqlite db");
    let helper_symbol_fqn = format!("{helper_path}::helper");
    let helper_row: (String, String) = db
        .query_row(
            "SELECT symbol_id, artefact_id FROM artefacts_current WHERE repo_id = ?1 AND symbol_fqn = ?2",
            [cfg.repo.repo_id.as_str(), helper_symbol_fqn.as_str()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("load helper artefact row");
    let call_edge: (Option<String>, Option<String>, Option<String>) = db
        .query_row(
            "SELECT to_symbol_id, to_artefact_id, to_symbol_ref \
             FROM artefact_edges_current \
             WHERE repo_id = ?1 AND path = ?2 AND edge_kind = 'calls'",
            [cfg.repo.repo_id.as_str(), caller_path],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .expect("load caller call edge");

    assert_eq!(call_edge.0.as_deref(), Some(helper_row.0.as_str()));
    assert_eq!(call_edge.1.as_deref(), Some(helper_row.1.as_str()));
    assert_eq!(call_edge.2.as_deref(), Some(helper_symbol_fqn.as_str()));
}

#[tokio::test]
async fn materialize_resolves_csharp_using_edges_to_namespaces() {
    let cfg = sync_test_cfg();
    let temp = tempdir().expect("temp dir");
    let sqlite_path = temp.path().join("devql.sqlite");
    let relational = sqlite_relational_store_with_sync_schema(&sqlite_path).await;
    seed_sync_repository_catalog_row(&relational, &cfg).await;

    let parser_version = "tree-sitter-csharp@1";
    let extractor_version = "csharp-language-pack@1";
    let helper_path = "src/BaseService.cs";
    let caller_path = "src/UserService.cs";
    let helper_content = r#"namespace MyApp.Services;

public class BaseService {}
"#;
    let caller_content = r#"using MyApp.Services;

namespace MyApp.Features;

public class UserService : BaseService
{
}
"#;

    materialize_cached_path(
        &cfg,
        &relational,
        helper_path,
        "csharp",
        helper_content,
        parser_version,
        extractor_version,
    )
    .await;
    materialize_cached_path(
        &cfg,
        &relational,
        caller_path,
        "csharp",
        caller_content,
        parser_version,
        extractor_version,
    )
    .await;

    let db = Connection::open(&sqlite_path).expect("open sqlite db");
    let target_symbol_fqn = format!("{helper_path}::ns::MyApp.Services");
    let target_row: (String, String) = db
        .query_row(
            "SELECT symbol_id, artefact_id FROM artefacts_current WHERE repo_id = ?1 AND symbol_fqn = ?2",
            [cfg.repo.repo_id.as_str(), target_symbol_fqn.as_str()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("load imported namespace artefact row");
    let import_edge: (Option<String>, Option<String>, Option<String>) = db
        .query_row(
            "SELECT to_symbol_id, to_artefact_id, to_symbol_ref \
             FROM artefact_edges_current \
             WHERE repo_id = ?1 AND path = ?2 AND edge_kind = 'imports'",
            [cfg.repo.repo_id.as_str(), caller_path],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .expect("load namespace import edge");

    assert_eq!(import_edge.0.as_deref(), Some(target_row.0.as_str()));
    assert_eq!(import_edge.1.as_deref(), Some(target_row.1.as_str()));
    assert_eq!(import_edge.2.as_deref(), Some(target_symbol_fqn.as_str()));
}

#[tokio::test]
async fn materialize_resolves_java_imported_type_call_targets_across_files() {
    let cfg = sync_test_cfg();
    let temp = tempdir().expect("temp dir");
    let sqlite_path = temp.path().join("devql.sqlite");
    let relational = sqlite_relational_store_with_sync_schema(&sqlite_path).await;
    seed_sync_repository_catalog_row(&relational, &cfg).await;

    let parser_version = "tree-sitter-java@1";
    let extractor_version = "java-language-pack@1";
    let helper_path = "src/com/acme/Util.java";
    let caller_path = "src/com/acme/Greeter.java";
    let helper_content = r#"package com.acme;

class Util {
    static void helper() {}
}
"#;
    let caller_content = r#"package com.acme;

import com.acme.Util;

class Greeter {
    void greet() {
        Util.helper();
    }
}
"#;

    materialize_cached_path(
        &cfg,
        &relational,
        helper_path,
        "java",
        helper_content,
        parser_version,
        extractor_version,
    )
    .await;
    materialize_cached_path(
        &cfg,
        &relational,
        caller_path,
        "java",
        caller_content,
        parser_version,
        extractor_version,
    )
    .await;

    let db = Connection::open(&sqlite_path).expect("open sqlite db");
    let helper_symbol_fqn = format!("{helper_path}::Util::helper");
    let helper_row: (String, String) = db
        .query_row(
            "SELECT symbol_id, artefact_id FROM artefacts_current WHERE repo_id = ?1 AND symbol_fqn = ?2",
            [cfg.repo.repo_id.as_str(), helper_symbol_fqn.as_str()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("load helper artefact row");
    let call_edge: (Option<String>, Option<String>, Option<String>) = db
        .query_row(
            "SELECT to_symbol_id, to_artefact_id, to_symbol_ref \
             FROM artefact_edges_current \
             WHERE repo_id = ?1 AND path = ?2 AND edge_kind = 'calls'",
            [cfg.repo.repo_id.as_str(), caller_path],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .expect("load caller call edge");

    assert_eq!(call_edge.0.as_deref(), Some(helper_row.0.as_str()));
    assert_eq!(call_edge.1.as_deref(), Some(helper_row.1.as_str()));
    assert_eq!(call_edge.2.as_deref(), Some(helper_symbol_fqn.as_str()));
}

#[tokio::test]
async fn materialize_expands_grouped_rust_import_edges_into_resolved_local_targets() {
    let cfg = sync_test_cfg();
    let temp = tempdir().expect("temp dir");
    let sqlite_path = temp.path().join("devql.sqlite");
    let relational = sqlite_relational_store_with_sync_schema(&sqlite_path).await;
    seed_sync_repository_catalog_row(&relational, &cfg).await;

    let parser_version = "tree-sitter-rust@1";
    let extractor_version = "rust-language-pack@1";
    let helper_path = "crates/ruff_linter/src/rules/pyflakes/fixes.rs";
    let caller_path = "crates/ruff_linter/src/rules/pyflakes/rules/strings.rs";
    let helper_content = "pub(crate) fn remove_unused_positional_arguments_from_format_call() {}\n";
    let caller_content = r#"use super::super::fixes::{remove_unused_positional_arguments_from_format_call, self};

pub(crate) fn string_dot_format_extra_positional_arguments() {
    remove_unused_positional_arguments_from_format_call();
}
"#;

    materialize_cached_path(
        &cfg,
        &relational,
        helper_path,
        "rust",
        helper_content,
        parser_version,
        extractor_version,
    )
    .await;
    materialize_cached_path(
        &cfg,
        &relational,
        caller_path,
        "rust",
        caller_content,
        parser_version,
        extractor_version,
    )
    .await;

    let db = Connection::open(&sqlite_path).expect("open sqlite db");
    let refs = {
        let mut stmt = db
            .prepare(
                "SELECT to_symbol_ref \
                 FROM artefact_edges_current \
                 WHERE repo_id = ?1 AND path = ?2 AND edge_kind = 'imports' \
                 ORDER BY to_symbol_ref",
            )
            .expect("prepare rust import edge query");
        stmt.query_map([cfg.repo.repo_id.as_str(), caller_path], |row| {
            row.get::<_, Option<String>>(0)
        })
        .expect("query rust import edges")
        .collect::<Result<Vec<_>, _>>()
        .expect("collect rust import edges")
    };

    assert_eq!(
        refs,
        vec![
            Some(helper_path.to_string()),
            Some(format!(
                "{helper_path}::remove_unused_positional_arguments_from_format_call"
            )),
        ]
    );
}

#[tokio::test]
async fn materialize_resolves_csharp_same_namespace_type_targets_across_files() {
    let cfg = sync_test_cfg();
    let temp = tempdir().expect("temp dir");
    let sqlite_path = temp.path().join("devql.sqlite");
    let relational = sqlite_relational_store_with_sync_schema(&sqlite_path).await;
    seed_sync_repository_catalog_row(&relational, &cfg).await;

    let parser_version = "tree-sitter-csharp@1";
    let extractor_version = "csharp-language-pack@1";
    let base_path = "src/BaseService.cs";
    let interface_path = "src/IRepository.cs";
    let caller_path = "src/UserService.cs";
    let base_content = r#"namespace MyApp.Services;

public class BaseService {}
"#;
    let interface_content = r#"namespace MyApp.Services;

public interface IRepository {}
"#;
    let caller_content = r#"namespace MyApp.Services;

public class UserService : BaseService, IRepository
{
}
"#;

    materialize_cached_path(
        &cfg,
        &relational,
        base_path,
        "csharp",
        base_content,
        parser_version,
        extractor_version,
    )
    .await;
    materialize_cached_path(
        &cfg,
        &relational,
        interface_path,
        "csharp",
        interface_content,
        parser_version,
        extractor_version,
    )
    .await;
    materialize_cached_path(
        &cfg,
        &relational,
        caller_path,
        "csharp",
        caller_content,
        parser_version,
        extractor_version,
    )
    .await;

    let db = Connection::open(&sqlite_path).expect("open sqlite db");
    let base_symbol_fqn = format!("{base_path}::BaseService");
    let interface_symbol_fqn = format!("{interface_path}::IRepository");
    let base_row: (String, String) = db
        .query_row(
            "SELECT symbol_id, artefact_id FROM artefacts_current WHERE repo_id = ?1 AND symbol_fqn = ?2",
            [cfg.repo.repo_id.as_str(), base_symbol_fqn.as_str()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("load base artefact row");
    let interface_row: (String, String) = db
        .query_row(
            "SELECT symbol_id, artefact_id FROM artefacts_current WHERE repo_id = ?1 AND symbol_fqn = ?2",
            [cfg.repo.repo_id.as_str(), interface_symbol_fqn.as_str()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("load interface artefact row");
    let extends_edge: (Option<String>, Option<String>, Option<String>) = db
        .query_row(
            "SELECT to_symbol_id, to_artefact_id, to_symbol_ref \
             FROM artefact_edges_current \
             WHERE repo_id = ?1 AND path = ?2 AND edge_kind = 'extends'",
            [cfg.repo.repo_id.as_str(), caller_path],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .expect("load extends edge");

    assert_eq!(extends_edge.0.as_deref(), Some(base_row.0.as_str()));
    assert_eq!(extends_edge.1.as_deref(), Some(base_row.1.as_str()));
    assert_eq!(extends_edge.2.as_deref(), Some(base_symbol_fqn.as_str()));

    let implements_edge: (Option<String>, Option<String>, Option<String>) = db
        .query_row(
            "SELECT to_symbol_id, to_artefact_id, to_symbol_ref \
             FROM artefact_edges_current \
             WHERE repo_id = ?1 AND path = ?2 AND edge_kind = 'implements'",
            [cfg.repo.repo_id.as_str(), caller_path],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .expect("load implements edge");

    assert_eq!(implements_edge.0.as_deref(), Some(interface_row.0.as_str()));
    assert_eq!(implements_edge.1.as_deref(), Some(interface_row.1.as_str()));
    assert_eq!(
        implements_edge.2.as_deref(),
        Some(interface_symbol_fqn.as_str())
    );

    let mut stmt = db
        .prepare(
            "SELECT af.symbol_fqn \
             FROM artefact_edges_current e \
             JOIN artefacts_current af \
               ON af.repo_id = e.repo_id AND af.artefact_id = e.from_artefact_id \
             JOIN artefacts_current at \
               ON at.repo_id = e.repo_id AND at.artefact_id = e.to_artefact_id \
             WHERE e.repo_id = ?1 AND e.edge_kind = 'extends' AND at.symbol_fqn = ?2 \
             ORDER BY af.symbol_fqn",
        )
        .expect("prepare inbound extends query");
    let inbound_extenders = stmt
        .query_map(
            [cfg.repo.repo_id.as_str(), base_symbol_fqn.as_str()],
            |row| row.get::<_, String>(0),
        )
        .expect("query inbound extends rows")
        .collect::<Result<Vec<_>, _>>()
        .expect("collect inbound extends rows");

    assert_eq!(
        inbound_extenders,
        vec![format!("{caller_path}::UserService")]
    );
}
