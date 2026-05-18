const REQUIRED_SEMANTIC_CLONE_HEALTH_CHECKS: [&str; 3] = [
    "semantic_clones.profile_resolution",
    "semantic_clones.runtime_command",
    "semantic_clones.runtime_handshake",
];
const TEST_EMBEDDINGS_DRIVER: &str = "bitloops_embeddings_ipc";

fn scenario_repo_config_path(world: &QatWorld) -> std::path::PathBuf {
    world.repo_dir().join("config.toml")
}

fn scenario_global_config_paths(world: &QatWorld) -> Vec<std::path::PathBuf> {
    let home = world.run_dir().join("home");
    vec![
        home.join("xdg").join("bitloops").join("config.toml"),
        home.join("Library")
            .join("Application Support")
            .join("bitloops")
            .join("config.toml"),
    ]
}

fn write_scenario_capability_config(world: &QatWorld, config: &str) -> Result<()> {
    let mut paths = vec![scenario_repo_config_path(world)];
    paths.extend(scenario_global_config_paths(world));
    for path in paths {
        ensure_parent_dir(&path)?;
        fs::write(&path, config).with_context(|| format!("writing {}", path.display()))?;
    }
    Ok(())
}

fn append_scenario_capability_config(world: &QatWorld, config: &str) -> Result<()> {
    let mut paths = vec![scenario_repo_config_path(world)];
    paths.extend(scenario_global_config_paths(world));
    for path in paths {
        ensure_parent_dir(&path)?;
        let mut rendered = if path.exists() {
            fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?
        } else {
            String::new()
        };
        if !rendered.ends_with('\n') {
            rendered.push('\n');
        }
        rendered.push_str(config);
        if !rendered.ends_with('\n') {
            rendered.push('\n');
        }
        fs::write(&path, rendered).with_context(|| format!("writing {}", path.display()))?;
    }
    Ok(())
}

fn write_scenario_repo_semantic_clone_policy(world: &QatWorld) -> Result<()> {
    let policy_path = settings_local_path(world.repo_dir());
    let policy = RepoSemanticEmbeddingPolicy {
        present: true,
        embedding_mode: Some(SemanticCloneEmbeddingMode::Deterministic),
        inference: SemanticClonesInferenceBindings {
            summary_generation: None,
            code_embeddings: Some("fake".to_string()),
            summary_embeddings: Some("fake".to_string()),
        },
    };
    set_repo_semantic_embedding_policy(&policy_path, &policy)
        .with_context(|| format!("writing {}", policy_path.display()))
}

#[cfg(unix)]
fn fake_embeddings_runtime_command_and_args(
    world: &QatWorld,
) -> Result<(String, Vec<String>, std::path::PathBuf)> {
    use std::os::unix::fs::PermissionsExt;

    let script_path = world
        .run_dir()
        .join("capability-runtime")
        .join("fake-embeddings-runtime.sh");
    let script = r#"#!/bin/sh
printf '{"event":"ready","protocol":1,"capabilities":["embed","shutdown"]}\n'
while IFS= read -r line; do
  req_id=$(printf '%s\n' "$line" | sed -n 's/.*"id":"\([^"]*\)".*/\1/p')
  case "$line" in
    *'"cmd":"embed"'*)
      sleep 0.15
      printf '{"id":"%s","ok":true,"vectors":[[0.1,0.2,0.3]],"model":"qat-test-model"}\n' "$req_id"
      ;;
    *'"cmd":"shutdown"'*)
      printf '{"id":"%s","ok":true,"model":"qat-test-model"}\n' "$req_id"
      exit 0
      ;;
    *)
      printf '{"id":"%s","ok":false,"error":{"message":"unexpected request"}}\n' "$req_id"
      ;;
  esac
done
"#;

    ensure_parent_dir(&script_path)?;
    fs::write(&script_path, script)
        .with_context(|| format!("writing {}", script_path.display()))?;
    let mut permissions = fs::metadata(&script_path)
        .with_context(|| format!("reading {}", script_path.display()))?
        .permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&script_path, permissions)
        .with_context(|| format!("chmod {}", script_path.display()))?;
    Ok((
        "sh".to_string(),
        vec![script_path.display().to_string()],
        script_path,
    ))
}

