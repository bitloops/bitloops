use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use tempfile::TempDir;

use crate::config::{
    BITLOOPS_CONFIG_RELATIVE_PATH, InferenceConfig, InferenceRuntimeConfig, InferenceTask,
    REPO_POLICY_LOCAL_FILE_NAME, resolve_inference_capability_config_for_repo,
};
use crate::host::inference::{
    BITLOOPS_PLATFORM_CHAT_DRIVER, EmptyInferenceGateway, InferenceGateway, LocalInferenceGateway,
    TextGenerationOptions,
};
use crate::test_support::process_state::enter_process_state;

use super::*;

fn test_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .expect("text-generation test lock")
}

fn write_fake_runtime_script(
    script_path: &Path,
    timeout_marker: Option<&Path>,
    describe_capabilities_json: &str,
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
    fs::write(
        script_path,
        format!(
            r#"launch_log="$1"
shift
printf '%s\n' "$$" >> "$launch_log"

while IFS= read -r line; do
  request_id=$(printf '%s' "$line" | sed -n 's/.*"request_id":"\([^"]*\)".*/\1/p')
  case "$line" in
    *'"type":"describe"'*)
      printf '{{"type":"describe","request_id":"%s","protocol_version":1,"runtime_name":"bitloops-inference","runtime_version":"0.1.0","profile_name":"summary_local","provider":{{"kind":"ollama_chat","provider_name":"ollama","model_name":"ministral-3:3b","endpoint":"http://127.0.0.1:11434","capabilities":{describe_capabilities_json}}}}}\n' "$request_id"
      ;;
    *'"type":"shutdown"'*)
      printf '{{"type":"shutdown","request_id":"%s"}}\n' "$request_id"
      exit 0
      ;;
    *'"type":"infer"'*)
{timeout_branch}      printf '{{"type":"infer","request_id":"%s","text":"Summarises the symbol.","provider_name":"ollama","model_name":"ministral-3:3b"}}\n' "$request_id"
      ;;
  esac
done
"#,
            timeout_branch = timeout_branch,
            describe_capabilities_json = describe_capabilities_json,
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

fn fake_structured_runtime_command_and_args(
    repo_root: &Path,
    provider_name: &str,
    model_name: &str,
    response: serde_json::Value,
) -> (String, Vec<String>) {
    let script_path = repo_root.join(".bitloops/test-bin/fake-structured-runtime.sh");
    fs::create_dir_all(script_path.parent().expect("script parent"))
        .expect("create fake structured runtime dir");
    let response_json = serde_json::to_string(&response).expect("serialise response");
    fs::write(
        &script_path,
        format!(
            r#"payload=$(cat <<'JSON'
{response_json}
JSON
)

while IFS= read -r line; do
  request_id=$(printf '%s' "$line" | sed -n 's/.*"request_id":"\([^"]*\)".*/\1/p')
  case "$line" in
    *'"type":"describe"'*)
      printf '{{"type":"describe","request_id":"%s","protocol_version":1,"runtime_name":"bitloops-inference","runtime_version":"0.1.0","profile_name":"architecture_fact_synthesis_codex","provider":{{"kind":"codex_exec","provider_name":"{provider_name}","model_name":"{model_name}","endpoint":"codex","capabilities":{{"response_modes":["json_object"],"usage_reporting":false,"structured_output":["json_object","json_schema"]}}}}}}\n' "$request_id"
      ;;
    *'"type":"infer"'*)
      case "$line" in
        *'"json_schema"'*) has_schema=1 ;;
        *) has_schema=0 ;;
      esac
      case "$line" in
        *'"workspace_path"'*) has_workspace=1 ;;
        *) has_workspace=0 ;;
      esac
      if [ "$has_schema" = 1 ] && [ "$has_workspace" = 1 ]; then
        printf '{{"type":"infer","request_id":"%s","text":"","parsed_json":%s,"provider_name":"{provider_name}","model_name":"{model_name}"}}\n' "$request_id" "$payload"
      else
        printf '{{"type":"error","request_id":"%s","code":"missing_metadata","message":"expected json_schema and workspace_path metadata"}}\n' "$request_id"
      fi
      ;;
    *'"type":"shutdown"'*)
      printf '{{"type":"shutdown","request_id":"%s"}}\n' "$request_id"
      exit 0
      ;;
  esac
