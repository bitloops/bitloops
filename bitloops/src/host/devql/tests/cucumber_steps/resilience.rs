use crate::capability_packs::semantic_clones::health::SEMANTIC_CLONES_HEALTH_CHECKS;
use crate::capability_packs::semantic_clones::types::{
    SEMANTIC_CLONES_CAPABILITY_ID, SEMANTIC_CLONES_CLONE_REBUILD_MAILBOX,
    SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX, SEMANTIC_CLONES_SUMMARY_REFRESH_MAILBOX,
};
use crate::cli::devql::graphql::{
    with_graphql_executor_hook_async, with_ingest_daemon_bootstrap_hook_async,
};
use crate::cli::embeddings::{
    EmbeddingsArgs, EmbeddingsClearCacheArgs, EmbeddingsCommand, EmbeddingsPullArgs,
};
use crate::config::{BITLOOPS_CONFIG_RELATIVE_PATH, resolve_embedding_capability_config_for_repo};
use crate::daemon;
use crate::host::capability_host::runtime_contexts::LocalCapabilityRuntimeResources;
use crate::host::devql::cucumber_world::DevqlBddWorld;
use crate::host::devql::{RepoIdentity, deterministic_uuid};
use crate::host::runtime_store::{
    DaemonSqliteRuntimeStore, RepoSqliteRuntimeStore, WorkplaneJobStatus,
};
use crate::test_support::git_fixtures::init_test_repo;
use crate::test_support::process_state::enter_process_state;
use cucumber::{codegen::LocalBoxFuture, step::Collection};
use serde_json::json;
use std::env;
use std::fs;
use std::future::Future;
use std::path::PathBuf;

fn doc_string(ctx: &cucumber::step::Context) -> String {
    ctx.step
        .docstring
        .as_ref()
        .map(ToString::to_string)
        .expect("step docstring should be present")
}

fn table_rows(ctx: &cucumber::step::Context) -> Vec<Vec<String>> {
    ctx.step
        .table
        .as_ref()
        .map(|table| table.rows.clone())
        .expect("step table should be present")
}

fn table_row_maps(ctx: &cucumber::step::Context) -> Vec<std::collections::HashMap<String, String>> {
    let rows = table_rows(ctx);
    let (header, values) = rows
        .split_first()
        .expect("table should include a header row");
    values
        .iter()
        .map(|row| {
            header
                .iter()
                .cloned()
                .zip(row.iter().cloned())
                .collect::<std::collections::HashMap<_, _>>()
        })
        .collect()
}

fn regex(pattern: &str) -> regex::Regex {
    regex::Regex::new(pattern).unwrap_or_else(|err| panic!("invalid step regex `{pattern}`: {err}"))
}

fn step_fn(
    f: for<'a> fn(&'a mut DevqlBddWorld, cucumber::step::Context) -> LocalBoxFuture<'a, ()>,
) -> for<'a> fn(&'a mut DevqlBddWorld, cucumber::step::Context) -> LocalBoxFuture<'a, ()> {
    f
}

fn ensure_scenario_repo(world: &mut DevqlBddWorld) -> PathBuf {
    let repo_root = world.scenario_repo_root();
    if !repo_root.join(".git").exists() {
        init_test_repo(
            &repo_root,
            "main",
            "Bitloops Test",
            "bitloops-test@example.com",
        );
    }
    world.cfg.repo_root = repo_root.clone();
    world.cfg.daemon_config_root = repo_root.clone();
    world.cfg.repo = RepoIdentity {
        provider: "github".to_string(),
        organization: "bitloops".to_string(),
        name: "bdd-repo".to_string(),
        identity: "github/bitloops/bdd-repo".to_string(),
        repo_id: deterministic_uuid("repo://github/bitloops/bdd-repo"),
    };
    repo_root
}

fn scenario_env_overrides(world: &mut DevqlBddWorld) -> (PathBuf, String, String, String) {
    let repo_root = ensure_scenario_repo(world);
    let config_override = world.scenario_config_override_root();
    let state_override = world.scenario_state_override_root();
    let bin_dir = world.scenario_bin_dir();
    let existing_paths = env::var_os("PATH")
        .map(|raw| env::split_paths(&raw).collect::<Vec<_>>())
        .unwrap_or_default();
    let mut paths = vec![bin_dir];
    paths.extend(existing_paths);
    let path_value = env::join_paths(paths)
        .expect("join PATH values")
        .to_string_lossy()
        .to_string();
    (
        repo_root,
        config_override.to_string_lossy().to_string(),
        state_override.to_string_lossy().to_string(),
        path_value,
    )
}

