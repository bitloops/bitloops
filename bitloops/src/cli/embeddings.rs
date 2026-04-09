use anyhow::{Context, Result, bail};
use clap::{Args, Subcommand};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use crate::cli::enable::find_repo_root;
use crate::config::unified_config::resolve_embedding_capability_from_unified;
use crate::config::{
    BITLOOPS_CONFIG_RELATIVE_PATH, DaemonEmbeddingsInstallMode, EmbeddingCapabilityConfig,
    EmbeddingProfileConfig, InferenceTask, load_daemon_settings, prepare_daemon_embeddings_install,
    resolve_daemon_config_path_for_repo, resolve_embedding_capability_config_for_repo,
};
use crate::host::inference::{
    BITLOOPS_EMBEDDINGS_IPC_DRIVER, EmbeddingInputType, InferenceGateway, LocalInferenceGateway,
};

#[cfg(test)]
const LOCAL_PULL_TIMEOUT_SECS: u64 = 300;

#[derive(Args, Debug, Clone, Default)]
pub struct EmbeddingsArgs {
    #[command(subcommand)]
    pub command: Option<EmbeddingsCommand>,
}

#[derive(Subcommand, Debug, Clone)]
pub enum EmbeddingsCommand {
    /// Download or warm a local embedding profile into its cache directory.
    Pull(EmbeddingsPullArgs),
    /// Inspect the selected or explicitly named embedding profile.
    Doctor(EmbeddingsDoctorArgs),
    /// Remove the cache for a local embedding profile.
    ClearCache(EmbeddingsClearCacheArgs),
}

#[derive(Args, Debug, Clone)]
pub struct EmbeddingsPullArgs {
    pub profile: String,
}

#[derive(Args, Debug, Clone, Default)]
pub struct EmbeddingsDoctorArgs {
    #[arg(value_name = "profile")]
    pub profile: Option<String>,
}

