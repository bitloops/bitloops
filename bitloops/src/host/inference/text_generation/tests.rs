use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::Path;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use serde_json::Value;
use tempfile::TempDir;

use crate::config::{InferenceConfig, InferenceRuntimeConfig, InferenceTask};
use crate::host::inference::{EmptyInferenceGateway, InferenceGateway, LocalInferenceGateway};

use super::*;

fn test_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .expect("text-generation test lock")
}

fn write_fake_runtime_script(script_path: &Path, timeout_marker: Option<&Path>) {
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
      printf '{{"type":"describe","request_id":"%s","protocol_version":1,"runtime_name":"bitloops-inference","runtime_version":"0.1.0","profile_name":"summary_local","provider":{{"kind":"ollama_chat","provider_name":"ollama","model_name":"ministral-3:3b","endpoint":"http://127.0.0.1:11434","capabilities":["text","json_object"]}}}}\n' "$request_id"
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
    write_fake_runtime_script(&script_path, Some(&timeout_marker));

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
fn runtime_service_reuses_hot_runtime_across_service_instances() {
    let _guard = test_lock();
    let temp = TempDir::new().expect("temp dir");
    let script_path = temp.path().join("fake_inference_runtime.sh");
    let launch_log = temp.path().join("launches.log");
    let config_path = temp.path().join("bitloops.toml");
    fs::write(&config_path, "[inference]\n").expect("write fake config");
    write_fake_runtime_script(&script_path, None);

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
    write_fake_runtime_script(&script_path, None);

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
            base_url: None,
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
