use anyhow::{Context, Result, bail};
use clap::{Args, Subcommand};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use crate::adapters::model_providers::embeddings::{
    EmbeddingInputType, EmbeddingRuntimeClientConfig, build_embedding_provider,
};
use crate::cli::enable::find_repo_root;
use crate::config::{
    BITLOOPS_CONFIG_RELATIVE_PATH, EmbeddingCapabilityConfig, EmbeddingProfileConfig,
    default_daemon_config_path, resolve_embedding_capability_config_for_repo,
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

pub fn run(args: EmbeddingsArgs) -> Result<()> {
    let Some(command) = args.command else {
        bail!(
            "missing subcommand. Use one of: `bitloops embeddings pull`, `bitloops embeddings doctor`, `bitloops embeddings clear-cache`"
        );
    };

    let repo_root = current_repo_root()?;
    let capability = resolve_embedding_capability_config_for_repo(&repo_root);

    match command {
        EmbeddingsCommand::Pull(args) => {
            let lines = pull_profile(&repo_root, &capability, &args.profile)?;
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

fn pull_profile(
    repo_root: &Path,
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

    let runtime = pull_runtime_client_config(repo_root, capability, profile_name);
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

fn doctor_profile(
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

fn clear_cache_for_profile(
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

fn runtime_client_config(
    repo_root: &Path,
    capability: &EmbeddingCapabilityConfig,
    profile_name: &str,
) -> EmbeddingRuntimeClientConfig {
    EmbeddingRuntimeClientConfig {
        command: capability.embeddings.runtime.command.clone(),
        args: capability.embeddings.runtime.args.clone(),
        startup_timeout_secs: capability.embeddings.runtime.startup_timeout_secs,
        request_timeout_secs: capability.embeddings.runtime.request_timeout_secs,
        config_path: default_daemon_config_path()
            .unwrap_or_else(|_| repo_root.join(BITLOOPS_CONFIG_RELATIVE_PATH)),
        profile_name: profile_name.to_string(),
        repo_root: Some(repo_root.to_path_buf()),
    }
}

fn pull_runtime_client_config(
    repo_root: &Path,
    capability: &EmbeddingCapabilityConfig,
    profile_name: &str,
) -> EmbeddingRuntimeClientConfig {
    let mut config = runtime_client_config(repo_root, capability, profile_name);
    config.startup_timeout_secs = config.startup_timeout_secs.max(LOCAL_PULL_TIMEOUT_SECS);
    config.request_timeout_secs = config.request_timeout_secs.max(LOCAL_PULL_TIMEOUT_SECS);
    config
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::{Cli, Commands};
    use clap::Parser;
    use std::fs;
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
        let runtime = pull_runtime_client_config(repo.path(), &capability, "local");

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
}