done
"#,
        ),
    )
    .expect("write fake structured runtime script");
    (
        "/bin/sh".to_string(),
        vec![script_path.to_string_lossy().into_owned()],
    )
}

#[test]
fn empty_gateway_rejects_unknown_text_generation_slots() {
    let _guard = test_lock();
    let gateway = EmptyInferenceGateway;
    let err = match gateway.text_generation("summary_generation") {
        Ok(_) => panic!("missing slot must fail"),
        Err(err) => err,
    };

    assert!(
        err.to_string().contains("summary_generation"),
        "unexpected error: {err}"
    );
}

#[test]
fn remote_drivers_default_text_generation_pool_to_eight_workers() {
    let _guard = test_lock();

    assert_eq!(
        text_generation_session_pool_size(BITLOOPS_PLATFORM_CHAT_DRIVER),
        8
    );
    assert_eq!(
        text_generation_session_pool_size(crate::host::inference::OPENAI_CHAT_COMPLETIONS_DRIVER),
        8
    );
    assert_eq!(text_generation_session_pool_size("ollama_chat"), 1);
}

#[test]
fn runtime_service_restarts_after_request_timeout() {
    let _guard = test_lock();
    let temp = TempDir::new().expect("temp dir");
    let script_path = temp.path().join("fake_inference_runtime.sh");
    let launch_log = temp.path().join("launches.log");
    let timeout_marker = temp.path().join("first-request-timed-out");
    let config_path = temp.path().join("bitloops.toml");
    fs::write(&config_path, "[inference]\n").expect("write fake config");
    write_fake_runtime_script(
        &script_path,
        Some(&timeout_marker),
        "[\"text\",\"json_object\"]",
    );

    let runtime = fake_runtime_config(&script_path, &launch_log);
    let service = BitloopsInferenceTextGenerationService::new(
        "summary_local",
        "ollama_chat",
        &runtime,
        &config_path,
    )
    .expect("build runtime service");

    let text = service
        .complete("system", "user")
        .expect("text-generation request should recover after timeout");

    assert_eq!(text, "Summarises the symbol.");
    assert!(
        timeout_marker.exists(),
        "first request should have timed out"
    );
}

#[test]
fn canonical_text_prefers_runtime_text() {
    let response = InferResponse {
        request_id: "infer-1".to_string(),
        text: "Plain summary text.".to_string(),
        parsed_json: Some(serde_json::json!({
            "summary": "Canonical parsed JSON"
        })),
        usage: None,
        finish_reason: None,
        provider_name: "ollama".to_string(),
        model_name: "ministral-3:3b".to_string(),
    };

    let text = canonical_text_from_response(&response).expect("canonical text");

    assert_eq!(text, "Plain summary text.");
}

#[test]
fn runtime_service_sends_text_response_mode_for_completion() {
    let _guard = test_lock();
    let temp = TempDir::new().expect("temp dir");
    let script_path = temp.path().join("text_mode_runtime.sh");
    let launch_log = temp.path().join("launches.log");
    let config_path = temp.path().join("bitloops.toml");
    fs::write(&config_path, "[inference]\n").expect("write fake config");
    fs::write(
        &script_path,
        r#"launch_log="$1"
shift
printf '%s\n' "$$" >> "$launch_log"

while IFS= read -r line; do
  request_id=$(printf '%s' "$line" | sed -n 's/.*"request_id":"\([^"]*\)".*/\1/p')
  case "$line" in
    *'"type":"describe"'*)
      printf '{"type":"describe","request_id":"%s","protocol_version":1,"runtime_name":"bitloops-inference","runtime_version":"0.1.0","profile_name":"summary_local","provider":{"kind":"ollama_chat","provider_name":"ollama","model_name":"ministral-3:3b","endpoint":"http://127.0.0.1:11434","capabilities":{"response_modes":["text","json_object"],"usage_reporting":true,"structured_output":[]}}}\n' "$request_id"
      ;;
    *'"type":"shutdown"'*)
      printf '{"type":"shutdown","request_id":"%s"}\n' "$request_id"
      exit 0
      ;;
    *'"type":"infer"'*)
      case "$line" in
        *'"response_mode":"text"'*)
          printf '{"type":"infer","request_id":"%s","text":"Plain text summary.","provider_name":"ollama","model_name":"ministral-3:3b"}\n' "$request_id"
          ;;
        *)
          printf '{"type":"infer","request_id":"%s","text":"wrong mode","provider_name":"ollama","model_name":"ministral-3:3b"}\n' "$request_id"
          ;;
      esac
      ;;
  esac
