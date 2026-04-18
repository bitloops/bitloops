use super::*;
use crate::host::runtime_store::{
    DaemonSqliteRuntimeStore, RepoSqliteRuntimeStore, WorkplaneJobRecord, WorkplaneJobStatus,
};
use crate::test_support::git_fixtures::init_test_repo;
use serde_json::json;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use tempfile::TempDir;
use tokio::sync::{Mutex, Notify};
use tokio::time::{Duration, advance};

fn sample_input() -> semantic_features::SemanticFeatureInput {
    semantic_features::SemanticFeatureInput {
        artefact_id: "artefact-1".to_string(),
        symbol_id: Some("symbol-1".to_string()),
        repo_id: "repo-1".to_string(),
        blob_sha: "blob-1".to_string(),
        path: "src/service.rs".to_string(),
        language: "rust".to_string(),
        canonical_kind: "function".to_string(),
        language_kind: "function".to_string(),
        symbol_fqn: "src/service.rs::load_user".to_string(),
        name: "load_user".to_string(),
        signature: Some("fn load_user(id: &str)".to_string()),
        modifiers: vec!["pub".to_string()],
        body: "load_user_impl(id)".to_string(),
        docstring: Some("Loads a user.".to_string()),
        parent_kind: None,
        dependency_signals: vec!["calls:user_store::load".to_string()],
        content_hash: Some("content-hash".to_string()),
    }
}

fn sample_input_with_artefact_id(artefact_id: &str) -> semantic_features::SemanticFeatureInput {
    let mut input = sample_input();
    input.artefact_id = artefact_id.to_string();
    input.symbol_id = Some(format!("symbol-{artefact_id}"));
    input.symbol_fqn = format!("src/service.rs::{artefact_id}");
    input.name = artefact_id.to_string();
    input
}

#[test]
fn enrichment_job_kind_serializes_lightweight_artefact_ids() {
    let job = EnrichmentJobKind::SemanticSummaries {
        artefact_ids: vec!["artefact-1".to_string()],
        input_hashes: BTreeMap::from([("artefact-1".to_string(), "hash-1".to_string())]),
        batch_key: "artefact-1".to_string(),
    };

    let value = serde_json::to_value(job).expect("serialize job kind");
    assert_eq!(
        value.get("kind").and_then(|value| value.as_str()),
        Some("semantic_summaries")
    );
    assert_eq!(
        value
            .get("artefact_ids")
            .and_then(|value| value.as_array())
            .map(|values| values.len()),
        Some(1)
    );
    assert!(value.get("inputs").is_none());
}

#[test]
fn enrichment_job_kind_deserializes_legacy_inputs_into_artefact_ids() {
    let input = sample_input();
    let job = serde_json::from_value::<EnrichmentJobKind>(json!({
        "kind": "semantic_summaries",
        "inputs": [input],
        "input_hashes": { "artefact-1": "hash-1" },
        "batch_key": "artefact-1",
        "embedding_mode": "semantic_aware_once"
    }))
    .expect("deserialize legacy job kind");

    match job {
        EnrichmentJobKind::SemanticSummaries { artefact_ids, .. } => {
            assert_eq!(artefact_ids, vec!["artefact-1".to_string()]);
        }
        other => panic!("expected semantic summaries job, got {other:?}"),
    }
}

#[test]
fn load_workplane_jobs_prioritises_embedding_mailboxes_before_summary_refresh() {
    let temp = TempDir::new().expect("temp dir");
    let (coordinator, target, repo_id) = new_test_coordinator(&temp);

    insert_workplane_job(
        &coordinator,
        &target,
        WorkplaneJobFixture {
            repo_id: &repo_id,
            mailbox_name: SEMANTIC_CLONES_SUMMARY_REFRESH_MAILBOX,
            status: WorkplaneJobStatus::Pending,
            artefact_id: Some("summary-a"),
            job_id: "summary-a",
            updated_at_unix: 1,
            attempts: 0,
            last_error: None,
        },
    );
    insert_workplane_job(
        &coordinator,
        &target,
        WorkplaneJobFixture {
            repo_id: &repo_id,
            mailbox_name: SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX,
            status: WorkplaneJobStatus::Pending,
            artefact_id: Some("code-a"),
            job_id: "code-a",
            updated_at_unix: 2,
            attempts: 0,
            last_error: None,
        },
    );
    insert_workplane_job(
        &coordinator,
        &target,
        WorkplaneJobFixture {
            repo_id: &repo_id,
            mailbox_name: SEMANTIC_CLONES_SUMMARY_EMBEDDING_MAILBOX,
            status: WorkplaneJobStatus::Pending,
            artefact_id: Some("summary-embed-a"),
            job_id: "summary-embed-a",
            updated_at_unix: 3,
            attempts: 0,
            last_error: None,
        },
    );
    insert_workplane_job(
        &coordinator,
        &target,
        WorkplaneJobFixture {
            repo_id: &repo_id,
            mailbox_name: SEMANTIC_CLONES_CLONE_REBUILD_MAILBOX,
            status: WorkplaneJobStatus::Pending,
            artefact_id: None,
            job_id: "clone-a",
            updated_at_unix: 4,
            attempts: 0,
            last_error: None,
        },
    );

    let pending_jobs = load_workplane_jobs(&coordinator, WorkplaneJobStatus::Pending);
    let mailboxes = pending_jobs
        .iter()
        .map(|job| job.mailbox_name.as_str())
        .collect::<Vec<_>>();

    assert_eq!(
        mailboxes,
        vec![
            SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX,
            SEMANTIC_CLONES_SUMMARY_EMBEDDING_MAILBOX,
            SEMANTIC_CLONES_SUMMARY_REFRESH_MAILBOX,
            SEMANTIC_CLONES_CLONE_REBUILD_MAILBOX,
        ]
    );
}

fn sample_target(config_root: PathBuf, repo_root: PathBuf) -> EnrichmentJobTarget {
    EnrichmentJobTarget::new(config_root, repo_root)
}

fn new_test_coordinator(temp: &TempDir) -> (EnrichmentCoordinator, EnrichmentJobTarget, String) {
    let config_root = temp.path().join("config");
    let repo_root = temp.path().join("repo");
    fs::create_dir_all(&config_root).expect("create test config root");
    fs::create_dir_all(&repo_root).expect("create test repo root");
    init_test_repo(&repo_root, "main", "Bitloops Test", "bitloops@example.com");
    let repo_store = RepoSqliteRuntimeStore::open_for_roots(&config_root, &repo_root)
        .expect("open repo workplane store");
    let runtime_db_path = repo_store.db_path().to_path_buf();
    let repo_id = repo_store.repo_id().to_string();
    (
        EnrichmentCoordinator {
            runtime_store: DaemonSqliteRuntimeStore::open_at(runtime_db_path.clone())
                .expect("open test daemon runtime store"),
            workplane_store: DaemonSqliteRuntimeStore::open_at(runtime_db_path)
                .expect("open test workplane store"),
            daemon_config_root: config_root.clone(),
            subscription_hub: std::sync::Mutex::new(None),
            lock: Mutex::new(()),
            notify: Notify::new(),
            state_initialised: AtomicBool::new(false),
            maintenance_started: AtomicBool::new(false),
            started_worker_counts: std::sync::Mutex::new(
                super::worker_count::EnrichmentWorkerBudgets::default(),
            ),
        },
        sample_target(config_root, repo_root),
        repo_id,
    )
}

fn configure_summary_refresh_for_repo(target: &EnrichmentJobTarget) {
    let config_path =
        crate::test_support::git_fixtures::write_test_daemon_config(&target.config_root);
    crate::config::settings::write_repo_daemon_binding(
        &target
            .repo_root
            .join(crate::config::REPO_POLICY_LOCAL_FILE_NAME),
        &config_path,
    )
    .expect("bind repo root to daemon config");

    #[cfg(unix)]
    let (command, args) = fake_text_generation_runtime_command_and_args(&target.repo_root);
    #[cfg(windows)]
    let (command, args) = fake_text_generation_runtime_command_and_args(&target.repo_root);
    let runtime_args = args
        .iter()
        .map(|arg| format!("{arg:?}"))
        .collect::<Vec<_>>()
        .join(", ");
    let mut config = fs::read_to_string(&config_path).expect("read test daemon config");
    config.push_str(&format!(
        r#"
[semantic_clones.inference]
summary_generation = "summary_local"

[inference.runtimes.bitloops_inference]
command = {command:?}
args = [{runtime_args}]
startup_timeout_secs = 1
request_timeout_secs = 1

[inference.profiles.summary_local]
task = "text_generation"
driver = "ollama_chat"
runtime = "bitloops_inference"
model = "ministral-3:3b"
base_url = "http://127.0.0.1:11434/api/chat"
temperature = "0.1"
max_output_tokens = 200
"#,
    ));
    fs::write(&config_path, config).expect("write test daemon config with summary profile");
}

