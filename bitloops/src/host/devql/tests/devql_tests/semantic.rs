use super::*;
use crate::host::checkpoints::strategy::manual_commit::{WriteCommittedOptions, write_committed};
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
async fn init_sqlite_schema_creates_symbol_clone_edges_current_table() {
    let temp = tempdir().expect("temp dir");
    let db_path = temp.path().join("devql.sqlite");

    init_sqlite_schema(&db_path)
        .await
        .expect("initialise sqlite relational schema");

    let conn = rusqlite::Connection::open(&db_path).expect("open sqlite db");
    let mut stmt = conn
        .prepare(
            "SELECT name FROM sqlite_master WHERE type = 'table' AND name = 'symbol_clone_edges_current'",
        )
        .expect("prepare sqlite master query");
    let table_name: String = stmt
        .query_row([], |row| row.get(0))
        .expect("symbol_clone_edges_current table");

    assert_eq!(table_name, "symbol_clone_edges_current");
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
model=${BITLOOPS_TEST_EMBED_MODEL:-bdd-test-model}
dimension=${BITLOOPS_TEST_EMBED_DIMENSION:-3}
case "$dimension" in
  4) vector='[[0.1,0.2,0.3,0.4]]' ;;
  *) vector='[[0.1,0.2,0.3]]' ;;
esac
printf '{"event":"ready","protocol":1,"capabilities":["embed","shutdown"]}\n'
while IFS= read -r line; do
  req_id=$(printf '%s\n' "$line" | sed -n 's/.*"id":"\([^"]*\)".*/\1/p')
  case "$line" in
    *'"cmd":"embed"'*)
      printf '{"id":"%s","ok":true,"vectors":%s,"model":"%s"}\n' "$req_id" "$vector" "$model"
      ;;
    *'"cmd":"shutdown"'*)
      printf '{"id":"%s","ok":true,"model":"%s"}\n' "$req_id" "$model"
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
    let script_path = repo_root.join(".bitloops/test-bin/fake-embeddings-runtime.ps1");
    if let Some(parent) = script_path.parent() {
        fs::create_dir_all(parent).expect("create fake runtime dir");
    }
    let script = r#"
$model = if ($env:BITLOOPS_TEST_EMBED_MODEL) { $env:BITLOOPS_TEST_EMBED_MODEL } else { "bdd-test-model" }
$dimension = if ($env:BITLOOPS_TEST_EMBED_DIMENSION) { [int]$env:BITLOOPS_TEST_EMBED_DIMENSION } else { 3 }
$vector = if ($dimension -eq 4) { @(@(0.1, 0.2, 0.3, 0.4)) } else { @(@(0.1, 0.2, 0.3)) }
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
        vectors = $vector
        model = $model
      }
    }
    "shutdown" {
      $response = @{
        id = $request.id
        ok = $true
        model = $model
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

fn write_semantic_clone_ingest_config(repo_root: &Path, profile_name: &str, model: &str) {
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

[semantic_clones]
summary_mode = "off"
embedding_mode = "deterministic"

[semantic_clones.inference]
code_embeddings = "{profile_name}"
summary_embeddings = "{profile_name}"

[inference.runtimes.bitloops_local_embeddings]
command = {command:?}
args = [{runtime_args}]
startup_timeout_secs = 5
request_timeout_secs = 5

[inference.profiles.alpha]
task = "embeddings"
driver = "bitloops_embeddings_ipc"
runtime = "bitloops_local_embeddings"
model = {model:?}

[inference.profiles.beta]
task = "embeddings"
driver = "bitloops_embeddings_ipc"
runtime = "bitloops_local_embeddings"
model = {model:?}
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
        repo.path().join("package.json"),
        "{\n  \"name\": \"semantic-direct-ingest-test\",\n  \"private\": true,\n  \"devDependencies\": {\n    \"typescript\": \"5.0.0\"\n  }\n}\n",
    )
    .expect("write package.json");
    fs::write(
        repo.path().join("tsconfig.json"),
        "{\n  \"compilerOptions\": {\n    \"target\": \"ES2020\",\n    \"module\": \"ESNext\"\n  }\n}\n",
    )
    .expect("write tsconfig.json");

    fs::write(
        src_dir.join("invoice.ts"),
        r#"export function renderInvoice(orderId: string, locale: string): string {
  const invoiceKey = `${orderId}:${locale}`;
  return invoiceKey.toUpperCase();
}
"#,
    )
    .expect("write invoice source");
    git_ok(
        repo.path(),
        &["add", "package.json", "tsconfig.json", "src/invoice.ts"],
    );
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
    representation_kind: String,
    provider: String,
    model: String,
    dimension: i64,
    embedding_input_hash: String,
}