#[cfg(windows)]
fn fake_embeddings_runtime_command_and_args(
    world: &QatWorld,
) -> Result<(String, Vec<String>, std::path::PathBuf)> {
    let script_path = world
        .run_dir()
        .join("capability-runtime")
        .join("fake-embeddings-runtime.ps1");
    let script = r#"
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
      Start-Sleep -Milliseconds 150
      $response = @{
        id = $request.id
        ok = $true
        vectors = @(@(0.1, 0.2, 0.3))
        model = "qat-test-model"
      }
    }
    "shutdown" {
      $response = @{
        id = $request.id
        ok = $true
        model = "qat-test-model"
      }
      $response | ConvertTo-Json -Compress
      exit 0
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

    ensure_parent_dir(&script_path)?;
    fs::write(&script_path, script)
        .with_context(|| format!("writing {}", script_path.display()))?;
    Ok((
        "powershell".to_string(),
        vec![
            "-NoProfile".to_string(),
            "-ExecutionPolicy".to_string(),
            "Bypass".to_string(),
            "-File".to_string(),
            script_path.display().to_string(),
        ],
        script_path,
    ))
}

#[cfg(unix)]
fn fake_text_generation_runtime_command_and_args(
    world: &QatWorld,
) -> Result<(String, Vec<String>, std::path::PathBuf)> {
    use std::os::unix::fs::PermissionsExt;

    let script_path = world
        .run_dir()
        .join("capability-runtime")
        .join("fake-text-generation-runtime.sh");
    let script = r#"#!/bin/sh
while IFS= read -r line; do
  request_id=$(printf '%s' "$line" | sed -n 's/.*"request_id":"\([^"]*\)".*/\1/p')
  case "$line" in
    *'"type":"describe"'*)
      printf '{"type":"describe","request_id":"%s","protocol_version":1,"runtime_name":"bitloops-inference","runtime_version":"0.1.0","profile_name":"guidance_fake","provider":{"kind":"ollama_chat","provider_name":"qat","model_name":"qat-guidance-model","endpoint":"http://127.0.0.1:11434","capabilities":["text","json_object"]}}\n' "$request_id"
      ;;
    *'"type":"shutdown"'*)
      printf '{"type":"shutdown","request_id":"%s"}\n' "$request_id"
      exit 0
      ;;
    *'"type":"infer"'*)
      printf '{"type":"infer","request_id":"%s","text":"","parsed_json":{"summary":{"intent":"Capture durable QAT context guidance.","outcome":"Persisted mocked guidance for the changed TypeScript service.","decisions":["Use deterministic QAT guidance output."],"rejectedApproaches":[],"patterns":[],"verification":["Run cargo qat-devql-capabilities after changing context guidance."],"openItems":[]},"guidanceFacts":[{"category":"DECISION","kind":"qat_mocked_guidance_generation","guidance":"Use deterministic mocked context guidance in QAT so DevQL capability composition can be tested offline.","evidenceExcerpt":"QAT fake guidance runtime returned deterministic guidance.","appliesTo":{"paths":["src/services/user-service.ts"],"symbols":[]},"confidence":"HIGH"}]},"provider_name":"qat","model_name":"qat-guidance-model"}\n' "$request_id"
      ;;
  esac
done
"#;

    ensure_parent_dir(&script_path)?;
    fs::write(&script_path, script)
        .with_context(|| format!("writing {}", script_path.display()))?;
    let mut permissions = fs::metadata(&script_path)
        .with_context(|| format!("reading {}", script_path.display()))?
        .permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&script_path, permissions)
        .with_context(|| format!("chmod {}", script_path.display()))?;
    Ok((
        "sh".to_string(),
        vec![script_path.display().to_string()],
        script_path,
    ))
}

