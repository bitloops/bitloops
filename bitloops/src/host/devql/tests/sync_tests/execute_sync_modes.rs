use rusqlite::Connection;
use std::fs;
use std::path::Path;
use tempfile::tempdir;

use super::fixtures::{
    seed_full_sync_repo, seed_supported_and_unsupported_repo,
    sqlite_relational_store_with_sync_schema, sync_test_cfg_for_repo,
};

#[cfg(unix)]
fn fake_runtime_command_and_args(repo_root: &Path) -> (String, Vec<String>) {
    use std::os::unix::fs::PermissionsExt;

    let script_path = repo_root.join(".bitloops/test-bin/fake-sync-embeddings-runtime.sh");
    if let Some(parent) = script_path.parent() {
        fs::create_dir_all(parent).expect("create fake runtime dir");
    }
    let script = r#"#!/bin/sh
printf '{"event":"ready","protocol":1,"capabilities":["embed","shutdown"]}\n'
while IFS= read -r line; do
  req_id=$(printf '%s\n' "$line" | sed -n 's/.*"id":"\([^"]*\)".*/\1/p')
  case "$line" in
    *'"cmd":"embed"'*)
      printf '{"id":"%s","ok":true,"vectors":[[0.1,0.2,0.3]],"model":"sync-test-model"}\n' "$req_id"
      ;;
    *'"cmd":"shutdown"'*)
      printf '{"id":"%s","ok":true,"model":"sync-test-model"}\n' "$req_id"
      exit 0
      ;;
    *)
      printf '{"id":"%s","ok":false,"error":{"message":"unexpected request"}}\n' "$req_id"
      ;;
  esac
done
"#;
    fs::write(&script_path, script).expect("write fake runtime script");
    let mut permissions = fs::metadata(&script_path)
        .expect("stat fake runtime script")
        .permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&script_path, permissions).expect("chmod fake runtime script");
    ("sh".to_string(), vec![script_path.display().to_string()])
}

#[cfg(windows)]
fn fake_runtime_command_and_args(repo_root: &Path) -> (String, Vec<String>) {
    let script_path = repo_root.join(".bitloops/test-bin/fake-sync-embeddings-runtime.ps1");
    if let Some(parent) = script_path.parent() {
        fs::create_dir_all(parent).expect("create fake runtime dir");
    }
    let script = r#"
$ready = @{
  event = "ready"
  protocol = 1
  capabilities = @("embed", "shutdown")
}
$ready | ConvertTo-Json -Compress
$stdin = [Console]::In
while (($line = $stdin.ReadLine()) -ne $null) {
  if ([string]::IsNullOrWhiteSpace($line)) { continue }
  $request = $line | ConvertFrom-Json
  switch ($request.cmd) {
    "embed" {
      $response = @{
        id = $request.id
        ok = $true
        vectors = @(@(0.1, 0.2, 0.3))
        model = "sync-test-model"
      }
    }
    "shutdown" {
      $response = @{
        id = $request.id
        ok = $true
        model = "sync-test-model"
      }
      $response | ConvertTo-Json -Compress
      break
    }
    default {
      $response = @{
        id = $request.id
        ok = $false
        error = @{
          message = "unexpected request"
        }
      }
    }
  }
  $response | ConvertTo-Json -Compress
}
"#;
    fs::write(&script_path, script).expect("write fake runtime script");
    (
        "powershell".to_string(),
        vec![
            "-NoProfile".to_string(),
            "-ExecutionPolicy".to_string(),
            "Bypass".to_string(),
            "-File".to_string(),
            script_path.display().to_string(),
        ],
    )
}

fn write_sync_semantic_clone_config(repo_root: &Path) {
    let (command, args) = fake_runtime_command_and_args(repo_root);
    let runtime_args = args
        .iter()
        .map(|arg| format!("{arg:?}"))
        .collect::<Vec<_>>()
        .join(", ");
    let config_path = repo_root.join(crate::config::BITLOOPS_CONFIG_RELATIVE_PATH);
    if let Some(parent) = config_path.parent() {
        fs::create_dir_all(parent).expect("create daemon config dir");
    }
    fs::write(
        config_path,
        format!(
            r#"[semantic_clones]
summary_mode = "off"
embedding_mode = "deterministic"

[semantic_clones.inference]
code_embeddings = "alpha"
summary_embeddings = "alpha"

[inference.runtimes.bitloops_embeddings]
command = {command:?}
args = [{runtime_args}]
startup_timeout_secs = 5
request_timeout_secs = 5

[inference.profiles.alpha]
task = "embeddings"
driver = "bitloops_embeddings_ipc"
runtime = "bitloops_embeddings"
model = "sync-test-model"
"#
        ),
    )
    .expect("write sync semantic clone config");
}