fn with_scenario_process_state<T>(world: &mut DevqlBddWorld, f: impl FnOnce() -> T) -> T {
    let (repo_root, config_override, state_override, path_value) = scenario_env_overrides(world);
    let _guard = enter_process_state(
        Some(&repo_root),
        &[
            (
                "BITLOOPS_TEST_CONFIG_DIR_OVERRIDE",
                Some(config_override.as_str()),
            ),
            (
                "BITLOOPS_TEST_STATE_DIR_OVERRIDE",
                Some(state_override.as_str()),
            ),
            ("BITLOOPS_TEST_ASSUME_DAEMON_RUNNING", Some("1")),
            ("PATH", Some(path_value.as_str())),
        ],
    );
    f()
}

async fn with_scenario_process_state_async<T, Fut>(
    world: &mut DevqlBddWorld,
    f: impl FnOnce() -> Fut,
) -> T
where
    Fut: Future<Output = T>,
{
    let (repo_root, config_override, state_override, path_value) = scenario_env_overrides(world);
    let _guard = enter_process_state(
        Some(&repo_root),
        &[
            (
                "BITLOOPS_TEST_CONFIG_DIR_OVERRIDE",
                Some(config_override.as_str()),
            ),
            (
                "BITLOOPS_TEST_STATE_DIR_OVERRIDE",
                Some(state_override.as_str()),
            ),
            ("BITLOOPS_TEST_ASSUME_DAEMON_RUNNING", Some("1")),
            ("PATH", Some(path_value.as_str())),
        ],
    );
    f().await
}

fn daemon_config_path(world: &mut DevqlBddWorld) -> PathBuf {
    world
        .scenario_config_override_root()
        .join("bitloops")
        .join("config.toml")
}

fn completed_embeddings_bootstrap_task_graphql_json(
    task: &crate::daemon::DevqlTaskRecord,
    message: String,
) -> serde_json::Value {
    let embeddings_bootstrap_spec = task.embeddings_bootstrap_spec().map(|spec| {
        serde_json::json!({
            "configPath": spec.config_path.display().to_string(),
            "profileName": spec.profile_name,
        })
    });

    serde_json::json!({
        "taskId": task.task_id,
        "repoId": task.repo_id,
        "repoName": task.repo_name,
        "repoIdentity": task.repo_identity,
        "kind": task.kind.to_string().to_ascii_uppercase(),
        "source": task.source.to_string(),
        "status": "COMPLETED",
        "submittedAtUnix": task.submitted_at_unix,
        "startedAtUnix": task.started_at_unix.or(Some(task.submitted_at_unix)),
        "updatedAtUnix": task.updated_at_unix.saturating_add(1),
        "completedAtUnix": task.completed_at_unix.or(Some(task.updated_at_unix.saturating_add(1))),
        "queuePosition": serde_json::Value::Null,
        "tasksAhead": serde_json::Value::Null,
        "error": serde_json::Value::Null,
        "syncSpec": serde_json::Value::Null,
        "ingestSpec": serde_json::Value::Null,
        "embeddingsBootstrapSpec": embeddings_bootstrap_spec,
        "syncProgress": serde_json::Value::Null,
        "ingestProgress": serde_json::Value::Null,
        "embeddingsBootstrapProgress": {
            "phase": "complete",
            "assetName": serde_json::Value::Null,
            "bytesDownloaded": 0,
            "bytesTotal": serde_json::Value::Null,
            "version": serde_json::Value::Null,
            "message": "Bootstrap completed"
        },
        "syncResult": serde_json::Value::Null,
        "ingestResult": serde_json::Value::Null,
        "embeddingsBootstrapResult": {
            "version": serde_json::Value::Null,
            "binaryPath": serde_json::Value::Null,
            "cacheDir": serde_json::Value::Null,
            "runtimeName": serde_json::Value::Null,
            "modelName": serde_json::Value::Null,
            "freshlyInstalled": false,
            "message": message
        },
    })
}