fn configure_embeddings_for_repo(target: &EnrichmentJobTarget, profile_name: &str) -> PathBuf {
    let config_path =
        crate::test_support::git_fixtures::write_test_daemon_config(&target.config_root);
    crate::config::settings::write_repo_daemon_binding(
        &target
            .repo_root
            .join(crate::config::REPO_POLICY_LOCAL_FILE_NAME),
        &config_path,
    )
    .expect("bind repo root to daemon config");

    #[cfg(unix)]
    let (command, args) = fake_embeddings_runtime_command_and_args(&target.repo_root);
    #[cfg(windows)]
    let (command, args) = fake_embeddings_runtime_command_and_args(&target.repo_root);
    let runtime_args = args
        .iter()
        .map(|arg| format!("{arg:?}"))
        .collect::<Vec<_>>()
        .join(", ");
    let mut config = fs::read_to_string(&config_path).expect("read test daemon config");
    config.push_str(&format!(
        r#"
[semantic_clones]
embedding_mode = "semantic_aware_once"

[semantic_clones.inference]
code_embeddings = "{profile_name}"
summary_embeddings = "{profile_name}"

[inference.runtimes.bitloops_local_embeddings]
command = {command:?}
args = [{runtime_args}]
startup_timeout_secs = 1
request_timeout_secs = 1

[inference.profiles.{profile_name}]
task = "embeddings"
driver = "bitloops_embeddings_ipc"
runtime = "bitloops_local_embeddings"
model = "local-code"
"#
    ));
    fs::write(&config_path, config).expect("write test daemon config with embeddings profile");
    config_path
}

fn configure_summary_embeddings_only_for_repo(
    target: &EnrichmentJobTarget,
    profile_name: &str,
) -> PathBuf {
    let config_path =
        crate::test_support::git_fixtures::write_test_daemon_config(&target.config_root);
    crate::config::settings::write_repo_daemon_binding(
        &target
            .repo_root
            .join(crate::config::REPO_POLICY_LOCAL_FILE_NAME),
        &config_path,
    )
    .expect("bind repo root to daemon config");

    #[cfg(unix)]
    let (command, args) = fake_embeddings_runtime_command_and_args(&target.repo_root);
    #[cfg(windows)]
    let (command, args) = fake_embeddings_runtime_command_and_args(&target.repo_root);
    let runtime_args = args
        .iter()
        .map(|arg| format!("{arg:?}"))
        .collect::<Vec<_>>()
        .join(", ");
    let mut config = fs::read_to_string(&config_path).expect("read test daemon config");
    config.push_str(&format!(
        r#"
[semantic_clones]
embedding_mode = "semantic_aware_once"

[semantic_clones.inference]
summary_embeddings = "{profile_name}"

[inference.runtimes.bitloops_local_embeddings]
command = {command:?}
args = [{runtime_args}]
startup_timeout_secs = 1
request_timeout_secs = 1

[inference.profiles.{profile_name}]
task = "embeddings"
driver = "bitloops_embeddings_ipc"
runtime = "bitloops_local_embeddings"
model = "local-summary"
"#
    ));
    fs::write(&config_path, config)
        .expect("write test daemon config with summary-only embeddings profile");
    config_path
}

fn configure_remote_embeddings_for_repo(
    target: &EnrichmentJobTarget,
    profile_name: &str,
) -> PathBuf {
    let config_path =
        crate::test_support::git_fixtures::write_test_daemon_config(&target.config_root);
    crate::config::settings::write_repo_daemon_binding(
        &target
            .repo_root
            .join(crate::config::REPO_POLICY_LOCAL_FILE_NAME),
        &config_path,
    )
    .expect("bind repo root to daemon config");

    let mut config = fs::read_to_string(&config_path).expect("read test daemon config");
    config.push_str(&format!(
        r#"
[semantic_clones]
embedding_mode = "semantic_aware_once"

[semantic_clones.inference]
code_embeddings = "{profile_name}"
summary_embeddings = "{profile_name}"

[inference.runtimes.bitloops_platform_embeddings]
command = "platform-embeddings"
args = []
startup_timeout_secs = 60
request_timeout_secs = 300

[inference.profiles.{profile_name}]
task = "embeddings"
driver = "bitloops_embeddings_ipc"
runtime = "bitloops_platform_embeddings"
model = "bge-m3"
"#
    ));
    fs::write(&config_path, config)
        .expect("write test daemon config with remote embeddings profile");
    config_path
}

#[cfg(unix)]
fn fake_text_generation_runtime_command_and_args(repo_root: &Path) -> (String, Vec<String>) {
    use std::os::unix::fs::PermissionsExt;

    let script_path = repo_root.join(".bitloops/test-bin/fake-text-generation-runtime.sh");
    if let Some(parent) = script_path.parent() {
        fs::create_dir_all(parent).expect("create fake text-generation runtime dir");
    }
    fs::write(
        &script_path,
        r#"#!/bin/sh
while IFS= read -r line; do
  request_id=$(printf '%s' "$line" | sed -n 's/.*"request_id":"\([^"]*\)".*/\1/p')
  case "$line" in
    *'"type":"describe"'*)
      printf '{"type":"describe","request_id":"%s","protocol_version":1,"runtime_name":"bitloops-inference","runtime_version":"0.1.0","profile_name":"summary_local","provider":{"kind":"ollama_chat","provider_name":"ollama","model_name":"ministral-3:3b","endpoint":"http://127.0.0.1:11434","capabilities":["text","json_object"]}}\n' "$request_id"
      ;;
    *'"type":"shutdown"'*)
      printf '{"type":"shutdown","request_id":"%s"}\n' "$request_id"
      exit 0
      ;;
    *'"type":"infer"'*)
      printf '{"type":"infer","request_id":"%s","text":"","parsed_json":{"summary":"Summarises the symbol.","confidence":0.91},"provider_name":"ollama","model_name":"ministral-3:3b"}\n' "$request_id"
      ;;
  esac
done
"#,
    )
    .expect("write fake text-generation runtime script");
    let mut permissions = fs::metadata(&script_path)
        .expect("stat fake text-generation runtime script")
        .permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&script_path, permissions)
        .expect("chmod fake text-generation runtime script");
    (
        "/bin/sh".to_string(),
        vec![script_path.to_string_lossy().into_owned()],
    )
}

#[cfg(windows)]
fn fake_text_generation_runtime_command_and_args(repo_root: &Path) -> (String, Vec<String>) {
    let script_path = repo_root.join(".bitloops/test-bin/fake-text-generation-runtime.ps1");
    if let Some(parent) = script_path.parent() {
        fs::create_dir_all(parent).expect("create fake text-generation runtime dir");
    }
    fs::write(
        &script_path,
        r#"
while (($line = [Console]::In.ReadLine()) -ne $null) {
  if ([string]::IsNullOrWhiteSpace($line)) { continue }
  $requestId = [regex]::Match($line, '"request_id":"([^"]+)"').Groups[1].Value
  if ($line -like '*"type":"describe"*') {
    Write-Output '{"type":"describe","request_id":"'"$requestId"'","protocol_version":1,"runtime_name":"bitloops-inference","runtime_version":"0.1.0","profile_name":"summary_local","provider":{"kind":"ollama_chat","provider_name":"ollama","model_name":"ministral-3:3b","endpoint":"http://127.0.0.1:11434","capabilities":["text","json_object"]}}'
  } elseif ($line -like '*"type":"shutdown"*') {
    Write-Output '{"type":"shutdown","request_id":"'"$requestId"'"}'
    exit 0
  } elseif ($line -like '*"type":"infer"*') {
    Write-Output '{"type":"infer","request_id":"'"$requestId"'","text":"","parsed_json":{"summary":"Summarises the symbol.","confidence":0.91},"provider_name":"ollama","model_name":"ministral-3:3b"}'
  }
}
"#,
    )
    .expect("write fake text-generation runtime script");
    (
        "powershell".to_string(),
        vec![
            "-NoProfile".to_string(),
            "-ExecutionPolicy".to_string(),
            "Bypass".to_string(),
            "-File".to_string(),
            script_path.to_string_lossy().into_owned(),
        ],
    )
}

#[cfg(unix)]
fn fake_embeddings_runtime_command_and_args(repo_root: &Path) -> (String, Vec<String>) {
    use std::os::unix::fs::PermissionsExt;

    let script_path = repo_root.join(".bitloops/test-bin/fake-embeddings-runtime.sh");
    if let Some(parent) = script_path.parent() {
        fs::create_dir_all(parent).expect("create fake embeddings runtime dir");
    }
    fs::write(
        &script_path,
        r#"#!/bin/sh
printf '{"event":"ready","protocol":1,"capabilities":["embed","shutdown"]}\n'
while IFS= read -r line; do
  req_id=$(printf '%s\n' "$line" | sed -n 's/.*"id":"\([^"]*\)".*/\1/p')
  case "$line" in
    *'"cmd":"embed"'*)
      printf '{"id":"%s","ok":true,"vectors":[[0.1,0.2,0.3]],"model":"local-code"}\n' "$req_id"
      ;;
    *'"cmd":"shutdown"'*)
      printf '{"id":"%s","ok":true,"model":"local-code"}\n' "$req_id"
      exit 0
      ;;
    *)
      printf '{"id":"%s","ok":false,"error":{"message":"unexpected request"}}\n' "$req_id"
      ;;
  esac
done
"#,
    )
    .expect("write fake embeddings runtime script");
    let mut permissions = fs::metadata(&script_path)
        .expect("stat fake embeddings runtime script")
        .permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&script_path, permissions).expect("chmod fake embeddings runtime script");
    (
        "/bin/sh".to_string(),
        vec![script_path.to_string_lossy().into_owned()],
    )
}