#[cfg(windows)]
fn fake_text_generation_runtime_command_and_args(
    world: &QatWorld,
) -> Result<(String, Vec<String>, std::path::PathBuf)> {
    let script_path = world
        .run_dir()
        .join("capability-runtime")
        .join("fake-text-generation-runtime.ps1");
    let script = r#"
while (($line = [Console]::In.ReadLine()) -ne $null) {
  if ([string]::IsNullOrWhiteSpace($line)) { continue }
  $requestId = [regex]::Match($line, '"request_id":"([^"]+)"').Groups[1].Value
  if ($line -like '*"type":"describe"*') {
    Write-Output '{"type":"describe","request_id":"'"$requestId"'","protocol_version":1,"runtime_name":"bitloops-inference","runtime_version":"0.1.0","profile_name":"guidance_fake","provider":{"kind":"ollama_chat","provider_name":"qat","model_name":"qat-guidance-model","endpoint":"http://127.0.0.1:11434","capabilities":["text","json_object"]}}'
  } elseif ($line -like '*"type":"shutdown"*') {
    Write-Output '{"type":"shutdown","request_id":"'"$requestId"'"}'
    exit 0
  } elseif ($line -like '*"type":"infer"*') {
    Write-Output '{"type":"infer","request_id":"'"$requestId"'","text":"","parsed_json":{"summary":{"intent":"Capture durable QAT context guidance.","outcome":"Persisted mocked guidance for the changed TypeScript service.","decisions":["Use deterministic QAT guidance output."],"rejectedApproaches":[],"patterns":[],"verification":["Run cargo qat-devql-capabilities after changing context guidance."],"openItems":[]},"guidanceFacts":[{"category":"DECISION","kind":"qat_mocked_guidance_generation","guidance":"Use deterministic mocked context guidance in QAT so DevQL capability composition can be tested offline.","evidenceExcerpt":"QAT fake guidance runtime returned deterministic guidance.","appliesTo":{"paths":["src/services/user-service.ts"],"symbols":[]},"confidence":"HIGH"}]},"provider_name":"qat","model_name":"qat-guidance-model"}'
  }
}
"#;

    ensure_parent_dir(&script_path)?;
    fs::write(&script_path, script)
        .with_context(|| format!("writing {}", script_path.display()))?;
    Ok((
        "powershell".to_string(),
        vec![
            "-NoProfile".to_string(),
            "-ExecutionPolicy".to_string(),
            "Bypass".to_string(),
            "-File".to_string(),
            script_path.display().to_string(),
        ],
        script_path,
    ))
}

#[cfg(unix)]
fn fake_semantic_summary_runtime_command_and_args(
    world: &QatWorld,
) -> Result<(String, Vec<String>, std::path::PathBuf)> {
    use std::os::unix::fs::PermissionsExt;

    let script_path = world
        .run_dir()
        .join("capability-runtime")
        .join("fake-semantic-summary-runtime.sh");
    let script = r#"#!/bin/sh
while IFS= read -r line; do
  request_id=$(printf '%s' "$line" | sed -n 's/.*"request_id":"\([^"]*\)".*/\1/p')
  case "$line" in
    *'"type":"describe"'*)
      printf '{"type":"describe","request_id":"%s","protocol_version":1,"runtime_name":"bitloops-inference","runtime_version":"0.1.0","profile_name":"summary_fake","provider":{"kind":"ollama_chat","provider_name":"qat","model_name":"qat-summary-model","endpoint":"http://127.0.0.1:11434","capabilities":["text","json_object"]}}\n' "$request_id"
      ;;
    *'"type":"shutdown"'*)
      printf '{"type":"shutdown","request_id":"%s"}\n' "$request_id"
      exit 0
      ;;
    *'"type":"infer"'*)
      printf '{"type":"infer","request_id":"%s","text":"QAT deterministic semantic summary for clone ranking.","provider_name":"qat","model_name":"qat-summary-model"}\n' "$request_id"
      ;;
  esac
done
"#;

    ensure_parent_dir(&script_path)?;
    fs::write(&script_path, script)
        .with_context(|| format!("writing {}", script_path.display()))?;
    let mut permissions = fs::metadata(&script_path)
        .with_context(|| format!("reading {}", script_path.display()))?
        .permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&script_path, permissions)
        .with_context(|| format!("chmod {}", script_path.display()))?;
    Ok((
        "sh".to_string(),
        vec![script_path.display().to_string()],
        script_path,
    ))
}

