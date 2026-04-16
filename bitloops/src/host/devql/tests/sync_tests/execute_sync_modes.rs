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

[inference.runtimes.bitloops_local_embeddings]
command = {command:?}
args = [{runtime_args}]
startup_timeout_secs = 5
request_timeout_secs = 5

[inference.profiles.alpha]
task = "embeddings"
driver = "bitloops_embeddings_ipc"
runtime = "bitloops_local_embeddings"
model = "sync-test-model"
"#
        ),
    )
    .expect("write sync semantic clone config");
}

fn ruff_e501_4_python_fixture_bytes() -> &'static [u8] {
    b"# Regression test for https://github.com/astral-sh/ruff/issues/12130\naaaaaaaaaaaaaaaaaaaaaaaa ://aaaaaaaaaaaaaaaaaaaaaaaa\x00\x00\x00\x00\x00\x00\x00aa\x00a\x00\x00\x00\x00\x00aaaaaaaaaaaaaaaaaaaaaa\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00aaaaaaaaaaaaaaaaa\n"
}

fn seed_ruff_like_rust_local_dependency_repo() -> tempfile::TempDir {
    let dir = tempdir().expect("temp dir");
    crate::test_support::git_fixtures::init_test_repo(
        dir.path(),
        "main",
        "Bitloops Test",
        "bitloops-test@example.com",
    );

    fs::create_dir_all(
        dir.path()
            .join("crates/ruff_linter/src/rules/pyflakes/rules"),
    )
    .expect("create Ruff-like rules dir");

    fs::write(
        dir.path().join("Cargo.toml"),
        "[workspace]\nmembers = [\"crates/ruff_linter\"]\nresolver = \"2\"\n",
    )
    .expect("write workspace Cargo.toml");
    fs::write(
        dir.path().join("crates/ruff_linter/Cargo.toml"),
        "[package]\nname = \"ruff_linter\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .expect("write crate Cargo.toml");
    fs::write(
        dir.path().join("crates/ruff_linter/src/lib.rs"),
        "pub mod rules;\n",
    )
    .expect("write lib.rs");
    fs::write(
        dir.path().join("crates/ruff_linter/src/rules/mod.rs"),
        "pub mod pyflakes;\n",
    )
    .expect("write rules mod.rs");
    fs::write(
        dir.path()
            .join("crates/ruff_linter/src/rules/pyflakes/mod.rs"),
        "pub mod fixes;\npub mod rules;\n",
    )
    .expect("write pyflakes mod.rs");
    fs::write(
        dir.path()
            .join("crates/ruff_linter/src/rules/pyflakes/fixes.rs"),
        "pub(crate) fn remove_unused_positional_arguments_from_format_call() {}\n",
    )
    .expect("write fixes.rs");
    fs::write(
        dir.path()
            .join("crates/ruff_linter/src/rules/pyflakes/rules/mod.rs"),
        "pub mod strings;\n",
    )
    .expect("write nested rules mod.rs");
    fs::write(
        dir.path()
            .join("crates/ruff_linter/src/rules/pyflakes/rules/strings.rs"),
        r#"use super::super::fixes::remove_unused_positional_arguments_from_format_call;

pub(crate) fn string_dot_format_extra_positional_arguments() {
    remove_unused_positional_arguments_from_format_call();
}
"#,
    )
    .expect("write strings.rs");

    crate::test_support::git_fixtures::git_ok(dir.path(), &["add", "."]);
    crate::test_support::git_fixtures::git_ok(dir.path(), &["commit", "-m", "initial"]);
    dir
}

fn seed_typescript_local_dependency_repo(include_target: bool) -> tempfile::TempDir {
    let dir = tempdir().expect("temp dir");
    crate::test_support::git_fixtures::init_test_repo(
        dir.path(),
        "main",
        "Bitloops Test",
        "bitloops-test@example.com",
    );

    fs::create_dir_all(dir.path().join("src")).expect("create src dir");
    fs::write(
        dir.path().join("package.json"),
        "{\n  \"name\": \"ts-sync-fixture\",\n  \"private\": true,\n  \"devDependencies\": {\n    \"typescript\": \"5.0.0\"\n  }\n}\n",
    )
    .expect("write package.json");
    fs::write(
        dir.path().join("tsconfig.json"),
        "{\n  \"compilerOptions\": {\n    \"target\": \"ES2020\",\n    \"module\": \"ESNext\"\n  }\n}\n",
    )
    .expect("write tsconfig.json");
    fs::write(
        dir.path().join("src/caller.ts"),
        "import { helper } from \"./utils\";\n\nexport function run(): number {\n  return helper();\n}\n",
    )
    .expect("write caller.ts");
    if include_target {
        fs::write(
            dir.path().join("src/utils.ts"),
            "export function helper(): number {\n  return 1;\n}\n",
        )
        .expect("write utils.ts");
    }

    crate::test_support::git_fixtures::git_ok(dir.path(), &["add", "."]);
    crate::test_support::git_fixtures::git_ok(dir.path(), &["commit", "-m", "initial"]);
    dir
}

fn seed_ruff_like_rust_grouped_import_repo() -> tempfile::TempDir {
    let dir = tempdir().expect("temp dir");
    crate::test_support::git_fixtures::init_test_repo(
        dir.path(),
        "main",
        "Bitloops Test",
        "bitloops-test@example.com",
    );

    fs::create_dir_all(
        dir.path()
            .join("crates/ruff_linter/src/rules/pyflakes/rules"),
    )
    .expect("create Ruff-like rules dir");

    fs::write(
        dir.path().join("Cargo.toml"),
        "[workspace]\nmembers = [\"crates/ruff_linter\"]\nresolver = \"2\"\n",
    )
    .expect("write workspace Cargo.toml");
    fs::write(
        dir.path().join("crates/ruff_linter/Cargo.toml"),
        "[package]\nname = \"ruff_linter\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .expect("write crate Cargo.toml");
    fs::write(
        dir.path().join("crates/ruff_linter/src/lib.rs"),
        "pub mod rules;\n",
    )
    .expect("write lib.rs");
    fs::write(
        dir.path().join("crates/ruff_linter/src/rules/mod.rs"),
        "pub mod pyflakes;\n",
    )
    .expect("write rules mod.rs");
    fs::write(
        dir.path()
            .join("crates/ruff_linter/src/rules/pyflakes/mod.rs"),
        "pub mod fixes;\npub mod rules;\n",
    )
    .expect("write pyflakes mod.rs");
    fs::write(
        dir.path()
            .join("crates/ruff_linter/src/rules/pyflakes/fixes.rs"),
        "pub(crate) fn remove_unused_positional_arguments_from_format_call() {}\n",
    )
    .expect("write fixes.rs");
    fs::write(
        dir.path()
            .join("crates/ruff_linter/src/rules/pyflakes/rules/mod.rs"),
        "pub mod strings;\n",
    )
    .expect("write nested rules mod.rs");
    fs::write(
        dir.path()
            .join("crates/ruff_linter/src/rules/pyflakes/rules/strings.rs"),
        r#"use super::super::fixes::{remove_unused_positional_arguments_from_format_call, self};

pub(crate) fn string_dot_format_extra_positional_arguments() {
    remove_unused_positional_arguments_from_format_call();
}
"#,
    )
    .expect("write grouped-import strings.rs");

    crate::test_support::git_fixtures::git_ok(dir.path(), &["add", "."]);
    crate::test_support::git_fixtures::git_ok(dir.path(), &["commit", "-m", "initial"]);
    dir
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
async fn unsupported_file_becomes_track_only_current_state() {
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
    let unsupported_language: String = db
        .query_row(
            "SELECT language FROM current_file_state WHERE repo_id = ?1 AND path = ?2",
            [cfg.repo.repo_id.as_str(), "docs/notes.foo"],
            |row| row.get(0),
        )
        .expect("read unsupported current_file_state language");

    assert!(
        result.success,
        "sync should succeed while retaining unsupported files as track-only state"
    );
    assert_eq!(result.paths_added, 2);
    assert_eq!(result.paths_changed, 0);
    assert_eq!(result.paths_removed, 0);
    assert_eq!(result.paths_unchanged, 0);
    assert_eq!(
        current_paths,
        vec!["docs/notes.foo".to_string(), "src/lib.rs".to_string()]
    );
    assert_eq!(unsupported_rows, 1);
    assert_eq!(unsupported_language, "track_only");
}

#[tokio::test]
async fn full_sync_continues_when_one_supported_file_has_invalid_utf8() {
    let repo = tempdir().expect("temp dir");
    crate::test_support::git_fixtures::init_test_repo(
        repo.path(),
        "main",
        "Bitloops Test",
        "bitloops-test@example.com",
    );
    fs::create_dir_all(repo.path().join("src")).expect("create src dir");
    fs::write(
        repo.path().join("Cargo.toml"),
        "[package]\nname = \"invalid-utf8-sync-test\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .expect("write Cargo.toml");
    fs::write(
        repo.path().join("src/good.rs"),
        "pub fn good() -> i32 {\n    1\n}\n",
    )
    .expect("write good rust file");
    fs::write(
        repo.path().join("src/bad.rs"),
        "pub fn bad() -> i32 {\n    2\n}\n",
    )
    .expect("write bad rust file");
    crate::test_support::git_fixtures::git_ok(repo.path(), &["add", "."]);
    crate::test_support::git_fixtures::git_ok(repo.path(), &["commit", "-m", "seed files"]);

    fs::write(
        repo.path().join("src/bad.rs"),
        [
            0x2f, 0x2f, 0x20, 0x62, 0x61, 0x64, 0xff, 0x0a, 0x70, 0x75, 0x62, 0x20, 0x66, 0x6e,
            0x20, 0x62, 0x61, 0x64, 0x28, 0x29, 0x20, 0x2d, 0x3e, 0x20, 0x69, 0x33, 0x32, 0x20,
            0x7b, 0x0a, 0x20, 0x20, 0x20, 0x20, 0x32, 0x0a, 0x7d, 0x0a,
        ],
    )
    .expect("overwrite bad rust file with invalid UTF-8");

    let cfg = sync_test_cfg_for_repo(repo.path());
    let sqlite_path = repo.path().join("devql.sqlite");
    let relational = sqlite_relational_store_with_sync_schema(&sqlite_path).await;

    let result = crate::host::devql::execute_sync(
        &cfg,
        &relational,
        crate::host::devql::sync::types::SyncMode::Full,
    )
    .await
    .expect("execute full sync with one invalid UTF-8 file");

    let db = Connection::open(&sqlite_path).expect("open sqlite db");
    let good_rows: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM current_file_state WHERE repo_id = ?1 AND path = ?2",
            [cfg.repo.repo_id.as_str(), "src/good.rs"],
            |row| row.get(0),
        )
        .expect("count good.rs rows");
    let bad_rows: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM current_file_state WHERE repo_id = ?1 AND path = ?2",
            [cfg.repo.repo_id.as_str(), "src/bad.rs"],
            |row| row.get(0),
        )
        .expect("count bad.rs rows");
    let bad_artefact_rows: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM artefacts_current WHERE repo_id = ?1 AND path = ?2",
            [cfg.repo.repo_id.as_str(), "src/bad.rs"],
            |row| row.get(0),
        )
        .expect("count artefacts_current rows for bad.rs");
    let bad_edge_rows: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM artefact_edges_current WHERE repo_id = ?1 AND path = ?2",
            [cfg.repo.repo_id.as_str(), "src/bad.rs"],
            |row| row.get(0),
        )
        .expect("count artefact_edges_current rows for bad.rs");
    let bad_file_kind: String = db
        .query_row(
            "SELECT canonical_kind FROM artefacts_current WHERE repo_id = ?1 AND path = ?2",
            [cfg.repo.repo_id.as_str(), "src/bad.rs"],
            |row| row.get(0),
        )
        .expect("read canonical_kind for bad.rs");

    assert!(
        result.success,
        "sync should continue even when one supported file fails decoding"
    );
    assert!(
        result.parse_errors >= 1,
        "expected at least one parse error for invalid UTF-8 input"
    );
    assert_eq!(good_rows, 1, "good path should still be materialized");
    assert_eq!(
        bad_rows, 1,
        "decode-degraded path should still be persisted in current_file_state"
    );
    assert_eq!(
        bad_artefact_rows, 1,
        "bad.rs should materialize one file artefact"
    );
    assert_eq!(
        bad_edge_rows, 0,
        "bad.rs should not materialize dependency edges"
    );
    assert_eq!(bad_file_kind, "file");
}