#[cfg(windows)]
fn fake_embeddings_runtime_command_and_args(repo_root: &Path) -> (String, Vec<String>) {
    let script_path = repo_root.join(".bitloops/test-bin/fake-embeddings-runtime.ps1");
    if let Some(parent) = script_path.parent() {
        fs::create_dir_all(parent).expect("create fake embeddings runtime dir");
    }
    fs::write(
        &script_path,
        r#"
$ready = @{ event = "ready"; protocol = 1; capabilities = @("embed", "shutdown") }
$ready | ConvertTo-Json -Compress
while (($line = [Console]::In.ReadLine()) -ne $null) {
  if ([string]::IsNullOrWhiteSpace($line)) { continue }
  $request = $line | ConvertFrom-Json
  switch ($request.cmd) {
    "embed" {
      @{ id = $request.id; ok = $true; vectors = @(@(0.1, 0.2, 0.3)); model = "local-code" } | ConvertTo-Json -Compress
    }
    "shutdown" {
      @{ id = $request.id; ok = $true; model = "local-code" } | ConvertTo-Json -Compress
      exit 0
    }
    default {
      @{ id = $request.id; ok = $false; error = @{ message = "unexpected request" } } | ConvertTo-Json -Compress
    }
  }
}
"#,
    )
    .expect("write fake embeddings runtime script");
    (
        "powershell".to_string(),
        vec![
            "-NoProfile".to_string(),
            "-ExecutionPolicy".to_string(),
            "Bypass".to_string(),
            "-File".to_string(),
            script_path.to_string_lossy().into_owned(),
        ],
    )
}

fn load_workplane_jobs(
    coordinator: &EnrichmentCoordinator,
    status: WorkplaneJobStatus,
) -> Vec<WorkplaneJobRecord> {
    coordinator
        .workplane_store
        .with_connection(|conn| super::load_workplane_jobs_by_status(conn, status))
        .expect("load workplane jobs")
}

#[test]
fn summary_refresh_pool_only_claims_summary_jobs() {
    let temp = TempDir::new().expect("temp dir");
    let (coordinator, target, repo_id) = new_test_coordinator(&temp);
    configure_summary_refresh_for_repo(&target);

    insert_workplane_job(
        &coordinator,
        &target,
        WorkplaneJobFixture {
            repo_id: &repo_id,
            mailbox_name: SEMANTIC_CLONES_SUMMARY_REFRESH_MAILBOX,
            status: WorkplaneJobStatus::Pending,
            artefact_id: Some("summary-a"),
            job_id: "summary-a",
            updated_at_unix: 1,
            attempts: 0,
            last_error: None,
        },
    );
    insert_workplane_job(
        &coordinator,
        &target,
        WorkplaneJobFixture {
            repo_id: &repo_id,
            mailbox_name: SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX,
            status: WorkplaneJobStatus::Pending,
            artefact_id: Some("code-a"),
            job_id: "code-a",
            updated_at_unix: 2,
            attempts: 0,
            last_error: None,
        },
    );
    insert_workplane_job(
        &coordinator,
        &target,
        WorkplaneJobFixture {
            repo_id: &repo_id,
            mailbox_name: SEMANTIC_CLONES_CLONE_REBUILD_MAILBOX,
            status: WorkplaneJobStatus::Pending,
            artefact_id: None,
            job_id: "clone-a",
            updated_at_unix: 3,
            attempts: 0,
            last_error: None,
        },
    );

    let claimed = claim_next_workplane_job(
        &coordinator.workplane_store,
        &coordinator.runtime_store,
        &default_state(),
        super::worker_count::EnrichmentWorkerPool::SummaryRefresh,
    )
    .expect("claim summary refresh job")
    .expect("summary refresh job should be claimable");

    assert_eq!(
        claimed.mailbox_name,
        SEMANTIC_CLONES_SUMMARY_REFRESH_MAILBOX
    );
}

#[test]
fn summary_refresh_pool_skips_paused_jobs() {
    let temp = TempDir::new().expect("temp dir");
    let (coordinator, target, repo_id) = new_test_coordinator(&temp);
    configure_summary_refresh_for_repo(&target);

    insert_workplane_job(
        &coordinator,
        &target,
        WorkplaneJobFixture {
            repo_id: &repo_id,
            mailbox_name: SEMANTIC_CLONES_SUMMARY_REFRESH_MAILBOX,
            status: WorkplaneJobStatus::Pending,
            artefact_id: Some("summary-a"),
            job_id: "summary-a",
            updated_at_unix: 1,
            attempts: 0,
            last_error: None,
        },
    );

    let mut state = default_state();
    state.paused_semantic = true;
    let claimed = claim_next_workplane_job(
        &coordinator.workplane_store,
        &coordinator.runtime_store,
        &state,
        super::worker_count::EnrichmentWorkerPool::SummaryRefresh,
    )
    .expect("attempt paused summary refresh claim");

    assert!(claimed.is_none());
}

#[test]
fn embeddings_pool_only_claims_embedding_jobs() {
    let temp = TempDir::new().expect("temp dir");
    let (coordinator, target, repo_id) = new_test_coordinator(&temp);
    let _config_path = configure_embeddings_for_repo(&target, "local_code");

    insert_workplane_job(
        &coordinator,
        &target,
        WorkplaneJobFixture {
            repo_id: &repo_id,
            mailbox_name: SEMANTIC_CLONES_SUMMARY_REFRESH_MAILBOX,
            status: WorkplaneJobStatus::Pending,
            artefact_id: Some("summary-a"),
            job_id: "summary-a",
            updated_at_unix: 1,
            attempts: 0,
            last_error: None,
        },
    );
    insert_workplane_job(
        &coordinator,
        &target,
        WorkplaneJobFixture {
            repo_id: &repo_id,
            mailbox_name: SEMANTIC_CLONES_SUMMARY_EMBEDDING_MAILBOX,
            status: WorkplaneJobStatus::Pending,
            artefact_id: Some("summary-embed-a"),
            job_id: "summary-embed-a",
            updated_at_unix: 2,
            attempts: 0,
            last_error: None,
        },
    );
    insert_workplane_job(
        &coordinator,
        &target,
        WorkplaneJobFixture {
            repo_id: &repo_id,
            mailbox_name: SEMANTIC_CLONES_CLONE_REBUILD_MAILBOX,
            status: WorkplaneJobStatus::Pending,
            artefact_id: None,
            job_id: "clone-a",
            updated_at_unix: 3,
            attempts: 0,
            last_error: None,
        },
    );

    let claimed = claim_next_workplane_job(
        &coordinator.workplane_store,
        &coordinator.runtime_store,
        &default_state(),
        super::worker_count::EnrichmentWorkerPool::Embeddings,
    )
    .expect("claim embeddings job")
    .expect("embedding job should be claimable");

    assert_eq!(
        claimed.mailbox_name,
        SEMANTIC_CLONES_SUMMARY_EMBEDDING_MAILBOX
    );
}

#[test]
fn embeddings_pool_skips_unready_candidates_with_bounded_query() {
    let temp = TempDir::new().expect("temp dir");
    let (coordinator, target, repo_id) = new_test_coordinator(&temp);
    let _config_path = configure_summary_embeddings_only_for_repo(&target, "summary_only");

    for index in 0..31 {
        let job_id = format!("blocked-code-{index}");
        let artefact_id = format!("code-{index}");
        insert_workplane_job(
            &coordinator,
            &target,
            WorkplaneJobFixture {
                repo_id: &repo_id,
                mailbox_name: SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX,
                status: WorkplaneJobStatus::Pending,
                artefact_id: Some(&artefact_id),
                job_id: &job_id,
                updated_at_unix: (index + 1) as u64,
                attempts: 0,
                last_error: None,
            },
        );
    }
    insert_workplane_job(
        &coordinator,
        &target,
        WorkplaneJobFixture {
            repo_id: &repo_id,
            mailbox_name: SEMANTIC_CLONES_SUMMARY_EMBEDDING_MAILBOX,
            status: WorkplaneJobStatus::Pending,
            artefact_id: Some("summary-ready"),
            job_id: "summary-ready",
            updated_at_unix: 32,
            attempts: 0,
            last_error: None,
        },
    );
    insert_workplane_job(
        &coordinator,
        &target,
        WorkplaneJobFixture {
            repo_id: &repo_id,
            mailbox_name: SEMANTIC_CLONES_SUMMARY_EMBEDDING_MAILBOX,
            status: WorkplaneJobStatus::Pending,
            artefact_id: Some("summary-later"),
            job_id: "summary-later",
            updated_at_unix: 33,
            attempts: 0,
            last_error: None,
        },
    );

    let claimed = claim_next_workplane_job(
        &coordinator.workplane_store,
        &coordinator.runtime_store,
        &default_state(),
        super::worker_count::EnrichmentWorkerPool::Embeddings,
    )
    .expect("claim embeddings job with blocked candidates")
    .expect("summary embedding job should be claimable");

    assert_eq!(
        claimed.mailbox_name,
        SEMANTIC_CLONES_SUMMARY_EMBEDDING_MAILBOX
    );
    assert_eq!(claimed.job_id, "summary-ready");
}

