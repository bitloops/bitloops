use std::path::PathBuf;

use rusqlite::Connection;
use tempfile::tempdir;

use crate::daemon::{
    DevqlTaskKind, DevqlTaskProgress, DevqlTaskRecord, DevqlTaskSource, DevqlTaskSpec,
    DevqlTaskStatus, SyncTaskMode, SyncTaskSpec,
};
use crate::host::devql::SyncProgressUpdate;

use super::snapshot::snapshot_committed_current_rows_for_commit;
use super::stats::{PostCommitArtefactRefreshStats, QueuedSyncTaskMetadata};

fn sample_queued_result() -> crate::daemon::DevqlTaskEnqueueResult {
    crate::daemon::DevqlTaskEnqueueResult {
        task: DevqlTaskRecord {
            task_id: "sync-task-123".to_string(),
            repo_id: "repo-1".to_string(),
            repo_name: "demo".to_string(),
            repo_provider: "local".to_string(),
            repo_organisation: "local".to_string(),
            repo_identity: "local/demo".to_string(),
            daemon_config_root: PathBuf::from("/tmp/repo"),
            repo_root: PathBuf::from("/tmp/repo"),
            kind: DevqlTaskKind::Sync,
            source: DevqlTaskSource::PostCommit,
            spec: DevqlTaskSpec::Sync(SyncTaskSpec {
                mode: SyncTaskMode::Paths {
                    paths: vec!["src/lib.rs".to_string()],
                },
                post_commit_snapshot: None,
            }),
            init_session_id: None,
            status: DevqlTaskStatus::Queued,
            submitted_at_unix: 1,
            started_at_unix: None,
            updated_at_unix: 1,
            completed_at_unix: None,
            queue_position: Some(3),
            tasks_ahead: Some(2),
            progress: DevqlTaskProgress::Sync(SyncProgressUpdate::default()),
            error: None,
            result: None,
        },
        merged: true,
    }
}

#[test]
fn queued_refresh_stats_include_task_metadata() {
    let stats = PostCommitArtefactRefreshStats::queued(2, sample_queued_result());

    assert_eq!(stats.files_seen, 2);
    assert_eq!(stats.files_indexed, 0);
    assert_eq!(stats.files_deleted, 0);
    assert_eq!(stats.files_failed, 0);
    assert_eq!(
        stats.queued_task,
        Some(QueuedSyncTaskMetadata {
            task_id: "sync-task-123".to_string(),
            merged: true,
            queue_position: Some(3),
            tasks_ahead: Some(2),
        })
    );
    assert!(!stats.completed_with_failures());
}

#[test]
fn inline_refresh_stats_report_completed_failures() {
    let stats = PostCommitArtefactRefreshStats {
        files_seen: 2,
        files_indexed: 1,
        files_deleted: 0,
        files_failed: 1,
        queued_task: None,
    };

    assert!(stats.completed_with_failures());
}