fn load_current_embedding_rows(sqlite_path: &Path, repo_id: &str) -> Vec<CurrentEmbeddingRow> {
    let conn = rusqlite::Connection::open(sqlite_path).expect("open sqlite db");
    let mut stmt = conn
        .prepare(
            "SELECT a.symbol_fqn, a.path, e.representation_kind, e.provider, e.model, e.dimension, e.embedding_input_hash
             FROM artefacts_current a
             JOIN symbol_embeddings_current e
               ON e.repo_id = a.repo_id
              AND e.artefact_id = a.artefact_id
              AND e.content_id = a.content_id
             WHERE a.repo_id = ?1
             ORDER BY a.path, a.start_line, a.symbol_fqn, e.representation_kind",
        )
        .expect("prepare current embeddings query");
    stmt.query_map([repo_id], |row| {
        Ok(CurrentEmbeddingRow {
            symbol_fqn: row.get(0)?,
            path: row.get(1)?,
            representation_kind: row.get(2)?,
            provider: row.get(3)?,
            model: row.get(4)?,
            dimension: row.get(5)?,
            embedding_input_hash: row.get(6)?,
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

fn load_clone_edge_count(sqlite_path: &Path, repo_id: &str) -> i64 {
    let conn = rusqlite::Connection::open(sqlite_path).expect("open sqlite db");
    conn.query_row(
        "SELECT COUNT(*) FROM symbol_clone_edges_current WHERE repo_id = ?1",
        [repo_id],
        |row| row.get(0),
    )
    .expect("count clone edges")
}

#[tokio::test]
async fn direct_ingest_does_not_produce_semantic_clone_outputs_even_when_embeddings_are_configured()
{
    let repo = seed_direct_ingest_semantic_repo();
    write_semantic_clone_ingest_config(repo.path(), "alpha", "bootstrap-model");
    let home = TempDir::new().expect("home dir");
    let home_path = home.path().to_string_lossy().to_string();
    let _guard = enter_process_state(
        Some(repo.path()),
        &[
            ("HOME", Some(home_path.as_str())),
            ("USERPROFILE", Some(home_path.as_str())),
            ("BITLOOPS_DEVQL_PG_DSN", None),
            ("BITLOOPS_DEVQL_CH_URL", None),
            ("BITLOOPS_DEVQL_CH_USER", None),
            ("BITLOOPS_DEVQL_CH_PASSWORD", None),
            ("BITLOOPS_DEVQL_CH_DATABASE", None),
            ("BITLOOPS_TEST_EMBED_MODEL", Some("bootstrap-model")),
            ("BITLOOPS_TEST_EMBED_DIMENSION", Some("3")),
        ],
    );

    let cfg = semantic_ingest_test_cfg_for_repo(repo.path());
    execute_init_schema(&cfg, "direct ingest semantic no-op test")
        .await
        .expect("initialise devql schema for direct ingest semantic no-op test");
    let summary = execute_ingest_with_observer(&cfg, false, 10, None, None)
        .await
        .expect("execute direct ingest without sync pre-seeding");
    let sqlite_path = checkpoint_sqlite_path(repo.path());
    let sqlite = rusqlite::Connection::open(&sqlite_path).expect("open sqlite db");

    let historical_semantics: i64 = sqlite
        .query_row(
            "SELECT COUNT(*) FROM symbol_semantics WHERE repo_id = ?1",
            [cfg.repo.repo_id.as_str()],
            |row| row.get(0),
        )
        .expect("count historical semantic rows");
    let historical_embeddings: i64 = sqlite
        .query_row(
            "SELECT COUNT(*) FROM symbol_embeddings WHERE repo_id = ?1",
            [cfg.repo.repo_id.as_str()],
            |row| row.get(0),
        )
        .expect("count historical embedding rows");
    let current_semantics: i64 = sqlite
        .query_row(
            "SELECT COUNT(*) FROM symbol_semantics_current WHERE repo_id = ?1",
            [cfg.repo.repo_id.as_str()],
            |row| row.get(0),
        )
        .expect("count current semantic rows");
    let current_embeddings: i64 = sqlite
        .query_row(
            "SELECT COUNT(*) FROM symbol_embeddings_current WHERE repo_id = ?1",
            [cfg.repo.repo_id.as_str()],
            |row| row.get(0),
        )
        .expect("count current embedding rows");

    assert!(summary.success);
    assert_eq!(summary.semantic_feature_rows_upserted, 0);
    assert_eq!(summary.symbol_embedding_rows_upserted, 0);
    assert_eq!(summary.symbol_clone_edges_upserted, 0);
    assert_eq!(historical_semantics, 0);
    assert_eq!(historical_embeddings, 0);
    assert_eq!(current_semantics, 0);
    assert_eq!(current_embeddings, 0);
    assert_eq!(load_clone_edge_count(&sqlite_path, &cfg.repo.repo_id), 0);
    assert_eq!(load_active_setup_row(&sqlite_path, &cfg.repo.repo_id), None);
    assert!(load_current_embedding_rows(&sqlite_path, &cfg.repo.repo_id).is_empty());
}