#[test]
fn clone_rebuild_pool_only_claims_clone_rebuild_jobs() {
    let temp = TempDir::new().expect("temp dir");
    let (coordinator, target, repo_id) = new_test_coordinator(&temp);
    configure_summary_refresh_for_repo(&target);

    insert_workplane_job(
        &coordinator,
        &target,
        WorkplaneJobFixture {
            repo_id: &repo_id,
            mailbox_name: SEMANTIC_CLONES_SUMMARY_REFRESH_MAILBOX,
            status: WorkplaneJobStatus::Pending,
            artefact_id: Some("summary-a"),
            job_id: "summary-a",
            updated_at_unix: 1,
            attempts: 0,
            last_error: None,
        },
    );
    insert_workplane_job(
        &coordinator,
        &target,
        WorkplaneJobFixture {
            repo_id: &repo_id,
            mailbox_name: SEMANTIC_CLONES_CLONE_REBUILD_MAILBOX,
            status: WorkplaneJobStatus::Pending,
            artefact_id: None,
            job_id: "clone-a",
            updated_at_unix: 2,
            attempts: 0,
            last_error: None,
        },
    );

    let claimed = claim_next_workplane_job(
        &coordinator.workplane_store,
        &coordinator.runtime_store,
        &default_state(),
        super::worker_count::EnrichmentWorkerPool::CloneRebuild,
    )
    .expect("claim clone rebuild job")
    .expect("clone rebuild job should be claimable");

    assert_eq!(claimed.mailbox_name, SEMANTIC_CLONES_CLONE_REBUILD_MAILBOX);
}

#[test]
fn embeddings_pool_does_not_borrow_summary_or_clone_rebuild_work() {
    let temp = TempDir::new().expect("temp dir");
    let (coordinator, target, repo_id) = new_test_coordinator(&temp);
    configure_summary_refresh_for_repo(&target);

    insert_workplane_job(
        &coordinator,
        &target,
        WorkplaneJobFixture {
            repo_id: &repo_id,
            mailbox_name: SEMANTIC_CLONES_SUMMARY_REFRESH_MAILBOX,
            status: WorkplaneJobStatus::Pending,
            artefact_id: Some("summary-a"),
            job_id: "summary-a",
            updated_at_unix: 1,
            attempts: 0,
            last_error: None,
        },
    );
    insert_workplane_job(
        &coordinator,
        &target,
        WorkplaneJobFixture {
            repo_id: &repo_id,
            mailbox_name: SEMANTIC_CLONES_CLONE_REBUILD_MAILBOX,
            status: WorkplaneJobStatus::Pending,
            artefact_id: None,
            job_id: "clone-a",
            updated_at_unix: 2,
            attempts: 0,
            last_error: None,
        },
    );

    let claimed = claim_next_workplane_job(
        &coordinator.workplane_store,
        &coordinator.runtime_store,
        &default_state(),
        super::worker_count::EnrichmentWorkerPool::Embeddings,
    )
    .expect("attempt embeddings pool claim");

    assert!(claimed.is_none());
}

#[test]
fn summary_refresh_pool_skips_future_available_jobs() {
    let temp = TempDir::new().expect("temp dir");
    let (coordinator, target, repo_id) = new_test_coordinator(&temp);
    configure_summary_refresh_for_repo(&target);
    let now = unix_timestamp_now();

    insert_workplane_job(
        &coordinator,
        &target,
        WorkplaneJobFixture {
            repo_id: &repo_id,
            mailbox_name: SEMANTIC_CLONES_SUMMARY_REFRESH_MAILBOX,
            status: WorkplaneJobStatus::Pending,
            artefact_id: Some("summary-future"),
            job_id: "summary-future",
            updated_at_unix: 1,
            attempts: 0,
            last_error: None,
        },
    );
    set_workplane_job_schedule(&coordinator, "summary-future", now + 300, 1);
    insert_workplane_job(
        &coordinator,
        &target,
        WorkplaneJobFixture {
            repo_id: &repo_id,
            mailbox_name: SEMANTIC_CLONES_SUMMARY_REFRESH_MAILBOX,
            status: WorkplaneJobStatus::Pending,
            artefact_id: Some("summary-ready"),
            job_id: "summary-ready",
            updated_at_unix: 2,
            attempts: 0,
            last_error: None,
        },
    );

    let claimed = claim_next_workplane_job(
        &coordinator.workplane_store,
        &coordinator.runtime_store,
        &default_state(),
        super::worker_count::EnrichmentWorkerPool::SummaryRefresh,
    )
    .expect("claim summary refresh job with future candidate")
    .expect("ready summary refresh job should be claimable");

    assert_eq!(claimed.job_id, "summary-ready");
}

#[test]
fn projected_workplane_status_reports_per_pool_counts() {
    let temp = TempDir::new().expect("temp dir");
    let (coordinator, target, repo_id) = new_test_coordinator(&temp);
    configure_summary_refresh_for_repo(&target);
    let _config_path = configure_embeddings_for_repo(&target, "local_code");

    insert_workplane_job(
        &coordinator,
        &target,
        WorkplaneJobFixture {
            repo_id: &repo_id,
            mailbox_name: SEMANTIC_CLONES_SUMMARY_REFRESH_MAILBOX,
            status: WorkplaneJobStatus::Completed,
            artefact_id: Some("summary-complete"),
            job_id: "summary-complete",
            updated_at_unix: 1,
            attempts: 1,
            last_error: None,
        },
    );
    insert_workplane_job(
        &coordinator,
        &target,
        WorkplaneJobFixture {
            repo_id: &repo_id,
            mailbox_name: SEMANTIC_CLONES_SUMMARY_REFRESH_MAILBOX,
            status: WorkplaneJobStatus::Pending,
            artefact_id: Some("summary-pending"),
            job_id: "summary-pending",
            updated_at_unix: 2,
            attempts: 0,
            last_error: None,
        },
    );
    insert_workplane_job(
        &coordinator,
        &target,
        WorkplaneJobFixture {
            repo_id: &repo_id,
            mailbox_name: SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX,
            status: WorkplaneJobStatus::Running,
            artefact_id: Some("code-running"),
            job_id: "code-running",
            updated_at_unix: 3,
            attempts: 1,
            last_error: None,
        },
    );
    insert_workplane_job(
        &coordinator,
        &target,
        WorkplaneJobFixture {
            repo_id: &repo_id,
            mailbox_name: SEMANTIC_CLONES_CLONE_REBUILD_MAILBOX,
            status: WorkplaneJobStatus::Failed,
            artefact_id: None,
            job_id: "clone-failed",
            updated_at_unix: 4,
            attempts: 2,
            last_error: Some("failed"),
        },
    );

    let projected = project_workplane_status(
        &coordinator.workplane_store,
        &default_state(),
        super::worker_count::EnrichmentWorkerBudgets {
            summary_refresh: 1,
            embeddings: 1,
            clone_rebuild: 1,
        },
    )
    .expect("project workplane status");

    assert_eq!(projected.completed_recent_jobs, 1);
    assert_eq!(projected.worker_pools.len(), 3);
    assert_eq!(
        projected
            .worker_pools
            .iter()
            .find(|pool| pool.kind == crate::daemon::EnrichmentWorkerPoolKind::SummaryRefresh)
            .map(|pool| {
                (
                    pool.pending_jobs,
                    pool.running_jobs,
                    pool.failed_jobs,
                    pool.completed_recent_jobs,
                )
            }),
        Some((1, 0, 0, 1))
    );
    assert_eq!(
        projected
            .worker_pools
            .iter()
            .find(|pool| pool.kind == crate::daemon::EnrichmentWorkerPoolKind::Embeddings)
            .map(|pool| {
                (
                    pool.pending_jobs,
                    pool.running_jobs,
                    pool.failed_jobs,
                    pool.completed_recent_jobs,
                )
            }),
        Some((0, 1, 0, 0))
    );
    assert_eq!(
        projected
            .worker_pools
            .iter()
            .find(|pool| pool.kind == crate::daemon::EnrichmentWorkerPoolKind::CloneRebuild)
            .map(|pool| {
                (
                    pool.pending_jobs,
                    pool.running_jobs,
                    pool.failed_jobs,
                    pool.completed_recent_jobs,
                )
            }),
        Some((0, 0, 1, 0))
    );
}

#[test]
fn effective_worker_budgets_use_remote_embedding_defaults_for_active_config_roots() {
    let temp = TempDir::new().expect("temp dir");
    let (coordinator, target, repo_id) = new_test_coordinator(&temp);
    let _config_path = configure_remote_embeddings_for_repo(&target, "platform_code");

    insert_workplane_job(
        &coordinator,
        &target,
        WorkplaneJobFixture {
            repo_id: &repo_id,
            mailbox_name: SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX,
            status: WorkplaneJobStatus::Pending,
            artefact_id: Some("code-pending"),
            job_id: "code-pending",
            updated_at_unix: 1,
            attempts: 0,
            last_error: None,
        },
    );

    let budgets = effective_worker_budgets(
        &coordinator.workplane_store,
        &coordinator.daemon_config_root,
    )
    .expect("resolve effective worker budgets");

    assert_eq!(budgets.embeddings, 6);
}

struct WorkplaneJobFixture<'a> {
    repo_id: &'a str,
    mailbox_name: &'a str,
    status: WorkplaneJobStatus,
    artefact_id: Option<&'a str>,
    job_id: &'a str,
    updated_at_unix: u64,
    attempts: u32,
    last_error: Option<&'a str>,
}