fn write_daemon_config(world: &mut DevqlBddWorld, config: &str) {
    let repo_root = ensure_scenario_repo(world);
    let repo_config_path = repo_root.join(BITLOOPS_CONFIG_RELATIVE_PATH);
    let daemon_config_path = daemon_config_path(world);
    if let Some(parent) = repo_config_path.parent() {
        fs::create_dir_all(parent).expect("create repo config parent");
    }
    if let Some(parent) = daemon_config_path.parent() {
        fs::create_dir_all(parent).expect("create daemon config parent");
    }
    fs::write(&repo_config_path, config).expect("write repo-local config");
    fs::write(&daemon_config_path, config).expect("write daemon config");
}

#[cfg(unix)]
fn fake_runtime_command_and_args(world: &mut DevqlBddWorld) -> (String, Vec<String>) {
    use std::os::unix::fs::PermissionsExt;

    let script_path = world.scenario_bin_dir().join("fake-embeddings-runtime.sh");
    let script = r#"#!/bin/sh
model_name="bdd-test-model"
printf '{"event":"ready","protocol":1,"capabilities":["embed","shutdown"]}\n'
while IFS= read -r line; do
  req_id=$(printf '%s\n' "$line" | sed -n 's/.*"id":"\([^"]*\)".*/\1/p')
  case "$line" in
    *'"cmd":"embed"'*)
      printf '{"id":"%s","ok":true,"vectors":[[0.1,0.2,0.3]],"model":"%s"}\n' "$req_id" "$model_name"
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
fn fake_runtime_command_and_args(world: &mut DevqlBddWorld) -> (String, Vec<String>) {
    let script_path = world.scenario_bin_dir().join("fake-embeddings-runtime.ps1");
    let script = r#"
$modelName = "bdd-test-model"
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
        model = $modelName
      }
    }
    "shutdown" {
      $response = @{
        id = $request.id
        ok = $true
        model = $modelName
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

fn config_with_fake_runtime(world: &mut DevqlBddWorld, base: &str) -> String {
    let (command, args) = fake_runtime_command_and_args(world);
    let runtime_args = args
        .iter()
        .map(|arg| format!("{arg:?}"))
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "{base}\n\n[inference.runtimes.bitloops_local_embeddings]\ncommand = {command:?}\nargs = [{runtime_args}]\nstartup_timeout_secs = 5\nrequest_timeout_secs = 5\n"
    )
}

fn given_daemon_config(
    world: &mut DevqlBddWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let config = doc_string(&ctx);
        write_daemon_config(world, config.trim());
    })
}

fn given_daemon_config_using_fake_runtime(
    world: &mut DevqlBddWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let config = config_with_fake_runtime(world, doc_string(&ctx).trim());
        write_daemon_config(world, &config);
    })
}

fn when_semantic_clone_health_checks_run(
    world: &mut DevqlBddWorld,
    _ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        world.health_results.clear();
        let repo_root = ensure_scenario_repo(world);
        let repo = world.cfg.repo.clone();
        let results = with_scenario_process_state(world, || {
            let resources = LocalCapabilityRuntimeResources::new(&repo_root, repo)
                .expect("build local capability runtime resources");
            let ctx = resources.runtime_for_capability(SEMANTIC_CLONES_CAPABILITY_ID, &[]);
            SEMANTIC_CLONES_HEALTH_CHECKS
                .iter()
                .map(|check| (check.name.to_string(), (check.run)(&ctx)))
                .collect::<std::collections::HashMap<_, _>>()
        });
        world.health_results = results;
    })
}