done
"#,
    )
    .expect("write text mode runtime script");

    let runtime = fake_runtime_config(&script_path, &launch_log);
    let service = BitloopsInferenceTextGenerationService::new(
        "summary_local",
        "ollama_chat",
        &runtime,
        &config_path,
    )
    .expect("build runtime service");

    assert_eq!(
        service.complete("system", "user").expect("completion"),
        "Plain text summary."
    );
}

#[test]
fn runtime_service_sends_refresh_cache_metadata_when_requested() {
    let _guard = test_lock();
    let temp = TempDir::new().expect("temp dir");
    let script_path = temp.path().join("refresh_runtime.sh");
    let launch_log = temp.path().join("launches.log");
    let config_path = temp.path().join("bitloops.toml");
    fs::write(&config_path, "[inference]\n").expect("write fake config");
    fs::write(
        &script_path,
        r#"launch_log="$1"
shift
printf '%s\n' "$$" >> "$launch_log"

while IFS= read -r line; do
  request_id=$(printf '%s' "$line" | sed -n 's/.*"request_id":"\([^"]*\)".*/\1/p')
  case "$line" in
    *'"type":"describe"'*)
      printf '{"type":"describe","request_id":"%s","protocol_version":1,"runtime_name":"bitloops-inference","runtime_version":"0.1.0","profile_name":"summary_local","provider":{"kind":"ollama_chat","provider_name":"ollama","model_name":"ministral-3:3b","endpoint":"http://127.0.0.1:11434","capabilities":{"response_modes":["text","json_object"],"usage_reporting":true,"structured_output":[]}}}\n' "$request_id"
      ;;
    *'"type":"shutdown"'*)
      printf '{"type":"shutdown","request_id":"%s"}\n' "$request_id"
      exit 0
      ;;
    *'"type":"infer"'*)
      case "$line" in
        *'"bitloops_refresh_cache":true'*)
          printf '{"type":"infer","request_id":"%s","text":"Refreshed summary.","provider_name":"ollama","model_name":"ministral-3:3b"}\n' "$request_id"
          ;;
        *)
          printf '{"type":"infer","request_id":"%s","text":"Cached summary.","provider_name":"ollama","model_name":"ministral-3:3b"}\n' "$request_id"
          ;;
      esac
      ;;
  esac
done
"#,
    )
    .expect("write refresh runtime script");

    let runtime = fake_runtime_config(&script_path, &launch_log);
    let service = BitloopsInferenceTextGenerationService::new(
        "summary_local",
        "ollama_chat",
        &runtime,
        &config_path,
    )
    .expect("build runtime service");

    assert_eq!(
        service
            .complete_with_options(
                "system",
                "user",
                TextGenerationOptions {
                    refresh_cache: true
                },
            )
            .expect("refresh completion"),
        "Refreshed summary."
    );
}

#[test]
fn platform_runtime_service_requires_authenticated_session() {
    let _guard = test_lock();
    let temp = TempDir::new().expect("temp dir");
    let script_path = temp.path().join("fake_inference_runtime.sh");
    let launch_log = temp.path().join("launches.log");
    let config_path = temp.path().join("bitloops.toml");
    fs::write(&config_path, "[inference]\n").expect("write fake config");
    write_fake_runtime_script(&script_path, None, "[\"text\",\"json_object\"]");

    let runtime = fake_runtime_config(&script_path, &launch_log);
    let err = with_platform_runtime_auth_environment_hook(
        || Ok(Vec::new()),
        || match BitloopsInferenceTextGenerationService::new(
            "summary_llm",
            BITLOOPS_PLATFORM_CHAT_DRIVER,
            &runtime,
            &config_path,
        ) {
            Ok(_) => panic!("platform service without auth must fail"),
            Err(err) => err,
        },
    );

    assert!(
        format!("{err:#}").contains("requires an authenticated Bitloops session"),
        "unexpected error: {err:#}"
    );
    assert!(
        !launch_log.exists(),
        "platform runtime should not be spawned without an auth token"
    );
}