#[tokio::test]
async fn unborn_head_syncs_from_index_and_worktree() {
    let repo = tempdir().expect("temp dir");
    crate::test_support::git_fixtures::init_test_repo(
        repo.path(),
        "main",
        "Bitloops Test",
        "bitloops-test@example.com",
    );
    fs::create_dir_all(repo.path().join("src")).expect("create src dir");
    fs::write(
        repo.path().join("src/lib.rs"),
        "pub fn draft() -> bool {\n    true\n}\n",
    )
    .expect("write supported source file");
    crate::test_support::git_fixtures::git_ok(repo.path(), &["add", "src/lib.rs"]);
    let staged_blob = crate::host::checkpoints::strategy::manual_commit::run_git(
        repo.path(),
        &["rev-parse", ":src/lib.rs"],
    )
    .expect("resolve staged blob");

    let cfg = sync_test_cfg_for_repo(repo.path());
    let sqlite_path = repo.path().join("devql.sqlite");
    let relational = sqlite_relational_store_with_sync_schema(&sqlite_path).await;

    let result = crate::host::devql::execute_sync(
        &cfg,
        &relational,
        crate::host::devql::sync::types::SyncMode::Full,
    )
    .await
    .expect("execute full sync for unborn HEAD repo");

    let db = Connection::open(&sqlite_path).expect("open sqlite db");
    let current_paths = {
        let mut stmt = db
            .prepare(
                "SELECT path \
                 FROM current_file_state \
                 WHERE repo_id = ?1 \
                 ORDER BY path",
            )
            .expect("prepare current_file_state path query");
        stmt.query_map([cfg.repo.repo_id.as_str()], |row| row.get::<_, String>(0))
            .expect("query current_file_state paths")
            .collect::<Result<Vec<_>, _>>()
            .expect("collect current_file_state paths")
    };
    let current_state: (String, Option<String>, Option<String>, Option<String>, String) = db
        .query_row(
            "SELECT effective_content_id, index_content_id, worktree_content_id, head_content_id, effective_source \
             FROM current_file_state \
             WHERE repo_id = ?1 AND path = ?2",
            [cfg.repo.repo_id.as_str(), "src/lib.rs"],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            },
        )
        .expect("read current_file_state for unborn HEAD path");

    assert!(result.success, "unborn-head full sync should succeed");
    assert!(result.paths_added >= 1);
    assert_eq!(result.paths_removed, 0);
    assert_eq!(result.paths_changed, 0);
    assert_eq!(current_paths, vec!["src/lib.rs".to_string()]);
    assert_eq!(current_state.0, staged_blob);
    assert_eq!(current_state.1.as_deref(), Some(staged_blob.as_str()));
    assert_eq!(current_state.2.as_deref(), Some(staged_blob.as_str()));
    assert_eq!(current_state.3, None);
    assert_eq!(current_state.4, "index");
}

#[tokio::test]
async fn unsupported_file_ignored_supported_file_added() {
    let repo = seed_supported_and_unsupported_repo();
    let cfg = sync_test_cfg_for_repo(repo.path());
    let sqlite_path = repo.path().join("devql.sqlite");
    let relational = sqlite_relational_store_with_sync_schema(&sqlite_path).await;

    let result = crate::host::devql::execute_sync(
        &cfg,
        &relational,
        crate::host::devql::sync::types::SyncMode::Full,
    )
    .await
    .expect("execute full sync");

    let db = Connection::open(&sqlite_path).expect("open sqlite db");
    let current_paths = {
        let mut stmt = db
            .prepare(
                "SELECT path \
                 FROM current_file_state \
                 WHERE repo_id = ?1 \
                 ORDER BY path",
            )
            .expect("prepare current_file_state path query");
        stmt.query_map([cfg.repo.repo_id.as_str()], |row| row.get::<_, String>(0))
            .expect("query current_file_state paths")
            .collect::<Result<Vec<_>, _>>()
            .expect("collect current_file_state paths")
    };
    let unsupported_rows: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM current_file_state WHERE repo_id = ?1 AND path = ?2",
            [cfg.repo.repo_id.as_str(), "docs/notes.foo"],
            |row| row.get(0),
        )
        .expect("count unsupported current_file_state rows");

    assert!(
        result.success,
        "sync should succeed with ignored unsupported files"
    );
    assert_eq!(result.paths_added, 1);
    assert_eq!(result.paths_changed, 0);
    assert_eq!(result.paths_removed, 0);
    assert_eq!(result.paths_unchanged, 0);
    assert_eq!(current_paths, vec!["src/lib.rs".to_string()]);
    assert_eq!(unsupported_rows, 0);
}

