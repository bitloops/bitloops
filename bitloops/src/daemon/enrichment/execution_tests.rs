use super::*;
use crate::capability_packs::semantic_clones::SEMANTIC_CLONES_CAPABILITY_ID;
use crate::capability_packs::semantic_clones::clear_repo_symbol_embedding_rows;
use crate::capability_packs::semantic_clones::runtime_config::{
    SummaryProviderMode, resolve_semantic_clones_config, resolve_summary_provider,
};
use crate::capability_packs::semantic_clones::upsert_semantic_feature_rows;
use crate::config::BITLOOPS_CONFIG_RELATIVE_PATH;
use crate::host::checkpoints::strategy::manual_commit::{WriteCommittedOptions, write_committed};
use crate::host::devql::{
    RelationalStorage, build_capability_host, execute_ingest_with_observer, execute_sync,
    resolve_repo_identity,
};
use crate::host::runtime_store::{
    SemanticEmbeddingMailboxItemRecord, SemanticMailboxItemKind, SemanticMailboxItemStatus,
    SemanticSummaryMailboxItemRecord, WorkplaneJobRecord, WorkplaneJobStatus,
};
use crate::test_support::git_fixtures::{git_ok, init_test_repo};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tempfile::TempDir;

const TEST_EMBEDDINGS_DRIVER: &str = crate::host::inference::BITLOOPS_EMBEDDINGS_IPC_DRIVER;

#[test]
fn semantic_embedding_work_batches_match_platform_embedding_request_limit() {
    assert_eq!(
        crate::daemon::enrichment::workplane::SEMANTIC_EMBEDDING_MAILBOX_BATCH_SIZE,
        32
    );
}

#[test]
fn repo_backfill_selection_distinguishes_empty_explicit_ids_from_full_repo() {
    let full_repo_items = vec![selection_test_embedding_item(None)];
    let full_repo = super::helpers::select_current_semantic_input_scope(&full_repo_items);
    assert!(full_repo.requested_artefact_ids().is_none());

    let empty_explicit_items = vec![selection_test_embedding_item(Some(serde_json::json!([])))];
    let empty_explicit = super::helpers::select_current_semantic_input_scope(&empty_explicit_items);
    assert!(
        empty_explicit
            .requested_artefact_ids()
            .is_some_and(|ids| ids.is_empty())
    );
}

fn selection_test_embedding_item(
    payload_json: Option<serde_json::Value>,
) -> SemanticEmbeddingMailboxItemRecord {
    SemanticEmbeddingMailboxItemRecord {
        item_id: "selection-test-item".to_string(),
        repo_id: "repo-selection-test".to_string(),
        repo_root: PathBuf::from("/tmp/repo-selection-test"),
        config_root: PathBuf::from("/tmp/repo-selection-test"),
        init_session_id: None,
        representation_kind: "code".to_string(),
        item_kind: SemanticMailboxItemKind::RepoBackfill,
        artefact_id: None,
        payload_json,
        dedupe_key: Some("selection-test-dedupe".to_string()),
        status: SemanticMailboxItemStatus::Leased,
        attempts: 0,
        available_at_unix: 1,
        submitted_at_unix: 1,
        leased_at_unix: Some(1),
        lease_expires_at_unix: Some(301),
        lease_token: Some("selection-test-lease".to_string()),
        updated_at_unix: 1,
        last_error: None,
    }
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

fn summary_plan_input(index: usize) -> SemanticFeatureInput {
    SemanticFeatureInput {
        artefact_id: format!("artefact-{index}"),
        symbol_id: Some(format!("symbol-{index}")),
        repo_id: "repo-summary-plan".to_string(),
        blob_sha: format!("blob-{index}"),
        path: format!("src/file_{index}.ts"),
        language: "typescript".to_string(),
        canonical_kind: "function".to_string(),
        language_kind: "function".to_string(),
        symbol_fqn: format!("src/file_{index}.ts::fn_{index}"),
        name: format!("fn_{index}"),
        signature: Some(format!("function fn_{index}(): string")),
        modifiers: vec!["export".to_string()],
        body: format!("return '{index}';"),
        docstring: Some(format!("Summary input {index}")),
        parent_kind: None,
        dependency_signals: Vec::new(),
        content_hash: Some(format!("content-{index}")),
    }
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
        "[0.1,0.2,0.3,0.4]"
    } else {
        "[0.1,0.2,0.3]"
    };
    let request_log_path = fake_embedding_request_log_path(repo_root);
    let script_template = r#"#!/bin/sh
model_name='__MODEL__'
vector='__VECTOR__'
request_log='__REQUEST_LOG__'
printf '{"event":"ready","protocol":1,"capabilities":["embed","shutdown"]}\n'
while IFS= read -r line; do
  req_id=$(printf '%s\n' "$line" | sed -n 's/.*"id":"\([^"]*\)".*/\1/p')
  case "$line" in
    *'"cmd":"embed"'*)
      printf '%s\n' "$line" >> "$request_log"
      texts_payload=$(printf '%s\n' "$line" | sed -n 's/.*"texts":\[\(.*\)\].*/\1/p')
      text_count=0
      if [ -n "$texts_payload" ]; then
        text_count=1
        remaining="$texts_payload"
        while [ "$remaining" != "${remaining#*\",\"}" ]; do
          text_count=$((text_count + 1))
          remaining="${remaining#*\",\"}"
        done
      fi
      vectors="$vector"
      while [ "$text_count" -gt 1 ]; do
        vectors="$vectors,$vector"
        text_count=$((text_count - 1))
      done
      printf '{"id":"%s","ok":true,"vectors":[%s],"model":"%s"}\n' "$req_id" "$vectors" "$model_name"
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
        .replace("__VECTOR__", vector)
        .replace("__REQUEST_LOG__", &request_log_path.to_string_lossy());
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
        "@(0.1, 0.2, 0.3, 0.4)"
    } else {
        "@(0.1, 0.2, 0.3)"
    };
    let request_log_path = fake_embedding_request_log_path(repo_root);
    let script_template = r#"
$model = "__MODEL__"
$vector = __VECTOR__
$requestLog = "__REQUEST_LOG__"
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
      Add-Content -Path $requestLog -Value $line
      $vectors = @()
      foreach ($text in $request.texts) {
        $vectors += ,$vector
      }
      $response = @{
        id = $request.id
        ok = $true
        vectors = $vectors
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
        .replace("__VECTOR__", vector)
        .replace("__REQUEST_LOG__", &request_log_path.to_string_lossy());
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

fn fake_embedding_request_log_path(repo_root: &Path) -> PathBuf {
    repo_root.join(".bitloops/test-bin/fake-embeddings-requests.log")
}

fn fake_embedding_request_lines(repo_root: &Path) -> Vec<String> {
    fs::read_to_string(fake_embedding_request_log_path(repo_root))
        .unwrap_or_default()
        .lines()
        .map(str::to_string)
        .collect()
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

[inference.runtimes.bitloops_local_embeddings]
command = {command:?}
args = [{runtime_args}]
startup_timeout_secs = 5
request_timeout_secs = 5

[inference.profiles.{profile_name}]
task = "embeddings"
driver = "bitloops_embeddings_ipc"
runtime = "bitloops_local_embeddings"
model = {model:?}
"#
        ),
    )
    .expect("write daemon embedding config");
}