#[test]
fn runtime_service_reuses_hot_runtime_across_service_instances() {
    let _guard = test_lock();
    let temp = TempDir::new().expect("temp dir");
    let script_path = temp.path().join("fake_inference_runtime.sh");
    let launch_log = temp.path().join("launches.log");
    let config_path = temp.path().join("bitloops.toml");
    fs::write(&config_path, "[inference]\n").expect("write fake config");
    write_fake_runtime_script(&script_path, None, "[\"text\",\"json_object\"]");

    let runtime = fake_runtime_config(&script_path, &launch_log);
    let first = BitloopsInferenceTextGenerationService::new(
        "summary_local",
        "ollama_chat",
        &runtime,
        &config_path,
    )
    .expect("build first runtime service");
    assert_eq!(
        first.complete("system", "user").expect("first completion"),
        "Summarises the symbol."
    );
    drop(first);

    let second = BitloopsInferenceTextGenerationService::new(
        "summary_local",
        "ollama_chat",
        &runtime,
        &config_path,
    )
    .expect("build second runtime service");
    assert_eq!(
        second
            .complete("system", "user")
            .expect("second completion"),
        "Summarises the symbol."
    );

    let launches = fs::read_to_string(&launch_log).expect("read launch log");
    assert_eq!(
        launches.lines().count(),
        1,
        "expected one shared runtime launch, got: {launches}"
    );
}

#[test]
fn platform_runtime_service_processes_parallel_requests() {
    let _guard = test_lock();
    let temp = TempDir::new().expect("temp dir");
    let script_path = temp.path().join("parallel_inference_runtime.sh");
    let launch_log = temp.path().join("launches.log");
    let config_path = temp.path().join("bitloops.toml");
    fs::write(&config_path, "[inference]\n").expect("write fake config");
    fs::write(
        &script_path,
        r#"launch_log="$1"
shift
printf '%s\n' "$$" >> "$launch_log"

while IFS= read -r line; do
  request_id=$(printf '%s' "$line" | sed -n 's/.*"request_id":"\([^"]*\)".*/\1/p')
  case "$line" in
    *'"type":"describe"'*)
      printf '{"type":"describe","request_id":"%s","protocol_version":1,"runtime_name":"bitloops-inference","runtime_version":"0.1.0","profile_name":"summary_llm","provider":{"kind":"hosted_chat","provider_name":"bitloops-platform","model_name":"ministral-3-3b-instruct","endpoint":"https://platform.example.test","capabilities":["text","json_object"]}}\n' "$request_id"
      ;;
    *'"type":"shutdown"'*)
      printf '{"type":"shutdown","request_id":"%s"}\n' "$request_id"
      exit 0
      ;;
    *'"type":"infer"'*)
      sleep 1
      printf '{"type":"infer","request_id":"%s","text":"Summarises the symbol.","provider_name":"bitloops-platform","model_name":"ministral-3-3b-instruct"}\n' "$request_id"
      ;;
  esac