#[tokio::test]
async fn full_sync_keeps_utf8_nul_python_paths_as_file_only() {
    let repo = tempdir().expect("temp dir");
    crate::test_support::git_fixtures::init_test_repo(
        repo.path(),
        "main",
        "Bitloops Test",
        "bitloops-test@example.com",
    );
    fs::create_dir_all(repo.path().join("scripts")).expect("create scripts dir");
    fs::write(
        repo.path().join("scripts/good.py"),
        "def good() -> int:\n    return 1\n",
    )
    .expect("write good python file");
    fs::write(
        repo.path().join("scripts/E501_4.py"),
        "def seed() -> int:\n    return 2\n",
    )
    .expect("write seed python file");
    fs::write(
        repo.path().join("scripts/pyproject.toml"),
        "[project]\nname = \"scripts\"\nversion = \"0.1.0\"\nrequires-python = \">=3.11\"\n",
    )
    .expect("write pyproject.toml");
    crate::test_support::git_fixtures::git_ok(repo.path(), &["add", "."]);
    crate::test_support::git_fixtures::git_ok(repo.path(), &["commit", "-m", "seed python files"]);

    fs::write(
        repo.path().join("scripts/E501_4.py"),
        ruff_e501_4_python_fixture_bytes(),
    )
    .expect("overwrite E501_4.py with NUL-containing UTF-8");

    let cfg = sync_test_cfg_for_repo(repo.path());
    let sqlite_path = repo.path().join("devql.sqlite");
    let relational = sqlite_relational_store_with_sync_schema(&sqlite_path).await;

    let result = crate::host::devql::execute_sync(
        &cfg,
        &relational,
        crate::host::devql::sync::types::SyncMode::Full,
    )
    .await
    .expect("execute full sync with NUL-containing python file");

    let db = Connection::open(&sqlite_path).expect("open sqlite db");
    let good_rows: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM current_file_state WHERE repo_id = ?1 AND path = ?2",
            [cfg.repo.repo_id.as_str(), "scripts/good.py"],
            |row| row.get(0),
        )
        .expect("count good.py rows");
    let bad_rows: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM current_file_state WHERE repo_id = ?1 AND path = ?2",
            [cfg.repo.repo_id.as_str(), "scripts/E501_4.py"],
            |row| row.get(0),
        )
        .expect("count E501_4.py rows");
    let bad_artefact_rows: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM artefacts_current WHERE repo_id = ?1 AND path = ?2",
            [cfg.repo.repo_id.as_str(), "scripts/E501_4.py"],
            |row| row.get(0),
        )
        .expect("count artefacts_current rows for E501_4.py");
    let bad_edge_rows: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM artefact_edges_current WHERE repo_id = ?1 AND path = ?2",
            [cfg.repo.repo_id.as_str(), "scripts/E501_4.py"],
            |row| row.get(0),
        )
        .expect("count artefact_edges_current rows for E501_4.py");
    let bad_semantics_rows: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM symbol_semantics_current WHERE repo_id = ?1 AND path = ?2",
            [cfg.repo.repo_id.as_str(), "scripts/E501_4.py"],
            |row| row.get(0),
        )
        .expect("count symbol_semantics_current rows for E501_4.py");
    let bad_feature_rows: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM symbol_features_current WHERE repo_id = ?1 AND path = ?2",
            [cfg.repo.repo_id.as_str(), "scripts/E501_4.py"],
            |row| row.get(0),
        )
        .expect("count symbol_features_current rows for E501_4.py");
    let bad_file_kind: String = db
        .query_row(
            "SELECT canonical_kind FROM artefacts_current WHERE repo_id = ?1 AND path = ?2",
            [cfg.repo.repo_id.as_str(), "scripts/E501_4.py"],
            |row| row.get(0),
        )
        .expect("read canonical_kind for E501_4.py");

    assert!(
        result.success,
        "sync should continue even when one UTF-8 code file degrades during extraction"
    );
    assert!(
        result.parse_errors >= 1,
        "expected at least one parse error for NUL-containing Python input"
    );
    assert_eq!(good_rows, 1, "good path should still be materialized");
    assert_eq!(
        bad_rows, 1,
        "NUL-containing UTF-8 path should still be persisted in current_file_state"
    );
    assert_eq!(
        bad_artefact_rows, 1,
        "E501_4.py should materialize one file artefact"
    );
    assert_eq!(
        bad_edge_rows, 0,
        "E501_4.py should not materialize dependency edges"
    );
    assert_eq!(
        bad_semantics_rows, 0,
        "E501_4.py should not materialize semantic summaries"
    );
    assert_eq!(
        bad_feature_rows, 0,
        "E501_4.py should not materialize semantic feature rows"
    );
    assert_eq!(bad_file_kind, "file");
}