fn remove_summary_embedding_slot(repo_root: &Path, profile_name: &str) {
    let config_path = repo_root.join(BITLOOPS_CONFIG_RELATIVE_PATH);
    let config = fs::read_to_string(&config_path).expect("read daemon embedding config");
    let summary_slot = format!("summary_embeddings = \"{profile_name}\"\n");
    fs::write(config_path, config.replace(&summary_slot, ""))
        .expect("write daemon code-only embedding config");
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

#[test]
fn repo_backfill_summary_refresh_plan_batches_initial_work_and_queues_follow_ups() {
    let job = WorkplaneJobRecord {
        job_id: "summary-backfill-1".to_string(),
        repo_id: "repo-summary-plan".to_string(),
        repo_root: PathBuf::from("/tmp/repo-summary-plan"),
        config_root: PathBuf::from("/tmp/config-summary-plan"),
        capability_id: SEMANTIC_CLONES_CAPABILITY_ID.to_string(),
        mailbox_name:
            crate::capability_packs::semantic_clones::types::SEMANTIC_CLONES_SUMMARY_REFRESH_MAILBOX
                .to_string(),
        init_session_id: None,
        dedupe_key: Some(
            crate::capability_packs::semantic_clones::workplane::repo_backfill_dedupe_key(
                crate::capability_packs::semantic_clones::types::SEMANTIC_CLONES_SUMMARY_REFRESH_MAILBOX,
            ),
        ),
        payload: serde_json::to_value(
            crate::capability_packs::semantic_clones::workplane::SemanticClonesMailboxPayload::RepoBackfill {
                work_item_count: Some(2),
                artefact_ids: None,
            },
        )
        .expect("repo backfill payload"),
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
    let inputs = (0..40).map(summary_plan_input).collect::<Vec<_>>();

    let plan = build_summary_refresh_workplane_plan(&job, inputs, true);

    assert_eq!(
        plan.inputs.len(),
        WORKPLANE_SUMMARY_REPO_BACKFILL_BATCH_SIZE
    );
    assert_eq!(plan.follow_ups.len(), 2);

    match &plan.follow_ups[0] {
        FollowUpJob::SymbolEmbeddings {
            target,
            artefact_ids,
            representation_kind,
            ..
        } => {
            assert_eq!(target.repo_root, PathBuf::from("/tmp/repo-summary-plan"));
            assert_eq!(
                target.config_root,
                PathBuf::from("/tmp/config-summary-plan")
            );
            assert_eq!(
                *representation_kind,
                crate::capability_packs::semantic_clones::embeddings::EmbeddingRepresentationKind::Summary
            );
            assert_eq!(
                artefact_ids.len(),
                WORKPLANE_SUMMARY_REPO_BACKFILL_BATCH_SIZE
            );
            assert_eq!(artefact_ids.first().map(String::as_str), Some("artefact-0"));
            assert_eq!(artefact_ids.last().map(String::as_str), Some("artefact-23"));
        }
        other => panic!("expected summary-embedding follow-up, got {other:?}"),
    }

    match &plan.follow_ups[1] {
        FollowUpJob::RepoBackfillSummaries {
            target,
            artefact_ids,
        } => {
            assert_eq!(target.repo_root, PathBuf::from("/tmp/repo-summary-plan"));
            assert_eq!(
                target.config_root,
                PathBuf::from("/tmp/config-summary-plan")
            );
            assert_eq!(artefact_ids.len(), 16);
            assert_eq!(
                artefact_ids.first().map(String::as_str),
                Some("artefact-24")
            );
            assert_eq!(artefact_ids.last().map(String::as_str), Some("artefact-39"));
        }
        other => panic!("expected summary follow-up batch, got {other:?}"),
    }
}

#[test]
fn repo_backfill_embedding_plan_batches_initial_work_and_queues_follow_up_backfill() {
    let job = WorkplaneJobRecord {
        job_id: "embedding-backfill-1".to_string(),
        repo_id: "repo-embedding-plan".to_string(),
        repo_root: PathBuf::from("/tmp/repo-embedding-plan"),
        config_root: PathBuf::from("/tmp/config-embedding-plan"),
        capability_id: SEMANTIC_CLONES_CAPABILITY_ID.to_string(),
        mailbox_name:
            crate::capability_packs::semantic_clones::types::SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX
                .to_string(),
        init_session_id: Some("init-session-embedding-plan".to_string()),
        dedupe_key: Some(
            crate::capability_packs::semantic_clones::workplane::repo_backfill_dedupe_key(
                crate::capability_packs::semantic_clones::types::SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX,
            ),
        ),
        payload: serde_json::to_value(
            crate::capability_packs::semantic_clones::workplane::SemanticClonesMailboxPayload::RepoBackfill {
                work_item_count: Some(40),
                artefact_ids: None,
            },
        )
        .expect("repo backfill payload"),
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
    let inputs = (0..40).map(summary_plan_input).collect::<Vec<_>>();

    let plan = build_embedding_refresh_workplane_plan(
        &job,
        SymbolEmbeddingsRefreshScope::Historical,
        None,
        None,
        inputs,
        crate::capability_packs::semantic_clones::embeddings::EmbeddingRepresentationKind::Code,
    );

    assert_eq!(
        plan.inputs.len(),
        WORKPLANE_EMBEDDING_REPO_BACKFILL_BATCH_SIZE
    );
    assert!(
        !plan.manage_active_state,
        "intermediate repo-backfill batches must not try to activate the setup yet"
    );
    assert_eq!(plan.follow_ups.len(), 1);

    match &plan.follow_ups[0] {
        FollowUpJob::RepoBackfillEmbeddings {
            target,
            artefact_ids,
            representation_kind,
            ..
        } => {
            assert_eq!(target.repo_root, PathBuf::from("/tmp/repo-embedding-plan"));
            assert_eq!(
                target.config_root,
                PathBuf::from("/tmp/config-embedding-plan")
            );
            assert_eq!(
                *representation_kind,
                crate::capability_packs::semantic_clones::embeddings::EmbeddingRepresentationKind::Code
            );
            assert_eq!(artefact_ids.len(), 32);
            assert_eq!(artefact_ids.first().map(String::as_str), Some("artefact-8"));
            assert_eq!(artefact_ids.last().map(String::as_str), Some("artefact-39"));
        }
        other => panic!("expected embedding repo-backfill follow-up, got {other:?}"),
    }
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
    fs::write(
        dir.path().join("package.json"),
        "{\n  \"name\": \"daemon-embedding-test\",\n  \"private\": true,\n  \"devDependencies\": {\n    \"typescript\": \"5.0.0\"\n  }\n}\n",
    )
    .expect("write package.json");
    fs::write(
        dir.path().join("tsconfig.json"),
        "{\n  \"compilerOptions\": {\n    \"target\": \"ES2020\",\n    \"module\": \"ESNext\"\n  }\n}\n",
    )
    .expect("write tsconfig.json");
    git_ok(dir.path(), &["add", "package.json", "tsconfig.json"]);
    git_ok(dir.path(), &["commit", "-m", "initial"]);

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
             JOIN symbol_embeddings_current e
               ON e.repo_id = a.repo_id
              AND e.artefact_id = a.artefact_id
              AND e.content_id = a.content_id
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
             JOIN symbol_embeddings_current e
               ON e.repo_id = a.repo_id
              AND e.artefact_id = a.artefact_id
              AND e.content_id = a.content_id
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
        "SELECT COUNT(*) FROM symbol_clone_edges_current WHERE repo_id = ?1",
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
    crate::capability_packs::semantic_clones::pipeline::delete_repo_current_symbol_clone_edges(
        &relational,
        &cfg.repo.repo_id,
    )
    .await
    .expect("clear seeded current clone edges");

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
    let repo = resolve_repo_identity(repo_root).expect("resolve repo identity for summary seed");
    let capability_host =
        build_capability_host(repo_root, repo).expect("build capability host for summary seed");
    let semantic_clones =
        resolve_semantic_clones_config(&capability_host.config_view(SEMANTIC_CLONES_CAPABILITY_ID));
    let summary_provider = resolve_summary_provider(
        &semantic_clones,
        &capability_host.inference_for_capability(SEMANTIC_CLONES_CAPABILITY_ID),
        SummaryProviderMode::ConfiguredStrict,
    )
    .expect("resolve summary provider for semantic seed")
    .provider;
    upsert_semantic_feature_rows(&relational, &inputs, Arc::clone(&summary_provider))
        .await
        .expect("upsert semantic rows");

    let input_hashes = inputs
        .iter()
        .map(|input| {
            (
                input.artefact_id.clone(),
                semantic_features::build_semantic_feature_input_hash(
                    input,
                    summary_provider.as_ref(),
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
    build_embedding_job_for_representation(
        cfg,
        artefact_ids,
        input_hashes,
        crate::capability_packs::semantic_clones::embeddings::EmbeddingRepresentationKind::Code,
    )
}

fn build_embedding_job_for_representation(
    cfg: &DevqlConfig,
    artefact_ids: Vec<String>,
    input_hashes: BTreeMap<String, String>,
    representation_kind:
        crate::capability_packs::semantic_clones::embeddings::EmbeddingRepresentationKind,
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
            representation_kind,
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
async fn daemon_summary_embedding_job_skips_clone_rebuild_without_summary_provider() {
    let (repo, _first_sha, _second_sha) = seed_daemon_embedding_repo();
    let (cfg, _relational, inputs, input_hashes) = seed_current_state_and_semantics(
        repo.path(),
        "alpha",
        TEST_EMBEDDINGS_DRIVER,
        "stable-model",
        "3",
    )
    .await;

    let code_job = build_embedding_job(
        &cfg,
        inputs
            .iter()
            .map(|input| input.artefact_id.clone())
            .collect(),
        input_hashes.clone(),
    );
    let first_code =
        run_embedding_job_with_env(&code_job, TEST_EMBEDDINGS_DRIVER, "stable-model", "3").await;
    assert!(first_code.error.is_none());

    let full_summary_job = build_embedding_job_for_representation(
        &cfg,
        inputs
            .iter()
            .map(|input| input.artefact_id.clone())
            .collect(),
        input_hashes.clone(),
        crate::capability_packs::semantic_clones::embeddings::EmbeddingRepresentationKind::Summary,
    );
    let first_summary = run_embedding_job_with_env(
        &full_summary_job,
        TEST_EMBEDDINGS_DRIVER,
        "stable-model",
        "3",
    )
    .await;
    assert!(first_summary.error.is_none());

    let one_input = inputs
        .iter()
        .find(|input| input.path == "src/invoice.ts")
        .expect("invoice input");
    let incremental_summary_job = build_embedding_job_for_representation(
        &cfg,
        vec![one_input.artefact_id.clone()],
        BTreeMap::from([(
            one_input.artefact_id.clone(),
            input_hashes
                .get(&one_input.artefact_id)
                .expect("input hash for invoice artefact")
                .clone(),
        )]),
        crate::capability_packs::semantic_clones::embeddings::EmbeddingRepresentationKind::Summary,
    );
    let second_summary = run_embedding_job_with_env(
        &incremental_summary_job,
        TEST_EMBEDDINGS_DRIVER,
        "stable-model",
        "3",
    )
    .await;

    assert!(second_summary.error.is_none());
    assert!(
        second_summary.follow_ups.is_empty(),
        "providerless summary embeddings should not keep scheduling clone rebuild follow-ups"
    );
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
        init_session_id: None,
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
    assert_eq!(
        rows.len(),
        1,
        "workplane job should only embed the selected artefact"
    );
    assert_eq!(rows[0].path, "src/invoice.ts");
    assert_eq!(rows[0].model, "mailbox-model");
    assert_eq!(load_active_setup_row(&sqlite_path, &cfg.repo.repo_id), None);
}

#[tokio::test]
async fn repo_backfill_workplane_inputs_exclude_historical_only_artefacts() {
    use std::collections::BTreeSet;

    let repo = TempDir::new().expect("temp dir");
    init_test_repo(
        repo.path(),
        "main",
        "Bitloops Test",
        "bitloops-test@example.com",
    );
    fs::write(
        repo.path().join("package.json"),
        "{\n  \"name\": \"daemon-backfill-test\",\n  \"private\": true,\n  \"devDependencies\": {\n    \"typescript\": \"5.0.0\"\n  }\n}\n",
    )
    .expect("write package.json");
    fs::write(
        repo.path().join("tsconfig.json"),
        "{\n  \"compilerOptions\": {\n    \"target\": \"ES2020\",\n    \"module\": \"ESNext\"\n  }\n}\n",
    )
    .expect("write tsconfig.json");
    git_ok(repo.path(), &["add", "package.json", "tsconfig.json"]);
    git_ok(repo.path(), &["commit", "-m", "initial"]);

    let src_dir = repo.path().join("src");
    fs::create_dir_all(&src_dir).expect("create src dir");
    fs::write(
        src_dir.join("legacy.ts"),
        "export function legacyInvoice(): string { return 'legacy'; }\n",
    )
    .expect("write legacy source");
    git_ok(repo.path(), &["add", "src/legacy.ts"]);
    git_ok(repo.path(), &["commit", "-m", "add legacy artefact"]);

    fs::remove_file(src_dir.join("legacy.ts")).expect("remove legacy source");
    fs::write(
        src_dir.join("current.ts"),
        "export function currentInvoice(): string { return 'current'; }\n",
    )
    .expect("write current source");
    git_ok(repo.path(), &["add", "-A"]);
    git_ok(repo.path(), &["commit", "-m", "replace legacy artefact"]);

    write_daemon_embedding_config(
        repo.path(),
        "alpha",
        TEST_EMBEDDINGS_DRIVER,
        "repo-backfill-model",
        3,
    );

    let cfg = daemon_test_cfg_for_repo(repo.path());
    crate::host::devql::execute_init_schema(&cfg, "repo backfill daemon test")
        .await
        .expect("initialise devql schema for repo backfill test");
    let backends = resolve_store_backend_config_for_repo(repo.path())
        .expect("resolve backend config for repo backfill test");
    let relational =
        RelationalStorage::connect(&cfg, &backends.relational, "repo backfill daemon test")
            .await
            .expect("connect relational storage for repo backfill test");
    execute_ingest_with_observer(&cfg, false, 10, None, None)
        .await
        .expect("ingest repo backfill fixture");
    execute_sync(
        &cfg,
        &relational,
        crate::host::devql::sync::types::SyncMode::Full,
    )
    .await
    .expect("seed current state for repo backfill test");

    let current_inputs =
        crate::capability_packs::semantic_clones::load_semantic_feature_inputs_for_current_repo(
            &relational,
            repo.path(),
            &cfg.repo.repo_id,
        )
        .await
        .expect("load current semantic inputs");
    let historical_legacy_rows = relational
        .query_rows(&format!(
            "SELECT COUNT(*) AS count FROM file_state WHERE repo_id = '{}' AND path = 'src/legacy.ts'",
            crate::host::devql::esc_pg(&cfg.repo.repo_id),
        ))
        .await
        .expect("count historical legacy file rows");
    assert!(
        historical_legacy_rows
            .first()
            .and_then(|row| row.get("count"))
            .and_then(serde_json::Value::as_i64)
            .unwrap_or_default()
            > 0,
        "fixture must include a historical-only artefact"
    );
    assert!(
        current_inputs
            .iter()
            .all(|input| input.path != "src/legacy.ts"),
        "current inputs must exclude the deleted legacy artefact"
    );

    let job = WorkplaneJobRecord {
        job_id: "repo-backfill-job-1".to_string(),
        repo_id: cfg.repo.repo_id.clone(),
        repo_root: cfg.repo_root.clone(),
        config_root: cfg.daemon_config_root.clone(),
        capability_id: SEMANTIC_CLONES_CAPABILITY_ID.to_string(),
        mailbox_name:
            crate::capability_packs::semantic_clones::types::SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX
                .to_string(),
        init_session_id: None,
        dedupe_key: Some(
            crate::capability_packs::semantic_clones::workplane::repo_backfill_dedupe_key(
                crate::capability_packs::semantic_clones::types::SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX,
            ),
        ),
        payload: serde_json::to_value(
            crate::capability_packs::semantic_clones::workplane::SemanticClonesMailboxPayload::RepoBackfill {
                work_item_count: Some(2),
                artefact_ids: None,
            },
        )
        .expect("serialize repo backfill payload"),
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

    let repo_backfill_inputs = load_repo_backfill_inputs(&relational, &job)
        .await
        .expect("load repo backfill inputs");
    let current_artefact_ids = current_inputs
        .iter()
        .map(|input| input.artefact_id.clone())
        .collect::<BTreeSet<_>>();
    let repo_backfill_artefact_ids = repo_backfill_inputs
        .iter()
        .map(|input| input.artefact_id.clone())
        .collect::<BTreeSet<_>>();

    assert_eq!(
        repo_backfill_artefact_ids, current_artefact_ids,
        "repo backfill should use the current repo artefacts only"
    );
    assert!(
        repo_backfill_inputs
            .iter()
            .all(|input| input.path != "src/legacy.ts"),
        "repo backfill should exclude historical-only artefacts"
    );
}

#[tokio::test]
async fn workplane_embedding_repo_backfill_job_processes_current_repo_inputs() {
    let (repo, _first_sha, _second_sha) = seed_daemon_embedding_repo();
    let (cfg, _relational, _inputs, _input_hashes) = seed_current_state_and_semantics(
        repo.path(),
        "alpha",
        TEST_EMBEDDINGS_DRIVER,
        "repo-backfill-model",
        "3",
    )
    .await;
    let sqlite_path = daemon_relational_sqlite_path(repo.path());
    let job = WorkplaneJobRecord {
        job_id: "workplane-repo-backfill-job-1".to_string(),
        repo_id: cfg.repo.repo_id.clone(),
        repo_root: cfg.repo_root.clone(),
        config_root: cfg.daemon_config_root.clone(),
        capability_id: SEMANTIC_CLONES_CAPABILITY_ID.to_string(),
        mailbox_name:
            crate::capability_packs::semantic_clones::types::SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX
                .to_string(),
        init_session_id: None,
        dedupe_key: Some(
            crate::capability_packs::semantic_clones::workplane::repo_backfill_dedupe_key(
                crate::capability_packs::semantic_clones::types::SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX,
            ),
        ),
        payload: serde_json::to_value(
            crate::capability_packs::semantic_clones::workplane::SemanticClonesMailboxPayload::RepoBackfill {
                work_item_count: Some(2),
                artefact_ids: None,
            },
        )
        .expect("serialize repo backfill payload"),
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
    assert!(
        rows.len() >= 2,
        "repo backfill embedding job should process the current repo inputs"
    );
    assert_eq!(
        load_clone_edge_count(&sqlite_path, &cfg.repo.repo_id),
        0,
        "workplane embedding backfill should defer clone rebuild to the follow-up job"
    );
}

#[tokio::test]
async fn prepare_embedding_mailbox_batch_with_explicit_repo_backfill_ids_skips_unrelated_current_paths()
 {
    let (repo, _first_sha, _second_sha) = seed_daemon_embedding_repo();
    let (cfg, relational, inputs, _input_hashes) = seed_current_state_and_semantics(
        repo.path(),
        "alpha",
        TEST_EMBEDDINGS_DRIVER,
        "repo-backfill-model",
        "3",
    )
    .await;
    let requested = inputs.first().expect("at least one current semantic input");
    let unrelated = inputs
        .iter()
        .find(|input| input.path != requested.path)
        .expect("fixture should include a second path");

    relational
        .exec(&format!(
            "UPDATE current_file_state \
SET head_content_id = 'missing-explicit-backfill-blob' \
WHERE repo_id = '{}' AND path = '{}'",
            crate::host::devql::esc_pg(&cfg.repo.repo_id),
            crate::host::devql::esc_pg(&unrelated.path),
        ))
        .await
        .expect("break unrelated current projection path");

    super::helpers::load_current_semantic_inputs(&relational, repo.path(), &cfg.repo.repo_id, None)
        .await
        .expect_err("full current hydration should still fail on the broken unrelated path");

    let batch = super::super::workplane::ClaimedEmbeddingMailboxBatch {
        repo_id: cfg.repo.repo_id.clone(),
        repo_root: cfg.repo_root.clone(),
        config_root: cfg.daemon_config_root.clone(),
        representation_kind:
            crate::capability_packs::semantic_clones::embeddings::EmbeddingRepresentationKind::Code,
        lease_token: "explicit-repo-backfill-lease".to_string(),
        items: vec![SemanticEmbeddingMailboxItemRecord {
            item_id: "explicit-repo-backfill-item".to_string(),
            repo_id: cfg.repo.repo_id.clone(),
            repo_root: cfg.repo_root.clone(),
            config_root: cfg.daemon_config_root.clone(),
            init_session_id: None,
            representation_kind:
                crate::capability_packs::semantic_clones::embeddings::EmbeddingRepresentationKind::Code
                    .to_string(),
            item_kind: SemanticMailboxItemKind::RepoBackfill,
            artefact_id: None,
            payload_json: Some(serde_json::json!([requested.artefact_id.clone()])),
            dedupe_key: Some(
                crate::capability_packs::semantic_clones::workplane::repo_backfill_dedupe_key(
                    crate::capability_packs::semantic_clones::types::SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX,
                ),
            ),
            status: SemanticMailboxItemStatus::Leased,
            attempts: 0,
            available_at_unix: 1,
            submitted_at_unix: 1,
            leased_at_unix: Some(1),
            lease_expires_at_unix: Some(301),
            lease_token: Some("explicit-repo-backfill-lease".to_string()),
            updated_at_unix: 1,
            last_error: None,
        }],
    };

    let prepared = prepare_embedding_mailbox_batch(&batch)
        .await
        .expect("explicit repo backfill batch should only hydrate the requested current artefact");

    assert_eq!(prepared.expanded_count, 1);
    assert!(prepared.commit.replacement_backfill_item.is_none());
    assert!(
        !prepared.commit.embedding_statements.is_empty(),
        "requested artefact should still produce embedding work"
    );
}

#[tokio::test]
async fn prepare_embedding_mailbox_batch_splits_repo_wide_backfill_before_hydrating_later_paths() {
    let (repo, _first_sha, _second_sha) = seed_daemon_embedding_repo();
    let generated_dir = repo.path().join("src/generated");
    fs::create_dir_all(&generated_dir).expect("create generated source dir");
    for index in 0..60 {
        fs::write(
            generated_dir.join(format!("worker_{index:02}.ts")),
            format!(
                "export function generatedWorker{index:02}(input: string): string {{\n  return `${{input}}:{index}`;\n}}\n"
            ),
        )
        .expect("write generated source");
    }
    git_ok(repo.path(), &["add", "src/generated"]);
    git_ok(repo.path(), &["commit", "-m", "add generated sources"]);

    let (cfg, relational, inputs, _input_hashes) = seed_current_state_and_semantics(
        repo.path(),
        "alpha",
        TEST_EMBEDDINGS_DRIVER,
        "repo-backfill-model",
        "3",
    )
    .await;
    assert!(
        inputs.len() > super::super::workplane::SEMANTIC_EMBEDDING_MAILBOX_BATCH_SIZE,
        "fixture should exceed one embedding mailbox batch"
    );
    let unrelated = inputs
        .get(super::super::workplane::SEMANTIC_EMBEDDING_MAILBOX_BATCH_SIZE)
        .expect("fixture should include a later path outside the first batch");

    relational
        .exec(&format!(
            "UPDATE current_file_state \
SET head_content_id = 'missing-repo-wide-backfill-blob' \
WHERE repo_id = '{}' AND path = '{}'",
            crate::host::devql::esc_pg(&cfg.repo.repo_id),
            crate::host::devql::esc_pg(&unrelated.path),
        ))
        .await
        .expect("break later current projection path");

    super::helpers::load_current_semantic_inputs(&relational, repo.path(), &cfg.repo.repo_id, None)
        .await
        .expect_err("full current hydration should still fail on the broken later path");

    let batch = super::super::workplane::ClaimedEmbeddingMailboxBatch {
        repo_id: cfg.repo.repo_id.clone(),
        repo_root: cfg.repo_root.clone(),
        config_root: cfg.daemon_config_root.clone(),
        representation_kind:
            crate::capability_packs::semantic_clones::embeddings::EmbeddingRepresentationKind::Code,
        lease_token: "repo-wide-backfill-lease".to_string(),
        items: vec![SemanticEmbeddingMailboxItemRecord {
            item_id: "repo-wide-backfill-item".to_string(),
            repo_id: cfg.repo.repo_id.clone(),
            repo_root: cfg.repo_root.clone(),
            config_root: cfg.daemon_config_root.clone(),
            init_session_id: None,
            representation_kind:
                crate::capability_packs::semantic_clones::embeddings::EmbeddingRepresentationKind::Code
                    .to_string(),
            item_kind: SemanticMailboxItemKind::RepoBackfill,
            artefact_id: None,
            payload_json: None,
            dedupe_key: Some(
                crate::capability_packs::semantic_clones::workplane::repo_backfill_dedupe_key(
                    crate::capability_packs::semantic_clones::types::SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX,
                ),
            ),
            status: SemanticMailboxItemStatus::Leased,
            attempts: 0,
            available_at_unix: 1,
            submitted_at_unix: 1,
            leased_at_unix: Some(1),
            lease_expires_at_unix: Some(301),
            lease_token: Some("repo-wide-backfill-lease".to_string()),
            updated_at_unix: 1,
            last_error: None,
        }],
    };

    let prepared = prepare_embedding_mailbox_batch(&batch)
        .await
        .expect("repo-wide backfill should hydrate only the first chunk");

    assert_eq!(
        prepared.expanded_count,
        super::super::workplane::SEMANTIC_EMBEDDING_MAILBOX_BATCH_SIZE
    );
    assert!(prepared.commit.replacement_backfill_item.is_some());
}

#[tokio::test]
async fn prepare_embedding_mailbox_batch_persists_multiple_rows_from_one_batch() {
    let (repo, _first_sha, _second_sha) = seed_daemon_embedding_repo();
    let (cfg, _relational, inputs, _input_hashes) = seed_current_state_and_semantics(
        repo.path(),
        "alpha",
        TEST_EMBEDDINGS_DRIVER,
        "repo-backfill-model",
        "3",
    )
    .await;
    let requested_ids = inputs
        .iter()
        .take(2)
        .map(|input| input.artefact_id.clone())
        .collect::<Vec<_>>();
    assert_eq!(requested_ids.len(), 2, "fixture should include two inputs");
    let batch = super::super::workplane::ClaimedEmbeddingMailboxBatch {
        repo_id: cfg.repo.repo_id.clone(),
        repo_root: cfg.repo_root.clone(),
        config_root: cfg.daemon_config_root.clone(),
        representation_kind:
            crate::capability_packs::semantic_clones::embeddings::EmbeddingRepresentationKind::Code,
        lease_token: "batch-embedding-lease".to_string(),
        items: vec![SemanticEmbeddingMailboxItemRecord {
            item_id: "batch-embedding-item".to_string(),
            repo_id: cfg.repo.repo_id.clone(),
            repo_root: cfg.repo_root.clone(),
            config_root: cfg.daemon_config_root.clone(),
            init_session_id: None,
            representation_kind:
                crate::capability_packs::semantic_clones::embeddings::EmbeddingRepresentationKind::Code
                    .to_string(),
            item_kind: SemanticMailboxItemKind::RepoBackfill,
            artefact_id: None,
            payload_json: Some(serde_json::json!(requested_ids)),
            dedupe_key: Some(
                crate::capability_packs::semantic_clones::workplane::repo_backfill_dedupe_key(
                    crate::capability_packs::semantic_clones::types::SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX,
                ),
            ),
            status: SemanticMailboxItemStatus::Leased,
            attempts: 0,
            available_at_unix: 1,
            submitted_at_unix: 1,
            leased_at_unix: Some(1),
            lease_expires_at_unix: Some(301),
            lease_token: Some("batch-embedding-lease".to_string()),
            updated_at_unix: 1,
            last_error: None,
        }],
    };

    let prepared = prepare_embedding_mailbox_batch(&batch)
        .await
        .expect("prepare embedding batch");

    let embedding_inserts = prepared
        .commit
        .embedding_statements
        .iter()
        .filter(|sql| sql.contains("symbol_embeddings"))
        .count();
    let non_probe_embedding_requests = fake_embedding_request_lines(repo.path())
        .into_iter()
        .filter(|line| !line.contains("bitloops python embedding dimension probe"))
        .count();
    let prepared_sql = prepared.commit.embedding_statements.join("\n");

    assert!(embedding_inserts >= 2);
    assert_eq!(prepared.expanded_count, 2);
    assert_eq!(non_probe_embedding_requests, 1);
    assert!(
        prepared_sql.contains(
            "CREATE VIRTUAL TABLE IF NOT EXISTS semantic_embedding_current_vec_dim_3 USING vec0"
        ),
        "embedding batches should initialize sqlite-vec current tables for encountered dimensions"
    );
    assert!(
        prepared_sql.contains("vec_f32("),
        "embedding batches should mirror current rows into sqlite-vec tables"
    );
    assert!(
        !prepared_sql.contains("DELETE FROM symbol_embeddings_current"),
        "repo-backfill embedding batches should skip stale current pruning during fresh indexing"
    );
}

#[tokio::test]
async fn repo_backfill_batch_reembeds_when_current_projection_is_missing() {
    let (repo, _first_sha, _second_sha) = seed_daemon_embedding_repo();
    let (cfg, relational, inputs, _input_hashes) = seed_current_state_and_semantics(
        repo.path(),
        "alpha",
        TEST_EMBEDDINGS_DRIVER,
        "repo-backfill-model",
        "3",
    )
    .await;
    let requested_ids = inputs
        .iter()
        .take(2)
        .map(|input| input.artefact_id.clone())
        .collect::<Vec<_>>();
    assert_eq!(requested_ids.len(), 2, "fixture should include two inputs");

    let batch = super::super::workplane::ClaimedEmbeddingMailboxBatch {
        repo_id: cfg.repo.repo_id.clone(),
        repo_root: cfg.repo_root.clone(),
        config_root: cfg.daemon_config_root.clone(),
        representation_kind:
            crate::capability_packs::semantic_clones::embeddings::EmbeddingRepresentationKind::Code,
        lease_token: "repo-backfill-repair-lease".to_string(),
        items: vec![SemanticEmbeddingMailboxItemRecord {
            item_id: "repo-backfill-repair-item".to_string(),
            repo_id: cfg.repo.repo_id.clone(),
            repo_root: cfg.repo_root.clone(),
            config_root: cfg.daemon_config_root.clone(),
            init_session_id: None,
            representation_kind:
                crate::capability_packs::semantic_clones::embeddings::EmbeddingRepresentationKind::Code
                    .to_string(),
            item_kind: SemanticMailboxItemKind::RepoBackfill,
            artefact_id: None,
            payload_json: Some(serde_json::json!(requested_ids)),
            dedupe_key: Some(
                crate::capability_packs::semantic_clones::workplane::repo_backfill_dedupe_key(
                    crate::capability_packs::semantic_clones::types::SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX,
                ),
            ),
            status: SemanticMailboxItemStatus::Leased,
            attempts: 0,
            available_at_unix: 1,
            submitted_at_unix: 1,
            leased_at_unix: Some(1),
            lease_expires_at_unix: Some(301),
            lease_token: Some("repo-backfill-repair-lease".to_string()),
            updated_at_unix: 1,
            last_error: None,
        }],
    };

    let first_prepared = prepare_embedding_mailbox_batch(&batch)
        .await
        .expect("prepare first embedding batch");
    relational
        .exec_serialized_batch_transactional(&first_prepared.commit.embedding_statements)
        .await
        .expect("persist first embedding batch statements");
    relational
        .exec_serialized_batch_transactional(&first_prepared.commit.setup_statements)
        .await
        .expect("persist first embedding batch setup state");
    relational
        .exec(&format!(
            "DELETE FROM symbol_embeddings_current WHERE repo_id = '{}'",
            crate::host::devql::esc_pg(&cfg.repo.repo_id),
        ))
        .await
        .expect("clear current embedding projection only");

    let repaired = prepare_embedding_mailbox_batch(&batch)
        .await
        .expect("prepare second embedding batch after clearing current projection");
    let repaired_sql = repaired.commit.embedding_statements.join("\n");
    let non_probe_embedding_requests = fake_embedding_request_lines(repo.path())
        .into_iter()
        .filter(|line| !line.contains("bitloops python embedding dimension probe"))
        .count();

    assert!(
        repaired_sql.contains("INSERT INTO symbol_embeddings_current"),
        "repo-backfill batches should rebuild current projection rows even when historical embeddings already exist"
    );
    assert_eq!(
        non_probe_embedding_requests, 2,
        "clearing only the current projection should force one additional embedding request"
    );
}

#[tokio::test]
async fn prepare_embedding_mailbox_batch_artefact_batch_keeps_sibling_current_rows_for_same_path() {
    let (repo, _first_sha, _second_sha) = seed_daemon_embedding_repo();
    let (cfg, relational, inputs, input_hashes) = seed_current_state_and_semantics(
        repo.path(),
        "alpha",
        TEST_EMBEDDINGS_DRIVER,
        "path-pruning-model",
        "3",
    )
    .await;
    let selected_path = inputs
        .iter()
        .map(|input| input.path.clone())
        .find(|path| inputs.iter().filter(|input| input.path == *path).count() > 1)
        .expect("fixture should include a path with multiple current artefacts");
    let path_inputs = inputs
        .iter()
        .filter(|input| input.path == selected_path)
        .cloned()
        .collect::<Vec<_>>();
    let requested_ids = path_inputs
        .iter()
        .map(|input| input.artefact_id.clone())
        .collect::<Vec<_>>();
    let selected = path_inputs
        .first()
        .expect("path fixture should include at least one artefact");
    let sqlite_path = daemon_relational_sqlite_path(repo.path());
    let count_current_code_rows_for_path = |path: &str| {
        let conn = rusqlite::Connection::open(&sqlite_path).expect("open sqlite db");
        conn.query_row(
            "SELECT COUNT(*)
             FROM symbol_embeddings_current
             WHERE repo_id = ?1
               AND representation_kind = 'code'
               AND path = ?2",
            rusqlite::params![cfg.repo.repo_id.as_str(), path],
            |row| row.get::<_, i64>(0),
        )
        .expect("count current code rows for path")
    };

    let full_code_job = build_embedding_job_for_representation(
        &cfg,
        requested_ids.clone(),
        requested_ids
            .iter()
            .map(|artefact_id| {
                (
                    artefact_id.clone(),
                    input_hashes
                        .get(artefact_id)
                        .expect("input hash for selected path artefact")
                        .clone(),
                )
            })
            .collect(),
        crate::capability_packs::semantic_clones::embeddings::EmbeddingRepresentationKind::Code,
    );
    let seed_outcome = run_embedding_job_with_env(
        &full_code_job,
        TEST_EMBEDDINGS_DRIVER,
        "path-pruning-model",
        "3",
    )
    .await;
    assert!(
        seed_outcome.error.is_none(),
        "seed code embedding job should succeed"
    );

    let baseline_count = count_current_code_rows_for_path(&selected.path);
    assert_eq!(
        usize::try_from(baseline_count).expect("baseline count should fit into usize"),
        path_inputs.len(),
        "full code embedding seed should populate every current artefact on the selected path"
    );

    let artefact_batch = super::super::workplane::ClaimedEmbeddingMailboxBatch {
        repo_id: cfg.repo.repo_id.clone(),
        repo_root: cfg.repo_root.clone(),
        config_root: cfg.daemon_config_root.clone(),
        representation_kind:
            crate::capability_packs::semantic_clones::embeddings::EmbeddingRepresentationKind::Code,
        lease_token: "path-pruning-artefact-lease".to_string(),
        items: vec![SemanticEmbeddingMailboxItemRecord {
            item_id: "path-pruning-artefact-item".to_string(),
            repo_id: cfg.repo.repo_id.clone(),
            repo_root: cfg.repo_root.clone(),
            config_root: cfg.daemon_config_root.clone(),
            init_session_id: None,
            representation_kind:
                crate::capability_packs::semantic_clones::embeddings::EmbeddingRepresentationKind::Code
                    .to_string(),
            item_kind: SemanticMailboxItemKind::Artefact,
            artefact_id: Some(selected.artefact_id.clone()),
            payload_json: None,
            dedupe_key: Some(format!(
                "{}:{}",
                crate::capability_packs::semantic_clones::types::SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX,
                selected.artefact_id
            )),
            status: SemanticMailboxItemStatus::Leased,
            attempts: 0,
            available_at_unix: 1,
            submitted_at_unix: 1,
            leased_at_unix: Some(1),
            lease_expires_at_unix: Some(301),
            lease_token: Some("path-pruning-artefact-lease".to_string()),
            updated_at_unix: 1,
            last_error: None,
        }],
    };

    let second_prepared = prepare_embedding_mailbox_batch(&artefact_batch)
        .await
        .expect("prepare code artefact embedding batch");
    relational
        .exec_serialized_batch_transactional(&second_prepared.commit.embedding_statements)
        .await
        .expect("persist code artefact embedding statements");

    let after_count = count_current_code_rows_for_path(&selected.path);
    assert_eq!(
        after_count, baseline_count,
        "single-artefact embedding refresh should not delete sibling current rows from the same path"
    );
}

#[tokio::test]
async fn prepare_embedding_mailbox_batch_repairs_current_feature_projection_for_code_embeddings() {
    let (repo, _first_sha, _second_sha) = seed_daemon_embedding_repo();
    let (cfg, relational, inputs, _input_hashes) = seed_current_state_and_semantics(
        repo.path(),
        "alpha",
        TEST_EMBEDDINGS_DRIVER,
        "feature-repair-model",
        "3",
    )
    .await;
    let requested_ids = inputs
        .iter()
        .take(2)
        .map(|input| input.artefact_id.clone())
        .collect::<Vec<_>>();
    assert_eq!(requested_ids.len(), 2, "fixture should include two inputs");

    relational
        .exec(&format!(
            "DELETE FROM symbol_features_current WHERE repo_id = '{}'",
            crate::host::devql::esc_pg(&cfg.repo.repo_id),
        ))
        .await
        .expect("remove current feature projection");

    let batch = super::super::workplane::ClaimedEmbeddingMailboxBatch {
        repo_id: cfg.repo.repo_id.clone(),
        repo_root: cfg.repo_root.clone(),
        config_root: cfg.daemon_config_root.clone(),
        representation_kind:
            crate::capability_packs::semantic_clones::embeddings::EmbeddingRepresentationKind::Code,
        lease_token: "feature-repair-code-embedding-lease".to_string(),
        items: vec![SemanticEmbeddingMailboxItemRecord {
            item_id: "feature-repair-code-embedding-item".to_string(),
            repo_id: cfg.repo.repo_id.clone(),
            repo_root: cfg.repo_root.clone(),
            config_root: cfg.daemon_config_root.clone(),
            init_session_id: None,
            representation_kind:
                crate::capability_packs::semantic_clones::embeddings::EmbeddingRepresentationKind::Code
                    .to_string(),
            item_kind: SemanticMailboxItemKind::RepoBackfill,
            artefact_id: None,
            payload_json: Some(serde_json::json!(requested_ids)),
            dedupe_key: Some(
                crate::capability_packs::semantic_clones::workplane::repo_backfill_dedupe_key(
                    crate::capability_packs::semantic_clones::types::SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX,
                ),
            ),
            status: SemanticMailboxItemStatus::Leased,
            attempts: 0,
            available_at_unix: 1,
            submitted_at_unix: 1,
            leased_at_unix: Some(1),
            lease_expires_at_unix: Some(301),
            lease_token: Some("feature-repair-code-embedding-lease".to_string()),
            updated_at_unix: 1,
            last_error: None,
        }],
    };

    let prepared = prepare_embedding_mailbox_batch(&batch)
        .await
        .expect("prepare embedding batch");

    assert!(
        prepared
            .commit
            .embedding_statements
            .iter()
            .any(|sql| sql.contains("symbol_features_current")),
        "code embedding batches should repair current feature projection before clone rebuild"
    );
}

#[tokio::test]
async fn explicit_embedding_batch_for_one_path_uses_only_selected_ids_in_stale_delete_scope() {
    let (repo, _first_sha, _second_sha) = seed_daemon_embedding_repo();
    let mega_path = repo.path().join("src/mega.ts");
    let mega_source = (0..40)
        .map(|index| {
            format!(
                "export function megaHandler{index:02}(input: string): string {{\n  return `${{input}}:{index}`;\n}}\n"
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    fs::write(&mega_path, mega_source).expect("write large same-path source fixture");
    git_ok(repo.path(), &["add", "src/mega.ts"]);
    git_ok(
        repo.path(),
        &["commit", "-m", "add large same-path source fixture"],
    );

    let (cfg, _relational, inputs, _input_hashes) = seed_current_state_and_semantics(
        repo.path(),
        "alpha",
        TEST_EMBEDDINGS_DRIVER,
        "same-path-model",
        "3",
    )
    .await;
    let mega_inputs = inputs
        .iter()
        .filter(|input| input.path == "src/mega.ts")
        .cloned()
        .collect::<Vec<_>>();
    assert!(
        mega_inputs.len() > super::super::workplane::SEMANTIC_EMBEDDING_MAILBOX_BATCH_SIZE,
        "fixture should include more than one embedding batch for the same path"
    );

    let first_batch_ids = mega_inputs
        .iter()
        .take(super::super::workplane::SEMANTIC_EMBEDDING_MAILBOX_BATCH_SIZE)
        .map(|input| input.artefact_id.clone())
        .collect::<Vec<_>>();
    let second_batch_ids = mega_inputs
        .iter()
        .skip(super::super::workplane::SEMANTIC_EMBEDDING_MAILBOX_BATCH_SIZE)
        .map(|input| input.artefact_id.clone())
        .collect::<Vec<_>>();
    assert!(
        !second_batch_ids.is_empty(),
        "fixture should leave a second batch"
    );
    let later_same_path_id = second_batch_ids
        .first()
        .cloned()
        .expect("fixture should expose a later same-path artefact id");

    let first_batch_items = first_batch_ids
        .iter()
        .enumerate()
        .map(|(index, artefact_id)| SemanticEmbeddingMailboxItemRecord {
            item_id: format!("same-path-batch-item-{index}"),
            repo_id: cfg.repo.repo_id.clone(),
            repo_root: cfg.repo_root.clone(),
            config_root: cfg.daemon_config_root.clone(),
            init_session_id: None,
            representation_kind:
                crate::capability_packs::semantic_clones::embeddings::EmbeddingRepresentationKind::Code
                    .to_string(),
            item_kind: SemanticMailboxItemKind::Artefact,
            artefact_id: Some(artefact_id.clone()),
            payload_json: None,
            dedupe_key: None,
            status: SemanticMailboxItemStatus::Leased,
            attempts: 0,
            available_at_unix: 1,
            submitted_at_unix: 1,
            leased_at_unix: Some(1),
            lease_expires_at_unix: Some(301),
            lease_token: Some("same-path-batch-1".to_string()),
            updated_at_unix: 1,
            last_error: None,
        })
        .collect::<Vec<_>>();
    let first_batch = super::super::workplane::ClaimedEmbeddingMailboxBatch {
        repo_id: cfg.repo.repo_id.clone(),
        repo_root: cfg.repo_root.clone(),
        config_root: cfg.daemon_config_root.clone(),
        representation_kind:
            crate::capability_packs::semantic_clones::embeddings::EmbeddingRepresentationKind::Code,
        lease_token: "same-path-batch-1".to_string(),
        items: first_batch_items,
    };
    let prepared_first = prepare_embedding_mailbox_batch(&first_batch)
        .await
        .expect("prepare first same-path batch");
    let delete_sql = prepared_first
        .commit
        .embedding_statements
        .iter()
        .find(|sql| {
            sql.contains("DELETE FROM symbol_embeddings_current") && sql.contains("src/mega.ts")
        })
        .expect("expected stale delete SQL for the touched same-path batch");

    assert!(
        !delete_sql.contains(&later_same_path_id),
        "selected-batch stale delete scope should not retain artefact ids from later same-path batches"
    );
}

#[tokio::test]
async fn prepare_embedding_mailbox_batch_keeps_current_feature_hash_aligned_with_summary_hash() {
    let (repo, _first_sha, _second_sha) = seed_daemon_embedding_repo();
    let (cfg, _relational, inputs, input_hashes) = seed_current_state_and_semantics(
        repo.path(),
        "alpha",
        TEST_EMBEDDINGS_DRIVER,
        "feature-repair-model",
        "3",
    )
    .await;
    let requested_inputs = inputs.iter().take(2).cloned().collect::<Vec<_>>();
    let requested_ids = requested_inputs
        .iter()
        .map(|input| input.artefact_id.clone())
        .collect::<Vec<_>>();
    assert_eq!(requested_ids.len(), 2, "fixture should include two inputs");

    let batch = super::super::workplane::ClaimedEmbeddingMailboxBatch {
        repo_id: cfg.repo.repo_id.clone(),
        repo_root: cfg.repo_root.clone(),
        config_root: cfg.daemon_config_root.clone(),
        representation_kind:
            crate::capability_packs::semantic_clones::embeddings::EmbeddingRepresentationKind::Code,
        lease_token: "feature-hash-alignment-code-embedding-lease".to_string(),
        items: vec![SemanticEmbeddingMailboxItemRecord {
            item_id: "feature-hash-alignment-code-embedding-item".to_string(),
            repo_id: cfg.repo.repo_id.clone(),
            repo_root: cfg.repo_root.clone(),
            config_root: cfg.daemon_config_root.clone(),
            init_session_id: None,
            representation_kind:
                crate::capability_packs::semantic_clones::embeddings::EmbeddingRepresentationKind::Code
                    .to_string(),
            item_kind: SemanticMailboxItemKind::RepoBackfill,
            artefact_id: None,
            payload_json: Some(serde_json::json!(requested_ids)),
            dedupe_key: Some(
                crate::capability_packs::semantic_clones::workplane::repo_backfill_dedupe_key(
                    crate::capability_packs::semantic_clones::types::SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX,
                ),
            ),
            status: SemanticMailboxItemStatus::Leased,
            attempts: 0,
            available_at_unix: 1,
            submitted_at_unix: 1,
            leased_at_unix: Some(1),
            lease_expires_at_unix: Some(301),
            lease_token: Some("feature-hash-alignment-code-embedding-lease".to_string()),
            updated_at_unix: 1,
            last_error: None,
        }],
    };

    let prepared = prepare_embedding_mailbox_batch(&batch)
        .await
        .expect("prepare embedding batch");

    let repair_sql = prepared.commit.embedding_statements.join("\n");
    assert!(
        repair_sql.contains("symbol_features_current"),
        "expected the embedding batch to repair current feature rows"
    );
    for input in &requested_inputs {
        let expected_hash = input_hashes
            .get(&input.artefact_id)
            .expect("expected summary-backed input hash");
        let noop_hash =
            crate::capability_packs::semantic_clones::features::build_semantic_feature_input_hash(
                input,
                &crate::capability_packs::semantic_clones::features::NoopSemanticSummaryProvider,
            );

        assert!(
            repair_sql.contains(expected_hash),
            "code embedding repairs should preserve the summary-backed feature hash for {}",
            input.artefact_id
        );
        assert!(
            !repair_sql.contains(&noop_hash),
            "code embedding repairs should not stamp the noop summary hash for {}",
            input.artefact_id
        );
    }
}

#[tokio::test]
async fn prepare_summary_mailbox_batch_splits_repo_wide_backfill_before_hydrating_later_paths() {
    let (repo, _first_sha, _second_sha) = seed_daemon_embedding_repo();
    let generated_dir = repo.path().join("src/generated");
    fs::create_dir_all(&generated_dir).expect("create generated source dir");
    for index in 0..24 {
        fs::write(
            generated_dir.join(format!("summary_worker_{index:02}.ts")),
            format!(
                "export function generatedSummaryWorker{index:02}(input: string): string {{\n  return `${{input}}:{index}`;\n}}\n"
            ),
        )
        .expect("write generated source");
    }
    git_ok(repo.path(), &["add", "src/generated"]);
    git_ok(
        repo.path(),
        &["commit", "-m", "add summary backfill sources"],
    );

    let (cfg, relational, inputs, _input_hashes) = seed_current_state_and_semantics(
        repo.path(),
        "alpha",
        TEST_EMBEDDINGS_DRIVER,
        "repo-backfill-model",
        "3",
    )
    .await;
    assert!(
        inputs.len() > super::super::workplane::SEMANTIC_SUMMARY_MAILBOX_BATCH_SIZE,
        "fixture should exceed one summary mailbox batch"
    );
    let unrelated = inputs
        .get(super::super::workplane::SEMANTIC_SUMMARY_MAILBOX_BATCH_SIZE)
        .expect("fixture should include a later path outside the first summary batch");

    relational
        .exec(&format!(
            "UPDATE current_file_state \
SET head_content_id = 'missing-summary-repo-wide-backfill-blob' \
WHERE repo_id = '{}' AND path = '{}'",
            crate::host::devql::esc_pg(&cfg.repo.repo_id),
            crate::host::devql::esc_pg(&unrelated.path),
        ))
        .await
        .expect("break later current projection path");

    super::helpers::load_current_semantic_inputs(&relational, repo.path(), &cfg.repo.repo_id, None)
        .await
        .expect_err("full current hydration should still fail on the broken later path");

    let batch = super::super::workplane::ClaimedSummaryMailboxBatch {
        repo_id: cfg.repo.repo_id.clone(),
        repo_root: cfg.repo_root.clone(),
        config_root: cfg.daemon_config_root.clone(),
        lease_token: "repo-wide-summary-backfill-lease".to_string(),
        items: vec![SemanticSummaryMailboxItemRecord {
            item_id: "repo-wide-summary-backfill-item".to_string(),
            repo_id: cfg.repo.repo_id.clone(),
            repo_root: cfg.repo_root.clone(),
            config_root: cfg.daemon_config_root.clone(),
            init_session_id: None,
            item_kind: SemanticMailboxItemKind::RepoBackfill,
            artefact_id: None,
            payload_json: None,
            dedupe_key: Some(
                crate::capability_packs::semantic_clones::workplane::repo_backfill_dedupe_key(
                    crate::capability_packs::semantic_clones::types::SEMANTIC_CLONES_SUMMARY_REFRESH_MAILBOX,
                ),
            ),
            status: SemanticMailboxItemStatus::Leased,
            attempts: 0,
            available_at_unix: 1,
            submitted_at_unix: 1,
            leased_at_unix: Some(1),
            lease_expires_at_unix: Some(301),
            lease_token: Some("repo-wide-summary-backfill-lease".to_string()),
            updated_at_unix: 1,
            last_error: None,
        }],
    };

    let prepared = prepare_summary_mailbox_batch(&batch, |_, _| {})
        .await
        .expect("repo-wide summary backfill should hydrate only the first chunk");

    assert_eq!(
        prepared.expanded_count,
        super::super::workplane::SEMANTIC_SUMMARY_MAILBOX_BATCH_SIZE
    );
    assert!(prepared.commit.replacement_backfill_item.is_some());
}

#[tokio::test]
async fn prepare_summary_mailbox_batch_with_explicit_repo_backfill_ids_skips_unrelated_current_paths()
 {
    let (repo, _first_sha, _second_sha) = seed_daemon_embedding_repo();
    let (cfg, relational, inputs, _input_hashes) = seed_current_state_and_semantics(
        repo.path(),
        "alpha",
        TEST_EMBEDDINGS_DRIVER,
        "repo-backfill-model",
        "3",
    )
    .await;
    let requested = inputs.first().expect("at least one current semantic input");
    let unrelated = inputs
        .iter()
        .find(|input| input.path != requested.path)
        .expect("fixture should include a second path");

    relational
        .exec(&format!(
            "UPDATE current_file_state \
SET head_content_id = 'missing-explicit-backfill-blob' \
WHERE repo_id = '{}' AND path = '{}'",
            crate::host::devql::esc_pg(&cfg.repo.repo_id),
            crate::host::devql::esc_pg(&unrelated.path),
        ))
        .await
        .expect("break unrelated current projection path");

    super::helpers::load_current_semantic_inputs(&relational, repo.path(), &cfg.repo.repo_id, None)
        .await
        .expect_err("full current hydration should still fail on the broken unrelated path");

    let batch = super::super::workplane::ClaimedSummaryMailboxBatch {
        repo_id: cfg.repo.repo_id.clone(),
        repo_root: cfg.repo_root.clone(),
        config_root: cfg.daemon_config_root.clone(),
        lease_token: "explicit-repo-backfill-summary-lease".to_string(),
        items: vec![SemanticSummaryMailboxItemRecord {
            item_id: "explicit-repo-backfill-summary-item".to_string(),
            repo_id: cfg.repo.repo_id.clone(),
            repo_root: cfg.repo_root.clone(),
            config_root: cfg.daemon_config_root.clone(),
            init_session_id: None,
            item_kind: SemanticMailboxItemKind::RepoBackfill,
            artefact_id: None,
            payload_json: Some(serde_json::json!([requested.artefact_id.clone()])),
            dedupe_key: Some(
                crate::capability_packs::semantic_clones::workplane::repo_backfill_dedupe_key(
                    crate::capability_packs::semantic_clones::types::SEMANTIC_CLONES_SUMMARY_REFRESH_MAILBOX,
                ),
            ),
            status: SemanticMailboxItemStatus::Leased,
            attempts: 0,
            available_at_unix: 1,
            submitted_at_unix: 1,
            leased_at_unix: Some(1),
            lease_expires_at_unix: Some(301),
            lease_token: Some("explicit-repo-backfill-summary-lease".to_string()),
            updated_at_unix: 1,
            last_error: None,
        }],
    };

    let prepared = prepare_summary_mailbox_batch(&batch, |_, _| {})
        .await
        .expect("explicit repo backfill batch should only hydrate the requested current artefact");

    assert_eq!(prepared.expanded_count, 1);
    assert!(prepared.commit.replacement_backfill_item.is_none());
    assert_eq!(
        prepared.commit.acked_item_ids,
        vec!["explicit-repo-backfill-summary-item".to_string()]
    );
}

#[tokio::test]
async fn prepare_summary_mailbox_batch_skipped_fresh_input_without_docstring_summary_repairs_current_without_summary_embedding_follow_up()
 {
    let (repo, _first_sha, _second_sha) = seed_daemon_embedding_repo();
    let (cfg, relational, inputs, _input_hashes) = seed_current_state_and_semantics(
        repo.path(),
        "alpha",
        TEST_EMBEDDINGS_DRIVER,
        "repo-backfill-model",
        "3",
    )
    .await;
    let selected = inputs
        .iter()
        .find(|input| input.path == "src/invoice.ts")
        .expect("invoice artefact input");

    relational
        .exec(&format!(
            "DELETE FROM symbol_semantics_current \
WHERE repo_id = '{}' AND artefact_id = '{}'; \
DELETE FROM symbol_features_current \
WHERE repo_id = '{}' AND artefact_id = '{}'",
            crate::host::devql::esc_pg(&cfg.repo.repo_id),
            crate::host::devql::esc_pg(&selected.artefact_id),
            crate::host::devql::esc_pg(&cfg.repo.repo_id),
            crate::host::devql::esc_pg(&selected.artefact_id),
        ))
        .await
        .expect("remove current semantic projection for selected artefact");

    let batch = super::super::workplane::ClaimedSummaryMailboxBatch {
        repo_id: cfg.repo.repo_id.clone(),
        repo_root: cfg.repo_root.clone(),
        config_root: cfg.daemon_config_root.clone(),
        lease_token: "skipped-fresh-summary-lease".to_string(),
        items: vec![SemanticSummaryMailboxItemRecord {
            item_id: "skipped-fresh-summary-item".to_string(),
            repo_id: cfg.repo.repo_id.clone(),
            repo_root: cfg.repo_root.clone(),
            config_root: cfg.daemon_config_root.clone(),
            init_session_id: None,
            item_kind: SemanticMailboxItemKind::Artefact,
            artefact_id: Some(selected.artefact_id.clone()),
            payload_json: None,
            dedupe_key: Some(format!(
                "{}:{}",
                crate::capability_packs::semantic_clones::types::SEMANTIC_CLONES_SUMMARY_REFRESH_MAILBOX,
                selected.artefact_id
            )),
            status: SemanticMailboxItemStatus::Leased,
            attempts: 0,
            available_at_unix: 1,
            submitted_at_unix: 1,
            leased_at_unix: Some(1),
            lease_expires_at_unix: Some(301),
            lease_token: Some("skipped-fresh-summary-lease".to_string()),
            updated_at_unix: 1,
            last_error: None,
        }],
    };

    let prepared = prepare_summary_mailbox_batch(&batch, |_, _| {})
        .await
        .expect("prepare summary batch");

    assert!(
        prepared
            .commit
            .semantic_statements
            .iter()
            .any(|sql| sql.contains("symbol_features_current")),
        "expected current feature projection repair SQL"
    );
    assert!(
        prepared
            .commit
            .semantic_statements
            .iter()
            .any(|sql| sql.contains("symbol_semantics_current")),
        "expected current semantic projection repair SQL"
    );
    assert!(
        prepared.commit.embedding_follow_ups.is_empty(),
        "summary embeddings should only enqueue when the selected input still persists a summary"
    );
}

#[tokio::test]
async fn prepare_summary_mailbox_batch_skipped_fresh_input_with_code_only_embeddings_has_no_summary_follow_up()
 {
    let (repo, _first_sha, _second_sha) = seed_daemon_embedding_repo();
    let (cfg, _relational, inputs, _input_hashes) = seed_current_state_and_semantics(
        repo.path(),
        "alpha",
        TEST_EMBEDDINGS_DRIVER,
        "repo-backfill-model",
        "3",
    )
    .await;
    remove_summary_embedding_slot(repo.path(), "alpha");
    let selected = inputs
        .iter()
        .find(|input| input.path == "src/invoice.ts")
        .expect("invoice artefact input");
    let batch = super::super::workplane::ClaimedSummaryMailboxBatch {
        repo_id: cfg.repo.repo_id.clone(),
        repo_root: cfg.repo_root.clone(),
        config_root: cfg.daemon_config_root.clone(),
        lease_token: "code-only-summary-lease".to_string(),
        items: vec![SemanticSummaryMailboxItemRecord {
            item_id: "code-only-summary-item".to_string(),
            repo_id: cfg.repo.repo_id.clone(),
            repo_root: cfg.repo_root.clone(),
            config_root: cfg.daemon_config_root.clone(),
            init_session_id: None,
            item_kind: SemanticMailboxItemKind::Artefact,
            artefact_id: Some(selected.artefact_id.clone()),
            payload_json: None,
            dedupe_key: Some(format!(
                "{}:{}",
                crate::capability_packs::semantic_clones::types::SEMANTIC_CLONES_SUMMARY_REFRESH_MAILBOX,
                selected.artefact_id
            )),
            status: SemanticMailboxItemStatus::Leased,
            attempts: 0,
            available_at_unix: 1,
            submitted_at_unix: 1,
            leased_at_unix: Some(1),
            lease_expires_at_unix: Some(301),
            lease_token: Some("code-only-summary-lease".to_string()),
            updated_at_unix: 1,
            last_error: None,
        }],
    };

    let prepared = prepare_summary_mailbox_batch(&batch, |_, _| {})
        .await
        .expect("prepare summary batch");

    assert!(prepared.commit.embedding_follow_ups.is_empty());
}

#[tokio::test]
async fn workplane_clone_rebuild_job_populates_current_clone_edges() {
    let (repo, _first_sha, _second_sha) = seed_daemon_embedding_repo();
    let (cfg, _relational, _inputs, _input_hashes) = seed_current_state_and_semantics(
        repo.path(),
        "alpha",
        TEST_EMBEDDINGS_DRIVER,
        "clone-rebuild-model",
        "3",
    )
    .await;
    let sqlite_path = daemon_relational_sqlite_path(repo.path());
    let embedding_job = WorkplaneJobRecord {
        job_id: "workplane-repo-backfill-job-2".to_string(),
        repo_id: cfg.repo.repo_id.clone(),
        repo_root: cfg.repo_root.clone(),
        config_root: cfg.daemon_config_root.clone(),
        capability_id: SEMANTIC_CLONES_CAPABILITY_ID.to_string(),
        mailbox_name:
            crate::capability_packs::semantic_clones::types::SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX
                .to_string(),
        init_session_id: None,
        dedupe_key: Some(
            crate::capability_packs::semantic_clones::workplane::repo_backfill_dedupe_key(
                crate::capability_packs::semantic_clones::types::SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX,
            ),
        ),
        payload: serde_json::to_value(
            crate::capability_packs::semantic_clones::workplane::SemanticClonesMailboxPayload::RepoBackfill {
                work_item_count: Some(2),
                artefact_ids: None,
            },
        )
        .expect("serialize repo backfill payload"),
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
    let clone_rebuild_job = WorkplaneJobRecord {
        job_id: "workplane-clone-rebuild-job-1".to_string(),
        repo_id: cfg.repo.repo_id.clone(),
        repo_root: cfg.repo_root.clone(),
        config_root: cfg.daemon_config_root.clone(),
        capability_id: SEMANTIC_CLONES_CAPABILITY_ID.to_string(),
        mailbox_name:
            crate::capability_packs::semantic_clones::types::SEMANTIC_CLONES_CLONE_REBUILD_MAILBOX
                .to_string(),
        init_session_id: None,
        dedupe_key: Some(
            crate::capability_packs::semantic_clones::types::SEMANTIC_CLONES_CLONE_REBUILD_MAILBOX
                .to_string(),
        ),
        payload: serde_json::to_value(
            crate::capability_packs::semantic_clones::workplane::SemanticClonesMailboxPayload::RepoBackfill {
                work_item_count: Some(1),
                artefact_ids: None,
            },
        )
        .expect("serialize clone rebuild payload"),
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

    let embedding_outcome = execute_workplane_job(&embedding_job).await;
    assert!(embedding_outcome.error.is_none());
    assert_eq!(
        load_clone_edge_count(&sqlite_path, &cfg.repo.repo_id),
        0,
        "workplane embedding backfill should still defer clone rebuild until the dedicated job runs"
    );

    let rebuild_outcome = execute_workplane_job(&clone_rebuild_job).await;

    assert!(rebuild_outcome.error.is_none());
    assert!(
        load_clone_edge_count(&sqlite_path, &cfg.repo.repo_id) > 0,
        "clone rebuild job should populate current clone edges after deferred embeddings finish"
    );
}

#[tokio::test]
async fn workplane_clone_rebuild_populates_edges_when_feature_projection_was_missing_before_embedding()
 {
    let (repo, _first_sha, _second_sha) = seed_daemon_embedding_repo();
    let (cfg, relational, _inputs, _input_hashes) = seed_current_state_and_semantics(
        repo.path(),
        "alpha",
        TEST_EMBEDDINGS_DRIVER,
        "clone-feature-repair-model",
        "3",
    )
    .await;
    relational
        .exec(&format!(
            "DELETE FROM symbol_features_current WHERE repo_id = '{}'",
            crate::host::devql::esc_pg(&cfg.repo.repo_id),
        ))
        .await
        .expect("remove current feature projection");

    let sqlite_path = daemon_relational_sqlite_path(repo.path());
    let embedding_job = WorkplaneJobRecord {
        job_id: "workplane-code-embedding-feature-repair".to_string(),
        repo_id: cfg.repo.repo_id.clone(),
        repo_root: cfg.repo_root.clone(),
        config_root: cfg.daemon_config_root.clone(),
        capability_id: SEMANTIC_CLONES_CAPABILITY_ID.to_string(),
        mailbox_name:
            crate::capability_packs::semantic_clones::types::SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX
                .to_string(),
        init_session_id: None,
        dedupe_key: Some(
            crate::capability_packs::semantic_clones::workplane::repo_backfill_dedupe_key(
                crate::capability_packs::semantic_clones::types::SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX,
            ),
        ),
        payload: serde_json::to_value(
            crate::capability_packs::semantic_clones::workplane::SemanticClonesMailboxPayload::RepoBackfill {
                work_item_count: Some(2),
                artefact_ids: None,
            },
        )
        .expect("serialize repo backfill payload"),
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
    let clone_rebuild_job = WorkplaneJobRecord {
        job_id: "workplane-clone-feature-repair".to_string(),
        repo_id: cfg.repo.repo_id.clone(),
        repo_root: cfg.repo_root.clone(),
        config_root: cfg.daemon_config_root.clone(),
        capability_id: SEMANTIC_CLONES_CAPABILITY_ID.to_string(),
        mailbox_name:
            crate::capability_packs::semantic_clones::types::SEMANTIC_CLONES_CLONE_REBUILD_MAILBOX
                .to_string(),
        init_session_id: None,
        dedupe_key: Some(
            crate::capability_packs::semantic_clones::types::SEMANTIC_CLONES_CLONE_REBUILD_MAILBOX
                .to_string(),
        ),
        payload: serde_json::to_value(
            crate::capability_packs::semantic_clones::workplane::SemanticClonesMailboxPayload::RepoBackfill {
                work_item_count: Some(1),
                artefact_ids: None,
            },
        )
        .expect("serialize clone rebuild payload"),
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

    let embedding_outcome = execute_workplane_job(&embedding_job).await;
    assert!(embedding_outcome.error.is_none());

    let rebuild_outcome = execute_workplane_job(&clone_rebuild_job).await;
    assert!(rebuild_outcome.error.is_none());
    assert!(
        load_clone_edge_count(&sqlite_path, &cfg.repo.repo_id) > 0,
        "clone rebuild should populate current clone edges after code embeddings repair feature rows"
    );
}

#[tokio::test]
async fn workplane_summary_embedding_mailbox_job_enqueues_clone_rebuild_follow_up() {
    let (repo, _first_sha, _second_sha) = seed_daemon_embedding_repo();
    let (cfg, _relational, inputs, _input_hashes) = seed_current_state_and_semantics(
        repo.path(),
        "alpha",
        TEST_EMBEDDINGS_DRIVER,
        "mailbox-model",
        "3",
    )
    .await;
    let selected = inputs
        .iter()
        .find(|input| input.path == "src/invoice.ts")
        .expect("invoice artefact input");
    let job = WorkplaneJobRecord {
        job_id: "workplane-summary-job-1".to_string(),
        repo_id: cfg.repo.repo_id.clone(),
        repo_root: cfg.repo_root.clone(),
        config_root: cfg.daemon_config_root.clone(),
        capability_id: SEMANTIC_CLONES_CAPABILITY_ID.to_string(),
        mailbox_name:
            crate::capability_packs::semantic_clones::types::SEMANTIC_CLONES_SUMMARY_EMBEDDING_MAILBOX
                .to_string(),
        init_session_id: None,
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

    assert!(outcome.error.is_none());
    assert_eq!(outcome.follow_ups.len(), 1);
    assert!(matches!(
        outcome.follow_ups.first(),
        Some(FollowUpJob::CloneEdgesRebuild { .. })
    ));
}