done
"#,
    )
    .expect("write parallel runtime script");

    let runtime = InferenceRuntimeConfig {
        command: "/bin/sh".to_string(),
        args: vec![
            script_path.to_string_lossy().into_owned(),
            launch_log.to_string_lossy().into_owned(),
        ],
        startup_timeout_secs: 1,
        request_timeout_secs: 3,
    };

    let _process_state = enter_process_state(
        None,
        &[(
            crate::daemon::PLATFORM_GATEWAY_TOKEN_ENV,
            Some("test-token"),
        )],
    );
    let service = Arc::new(
        BitloopsInferenceTextGenerationService::new(
            "summary_llm",
            BITLOOPS_PLATFORM_CHAT_DRIVER,
            &runtime,
            &config_path,
        )
        .expect("build platform runtime service"),
    );
    let started = std::time::Instant::now();
    std::thread::scope(|scope| {
        let first = {
            let service = Arc::clone(&service);
            scope.spawn(move || service.complete("system", "user-one"))
        };
        let second = {
            let service = Arc::clone(&service);
            scope.spawn(move || service.complete("system", "user-two"))
        };
        for completion in [first.join(), second.join()] {
            let text = completion
                .expect("parallel completion thread should join")
                .expect("parallel completion should succeed");
            assert_eq!(text, "Summarises the symbol.");
        }
    });
    let elapsed = started.elapsed();

    assert!(
        elapsed < Duration::from_millis(1_800),
        "expected parallel platform requests to overlap, took {elapsed:?}"
    );
    let launches = fs::read_to_string(&launch_log).expect("read launch log");
    assert!(
        launches.lines().count() >= 2,
        "expected the platform pool to launch multiple runtimes, got: {launches}"
    );
}

#[test]
fn runtime_service_shuts_down_after_idle_eviction() {
    let _guard = test_lock();
    let temp = TempDir::new().expect("temp dir");
    let script_path = temp.path().join("fake_inference_runtime.sh");
    let launch_log = temp.path().join("launches.log");
    let config_path = temp.path().join("bitloops.toml");
    fs::write(&config_path, "[inference]\n").expect("write fake config");
    write_fake_runtime_script(&script_path, None, "[\"text\",\"json_object\"]");

    let runtime = fake_runtime_config(&script_path, &launch_log);
    let first = BitloopsInferenceTextGenerationService::new(
        "summary_local",
        "ollama_chat",
        &runtime,
        &config_path,
    )
    .expect("build first runtime service");
    assert_eq!(
        first.complete("system", "user").expect("first completion"),
        "Summarises the symbol."
    );

    evict_idle_text_generation_sessions_for_tests(Duration::ZERO);

    let second = BitloopsInferenceTextGenerationService::new(
        "summary_local",
        "ollama_chat",
        &runtime,
        &config_path,
    )
    .expect("build second runtime service");
    assert_eq!(
        second
            .complete("system", "user")
            .expect("second completion"),
        "Summarises the symbol."
    );

    let launches = fs::read_to_string(&launch_log).expect("read launch log");
    assert_eq!(
        launches.lines().count(),
        2,
        "expected a new runtime launch after idle eviction, got: {launches}"
    );
}

#[test]
fn runtime_service_accepts_structured_provider_capabilities_from_describe() {
    let _guard = test_lock();
    let temp = TempDir::new().expect("temp dir");
    let script_path = temp.path().join("fake_inference_runtime.sh");
    let launch_log = temp.path().join("launches.log");
    let config_path = temp.path().join("bitloops.toml");
    fs::write(&config_path, "[inference]\n").expect("write fake config");
    write_fake_runtime_script(
        &script_path,
        None,
        r#"{"response_modes":["text","json_object"],"usage_reporting":true}"#,
    );

    let runtime = fake_runtime_config(&script_path, &launch_log);
    let first = BitloopsInferenceTextGenerationService::new(
        "summary_local",
        "ollama_chat",
        &runtime,
        &config_path,
    )
    .expect("build first runtime service");
    assert_eq!(
        first.complete("system", "user").expect("first completion"),
        "Summarises the symbol."
    );

    evict_idle_text_generation_sessions_for_tests(Duration::ZERO);

    let second = BitloopsInferenceTextGenerationService::new(
        "summary_local",
        "ollama_chat",
        &runtime,
        &config_path,
    )
    .expect("build second runtime service");
    assert_eq!(
        second
            .complete("system", "user")
            .expect("second completion"),
        "Summarises the symbol."
    );

    let launches = fs::read_to_string(&launch_log).expect("read launch log");
    assert_eq!(
        launches.lines().count(),
        2,
        "expected idle eviction to force a second runtime launch, got: {launches}"
    );
}