fn when_embeddings_pull_runs_for_profile(
    world: &mut DevqlBddWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        world.operation_error = None;
        world.operation_output.clear();
        let profile = ctx.matches[1].1.clone();
        let result = with_ingest_daemon_bootstrap_hook_async(
            |_repo_root| Ok(()),
            || async {
                with_graphql_executor_hook_async(
                    |repo_root, query, variables| {
                        if query.contains("task(") || query.contains("query Task") {
                            let task_id = variables["id"].as_str().expect("task id");
                            let task = crate::daemon::devql_task(task_id)
                                .expect("load daemon task")
                                .expect("queued daemon task");
                            let capability =
                                resolve_embedding_capability_config_for_repo(repo_root);
                            let lines = crate::cli::embeddings::pull_profile(
                                repo_root,
                                &capability,
                                task.embeddings_bootstrap_spec()
                                    .expect("embeddings bootstrap spec")
                                    .profile_name
                                    .as_str(),
                            )
                            .expect("pull profile");
                            return Ok(serde_json::json!({
                                "task": completed_embeddings_bootstrap_task_graphql_json(
                                    &task,
                                    lines.join("\n"),
                                )
                            }));
                        }

                        panic!("unexpected repo-scoped query: {query}");
                    },
                    || async {
                        with_scenario_process_state_async(world, || async move {
                            crate::cli::embeddings::run_async(EmbeddingsArgs {
                                command: Some(EmbeddingsCommand::Pull(EmbeddingsPullArgs {
                                    profile,
                                })),
                            })
                            .await
                        })
                        .await
                    },
                )
                .await
            },
        )
        .await;
        if let Err(err) = result {
            world.operation_error = Some(format!("{err:#}"));
        }
    })
}

fn when_embeddings_doctor_runs(
    world: &mut DevqlBddWorld,
    _ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        world.operation_error = None;
        world.operation_output.clear();
        let repo_root = ensure_scenario_repo(world);
        let result = with_scenario_process_state(world, || {
            let capability = resolve_embedding_capability_config_for_repo(&repo_root);
            crate::cli::embeddings::doctor_profile(&repo_root, &capability, None)
        });
        match result {
            Ok(lines) => world.operation_output = lines,
            Err(err) => world.operation_error = Some(format!("{err:#}")),
        }
    })
}

fn when_embeddings_clear_cache_runs_for_profile(
    world: &mut DevqlBddWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        world.operation_error = None;
        world.operation_output.clear();
        let profile = ctx.matches[1].1.clone();
        let result = with_scenario_process_state_async(world, || async move {
            crate::cli::embeddings::run_async(EmbeddingsArgs {
                command: Some(EmbeddingsCommand::ClearCache(EmbeddingsClearCacheArgs {
                    profile,
                })),
            })
            .await
        })
        .await;
        if let Err(err) = result {
            world.operation_error = Some(format!("{err:#}"));
        }
    })
}

fn local_cache_dir(world: &mut DevqlBddWorld, profile_name: &str) -> PathBuf {
    let repo_root = ensure_scenario_repo(world);
    let capability = resolve_embedding_capability_config_for_repo(&repo_root);
    let profile = capability
        .inference
        .profiles
        .get(profile_name)
        .unwrap_or_else(|| panic!("missing embedding profile `{profile_name}`"));
    profile
        .cache_dir
        .clone()
        .unwrap_or_else(|| repo_root.join(".bitloops/embeddings/models"))
}

fn given_local_embedding_cache_exists_for_profile(
    world: &mut DevqlBddWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let profile_name = ctx.matches[1].1.clone();
        let cache_dir = local_cache_dir(world, &profile_name);
        fs::create_dir_all(&cache_dir).expect("create local embedding cache dir");
        fs::write(cache_dir.join("sentinel.bin"), "warm").expect("seed local embedding cache");
    })
}