fn insert_workplane_job(
    coordinator: &EnrichmentCoordinator,
    target: &EnrichmentJobTarget,
    fixture: WorkplaneJobFixture<'_>,
) {
    let dedupe_key = match (fixture.mailbox_name, fixture.artefact_id) {
        (SEMANTIC_CLONES_CLONE_REBUILD_MAILBOX, _) => {
            Some(SEMANTIC_CLONES_CLONE_REBUILD_MAILBOX.to_string())
        }
        (_, Some(artefact_id)) => Some(format!("{}:{artefact_id}", fixture.mailbox_name)),
        _ => None,
    };
    let payload = fixture
        .artefact_id
        .map(|artefact_id| serde_json::json!({ "artefact_id": artefact_id }))
        .unwrap_or_else(|| {
            serde_json::to_value(
                crate::capability_packs::semantic_clones::workplane::SemanticClonesMailboxPayload::RepoBackfill {
                    work_item_count: None,
                    artefact_ids: None,
                },
            )
            .expect("serialize repo backfill test payload")
        });
    let started_at_unix =
        (fixture.status == WorkplaneJobStatus::Running).then_some(fixture.updated_at_unix);
    let completed_at_unix = matches!(
        fixture.status,
        WorkplaneJobStatus::Completed | WorkplaneJobStatus::Failed
    )
    .then_some(fixture.updated_at_unix);
    coordinator
        .workplane_store
        .with_connection(|conn| {
            conn.execute(
                "INSERT INTO capability_workplane_jobs (
                     job_id, repo_id, repo_root, config_root, capability_id, mailbox_name,
                     dedupe_key, payload, status, attempts, available_at_unix, submitted_at_unix,
                     started_at_unix, updated_at_unix, completed_at_unix, lease_owner,
                     lease_expires_at_unix, last_error
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, NULL, NULL, ?16)",
                rusqlite::params![
                    fixture.job_id,
                    fixture.repo_id,
                    target.repo_root.to_string_lossy().to_string(),
                    target.config_root.to_string_lossy().to_string(),
                    SEMANTIC_CLONES_CAPABILITY_ID,
                    fixture.mailbox_name,
                    dedupe_key,
                    payload.to_string(),
                    fixture.status.as_str(),
                    fixture.attempts,
                    sql_i64(fixture.updated_at_unix)?,
                    sql_i64(fixture.updated_at_unix)?,
                    started_at_unix.map(sql_i64).transpose()?,
                    sql_i64(fixture.updated_at_unix)?,
                    completed_at_unix.map(sql_i64).transpose()?,
                    fixture.last_error,
                ],
            )
            .map(|_| ())
            .map_err(anyhow::Error::from)
        })
        .expect("insert workplane job");
}

fn insert_pending_artefact_jobs_bulk(
    coordinator: &EnrichmentCoordinator,
    target: &EnrichmentJobTarget,
    repo_id: &str,
    mailbox_name: &str,
    count: usize,
    submitted_at_unix: u64,
) {
    coordinator
        .workplane_store
        .with_connection(|conn| {
            let tx = conn.unchecked_transaction()?;
            {
                let mut stmt = tx.prepare(
                    "INSERT INTO capability_workplane_jobs (
                         job_id, repo_id, repo_root, config_root, capability_id, mailbox_name,
                         dedupe_key, payload, status, attempts, available_at_unix, submitted_at_unix,
                         started_at_unix, updated_at_unix, completed_at_unix, lease_owner,
                         lease_expires_at_unix, last_error
                    ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 0, ?10, ?11, NULL, ?12, NULL, NULL, NULL, NULL)",
                )?;
                for index in 0..count {
                    let artefact_id = format!("artefact-{index}");
                    stmt.execute(rusqlite::params![
                        format!("bulk-job-{mailbox_name}-{index}"),
                        repo_id,
                        target.repo_root.to_string_lossy().to_string(),
                        target.config_root.to_string_lossy().to_string(),
                        SEMANTIC_CLONES_CAPABILITY_ID,
                        mailbox_name,
                        format!("{mailbox_name}:{artefact_id}"),
                        serde_json::to_string(
                            &crate::capability_packs::semantic_clones::workplane::SemanticClonesMailboxPayload::Artefact { artefact_id }
                        )
                        .expect("serialize bulk artefact payload"),
                        WorkplaneJobStatus::Pending.as_str(),
                        sql_i64(submitted_at_unix)?,
                        sql_i64(submitted_at_unix)?,
                        sql_i64(submitted_at_unix)?,
                    ])?;
                }
            }
            tx.commit()?;
            Ok::<_, anyhow::Error>(())
        })
        .expect("insert bulk workplane jobs");
}

fn set_workplane_job_schedule(
    coordinator: &EnrichmentCoordinator,
    job_id: &str,
    available_at_unix: u64,
    submitted_at_unix: u64,
) {
    coordinator
        .workplane_store
        .with_connection(|conn| {
            conn.execute(
                "UPDATE capability_workplane_jobs
                 SET available_at_unix = ?1,
                     submitted_at_unix = ?2,
                     updated_at_unix = ?3
                 WHERE job_id = ?4",
                rusqlite::params![
                    sql_i64(available_at_unix)?,
                    sql_i64(submitted_at_unix)?,
                    sql_i64(submitted_at_unix)?,
                    job_id,
                ],
            )
            .map(|_| ())
            .map_err(anyhow::Error::from)
        })
        .expect("update workplane job schedule");
}

#[tokio::test]
async fn enqueue_clone_edges_rebuild_waits_for_embedding_and_semantic_jobs_to_drain() {
    let temp = TempDir::new().expect("temp dir");
    let (coordinator, target, repo_id) = new_test_coordinator(&temp);

    insert_workplane_job(
        &coordinator,
        &target,
        WorkplaneJobFixture {
            repo_id: &repo_id,
            mailbox_name: SEMANTIC_CLONES_SUMMARY_REFRESH_MAILBOX,
            status: WorkplaneJobStatus::Pending,
            artefact_id: Some("artefact-semantic-a"),
            job_id: "semantic-a",
            updated_at_unix: 1,
            attempts: 0,
            last_error: None,
        },
    );
    insert_workplane_job(
        &coordinator,
        &target,
        WorkplaneJobFixture {
            repo_id: &repo_id,
            mailbox_name: SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX,
            status: WorkplaneJobStatus::Pending,
            artefact_id: Some("artefact-embedding-a"),
            job_id: "embedding-a",
            updated_at_unix: 1,
            attempts: 0,
            last_error: None,
        },
    );
    insert_workplane_job(
        &coordinator,
        &target,
        WorkplaneJobFixture {
            repo_id: &repo_id,
            mailbox_name: SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX,
            status: WorkplaneJobStatus::Running,
            artefact_id: Some("artefact-embedding-b"),
            job_id: "embedding-b",
            updated_at_unix: 1,
            attempts: 1,
            last_error: None,
        },
    );

    coordinator
        .enqueue_clone_edges_rebuild(target.clone())
        .await
        .expect("enqueue coalesced clone rebuild request");

    let enqueued_state = load_workplane_jobs(&coordinator, WorkplaneJobStatus::Pending);
    assert_eq!(
        enqueued_state
            .iter()
            .filter(|job| job.mailbox_name == SEMANTIC_CLONES_CLONE_REBUILD_MAILBOX)
            .count(),
        1
    );

    coordinator
        .enqueue_clone_edges_rebuild(target)
        .await
        .expect("dedupe clone rebuild jobs");

    let deduped_state = load_workplane_jobs(&coordinator, WorkplaneJobStatus::Pending);
    assert_eq!(
        deduped_state
            .iter()
            .filter(|job| job.mailbox_name == SEMANTIC_CLONES_CLONE_REBUILD_MAILBOX)
            .count(),
        1
    );
}

#[tokio::test]
async fn enqueue_symbol_embeddings_splits_large_batches_into_smaller_jobs() {
    let temp = TempDir::new().expect("temp dir");
    let (coordinator, target, _repo_id) = new_test_coordinator(&temp);
    let inputs = (0..(MAX_SEMANTIC_ENRICHMENT_JOB_ARTEFACTS + 1))
        .map(|index| sample_input_with_artefact_id(&format!("artefact-{index}")))
        .collect::<Vec<_>>();
    let input_count = inputs.len();
    let input_hashes = inputs
        .iter()
        .map(|input| {
            (
                input.artefact_id.clone(),
                format!("hash-{}", input.artefact_id),
            )
        })
        .collect::<BTreeMap<_, _>>();

    coordinator
        .enqueue_symbol_embeddings(
            target,
            inputs,
            input_hashes,
            EmbeddingRepresentationKind::Code,
        )
        .await
        .expect("enqueue embedding jobs");

    let embedding_jobs = load_workplane_jobs(&coordinator, WorkplaneJobStatus::Pending)
        .iter()
        .filter(|job| job.mailbox_name == SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX)
        .map(|job| {
            job.payload["artefact_id"]
                .as_str()
                .unwrap_or_default()
                .to_string()
        })
        .collect::<Vec<_>>();

    assert_eq!(embedding_jobs.len(), input_count);
    assert!(
        embedding_jobs
            .iter()
            .all(|artefact_id| !artefact_id.is_empty())
    );
}