#[test]
fn scoped_gateway_reports_bound_text_generation_slots() {
    let _guard = test_lock();
    let gateway = LocalInferenceGateway::new(
        Path::new("/repo"),
        InferenceConfig::default(),
        HashMap::from([(
            "semantic_clones".to_string(),
            BTreeMap::from([(
                "summary_generation".to_string(),
                "summary_local".to_string(),
            )]),
        )]),
    );
    let scoped = gateway.scoped(Some("semantic_clones"));

    assert!(scoped.has_slot("summary_generation"));
    assert!(!scoped.has_slot("unknown"));
    let description = scoped
        .describe("summary_generation")
        .expect("slot description");
    assert_eq!(description.profile_name, "summary_local");
}

#[test]
fn gateway_rejects_text_generation_profile_without_runtime() {
    let _guard = test_lock();
    let mut inference = InferenceConfig::default();
    inference.profiles.insert(
        "summary_local".to_string(),
        crate::config::InferenceProfileConfig {
            name: "summary_local".to_string(),
            task: InferenceTask::TextGeneration,
            driver: "ollama_chat".to_string(),
            runtime: None,
            model: Some("ministral-3:3b".to_string()),
            api_key: None,
            base_url: Some("http://127.0.0.1:11434/api/chat".to_string()),
            temperature: Some("0.1".to_string()),
            max_output_tokens: Some(200),
            cache_dir: None,
        },
    );
    let temp = TempDir::new().expect("temp dir");
    let gateway = LocalInferenceGateway::new(
        temp.path(),
        inference,
        HashMap::from([(
            "semantic_clones".to_string(),
            BTreeMap::from([(
                "summary_generation".to_string(),
                "summary_local".to_string(),
            )]),
        )]),
    );

    let err = match gateway
        .scoped(Some("semantic_clones"))
        .text_generation("summary_generation")
    {
        Ok(_) => panic!("missing runtime must fail"),
        Err(err) => err,
    };

    assert!(
        err.to_string().contains("requires a runtime"),
        "unexpected error: {err:#}"
    );
}

#[test]
fn gateway_rejects_text_generation_profile_without_request_defaults() {
    let _guard = test_lock();
    let mut inference = InferenceConfig::default();
    inference.runtimes.insert(
        "bitloops_inference".to_string(),
        InferenceRuntimeConfig {
            command: "/bin/true".to_string(),
            args: Vec::new(),
            startup_timeout_secs: 1,
            request_timeout_secs: 1,
        },
    );
    inference.profiles.insert(
        "summary_local".to_string(),
        crate::config::InferenceProfileConfig {
            name: "summary_local".to_string(),
            task: InferenceTask::TextGeneration,
            driver: "ollama_chat".to_string(),
            runtime: Some("bitloops_inference".to_string()),
            model: Some("ministral-3:3b".to_string()),
            api_key: None,
            base_url: Some("http://127.0.0.1:11434/api/chat".to_string()),
            temperature: None,
            max_output_tokens: Some(200),
            cache_dir: None,
        },
    );
    let temp = TempDir::new().expect("temp dir");
    let gateway = LocalInferenceGateway::new(
        temp.path(),
        inference,
        HashMap::from([(
            "semantic_clones".to_string(),
            BTreeMap::from([(
                "summary_generation".to_string(),
                "summary_local".to_string(),
            )]),
        )]),
    );

    let err = match gateway
        .scoped(Some("semantic_clones"))
        .text_generation("summary_generation")
    {
        Ok(_) => panic!("missing defaults must fail"),
        Err(err) => err,
    };

    assert!(
        err.to_string().contains("requires a temperature"),
        "unexpected error: {err:#}"
    );
}