fn given_enrichment_queue_state(
    world: &mut DevqlBddWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let repo_id = world.cfg.repo.repo_id.clone();
        let repo_root = ensure_scenario_repo(world);
        let config = config_with_fake_runtime(
            world,
            r#"[semantic_clones]
summary_mode = "off"
embedding_mode = "deterministic"

[semantic_clones.inference]
code_embeddings = "local"
summary_embeddings = "local"

[inference.profiles.local]
task = "embeddings"
driver = "bitloops_embeddings_ipc"
runtime = "bitloops_local_embeddings"
model = "bdd-test-model""#,
        );
        write_daemon_config(world, &config);
        let daemon_config_root = daemon_config_path(world)
            .parent()
            .expect("daemon config should have parent")
            .to_path_buf();

        let rows = table_row_maps(&ctx);
        with_scenario_process_state(world, || {
            let runtime_store =
                DaemonSqliteRuntimeStore::open().expect("open daemon runtime store");
            runtime_store
                .save_enrichment_queue_state(&crate::daemon::PersistedEnrichmentQueueState {
                    version: 1,
                    paused_semantic: false,
                    paused_embeddings: false,
                    active_branch_by_repo: std::collections::BTreeMap::new(),
                    jobs: Vec::new(),
                    retried_failed_jobs: 0,
                    last_action: Some("seeded".to_string()),
                    paused_reason: None,
                    updated_at_unix: 1,
                })
                .expect("write enrichment control state");

            let repo_store =
                RepoSqliteRuntimeStore::open_for_roots(&daemon_config_root, &repo_root)
                    .expect("open repo workplane store");
            for (index, row) in rows.into_iter().enumerate() {
                let kind = row
                    .get("kind")
                    .expect("kind column should exist")
                    .as_str()
                    .trim();
                let status = WorkplaneJobStatus::parse(
                    row.get("status")
                        .expect("status column should exist")
                        .as_str()
                        .trim(),
                );
                let artefact_id = format!("artefact-{}", index + 1);
                let (mailbox_name, payload, last_error) = match kind {
                    "semantic_summaries" => (
                        SEMANTIC_CLONES_SUMMARY_REFRESH_MAILBOX,
                        json!({ "artefact_id": artefact_id }),
                        None,
                    ),
                    "symbol_embeddings" => (
                        SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX,
                        json!({ "artefact_id": artefact_id }),
                        Some("simulated failure"),
                    ),
                    "clone_edges_rebuild" => (
                        SEMANTIC_CLONES_CLONE_REBUILD_MAILBOX,
                        json!({}),
                        Some("simulated failure"),
                    ),
                    other => panic!("unsupported enrichment job kind `{other}`"),
                };
                let job_id = format!("{mailbox_name}-{index}");
                let dedupe_key = match mailbox_name {
                    SEMANTIC_CLONES_CLONE_REBUILD_MAILBOX => Some(mailbox_name.to_string()),
                    _ => Some(format!("{mailbox_name}:{artefact_id}")),
                };
                let started_at_unix = (status == WorkplaneJobStatus::Running).then_some(1_i64);
                let completed_at_unix = matches!(
                    status,
                    WorkplaneJobStatus::Completed | WorkplaneJobStatus::Failed
                )
                .then_some(1_i64);
                repo_store
                    .connect_repo_sqlite()
                    .expect("connect repo workplane sqlite")
                    .with_connection(|conn| {
                        conn.execute(
                            "INSERT INTO capability_workplane_jobs (
                                 job_id, repo_id, repo_root, config_root, capability_id, mailbox_name,
                                 dedupe_key, payload, status, attempts, available_at_unix, submitted_at_unix,
                                 started_at_unix, updated_at_unix, completed_at_unix, lease_owner,
                                 lease_expires_at_unix, last_error
                             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, NULL, NULL, ?16)",
                            rusqlite::params![
                                job_id,
                                &repo_id,
                                repo_root.to_string_lossy().to_string(),
                                daemon_config_root.to_string_lossy().to_string(),
                                SEMANTIC_CLONES_CAPABILITY_ID,
                                mailbox_name,
                                dedupe_key,
                                payload.to_string(),
                                status.as_str(),
                                1_u32,
                                1_i64,
                                1_i64,
                                started_at_unix,
                                1_i64,
                                completed_at_unix,
                                last_error,
                            ],
                        )
                        .map(|_| ())
                        .map_err(anyhow::Error::from)
                    })
                    .expect("insert seeded workplane job");
            }
        });
    })
}

fn when_enrichment_queue_status_requested(
    world: &mut DevqlBddWorld,
    _ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        world.enrichment_status = Some(with_scenario_process_state(world, || {
            daemon::enrichment_status().expect("read enrichment status")
        }));
    })
}

fn when_enrichment_queue_is_paused(
    world: &mut DevqlBddWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let reason = ctx.matches[1].1.clone();
        world.operation_error = None;
        let result = with_scenario_process_state(world, || daemon::pause_enrichments(Some(reason)));
        match result {
            Ok(_) => {}
            Err(err) => world.operation_error = Some(format!("{err:#}")),
        }
    })
}