#[derive(Args, Debug, Clone)]
pub struct EmbeddingsClearCacheArgs {
    pub profile: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum EmbeddingsInstallState {
    NotConfigured,
    ConfiguredLocal {
        profile_name: String,
    },
    ConfiguredNonLocal {
        profile_name: String,
        kind: Option<String>,
    },
}

pub fn run(args: EmbeddingsArgs) -> Result<()> {
    let Some(command) = args.command else {
        bail!(
            "missing subcommand. Use one of: `bitloops embeddings pull`, `bitloops embeddings doctor`, `bitloops embeddings clear-cache`"
        );
    };

    let repo_root = current_repo_root()?;
    let capability = resolve_embedding_capability_config_for_repo(&repo_root);
    let config_path = resolve_daemon_config_path_for_repo(&repo_root)
        .unwrap_or_else(|_| repo_root.join(BITLOOPS_CONFIG_RELATIVE_PATH));

    match command {
        EmbeddingsCommand::Pull(args) => {
            let lines = pull_profile_with_config_path(
                &repo_root,
                &config_path,
                &capability,
                &args.profile,
            )?;
            for line in lines {
                println!("{line}");
            }
            Ok(())
        }
        EmbeddingsCommand::Doctor(args) => {
            let lines = doctor_profile(&repo_root, &capability, args.profile.as_deref())?;
            for line in lines {
                println!("{line}");
            }
            Ok(())
        }
        EmbeddingsCommand::ClearCache(args) => {
            let lines = clear_cache_for_profile(&repo_root, &capability, &args.profile)?;
            for line in lines {
                println!("{line}");
            }
            Ok(())
        }
    }
}

fn current_repo_root() -> Result<PathBuf> {
    let cwd = env::current_dir().context("getting current directory")?;
    find_repo_root(&cwd)
}

pub(crate) fn install_or_bootstrap_embeddings(repo_root: &Path) -> Result<Vec<String>> {
    let config_path = resolve_daemon_config_path_for_repo(repo_root)?;
    let plan = prepare_daemon_embeddings_install(&config_path)?;

    match plan.mode {
        DaemonEmbeddingsInstallMode::SkipHosted => {
            let profile_driver = plan
                .profile_driver
                .as_deref()
                .map(|driver| format!(" (driver `{driver}`)"))
                .unwrap_or_default();
            return Ok(vec![format!(
                "Embeddings are already configured via profile `{}`{}; skipped local runtime bootstrap.",
                plan.profile_name, profile_driver
            )]);
        }
        DaemonEmbeddingsInstallMode::WarmExisting | DaemonEmbeddingsInstallMode::Bootstrap => {}
    }

    let capability = embedding_capability_for_config_path(&config_path)?;
    let result =
        pull_profile_with_config_path(repo_root, &config_path, &capability, &plan.profile_name);
    match result {
        Ok(mut lines) => {
            if matches!(plan.mode, DaemonEmbeddingsInstallMode::Bootstrap) {
                lines.insert(
                    0,
                    format!("Configured embeddings in {}.", plan.config_path.display()),
                );
            } else {
                lines.insert(
                    0,
                    format!(
                        "Embeddings already configured via profile `{}`; warming local cache.",
                        plan.profile_name
                    ),
                );
            }
            Ok(lines)
        }
        Err(err) => {
            if plan.config_modified {
                plan.rollback()?;
            }
            Err(err)
        }
    }
}

pub(crate) fn inspect_embeddings_install_state(repo_root: &Path) -> EmbeddingsInstallState {
    let Ok(config_path) = resolve_daemon_config_path_for_repo(repo_root) else {
        return EmbeddingsInstallState::NotConfigured;
    };
    if !config_path.is_file() {
        return EmbeddingsInstallState::NotConfigured;
    }
    let Ok(capability) = embedding_capability_for_config_path(&config_path) else {
        return EmbeddingsInstallState::NotConfigured;
    };
    let Some(profile_name) = selected_inference_profile_name(&capability).map(ToOwned::to_owned)
    else {
        return EmbeddingsInstallState::NotConfigured;
    };
    let kind = capability
        .inference
        .profiles
        .get(&profile_name)
        .map(|profile| profile.driver.clone());
    if kind.as_deref() == Some(BITLOOPS_EMBEDDINGS_IPC_DRIVER) {
        EmbeddingsInstallState::ConfiguredLocal { profile_name }
    } else {
        EmbeddingsInstallState::ConfiguredNonLocal { profile_name, kind }
    }
}

#[cfg(test)]
pub(crate) fn pull_profile(
    repo_root: &Path,
    capability: &EmbeddingCapabilityConfig,
    profile_name: &str,
) -> Result<Vec<String>> {
    let config_path = resolve_daemon_config_path_for_repo(repo_root)
        .unwrap_or_else(|_| repo_root.join(BITLOOPS_CONFIG_RELATIVE_PATH));
    pull_profile_with_config_path(repo_root, &config_path, capability, profile_name)
}

fn pull_profile_with_config_path(
    repo_root: &Path,
    _config_path: &Path,
    capability: &EmbeddingCapabilityConfig,
    profile_name: &str,
) -> Result<Vec<String>> {
    let profile = resolve_profile(capability, profile_name)?;
    ensure_local_profile(profile, profile_name)?;

    let cache_dir = local_profile_cache_dir(profile)?;
    if let Some(parent) = cache_dir.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("creating cache parent {}", parent.display()))?;
    }

    let gateway = LocalInferenceGateway::new(
        repo_root,
        capability.inference.clone(),
        std::collections::HashMap::new(),
    );
    let provider = gateway.embeddings(profile_name)?;
    let _ = provider
        .embed(
            "bitloops embeddings cache warmup",
            EmbeddingInputType::Document,
        )
        .context("warming local embedding cache")?;

    Ok(vec![
        format!("Pulled embedding profile `{profile_name}`."),
        format!("Cache directory: {}", cache_dir.display()),
        format!(
            "Runtime: {} {}",
            provider.provider_name(),
            provider.model_name()
        ),
    ])
}