#[test]
fn gateway_uses_same_daemon_config_path_as_host_inference_resolution() {
    let _guard = test_lock();
    let temp = TempDir::new().expect("temp dir");
    let repo_root = temp.path();
    let script_path = repo_root.join("fake_inference_runtime.sh");
    let launch_log = repo_root.join("launches.log");
    let local_config_path = repo_root.join(BITLOOPS_CONFIG_RELATIVE_PATH);
    let bound_config_path = repo_root.join("bound-config.toml");
    std::fs::create_dir_all(local_config_path.parent().expect("config parent"))
        .expect("create config parent");
    std::fs::write(
        &script_path,
        r#"launch_log="$1"
printf '%s\n' "$4" >> "$launch_log"

while IFS= read -r line; do
  request_id=$(printf '%s' "$line" | sed -n 's/.*"request_id":"\([^"]*\)".*/\1/p')
  case "$line" in
    *'"type":"describe"'*)
      printf '{"type":"describe","request_id":"%s","protocol_version":1,"runtime_name":"bitloops-inference","runtime_version":"0.1.0","profile_name":"summary_local","provider":{"kind":"ollama_chat","provider_name":"ollama","model_name":"ministral-3:3b","endpoint":"http://127.0.0.1:11434","capabilities":{"response_modes":["text","json_object"],"usage_reporting":true,"structured_output":[]}}}\n' "$request_id"
      ;;
    *'"type":"shutdown"'*)
      printf '{"type":"shutdown","request_id":"%s"}\n' "$request_id"
      exit 0
      ;;
    *'"type":"infer"'*)
      printf '{"type":"infer","request_id":"%s","text":"Summarises the symbol.","provider_name":"ollama","model_name":"ministral-3:3b"}\n' "$request_id"
      ;;
  esac
done
"#,
    )
    .expect("write fake runtime script");
    std::fs::write(
        &local_config_path,
        r#"
[semantic_clones.inference]
summary_generation = "local_summary"

[inference.profiles.local_summary]
task = "text_generation"
driver = "openai"
model = "gpt-4.1-mini"
api_key = "sk-test"
base_url = "https://api.openai.com/v1/chat/completions"
temperature = "0.1"
max_output_tokens = 200
"#,
    )
    .expect("write local daemon config");
    std::fs::write(
        &bound_config_path,
        format!(
            r#"
[semantic_clones.inference]
summary_generation = "summary_local"

[inference.runtimes.bitloops_inference]
command = "/bin/sh"
args = ["{}", "{}"]
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
            script_path.display(),
            launch_log.display(),
        ),
    )
    .expect("write bound config");
    std::fs::write(
        repo_root.join(REPO_POLICY_LOCAL_FILE_NAME),
        format!(
            r#"
[daemon]
config_path = "{}"
"#,
            bound_config_path.display(),
        ),
    )
    .expect("write repo policy");

    let capability = resolve_inference_capability_config_for_repo(repo_root);
    let summary_profile = capability
        .semantic_clones
        .inference
        .summary_generation
        .clone()
        .expect("summary profile binding");
    let gateway = LocalInferenceGateway::new(
        repo_root,
        capability.inference,
        HashMap::from([(
            "semantic_clones".to_string(),
            BTreeMap::from([("summary_generation".to_string(), summary_profile)]),
        )]),
    );

    let text = gateway
        .scoped(Some("semantic_clones"))
        .text_generation("summary_generation")
        .and_then(|service| service.complete("system", "user"))
        .expect("runtime-backed text generation");
    assert_eq!(text, "Summarises the symbol.");

    let launched_with = std::fs::read_to_string(&launch_log).expect("read launch log");
    assert_eq!(
        launched_with.trim(),
        bound_config_path.to_string_lossy(),
        "runtime should reuse the same daemon config path as the host inference config"
    );
}