fn when_enrichment_queue_is_resumed(
    world: &mut DevqlBddWorld,
    _ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        world.operation_error = None;
        let result = with_scenario_process_state(world, daemon::resume_enrichments);
        match result {
            Ok(_) => {}
            Err(err) => world.operation_error = Some(format!("{err:#}")),
        }
    })
}

fn when_failed_enrichment_jobs_are_retried(
    world: &mut DevqlBddWorld,
    _ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        world.operation_error = None;
        let result = with_scenario_process_state(world, daemon::retry_failed_enrichments);
        match result {
            Ok(_) => {}
            Err(err) => world.operation_error = Some(format!("{err:#}")),
        }
    })
}

fn then_semantic_clone_health_includes(
    world: &mut DevqlBddWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        for row in table_row_maps(&ctx) {
            let check = row.get("check").expect("check column should exist");
            let healthy = row
                .get("healthy")
                .expect("healthy column should exist")
                .eq_ignore_ascii_case("true");
            let message_fragment = row
                .get("message_fragment")
                .expect("message_fragment column should exist");
            let result = world
                .health_results
                .get(check)
                .unwrap_or_else(|| panic!("missing health result for `{check}`"));
            assert_eq!(
                result.healthy, healthy,
                "unexpected health state for `{check}`: {result:?}"
            );
            assert!(
                result.message.contains(message_fragment),
                "expected `{check}` message to contain `{message_fragment}`, got `{}`",
                result.message
            );
        }
    })
}

fn then_last_operation_succeeds(
    world: &mut DevqlBddWorld,
    _ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        assert!(
            world.operation_error.is_none(),
            "expected operation to succeed, got {:?}",
            world.operation_error
        );
    })
}

fn then_last_operation_fails_with_message(
    world: &mut DevqlBddWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let fragment = &ctx.matches[1].1;
        let error = world
            .operation_error
            .as_deref()
            .expect("operation should have failed");
        assert!(
            error.contains(fragment),
            "expected error containing `{fragment}`, got `{error}`"
        );
    })
}

fn then_last_operation_output_includes(
    world: &mut DevqlBddWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let output = world.operation_output.join("\n");
        for row in table_row_maps(&ctx) {
            let fragment = row
                .get("line_fragment")
                .expect("line_fragment column should exist");
            assert!(
                output.contains(fragment),
                "expected output containing `{fragment}`, got `{output}`"
            );
        }
    })
}

fn then_local_embedding_cache_is_absent_for_profile(
    world: &mut DevqlBddWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let profile_name = ctx.matches[1].1.clone();
        let cache_dir = local_cache_dir(world, &profile_name);
        assert!(
            !cache_dir.exists(),
            "expected local embedding cache to be absent at {}",
            cache_dir.display()
        );
    })
}

fn then_enrichment_queue_mode_is(
    world: &mut DevqlBddWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let expected = &ctx.matches[1].1;
        let status = world
            .enrichment_status
            .as_ref()
            .expect("enrichment status should be present");
        assert_eq!(status.state.mode.to_string(), expected.as_str());
    })
}

fn then_enrichment_queue_pause_reason_is(
    world: &mut DevqlBddWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let expected = &ctx.matches[1].1;
        let status = world
            .enrichment_status
            .as_ref()
            .expect("enrichment status should be present");
        assert_eq!(
            status.state.paused_reason.as_deref(),
            Some(expected.as_str()),
            "unexpected pause reason"
        );
    })
}

fn enrichment_metric(status: &crate::daemon::EnrichmentQueueStatus, metric: &str) -> Option<u64> {
    match metric {
        "pending_jobs" => Some(status.state.pending_jobs),
        "pending_semantic_jobs" => Some(status.state.pending_semantic_jobs),
        "pending_embedding_jobs" => Some(status.state.pending_embedding_jobs),
        "pending_clone_edges_rebuild_jobs" => Some(status.state.pending_clone_edges_rebuild_jobs),
        "running_jobs" => Some(status.state.running_jobs),
        "running_semantic_jobs" => Some(status.state.running_semantic_jobs),
        "running_embedding_jobs" => Some(status.state.running_embedding_jobs),
        "running_clone_edges_rebuild_jobs" => Some(status.state.running_clone_edges_rebuild_jobs),
        "failed_jobs" => Some(status.state.failed_jobs),
        "failed_semantic_jobs" => Some(status.state.failed_semantic_jobs),
        "failed_embedding_jobs" => Some(status.state.failed_embedding_jobs),
        "failed_clone_edges_rebuild_jobs" => Some(status.state.failed_clone_edges_rebuild_jobs),
        "retried_failed_jobs" => Some(status.state.retried_failed_jobs),
        _ => None,
    }
}