#[tokio::test]
async fn full_sync_removes_current_rows_for_newly_excluded_paths() {
    let repo = tempdir().expect("temp dir");
    crate::test_support::git_fixtures::init_test_repo(
        repo.path(),
        "main",
        "Bitloops Test",
        "bitloops-test@example.com",
    );
    fs::create_dir_all(repo.path().join("src")).expect("create src dir");
    fs::create_dir_all(repo.path().join("docs")).expect("create docs dir");
    fs::write(
        repo.path().join("src/lib.rs"),
        "pub fn greet() -> &'static str {\n    \"hi\"\n}\n",
    )
    .expect("write rust file");
    fs::write(repo.path().join("docs/readme.md"), "# docs\n").expect("write docs file");
    crate::test_support::git_fixtures::git_ok(repo.path(), &["add", "."]);
    crate::test_support::git_fixtures::git_ok(repo.path(), &["commit", "-m", "seed"]);

    let cfg = sync_test_cfg_for_repo(repo.path());
    let sqlite_path = repo.path().join("devql.sqlite");
    let relational = sqlite_relational_store_with_sync_schema(&sqlite_path).await;

    crate::host::devql::execute_sync(
        &cfg,
        &relational,
        crate::host::devql::sync::types::SyncMode::Full,
    )
    .await
    .expect("execute baseline sync");

    fs::write(
        repo.path().join(crate::config::REPO_POLICY_LOCAL_FILE_NAME),
        r#"
[scope]
exclude = ["docs/**"]
"#,
    )
    .expect("write local exclusions");

    let result = crate::host::devql::execute_sync(
        &cfg,
        &relational,
        crate::host::devql::sync::types::SyncMode::Full,
    )
    .await
    .expect("execute sync after exclusions");

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

    assert!(
        result.success,
        "sync should succeed after exclusions update"
    );
    assert!(
        current_paths.iter().any(|path| path == "src/lib.rs"),
        "expected src/lib.rs to remain indexed after exclusions update"
    );
    assert!(
        !current_paths.iter().any(|path| path == "docs/readme.md"),
        "expected excluded docs/readme.md to be removed from current state"
    );
}