#[tokio::test]
async fn snapshot_committed_current_rows_for_commit_promotes_only_head_rows() {
    let temp = tempdir().expect("temp dir");
    let sqlite_path = temp.path().join("relational.sqlite");
    crate::host::devql::init_sqlite_schema(&sqlite_path)
        .await
        .expect("initialise sqlite schema");
    let relational = crate::host::devql::RelationalStorage::local_only(sqlite_path.clone());
    let repo_id = "repo-refresh-test";

    relational
        .exec_batch_transactional(&[
            format!(
                "INSERT INTO repositories (repo_id, provider, organization, name, default_branch) \
                 VALUES ('{repo_id}', 'local', 'local', 'demo', 'main')"
            ),
            format!(
                "INSERT INTO current_file_state (
                    repo_id, path, analysis_mode, file_role, text_index_mode, language,
                    resolved_language, secondary_context_ids_json, frameworks_json,
                    classification_reason, extraction_fingerprint, head_content_id,
                    index_content_id, worktree_content_id, effective_content_id,
                    effective_source, parser_version, extractor_version, exists_in_head,
                    exists_in_index, exists_in_worktree, last_synced_at
                 ) VALUES
                    ('{repo_id}', 'src/head_a.ts', 'code', 'source_code', 'none', 'typescript',
                     'typescript', '[]', '[]', 'test', 'fp-a', 'blob-a', 'blob-a', 'blob-a',
                     'blob-a', 'head', 'parser', 'extractor', 1, 1, 1, '2026-04-15T10:00:00Z'),
                    ('{repo_id}', 'src/head_b.ts', 'code', 'source_code', 'none', 'typescript',
                     'typescript', '[]', '[]', 'test', 'fp-b', 'blob-b', 'blob-b', 'blob-b',
                     'blob-b', 'head', 'parser', 'extractor', 1, 1, 1, '2026-04-15T10:00:00Z'),
                    ('{repo_id}', 'src/draft.ts', 'code', 'source_code', 'none', 'typescript',
                     'typescript', '[]', '[]', 'test', 'fp-draft', 'blob-draft-head', 'blob-draft-index', 'blob-draft-worktree',
                     'blob-draft-worktree', 'worktree', 'parser', 'extractor', 1, 1, 1, '2026-04-15T10:00:00Z')"
            ),
            format!(
                "INSERT INTO artefacts_current (
                    repo_id, path, content_id, symbol_id, artefact_id, language,
                    extraction_fingerprint, canonical_kind, language_kind, symbol_fqn,
                    parent_symbol_id, parent_artefact_id, start_line, end_line, start_byte,
                    end_byte, signature, modifiers, docstring, updated_at
                 ) VALUES
                    ('{repo_id}', 'src/head_a.ts', 'blob-a', 'sym::a', 'artefact::a', 'typescript',
                     'fp-a', 'function', 'function_declaration', 'src/head_a.ts::renderA',
                     NULL, NULL, 1, 4, 0, 40, 'function renderA()', '[]', NULL, '2026-04-15T10:00:00Z'),
                    ('{repo_id}', 'src/head_b.ts', 'blob-b', 'sym::b', 'artefact::b', 'typescript',
                     'fp-b', 'function', 'function_declaration', 'src/head_b.ts::renderB',
                     NULL, NULL, 1, 4, 0, 40, 'function renderB()', '[]', NULL, '2026-04-15T10:00:00Z'),
                    ('{repo_id}', 'src/draft.ts', 'blob-draft-worktree', 'sym::draft', 'artefact::draft', 'typescript',
                     'fp-draft', 'function', 'function_declaration', 'src/draft.ts::renderDraft',
                     NULL, NULL, 1, 4, 0, 40, 'function renderDraft()', '[]', NULL, '2026-04-15T10:00:00Z')"
            ),
            format!(
                "INSERT INTO symbol_semantics_current (
                    artefact_id, repo_id, path, content_id, symbol_id, semantic_features_input_hash,
                    template_summary, summary, confidence
                 ) VALUES
                    ('artefact::a', '{repo_id}', 'src/head_a.ts', 'blob-a', 'sym::a', 'hash-a',
                     'Head A summary', 'Head A summary', 0.9),
                    ('artefact::b', '{repo_id}', 'src/head_b.ts', 'blob-b', 'sym::b', 'hash-b',
                     'Head B summary', 'Head B summary', 0.9),
                    ('artefact::draft', '{repo_id}', 'src/draft.ts', 'blob-draft-worktree', 'sym::draft', 'hash-draft',
                     'Draft summary', 'Draft summary', 0.9)"
            ),
            format!(
                "INSERT INTO symbol_features_current (
                    artefact_id, repo_id, path, content_id, symbol_id, semantic_features_input_hash,
                    normalized_name, normalized_signature, modifiers, identifier_tokens,
                    normalized_body_tokens, parent_kind, context_tokens
                 ) VALUES
                    ('artefact::a', '{repo_id}', 'src/head_a.ts', 'blob-a', 'sym::a', 'hash-a',
                     'render_a', 'function renderA()', '[]', '[\"render\",\"a\"]', '[\"render\",\"a\"]', 'module', '[\"head\",\"a\"]'),
                    ('artefact::b', '{repo_id}', 'src/head_b.ts', 'blob-b', 'sym::b', 'hash-b',
                     'render_b', 'function renderB()', '[]', '[\"render\",\"b\"]', '[\"render\",\"b\"]', 'module', '[\"head\",\"b\"]'),
                    ('artefact::draft', '{repo_id}', 'src/draft.ts', 'blob-draft-worktree', 'sym::draft', 'hash-draft',
                     'render_draft', 'function renderDraft()', '[]', '[\"render\",\"draft\"]', '[\"render\",\"draft\"]', 'module', '[\"draft\"]')"
            ),
            "INSERT INTO semantic_embedding_setups (setup_fingerprint, provider, model, dimension)
                 VALUES ('setup-code', 'local', 'test-model', 3)"
                .to_string(),
            format!(
                "INSERT INTO symbol_embeddings_current (
                    artefact_id, repo_id, path, content_id, symbol_id, representation_kind,
                    setup_fingerprint, provider, model, dimension, embedding_input_hash, embedding
                 ) VALUES
                    ('artefact::a', '{repo_id}', 'src/head_a.ts', 'blob-a', 'sym::a', 'code',
                     'setup-code', 'local', 'test-model', 3, 'embed-a', '[0.1,0.2,0.3]'),
                    ('artefact::b', '{repo_id}', 'src/head_b.ts', 'blob-b', 'sym::b', 'code',
                     'setup-code', 'local', 'test-model', 3, 'embed-b', '[0.2,0.1,0.3]'),
                    ('artefact::draft', '{repo_id}', 'src/draft.ts', 'blob-draft-worktree', 'sym::draft', 'code',
                     'setup-code', 'local', 'test-model', 3, 'embed-draft', '[0.3,0.2,0.1]')"
            ),
            format!(
                "INSERT INTO symbol_clone_edges_current (
                    repo_id, source_symbol_id, source_artefact_id, target_symbol_id, target_artefact_id,
                    relation_kind, score, semantic_score, lexical_score, structural_score,
                    clone_input_hash, explanation_json
                 ) VALUES
                    ('{repo_id}', 'sym::a', 'artefact::a', 'sym::b', 'artefact::b',
                     'similar_implementation', 0.91, 0.9, 0.8, 0.7, 'clone-head', '{{}}'),
                    ('{repo_id}', 'sym::a', 'artefact::a', 'sym::draft', 'artefact::draft',
                     'similar_implementation', 0.51, 0.5, 0.4, 0.3, 'clone-draft', '{{}}')"
            ),
        ])
        .await
        .expect("seed current projection rows");

    snapshot_committed_current_rows_for_commit(
        &relational,
        repo_id,
        "commit-head",
        &[
            "src/head_a.ts".to_string(),
            "src/head_b.ts".to_string(),
            "src/draft.ts".to_string(),
        ],
    )
    .await
    .expect("snapshot committed current rows");

    let db = Connection::open(&sqlite_path).expect("open sqlite db");
    let file_state_rows: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM file_state WHERE repo_id = ?1 AND commit_sha = ?2",
            rusqlite::params![repo_id, "commit-head"],
            |row| row.get(0),
        )
        .expect("count file_state rows");
    let artefact_rows: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM artefacts WHERE repo_id = ?1",
            rusqlite::params![repo_id],
            |row| row.get(0),
        )
        .expect("count historical artefacts");
    let snapshot_rows: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM artefact_snapshots WHERE repo_id = ?1",
            rusqlite::params![repo_id],
            |row| row.get(0),
        )
        .expect("count artefact snapshots");
    let semantic_rows: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM symbol_semantics WHERE repo_id = ?1",
            rusqlite::params![repo_id],
            |row| row.get(0),
        )
        .expect("count historical semantic rows");
    let feature_rows: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM symbol_features WHERE repo_id = ?1",
            rusqlite::params![repo_id],
            |row| row.get(0),
        )
        .expect("count historical feature rows");
    let embedding_rows: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM symbol_embeddings WHERE repo_id = ?1",
            rusqlite::params![repo_id],
            |row| row.get(0),
        )
        .expect("count historical embedding rows");
    let clone_rows: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM symbol_clone_edges WHERE repo_id = ?1",
            rusqlite::params![repo_id],
            |row| row.get(0),
        )
        .expect("count historical clone rows");
    let draft_file_rows: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM file_state WHERE repo_id = ?1 AND commit_sha = ?2 AND path = ?3",
            rusqlite::params![repo_id, "commit-head", "src/draft.ts"],
            |row| row.get(0),
        )
        .expect("count draft file_state rows");
    let draft_embedding_rows: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM symbol_embeddings WHERE repo_id = ?1 AND blob_sha = ?2",
            rusqlite::params![repo_id, "blob-draft-worktree"],
            |row| row.get(0),
        )
        .expect("count draft embedding rows");

    assert_eq!(file_state_rows, 2);
    assert_eq!(artefact_rows, 2);
    assert_eq!(snapshot_rows, 2);
    assert_eq!(semantic_rows, 2);
    assert_eq!(feature_rows, 2);
    assert_eq!(embedding_rows, 2);
    assert_eq!(clone_rows, 1);
    assert_eq!(draft_file_rows, 0);
    assert_eq!(draft_embedding_rows, 0);
}