fn then_enrichment_queue_reports(
    world: &mut DevqlBddWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let status = world
            .enrichment_status
            .as_ref()
            .expect("enrichment status should be present");
        for row in table_row_maps(&ctx) {
            let metric = row.get("metric").expect("metric column should exist");
            let expected: u64 = row
                .get("value")
                .expect("value column should exist")
                .parse()
                .expect("value should be numeric");
            let actual = enrichment_metric(status, metric)
                .unwrap_or_else(|| panic!("unsupported enrichment metric `{metric}`"));
            assert_eq!(actual, expected, "unexpected value for `{metric}`");
        }
    })
}

pub(super) fn register(collection: Collection<DevqlBddWorld>) -> Collection<DevqlBddWorld> {
    collection
        .given(
            None,
            regex(r"^a daemon config:$"),
            step_fn(given_daemon_config),
        )
        .given(
            None,
            regex(r"^a daemon config using the fake embeddings runtime:$"),
            step_fn(given_daemon_config_using_fake_runtime),
        )
        .given(
            None,
            regex(r"^an enrichment queue state with jobs:$"),
            step_fn(given_enrichment_queue_state),
        )
        .given(
            None,
            regex(r#"^the local embedding cache exists for profile "([^"]+)"$"#),
            step_fn(given_local_embedding_cache_exists_for_profile),
        )
        .when(
            None,
            regex(r"^semantic clone health checks run$"),
            step_fn(when_semantic_clone_health_checks_run),
        )
        .when(
            None,
            regex(r#"^bitloops embeddings pull runs for profile "([^"]+)"$"#),
            step_fn(when_embeddings_pull_runs_for_profile),
        )
        .when(
            None,
            regex(r"^bitloops embeddings doctor runs$"),
            step_fn(when_embeddings_doctor_runs),
        )
        .when(
            None,
            regex(r#"^bitloops embeddings clear-cache runs for profile "([^"]+)"$"#),
            step_fn(when_embeddings_clear_cache_runs_for_profile),
        )
        .when(
            None,
            regex(r"^the enrichment queue status is requested$"),
            step_fn(when_enrichment_queue_status_requested),
        )
        .when(
            None,
            regex(r#"^the enrichment queue is paused with reason "([^"]+)"$"#),
            step_fn(when_enrichment_queue_is_paused),
        )
        .when(
            None,
            regex(r"^the enrichment queue is resumed$"),
            step_fn(when_enrichment_queue_is_resumed),
        )
        .when(
            None,
            regex(r"^failed enrichment jobs are retried$"),
            step_fn(when_failed_enrichment_jobs_are_retried),
        )
        .then(
            None,
            regex(r"^semantic clone health includes:$"),
            step_fn(then_semantic_clone_health_includes),
        )
        .then(
            None,
            regex(r"^the last operation succeeds$"),
            step_fn(then_last_operation_succeeds),
        )
        .then(
            None,
            regex(r#"^the last operation fails with message containing "([^"]+)"$"#),
            step_fn(then_last_operation_fails_with_message),
        )
        .then(
            None,
            regex(r"^the last operation output includes:$"),
            step_fn(then_last_operation_output_includes),
        )
        .then(
            None,
            regex(r#"^the local embedding cache is absent for profile "([^"]+)"$"#),
            step_fn(then_local_embedding_cache_is_absent_for_profile),
        )
        .then(
            None,
            regex(r#"^the enrichment queue mode is "([^"]+)"$"#),
            step_fn(then_enrichment_queue_mode_is),
        )
        .then(
            None,
            regex(r#"^the enrichment queue pause reason is "([^"]+)"$"#),
            step_fn(then_enrichment_queue_pause_reason_is),
        )
        .then(
            None,
            regex(r"^the enrichment queue reports:$"),
            step_fn(then_enrichment_queue_reports),
        )
}
