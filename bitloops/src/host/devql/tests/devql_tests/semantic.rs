use super::*;
use crate::host::checkpoints::strategy::manual_commit::{WriteCommittedOptions, write_committed};
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use tempfile::TempDir;

#[tokio::test]
async fn init_sqlite_schema_creates_symbol_embeddings_table() {
    let temp = tempdir().expect("temp dir");
    let db_path = temp.path().join("devql.sqlite");

    init_sqlite_schema(&db_path)
        .await
        .expect("initialise sqlite relational schema");

    let conn = rusqlite::Connection::open(&db_path).expect("open sqlite db");
    let mut stmt = conn
        .prepare(
            "SELECT name FROM sqlite_master WHERE type = 'table' AND name = 'symbol_embeddings'",
        )
        .expect("prepare sqlite master query");
    let table_name: String = stmt
        .query_row([], |row| row.get(0))
        .expect("symbol_embeddings table");

    assert_eq!(table_name, "symbol_embeddings");
}

#[tokio::test]
async fn init_relational_schema_creates_test_harness_tables() {
    let temp = tempdir().expect("temp dir");
    let home = TempDir::new().expect("home dir");
    let home_path = home.path().to_string_lossy().to_string();
    let _guard = enter_process_state(
        None,
        &[
            ("HOME", Some(home_path.as_str())),
            ("USERPROFILE", Some(home_path.as_str())),
            ("BITLOOPS_DEVQL_PG_DSN", None),
            ("BITLOOPS_DEVQL_CH_URL", None),
            ("BITLOOPS_DEVQL_CH_USER", None),
            ("BITLOOPS_DEVQL_CH_PASSWORD", None),
            ("BITLOOPS_DEVQL_CH_DATABASE", None),
        ],
    );
    let repo_root = temp.path().join("repo");
    let db_path = repo_root.join("devql.sqlite");
    write_repo_daemon_config(
        &repo_root,
        format!(
            "[stores.relational]\nsqlite_path = {path:?}\n",
            path = db_path.display()
        ),
    );

    let mut cfg = test_cfg();
    cfg.daemon_config_root = repo_root.clone();
    cfg.repo_root = repo_root;
    let relational = RelationalStorage::local_only(db_path.clone());
    init_relational_schema(&cfg, &relational)
        .await
        .expect("initialise sqlite relational schema");

    let conn = rusqlite::Connection::open(&db_path).expect("open sqlite db");
    for table in [
        "test_artefacts_current",
        "test_artefact_edges_current",
        "coverage_captures",
        "coverage_hits",
    ] {
        let table_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
                [table],
                |row| row.get(0),
            )
            .expect("query sqlite master");
        assert_eq!(table_count, 1, "expected sqlite table `{table}`");
    }

    for table in ["test_suites", "test_scenarios", "test_links"] {
        let table_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
                [table],
                |row| row.get(0),
            )
            .expect("query sqlite master");
        assert_eq!(
            table_count, 0,
            "did not expect legacy sqlite table `{table}`"
        );
    }
}

fn semantic_ingest_test_cfg_for_repo(repo_root: &Path) -> DevqlConfig {
    let mut cfg = test_cfg();
    cfg.daemon_config_root = repo_root.to_path_buf();
    cfg.repo_root = repo_root.to_path_buf();
    cfg.repo = resolve_repo_identity(repo_root).expect("resolve repo identity");
    cfg
}

