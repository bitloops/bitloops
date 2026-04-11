use super::*;
use crate::capability_packs::semantic_clones::SEMANTIC_CLONES_CAPABILITY_ID;
use crate::capability_packs::semantic_clones::clear_repo_symbol_embedding_rows;
use crate::capability_packs::semantic_clones::features::NoopSemanticSummaryProvider;
use crate::capability_packs::semantic_clones::runtime_config::resolve_semantic_clones_config;
use crate::capability_packs::semantic_clones::upsert_semantic_feature_rows;
use crate::config::BITLOOPS_CONFIG_RELATIVE_PATH;
use crate::host::checkpoints::strategy::manual_commit::{WriteCommittedOptions, write_committed};
use crate::host::devql::{
    RelationalStorage, build_capability_host, execute_ingest_with_observer, execute_sync,
    resolve_repo_identity,
};
use crate::host::runtime_store::{WorkplaneJobRecord, WorkplaneJobStatus};
use crate::test_support::git_fixtures::{git_ok, init_test_repo};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tempfile::TempDir;

const TEST_EMBEDDINGS_DRIVER: &str = crate::host::inference::BITLOOPS_EMBEDDINGS_IPC_DRIVER;

#[derive(Debug, Clone, PartialEq, Eq)]
struct CurrentEmbeddingRow {
    symbol_fqn: String,
    path: String,
    provider: String,
    model: String,
    dimension: i64,
    embedding_input_hash: String,
}

fn daemon_test_cfg_for_repo(repo_root: &Path) -> DevqlConfig {
    let repo = resolve_repo_identity(repo_root).expect("resolve repo identity");
    DevqlConfig::from_roots(repo_root.to_path_buf(), repo_root.to_path_buf(), repo)
        .expect("build daemon test config")
}

#[cfg(unix)]
fn fake_runtime_command_and_args(
    repo_root: &Path,
    _provider_name: &str,
    model: &str,
    dimension: usize,
) -> (String, Vec<String>) {
    use std::os::unix::fs::PermissionsExt;

    let script_path = repo_root.join(".bitloops/test-bin/fake-embeddings-runtime.sh");
    if let Some(parent) = script_path.parent() {
        fs::create_dir_all(parent).expect("create fake runtime dir");
    }
    let vector = if dimension == 4 {
        "[[0.1,0.2,0.3,0.4]]"
    } else {
        "[[0.1,0.2,0.3]]"
    };
    let script_template = r#"#!/bin/sh
model_name='__MODEL__'
vector='__VECTOR__'
printf '{"event":"ready","protocol":1,"capabilities":["embed","shutdown"]}\n'
while IFS= read -r line; do
  req_id=$(printf '%s\n' "$line" | sed -n 's/.*"id":"\([^"]*\)".*/\1/p')
  case "$line" in
    *'"cmd":"embed"'*)
      printf '{"id":"%s","ok":true,"vectors":%s,"model":"%s"}\n' "$req_id" "$vector" "$model_name"
      ;;
    *'"cmd":"shutdown"'*)
      printf '{"id":"%s","ok":true,"model":"%s"}\n' "$req_id" "$model_name"
      exit 0
      ;;
    *)
      printf '{"id":"%s","ok":false,"error":{"message":"unexpected request"}}\n' "$req_id"
      ;;
  esac
done
exit 0
"#;
    let script = script_template
        .replace("__MODEL__", model)
        .replace("__VECTOR__", vector);
    fs::write(&script_path, script).expect("write fake runtime script");
    let mut permissions = fs::metadata(&script_path)
        .expect("stat fake runtime script")
        .permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&script_path, permissions).expect("chmod fake runtime script");
    ("sh".to_string(), vec![script_path.display().to_string()])
}