#[tokio::test]
async fn path_scoped_sync_only_updates_specified_paths() {
    let repo = seed_full_sync_repo();
    let cfg = sync_test_cfg_for_repo(repo.path());
    let sqlite_path = repo.path().join("devql.sqlite");
    let relational = sqlite_relational_store_with_sync_schema(&sqlite_path).await;
    let scoped_path = "src/lib.rs";
    let unscoped_path = "web/app.ts";
    let scoped_content =
        "pub fn greet(name: &str) -> String {\n    format!(\"scoped {name}\")\n}\n";
    let unscoped_content = "import { helper } from \"./util\";\n\nexport function run(): number {\n  return helper() + 1;\n}\n";

    crate::host::devql::execute_sync(
        &cfg,
        &relational,
        crate::host::devql::sync::types::SyncMode::Full,
    )
    .await
    .expect("execute baseline full sync");

    let db = Connection::open(&sqlite_path).expect("open sqlite db");
    let load_artefacts = |db: &Connection, path: &str| {
        let mut stmt = db
            .prepare(
                "SELECT content_id, symbol_fqn, symbol_id, artefact_id \
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
                row.get::<_, String>(3)?,
            ))
        })
        .expect("query artefacts_current rows")
        .collect::<Result<Vec<_>, _>>()
        .expect("collect artefacts_current rows")
    };
    let load_current_state = |db: &Connection, path: &str| {
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

    let baseline_scoped_artefacts = load_artefacts(&db, scoped_path);
    let baseline_unscoped_artefacts = load_artefacts(&db, unscoped_path);
    let baseline_scoped_state = load_current_state(&db, scoped_path);
    let baseline_unscoped_state = load_current_state(&db, unscoped_path);

    fs::write(repo.path().join(scoped_path), scoped_content).expect("edit scoped file");
    fs::write(repo.path().join(unscoped_path), unscoped_content).expect("edit unscoped file");

    let result = crate::host::devql::execute_sync(
        &cfg,
        &relational,
        crate::host::devql::sync::types::SyncMode::Paths(vec![scoped_path.to_string()]),
    )
    .await
    .expect("execute path-scoped sync");

    let scoped_blob =
        crate::host::devql::sync::content_identity::compute_blob_oid(scoped_content.as_bytes());
    let scoped_state = load_current_state(&db, scoped_path);
    let unscoped_state = load_current_state(&db, unscoped_path);
    let scoped_artefacts = load_artefacts(&db, scoped_path);
    let unscoped_artefacts = load_artefacts(&db, unscoped_path);

    assert_eq!(result.paths_changed, 1);
    assert_eq!(result.paths_added, 0);
    assert_eq!(result.paths_removed, 0);
    assert_eq!(result.paths_unchanged, 0);
    assert_eq!(result.cache_hits, 0);
    assert_eq!(result.cache_misses, 1);
    assert_eq!(scoped_state.4, scoped_blob);
    assert_eq!(scoped_state.5, "worktree");
    assert_eq!(unscoped_state, baseline_unscoped_state);
    assert_eq!(unscoped_artefacts, baseline_unscoped_artefacts);
    assert_eq!(baseline_scoped_artefacts.len(), scoped_artefacts.len());
    assert_ne!(scoped_artefacts, baseline_scoped_artefacts);
    assert_eq!(
        unscoped_state.4, baseline_unscoped_state.4,
        "unscoped path should keep the previously materialized content id"
    );
    assert_eq!(
        baseline_scoped_state.4,
        crate::host::devql::sync::content_identity::compute_blob_oid(
            "pub fn greet(name: &str) -> String {\n    format!(\"hi {name}\")\n}\n".as_bytes()
        ),
        "baseline scoped state should still reflect the original materialization"
    );
    assert!(
        scoped_artefacts.iter().all(|row| row.0 == scoped_blob),
        "scoped artefacts should reflect the edited content"
    );
}