#[cfg(windows)]
fn fake_semantic_summary_runtime_command_and_args(
    world: &QatWorld,
) -> Result<(String, Vec<String>, std::path::PathBuf)> {
    let script_path = world
        .run_dir()
        .join("capability-runtime")
        .join("fake-semantic-summary-runtime.ps1");
    let script = r#"
while (($line = [Console]::In.ReadLine()) -ne $null) {
  if ([string]::IsNullOrWhiteSpace($line)) { continue }
  $requestId = [regex]::Match($line, '"request_id":"([^"]+)"').Groups[1].Value
  if ($line -like '*"type":"describe"*') {
    Write-Output '{"type":"describe","request_id":"'"$requestId"'","protocol_version":1,"runtime_name":"bitloops-inference","runtime_version":"0.1.0","profile_name":"summary_fake","provider":{"kind":"ollama_chat","provider_name":"qat","model_name":"qat-summary-model","endpoint":"http://127.0.0.1:11434","capabilities":["text","json_object"]}}'
  } elseif ($line -like '*"type":"shutdown"*') {
    Write-Output '{"type":"shutdown","request_id":"'"$requestId"'"}'
    exit 0
  } elseif ($line -like '*"type":"infer"*') {
    Write-Output '{"type":"infer","request_id":"'"$requestId"'","text":"QAT deterministic semantic summary for clone ranking.","provider_name":"qat","model_name":"qat-summary-model"}'
  }
}
"#;

    ensure_parent_dir(&script_path)?;
    fs::write(&script_path, script)
        .with_context(|| format!("writing {}", script_path.display()))?;
    Ok((
        "powershell".to_string(),
        vec![
            "-NoProfile".to_string(),
            "-ExecutionPolicy".to_string(),
            "Bypass".to_string(),
            "-File".to_string(),
            script_path.display().to_string(),
        ],
        script_path,
    ))
}

fn render_guide_aligned_semantic_clones_config(
    world: &QatWorld,
    embeddings_command: &str,
    embeddings_args: &[String],
    summary_command: &str,
    summary_args: &[String],
) -> String {
    let embeddings_runtime_args = embeddings_args
        .iter()
        .map(|arg| format!("{arg:?}"))
        .collect::<Vec<_>>()
        .join(", ");
    let summary_runtime_args = summary_args
        .iter()
        .map(|arg| format!("{arg:?}"))
        .collect::<Vec<_>>()
        .join(", ");
    let relational_path = world
        .repo_dir()
        .join(".bitloops")
        .join("stores")
        .join("relational")
        .join("relational.db");
    let events_path = world
        .repo_dir()
        .join(".bitloops")
        .join("stores")
        .join("events.duckdb");
    let blob_path = world
        .repo_dir()
        .join(".bitloops")
        .join("stores")
        .join("blob");
    format!(
        r#"[stores.relational]
sqlite_path = {relational_path:?}

[stores.events]
duckdb_path = {events_path:?}

[stores.blob]
local_path = {blob_path:?}

[semantic_clones]
summary_mode = "auto"
embedding_mode = "deterministic"
enrichment_workers = 2

[semantic_clones.inference]
summary_generation = "summary_fake"
code_embeddings = "fake"
summary_embeddings = "fake"

[inference.runtimes.bitloops_local_embeddings]
command = {embeddings_command:?}
args = [{embeddings_runtime_args}]
startup_timeout_secs = 5
request_timeout_secs = 5

[inference.runtimes.bitloops_local_semantic_summary]
command = {summary_command:?}
args = [{summary_runtime_args}]
startup_timeout_secs = 5
request_timeout_secs = 5

[inference.profiles.fake]
task = "embeddings"
driver = "{driver}"
runtime = "bitloops_local_embeddings"
model = "qat-test-model"

[inference.profiles.summary_fake]
task = "text_generation"
driver = "ollama_chat"
runtime = "bitloops_local_semantic_summary"
model = "qat-summary-model"
base_url = "http://127.0.0.1:11434/api/chat"
temperature = "0.1"
max_output_tokens = 200
"#,
        relational_path = relational_path.display().to_string(),
        events_path = events_path.display().to_string(),
        blob_path = blob_path.display().to_string(),
        driver = TEST_EMBEDDINGS_DRIVER,
    )
}

fn render_context_guidance_fake_config(command: &str, args: &[String]) -> String {
    let runtime_args = args
        .iter()
        .map(|arg| format!("{arg:?}"))
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        r#"
[context_guidance.inference]
guidance_generation = "guidance_fake"

[inference.runtimes.bitloops_local_text_generation]
command = {command:?}
args = [{runtime_args}]
startup_timeout_secs = 5
request_timeout_secs = 5

[inference.profiles.guidance_fake]
task = "text_generation"
driver = "ollama_chat"
runtime = "bitloops_local_text_generation"
model = "qat-guidance-model"
base_url = "http://127.0.0.1:11434/api/chat"
temperature = "0.1"
max_output_tokens = 400
"#,
        command = command,
        runtime_args = runtime_args,
    )
}