pub(crate) fn doctor_profile(
    _repo_root: &Path,
    capability: &EmbeddingCapabilityConfig,
    profile_name: Option<&str>,
) -> Result<Vec<String>> {
    let Some((profile_name, profile)) = resolve_doctor_target(capability, profile_name)? else {
        return Ok(vec![
            "Embeddings: disabled".to_string(),
            "No embedding inference profile is bound in [semantic_clones.inference].".to_string(),
        ]);
    };

    let mut lines = vec![
        format!("Profile: {profile_name}"),
        format!("Task: {}", profile.task),
        format!("Driver: {}", profile.driver),
        format!("Kind: {}", profile.driver),
    ];

    if let Some(model) = profile.model.as_deref() {
        lines.push(format!("Model: {model}"));
    }
    if let Some(base_url) = profile.base_url.as_deref() {
        lines.push(format!("Base URL: {base_url}"));
    }

    match profile.driver.as_str() {
        BITLOOPS_EMBEDDINGS_IPC_DRIVER => {
            let cache_dir = local_profile_cache_dir(profile)?;
            lines.push(format!("Cache directory: {}", cache_dir.display()));
            lines.push(format!(
                "Cache status: {}",
                if cache_dir.exists() {
                    "present"
                } else {
                    "missing"
                }
            ));
            if let Some(runtime_name) = profile.runtime.as_deref() {
                let runtime = capability.inference.runtimes.get(runtime_name);
                lines.push(format!("Runtime: {runtime_name}"));
                if let Some(runtime) = runtime {
                    lines.push(format!("Runtime command: {}", runtime.command));
                }
            }
        }
        "openai" | "voyage" => {
            lines.push("Cache directory: not applicable".to_string());
            lines.push("Runtime: hosted profile".to_string());
        }
        other => {
            lines.push(format!("Cache directory: unsupported for driver `{other}`"));
        }
    }

    Ok(lines)
}

pub(crate) fn clear_cache_for_profile(
    _repo_root: &Path,
    capability: &EmbeddingCapabilityConfig,
    profile_name: &str,
) -> Result<Vec<String>> {
    let profile = resolve_profile(capability, profile_name)?;
    ensure_local_profile(profile, profile_name)?;
    let cache_dir = local_profile_cache_dir(profile)?;

    if cache_dir.exists() {
        fs::remove_dir_all(&cache_dir)
            .with_context(|| format!("removing cache directory {}", cache_dir.display()))?;
        return Ok(vec![
            format!("Cleared cache for profile `{profile_name}`."),
            format!("Cache directory: {}", cache_dir.display()),
        ]);
    }

    Ok(vec![
        format!("Cache already empty for profile `{profile_name}`."),
        format!("Cache directory: {}", cache_dir.display()),
    ])
}

fn resolve_doctor_target<'a>(
    capability: &'a EmbeddingCapabilityConfig,
    profile_name: Option<&'a str>,
) -> Result<Option<(&'a str, &'a EmbeddingProfileConfig)>> {
    if !capability
        .inference
        .profiles
        .values()
        .any(|profile| profile.task == InferenceTask::Embeddings)
    {
        return Ok(None);
    }

    if let Some(profile_name) = profile_name {
        let profile = resolve_profile(capability, profile_name)?;
        return Ok(Some((profile_name, profile)));
    }

    if let Some(active_profile) = selected_inference_profile_name(capability) {
        let profile = resolve_profile(capability, active_profile)?;
        return Ok(Some((active_profile, profile)));
    }

    if capability
        .inference
        .profiles
        .values()
        .filter(|profile| profile.task == InferenceTask::Embeddings)
        .count()
        == 1
    {
        let (name, profile) = capability
            .inference
            .profiles
            .iter()
            .find(|(_, profile)| profile.task == InferenceTask::Embeddings)
            .expect("at least one profile exists");
        return Ok(Some((name.as_str(), profile)));
    }

    Err(anyhow::anyhow!(
        "multiple embedding profiles are configured; pass one explicitly"
    ))
}

fn resolve_profile<'a>(
    capability: &'a EmbeddingCapabilityConfig,
    profile_name: &str,
) -> Result<&'a EmbeddingProfileConfig> {
    capability
        .inference
        .profiles
        .get(profile_name)
        .ok_or_else(|| anyhow::anyhow!("embedding profile `{profile_name}` was not found"))
}

fn ensure_local_profile(profile: &EmbeddingProfileConfig, profile_name: &str) -> Result<()> {
    if profile.task != InferenceTask::Embeddings {
        bail!("embedding profile `{profile_name}` is not an embeddings profile");
    }
    if profile.driver != BITLOOPS_EMBEDDINGS_IPC_DRIVER {
        bail!(
            "embedding profile `{profile_name}` is not a `{BITLOOPS_EMBEDDINGS_IPC_DRIVER}` profile"
        );
    }
    Ok(())
}

fn local_profile_cache_dir(profile: &EmbeddingProfileConfig) -> Result<PathBuf> {
    if let Some(cache_dir) = profile.cache_dir.clone() {
        return Ok(cache_dir);
    }

    dirs::cache_dir()
        .or_else(|| dirs::home_dir().map(|home| home.join(".cache")))
        .map(|dir| dir.join("bitloops-embeddings"))
        .context("resolving bitloops-embeddings cache directory")
}

