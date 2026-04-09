use anyhow::{Context, Result, bail};
use clap::{Args, Subcommand};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use crate::adapters::model_providers::embeddings::{
    EmbeddingInputType, EmbeddingRuntimeClientConfig, build_embedding_provider,
};
use crate::cli::enable::find_repo_root;
use crate::config::unified_config::resolve_embedding_capability_from_unified;
use crate::config::{
    BITLOOPS_CONFIG_RELATIVE_PATH, DaemonEmbeddingsInstallMode, EmbeddingCapabilityConfig,
    EmbeddingProfileConfig, load_daemon_settings, prepare_daemon_embeddings_install,
    resolve_daemon_config_path_for_repo, resolve_embedding_capability_config_for_repo,
};

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

const LOCAL_PULL_TIMEOUT_SECS: u64 = 300;

fn current_repo_root() -> Result<PathBuf> {
    let cwd = env::current_dir().context("getting current directory")?;
    find_repo_root(&cwd)
}

pub(crate) fn install_or_bootstrap_embeddings(repo_root: &Path) -> Result<Vec<String>> {
    let config_path = resolve_daemon_config_path_for_repo(repo_root)?;
    let plan = prepare_daemon_embeddings_install(&config_path)?;

    match plan.mode {
        DaemonEmbeddingsInstallMode::SkipHosted => {
            let profile_kind = plan
                .profile_kind
                .as_deref()
                .map(|kind| format!(" (kind `{kind}`)"))
                .unwrap_or_default();
            return Ok(vec![format!(
                "Embeddings are already configured via profile `{}`{}; skipped local runtime bootstrap.",
                plan.profile_name, profile_kind
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
    let Ok(capability) = embedding_capability_for_config_path(&config_path) else {
        return EmbeddingsInstallState::NotConfigured;
    };
    let Some(profile_name) = capability.semantic_clones.embedding_profile.clone() else {
        return EmbeddingsInstallState::NotConfigured;
    };
    let kind = capability
        .embeddings
        .profiles
        .get(&profile_name)
        .map(|profile| profile.kind.clone());
    if kind.as_deref() == Some("local_fastembed") {
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
    config_path: &Path,
    capability: &EmbeddingCapabilityConfig,
    profile_name: &str,
) -> Result<Vec<String>> {
    let profile = resolve_profile(capability, profile_name)?;
    ensure_local_profile(profile, profile_name)?;

    let cache_dir = local_profile_cache_dir(repo_root, profile);
    if let Some(parent) = cache_dir.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("creating cache parent {}", parent.display()))?;
    }

    let runtime = pull_runtime_client_config(repo_root, config_path, capability, profile_name);
    let provider = build_embedding_provider(&runtime)?;
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
    repo_root: &Path,
    capability: &EmbeddingCapabilityConfig,
    profile_name: Option<&str>,
) -> Result<Vec<String>> {
    let Some((profile_name, profile)) = resolve_doctor_target(capability, profile_name)? else {
        return Ok(vec![
            "Embeddings: disabled".to_string(),
            "No embedding profile is configured in [semantic_clones].".to_string(),
        ]);
    };

    let mut lines = vec![
        format!("Profile: {profile_name}"),
        format!("Kind: {}", profile.kind),
    ];

    if let Some(model) = profile.model.as_deref() {
        lines.push(format!("Model: {model}"));
    }
    if let Some(base_url) = profile.base_url.as_deref() {
        lines.push(format!("Base URL: {base_url}"));
    }

    match profile.kind.as_str() {
        "local_fastembed" => {
            let cache_dir = local_profile_cache_dir(repo_root, profile);
            lines.push(format!("Cache directory: {}", cache_dir.display()));
            lines.push(format!(
                "Cache status: {}",
                if cache_dir.exists() {
                    "present"
                } else {
                    "missing"
                }
            ));
            lines.push(format!(
                "Runtime: {}",
                capability.embeddings.runtime.command
            ));
        }
        "openai" | "voyage" => {
            lines.push("Cache directory: not applicable".to_string());
            lines.push("Runtime: hosted profile".to_string());
        }
        other => {
            lines.push(format!("Cache directory: unsupported for kind `{other}`"));
        }
    }

    Ok(lines)
}

pub(crate) fn clear_cache_for_profile(
    repo_root: &Path,
    capability: &EmbeddingCapabilityConfig,
    profile_name: &str,
) -> Result<Vec<String>> {
    let profile = resolve_profile(capability, profile_name)?;
    ensure_local_profile(profile, profile_name)?;
    let cache_dir = local_profile_cache_dir(repo_root, profile);

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
    if capability.embeddings.profiles.is_empty() {
        return Ok(None);
    }

    if let Some(profile_name) = profile_name {
        let profile = resolve_profile(capability, profile_name)?;
        return Ok(Some((profile_name, profile)));
    }

    if let Some(active_profile) = capability.semantic_clones.embedding_profile.as_deref() {
        let profile = resolve_profile(capability, active_profile)?;
        return Ok(Some((active_profile, profile)));
    }

    if capability.embeddings.profiles.len() == 1 {
        let (name, profile) = capability
            .embeddings
            .profiles
            .iter()
            .next()
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
        .embeddings
        .profiles
        .get(profile_name)
        .ok_or_else(|| anyhow::anyhow!("embedding profile `{profile_name}` was not found"))
}

fn ensure_local_profile(profile: &EmbeddingProfileConfig, profile_name: &str) -> Result<()> {
    if profile.kind != "local_fastembed" {
        bail!("embedding profile `{profile_name}` is not a local_fastembed profile");
    }
    Ok(())
}

fn local_profile_cache_dir(repo_root: &Path, profile: &EmbeddingProfileConfig) -> PathBuf {
    profile
        .cache_dir
        .clone()
        .unwrap_or_else(|| repo_root.join(".bitloops/embeddings/models"))
}

fn embedding_capability_for_config_path(config_path: &Path) -> Result<EmbeddingCapabilityConfig> {
    let loaded = load_daemon_settings(Some(config_path))?;
    Ok(resolve_embedding_capability_from_unified(
        &loaded.settings,
        &loaded.root,
        |key| env::var(key).ok(),
    ))
}

fn runtime_client_config(
    repo_root: &Path,
    config_path: &Path,
    capability: &EmbeddingCapabilityConfig,
    profile_name: &str,
) -> EmbeddingRuntimeClientConfig {
    EmbeddingRuntimeClientConfig {
        command: capability.embeddings.runtime.command.clone(),
        args: capability.embeddings.runtime.args.clone(),
        startup_timeout_secs: capability.embeddings.runtime.startup_timeout_secs,
        request_timeout_secs: capability.embeddings.runtime.request_timeout_secs,
        config_path: config_path.to_path_buf(),
        profile_name: profile_name.to_string(),
        repo_root: Some(repo_root.to_path_buf()),
    }
}

fn pull_runtime_client_config(
    repo_root: &Path,
    config_path: &Path,
    capability: &EmbeddingCapabilityConfig,
    profile_name: &str,
) -> EmbeddingRuntimeClientConfig {
    let mut config = runtime_client_config(repo_root, config_path, capability, profile_name);
    config.startup_timeout_secs = config.startup_timeout_secs.max(LOCAL_PULL_TIMEOUT_SECS);
    config.request_timeout_secs = config.request_timeout_secs.max(LOCAL_PULL_TIMEOUT_SECS);
    config
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::{Cli, Commands};
    use crate::test_support::process_state::with_process_state;
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

[semantic_clones]
embedding_profile = "local"

[embeddings.runtime]
command = "bitloops-embeddings"
args = []
startup_timeout_secs = 10
request_timeout_secs = 60

[embeddings.profiles.local]
kind = "local_fastembed"
model = "jinaai/jina-embeddings-v2-base-code"
cache_dir = ".bitloops/embeddings/models"

[embeddings.profiles.openai]
kind = "openai"
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
profile_name=fake
while [ $# -gt 0 ]; do
  case "$1" in
    --profile)
      profile_name=$2
      shift 2
      ;;
    *)
      shift
      ;;
  esac
done
while IFS= read -r line; do
  req_id=$(printf '%s\n' "$line" | sed -n 's/.*"request_id":"\([^"]*\)".*/\1/p')
  case "$line" in
    *'"type":"describe"'*)
      printf '{"type":"describe","request_id":"%s","protocol_version":1,"runtime":{"protocol_version":1,"runtime_name":"bitloops-embeddings","runtime_version":"test","profile_name":"%s","provider":{"kind":"local_fastembed","provider_name":"local_fastembed","model_name":"test-model","output_dimension":3,"cache_dir":null}}}\n' "$req_id" "$profile_name"
      ;;
    *'"type":"embed_batch"'*)
      printf '{"type":"embed_batch","request_id":"%s","protocol_version":1,"vectors":[{"index":0,"values":[0.1,0.2,0.3]}]}\n' "$req_id"
      ;;
    *'"type":"shutdown"'*)
      printf '{"type":"shutdown","request_id":"%s","protocol_version":1,"accepted":true}\n' "$req_id"
      exit 0
      ;;
    *)
      printf '{"type":"error","request_id":"%s","code":"runtime_error","message":"unexpected request"}\n' "$req_id"
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
$profileName = "fake"
for ($i = 0; $i -lt $args.Length; $i++) {
  if ($args[$i] -eq "--profile" -and ($i + 1) -lt $args.Length) {
    $profileName = $args[$i + 1]
    break
  }
}
$stdin = [Console]::In
while (($line = $stdin.ReadLine()) -ne $null) {
  if ([string]::IsNullOrWhiteSpace($line)) { continue }
  $request = $line | ConvertFrom-Json
  switch ($request.type) {
    "describe" {
      $response = @{
        type = "describe"
        request_id = $request.request_id
        protocol_version = 1
        runtime = @{
          protocol_version = 1
          runtime_name = "bitloops-embeddings"
          runtime_version = "test"
          profile_name = $profileName
          provider = @{
            kind = "local_fastembed"
            provider_name = "local_fastembed"
            model_name = "test-model"
            output_dimension = 3
            cache_dir = $null
          }
        }
      }
    }
    "embed_batch" {
      $response = @{
        type = "embed_batch"
        request_id = $request.request_id
        protocol_version = 1
        vectors = @(@{
          index = 0
          values = @(0.1, 0.2, 0.3)
        })
      }
    }
    "shutdown" {
      $response = @{
        type = "shutdown"
        request_id = $request.request_id
        protocol_version = 1
        accepted = $true
      }
      $response | ConvertTo-Json -Compress
      break
    }
    default {
      $response = @{
        type = "error"
        request_id = $request.request_id
        code = "runtime_error"
        message = "unexpected request"
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

[embeddings.runtime]
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
        let parsed = Cli::try_parse_from(["bitloops", "embeddings", "pull", "local"])
            .expect("pull should parse");
        let Some(Commands::Embeddings(args)) = parsed.command else {
            panic!("expected embeddings command");
        };
        assert!(matches!(args.command, Some(EmbeddingsCommand::Pull(_))));

        let parsed = Cli::try_parse_from(["bitloops", "embeddings", "clear-cache", "local"])
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
                .any(|line| line.contains("Kind: local_fastembed"))
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
        assert!(err.to_string().contains("not a local_fastembed"));
    }

    #[test]
    fn inspect_embeddings_install_state_reports_not_configured() {
        let repo = TempDir::new().expect("tempdir");
        crate::test_support::git_fixtures::init_test_repo(
            repo.path(),
            "main",
            "Alice",
            "alice@example.com",
        );

        let config_root = TempDir::new().expect("config tempdir");
        let config_root_value = config_root.path().to_string_lossy().into_owned();

        with_process_state(
            Some(repo.path()),
            &[(
                "BITLOOPS_TEST_CONFIG_DIR_OVERRIDE",
                Some(config_root_value.as_str()),
            )],
            || {
                assert!(matches!(
                    inspect_embeddings_install_state(repo.path()),
                    EmbeddingsInstallState::NotConfigured
                ));
            },
        );
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

        assert!(config.contains("embedding_profile = \"local\""));
        assert!(config.contains("[embeddings.profiles.local]"));
        assert!(config.contains("kind = \"local_fastembed\""));
        assert!(
            lines
                .iter()
                .any(|line| line.contains("Configured embeddings in")),
            "expected configuration line, got: {lines:?}"
        );
        assert!(
            lines
                .iter()
                .any(|line| line.contains("Pulled embedding profile `local`")),
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
            format!("{err:#}").contains("spawning embeddings runtime"),
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

[semantic_clones]
embedding_profile = "openai"

[embeddings.profiles.openai]
kind = "openai"
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