fn semantic_clone_health_rows(value: &serde_json::Value) -> Vec<&serde_json::Value> {
    let mut rows = Vec::new();
    if let Some(health) = value.get("health").and_then(serde_json::Value::as_array) {
        rows.extend(health.iter());
    }
    if let Some(health) = value
        .get("language_adapters")
        .and_then(|section| section.get("health"))
        .and_then(serde_json::Value::as_array)
    {
        rows.extend(health.iter());
    }
    rows
}

fn semantic_clone_health_is_ready(value: &serde_json::Value) -> bool {
    REQUIRED_SEMANTIC_CLONE_HEALTH_CHECKS
        .iter()
        .all(|check_id| {
            semantic_clone_health_rows(value).iter().any(|row| {
                row.get("check_id")
                    .and_then(serde_json::Value::as_str)
                    .is_some_and(|actual| actual.ends_with(check_id))
                    && row
                        .get("healthy")
                        .and_then(serde_json::Value::as_bool)
                        .is_some_and(std::convert::identity)
            })
        })
}

fn semantic_clone_health_diagnostics(value: &serde_json::Value) -> Vec<String> {
    REQUIRED_SEMANTIC_CLONE_HEALTH_CHECKS
        .iter()
        .map(|check_id| {
            let matching = semantic_clone_health_rows(value).into_iter().find(|row| {
                row.get("check_id")
                    .and_then(serde_json::Value::as_str)
                    .is_some_and(|actual| actual.ends_with(check_id))
            });
            match matching {
                Some(row) => {
                    let healthy = row
                        .get("healthy")
                        .and_then(serde_json::Value::as_bool)
                        .unwrap_or(false);
                    let message = row
                        .get("message")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("missing message");
                    let details = row
                        .get("details")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("");
                    format!("{check_id}: healthy={healthy}; message={message}; details={details}")
                }
                None => format!("{check_id}: missing health check result"),
            }
        })
        .collect()
}

pub fn configure_semantic_clones_with_fake_runtime(
    world: &mut QatWorld,
    repo_name: &str,
) -> Result<()> {
    configure_semantic_clones_with_guide_aligned_fake_runtime(world, repo_name)
}

pub fn configure_semantic_clones_with_guide_aligned_fake_runtime(
    world: &mut QatWorld,
    repo_name: &str,
) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    let (embeddings_command, embeddings_args, embeddings_script_path) =
        fake_embeddings_runtime_command_and_args(world)?;
    let (summary_command, summary_args, summary_script_path) =
        fake_semantic_summary_runtime_command_and_args(world)?;
    let config = render_guide_aligned_semantic_clones_config(
        world,
        &embeddings_command,
        &embeddings_args,
        &summary_command,
        &summary_args,
    );
    write_scenario_capability_config(world, &config)?;
    write_scenario_repo_semantic_clone_policy(world)?;
    append_world_log(
        world,
        &format!(
            "Configured guide-aligned semantic clones fake embeddings runtime at {} and semantic summary runtime at {}.\n",
            embeddings_script_path.display(),
            summary_script_path.display()
        ),
    )?;
    Ok(())
}

pub fn configure_context_guidance_with_fake_runtime(
    world: &mut QatWorld,
    repo_name: &str,
) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    let (command, args, script_path) = fake_text_generation_runtime_command_and_args(world)?;
    let config = render_context_guidance_fake_config(&command, &args);
    append_scenario_capability_config(world, &config)?;
    append_world_log(
        world,
        &format!(
            "Configured context guidance fake text-generation runtime at {}.\n",
            script_path.display()
        ),
    )?;
    Ok(())
}

pub fn assert_semantic_clones_pack_health_ready(
    world: &mut QatWorld,
    repo_name: &str,
) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    let output = run_command_capture(
        world,
        "bitloops devql packs --json --with-health",
        build_bitloops_command(world, &["devql", "packs", "--json", "--with-health"])?,
    )?;
    world.last_command_exit_code = Some(output.status.code().unwrap_or(-1));
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    world.last_command_stdout = Some(stdout.clone());
    ensure_success(&output, "bitloops devql packs --json --with-health")?;

    let value: serde_json::Value =
        serde_json::from_str(stdout.trim()).context("parsing DevQL pack health json")?;
    ensure!(
        semantic_clone_health_is_ready(&value),
        "semantic clones pack health is not ready:\n{}",
        semantic_clone_health_diagnostics(&value).join("\n")
    );
    Ok(())
}
