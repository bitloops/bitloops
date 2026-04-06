use super::*;

#[tokio::test]
async fn commit_revision_replaces_temporary_current_metadata_for_unchanged_content() {
    let cfg = test_cfg();
    let temp = tempdir().expect("temp dir");
    let sqlite_path = temp.path().join("relational.db");
    let relational = sqlite_relational_store_with_schema(&sqlite_path).await;
    let path = "src/train.txt";
    let blob_sha = "blob-stable";

    upsert_current_state_for_content(
        &cfg,
        &relational,
        &FileRevision {
            commit_sha: "commit-old",
            revision: TemporalRevisionRef {
                kind: TemporalRevisionKind::Temporary,
                id: "temp:7",
                temp_checkpoint_id: Some(7),
            },
            commit_unix: 100,
            path,
            blob_sha,
        },
        "hello world\n",
    )
    .await
    .expect("write temporary current state");

    upsert_current_state_for_content(
        &cfg,
        &relational,
        &FileRevision {
            commit_sha: "commit-new",
            revision: TemporalRevisionRef {
                kind: TemporalRevisionKind::Commit,
                id: "commit-new",
                temp_checkpoint_id: None,
            },
            commit_unix: 100,
            path,
            blob_sha,
        },
        "hello world\n",
    )
    .await
    .expect("write committed current state");

    let conn = rusqlite::Connection::open(sqlite_path).expect("open sqlite");
    let artefact_row: (String,) = conn
        .query_row(
            "SELECT content_id \
             FROM artefacts_current WHERE repo_id = ?1 AND symbol_id = ?2",
            rusqlite::params![cfg.repo.repo_id, file_symbol_id(path)],
            |row| Ok((row.get(0)?,)),
        )
        .expect("fetch current file artefact row");
    assert_eq!(artefact_row.0, blob_sha);
}

#[tokio::test]
async fn upsert_current_state_only_revises_changed_symbol_when_siblings_are_unchanged() {
    let cfg = test_cfg();
    let temp = tempdir().expect("temp dir");
    let sqlite_path = temp.path().join("relational.db");
    let relational = sqlite_relational_store_with_schema(&sqlite_path).await;
    let path = "src/sample.ts";
    let initial =
        "export function alpha() {\n  return 1;\n}\n\nexport function beta() {\n  return 2;\n}\n";
    let updated =
        "export function alpha() {\n  return 9;\n}\n\nexport function beta() {\n  return 2;\n}\n";

    upsert_current_state_for_content(
        &cfg,
        &relational,
        &FileRevision {
            commit_sha: "commit-base",
            revision: TemporalRevisionRef {
                kind: TemporalRevisionKind::Temporary,
                id: "temp:1",
                temp_checkpoint_id: Some(1),
            },
            commit_unix: 100,
            path,
            blob_sha: "blob-1",
        },
        initial,
    )
    .await
    .expect("write initial current state");

    upsert_current_state_for_content(
        &cfg,
        &relational,
        &FileRevision {
            commit_sha: "commit-base",
            revision: TemporalRevisionRef {
                kind: TemporalRevisionKind::Temporary,
                id: "temp:2",
                temp_checkpoint_id: Some(2),
            },
            commit_unix: 101,
            path,
            blob_sha: "blob-2",
        },
        updated,
    )
    .await
    .expect("write updated current state");

    let conn = rusqlite::Connection::open(sqlite_path).expect("open sqlite");
    let file_content_id: String = conn
        .query_row(
            "SELECT content_id FROM artefacts_current WHERE repo_id = ?1 AND symbol_id = ?2",
            rusqlite::params![cfg.repo.repo_id, file_symbol_id(path)],
            |row| row.get(0),
        )
        .expect("read file content_id");
    let alpha_content_id: String = conn
        .query_row(
            "SELECT content_id FROM artefacts_current WHERE repo_id = ?1 AND symbol_fqn = ?2",
            rusqlite::params![cfg.repo.repo_id, format!("{path}::alpha")],
            |row| row.get(0),
        )
        .expect("read alpha content_id");
    let beta_content_id: String = conn
        .query_row(
            "SELECT content_id FROM artefacts_current WHERE repo_id = ?1 AND symbol_fqn = ?2",
            rusqlite::params![cfg.repo.repo_id, format!("{path}::beta")],
            |row| row.get(0),
        )
        .expect("read beta content_id");

    assert_eq!(file_content_id, "blob-2");
    assert_eq!(alpha_content_id, "blob-2");
    assert_eq!(beta_content_id, "blob-2");
}