#[cfg(windows)]
fn fake_runtime_command_and_args(
    repo_root: &Path,
    _provider_name: &str,
    model: &str,
    dimension: usize,
) -> (String, Vec<String>) {
    let script_path = repo_root.join(".bitloops/test-bin/fake-embeddings-runtime.ps1");
    if let Some(parent) = script_path.parent() {
        fs::create_dir_all(parent).expect("create fake runtime dir");
    }
    let vector = if dimension == 4 {
        "@(@(0.1, 0.2, 0.3, 0.4))"
    } else {
        "@(@(0.1, 0.2, 0.3))"
    };
    let script_template = r#"
$model = "__MODEL__"
$vector = __VECTOR__
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
exit 0
"#;
    let script = script_template
        .replace("__MODEL__", model)
        .replace("__VECTOR__", vector);
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

fn write_daemon_embedding_config(
    repo_root: &Path,
    profile_name: &str,
    provider_name: &str,
    model: &str,
    dimension: usize,
) {
    let (command, args) = fake_runtime_command_and_args(repo_root, provider_name, model, dimension);
    let runtime_args = args
        .iter()
        .map(|arg| format!("{arg:?}"))
        .collect::<Vec<_>>()
        .join(", ");
    let config_path = repo_root.join(BITLOOPS_CONFIG_RELATIVE_PATH);
    if let Some(parent) = config_path.parent() {
        fs::create_dir_all(parent).expect("create config parent");
    }
    fs::write(
        &config_path,
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

[inference.runtimes.bitloops_embeddings]
command = {command:?}
args = [{runtime_args}]
startup_timeout_secs = 5
request_timeout_secs = 5

[inference.profiles.{profile_name}]
task = "embeddings"
driver = "bitloops_embeddings_ipc"
runtime = "bitloops_embeddings"
model = {model:?}
"#
        ),
    )
    .expect("write daemon embedding config");
}

#[test]
fn fake_runtime_scripts_bake_runtime_metadata_without_process_env() {
    let repo = TempDir::new().expect("temp dir");
    let (_command, args) = fake_runtime_command_and_args(repo.path(), "voyage", "model-b", 4);
    let script_path = PathBuf::from(args.last().expect("script path arg"));
    let script = fs::read_to_string(script_path).expect("read fake runtime script");

    assert!(script.contains("model-b"));
    assert!(script.contains("vectors"));
    assert!(!script.contains("BITLOOPS_TEST_EMBED_MODEL"));
}

fn daemon_checkpoint_write_options(
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

fn insert_commit_checkpoint_mapping(repo_root: &Path, commit_sha: &str, checkpoint_id: &str) {
    let sqlite_path = daemon_relational_sqlite_path(repo_root);
    let sqlite =
        crate::storage::SqliteConnectionPool::connect(sqlite_path).expect("connect sqlite");
    sqlite
        .initialise_checkpoint_schema()
        .expect("initialise checkpoint schema");
    let repo_id = crate::host::devql::resolve_repo_id(repo_root).expect("resolve repo id");
    sqlite
        .with_connection(|conn| {
            conn.execute(
                "INSERT INTO commit_checkpoints (commit_sha, checkpoint_id, repo_id)
                 VALUES (?1, ?2, ?3)",
                rusqlite::params![commit_sha, checkpoint_id, repo_id.as_str()],
            )?;
            Ok(())
        })
        .expect("insert commit-checkpoint mapping");
}

fn seed_daemon_embedding_repo() -> (TempDir, String, String) {
    let dir = TempDir::new().expect("temp dir");
    init_test_repo(
        dir.path(),
        "main",
        "Bitloops Test",
        "bitloops-test@example.com",
    );
    git_ok(dir.path(), &["commit", "--allow-empty", "-m", "initial"]);

    let src_dir = dir.path().join("src");
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
    git_ok(dir.path(), &["add", "src/invoice.ts"]);
    git_ok(dir.path(), &["commit", "-m", "add invoice source"]);
    let first_sha = git_ok(dir.path(), &["rev-parse", "HEAD"]);

    fs::write(
        src_dir.join("invoice_document.ts"),
        r#"export function renderInvoice(orderId: string, locale: string): string {
  const invoiceKey = `${orderId}:${locale}`;
  return invoiceKey.toUpperCase();
}
"#,
    )
    .expect("write invoice document source");
    git_ok(dir.path(), &["add", "src/invoice_document.ts"]);
    git_ok(dir.path(), &["commit", "-m", "add invoice document source"]);
    let second_sha = git_ok(dir.path(), &["rev-parse", "HEAD"]);
    (dir, first_sha, second_sha)
}