#[test]
fn codex_structured_generation_uses_bitloops_inference_launcher_runtime() {
    let _guard = test_lock();
    let repo = tempfile::TempDir::new().expect("tempdir");
    let repo_root = repo.path();
    let config_path = repo_root.join(BITLOOPS_CONFIG_RELATIVE_PATH);
    std::fs::create_dir_all(config_path.parent().expect("config parent"))
        .expect("create config parent");
    let (launcher_command, launcher_args) = fake_structured_runtime_command_and_args(
        repo_root,
        "codex",
        "gpt-5.4-mini",
        serde_json::json!({ "nodes": [], "edges": [] }),
    );
    let launcher_args = launcher_args
        .iter()
        .map(|arg| format!("{arg:?}"))
        .collect::<Vec<_>>()
        .join(", ");

    std::fs::write(
        &config_path,
        format!(
            r#"
[architecture.inference]
fact_synthesis = "architecture_fact_synthesis_codex"

[inference.runtimes.bitloops_inference]
command = {launcher_command:?}
args = [{launcher_args}]
startup_timeout_secs = 5
request_timeout_secs = 5

[inference.runtimes.codex]
command = "/tmp/bitloops-nonexistent-codex-for-launcher-regression"
args = ["--ask-for-approval", "never"]
startup_timeout_secs = 5
request_timeout_secs = 600

[inference.profiles.architecture_fact_synthesis_codex]
task = "structured_generation"
driver = "codex_exec"
runtime = "codex"
model = "gpt-5.4-mini"
temperature = "0.1"
max_output_tokens = 4096
"#
        ),
    )
    .expect("write config");

    let capability = resolve_inference_capability_config_for_repo(repo_root);
    let mut architecture_slots = BTreeMap::new();
    architecture_slots.insert(
        "fact_synthesis".to_string(),
        "architecture_fact_synthesis_codex".to_string(),
    );
    let mut slot_bindings = HashMap::new();
    slot_bindings.insert("architecture_graph".to_string(), architecture_slots);
    let gateway = LocalInferenceGateway::new(repo_root, capability.inference, slot_bindings);

    let service = gateway
        .scoped(Some("architecture_graph"))
        .structured_generation("fact_synthesis")
        .expect("service should use bitloops_inference launcher");
    assert_eq!(service.descriptor(), "codex:gpt-5.4-mini");
}

#[test]
fn codex_structured_generation_sends_schema_and_workspace_metadata() {
    let _guard = test_lock();
    let repo = tempfile::TempDir::new().expect("tempdir");
    let repo_root = repo.path();
    let config_path = repo_root.join(BITLOOPS_CONFIG_RELATIVE_PATH);
    std::fs::create_dir_all(config_path.parent().expect("config parent"))
        .expect("create config parent");
    let (launcher_command, launcher_args) = fake_structured_runtime_command_and_args(
        repo_root,
        "codex",
        "gpt-5.4-mini",
        serde_json::json!({ "nodes": [], "edges": [] }),
    );
    let launcher_args = launcher_args
        .iter()
        .map(|arg| format!("{arg:?}"))
        .collect::<Vec<_>>()
        .join(", ");

    std::fs::write(
        &config_path,
        format!(
            r#"
[architecture.inference]
fact_synthesis = "architecture_fact_synthesis_codex"

[inference.runtimes.bitloops_inference]
command = {launcher_command:?}
args = [{launcher_args}]
startup_timeout_secs = 5
request_timeout_secs = 5

[inference.runtimes.codex]
command = "codex"
args = ["--ask-for-approval", "never"]
startup_timeout_secs = 5
request_timeout_secs = 600

[inference.profiles.architecture_fact_synthesis_codex]
task = "structured_generation"
driver = "codex_exec"
runtime = "codex"
model = "gpt-5.4-mini"
temperature = "0.1"
max_output_tokens = 4096
"#
        ),
    )
    .expect("write config");

    let capability = resolve_inference_capability_config_for_repo(repo_root);
    let mut architecture_slots = BTreeMap::new();
    architecture_slots.insert(
        "fact_synthesis".to_string(),
        "architecture_fact_synthesis_codex".to_string(),
    );
    let mut slot_bindings = HashMap::new();
    slot_bindings.insert("architecture_graph".to_string(), architecture_slots);
    let gateway = LocalInferenceGateway::new(repo_root, capability.inference, slot_bindings);

    let service = gateway
        .scoped(Some("architecture_graph"))
        .structured_generation("fact_synthesis")
        .expect("service should use bitloops_inference launcher");
    let response = service
        .generate(crate::host::inference::StructuredGenerationRequest {
            system_prompt: "system".to_string(),
            user_prompt: "user".to_string(),
            json_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "nodes": { "type": "array" },
                    "edges": { "type": "array" }
                },
                "required": ["nodes", "edges"],
                "additionalProperties": false
            }),
            workspace_path: Some(repo_root.display().to_string()),
            metadata: serde_json::Map::new(),
        })
        .expect("structured generation request");

    assert_eq!(response["nodes"], serde_json::json!([]));
    assert_eq!(response["edges"], serde_json::json!([]));
}