fn semantic_checkpoint_write_options(
    checkpoint_id: &str,
    files_touched: &[&str],
) -> WriteCommittedOptions {
    WriteCommittedOptions {
        checkpoint_id: checkpoint_id.to_string(),
        session_id: format!("session-{checkpoint_id}"),
        strategy: "manual-commit".to_string(),
        agent: "codex".to_string(),
        transcript: br#"{"checkpoint": true}"#.to_vec(),
        prompts: None,
        context: None,
        checkpoints_count: 1,
        files_touched: files_touched
            .iter()
            .map(|path| (*path).to_string())
            .collect(),
        token_usage_input: None,
        token_usage_output: None,
        token_usage_api_call_count: None,
        turn_id: String::new(),
        transcript_identifier_at_start: String::new(),
        checkpoint_transcript_start: 0,
        token_usage: None,
        initial_attribution: None,
        author_name: "Bitloops Test".to_string(),
        author_email: "bitloops-test@example.com".to_string(),
        summary: None,
        is_task: false,
        tool_use_id: String::new(),
        agent_id: String::new(),
        transcript_path: String::new(),
        subagent_transcript_path: String::new(),
    }
}

#[cfg(unix)]
fn fake_runtime_command_and_args(repo_root: &Path) -> (String, Vec<String>) {
    use std::os::unix::fs::PermissionsExt;

    let script_path = repo_root.join(".bitloops/test-bin/fake-embeddings-runtime.sh");
    if let Some(parent) = script_path.parent() {
        fs::create_dir_all(parent).expect("create fake runtime dir");
    }
    let script = r#"#!/bin/sh
provider=${BITLOOPS_TEST_EMBED_PROVIDER:-local_fastembed}
model=${BITLOOPS_TEST_EMBED_MODEL:-bdd-test-model}
dimension=${BITLOOPS_TEST_EMBED_DIMENSION:-3}
profile_name=fake
while [ $# -gt 0 ]; do
  case "$1" in
    --profile)
      profile_name=$2
      shift 2
      ;;
    *)
      shift
      ;;
  esac
done
case "$dimension" in
  4) vector='[0.1,0.2,0.3,0.4]' ;;
  *) vector='[0.1,0.2,0.3]' ;;
esac
while IFS= read -r line; do
  req_id=$(printf '%s\n' "$line" | sed -n 's/.*"request_id":"\([^"]*\)".*/\1/p')
  case "$line" in
    *'"type":"describe"'*)
      printf '{"type":"describe","request_id":"%s","protocol_version":1,"runtime":{"protocol_version":1,"runtime_name":"bitloops-embeddings","runtime_version":"bdd","profile_name":"%s","provider":{"kind":"local_fastembed","provider_name":"%s","model_name":"%s","output_dimension":%s,"cache_dir":null}}}\n' "$req_id" "$profile_name" "$provider" "$model" "$dimension"
      ;;
    *'"type":"embed_batch"'*)
      printf '{"type":"embed_batch","request_id":"%s","protocol_version":1,"vectors":[{"index":0,"values":%s}]}\n' "$req_id" "$vector"
      ;;
    *'"type":"shutdown"'*)
      printf '{"type":"shutdown","request_id":"%s","protocol_version":1,"accepted":true}\n' "$req_id"
      exit 0
      ;;
    *)
      printf '{"type":"error","request_id":"%s","code":"runtime_error","message":"unexpected request"}\n' "$req_id"
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
    let script_path = repo_root.join(".bitloops/test-bin/fake-embeddings-runtime.ps1");
    if let Some(parent) = script_path.parent() {
        fs::create_dir_all(parent).expect("create fake runtime dir");
    }
    let script = r#"