fn embedding_capability_for_config_path(config_path: &Path) -> Result<EmbeddingCapabilityConfig> {
    let loaded = load_daemon_settings(Some(config_path))?;
    Ok(resolve_embedding_capability_from_unified(
        &loaded.settings,
        &loaded.root,
        |key| env::var(key).ok(),
    ))
}

fn selected_inference_profile_name(capability: &EmbeddingCapabilityConfig) -> Option<&str> {
    capability
        .semantic_clones
        .inference
        .code_embeddings
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .or_else(|| {
            capability
                .semantic_clones
                .inference
                .summary_embeddings
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
        })
}

#[cfg(test)]
struct PullRuntimeConfig {
    startup_timeout_secs: u64,
    request_timeout_secs: u64,
}

#[cfg(test)]
fn pull_runtime_client_config(
    _repo_root: &Path,
    _config_path: &Path,
    capability: &EmbeddingCapabilityConfig,
    profile_name: &str,
) -> PullRuntimeConfig {
    let profile = capability
        .inference
        .profiles
        .get(profile_name)
        .expect("embedding profile for timeout test");
    let runtime_name = profile
        .runtime
        .as_deref()
        .expect("runtime-backed embedding profile for timeout test");
    let runtime = capability
        .inference
        .runtimes
        .get(runtime_name)
        .expect("runtime config for timeout test");
    PullRuntimeConfig {
        startup_timeout_secs: runtime.startup_timeout_secs.max(LOCAL_PULL_TIMEOUT_SECS),
        request_timeout_secs: runtime.request_timeout_secs.max(LOCAL_PULL_TIMEOUT_SECS),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::{Cli, Commands};
    use crate::test_support::process_state::enter_process_state;
    use clap::Parser;
    use std::fs;
    use std::path::Path;
    use tempfile::TempDir;

    fn write_embedding_config(repo_root: &Path) {
        fs::write(
            repo_root.join(BITLOOPS_CONFIG_RELATIVE_PATH),
            r#"
[runtime]
local_dev = false

[semantic_clones.inference]
code_embeddings = "local"
summary_embeddings = "local"

[inference.runtimes.bitloops_embeddings]
command = "bitloops-embeddings"
args = []
startup_timeout_secs = 60
request_timeout_secs = 300

[inference.profiles.local]
task = "embeddings"
driver = "bitloops_embeddings_ipc"
runtime = "bitloops_embeddings"
model = "bge-m3"
cache_dir = ".bitloops/embeddings/models"

[inference.profiles.openai]
task = "embeddings"
driver = "openai"
model = "text-embedding-3-large"
api_key = "secret"
"#,
        )
        .expect("write config");
    }

    fn seed_repo() -> TempDir {
        let dir = TempDir::new().expect("tempdir");
        let repo_root = dir.path();
        crate::test_support::git_fixtures::init_test_repo(
            repo_root,
            "main",
            "Alice",
            "alice@example.com",
        );
        write_embedding_config(repo_root);
        dir
    }

    #[cfg(unix)]
    fn fake_runtime_command_and_args(repo_root: &Path) -> (String, Vec<String>) {
        use std::os::unix::fs::PermissionsExt;

        let script_path = repo_root.join(".bitloops/test-bin/fake-embeddings-runtime.sh");
        if let Some(parent) = script_path.parent() {
            fs::create_dir_all(parent).expect("create fake runtime dir");
        }
        let script = r#"#!/bin/sh
model_name="bge-m3"
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
    fn fake_runtime_command_and_args(repo_root: &Path) -> (String, Vec<String>) {
        let script_path = repo_root.join(".bitloops/test-bin/fake-embeddings-runtime.ps1");
        if let Some(parent) = script_path.parent() {
            fs::create_dir_all(parent).expect("create fake runtime dir");
        }
        let script = r#"
$modelName = "bge-m3"
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

    fn write_runtime_only_config(repo_root: &Path, command: &str, args: &[String]) {
        let runtime_args = args
            .iter()
            .map(|arg| format!("{arg:?}"))
            .collect::<Vec<_>>()
            .join(", ");
        fs::write(
            repo_root.join(BITLOOPS_CONFIG_RELATIVE_PATH),
            format!(
                r#"
[runtime]
local_dev = false

[inference.runtimes.bitloops_embeddings]
command = {command:?}
args = [{runtime_args}]
startup_timeout_secs = 5
request_timeout_secs = 5
"#
            ),
        )
        .expect("write runtime-only config");
    }

    #[test]
    fn embeddings_cli_parses_subcommands() {
        let parsed = Cli::try_parse_from(["bitloops", "embeddings", "doctor"])
            .expect("embeddings command should parse");
        let Some(Commands::Embeddings(args)) = parsed.command else {
            panic!("expected embeddings command");
        };
        assert!(matches!(args.command, Some(EmbeddingsCommand::Doctor(_))));
    }

    #[test]
    fn embeddings_cli_parses_pull_and_clear_cache() {
        let parsed = Cli::try_parse_from(["bitloops", "embeddings", "pull", "local_code"])
            .expect("pull should parse");
        let Some(Commands::Embeddings(args)) = parsed.command else {
            panic!("expected embeddings command");
        };
        assert!(matches!(args.command, Some(EmbeddingsCommand::Pull(_))));

        let parsed = Cli::try_parse_from(["bitloops", "embeddings", "clear-cache", "local_code"])
            .expect("clear-cache should parse");
        let Some(Commands::Embeddings(args)) = parsed.command else {
            panic!("expected embeddings command");
        };
        assert!(matches!(
            args.command,
            Some(EmbeddingsCommand::ClearCache(_))
        ));
    }

    #[test]
    fn doctor_uses_active_profile_when_not_explicit() {
        let repo = seed_repo();
        let capability = resolve_embedding_capability_config_for_repo(repo.path());
        let lines = doctor_profile(repo.path(), &capability, None).expect("doctor report");

        assert!(lines.iter().any(|line| line.contains("Profile: local")));
        assert!(
            lines
                .iter()
                .any(|line| line.contains("Kind: bitloops_embeddings_ipc"))
        );
    }

    #[test]
    fn doctor_reports_hosted_profile_sensibly() {
        let repo = seed_repo();
        let capability = resolve_embedding_capability_config_for_repo(repo.path());
        let lines =
            doctor_profile(repo.path(), &capability, Some("openai")).expect("hosted doctor report");

        assert!(lines.iter().any(|line| line.contains("Profile: openai")));
        assert!(
            lines
                .iter()
                .any(|line| line.contains("Runtime: hosted profile"))
        );
    }

    #[test]
    fn doctor_reports_disabled_when_no_profiles_exist() {
        let repo = TempDir::new().expect("tempdir");
        let capability = resolve_embedding_capability_config_for_repo(repo.path());
        let lines = doctor_profile(repo.path(), &capability, None).expect("disabled report");

        assert!(
            lines
                .iter()
                .any(|line| line.contains("Embeddings: disabled"))
        );
    }

    #[test]
    fn clear_cache_removes_local_cache_directory() {
        let repo = seed_repo();
        let capability = resolve_embedding_capability_config_for_repo(repo.path());
        let cache_dir = repo.path().join(".bitloops/embeddings/models");
        fs::create_dir_all(&cache_dir).expect("create cache dir");

        let lines =
            clear_cache_for_profile(repo.path(), &capability, "local").expect("clear cache report");

        assert!(!cache_dir.exists());
        assert!(
            lines
                .iter()
                .any(|line| line.contains("Cleared cache for profile `local`"))
        );
    }

    #[test]
    fn pull_uses_extended_timeouts_for_local_warmup() {
        let repo = seed_repo();
        let capability = resolve_embedding_capability_config_for_repo(repo.path());
        let runtime = pull_runtime_client_config(
            repo.path(),
            &repo.path().join(BITLOOPS_CONFIG_RELATIVE_PATH),
            &capability,
            "local",
        );

        assert_eq!(runtime.startup_timeout_secs, LOCAL_PULL_TIMEOUT_SECS);
        assert_eq!(runtime.request_timeout_secs, LOCAL_PULL_TIMEOUT_SECS);
    }

    #[test]
    fn pull_rejects_hosted_profiles_without_network() {
        let repo = seed_repo();
        let capability = resolve_embedding_capability_config_for_repo(repo.path());
        let err = pull_profile(repo.path(), &capability, "openai").expect_err("hosted pull");
        assert!(
            err.to_string()
                .contains("not a `bitloops_embeddings_ipc` profile")
        );
    }

    #[test]
    fn inspect_embeddings_install_state_reports_not_configured() {
        let repo = TempDir::new().expect("tempdir");
        let home = TempDir::new().expect("home dir");
        let home_path = home.path().to_string_lossy().to_string();
        let config_root = TempDir::new().expect("config tempdir");
        let config_root_value = config_root.path().to_string_lossy().into_owned();
        let _guard = enter_process_state(
            Some(repo.path()),
            &[
                ("HOME", Some(home_path.as_str())),
                ("USERPROFILE", Some(home_path.as_str())),
                (
                    "BITLOOPS_TEST_CONFIG_DIR_OVERRIDE",
                    Some(config_root_value.as_str()),
                ),
            ],
        );
        crate::test_support::git_fixtures::init_test_repo(
            repo.path(),
            "main",
            "Alice",
            "alice@example.com",
        );

        assert!(matches!(
            inspect_embeddings_install_state(repo.path()),
            EmbeddingsInstallState::NotConfigured
        ));
    }

    #[test]
    fn install_or_bootstrap_embeddings_writes_local_profile_and_warms_runtime() {
        let repo = TempDir::new().expect("tempdir");
        crate::test_support::git_fixtures::init_test_repo(
            repo.path(),
            "main",
            "Alice",
            "alice@example.com",
        );
        let (command, args) = fake_runtime_command_and_args(repo.path());
        write_runtime_only_config(repo.path(), &command, &args);

        let lines =
            install_or_bootstrap_embeddings(repo.path()).expect("install embeddings via runtime");
        let config = fs::read_to_string(repo.path().join(BITLOOPS_CONFIG_RELATIVE_PATH))
            .expect("read updated config");

        assert!(config.contains("code_embeddings = \"local_code\""));
        assert!(config.contains("[inference.profiles.local_code]"));
        assert!(config.contains("driver = \"bitloops_embeddings_ipc\""));
        assert!(
            lines
                .iter()
                .any(|line| line.contains("Configured embeddings in")),
            "expected configuration line, got: {lines:?}"
        );
        assert!(
            lines
                .iter()
                .any(|line| line.contains("Pulled embedding profile `local_code`")),
            "expected warmup line, got: {lines:?}"
        );
    }

    #[test]
    fn install_or_bootstrap_embeddings_rolls_back_when_runtime_bootstrap_fails() {
        let repo = TempDir::new().expect("tempdir");
        crate::test_support::git_fixtures::init_test_repo(
            repo.path(),
            "main",
            "Alice",
            "alice@example.com",
        );
        write_runtime_only_config(repo.path(), "definitely-missing-embeddings-runtime", &[]);
        let config_path = repo.path().join(BITLOOPS_CONFIG_RELATIVE_PATH);
        let original = fs::read_to_string(&config_path).expect("read original config");

        let err = install_or_bootstrap_embeddings(repo.path())
            .expect_err("runtime bootstrap should fail");
        let after = fs::read_to_string(&config_path).expect("read rolled-back config");

        assert_eq!(after, original);
        assert!(
            format!("{err:#}").contains("spawning python embeddings runtime"),
            "unexpected error: {err:#}"
        );
    }

    #[test]
    fn install_or_bootstrap_embeddings_preserves_existing_hosted_profile() {
        let repo = TempDir::new().expect("tempdir");
        crate::test_support::git_fixtures::init_test_repo(
            repo.path(),
            "main",
            "Alice",
            "alice@example.com",
        );
        let config_path = repo.path().join(BITLOOPS_CONFIG_RELATIVE_PATH);
        fs::write(
            &config_path,
            r#"
[runtime]
local_dev = false

[semantic_clones.inference]
code_embeddings = "openai"

[inference.profiles.openai]
task = "embeddings"
driver = "openai"
model = "text-embedding-3-large"
"#,
        )
        .expect("write hosted config");
        let original = fs::read_to_string(&config_path).expect("read original config");

        let lines =
            install_or_bootstrap_embeddings(repo.path()).expect("existing hosted profile result");
        let after = fs::read_to_string(&config_path).expect("read final config");

        assert_eq!(after, original);
        assert!(
            lines
                .iter()
                .any(|line| line.contains("skipped local runtime bootstrap")),
            "expected hosted skip line, got: {lines:?}"
        );
    }
}
