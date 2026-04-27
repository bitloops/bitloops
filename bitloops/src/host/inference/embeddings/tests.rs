use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::Path;
use std::time::Duration;

use tempfile::TempDir;

use crate::config::{InferenceConfig, InferenceRuntimeConfig};

use super::super::{
    EmbeddingInputType, EmbeddingService, EmptyInferenceGateway, InferenceGateway,
    LocalInferenceGateway,
};
use super::auth::with_platform_runtime_auth_environment_hook;
use super::service::BitloopsEmbeddingsIpcService;
use super::shared::evict_idle_embeddings_sessions_for_tests;

fn write_fake_runtime_script(
    script_path: &Path,
    timeout_marker: Option<&Path>,
    env_log: Option<&Path>,
) {
    let timeout_branch = timeout_marker
        .map(|path| {
            format!(
                r#"
      if [ ! -f "{path}" ]; then
        : > "{path}"
        sleep 2
      fi
"#,
                path = path.display()
            )
        })
        .unwrap_or_default();
    let env_log_branch = env_log
        .map(|path| {
            format!(
                r#"printf 'HF_HUB_OFFLINE=%s\n' "${{HF_HUB_OFFLINE:-}}" > "{path}"
printf 'TRANSFORMERS_OFFLINE=%s\n' "${{TRANSFORMERS_OFFLINE:-}}" >> "{path}"
printf 'BITLOOPS_PLATFORM_GATEWAY_TOKEN=%s\n' "${{BITLOOPS_PLATFORM_GATEWAY_TOKEN:-}}" >> "{path}"
printf 'BITLOOPS_CUSTOM_PLATFORM_TOKEN=%s\n' "${{BITLOOPS_CUSTOM_PLATFORM_TOKEN:-}}" >> "{path}"
"#,
                path = path.display()
            )
        })
        .unwrap_or_default();
    fs::write(
        script_path,
        format!(
            r#"launch_log="$1"
shift
printf '%s\n' "$$" >> "$launch_log"
{env_log_branch}\
printf '%s\n' '{{"event":"ready","protocol":1,"capabilities":["embed","shutdown"]}}'

while IFS= read -r line; do
  request_id=$(printf '%s' "$line" | sed -n 's/.*"id":"\([^"]*\)".*/\1/p')
  case "$line" in
    *'"cmd":"shutdown"'*)
      printf '{{"id":"%s","ok":true}}\n' "$request_id"
      exit 0
      ;;
    *'"cmd":"embed"'*)
      case "$line" in
        *'bitloops python embedding dimension probe'*)
          printf '{{"id":"%s","ok":true,"vectors":[[1.0,2.0]]}}\n' "$request_id"
          ;;
        *'second document'*)
          printf '{{"id":"%s","ok":true,"vectors":[[1.0,2.0],[3.0,4.0]]}}\n' "$request_id"
          ;;
        *)
{timeout_branch}          printf '{{"id":"%s","ok":true,"vectors":[[1.0,2.0]]}}\n' "$request_id"
          ;;
      esac
      ;;
  esac
done
"#,
            env_log_branch = env_log_branch,
            timeout_branch = timeout_branch,
        ),
    )
    .expect("write fake runtime script");
}

fn fake_runtime_config(script_path: &Path, launch_log: &Path) -> InferenceRuntimeConfig {
    InferenceRuntimeConfig {
        command: "/bin/sh".to_string(),
        args: vec![
            script_path.to_string_lossy().into_owned(),
            launch_log.to_string_lossy().into_owned(),
        ],
        startup_timeout_secs: 1,
        request_timeout_secs: 1,
    }
}

#[test]
fn empty_gateway_rejects_unknown_slots() {
    let gateway = EmptyInferenceGateway;
    let err = match gateway.embeddings("code_embeddings") {
        Ok(_) => panic!("missing slot must fail"),
        Err(err) => err,
    };

    assert!(
        err.to_string().contains("code_embeddings"),
        "unexpected error: {err}"
    );
}

#[test]
fn scoped_gateway_reports_bound_slots() {
    let gateway = LocalInferenceGateway::new(
        Path::new("/repo"),
        InferenceConfig::default(),
        HashMap::from([(
            "semantic_clones".to_string(),
            BTreeMap::from([("code_embeddings".to_string(), "local".to_string())]),
        )]),
    );
    let scoped = gateway.scoped(Some("semantic_clones"));

    assert!(scoped.has_slot("code_embeddings"));
    assert!(!scoped.has_slot("summary_embeddings"));
    let description = scoped
        .describe("code_embeddings")
        .expect("slot description");
    assert_eq!(description.profile_name, "local");
}