#[tokio::test]
async fn full_sync_removes_current_rows_for_plain_folder_exclusion() {
    let repo = tempdir().expect("temp dir");
    crate::test_support::git_fixtures::init_test_repo(
        repo.path(),
        "main",
        "Bitloops Test",
        "bitloops-test@example.com",
    );
    fs::create_dir_all(repo.path().join("src")).expect("create src dir");
    fs::create_dir_all(repo.path().join("docs")).expect("create docs dir");
    fs::write(
        repo.path().join("src/lib.rs"),
        "pub fn greet() -> &'static str {\n    \"hi\"\n}\n",
    )
    .expect("write rust file");
    fs::write(repo.path().join("docs/readme.md"), "# docs\n").expect("write docs file");
    crate::test_support::git_fixtures::git_ok(repo.path(), &["add", "."]);
    crate::test_support::git_fixtures::git_ok(repo.path(), &["commit", "-m", "seed"]);

    let cfg = sync_test_cfg_for_repo(repo.path());
    let sqlite_path = repo.path().join("devql.sqlite");
    let relational = sqlite_relational_store_with_sync_schema(&sqlite_path).await;

    crate::host::devql::execute_sync(
        &cfg,
        &relational,
        crate::host::devql::sync::types::SyncMode::Full,
    )
    .await
    .expect("execute baseline sync");

    fs::write(
        repo.path().join(crate::config::REPO_POLICY_LOCAL_FILE_NAME),
        r#"
[scope]
exclude = ["docs"]
"#,
    )
    .expect("write local exclusions");

    let result = crate::host::devql::execute_sync(
        &cfg,
        &relational,
        crate::host::devql::sync::types::SyncMode::Full,
    )
    .await
    .expect("execute sync after exclusions");

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

    assert!(
        result.success,
        "sync should succeed after exclusions update"
    );
    assert!(
        current_paths.iter().any(|path| path == "src/lib.rs"),
        "expected src/lib.rs to remain indexed after exclusions update"
    );
    assert!(
        !current_paths.iter().any(|path| path == "docs/readme.md"),
        "expected excluded docs/readme.md to be removed from current state"
    );
}