$provider = if ($env:BITLOOPS_TEST_EMBED_PROVIDER) { $env:BITLOOPS_TEST_EMBED_PROVIDER } else { "local_fastembed" }
$model = if ($env:BITLOOPS_TEST_EMBED_MODEL) { $env:BITLOOPS_TEST_EMBED_MODEL } else { "bdd-test-model" }
$dimension = if ($env:BITLOOPS_TEST_EMBED_DIMENSION) { [int]$env:BITLOOPS_TEST_EMBED_DIMENSION } else { 3 }
$profileName = "fake"
for ($i = 0; $i -lt $args.Length; $i++) {
  if ($args[$i] -eq "--profile" -and ($i + 1) -lt $args.Length) {
    $profileName = $args[$i + 1]
    break
  }
}
$vector = if ($dimension -eq 4) { @(0.1, 0.2, 0.3, 0.4) } else { @(0.1, 0.2, 0.3) }
$stdin = [Console]::In
while (($line = $stdin.ReadLine()) -ne $null) {
  if ([string]::IsNullOrWhiteSpace($line)) { continue }
  $request = $line | ConvertFrom-Json
  switch ($request.type) {
    "describe" {
      $response = @{
        type = "describe"
        request_id = $request.request_id
        protocol_version = 1
        runtime = @{
          protocol_version = 1
          runtime_name = "bitloops-embeddings"
          runtime_version = "bdd"
          profile_name = $profileName
          provider = @{
            kind = "local_fastembed"
            provider_name = $provider
            model_name = $model
            output_dimension = $dimension
            cache_dir = $null
          }
        }
      }
    }
    "embed_batch" {
      $response = @{
        type = "embed_batch"
        request_id = $request.request_id
        protocol_version = 1
        vectors = @(@{
          index = 0
          values = $vector
        })
      }
    }
    "shutdown" {
      $response = @{
        type = "shutdown"
        request_id = $request.request_id
        protocol_version = 1
        accepted = $true
      }
      $response | ConvertTo-Json -Compress
      break
    }
    default {
      $response = @{
        type = "error"
        request_id = $request.request_id
        code = "runtime_error"
        message = "unexpected request"
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

fn write_semantic_clone_ingest_config(repo_root: &Path, profile_name: &str) {
    let (command, args) = fake_runtime_command_and_args(repo_root);
    let runtime_args = args
        .iter()
        .map(|arg| format!("{arg:?}"))
        .collect::<Vec<_>>()
        .join(", ");
    write_repo_daemon_config(
        repo_root,
        format!(
            r#"[stores.relational]
sqlite_path = ".bitloops/stores/relational/relational.db"

[stores.events]
duckdb_path = ".bitloops/stores/events.duckdb"

[semantic]
provider = "disabled"

[semantic_clones]
summary_mode = "off"
embedding_mode = "deterministic"
embedding_profile = "{profile_name}"

[embeddings.runtime]
command = {command:?}
args = [{runtime_args}]
startup_timeout_secs = 5
request_timeout_secs = 5

[embeddings.profiles.alpha]
kind = "local_fastembed"
model = "ignored-by-fake-runtime"

[embeddings.profiles.beta]
kind = "local_fastembed"
model = "ignored-by-fake-runtime"
"#
        ),
    );
}

fn seed_direct_ingest_semantic_repo() -> TempDir {
    let repo = seed_git_repo();
    write_repo_daemon_config(
        repo.path(),
        r#"[stores.relational]
sqlite_path = ".bitloops/stores/relational/relational.db"

[stores.events]
duckdb_path = ".bitloops/stores/events.duckdb"
"#,
    );
    let sqlite_path = checkpoint_sqlite_path(repo.path());
    if let Some(parent) = sqlite_path.parent() {
        fs::create_dir_all(parent).expect("create checkpoint sqlite parent");
    }
    rusqlite::Connection::open(&sqlite_path).expect("create checkpoint sqlite file");
    let src_dir = repo.path().join("src");
    fs::create_dir_all(&src_dir).expect("create src dir");

    fs::write(
        src_dir.join("invoice.ts"),
        r#"export function renderInvoice(orderId: string, locale: string): string {
  const invoiceKey = `${orderId}:${locale}`;
  return invoiceKey.toUpperCase();
}
"#,
    )
    .expect("write invoice source");
    git_ok(repo.path(), &["add", "src/invoice.ts"]);
    git_ok(repo.path(), &["commit", "-m", "add invoice source"]);
    let first_sha = git_ok(repo.path(), &["rev-parse", "HEAD"]);
    write_committed(
        repo.path(),
        semantic_checkpoint_write_options("a1b2c3d4e5f6", &["src/invoice.ts"]),
    )
    .expect("write first committed checkpoint");
    insert_commit_checkpoint_mapping(repo.path(), &first_sha, "a1b2c3d4e5f6");

    fs::write(
        src_dir.join("invoice_document.ts"),
        r#"export function renderInvoice(orderId: string, locale: string): string {
  const invoiceKey = `${orderId}:${locale}`;
  return invoiceKey.toUpperCase();
}
"#,
    )
    .expect("write invoice document source");
    git_ok(repo.path(), &["add", "src/invoice_document.ts"]);
    git_ok(
        repo.path(),
        &["commit", "-m", "add invoice document source"],
    );
    let second_sha = git_ok(repo.path(), &["rev-parse", "HEAD"]);
    write_committed(
        repo.path(),
        semantic_checkpoint_write_options("b1c2d3e4f5a6", &["src/invoice_document.ts"]),
    )
    .expect("write second committed checkpoint");
    insert_commit_checkpoint_mapping(repo.path(), &second_sha, "b1c2d3e4f5a6");
    assert_eq!(
        list_committed(repo.path())
            .expect("list committed checkpoints after seeding")
            .len(),
        2
    );

    repo
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CurrentEmbeddingRow {
    symbol_fqn: String,
    path: String,
    provider: String,
    model: String,
    dimension: i64,
    embedding_input_hash: String,
}

fn load_current_embedding_rows(sqlite_path: &Path, repo_id: &str) -> Vec<CurrentEmbeddingRow> {
    let conn = rusqlite::Connection::open(sqlite_path).expect("open sqlite db");
    let mut stmt = conn
        .prepare(
            "SELECT a.symbol_fqn, a.path, e.provider, e.model, e.dimension, e.embedding_input_hash
             FROM artefacts_current a
             JOIN symbol_embeddings e ON e.artefact_id = a.artefact_id
             WHERE a.repo_id = ?1
             ORDER BY a.path, a.start_line, a.symbol_fqn",
        )
        .expect("prepare current embeddings query");
    stmt.query_map([repo_id], |row| {
        Ok(CurrentEmbeddingRow {
            symbol_fqn: row.get(0)?,
            path: row.get(1)?,
            provider: row.get(2)?,
            model: row.get(3)?,
            dimension: row.get(4)?,
            embedding_input_hash: row.get(5)?,
        })
    })
    .expect("query current embeddings")
    .collect::<Result<Vec<_>, _>>()
    .expect("collect current embeddings")
}

fn load_active_setup_row(
    sqlite_path: &Path,
    repo_id: &str,
) -> Option<(String, String, i64, String)> {
    let conn = rusqlite::Connection::open(sqlite_path).expect("open sqlite db");
    conn.query_row(
        "SELECT provider, model, dimension, setup_fingerprint
         FROM semantic_clone_embedding_setup_state
         WHERE repo_id = ?1",
        [repo_id],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
    )
    .ok()
}

fn load_current_embedding_setups(sqlite_path: &Path, repo_id: &str) -> Vec<(String, String, i64)> {
    let conn = rusqlite::Connection::open(sqlite_path).expect("open sqlite db");
    let mut stmt = conn
        .prepare(
            "SELECT DISTINCT e.provider, e.model, e.dimension
             FROM artefacts_current a
             JOIN symbol_embeddings e ON e.artefact_id = a.artefact_id
             WHERE a.repo_id = ?1
             ORDER BY e.provider, e.model, e.dimension",
        )
        .expect("prepare current setup query");
    stmt.query_map([repo_id], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
        .expect("query current setups")
        .collect::<Result<Vec<_>, _>>()
        .expect("collect current setups")
}

fn load_clone_edge_count(sqlite_path: &Path, repo_id: &str) -> i64 {
    let conn = rusqlite::Connection::open(sqlite_path).expect("open sqlite db");
    conn.query_row(
        "SELECT COUNT(*) FROM symbol_clone_edges WHERE repo_id = ?1",
        [repo_id],
        |row| row.get(0),
    )
    .expect("count clone edges")
}

fn hash_by_symbol(rows: &[CurrentEmbeddingRow]) -> BTreeMap<String, String> {
    rows.iter()
        .map(|row| (row.symbol_fqn.clone(), row.embedding_input_hash.clone()))
        .collect()
}

async fn run_direct_ingest_with_env(
    repo_root: &Path,
    max_checkpoints: usize,
    profile_name: &str,
    provider_name: &str,
    model: &str,
    dimension: &str,
) -> IngestionCounters {
    write_semantic_clone_ingest_config(repo_root, profile_name);
    assert_eq!(
        list_committed(repo_root)
            .expect("list committed checkpoints before direct ingest")
            .len(),
        2
    );
    let home = TempDir::new().expect("home dir");
    let home_path = home.path().to_string_lossy().to_string();
    let _guard = enter_process_state(
        Some(repo_root),
        &[
            ("HOME", Some(home_path.as_str())),
            ("USERPROFILE", Some(home_path.as_str())),
            ("BITLOOPS_DEVQL_PG_DSN", None),
            ("BITLOOPS_DEVQL_CH_URL", None),
            ("BITLOOPS_DEVQL_CH_USER", None),
            ("BITLOOPS_DEVQL_CH_PASSWORD", None),
            ("BITLOOPS_DEVQL_CH_DATABASE", None),
            ("BITLOOPS_TEST_EMBED_PROVIDER", Some(provider_name)),
            ("BITLOOPS_TEST_EMBED_MODEL", Some(model)),
            ("BITLOOPS_TEST_EMBED_DIMENSION", Some(dimension)),
        ],
    );
    let cfg = semantic_ingest_test_cfg_for_repo(repo_root);
    execute_init_schema(&cfg, "direct ingest test")
        .await
        .expect("initialise devql schema for direct ingest test");
    let backends = resolve_store_backend_config_for_repo(repo_root)
        .expect("resolve backend config for direct ingest test");
    let relational = RelationalStorage::connect(&cfg, &backends.relational, "direct ingest test")
        .await
        .expect("connect relational storage for direct ingest test");
    execute_sync(
        &cfg,
        &relational,
        crate::host::devql::sync::types::SyncMode::Full,
    )
    .await
    .expect("seed current state before direct ingest");
    let embedding_provider = crate::capability_packs::semantic_clones::extension_descriptor::build_symbol_embedding_provider(
        &embedding_provider_config(&cfg),
        Some(repo_root),
    )
    .expect("build fake embedding provider");
    let embedding_provider = embedding_provider.expect("expected fake embedding provider");
    let setup = crate::capability_packs::semantic_clones::embeddings::resolve_embedding_setup(
        embedding_provider.as_ref(),
    )
    .expect("resolve fake embedding setup");
    assert_eq!(setup.provider, provider_name);
    assert_eq!(setup.model, model);
    assert_eq!(
        setup.dimension,
        dimension.parse::<usize>().expect("parse dimension")
    );

    execute_ingest_with_observer(&cfg, false, max_checkpoints, None, None)
        .await
        .expect("execute direct ingest")
}

#[tokio::test]
async fn direct_ingest_bootstraps_active_embedding_setup_from_single_runtime() {
    let repo = seed_direct_ingest_semantic_repo();

    let summary = run_direct_ingest_with_env(
        repo.path(),
        10,
        "alpha",
        "local_fastembed",
        "bootstrap-model",
        "3",
    )
    .await;

    let cfg = semantic_ingest_test_cfg_for_repo(repo.path());
    let sqlite_path = checkpoint_sqlite_path(repo.path());
    let setup = load_active_setup_row(&sqlite_path, &cfg.repo.repo_id).expect("active setup row");
    let current_rows = load_current_embedding_rows(&sqlite_path, &cfg.repo.repo_id);

    assert!(summary.success);
    assert_eq!(summary.commits_processed, 3);
    assert!(!current_rows.is_empty());
    assert_eq!(setup.0, "local_fastembed");
    assert_eq!(setup.1, "bootstrap-model");
    assert_eq!(setup.2, 3);
    assert_eq!(
        setup.3,
        crate::capability_packs::semantic_clones::embeddings::EmbeddingSetup::new(
            "local_fastembed",
            "bootstrap-model",
            3,
        )
        .setup_fingerprint
    );
    assert_eq!(
        load_current_embedding_setups(&sqlite_path, &cfg.repo.repo_id),
        vec![(
            "local_fastembed".to_string(),
            "bootstrap-model".to_string(),
            3,
        )]
    );
    assert!(load_clone_edge_count(&sqlite_path, &cfg.repo.repo_id) > 0);
    assert!(summary.symbol_clone_edges_upserted > 0);
}

#[tokio::test]
async fn direct_ingest_refreshes_repo_when_provider_or_model_changes() {
    let repo = seed_direct_ingest_semantic_repo();
    let cfg = semantic_ingest_test_cfg_for_repo(repo.path());
    let sqlite_path = checkpoint_sqlite_path(repo.path());

    run_direct_ingest_with_env(repo.path(), 10, "alpha", "local_fastembed", "model-a", "3").await;
    let first_rows = load_current_embedding_rows(&sqlite_path, &cfg.repo.repo_id);
    let first_hashes = hash_by_symbol(&first_rows);

    let second =
        run_direct_ingest_with_env(repo.path(), 1, "alpha", "voyage", "model-b", "3").await;
    let second_rows = load_current_embedding_rows(&sqlite_path, &cfg.repo.repo_id);
    let second_hashes = hash_by_symbol(&second_rows);
    let active_setup =
        load_active_setup_row(&sqlite_path, &cfg.repo.repo_id).expect("active setup row");

    assert_eq!(second.symbol_embedding_rows_upserted, first_rows.len());
    assert_eq!(
        load_current_embedding_setups(&sqlite_path, &cfg.repo.repo_id),
        vec![("voyage".to_string(), "model-b".to_string(), 3)]
    );
    assert_eq!(
        active_setup,
        (
            "voyage".to_string(),
            "model-b".to_string(),
            3,
            crate::capability_packs::semantic_clones::embeddings::EmbeddingSetup::new(
                "voyage", "model-b", 3,
            )
            .setup_fingerprint,
        )
    );
    for (symbol_fqn, first_hash) in first_hashes {
        assert_ne!(
            second_hashes
                .get(&symbol_fqn)
                .expect("symbol hash after refresh"),
            &first_hash
        );
    }
    assert!(load_clone_edge_count(&sqlite_path, &cfg.repo.repo_id) > 0);
}

#[tokio::test]
async fn direct_ingest_does_not_refresh_on_profile_rename_with_same_runtime_descriptor() {
    let repo = seed_direct_ingest_semantic_repo();
    let cfg = semantic_ingest_test_cfg_for_repo(repo.path());
    let sqlite_path = checkpoint_sqlite_path(repo.path());

    run_direct_ingest_with_env(
        repo.path(),
        10,
        "alpha",
        "local_fastembed",
        "stable-model",
        "3",
    )
    .await;
    let first_rows = load_current_embedding_rows(&sqlite_path, &cfg.repo.repo_id);
    let first_hashes = hash_by_symbol(&first_rows);

    let second = run_direct_ingest_with_env(
        repo.path(),
        1,
        "beta",
        "local_fastembed",
        "stable-model",
        "3",
    )
    .await;
    let second_rows = load_current_embedding_rows(&sqlite_path, &cfg.repo.repo_id);
    let second_hashes = hash_by_symbol(&second_rows);
    let active_setup =
        load_active_setup_row(&sqlite_path, &cfg.repo.repo_id).expect("active setup row");

    assert!(second.symbol_embedding_rows_upserted > 0);
    assert!(second.symbol_embedding_rows_upserted < first_rows.len());
    assert_eq!(
        load_current_embedding_setups(&sqlite_path, &cfg.repo.repo_id),
        vec![("local_fastembed".to_string(), "stable-model".to_string(), 3,)]
    );
    assert_eq!(
        active_setup,
        (
            "local_fastembed".to_string(),
            "stable-model".to_string(),
            3,
            crate::capability_packs::semantic_clones::embeddings::EmbeddingSetup::new(
                "local_fastembed",
                "stable-model",
                3,
            )
            .setup_fingerprint,
        )
    );

    let mut unchanged_symbol_count = 0usize;
    let mut changed_latest_checkpoint_symbol_count = 0usize;
    for row in &second_rows {
        let first_hash = first_hashes
            .get(&row.symbol_fqn)
            .expect("symbol present after profile rename");
        if row.path == "src/invoice.ts" {
            assert_eq!(&row.embedding_input_hash, first_hash);
            unchanged_symbol_count += 1;
        }
        if row.path == "src/invoice_document.ts" {
            assert_ne!(&row.embedding_input_hash, first_hash);
            changed_latest_checkpoint_symbol_count += 1;
        }
    }
    assert!(unchanged_symbol_count > 0);
    assert!(changed_latest_checkpoint_symbol_count > 0);
    assert_eq!(
        second_hashes
            .get("src/invoice.ts")
            .expect("file artefact hash for unchanged path"),
        first_hashes
            .get("src/invoice.ts")
            .expect("first file artefact hash for unchanged path")
    );
}

#[tokio::test]
async fn direct_ingest_treats_dimension_change_as_setup_change() {
    let repo = seed_direct_ingest_semantic_repo();
    let cfg = semantic_ingest_test_cfg_for_repo(repo.path());
    let sqlite_path = checkpoint_sqlite_path(repo.path());

    run_direct_ingest_with_env(
        repo.path(),
        10,
        "alpha",
        "local_fastembed",
        "dimension-model",
        "3",
    )
    .await;
    let first_rows = load_current_embedding_rows(&sqlite_path, &cfg.repo.repo_id);
    let first_hashes = hash_by_symbol(&first_rows);

    let second = run_direct_ingest_with_env(
        repo.path(),
        1,
        "alpha",
        "local_fastembed",
        "dimension-model",
        "4",
    )
    .await;
    let second_rows = load_current_embedding_rows(&sqlite_path, &cfg.repo.repo_id);
    let second_hashes = hash_by_symbol(&second_rows);
    let active_setup =
        load_active_setup_row(&sqlite_path, &cfg.repo.repo_id).expect("active setup row");

    assert_eq!(second.symbol_embedding_rows_upserted, first_rows.len());
    assert_eq!(
        load_current_embedding_setups(&sqlite_path, &cfg.repo.repo_id),
        vec![(
            "local_fastembed".to_string(),
            "dimension-model".to_string(),
            4,
        )]
    );
    assert_eq!(
        active_setup,
        (
            "local_fastembed".to_string(),
            "dimension-model".to_string(),
            4,
            crate::capability_packs::semantic_clones::embeddings::EmbeddingSetup::new(
                "local_fastembed",
                "dimension-model",
                4,
            )
            .setup_fingerprint,
        )
    );
    for (symbol_fqn, first_hash) in first_hashes {
        assert_ne!(
            second_hashes
                .get(&symbol_fqn)
                .expect("symbol hash after dimension refresh"),
            &first_hash
        );
    }
    assert!(load_clone_edge_count(&sqlite_path, &cfg.repo.repo_id) > 0);
}