#[test]
fn platform_ipc_service_requires_authenticated_session() {
    let temp = TempDir::new().expect("temp dir");
    let script_path = temp.path().join("fake_embeddings_runtime.sh");
    let launch_log = temp.path().join("launches.log");
    write_fake_runtime_script(&script_path, None, None);

    let mut runtime = fake_runtime_config(&script_path, &launch_log);
    runtime.args.push("--api-key-env".to_string());
    runtime
        .args
        .push("BITLOOPS_CUSTOM_PLATFORM_TOKEN".to_string());
    let err = with_platform_runtime_auth_environment_hook(
        |_| Ok(Vec::new()),
        || match BitloopsEmbeddingsIpcService::new(
            "platform_code",
            &runtime,
            "test-model",
            None,
            true,
        ) {
            Ok(_) => panic!("platform embeddings service without auth must fail"),
            Err(err) => err,
        },
    );

    assert!(
        format!("{err:#}").contains("requires an authenticated Bitloops session"),
        "unexpected error: {err:#}"
    );
    assert!(
        !launch_log.exists(),
        "platform embeddings runtime should not be spawned without an auth token"
    );
}

#[test]
fn platform_ipc_service_injects_logged_in_token_into_requested_env_var() {
    let temp = TempDir::new().expect("temp dir");
    let script_path = temp.path().join("fake_embeddings_runtime.sh");
    let launch_log = temp.path().join("launches.log");
    let env_log = temp.path().join("env.log");
    write_fake_runtime_script(&script_path, None, Some(&env_log));

    let mut runtime = fake_runtime_config(&script_path, &launch_log);
    runtime.args.push("--api-key-env".to_string());
    runtime
        .args
        .push("BITLOOPS_CUSTOM_PLATFORM_TOKEN".to_string());
    let service = with_platform_runtime_auth_environment_hook(
        |api_key_env| {
            assert_eq!(api_key_env, "BITLOOPS_CUSTOM_PLATFORM_TOKEN");
            Ok(vec![(
                api_key_env.to_string(),
                "token-from-login".to_string(),
            )])
        },
        || BitloopsEmbeddingsIpcService::new("platform_code", &runtime, "test-model", None, true),
    )
    .expect("build platform ipc service");
    assert_eq!(
        service
            .embed("hello world", EmbeddingInputType::Document)
            .expect("embedding request"),
        vec![1.0, 2.0]
    );

    let env = fs::read_to_string(&env_log).expect("read env log");
    assert!(
        env.contains("BITLOOPS_CUSTOM_PLATFORM_TOKEN=token-from-login"),
        "expected injected custom platform token env var, got: {env}"
    );
}

#[test]
fn bitloops_embeddings_ipc_service_supports_batch_embed() {
    let temp = TempDir::new().expect("temp dir");
    let script_path = temp.path().join("fake_embeddings_runtime.sh");
    let launch_log = temp.path().join("launches.log");
    write_fake_runtime_script(&script_path, None, None);

    let runtime = fake_runtime_config(&script_path, &launch_log);
    let service =
        BitloopsEmbeddingsIpcService::new("local_code", &runtime, "test-model", None, false)
            .expect("build ipc service");

    let vectors = service
        .embed_batch(
            &["first document".to_string(), "second document".to_string()],
            EmbeddingInputType::Document,
        )
        .expect("batch embed");

    assert_eq!(vectors.len(), 2);
    assert_eq!(vectors[0].len(), service.output_dimension().unwrap());
    assert_eq!(vectors[1].len(), service.output_dimension().unwrap());
}

#[test]
fn ipc_service_recovers_on_next_request_after_request_timeout() {
    let temp = TempDir::new().expect("temp dir");
    let script_path = temp.path().join("fake_embeddings_runtime.sh");
    let launch_log = temp.path().join("launches.log");
    let timeout_marker = temp.path().join("first-request-timed-out");
    write_fake_runtime_script(&script_path, Some(&timeout_marker), None);

    let runtime = fake_runtime_config(&script_path, &launch_log);
    let service =
        BitloopsEmbeddingsIpcService::new("local_code", &runtime, "test-model", None, false)
            .expect("build ipc service");

    let err = service
        .embed("hello world", EmbeddingInputType::Document)
        .expect_err("first embedding request should time out");

    assert!(
        format!("{err:#}").contains("timed out after"),
        "unexpected timeout error: {err:#}"
    );
    assert!(
        timeout_marker.exists(),
        "first request should have timed out"
    );

    let vector = service
        .embed("hello world", EmbeddingInputType::Document)
        .expect("next embedding request should recover after timeout");
    assert_eq!(vector, vec![1.0, 2.0]);
}