#[tokio::test]
async fn upsert_current_state_revises_shifted_sibling_when_lines_move() {
    let cfg = test_cfg();
    let temp = tempdir().expect("temp dir");
    let sqlite_path = temp.path().join("relational.db");
    let relational = sqlite_relational_store_with_schema(&sqlite_path).await;
    let path = "src/sample.ts";
    let initial =
        "export function alpha() {\n  return 1;\n}\n\nexport function beta() {\n  return 2;\n}\n";
    let updated = "export function alpha() {\n  const next = 1;\n  return next;\n}\n\nexport function beta() {\n  return 2;\n}\n";

    upsert_current_state_for_content(
        &cfg,
        &relational,
        &FileRevision {
            commit_sha: "commit-base",
            revision: TemporalRevisionRef {
                kind: TemporalRevisionKind::Temporary,
                id: "temp:1",
                temp_checkpoint_id: Some(1),
            },
            commit_unix: 100,
            path,
            blob_sha: "blob-1",
        },
        initial,
    )
    .await
    .expect("write initial current state");

    let conn = rusqlite::Connection::open(&sqlite_path).expect("open sqlite");
    let old_beta_start_line: i32 = conn
        .query_row(
            "SELECT start_line FROM artefacts_current WHERE repo_id = ?1 AND symbol_fqn = ?2",
            rusqlite::params![cfg.repo.repo_id, format!("{path}::beta")],
            |row| row.get(0),
        )
        .expect("read original beta start line");

    upsert_current_state_for_content(
        &cfg,
        &relational,
        &FileRevision {
            commit_sha: "commit-base",
            revision: TemporalRevisionRef {
                kind: TemporalRevisionKind::Temporary,
                id: "temp:2",
                temp_checkpoint_id: Some(2),
            },
            commit_unix: 101,
            path,
            blob_sha: "blob-2",
        },
        updated,
    )
    .await
    .expect("write updated current state");

    let (beta_content_id, beta_start_line): (String, i32) = conn
        .query_row(
            "SELECT content_id, start_line FROM artefacts_current WHERE repo_id = ?1 AND symbol_fqn = ?2",
            rusqlite::params![cfg.repo.repo_id, format!("{path}::beta")],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("read updated beta state");

    assert_eq!(beta_content_id, "blob-2");
    assert!(beta_start_line > old_beta_start_line);
}

#[tokio::test]
async fn unchanged_edge_keeps_previous_revision_when_other_symbol_changes() {
    let cfg = test_cfg();
    let temp = tempdir().expect("temp dir");
    let sqlite_path = temp.path().join("relational.db");
    let relational = sqlite_relational_store_with_schema(&sqlite_path).await;
    let path = "src/sample.ts";
    let initial = "export function helper() {\n  return 1;\n}\n\nexport function alpha() {\n  return helper();\n}\n\nexport function beta() {\n  return 2;\n}\n";
    let updated = "export function helper() {\n  return 1;\n}\n\nexport function alpha() {\n  return helper();\n}\n\nexport function beta() {\n  return 3;\n}\n";

    upsert_current_state_for_content(
        &cfg,
        &relational,
        &FileRevision {
            commit_sha: "commit-base",
            revision: TemporalRevisionRef {
                kind: TemporalRevisionKind::Temporary,
                id: "temp:1",
                temp_checkpoint_id: Some(1),
            },
            commit_unix: 100,
            path,
            blob_sha: "blob-1",
        },
        initial,
    )
    .await
    .expect("write initial current state");

    upsert_current_state_for_content(
        &cfg,
        &relational,
        &FileRevision {
            commit_sha: "commit-base",
            revision: TemporalRevisionRef {
                kind: TemporalRevisionKind::Temporary,
                id: "temp:2",
                temp_checkpoint_id: Some(2),
            },
            commit_unix: 101,
            path,
            blob_sha: "blob-2",
        },
        updated,
    )
    .await
    .expect("write updated current state");

    let conn = rusqlite::Connection::open(sqlite_path).expect("open sqlite");
    let alpha_symbol_id: String = conn
        .query_row(
            "SELECT symbol_id FROM artefacts_current WHERE repo_id = ?1 AND symbol_fqn = ?2",
            rusqlite::params![cfg.repo.repo_id, format!("{path}::alpha")],
            |row| row.get(0),
        )
        .expect("read alpha symbol id");
    let edge_content_id: String = conn
        .query_row(
            "SELECT content_id FROM artefact_edges_current WHERE repo_id = ?1 AND path = ?2 AND from_symbol_id = ?3 AND edge_kind = 'calls'",
            rusqlite::params![cfg.repo.repo_id, path, alpha_symbol_id],
            |row| row.get(0),
        )
        .expect("read alpha edge content_id");

    assert_eq!(edge_content_id, "blob-2");
}

#[tokio::test]
async fn refresh_current_state_deletes_stale_edge_ids_before_upserting_new_natural_edge() {
    let cfg = test_cfg();
    let temp = tempdir().expect("temp dir");
    let sqlite_path = temp.path().join("relational.db");
    let relational = sqlite_relational_store_with_schema(&sqlite_path).await;

    let path = "src/main.ts";
    let blob_sha = "blob-123";
    let file_artefact = test_file_row(&cfg, path, blob_sha, 20, 200);
    let source = test_symbol_record(&cfg, path, blob_sha, "source-symbol", "source", 1, 5);
    let edge = test_unresolved_call_edge(&source.symbol_fqn, "pkg::remote", 4);
    let edge_metadata = edge.metadata.to_value().to_string();

    let conn = rusqlite::Connection::open(&sqlite_path).expect("open sqlite");
    conn.execute(
        "INSERT INTO artefact_edges_current (
            repo_id, edge_id, path, content_id, from_symbol_id, from_artefact_id,
            to_symbol_id, to_artefact_id, to_symbol_ref, edge_kind, language,
            start_line, end_line, metadata, updated_at
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, NULL, NULL, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
        rusqlite::params![
            cfg.repo.repo_id,
            "legacy-edge-id",
            path,
            "blob-old",
            source.symbol_id,
            source.artefact_id,
            "pkg::remote",
            "calls",
            "typescript",
            4_i64,
            4_i64,
            edge_metadata,
            "2026-03-26T09:00:00Z",
        ],
    )
    .expect("insert stale edge row with legacy edge_id");

    refresh_current_state_for_path(
        &cfg,
        &relational,
        &FileRevision {
            commit_sha: "commit-new",
            revision: TemporalRevisionRef {
                kind: TemporalRevisionKind::Commit,
                id: "commit-new",
                temp_checkpoint_id: None,
            },
            commit_unix: 200,
            path,
            blob_sha,
        },
        &file_artefact,
        None,
        &[source],
        vec![edge],
    )
    .await
    .expect("refresh current state should replace stale edge ids");

    let edge_rows: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM artefact_edges_current WHERE repo_id = ?1 AND path = ?2",
            rusqlite::params![cfg.repo.repo_id, path],
            |row| row.get(0),
        )
        .expect("count current edges");
    assert_eq!(edge_rows, 1);
    let has_legacy: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM artefact_edges_current WHERE repo_id = ?1 AND edge_id = 'legacy-edge-id'",
            rusqlite::params![cfg.repo.repo_id],
            |row| row.get(0),
        )
        .expect("count legacy edge ids");
    assert_eq!(has_legacy, 0);
}