#[tokio::test]
async fn snapshot_committed_current_rows_for_commit_limits_semantic_history_to_changed_paths() {
    let temp = tempdir().expect("temp dir");
    let sqlite_path = temp.path().join("relational.sqlite");
    crate::host::devql::init_sqlite_schema(&sqlite_path)
        .await
        .expect("initialise sqlite schema");
    let relational = crate::host::devql::RelationalStorage::local_only(sqlite_path.clone());
    let repo_id = "repo-refresh-scope-test";

    relational
        .exec_batch_transactional(&[
            format!(
                "INSERT INTO repositories (repo_id, provider, organization, name, default_branch) \
                 VALUES ('{repo_id}', 'local', 'local', 'demo', 'main')"
            ),
            format!(
                "INSERT INTO current_file_state (
                    repo_id, path, analysis_mode, file_role, text_index_mode, language,
                    resolved_language, secondary_context_ids_json, frameworks_json,
                    classification_reason, extraction_fingerprint, head_content_id,
                    index_content_id, worktree_content_id, effective_content_id,
                    effective_source, parser_version, extractor_version, exists_in_head,
                    exists_in_index, exists_in_worktree, last_synced_at
                 ) VALUES
                    ('{repo_id}', 'src/head_a.ts', 'code', 'source_code', 'none', 'typescript',
                     'typescript', '[]', '[]', 'test', 'fp-a', 'blob-a', 'blob-a', 'blob-a',
                     'blob-a', 'head', 'parser', 'extractor', 1, 1, 1, '2026-04-15T10:00:00Z'),
                    ('{repo_id}', 'src/head_b.ts', 'code', 'source_code', 'none', 'typescript',
                     'typescript', '[]', '[]', 'test', 'fp-b', 'blob-b', 'blob-b', 'blob-b',
                     'blob-b', 'head', 'parser', 'extractor', 1, 1, 1, '2026-04-15T10:00:00Z')"
            ),
            format!(
                "INSERT INTO artefacts_current (
                    repo_id, path, content_id, symbol_id, artefact_id, language,
                    extraction_fingerprint, canonical_kind, language_kind, symbol_fqn,
                    parent_symbol_id, parent_artefact_id, start_line, end_line, start_byte,
                    end_byte, signature, modifiers, docstring, updated_at
                 ) VALUES
                    ('{repo_id}', 'src/head_a.ts', 'blob-a', 'sym::a', 'artefact::a', 'typescript',
                     'fp-a', 'function', 'function_declaration', 'src/head_a.ts::renderA',
                     NULL, NULL, 1, 4, 0, 40, 'function renderA()', '[]', NULL, '2026-04-15T10:00:00Z'),
                    ('{repo_id}', 'src/head_b.ts', 'blob-b', 'sym::b', 'artefact::b', 'typescript',
                     'fp-b', 'function', 'function_declaration', 'src/head_b.ts::renderB',
                     NULL, NULL, 1, 4, 0, 40, 'function renderB()', '[]', NULL, '2026-04-15T10:00:00Z')"
            ),
            format!(
                "INSERT INTO symbol_semantics_current (
                    artefact_id, repo_id, path, content_id, symbol_id, semantic_features_input_hash,
                    template_summary, summary, confidence
                 ) VALUES
                    ('artefact::a', '{repo_id}', 'src/head_a.ts', 'blob-a', 'sym::a', 'hash-a',
                     'Head A summary', 'Head A summary', 0.9),
                    ('artefact::b', '{repo_id}', 'src/head_b.ts', 'blob-b', 'sym::b', 'hash-b',
                     'Head B summary', 'Head B summary', 0.9)"
            ),
            format!(
                "INSERT INTO symbol_features_current (
                    artefact_id, repo_id, path, content_id, symbol_id, semantic_features_input_hash,
                    normalized_name, normalized_signature, modifiers, identifier_tokens,
                    normalized_body_tokens, parent_kind, context_tokens
                 ) VALUES
                    ('artefact::a', '{repo_id}', 'src/head_a.ts', 'blob-a', 'sym::a', 'hash-a',
                     'render_a', 'function renderA()', '[]', '[\"render\",\"a\"]', '[\"render\",\"a\"]', 'module', '[\"head\",\"a\"]'),
                    ('artefact::b', '{repo_id}', 'src/head_b.ts', 'blob-b', 'sym::b', 'hash-b',
                     'render_b', 'function renderB()', '[]', '[\"render\",\"b\"]', '[\"render\",\"b\"]', 'module', '[\"head\",\"b\"]')"
            ),
            "INSERT INTO semantic_embedding_setups (setup_fingerprint, provider, model, dimension)
                 VALUES ('setup-code', 'local', 'test-model', 3)"
                .to_string(),
            format!(
                "INSERT INTO symbol_embeddings_current (
                    artefact_id, repo_id, path, content_id, symbol_id, representation_kind,
                    setup_fingerprint, provider, model, dimension, embedding_input_hash, embedding
                 ) VALUES
                    ('artefact::a', '{repo_id}', 'src/head_a.ts', 'blob-a', 'sym::a', 'code',
                     'setup-code', 'local', 'test-model', 3, 'embed-a', '[0.1,0.2,0.3]'),
                    ('artefact::b', '{repo_id}', 'src/head_b.ts', 'blob-b', 'sym::b', 'code',
                     'setup-code', 'local', 'test-model', 3, 'embed-b', '[0.2,0.1,0.3]')"
            ),
            format!(
                "INSERT INTO symbol_clone_edges_current (
                    repo_id, source_symbol_id, source_artefact_id, target_symbol_id, target_artefact_id,
                    relation_kind, score, semantic_score, lexical_score, structural_score,
                    clone_input_hash, explanation_json
                 ) VALUES
                    ('{repo_id}', 'sym::a', 'artefact::a', 'sym::b', 'artefact::b',
                     'similar_implementation', 0.91, 0.9, 0.8, 0.7, 'clone-head', '{{}}')"
            ),
        ])
        .await
        .expect("seed current projection rows");

    snapshot_committed_current_rows_for_commit(
        &relational,
        repo_id,
        "commit-head",
        &["src/head_a.ts".to_string()],
    )
    .await
    .expect("snapshot scoped current rows");

    let db = Connection::open(&sqlite_path).expect("open sqlite db");
    let file_state_rows: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM file_state WHERE repo_id = ?1 AND commit_sha = ?2",
            rusqlite::params![repo_id, "commit-head"],
            |row| row.get(0),
        )
        .expect("count file_state rows");
    let artefact_rows: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM artefacts WHERE repo_id = ?1",
            rusqlite::params![repo_id],
            |row| row.get(0),
        )
        .expect("count historical artefacts");
    let semantic_rows: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM symbol_semantics WHERE repo_id = ?1",
            rusqlite::params![repo_id],
            |row| row.get(0),
        )
        .expect("count historical semantic rows");
    let embedding_rows: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM symbol_embeddings WHERE repo_id = ?1",
            rusqlite::params![repo_id],
            |row| row.get(0),
        )
        .expect("count historical embedding rows");
    let clone_rows: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM symbol_clone_edges WHERE repo_id = ?1",
            rusqlite::params![repo_id],
            |row| row.get(0),
        )
        .expect("count historical clone rows");

    assert_eq!(file_state_rows, 2);
    assert_eq!(artefact_rows, 1);
    assert_eq!(semantic_rows, 1);
    assert_eq!(embedding_rows, 1);
    assert_eq!(clone_rows, 1);
}