#[tokio::test]
async fn enqueue_semantic_summaries_keeps_larger_semantic_batches() {
    let temp = TempDir::new().expect("temp dir");
    let (coordinator, target, _repo_id) = new_test_coordinator(&temp);
    configure_summary_refresh_for_repo(&target);
    let inputs = (0..(MAX_SEMANTIC_ENRICHMENT_JOB_ARTEFACTS + 1))
        .map(|index| sample_input_with_artefact_id(&format!("artefact-{index}")))
        .collect::<Vec<_>>();
    let input_hashes = inputs
        .iter()
        .map(|input| {
            (
                input.artefact_id.clone(),
                format!("hash-{}", input.artefact_id),
            )
        })
        .collect::<BTreeMap<_, _>>();

    coordinator
        .enqueue_semantic_summaries(target, inputs, input_hashes)
        .await
        .expect("enqueue semantic jobs");

    let semantic_jobs = load_workplane_jobs(&coordinator, WorkplaneJobStatus::Pending)
        .iter()
        .filter(|job| job.mailbox_name == SEMANTIC_CLONES_SUMMARY_REFRESH_MAILBOX)
        .map(|job| {
            job.payload["artefact_id"]
                .as_str()
                .unwrap_or_default()
                .to_string()
        })
        .collect::<Vec<_>>();

    assert_eq!(
        semantic_jobs.len(),
        MAX_SEMANTIC_ENRICHMENT_JOB_ARTEFACTS + 1
    );
    assert!(
        semantic_jobs
            .iter()
            .all(|artefact_id| !artefact_id.is_empty())
    );
}

#[test]
fn requeue_running_jobs_moves_stale_running_jobs_back_to_pending() {
    let temp = TempDir::new().expect("temp dir");
    let (coordinator, target, repo_id) = new_test_coordinator(&temp);

    coordinator
        .runtime_store
        .save_enrichment_queue_state(&default_state())
        .expect("write initial control state");
    insert_workplane_job(
        &coordinator,
        &target,
        WorkplaneJobFixture {
            repo_id: &repo_id,
            mailbox_name: SEMANTIC_CLONES_SUMMARY_REFRESH_MAILBOX,
            status: WorkplaneJobStatus::Running,
            artefact_id: Some("artefact-semantic-a"),
            job_id: "semantic-a",
            updated_at_unix: 1,
            attempts: 1,
            last_error: None,
        },
    );
    insert_workplane_job(
        &coordinator,
        &target,
        WorkplaneJobFixture {
            repo_id: &repo_id,
            mailbox_name: SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX,
            status: WorkplaneJobStatus::Running,
            artefact_id: Some("artefact-embedding-a"),
            job_id: "embedding-a",
            updated_at_unix: 1,
            attempts: 1,
            last_error: None,
        },
    );
    insert_workplane_job(
        &coordinator,
        &target,
        WorkplaneJobFixture {
            repo_id: &repo_id,
            mailbox_name: SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX,
            status: WorkplaneJobStatus::Pending,
            artefact_id: Some("artefact-embedding-b"),
            job_id: "embedding-b",
            updated_at_unix: 1,
            attempts: 0,
            last_error: None,
        },
    );

    coordinator.requeue_running_jobs();

    let recovered_running = load_workplane_jobs(&coordinator, WorkplaneJobStatus::Running);
    let recovered_pending = load_workplane_jobs(&coordinator, WorkplaneJobStatus::Pending);
    assert_eq!(recovered_running.len(), 0);
    assert_eq!(recovered_pending.len(), 3);
    assert_eq!(
        coordinator
            .runtime_store
            .load_enrichment_queue_state()
            .expect("read recovered control state")
            .expect("state exists")
            .last_action
            .as_deref(),
        Some("requeue_running")
    );
}

#[test]
fn ensure_started_recovers_stale_running_jobs_on_startup() {
    let temp = TempDir::new().expect("temp dir");
    let (coordinator, target, repo_id) = new_test_coordinator(&temp);
    let coordinator = Arc::new(coordinator);
    configure_summary_refresh_for_repo(&target);

    coordinator
        .runtime_store
        .save_enrichment_queue_state(&default_state())
        .expect("write initial control state");
    insert_workplane_job(
        &coordinator,
        &target,
        WorkplaneJobFixture {
            repo_id: &repo_id,
            mailbox_name: SEMANTIC_CLONES_SUMMARY_REFRESH_MAILBOX,
            status: WorkplaneJobStatus::Running,
            artefact_id: Some("artefact-semantic-a"),
            job_id: "semantic-a",
            updated_at_unix: 1,
            attempts: 1,
            last_error: None,
        },
    );
    insert_workplane_job(
        &coordinator,
        &target,
        WorkplaneJobFixture {
            repo_id: &repo_id,
            mailbox_name: SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX,
            status: WorkplaneJobStatus::Running,
            artefact_id: Some("artefact-embedding-a"),
            job_id: "embedding-a",
            updated_at_unix: 1,
            attempts: 1,
            last_error: None,
        },
    );
    insert_workplane_job(
        &coordinator,
        &target,
        WorkplaneJobFixture {
            repo_id: &repo_id,
            mailbox_name: SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX,
            status: WorkplaneJobStatus::Pending,
            artefact_id: Some("artefact-embedding-b"),
            job_id: "embedding-b",
            updated_at_unix: 1,
            attempts: 0,
            last_error: None,
        },
    );

    coordinator.ensure_started();

    let recovered_running = load_workplane_jobs(&coordinator, WorkplaneJobStatus::Running);
    let recovered_pending = load_workplane_jobs(&coordinator, WorkplaneJobStatus::Pending);
    assert_eq!(recovered_running.len(), 0);
    assert_eq!(recovered_pending.len(), 3);
    assert_eq!(
        coordinator
            .runtime_store
            .load_enrichment_queue_state()
            .expect("read recovered control state")
            .expect("state exists")
            .last_action
            .as_deref(),
        Some("requeue_running")
    );
}

#[test]
fn snapshot_projects_last_failed_embedding_job_details() {
    let temp = TempDir::new().expect("temp dir");
    let (coordinator, target, repo_id) = new_test_coordinator(&temp);
    insert_workplane_job(
        &coordinator,
        &target,
        WorkplaneJobFixture {
            repo_id: &repo_id,
            mailbox_name: SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX,
            status: WorkplaneJobStatus::Failed,
            artefact_id: Some("artefact-older"),
            job_id: "embedding-older",
            updated_at_unix: 10,
            attempts: 1,
            last_error: Some("older failure"),
        },
    );
    insert_workplane_job(
        &coordinator,
        &target,
        WorkplaneJobFixture {
            repo_id: &repo_id,
            mailbox_name: SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX,
            status: WorkplaneJobStatus::Failed,
            artefact_id: Some("artefact-newer"),
            job_id: "embedding-newer",
            updated_at_unix: 20,
            attempts: 3,
            last_error: Some("[capability_host:timeout] capability ingester timed out after 300s"),
        },
    );

    let summary = super::last_failed_embedding_job_from_workplane(&coordinator.workplane_store)
        .expect("read failed embedding summary")
        .expect("failed embedding summary");
    assert_eq!(summary.job_id, "embedding-newer");
    assert_eq!(summary.repo_id, repo_id);
    assert_eq!(summary.branch, "unknown");
    assert_eq!(summary.representation_kind, "code");
    assert_eq!(summary.artefact_count, 1);
    assert_eq!(summary.attempts, 3);
    assert_eq!(
        summary.error.as_deref(),
        Some("[capability_host:timeout] capability ingester timed out after 300s")
    );
    assert_eq!(summary.updated_at_unix, 20);
}

#[test]
fn compaction_replaces_large_old_pending_embedding_backlog_with_repo_backfill_job() {
    let temp = TempDir::new().expect("temp dir");
    let (coordinator, target, repo_id) = new_test_coordinator(&temp);
    let pending_count = usize::try_from(WORKPLANE_PENDING_COMPACTION_MIN_COUNT)
        .expect("pending compaction threshold fits usize");
    insert_pending_artefact_jobs_bulk(
        &coordinator,
        &target,
        &repo_id,
        SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX,
        pending_count,
        1,
    );

    super::compact_and_prune_workplane_jobs(&coordinator.workplane_store)
        .expect("compact pending workplane backlog");

    let pending_jobs = load_workplane_jobs(&coordinator, WorkplaneJobStatus::Pending)
        .into_iter()
        .filter(|job| job.mailbox_name == SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX)
        .collect::<Vec<_>>();
    let expected_dedupe_key =
        crate::capability_packs::semantic_clones::workplane::repo_backfill_dedupe_key(
            SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX,
        );

    assert_eq!(
        pending_jobs.len(),
        1,
        "artefact backlog should compact to a single repo backfill job"
    );
    assert_eq!(
        pending_jobs[0].dedupe_key.as_deref(),
        Some(expected_dedupe_key.as_str())
    );
    assert!(
        crate::capability_packs::semantic_clones::workplane::payload_is_repo_backfill(
            &pending_jobs[0].payload
        ),
        "pending job should be converted to a repo backfill payload"
    );
    assert_eq!(
        crate::capability_packs::semantic_clones::workplane::payload_work_item_count(
            &pending_jobs[0].payload,
            pending_jobs[0].mailbox_name.as_str(),
        ),
        u64::try_from(pending_count).expect("pending count fits u64"),
        "compacted repo backfill job should retain the exact artefact workload size",
    );
}