#[test]
fn ipc_service_reuses_hot_runtime_across_service_instances() {
    let temp = TempDir::new().expect("temp dir");
    let script_path = temp.path().join("fake_embeddings_runtime.sh");
    let launch_log = temp.path().join("launches.log");
    write_fake_runtime_script(&script_path, None, None);

    let runtime = fake_runtime_config(&script_path, &launch_log);
    let first =
        BitloopsEmbeddingsIpcService::new("local_code", &runtime, "test-model", None, false)
            .expect("build first ipc service");
    assert_eq!(
        first
            .embed("hello world", EmbeddingInputType::Document)
            .expect("first embed"),
        vec![1.0, 2.0]
    );
    drop(first);

    let second =
        BitloopsEmbeddingsIpcService::new("local_code", &runtime, "test-model", None, false)
            .expect("build second ipc service");
    assert_eq!(
        second
            .embed("goodbye world", EmbeddingInputType::Document)
            .expect("second embed"),
        vec![1.0, 2.0]
    );

    let launches = fs::read_to_string(&launch_log).expect("read launch log");
    assert_eq!(
        launches.lines().count(),
        1,
        "expected one shared runtime launch, got: {launches}"
    );
}

#[test]
fn ipc_service_shuts_down_after_idle_eviction() {
    let temp = TempDir::new().expect("temp dir");
    let script_path = temp.path().join("fake_embeddings_runtime.sh");
    let launch_log = temp.path().join("launches.log");
    write_fake_runtime_script(&script_path, None, None);

    let runtime = fake_runtime_config(&script_path, &launch_log);
    let first =
        BitloopsEmbeddingsIpcService::new("local_code", &runtime, "test-model", None, false)
            .expect("build first ipc service");
    assert_eq!(
        first
            .embed("hello world", EmbeddingInputType::Document)
            .expect("first embed"),
        vec![1.0, 2.0]
    );

    evict_idle_embeddings_sessions_for_tests(Duration::ZERO);

    let second =
        BitloopsEmbeddingsIpcService::new("local_code", &runtime, "test-model", None, false)
            .expect("build second ipc service");
    assert_eq!(
        second
            .embed("goodbye world", EmbeddingInputType::Document)
            .expect("second embed"),
        vec![1.0, 2.0]
    );

    let launches = fs::read_to_string(&launch_log).expect("read launch log");
    assert_eq!(
        launches.lines().count(),
        2,
        "expected idle eviction to force a second runtime launch, got: {launches}"
    );
}

#[test]
fn ipc_service_forces_offline_startup_when_cache_already_contains_model() {
    let temp = TempDir::new().expect("temp dir");
    let script_path = temp.path().join("fake_embeddings_runtime.sh");
    let launch_log = temp.path().join("launches.log");
    let env_log = temp.path().join("env.log");
    write_fake_runtime_script(&script_path, None, Some(&env_log));

    let cache_root = temp.path().join("cache");
    let model_cache = cache_root.join("models--BAAI--test-model").join("refs");
    fs::create_dir_all(&model_cache).expect("create cached model refs");
    fs::write(model_cache.join("main"), "commit").expect("seed cached model ref");

    let runtime = fake_runtime_config(&script_path, &launch_log);
    let service = BitloopsEmbeddingsIpcService::new(
        "local_code",
        &runtime,
        "test-model",
        Some(cache_root.as_path()),
        false,
    )
    .expect("build ipc service");
    assert_eq!(
        service
            .embed("hello world", EmbeddingInputType::Document)
            .expect("embedding request"),
        vec![1.0, 2.0]
    );

    let env = fs::read_to_string(&env_log).expect("read env log");
    assert!(
        env.contains("HF_HUB_OFFLINE=1"),
        "expected offline Hugging Face env var, got: {env}"
    );
    assert!(
        env.contains("TRANSFORMERS_OFFLINE=1"),
        "expected offline transformers env var, got: {env}"
    );
}

#[test]
fn ipc_service_keeps_online_startup_when_cache_does_not_contain_model() {
    let temp = TempDir::new().expect("temp dir");
    let script_path = temp.path().join("fake_embeddings_runtime.sh");
    let launch_log = temp.path().join("launches.log");
    let env_log = temp.path().join("env.log");
    write_fake_runtime_script(&script_path, None, Some(&env_log));

    let cache_root = temp.path().join("cache");
    fs::create_dir_all(&cache_root).expect("create cache root");

    let runtime = fake_runtime_config(&script_path, &launch_log);
    let service = BitloopsEmbeddingsIpcService::new(
        "local_code",
        &runtime,
        "test-model",
        Some(cache_root.as_path()),
        false,
    )
    .expect("build ipc service");
    assert_eq!(
        service
            .embed("hello world", EmbeddingInputType::Document)
            .expect("embedding request"),
        vec![1.0, 2.0]
    );

    let env = fs::read_to_string(&env_log).expect("read env log");
    assert!(
        env.contains("HF_HUB_OFFLINE="),
        "expected env log output, got: {env}"
    );
    assert!(
        env.contains("TRANSFORMERS_OFFLINE="),
        "expected env log output, got: {env}"
    );
    assert!(
        !env.contains("HF_HUB_OFFLINE=1"),
        "expected online startup without warm cache, got: {env}"
    );
    assert!(
        !env.contains("TRANSFORMERS_OFFLINE=1"),
        "expected online startup without warm cache, got: {env}"
    );
}