#[tokio::test]
async fn repair_mode_reprocesses_all_paths_using_cache_when_available() {
    let repo = seed_full_sync_repo();
    let cfg = sync_test_cfg_for_repo(repo.path());
    let sqlite_path = repo.path().join("devql.sqlite");
    let relational = sqlite_relational_store_with_sync_schema(&sqlite_path).await;
    let path = "src/lib.rs";

    crate::host::devql::execute_sync(
        &cfg,
        &relational,
        crate::host::devql::sync::types::SyncMode::Full,
    )
    .await
    .expect("execute baseline full sync");

    let db = Connection::open(&sqlite_path).expect("open sqlite db");
    let baseline_state: (String, String, String) = db
        .query_row(
            "SELECT effective_content_id, effective_source, parser_version \
             FROM current_file_state \
             WHERE repo_id = ?1 AND path = ?2",
            [cfg.repo.repo_id.as_str(), path],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .expect("read baseline current_file_state row");
    let baseline_versions: (String, String) = db
        .query_row(
            "SELECT parser_version, extractor_version \
             FROM repo_sync_state \
             WHERE repo_id = ?1",
            [cfg.repo.repo_id.as_str()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("read baseline sync versions");
    let expected_count: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM current_file_state WHERE repo_id = ?1",
            [cfg.repo.repo_id.as_str()],
            |row| row.get(0),
        )
        .expect("count baseline supported paths");
    let baseline_retention_class: String = db
        .query_row(
            "SELECT retention_class \
             FROM content_cache \
             WHERE content_id = ?1 AND language = ?2 AND parser_version = ?3 AND extractor_version = ?4",
            [
                baseline_state.0.as_str(),
                "rust",
                baseline_versions.0.as_str(),
                baseline_versions.1.as_str(),
            ],
            |row| row.get(0),
        )
        .expect("read baseline retention class");

    let result = crate::host::devql::execute_sync(
        &cfg,
        &relational,
        crate::host::devql::sync::types::SyncMode::Repair,
    )
    .await
    .expect("execute repair sync");

    let repaired_state: (String, String, String) = db
        .query_row(
            "SELECT effective_content_id, effective_source, parser_version \
             FROM current_file_state \
             WHERE repo_id = ?1 AND path = ?2",
            [cfg.repo.repo_id.as_str(), path],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .expect("read repaired current_file_state row");
    let retention_class: String = db
        .query_row(
            "SELECT retention_class \
             FROM content_cache \
             WHERE content_id = ?1 AND language = ?2 AND parser_version = ?3 AND extractor_version = ?4",
            [
                repaired_state.0.as_str(),
                "rust",
                result.parser_version.as_str(),
                result.extractor_version.as_str(),
            ],
            |row| row.get(0),
        )
        .expect("read repaired retention class");

    assert_eq!(result.paths_changed as i64, expected_count);
    assert_eq!(result.paths_added, 0);
    assert_eq!(result.paths_removed, 0);
    assert_eq!(result.paths_unchanged, 0);
    assert_eq!(result.cache_hits as i64, expected_count);
    assert_eq!(result.cache_misses, 0);
    assert_eq!(repaired_state, baseline_state);
    assert_eq!(retention_class, baseline_retention_class);
}

#[tokio::test]
async fn execute_sync_with_stats_reports_batched_sqlite_writes() {
    let repo = seed_full_sync_repo();
    let cfg = sync_test_cfg_for_repo(repo.path());
    let sqlite_path = repo.path().join("devql.sqlite");
    let relational = sqlite_relational_store_with_sync_schema(&sqlite_path).await;

    let (summary, stats) = crate::host::devql::execute_sync_with_stats(
        &cfg,
        &relational,
        crate::host::devql::sync::types::SyncMode::Full,
    )
    .await
    .expect("execute full sync with stats");

    assert!(summary.paths_added > 0);
    assert_eq!(stats.prepare_worker_count, summary.paths_added.min(8));
    assert!(stats.sqlite_commits > 0);
    assert!(
        stats.sqlite_commits < summary.paths_added.saturating_mul(2),
        "batched writer should use fewer commits than per-file cache+materialise writes"
    );
    assert!(
        !stats.workspace_inspection.is_zero(),
        "workspace inspection timing should be recorded"
    );
    assert!(
        !stats.desired_manifest_build.is_zero(),
        "manifest timing should be recorded"
    );
    assert!(
        stats.sqlite_rows_written > 0,
        "writer stats should record SQLite row mutations"
    );
}

#[tokio::test]
async fn sync_removes_deleted_file() {
    let repo = seed_full_sync_repo();
    let cfg = sync_test_cfg_for_repo(repo.path());
    let sqlite_path = repo.path().join("devql.sqlite");
    let relational = sqlite_relational_store_with_sync_schema(&sqlite_path).await;

    crate::host::devql::execute_sync(
        &cfg,
        &relational,
        crate::host::devql::sync::types::SyncMode::Full,
    )
    .await
    .expect("execute initial full sync");

    fs::remove_file(repo.path().join("web/app.ts")).expect("delete tracked source file");

    let result = crate::host::devql::execute_sync(
        &cfg,
        &relational,
        crate::host::devql::sync::types::SyncMode::Full,
    )
    .await
    .expect("execute sync after delete");

    let db = Connection::open(&sqlite_path).expect("open sqlite db");
    let artefact_count: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM artefacts_current WHERE repo_id = ?1 AND path = ?2",
            [cfg.repo.repo_id.as_str(), "web/app.ts"],
            |row| row.get(0),
        )
        .expect("count artefacts for deleted path");
    let current_state_count: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM current_file_state WHERE repo_id = ?1 AND path = ?2",
            [cfg.repo.repo_id.as_str(), "web/app.ts"],
            |row| row.get(0),
        )
        .expect("count current_file_state for deleted path");
    let edge_count: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM artefact_edges_current WHERE repo_id = ?1 AND path = ?2",
            [cfg.repo.repo_id.as_str(), "web/app.ts"],
            |row| row.get(0),
        )
        .expect("count artefact_edges_current for deleted path");

    assert_eq!(result.paths_removed, 1);
    assert_eq!(result.paths_added, 0);
    assert_eq!(result.paths_changed, 0);
    assert_eq!(result.paths_unchanged, 3);
    assert_eq!(artefact_count, 0);
    assert_eq!(edge_count, 0);
    assert_eq!(current_state_count, 0);
}