fn daemon_relational_sqlite_path(repo_root: &Path) -> PathBuf {
    repo_root.join(".bitloops/stores/relational/relational.db")
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
    .collect::<std::result::Result<Vec<_>, _>>()
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
        .collect::<std::result::Result<Vec<_>, _>>()
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

async fn seed_current_state_and_semantics(
    repo_root: &Path,
    profile_name: &str,
    _provider_name: &str,
    model: &str,
    dimension: &str,
) -> (
    DevqlConfig,
    RelationalStorage,
    Vec<semantic_features::SemanticFeatureInput>,
    BTreeMap<String, String>,
) {
    let dimension = dimension
        .parse::<usize>()
        .expect("parse daemon test dimension");
    write_daemon_embedding_config(
        repo_root,
        profile_name,
        TEST_EMBEDDINGS_DRIVER,
        model,
        dimension,
    );
    let sqlite_path = daemon_relational_sqlite_path(repo_root);
    if let Some(parent) = sqlite_path.parent() {
        fs::create_dir_all(parent).expect("create daemon relational db parent");
    }
    rusqlite::Connection::open(&sqlite_path).expect("create daemon relational db file");
    let head_sha = git_ok(repo_root, &["rev-parse", "HEAD"]);
    let first_sha = git_ok(repo_root, &["rev-parse", "HEAD~1"]);
    write_committed(
        repo_root,
        daemon_checkpoint_write_options("a1b2c3d4e5f6", &["src/invoice.ts"]),
    )
    .expect("write first daemon checkpoint");
    insert_commit_checkpoint_mapping(repo_root, &first_sha, "a1b2c3d4e5f6");
    write_committed(
        repo_root,
        daemon_checkpoint_write_options("b1c2d3e4f5a6", &["src/invoice_document.ts"]),
    )
    .expect("write second daemon checkpoint");
    insert_commit_checkpoint_mapping(repo_root, &head_sha, "b1c2d3e4f5a6");

    let cfg = daemon_test_cfg_for_repo(repo_root);
    crate::host::devql::execute_init_schema(&cfg, "daemon test")
        .await
        .expect("initialise devql schema for daemon test");
    let backends = resolve_store_backend_config_for_repo(repo_root)
        .expect("resolve backend config for daemon test");
    let relational = RelationalStorage::connect(&cfg, &backends.relational, "daemon test")
        .await
        .expect("connect relational storage for daemon test");
    execute_ingest_with_observer(&cfg, false, 10, None, None)
        .await
        .expect("ingest daemon checkpoint fixtures");
    execute_sync(
        &cfg,
        &relational,
        crate::host::devql::sync::types::SyncMode::Full,
    )
    .await
    .expect("seed current state for daemon job");
    clear_repo_symbol_embedding_rows(&relational, &cfg.repo.repo_id)
        .await
        .expect("clear seeded embedding rows");
    clear_repo_active_embedding_setup(&relational, &cfg.repo.repo_id)
        .await
        .expect("clear seeded active setup");
    crate::capability_packs::semantic_clones::pipeline::delete_repo_symbol_clone_edges(
        &relational,
        &cfg.repo.repo_id,
    )
    .await
    .expect("clear seeded clone edges");

    let inputs =
        crate::capability_packs::semantic_clones::load_semantic_feature_inputs_for_current_repo(
            &relational,
            repo_root,
            &cfg.repo.repo_id,
        )
        .await
        .expect("load current semantic inputs");
    assert!(
        !inputs.is_empty(),
        "expected current semantic inputs after sync for daemon embedding test"
    );
    let summary_provider = Arc::new(NoopSemanticSummaryProvider);
    upsert_semantic_feature_rows(&relational, &inputs, summary_provider)
        .await
        .expect("upsert semantic rows");

    let input_hashes = inputs
        .iter()
        .map(|input| {
            (
                input.artefact_id.clone(),
                semantic_features::build_semantic_feature_input_hash(
                    input,
                    &NoopSemanticSummaryProvider,
                ),
            )
        })
        .collect::<BTreeMap<_, _>>();

    (cfg, relational, inputs, input_hashes)
}

fn build_embedding_job(
    cfg: &DevqlConfig,
    artefact_ids: Vec<String>,
    input_hashes: BTreeMap<String, String>,
) -> EnrichmentJob {
    EnrichmentJob {
        id: "job-1".to_string(),
        repo_id: cfg.repo.repo_id.clone(),
        repo_root: cfg.repo_root.clone(),
        config_root: cfg.daemon_config_root.clone(),
        branch: "main".to_string(),
        status: crate::daemon::enrichment::EnrichmentJobStatus::Pending,
        attempts: 0,
        error: None,
        created_at_unix: 1,
        updated_at_unix: 1,
        job: EnrichmentJobKind::SymbolEmbeddings {
            batch_key: artefact_ids
                .first()
                .cloned()
                .unwrap_or_else(|| "batch".to_string()),
            artefact_ids,
            input_hashes,
            representation_kind:
                crate::capability_packs::semantic_clones::embeddings::EmbeddingRepresentationKind::Code,
        },
    }
}

async fn run_embedding_job_with_env(
    job: &EnrichmentJob,
    _provider_name: &str,
    model: &str,
    dimension: &str,
) -> JobExecutionOutcome {
    let repo = resolve_repo_identity(&job.repo_root).expect("resolve repo identity for host");
    let capability_host =
        build_capability_host(&job.repo_root, repo).expect("build capability host");
    let semantic_clones =
        resolve_semantic_clones_config(&capability_host.config_view(SEMANTIC_CLONES_CAPABILITY_ID));
    let profile_name = semantic_clones
        .inference
        .code_embeddings
        .clone()
        .or_else(|| semantic_clones.inference.summary_embeddings.clone())
        .clone()
        .unwrap_or_else(|| "alpha".to_string());
    let dimension = dimension
        .parse::<usize>()
        .expect("parse daemon test dimension");
    write_daemon_embedding_config(
        &job.repo_root,
        &profile_name,
        TEST_EMBEDDINGS_DRIVER,
        model,
        dimension,
    );
    let repo = resolve_repo_identity(&job.repo_root).expect("resolve repo identity for inference");
    let capability_host =
        build_capability_host(&job.repo_root, repo).expect("build capability host");
    let provider = capability_host
        .inference()
        .embeddings(&profile_name)
        .expect("build fake embedding service for daemon test");
    assert_eq!(provider.provider_name(), TEST_EMBEDDINGS_DRIVER);
    assert_eq!(provider.model_name(), model);
    assert_eq!(provider.output_dimension(), Some(dimension));
    execute_job(job).await
}

#[tokio::test]
async fn daemon_embedding_job_bootstraps_active_setup_from_single_runtime() {
    let (repo, _first_sha, _second_sha) = seed_daemon_embedding_repo();
    let (cfg, _relational, inputs, input_hashes) = seed_current_state_and_semantics(
        repo.path(),
        "alpha",
        TEST_EMBEDDINGS_DRIVER,
        "bootstrap-model",
        "3",
    )
    .await;
    let sqlite_path = daemon_relational_sqlite_path(repo.path());
    let job = build_embedding_job(
        &cfg,
        inputs
            .iter()
            .map(|input| input.artefact_id.clone())
            .collect(),
        input_hashes,
    );

    let outcome =
        run_embedding_job_with_env(&job, TEST_EMBEDDINGS_DRIVER, "bootstrap-model", "3").await;

    assert!(outcome.error.is_none());
    assert!(outcome.follow_ups.is_empty());
    assert_eq!(
        load_current_embedding_setups(&sqlite_path, &cfg.repo.repo_id),
        vec![(
            TEST_EMBEDDINGS_DRIVER.to_string(),
            "bootstrap-model".to_string(),
            3,
        )]
    );
    assert_eq!(
        load_active_setup_row(&sqlite_path, &cfg.repo.repo_id),
        Some((
            TEST_EMBEDDINGS_DRIVER.to_string(),
            "bootstrap-model".to_string(),
            3,
            crate::capability_packs::semantic_clones::embeddings::EmbeddingSetup::new(
                TEST_EMBEDDINGS_DRIVER,
                "bootstrap-model",
                3,
            )
            .setup_fingerprint,
        ))
    );
    assert!(!load_current_embedding_rows(&sqlite_path, &cfg.repo.repo_id).is_empty());
    assert!(load_clone_edge_count(&sqlite_path, &cfg.repo.repo_id) > 0);
}

#[tokio::test]
async fn daemon_embedding_job_refreshes_repo_when_provider_or_model_changes_without_artefact_churn()
{
    let (repo, _first_sha, _second_sha) = seed_daemon_embedding_repo();
    let (cfg, _relational, inputs, input_hashes) = seed_current_state_and_semantics(
        repo.path(),
        "alpha",
        TEST_EMBEDDINGS_DRIVER,
        "model-a",
        "3",
    )
    .await;
    let sqlite_path = daemon_relational_sqlite_path(repo.path());
    let job = build_embedding_job(
        &cfg,
        inputs
            .iter()
            .map(|input| input.artefact_id.clone())
            .collect(),
        input_hashes.clone(),
    );
    let first = run_embedding_job_with_env(&job, TEST_EMBEDDINGS_DRIVER, "model-a", "3").await;
    assert!(first.error.is_none());
    let first_rows = load_current_embedding_rows(&sqlite_path, &cfg.repo.repo_id);
    let first_hashes = hash_by_symbol(&first_rows);

    let second = run_embedding_job_with_env(&job, TEST_EMBEDDINGS_DRIVER, "model-b", "3").await;
    let second_rows = load_current_embedding_rows(&sqlite_path, &cfg.repo.repo_id);
    let second_hashes = hash_by_symbol(&second_rows);

    assert!(second.error.is_none());
    assert!(second.follow_ups.is_empty());
    assert_eq!(
        load_current_embedding_setups(&sqlite_path, &cfg.repo.repo_id),
        vec![(TEST_EMBEDDINGS_DRIVER.to_string(), "model-b".to_string(), 3)]
    );
    assert_eq!(
        load_active_setup_row(&sqlite_path, &cfg.repo.repo_id),
        Some((
            TEST_EMBEDDINGS_DRIVER.to_string(),
            "model-b".to_string(),
            3,
            crate::capability_packs::semantic_clones::embeddings::EmbeddingSetup::new(
                TEST_EMBEDDINGS_DRIVER,
                "model-b",
                3,
            )
            .setup_fingerprint,
        ))
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
async fn daemon_embedding_job_treats_dimension_change_as_setup_change() {
    let (repo, _first_sha, _second_sha) = seed_daemon_embedding_repo();
    let (cfg, _relational, inputs, input_hashes) = seed_current_state_and_semantics(
        repo.path(),
        "alpha",
        TEST_EMBEDDINGS_DRIVER,
        "dimension-model",
        "3",
    )
    .await;
    let sqlite_path = daemon_relational_sqlite_path(repo.path());
    let job = build_embedding_job(
        &cfg,
        inputs
            .iter()
            .map(|input| input.artefact_id.clone())
            .collect(),
        input_hashes,
    );
    let first =
        run_embedding_job_with_env(&job, TEST_EMBEDDINGS_DRIVER, "dimension-model", "3").await;
    assert!(first.error.is_none());
    let first_rows = load_current_embedding_rows(&sqlite_path, &cfg.repo.repo_id);
    let first_hashes = hash_by_symbol(&first_rows);

    let second =
        run_embedding_job_with_env(&job, TEST_EMBEDDINGS_DRIVER, "dimension-model", "4").await;
    let second_rows = load_current_embedding_rows(&sqlite_path, &cfg.repo.repo_id);
    let second_hashes = hash_by_symbol(&second_rows);

    assert!(second.error.is_none());
    assert!(second.follow_ups.is_empty());
    assert_eq!(
        load_current_embedding_setups(&sqlite_path, &cfg.repo.repo_id),
        vec![(
            TEST_EMBEDDINGS_DRIVER.to_string(),
            "dimension-model".to_string(),
            4,
        )]
    );
    for (symbol_fqn, first_hash) in first_hashes {
        assert_ne!(
            second_hashes
                .get(&symbol_fqn)
                .expect("symbol hash after dimension refresh"),
            &first_hash
        );
    }
}

#[tokio::test]
async fn daemon_embedding_job_keeps_incremental_behavior_when_setup_is_unchanged() {
    let (repo, _first_sha, _second_sha) = seed_daemon_embedding_repo();
    let (cfg, _relational, inputs, input_hashes) = seed_current_state_and_semantics(
        repo.path(),
        "alpha",
        TEST_EMBEDDINGS_DRIVER,
        "stable-model",
        "3",
    )
    .await;
    let sqlite_path = daemon_relational_sqlite_path(repo.path());
    let full_job = build_embedding_job(
        &cfg,
        inputs
            .iter()
            .map(|input| input.artefact_id.clone())
            .collect(),
        input_hashes.clone(),
    );
    let first =
        run_embedding_job_with_env(&full_job, TEST_EMBEDDINGS_DRIVER, "stable-model", "3").await;
    assert!(first.error.is_none());
    let before_rows = load_current_embedding_rows(&sqlite_path, &cfg.repo.repo_id);
    let before_hashes = hash_by_symbol(&before_rows);

    let one_input = inputs
        .iter()
        .find(|input| input.path == "src/invoice.ts")
        .expect("invoice input");
    let incremental_job = build_embedding_job(
        &cfg,
        vec![one_input.artefact_id.clone()],
        BTreeMap::from([(
            one_input.artefact_id.clone(),
            input_hashes
                .get(&one_input.artefact_id)
                .expect("input hash for invoice artefact")
                .clone(),
        )]),
    );
    let second = run_embedding_job_with_env(
        &incremental_job,
        TEST_EMBEDDINGS_DRIVER,
        "stable-model",
        "3",
    )
    .await;
    let after_rows = load_current_embedding_rows(&sqlite_path, &cfg.repo.repo_id);
    let after_hashes = hash_by_symbol(&after_rows);

    assert!(second.error.is_none());
    assert_eq!(second.follow_ups.len(), 1);
    assert!(matches!(
        second.follow_ups.first(),
        Some(FollowUpJob::CloneEdgesRebuild { .. })
    ));
    assert_eq!(
        load_current_embedding_setups(&sqlite_path, &cfg.repo.repo_id),
        vec![(
            TEST_EMBEDDINGS_DRIVER.to_string(),
            "stable-model".to_string(),
            3,
        )]
    );
    assert_eq!(before_hashes, after_hashes);
}

#[tokio::test]
async fn workplane_embedding_mailbox_job_stays_incremental_without_active_state_management() {
    let (repo, _first_sha, _second_sha) = seed_daemon_embedding_repo();
    let (cfg, _relational, inputs, _input_hashes) = seed_current_state_and_semantics(
        repo.path(),
        "alpha",
        TEST_EMBEDDINGS_DRIVER,
        "mailbox-model",
        "3",
    )
    .await;
    let sqlite_path = daemon_relational_sqlite_path(repo.path());
    let selected = inputs
        .iter()
        .find(|input| input.path == "src/invoice.ts")
        .expect("invoice artefact input");
    let job = WorkplaneJobRecord {
        job_id: "workplane-job-1".to_string(),
        repo_id: cfg.repo.repo_id.clone(),
        repo_root: cfg.repo_root.clone(),
        config_root: cfg.daemon_config_root.clone(),
        capability_id: SEMANTIC_CLONES_CAPABILITY_ID.to_string(),
        mailbox_name:
            crate::capability_packs::semantic_clones::types::SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX
                .to_string(),
        dedupe_key: Some(selected.artefact_id.clone()),
        payload: serde_json::json!({ "artefact_id": selected.artefact_id }),
        status: WorkplaneJobStatus::Pending,
        attempts: 0,
        available_at_unix: 1,
        submitted_at_unix: 1,
        started_at_unix: None,
        updated_at_unix: 1,
        completed_at_unix: None,
        lease_owner: None,
        lease_expires_at_unix: None,
        last_error: None,
    };

    let outcome = execute_workplane_job(&job).await;
    let rows = load_current_embedding_rows(&sqlite_path, &cfg.repo.repo_id);

    assert!(outcome.error.is_none());
    assert_eq!(outcome.follow_ups.len(), 1);
    assert!(matches!(
        outcome.follow_ups.first(),
        Some(FollowUpJob::CloneEdgesRebuild { .. })
    ));
    assert_eq!(rows.len(), 1, "workplane job should only embed the selected artefact");
    assert_eq!(rows[0].path, "src/invoice.ts");
    assert_eq!(rows[0].model, "mailbox-model");
    assert_eq!(load_active_setup_row(&sqlite_path, &cfg.repo.repo_id), None);
}