#[tokio::test]
async fn full_sync_fails_fast_when_exclude_from_file_is_missing() {
    let repo = seed_full_sync_repo();
    fs::write(
        repo.path().join(crate::config::REPO_POLICY_FILE_NAME),
        r#"
[scope]
exclude_from = [".bitloopsignore"]
"#,
    )
    .expect("write shared policy");

    let cfg = sync_test_cfg_for_repo(repo.path());
    let sqlite_path = repo.path().join("devql.sqlite");
    let relational = sqlite_relational_store_with_sync_schema(&sqlite_path).await;

    let err = crate::host::devql::execute_sync(
        &cfg,
        &relational,
        crate::host::devql::sync::types::SyncMode::Full,
    )
    .await
    .expect_err("missing exclude_from file should fail sync");
    let err_chain = format!("{err:#}");
    assert!(
        err_chain.contains("scope.exclude_from"),
        "expected scope.exclude_from error, got: {err_chain}"
    );
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
async fn full_sync_resolves_ruff_style_rust_local_call_after_authoritative_reconciliation() {
    let repo = seed_ruff_like_rust_local_dependency_repo();
    let cfg = sync_test_cfg_for_repo(repo.path());
    let sqlite_path = repo.path().join("devql.sqlite");
    let relational = sqlite_relational_store_with_sync_schema(&sqlite_path).await;

    crate::host::devql::execute_sync(
        &cfg,
        &relational,
        crate::host::devql::sync::types::SyncMode::Full,
    )
    .await
    .expect("execute full sync for Ruff-like repo");

    let db = Connection::open(&sqlite_path).expect("open sqlite db");
    let caller_symbol_fqn = "crates/ruff_linter/src/rules/pyflakes/rules/strings.rs::string_dot_format_extra_positional_arguments";
    let target_symbol_fqn = "crates/ruff_linter/src/rules/pyflakes/fixes.rs::remove_unused_positional_arguments_from_format_call";
    let target_row: (String, String) = db
        .query_row(
            "SELECT symbol_id, artefact_id \
             FROM artefacts_current \
             WHERE repo_id = ?1 AND symbol_fqn = ?2",
            [cfg.repo.repo_id.as_str(), target_symbol_fqn],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("load target artefact row");
    let call_edge: (Option<String>, Option<String>, Option<String>) = db
        .query_row(
            "SELECT e.to_symbol_id, e.to_artefact_id, e.to_symbol_ref \
             FROM artefact_edges_current e \
             JOIN artefacts_current af \
               ON af.repo_id = e.repo_id AND af.artefact_id = e.from_artefact_id \
             WHERE e.repo_id = ?1 AND e.edge_kind = 'calls' AND af.symbol_fqn = ?2",
            [cfg.repo.repo_id.as_str(), caller_symbol_fqn],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .expect("load caller call edge");

    assert_eq!(
        call_edge.0.as_deref(),
        Some(target_row.0.as_str()),
        "full sync should reconcile the caller edge to the helper symbol"
    );
    assert_eq!(
        call_edge.1.as_deref(),
        Some(target_row.1.as_str()),
        "full sync should reconcile the caller edge to the helper artefact"
    );
    assert_eq!(
        call_edge.2.as_deref(),
        Some(target_symbol_fqn),
        "resolved edges should persist the canonical helper symbol FQN"
    );

    let inbound_count: i64 = db
        .query_row(
            "SELECT COUNT(*) \
             FROM artefact_edges_current e \
             WHERE e.repo_id = ?1 AND e.edge_kind = 'calls' AND e.to_artefact_id = ?2",
            [cfg.repo.repo_id.as_str(), target_row.1.as_str()],
            |row| row.get(0),
        )
        .expect("count inbound edges to target artefact");

    assert_eq!(inbound_count, 1);
}

#[tokio::test]
async fn full_sync_expands_grouped_rust_imports_into_resolved_local_edges() {
    let repo = seed_ruff_like_rust_grouped_import_repo();
    let cfg = sync_test_cfg_for_repo(repo.path());
    let sqlite_path = repo.path().join("devql.sqlite");
    let relational = sqlite_relational_store_with_sync_schema(&sqlite_path).await;

    crate::host::devql::execute_sync(
        &cfg,
        &relational,
        crate::host::devql::sync::types::SyncMode::Full,
    )
    .await
    .expect("execute full sync for grouped-import Ruff-like repo");

    let db = Connection::open(&sqlite_path).expect("open sqlite db");
    let refs = {
        let mut stmt = db
            .prepare(
                "SELECT to_symbol_ref \
                 FROM artefact_edges_current \
                 WHERE repo_id = ?1 \
                   AND path = 'crates/ruff_linter/src/rules/pyflakes/rules/strings.rs' \
                   AND edge_kind = 'imports' \
                 ORDER BY to_symbol_ref",
            )
            .expect("prepare grouped rust import query");
        stmt.query_map([cfg.repo.repo_id.as_str()], |row| {
            row.get::<_, Option<String>>(0)
        })
        .expect("query grouped rust import rows")
        .collect::<Result<Vec<_>, _>>()
        .expect("collect grouped rust import rows")
    };

    assert_eq!(
        refs,
        vec![
            Some("crates/ruff_linter/src/rules/pyflakes/fixes.rs".to_string()),
            Some(
                "crates/ruff_linter/src/rules/pyflakes/fixes.rs::remove_unused_positional_arguments_from_format_call"
                    .to_string(),
            ),
        ]
    );
}

#[tokio::test]
async fn full_sync_resolves_typescript_relative_local_call_after_authoritative_reconciliation() {
    let repo = seed_typescript_local_dependency_repo(true);
    let cfg = sync_test_cfg_for_repo(repo.path());
    let sqlite_path = repo.path().join("devql.sqlite");
    let relational = sqlite_relational_store_with_sync_schema(&sqlite_path).await;

    crate::host::devql::execute_sync(
        &cfg,
        &relational,
        crate::host::devql::sync::types::SyncMode::Full,
    )
    .await
    .expect("execute TypeScript full sync");

    let db = Connection::open(&sqlite_path).expect("open sqlite db");
    let helper_symbol_fqn = "src/utils.ts::helper";
    let helper_row: (String, String) = db
        .query_row(
            "SELECT symbol_id, artefact_id \
             FROM artefacts_current \
             WHERE repo_id = ?1 AND symbol_fqn = ?2",
            [cfg.repo.repo_id.as_str(), helper_symbol_fqn],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("load helper artefact row");
    let call_edge: (Option<String>, Option<String>, Option<String>) = db
        .query_row(
            "SELECT e.to_symbol_id, e.to_artefact_id, e.to_symbol_ref \
             FROM artefact_edges_current e \
             JOIN artefacts_current af \
               ON af.repo_id = e.repo_id AND af.artefact_id = e.from_artefact_id \
             WHERE e.repo_id = ?1 AND e.edge_kind = 'calls' AND af.symbol_fqn = 'src/caller.ts::run'",
            [cfg.repo.repo_id.as_str()],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .expect("load caller call edge");

    assert_eq!(call_edge.0.as_deref(), Some(helper_row.0.as_str()));
    assert_eq!(call_edge.1.as_deref(), Some(helper_row.1.as_str()));
    assert_eq!(call_edge.2.as_deref(), Some(helper_symbol_fqn));
}

#[tokio::test]
async fn full_sync_reconciles_previously_unresolved_typescript_import_edge_when_target_appears_later()
 {
    let repo = seed_typescript_local_dependency_repo(false);
    let cfg = sync_test_cfg_for_repo(repo.path());
    let sqlite_path = repo.path().join("devql.sqlite");
    let relational = sqlite_relational_store_with_sync_schema(&sqlite_path).await;

    crate::host::devql::execute_sync(
        &cfg,
        &relational,
        crate::host::devql::sync::types::SyncMode::Full,
    )
    .await
    .expect("execute initial sync without target");

    fs::write(
        repo.path().join("src/utils.ts"),
        "export function helper(): number {\n  return 2;\n}\n",
    )
    .expect("write utils.ts after initial sync");
    crate::test_support::git_fixtures::git_ok(repo.path(), &["add", "src/utils.ts"]);
    crate::test_support::git_fixtures::git_ok(repo.path(), &["commit", "-m", "add utils"]);

    crate::host::devql::execute_sync(
        &cfg,
        &relational,
        crate::host::devql::sync::types::SyncMode::Full,
    )
    .await
    .expect("execute sync after target appears");

    let db = Connection::open(&sqlite_path).expect("open sqlite db");
    let helper_row: (String, String) = db
        .query_row(
            "SELECT symbol_id, artefact_id \
             FROM artefacts_current \
             WHERE repo_id = ?1 AND symbol_fqn = 'src/utils.ts'",
            [cfg.repo.repo_id.as_str()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("load imported file row after target appears");
    let import_edge: (Option<String>, Option<String>, Option<String>) = db
        .query_row(
            "SELECT e.to_symbol_id, e.to_artefact_id, e.to_symbol_ref \
             FROM artefact_edges_current e \
             WHERE e.repo_id = ?1 AND e.path = 'src/caller.ts' AND e.edge_kind = 'imports'",
            [cfg.repo.repo_id.as_str()],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .expect("load caller import edge after target appears");

    assert_eq!(import_edge.0.as_deref(), Some(helper_row.0.as_str()));
    assert_eq!(import_edge.1.as_deref(), Some(helper_row.1.as_str()));
    assert_eq!(import_edge.2.as_deref(), Some("src/utils.ts"));
}

#[tokio::test]
async fn full_sync_reconciles_previously_unresolved_typescript_edge_when_target_appears_later() {
    let repo = seed_typescript_local_dependency_repo(false);
    let cfg = sync_test_cfg_for_repo(repo.path());
    let sqlite_path = repo.path().join("devql.sqlite");
    let relational = sqlite_relational_store_with_sync_schema(&sqlite_path).await;

    crate::host::devql::execute_sync(
        &cfg,
        &relational,
        crate::host::devql::sync::types::SyncMode::Full,
    )
    .await
    .expect("execute initial sync without target");

    fs::write(
        repo.path().join("src/utils.ts"),
        "export function helper(): number {\n  return 2;\n}\n",
    )
    .expect("write utils.ts after initial sync");
    crate::test_support::git_fixtures::git_ok(repo.path(), &["add", "src/utils.ts"]);
    crate::test_support::git_fixtures::git_ok(repo.path(), &["commit", "-m", "add utils"]);

    crate::host::devql::execute_sync(
        &cfg,
        &relational,
        crate::host::devql::sync::types::SyncMode::Full,
    )
    .await
    .expect("execute sync after target appears");

    let db = Connection::open(&sqlite_path).expect("open sqlite db");
    let helper_symbol_fqn = "src/utils.ts::helper";
    let helper_row: (String, String) = db
        .query_row(
            "SELECT symbol_id, artefact_id \
             FROM artefacts_current \
             WHERE repo_id = ?1 AND symbol_fqn = ?2",
            [cfg.repo.repo_id.as_str(), helper_symbol_fqn],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("load helper artefact row after target appears");
    let call_edge: (Option<String>, Option<String>, Option<String>) = db
        .query_row(
            "SELECT e.to_symbol_id, e.to_artefact_id, e.to_symbol_ref \
             FROM artefact_edges_current e \
             JOIN artefacts_current af \
               ON af.repo_id = e.repo_id AND af.artefact_id = e.from_artefact_id \
             WHERE e.repo_id = ?1 AND e.edge_kind = 'calls' AND af.symbol_fqn = 'src/caller.ts::run'",
            [cfg.repo.repo_id.as_str()],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .expect("load caller call edge after target appears");

    assert_eq!(call_edge.0.as_deref(), Some(helper_row.0.as_str()));
    assert_eq!(call_edge.1.as_deref(), Some(helper_row.1.as_str()));
    assert_eq!(call_edge.2.as_deref(), Some(helper_symbol_fqn));
}

#[tokio::test]
async fn full_sync_refreshes_and_clears_canonical_typescript_targets_when_target_changes_or_disappears()
 {
    let repo = seed_typescript_local_dependency_repo(true);
    let cfg = sync_test_cfg_for_repo(repo.path());
    let sqlite_path = repo.path().join("devql.sqlite");
    let relational = sqlite_relational_store_with_sync_schema(&sqlite_path).await;

    crate::host::devql::execute_sync(
        &cfg,
        &relational,
        crate::host::devql::sync::types::SyncMode::Full,
    )
    .await
    .expect("execute baseline sync");

    let db = Connection::open(&sqlite_path).expect("open sqlite db");
    let helper_symbol_fqn = "src/utils.ts::helper";
    let initial_helper_row: (String, String) = db
        .query_row(
            "SELECT symbol_id, artefact_id \
             FROM artefacts_current \
             WHERE repo_id = ?1 AND symbol_fqn = ?2",
            [cfg.repo.repo_id.as_str(), helper_symbol_fqn],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("load initial helper row");

    fs::write(
        repo.path().join("src/utils.ts"),
        "export function helper(): number {\n  return 9;\n}\n",
    )
    .expect("update utils.ts");
    crate::test_support::git_fixtures::git_ok(repo.path(), &["add", "src/utils.ts"]);
    crate::test_support::git_fixtures::git_ok(repo.path(), &["commit", "-m", "update utils"]);

    crate::host::devql::execute_sync(
        &cfg,
        &relational,
        crate::host::devql::sync::types::SyncMode::Full,
    )
    .await
    .expect("execute sync after target refresh");

    let refreshed_helper_row: (String, String) = db
        .query_row(
            "SELECT symbol_id, artefact_id \
             FROM artefacts_current \
             WHERE repo_id = ?1 AND symbol_fqn = ?2",
            [cfg.repo.repo_id.as_str(), helper_symbol_fqn],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("load refreshed helper row");
    let refreshed_edge: (Option<String>, Option<String>, Option<String>) = db
        .query_row(
            "SELECT e.to_symbol_id, e.to_artefact_id, e.to_symbol_ref \
             FROM artefact_edges_current e \
             JOIN artefacts_current af \
               ON af.repo_id = e.repo_id AND af.artefact_id = e.from_artefact_id \
             WHERE e.repo_id = ?1 AND e.edge_kind = 'calls' AND af.symbol_fqn = 'src/caller.ts::run'",
            [cfg.repo.repo_id.as_str()],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .expect("load refreshed caller edge");

    assert_eq!(
        refreshed_edge.0.as_deref(),
        Some(refreshed_helper_row.0.as_str())
    );
    assert_eq!(
        refreshed_edge.1.as_deref(),
        Some(refreshed_helper_row.1.as_str())
    );
    assert_eq!(refreshed_edge.2.as_deref(), Some(helper_symbol_fqn));
    assert_eq!(refreshed_helper_row.0, initial_helper_row.0);
    assert_ne!(refreshed_helper_row.1, initial_helper_row.1);

    fs::remove_file(repo.path().join("src/utils.ts")).expect("remove utils.ts");
    crate::test_support::git_fixtures::git_ok(repo.path(), &["rm", "src/utils.ts"]);
    crate::test_support::git_fixtures::git_ok(repo.path(), &["commit", "-m", "remove utils"]);

    crate::host::devql::execute_sync(
        &cfg,
        &relational,
        crate::host::devql::sync::types::SyncMode::Full,
    )
    .await
    .expect("execute sync after target removal");

    let cleared_edge: (Option<String>, Option<String>, Option<String>) = db
        .query_row(
            "SELECT e.to_symbol_id, e.to_artefact_id, e.to_symbol_ref \
             FROM artefact_edges_current e \
             JOIN artefacts_current af \
               ON af.repo_id = e.repo_id AND af.artefact_id = e.from_artefact_id \
             WHERE e.repo_id = ?1 AND e.edge_kind = 'calls' AND af.symbol_fqn = 'src/caller.ts::run'",
            [cfg.repo.repo_id.as_str()],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .expect("load cleared caller edge");

    assert_eq!(cleared_edge.0, None);
    assert_eq!(cleared_edge.1, None);
    assert_eq!(cleared_edge.2.as_deref(), Some(helper_symbol_fqn));
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
    assert_eq!(result.paths_unchanged, 7);
    assert_eq!(artefact_count, 0);
    assert_eq!(edge_count, 0);
    assert_eq!(current_state_count, 0);
}

#[tokio::test]
async fn sync_populates_current_semantic_tables_with_current_embeddings_and_clone_edges() {
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
    let code_embedding_rows: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM symbol_embeddings_current WHERE repo_id = ?1 AND representation_kind = 'code'",
            [cfg.repo.repo_id.as_str()],
            |row| row.get(0),
        )
        .expect("count code symbol_embeddings_current rows");
    let summary_embedding_rows: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM symbol_embeddings_current WHERE repo_id = ?1 AND representation_kind = 'summary'",
            [cfg.repo.repo_id.as_str()],
            |row| row.get(0),
        )
        .expect("count summary symbol_embeddings_current rows");
    let current_clone_edge_rows: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM symbol_clone_edges_current WHERE repo_id = ?1",
            [cfg.repo.repo_id.as_str()],
            |row| row.get(0),
        )
        .expect("count symbol_clone_edges_current rows");

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
        code_embedding_rows > 0,
        "current code embeddings should be populated during sync"
    );
    assert!(
        summary_embedding_rows > 0,
        "current summary embeddings should be populated during sync"
    );
    assert!(
        current_clone_edge_rows > 0,
        "current clone edges should be rebuilt from current projection during sync"
    );
}

#[tokio::test]
async fn sync_rehydrates_current_semantic_clone_tables_for_unchanged_repo_without_rebuilding_historical_tables()
 {
    let repo = seed_full_sync_repo();
    write_sync_semantic_clone_config(repo.path());
    let cfg = sync_test_cfg_for_repo(repo.path());
    let sqlite_path = repo.path().join("devql.sqlite");
    let relational = sqlite_relational_store_with_sync_schema(&sqlite_path).await;

    crate::host::devql::execute_sync(
        &cfg,
        &relational,
        crate::host::devql::sync::types::SyncMode::Full,
    )
    .await
    .expect("execute baseline sync");

    let db = Connection::open(&sqlite_path).expect("open sqlite db");
    db.execute(
        "DELETE FROM symbol_embeddings WHERE repo_id = ?1",
        [cfg.repo.repo_id.as_str()],
    )
    .expect("delete historical embeddings");
    db.execute(
        "DELETE FROM symbol_embeddings_current WHERE repo_id = ?1",
        [cfg.repo.repo_id.as_str()],
    )
    .expect("delete current embeddings");
    db.execute(
        "DELETE FROM symbol_clone_edges WHERE repo_id = ?1",
        [cfg.repo.repo_id.as_str()],
    )
    .expect("delete historical clone edges");
    db.execute(
        "DELETE FROM symbol_clone_edges_current WHERE repo_id = ?1",
        [cfg.repo.repo_id.as_str()],
    )
    .expect("delete current clone edges");
    db.execute(
        "DELETE FROM semantic_clone_embedding_setup_state WHERE repo_id = ?1",
        [cfg.repo.repo_id.as_str()],
    )
    .expect("delete active embedding setup");

    let result = crate::host::devql::execute_sync(
        &cfg,
        &relational,
        crate::host::devql::sync::types::SyncMode::Full,
    )
    .await
    .expect("execute unchanged sync after semantic clone table reset");
    let db = Connection::open(&sqlite_path).expect("reopen sqlite db after sync");

    let historical_code_embedding_rows: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM symbol_embeddings WHERE repo_id = ?1 AND representation_kind = 'code'",
            [cfg.repo.repo_id.as_str()],
            |row| row.get(0),
        )
        .expect("count historical code embeddings");
    let historical_summary_embedding_rows: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM symbol_embeddings WHERE repo_id = ?1 AND representation_kind = 'summary'",
            [cfg.repo.repo_id.as_str()],
            |row| row.get(0),
        )
        .expect("count historical summary embeddings");
    let current_code_embedding_rows: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM symbol_embeddings_current WHERE repo_id = ?1 AND representation_kind = 'code'",
            [cfg.repo.repo_id.as_str()],
            |row| row.get(0),
        )
        .expect("count current code embeddings");
    let current_summary_embedding_rows: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM symbol_embeddings_current WHERE repo_id = ?1 AND representation_kind = 'summary'",
            [cfg.repo.repo_id.as_str()],
            |row| row.get(0),
        )
        .expect("count current summary embeddings");
    let current_clone_edge_rows: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM symbol_clone_edges_current WHERE repo_id = ?1",
            [cfg.repo.repo_id.as_str()],
            |row| row.get(0),
        )
        .expect("count current clone edges");

    assert!(result.success, "unchanged sync should still succeed");
    assert_eq!(result.paths_added, 0);
    assert_eq!(result.paths_changed, 0);
    assert!(result.paths_unchanged > 0);
    assert_eq!(
        historical_code_embedding_rows, 0,
        "unchanged sync should not repopulate historical code embeddings"
    );
    assert_eq!(
        historical_summary_embedding_rows, 0,
        "unchanged sync should not repopulate historical summary embeddings"
    );
    assert!(
        current_code_embedding_rows > 0,
        "unchanged sync should repopulate current code embeddings"
    );
    assert!(
        current_summary_embedding_rows > 0,
        "unchanged sync should repopulate current summary embeddings"
    );
    assert!(
        current_clone_edge_rows > 0,
        "unchanged sync should rebuild current clone edges"
    );
}

#[tokio::test]
async fn sync_skips_current_semantic_projection_for_decode_degraded_file_only_path() {
    let repo = tempdir().expect("temp dir");
    crate::test_support::git_fixtures::init_test_repo(
        repo.path(),
        "main",
        "Bitloops Test",
        "bitloops-test@example.com",
    );
    fs::create_dir_all(repo.path().join("src")).expect("create src dir");
    fs::write(
        repo.path().join("Cargo.toml"),
        "[package]\nname = \"invalid-utf8-sync-semantics\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .expect("write Cargo.toml");
    fs::write(
        repo.path().join("src/good.rs"),
        "pub fn good() -> i32 {\n    1\n}\n",
    )
    .expect("write good rust file");
    fs::write(
        repo.path().join("src/bad.rs"),
        "pub fn bad() -> i32 {\n    2\n}\n",
    )
    .expect("write bad rust file");
    crate::test_support::git_fixtures::git_ok(repo.path(), &["add", "."]);
    crate::test_support::git_fixtures::git_ok(repo.path(), &["commit", "-m", "seed files"]);
    fs::write(
        repo.path().join("src/bad.rs"),
        [
            0x2f, 0x2f, 0x20, 0x62, 0x61, 0x64, 0xff, 0x0a, 0x70, 0x75, 0x62, 0x20, 0x66, 0x6e,
            0x20, 0x62, 0x61, 0x64, 0x28, 0x29, 0x20, 0x2d, 0x3e, 0x20, 0x69, 0x33, 0x32, 0x20,
            0x7b, 0x0a, 0x20, 0x20, 0x20, 0x20, 0x32, 0x0a, 0x7d, 0x0a,
        ],
    )
    .expect("overwrite bad rust file with invalid UTF-8");
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
    .expect("execute sync with invalid UTF-8 and semantic projection enabled");

    let db = Connection::open(&sqlite_path).expect("open sqlite db");
    let bad_file_rows: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM artefacts_current WHERE repo_id = ?1 AND path = ?2",
            [cfg.repo.repo_id.as_str(), "src/bad.rs"],
            |row| row.get(0),
        )
        .expect("count file artefacts for bad.rs");
    let bad_semantics_rows: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM symbol_semantics_current WHERE repo_id = ?1 AND path = ?2",
            [cfg.repo.repo_id.as_str(), "src/bad.rs"],
            |row| row.get(0),
        )
        .expect("count symbol_semantics_current rows for bad.rs");
    let bad_feature_rows: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM symbol_features_current WHERE repo_id = ?1 AND path = ?2",
            [cfg.repo.repo_id.as_str(), "src/bad.rs"],
            |row| row.get(0),
        )
        .expect("count symbol_features_current rows for bad.rs");
    let good_feature_rows: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM symbol_features_current WHERE repo_id = ?1 AND path = ?2",
            [cfg.repo.repo_id.as_str(), "src/good.rs"],
            |row| row.get(0),
        )
        .expect("count symbol_features_current rows for good.rs");
    let bad_embedding_rows: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM symbol_embeddings_current WHERE repo_id = ?1 AND path = ?2",
            [cfg.repo.repo_id.as_str(), "src/bad.rs"],
            |row| row.get(0),
        )
        .expect("count symbol_embeddings_current rows for bad.rs");
    let good_embedding_rows: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM symbol_embeddings_current WHERE repo_id = ?1 AND path = ?2",
            [cfg.repo.repo_id.as_str(), "src/good.rs"],
            |row| row.get(0),
        )
        .expect("count symbol_embeddings_current rows for good.rs");

    assert!(
        result.success,
        "sync should succeed with decode-degraded input"
    );
    assert!(
        result.parse_errors >= 1,
        "decode degradation should count as a parse error"
    );
    assert_eq!(
        bad_file_rows, 1,
        "bad.rs should still materialize as a file-only path"
    );
    assert_eq!(
        bad_semantics_rows, 0,
        "bad.rs should not project semantic summaries"
    );
    assert_eq!(
        bad_feature_rows, 0,
        "bad.rs should not project semantic features"
    );
    assert_eq!(
        bad_embedding_rows, 0,
        "bad.rs should not project current embeddings"
    );
    assert!(
        good_feature_rows > 0,
        "good.rs should still populate semantic features"
    );
    assert!(
        good_embedding_rows > 0,
        "good.rs should still populate current embeddings"
    );
}
