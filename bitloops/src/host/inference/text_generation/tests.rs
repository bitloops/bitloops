use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::Path;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use serde_json::Value;
use tempfile::TempDir;

use crate::config::{
    BITLOOPS_CONFIG_RELATIVE_PATH, InferenceConfig, InferenceRuntimeConfig, InferenceTask,
    REPO_POLICY_LOCAL_FILE_NAME, resolve_inference_capability_config_for_repo,
};
use crate::host::inference::{
    BITLOOPS_PLATFORM_CHAT_DRIVER, EmptyInferenceGateway, InferenceGateway, LocalInferenceGateway,
};

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
{timeout_branch}      printf '{{"type":"infer","request_id":"%s","text":"","parsed_json":{{"summary":"Summarises the symbol.","confidence":0.91}},"provider_name":"ollama","model_name":"ministral-3:3b"}}\n' "$request_id"
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

    assert_eq!(
        serde_json::from_str::<Value>(&text).expect("parse response json"),
        serde_json::json!({
            "summary": "Summarises the symbol.",
            "confidence": 0.91
        })
    );
    assert!(
        timeout_marker.exists(),
        "first request should have timed out"
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
        serde_json::from_str::<Value>(&first.complete("system", "user").expect("first completion"))
            .expect("parse first completion"),
        serde_json::json!({
            "summary": "Summarises the symbol.",
            "confidence": 0.91
        })
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
        serde_json::from_str::<Value>(
            &second
                .complete("system", "user")
                .expect("second completion")
        )
        .expect("parse second completion"),
        serde_json::json!({
            "summary": "Summarises the symbol.",
            "confidence": 0.91
        })
    );

    let launches = fs::read_to_string(&launch_log).expect("read launch log");
    assert_eq!(
        launches.lines().count(),
        1,
        "expected one shared runtime launch, got: {launches}"
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
        serde_json::from_str::<Value>(&first.complete("system", "user").expect("first completion"))
            .expect("parse first completion"),
        serde_json::json!({
            "summary": "Summarises the symbol.",
            "confidence": 0.91
        })
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
        serde_json::from_str::<Value>(
            &second
                .complete("system", "user")
                .expect("second completion")
        )
        .expect("parse second completion"),
        serde_json::json!({
            "summary": "Summarises the symbol.",
            "confidence": 0.91
        })
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
        serde_json::from_str::<Value>(&first.complete("system", "user").expect("first completion"))
            .expect("parse first completion"),
        serde_json::json!({
            "summary": "Summarises the symbol.",
            "confidence": 0.91
        })
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
        serde_json::from_str::<Value>(
            &second
                .complete("system", "user")
                .expect("second completion")
        )
        .expect("parse second completion"),
        serde_json::json!({
            "summary": "Summarises the symbol.",
            "confidence": 0.91
        })
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
    assert_eq!(
        serde_json::from_str::<Value>(&text).expect("parse response json"),
        serde_json::json!({
            "summary": "Summarises the symbol.",
            "confidence": 0.91
        })
    );

    let launched_with = std::fs::read_to_string(&launch_log).expect("read launch log");
    assert_eq!(
        launched_with.trim(),
        bound_config_path.to_string_lossy(),
        "runtime should reuse the same daemon config path as the host inference config"
    );
}
