use super::*;
use crate::capability_packs::semantic_clones::features::NoopSemanticSummaryProvider;
use crate::capability_packs::semantic_clones::upsert_semantic_feature_rows;
use crate::host::checkpoints::strategy::manual_commit::{WriteCommittedOptions, write_committed};
use crate::host::devql::{
    RelationalStorage, execute_ingest_with_observer, execute_sync, resolve_repo_identity,
};
use crate::test_support::git_fixtures::{git_ok, init_test_repo};
use crate::test_support::process_state::enter_process_state;
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

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

fn write_daemon_embedding_config(repo_root: &Path, profile_name: &str) {
    let (command, args) = fake_runtime_command_and_args(repo_root);
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
    )
    .expect("write daemon embedding config");
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
    provider_name: &str,
    model: &str,
    dimension: &str,
) -> (
    DevqlConfig,
    RelationalStorage,
    Vec<semantic_features::SemanticFeatureInput>,
    BTreeMap<String, String>,
) {
    write_daemon_embedding_config(repo_root, profile_name);
    let sqlite_path = daemon_relational_sqlite_path(repo_root);
    if let Some(parent) = sqlite_path.parent() {
        fs::create_dir_all(parent).expect("create daemon relational db parent");
    }
    rusqlite::Connection::open(&sqlite_path).expect("create daemon relational db file");
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
            embedding_mode: SemanticCloneEmbeddingMode::Deterministic,
        },
    }
}

async fn run_embedding_job_with_env(
    job: &EnrichmentJob,
    provider_name: &str,
    model: &str,
    dimension: &str,
) -> JobExecutionOutcome {
    let home = TempDir::new().expect("home dir");
    let home_path = home.path().to_string_lossy().to_string();
    let _guard = enter_process_state(
        Some(&job.repo_root),
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
    let capability = resolve_embedding_capability_config_for_repo(&job.config_root);
    let provider_config = EmbeddingProviderConfig {
        daemon_config_path: job.config_root.join(BITLOOPS_CONFIG_RELATIVE_PATH),
        embedding_profile: capability.semantic_clones.embedding_profile,
        runtime_command: capability.embeddings.runtime.command,
        runtime_args: capability.embeddings.runtime.args,
        startup_timeout_secs: capability.embeddings.runtime.startup_timeout_secs,
        request_timeout_secs: capability.embeddings.runtime.request_timeout_secs,
        warnings: capability.embeddings.warnings,
    };
    let provider = build_symbol_embedding_provider(&provider_config, Some(&job.repo_root))
        .expect("build fake embedding provider for daemon test")
        .expect("expected fake embedding provider for daemon test");
    let setup = crate::capability_packs::semantic_clones::embeddings::resolve_embedding_setup(
        provider.as_ref(),
    )
    .expect("resolve fake embedding setup for daemon test");
    assert_eq!(setup.provider, provider_name);
    assert_eq!(setup.model, model);
    assert_eq!(
        setup.dimension,
        dimension
            .parse::<usize>()
            .expect("parse daemon test dimension")
    );
    execute_job(job).await
}

#[tokio::test]
async fn daemon_embedding_job_bootstraps_active_setup_from_single_runtime() {
    let (repo, _first_sha, _second_sha) = seed_daemon_embedding_repo();
    let (cfg, _relational, inputs, input_hashes) = seed_current_state_and_semantics(
        repo.path(),
        "alpha",
        "local_fastembed",
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

    let outcome = run_embedding_job_with_env(&job, "local_fastembed", "bootstrap-model", "3").await;

    assert!(outcome.error.is_none());
    assert!(outcome.follow_ups.is_empty());
    assert_eq!(
        load_current_embedding_setups(&sqlite_path, &cfg.repo.repo_id),
        vec![(
            "local_fastembed".to_string(),
            "bootstrap-model".to_string(),
            3,
        )]
    );
    assert_eq!(
        load_active_setup_row(&sqlite_path, &cfg.repo.repo_id),
        Some((
            "local_fastembed".to_string(),
            "bootstrap-model".to_string(),
            3,
            crate::capability_packs::semantic_clones::embeddings::EmbeddingSetup::new(
                "local_fastembed",
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
    let (cfg, _relational, inputs, input_hashes) =
        seed_current_state_and_semantics(repo.path(), "alpha", "local_fastembed", "model-a", "3")
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
    let first = run_embedding_job_with_env(&job, "local_fastembed", "model-a", "3").await;
    assert!(first.error.is_none());
    let first_rows = load_current_embedding_rows(&sqlite_path, &cfg.repo.repo_id);
    let first_hashes = hash_by_symbol(&first_rows);

    let second = run_embedding_job_with_env(&job, "voyage", "model-b", "3").await;
    let second_rows = load_current_embedding_rows(&sqlite_path, &cfg.repo.repo_id);
    let second_hashes = hash_by_symbol(&second_rows);

    assert!(second.error.is_none());
    assert!(second.follow_ups.is_empty());
    assert_eq!(
        load_current_embedding_setups(&sqlite_path, &cfg.repo.repo_id),
        vec![("voyage".to_string(), "model-b".to_string(), 3)]
    );
    assert_eq!(
        load_active_setup_row(&sqlite_path, &cfg.repo.repo_id),
        Some((
            "voyage".to_string(),
            "model-b".to_string(),
            3,
            crate::capability_packs::semantic_clones::embeddings::EmbeddingSetup::new(
                "voyage", "model-b", 3,
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
        "local_fastembed",
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
    let first = run_embedding_job_with_env(&job, "local_fastembed", "dimension-model", "3").await;
    assert!(first.error.is_none());
    let first_rows = load_current_embedding_rows(&sqlite_path, &cfg.repo.repo_id);
    let first_hashes = hash_by_symbol(&first_rows);

    let second = run_embedding_job_with_env(&job, "local_fastembed", "dimension-model", "4").await;
    let second_rows = load_current_embedding_rows(&sqlite_path, &cfg.repo.repo_id);
    let second_hashes = hash_by_symbol(&second_rows);

    assert!(second.error.is_none());
    assert!(second.follow_ups.is_empty());
    assert_eq!(
        load_current_embedding_setups(&sqlite_path, &cfg.repo.repo_id),
        vec![(
            "local_fastembed".to_string(),
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
        "local_fastembed",
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
    let first = run_embedding_job_with_env(&full_job, "local_fastembed", "stable-model", "3").await;
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
    let second =
        run_embedding_job_with_env(&incremental_job, "local_fastembed", "stable-model", "3").await;
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
        vec![("local_fastembed".to_string(), "stable-model".to_string(), 3,)]
    );
    assert_eq!(before_hashes, after_hashes);
}