#[test]
fn compaction_prunes_pending_summary_refresh_jobs_when_summary_provider_is_unconfigured() {
    let temp = TempDir::new().expect("temp dir");
    let (coordinator, target, repo_id) = new_test_coordinator(&temp);
    let config_path =
        crate::test_support::git_fixtures::write_test_daemon_config(&target.config_root);
    crate::config::settings::write_repo_daemon_binding(
        &target
            .repo_root
            .join(crate::config::REPO_POLICY_LOCAL_FILE_NAME),
        &config_path,
    )
    .expect("bind repo root to config");
    crate::capability_packs::semantic_clones::workplane::activate_deferred_pipeline_mailboxes(
        &target.repo_root,
        "init",
    )
    .expect("activate deferred mailboxes");

    insert_workplane_job(
        &coordinator,
        &target,
        WorkplaneJobFixture {
            repo_id: &repo_id,
            mailbox_name: SEMANTIC_CLONES_SUMMARY_REFRESH_MAILBOX,
            status: WorkplaneJobStatus::Pending,
            artefact_id: Some("artefact-semantic-a"),
            job_id: "semantic-a",
            updated_at_unix: 1,
            attempts: 0,
            last_error: None,
        },
    );
    insert_workplane_job(
        &coordinator,
        &target,
        WorkplaneJobFixture {
            repo_id: &repo_id,
            mailbox_name: SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX,
            status: WorkplaneJobStatus::Pending,
            artefact_id: Some("artefact-embedding-a"),
            job_id: "embedding-a",
            updated_at_unix: 1,
            attempts: 0,
            last_error: None,
        },
    );

    super::compact_and_prune_workplane_jobs(&coordinator.workplane_store)
        .expect("prune inactive summary refresh jobs");

    let pending_jobs = load_workplane_jobs(&coordinator, WorkplaneJobStatus::Pending);
    assert_eq!(
        pending_jobs
            .iter()
            .filter(|job| job.mailbox_name == SEMANTIC_CLONES_SUMMARY_REFRESH_MAILBOX)
            .count(),
        0,
        "summary refresh jobs should be dropped when no summary provider is configured"
    );
    assert_eq!(
        pending_jobs
            .iter()
            .filter(|job| job.mailbox_name == SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX)
            .count(),
        1,
        "other pending work should be preserved"
    );
}

#[tokio::test]
async fn enqueue_does_not_compact_large_pending_backlog() {
    let temp = TempDir::new().expect("temp dir");
    let (coordinator, target, repo_id) = new_test_coordinator(&temp);
    let pending_count = usize::try_from(WORKPLANE_PENDING_COMPACTION_MIN_COUNT)
        .expect("pending compaction threshold fits usize");
    insert_pending_artefact_jobs_bulk(
        &coordinator,
        &target,
        &repo_id,
        SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX,
        pending_count,
        1,
    );

    coordinator
        .enqueue_clone_edges_rebuild(target.clone())
        .await
        .expect("enqueue clone rebuild job");

    let pending_jobs = load_workplane_jobs(&coordinator, WorkplaneJobStatus::Pending);
    let embedding_jobs = pending_jobs
        .iter()
        .filter(|job| job.mailbox_name == SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX)
        .collect::<Vec<_>>();

    assert_eq!(embedding_jobs.len(), pending_count);
    assert!(
        embedding_jobs.iter().all(|job| {
            !crate::capability_packs::semantic_clones::workplane::payload_is_repo_backfill(
                &job.payload,
            )
        }),
        "enqueue should not compact a pending artefact backlog on the hot path",
    );
    assert_eq!(
        pending_jobs
            .iter()
            .filter(|job| job.mailbox_name == SEMANTIC_CLONES_CLONE_REBUILD_MAILBOX)
            .count(),
        1,
    );
}

#[test]
fn claim_does_not_compact_large_pending_backlog() {
    let temp = TempDir::new().expect("temp dir");
    let (coordinator, target, repo_id) = new_test_coordinator(&temp);
    configure_summary_refresh_for_repo(&target);
    let pending_count = usize::try_from(WORKPLANE_PENDING_COMPACTION_MIN_COUNT)
        .expect("pending compaction threshold fits usize");
    insert_pending_artefact_jobs_bulk(
        &coordinator,
        &target,
        &repo_id,
        SEMANTIC_CLONES_SUMMARY_REFRESH_MAILBOX,
        pending_count,
        1,
    );

    let claimed = claim_next_workplane_job(
        &coordinator.workplane_store,
        &coordinator.runtime_store,
        &default_state(),
        super::worker_count::EnrichmentWorkerPool::SummaryRefresh,
    )
    .expect("claim summary refresh job")
    .expect("summary refresh job should be claimable");

    assert_eq!(
        claimed.mailbox_name,
        SEMANTIC_CLONES_SUMMARY_REFRESH_MAILBOX
    );
    let pending_jobs = load_workplane_jobs(&coordinator, WorkplaneJobStatus::Pending)
        .into_iter()
        .filter(|job| job.mailbox_name == SEMANTIC_CLONES_SUMMARY_REFRESH_MAILBOX)
        .collect::<Vec<_>>();
    assert_eq!(pending_jobs.len(), pending_count - 1);
    assert!(
        pending_jobs.iter().all(|job| {
            !crate::capability_packs::semantic_clones::workplane::payload_is_repo_backfill(
                &job.payload,
            )
        }),
        "claiming should not compact a pending artefact backlog on the hot path",
    );
}

#[test]
fn ensure_started_compacts_large_pending_backlog_on_startup() {
    let temp = TempDir::new().expect("temp dir");
    let (coordinator, target, repo_id) = new_test_coordinator(&temp);
    let coordinator = Arc::new(coordinator);
    let pending_count = usize::try_from(WORKPLANE_PENDING_COMPACTION_MIN_COUNT)
        .expect("pending compaction threshold fits usize");
    insert_pending_artefact_jobs_bulk(
        &coordinator,
        &target,
        &repo_id,
        SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX,
        pending_count,
        1,
    );

    coordinator.ensure_started();

    let pending_jobs = load_workplane_jobs(&coordinator, WorkplaneJobStatus::Pending)
        .into_iter()
        .filter(|job| job.mailbox_name == SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX)
        .collect::<Vec<_>>();
    assert_eq!(pending_jobs.len(), 1);
    assert!(
        crate::capability_packs::semantic_clones::workplane::payload_is_repo_backfill(
            &pending_jobs[0].payload
        ),
        "startup maintenance should compact an old artefact backlog into a repo-backfill job",
    );
}

#[test]
fn retry_failed_jobs_runs_maintenance() {
    let temp = TempDir::new().expect("temp dir");
    let (coordinator, target, repo_id) = new_test_coordinator(&temp);
    let pending_count = usize::try_from(WORKPLANE_PENDING_COMPACTION_MIN_COUNT)
        .expect("pending compaction threshold fits usize");
    insert_pending_artefact_jobs_bulk(
        &coordinator,
        &target,
        &repo_id,
        SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX,
        pending_count,
        1,
    );
    insert_workplane_job(
        &coordinator,
        &target,
        WorkplaneJobFixture {
            repo_id: &repo_id,
            mailbox_name: SEMANTIC_CLONES_CLONE_REBUILD_MAILBOX,
            status: WorkplaneJobStatus::Failed,
            artefact_id: None,
            job_id: "clone-failed",
            updated_at_unix: 10,
            attempts: 2,
            last_error: Some("boom"),
        },
    );

    let retried =
        super::retry_failed_jobs_in_store(&coordinator.workplane_store).expect("retry failed jobs");

    assert_eq!(retried, 1);
    let pending_jobs = load_workplane_jobs(&coordinator, WorkplaneJobStatus::Pending);
    assert_eq!(
        pending_jobs
            .iter()
            .filter(|job| job.mailbox_name == SEMANTIC_CLONES_CLONE_REBUILD_MAILBOX)
            .count(),
        1,
    );
    let embedding_jobs = pending_jobs
        .iter()
        .filter(|job| job.mailbox_name == SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX)
        .collect::<Vec<_>>();
    assert_eq!(embedding_jobs.len(), 1);
    assert!(
        crate::capability_packs::semantic_clones::workplane::payload_is_repo_backfill(
            &embedding_jobs[0].payload
        ),
        "explicit retry should still run maintenance after requeueing failed jobs",
    );
}

#[tokio::test(start_paused = true, flavor = "current_thread")]
async fn periodic_maintenance_runs_on_the_sixty_second_tick() {
    let temp = TempDir::new().expect("temp dir");
    let (coordinator, target, repo_id) = new_test_coordinator(&temp);
    if let Ok(mut counts) = coordinator.started_worker_counts.lock() {
        *counts = super::worker_count::EnrichmentWorkerBudgets {
            summary_refresh: 32,
            embeddings: 32,
            clone_rebuild: 32,
        };
    }
    let coordinator = Arc::new(coordinator);
    let pending_count = usize::try_from(WORKPLANE_PENDING_COMPACTION_MIN_COUNT)
        .expect("pending compaction threshold fits usize");

    coordinator.ensure_started();
    tokio::task::yield_now().await;

    insert_pending_artefact_jobs_bulk(
        &coordinator,
        &target,
        &repo_id,
        SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX,
        pending_count,
        1,
    );

    let load_embedding_jobs = || {
        load_workplane_jobs(&coordinator, WorkplaneJobStatus::Pending)
            .into_iter()
            .filter(|job| job.mailbox_name == SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX)
            .collect::<Vec<_>>()
    };

    assert_eq!(load_embedding_jobs().len(), pending_count);

    advance(Duration::from_secs(59)).await;
    tokio::task::yield_now().await;
    assert_eq!(load_embedding_jobs().len(), pending_count);

    advance(Duration::from_secs(1)).await;
    tokio::task::yield_now().await;
    tokio::task::yield_now().await;

    let embedding_jobs = load_embedding_jobs();
    assert_eq!(embedding_jobs.len(), 1);
    assert!(
        crate::capability_packs::semantic_clones::workplane::payload_is_repo_backfill(
            &embedding_jobs[0].payload
        ),
        "periodic maintenance should compact the backlog on the scheduled tick",
    );
}

