use rusqlite::Connection;
use tempfile::tempdir;

use super::fixtures::{
    desired_file_state, expected_symbol_id_by_fqn, seed_sync_repository_catalog_row,
    sqlite_relational_store_with_sync_schema, sync_test_cfg,
};

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
        path,
        "typescript",
        &content_id,
        "tree-sitter-ts@1",
        "ts-language-pack@1",
        content,
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
        path,
        "typescript",
        &content_id,
        "tree-sitter-ts@1",
        "ts-language-pack@1",
        content,
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
    let extraction = crate::host::devql::sync::extraction::extract_to_cache_format(
        &cfg,
        original_path,
        "typescript",
        &content_id,
        "tree-sitter-ts@1",
        "ts-language-pack@1",
        content,
    )
    .expect("extract original TypeScript content into cache format")
    .expect("original TypeScript cache extraction should be supported");
    let desired = desired_file_state(materialized_path, "typescript", &content_id);
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
        path,
        "typescript",
        &content_id,
        "tree-sitter-ts@1",
        "ts-language-pack@1",
        content,
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