#[tokio::test]
async fn sync_populates_current_semantic_and_embedding_tables() {
    let repo = seed_full_sync_repo();
    write_sync_semantic_clone_config(repo.path());
    let cfg = sync_test_cfg_for_repo(repo.path());
    let sqlite_path = repo.path().join("devql.sqlite");
    let relational = sqlite_relational_store_with_sync_schema(&sqlite_path).await;

    let result = crate::host::devql::execute_sync(
        &cfg,
        &relational,
        crate::host::devql::sync::types::SyncMode::Full,
    )
    .await
    .expect("execute sync with current semantic clone projection");

    let db = Connection::open(&sqlite_path).expect("open sqlite db");
    let semantic_rows: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM symbol_semantics_current WHERE repo_id = ?1",
            [cfg.repo.repo_id.as_str()],
            |row| row.get(0),
        )
        .expect("count symbol_semantics_current rows");
    let feature_rows: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM symbol_features_current WHERE repo_id = ?1",
            [cfg.repo.repo_id.as_str()],
            |row| row.get(0),
        )
        .expect("count symbol_features_current rows");
    let embedding_rows: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM symbol_embeddings_current WHERE repo_id = ?1 AND representation_kind = 'code'",
            [cfg.repo.repo_id.as_str()],
            |row| row.get(0),
        )
        .expect("count symbol_embeddings_current rows");

    assert!(
        result.success,
        "sync should succeed with current clone projection"
    );
    assert!(semantic_rows > 0, "current semantics should be populated");
    assert!(
        feature_rows > 0,
        "current semantic features should be populated"
    );
    assert!(
        embedding_rows > 0,
        "current code embeddings should be populated"
    );
}