#[test]
fn retry_failed_jobs_requeues_historical_repo_backfill_payloads() {
    let temp = TempDir::new().expect("temp dir");
    let (coordinator, target, repo_id) = new_test_coordinator(&temp);
    let artefact_ids = (0..40)
        .map(|index| format!("artefact-{index}"))
        .collect::<Vec<_>>();
    let payload = serde_json::to_string(
        &crate::capability_packs::semantic_clones::workplane::SemanticClonesMailboxPayload::RepoBackfill {
            work_item_count: Some(artefact_ids.len() as u64),
            artefact_ids: Some(artefact_ids.clone()),
        },
    )
    .expect("serialize repo backfill payload");
    let dedupe_key = crate::capability_packs::semantic_clones::workplane::repo_backfill_dedupe_key(
        SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX,
    );
    coordinator
        .workplane_store
        .with_connection(|conn| {
            conn.execute(
                "INSERT INTO capability_workplane_jobs (
                     job_id, repo_id, repo_root, config_root, capability_id, mailbox_name,
                     dedupe_key, payload, status, attempts, available_at_unix, submitted_at_unix,
                     started_at_unix, updated_at_unix, completed_at_unix, lease_owner,
                     lease_expires_at_unix, last_error
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, NULL, NULL, ?16)",
                rusqlite::params![
                    "failed-backfill",
                    repo_id,
                    target.repo_root.to_string_lossy().to_string(),
                    target.config_root.to_string_lossy().to_string(),
                    SEMANTIC_CLONES_CAPABILITY_ID,
                    SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX,
                    dedupe_key,
                    payload,
                    WorkplaneJobStatus::Failed.as_str(),
                    2u32,
                    sql_i64(1)?,
                    sql_i64(1)?,
                    sql_i64(1)?,
                    sql_i64(1)?,
                    sql_i64(1)?,
                    "timeout",
                ],
            )
            .map(|_| ())
            .map_err(anyhow::Error::from)
        })
        .expect("insert failed repo backfill job");

    let retried =
        super::retry_failed_jobs_in_store(&coordinator.workplane_store).expect("retry failed jobs");

    assert_eq!(retried, 1);
    let pending_jobs = load_workplane_jobs(&coordinator, WorkplaneJobStatus::Pending)
        .into_iter()
        .filter(|job| job.mailbox_name == SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX)
        .collect::<Vec<_>>();
    assert_eq!(pending_jobs.len(), 1);
    assert!(
        crate::capability_packs::semantic_clones::workplane::payload_is_repo_backfill(
            &pending_jobs[0].payload
        )
    );
    assert_eq!(
        crate::capability_packs::semantic_clones::workplane::payload_work_item_count(
            &pending_jobs[0].payload,
            pending_jobs[0].mailbox_name.as_str(),
        ),
        40,
    );
    let requeued_artefact_ids =
        crate::capability_packs::semantic_clones::workplane::payload_repo_backfill_artefact_ids(
            &pending_jobs[0].payload,
        )
        .expect("explicit artefact ids should be preserved for retried repo backfill jobs");
    assert_eq!(requeued_artefact_ids.len(), 40);
    assert_eq!(
        requeued_artefact_ids.first().map(String::as_str),
        Some("artefact-0")
    );
    assert_eq!(
        requeued_artefact_ids.last().map(String::as_str),
        Some("artefact-39")
    );
}

#[test]
fn transient_embedding_timeout_is_requeued_with_backoff() {
    let temp = TempDir::new().expect("temp dir");
    let (coordinator, target, repo_id) = new_test_coordinator(&temp);

    insert_workplane_job(
        &coordinator,
        &target,
        WorkplaneJobFixture {
            repo_id: &repo_id,
            mailbox_name: SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX,
            status: WorkplaneJobStatus::Running,
            artefact_id: Some("code-timeout"),
            job_id: "code-timeout",
            updated_at_unix: 10,
            attempts: 1,
            last_error: None,
        },
    );
    let running_job = load_workplane_jobs(&coordinator, WorkplaneJobStatus::Running)
        .into_iter()
        .find(|job| job.job_id == "code-timeout")
        .expect("running embedding job");

    let before = unix_timestamp_now();
    let outcome = JobExecutionOutcome::failed(anyhow::anyhow!(
        "[capability_host:timeout] capability ingester timed out after 300s"
    ));
    let disposition = super::workplane::persist_workplane_job_completion(
        &coordinator.workplane_store,
        &running_job,
        &outcome,
    )
    .expect("persist retryable timeout");

    match disposition {
        super::workplane::WorkplaneJobCompletionDisposition::RetryScheduled {
            available_at_unix,
            retry_in_secs,
        } => {
            assert_eq!(retry_in_secs, 5);
            assert!(
                available_at_unix >= before + retry_in_secs,
                "retry should be scheduled in the future"
            );
        }
        other => panic!("expected retry disposition, got {other:?}"),
    }

    let pending_job = load_workplane_jobs(&coordinator, WorkplaneJobStatus::Pending)
        .into_iter()
        .find(|job| job.job_id == "code-timeout")
        .expect("requeued embedding job");
    assert_eq!(pending_job.last_error.as_deref(), outcome.error.as_deref());
    assert!(pending_job.started_at_unix.is_none());
    assert!(pending_job.completed_at_unix.is_none());
}

#[test]
fn embedding_timeout_at_retry_limit_stays_failed() {
    let temp = TempDir::new().expect("temp dir");
    let (coordinator, target, repo_id) = new_test_coordinator(&temp);

    insert_workplane_job(
        &coordinator,
        &target,
        WorkplaneJobFixture {
            repo_id: &repo_id,
            mailbox_name: SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX,
            status: WorkplaneJobStatus::Running,
            artefact_id: Some("code-timeout-terminal"),
            job_id: "code-timeout-terminal",
            updated_at_unix: 10,
            attempts: 3,
            last_error: None,
        },
    );
    let running_job = load_workplane_jobs(&coordinator, WorkplaneJobStatus::Running)
        .into_iter()
        .find(|job| job.job_id == "code-timeout-terminal")
        .expect("running embedding job");

    let outcome = JobExecutionOutcome::failed(anyhow::anyhow!(
        "[capability_host:timeout] capability ingester timed out after 300s"
    ));
    let disposition = super::workplane::persist_workplane_job_completion(
        &coordinator.workplane_store,
        &running_job,
        &outcome,
    )
    .expect("persist terminal timeout");

    assert_eq!(
        disposition,
        super::workplane::WorkplaneJobCompletionDisposition::Failed
    );
    let failed_job = load_workplane_jobs(&coordinator, WorkplaneJobStatus::Failed)
        .into_iter()
        .find(|job| job.job_id == "code-timeout-terminal")
        .expect("failed embedding job");
    assert_eq!(failed_job.last_error.as_deref(), outcome.error.as_deref());
    assert!(failed_job.completed_at_unix.is_some());
}

#[test]
fn workplane_completion_log_emits_queue_wait_and_run_durations() {
    let job = WorkplaneJobRecord {
        job_id: "job-1".to_string(),
        repo_id: "repo-1".to_string(),
        repo_root: PathBuf::from("/tmp/repo-1"),
        config_root: PathBuf::from("/tmp/config-1"),
        capability_id: SEMANTIC_CLONES_CAPABILITY_ID.to_string(),
        mailbox_name: SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX.to_string(),
        init_session_id: None,
        dedupe_key: Some("semantic_clones.embedding.code:artefact-1".to_string()),
        payload: serde_json::to_value(
            crate::capability_packs::semantic_clones::workplane::SemanticClonesMailboxPayload::RepoBackfill {
                work_item_count: Some(4),
                artefact_ids: Some(vec![
                    "artefact-1".to_string(),
                    "artefact-2".to_string(),
                    "artefact-3".to_string(),
                    "artefact-4".to_string(),
                ]),
            },
        )
        .expect("serialize repo backfill payload"),
        status: WorkplaneJobStatus::Running,
        attempts: 3,
        available_at_unix: 5,
        submitted_at_unix: 10,
        started_at_unix: Some(25),
        updated_at_unix: 25,
        completed_at_unix: None,
        lease_owner: None,
        lease_expires_at_unix: None,
        last_error: None,
    };

    let log_line =
        super::workplane::format_workplane_job_completion_log(&job, 40, &JobExecutionOutcome::ok());

    assert!(log_line.contains("mailbox_name=semantic_clones.embedding.code"));
    assert!(log_line.contains("payload_work_item_count=4"));
    assert!(log_line.contains("queue_wait_secs=15"));
    assert!(log_line.contains("run_secs=15"));
    assert!(log_line.contains("attempts=3"));
    assert!(log_line.contains("outcome=completed"));
}
