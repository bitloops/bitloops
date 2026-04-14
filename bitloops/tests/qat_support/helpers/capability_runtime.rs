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
    fs::write(&script_path, script).with_context(|| format!("writing {}", script_path.display()))?;
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
    fs::write(&script_path, script).with_context(|| format!("writing {}", script_path.display()))?;
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
    command: &str,
    args: &[String],
) -> String {
    let runtime_args = args
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
code_embeddings = "fake"
summary_embeddings = "fake"

[inference.runtimes.bitloops_embeddings]
command = {command:?}
args = [{runtime_args}]
startup_timeout_secs = 5
request_timeout_secs = 5

[inference.profiles.fake]
task = "embeddings"
driver = "{driver}"
runtime = "bitloops_embeddings"
model = "qat-test-model"
"#,
        relational_path = relational_path.display().to_string(),
        events_path = events_path.display().to_string(),
        blob_path = blob_path.display().to_string(),
        driver = TEST_EMBEDDINGS_DRIVER,
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
    REQUIRED_SEMANTIC_CLONE_HEALTH_CHECKS.iter().all(|check_id| {
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
    let (command, args, script_path) = fake_embeddings_runtime_command_and_args(world)?;
    let config = render_guide_aligned_semantic_clones_config(world, &command, &args);
    write_scenario_capability_config(world, &config)?;
    append_world_log(
        world,
        &format!(
            "Configured guide-aligned semantic clones fake embeddings runtime at {}.\n",
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
