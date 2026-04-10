use anyhow::{Context, Result, anyhow, bail};
use reqwest::blocking::Client;
use reqwest::header::{ACCEPT, USER_AGENT};
use serde::{Deserialize, Serialize};
use std::env;
use std::path::{Path, PathBuf};
use std::time::Duration;

#[cfg(test)]
use std::cell::RefCell;
#[cfg(test)]
use std::rc::Rc;

use crate::config::{
    DaemonEmbeddingsInstallMode, prepare_daemon_embeddings_install,
    resolve_daemon_config_path_for_repo,
};

use super::super::profiles::{embedding_capability_for_config_path, pull_profile_with_config_path};
use super::archive::{
    ManagedEmbeddingsArchiveKind, extract_managed_embeddings_bundle_entries, sha256_hex,
    write_file_atomically,
};
use super::config::{
    DEFAULT_MANAGED_EMBEDDINGS_VERSION, MANAGED_EMBEDDINGS_VERSION_OVERRIDE_ENV,
    ManagedEmbeddingsInstallMetadata, load_managed_embeddings_install_metadata,
    managed_embeddings_binary_dir, managed_embeddings_binary_name, managed_embeddings_binary_path,
    managed_embeddings_bundle_is_complete, reset_managed_embeddings_install_dir,
    rewrite_managed_runtime_command_if_eligible, save_managed_embeddings_install_metadata,
};

const MANAGED_EMBEDDINGS_RELEASES_API_BASE: &str =
    "https://api.github.com/repos/bitloops/bitloops-embeddings";
const MANAGED_EMBEDDINGS_HTTP_TIMEOUT_SECS: u64 = 300;
const MANAGED_EMBEDDINGS_USER_AGENT: &str = "bitloops-cli";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ManagedEmbeddingsBinaryInstallOutcome {
    pub(crate) version: String,
    pub(crate) binary_path: PathBuf,
    pub(crate) freshly_installed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct GitHubReleasePayload {
    tag_name: String,
    assets: Vec<GitHubReleaseAssetPayload>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct GitHubReleaseAssetPayload {
    name: String,
    digest: Option<String>,
    browser_download_url: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ManagedEmbeddingsAssetSpec {
    pub(crate) asset_name: String,
    pub(crate) archive_kind: ManagedEmbeddingsArchiveKind,
}

#[cfg(test)]
type ManagedEmbeddingsInstallHook =
    dyn Fn(&Path) -> Result<ManagedEmbeddingsBinaryInstallOutcome> + 'static;

#[cfg(test)]
thread_local! {
    static MANAGED_EMBEDDINGS_INSTALL_HOOK: RefCell<Option<Rc<ManagedEmbeddingsInstallHook>>> =
        RefCell::new(None);
}

pub(crate) fn ensure_managed_embeddings_runtime(
    repo_root: &Path,
    config_path: Option<&Path>,
) -> Result<Vec<String>> {
    let outcome = install_managed_embeddings_binary(repo_root)?;
    let mut lines = vec![if outcome.freshly_installed {
        format!(
            "Installed managed standalone `bitloops-embeddings` runtime {}.",
            outcome.version
        )
    } else {
        format!(
            "Managed standalone `bitloops-embeddings` runtime {} already installed.",
            outcome.version
        )
    }];
    lines.push(format!("Binary path: {}", outcome.binary_path.display()));

    if let Some(config_path) = config_path
        && rewrite_managed_runtime_command_if_eligible(config_path, &outcome.binary_path)?
    {
        lines.push(format!(
            "Updated embeddings runtime command and args in {}.",
            config_path.display()
        ));
    }

    Ok(lines)
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

fn install_managed_embeddings_binary(
    repo_root: &Path,
) -> Result<ManagedEmbeddingsBinaryInstallOutcome> {
    #[cfg(test)]
    if let Some(hook) = MANAGED_EMBEDDINGS_INSTALL_HOOK.with(|cell| cell.borrow().clone()) {
        return hook(repo_root);
    }

    let _ = repo_root;
    let version = managed_embeddings_target_version();
    let binary_path = managed_embeddings_binary_path()?;
    if let Some(metadata) = load_managed_embeddings_install_metadata()?
        && metadata.version == version
        && metadata.binary_path == binary_path
        && managed_embeddings_bundle_is_complete(&binary_path)
    {
        return Ok(ManagedEmbeddingsBinaryInstallOutcome {
            version,
            binary_path,
            freshly_installed: false,
        });
    }

    let asset_spec = managed_embeddings_asset_spec(&version)?;
    let release = fetch_managed_embeddings_release(&version)?;
    let asset = release
        .assets
        .iter()
        .find(|asset| asset.name == asset_spec.asset_name)
        .ok_or_else(|| {
            anyhow!(
                "managed bitloops-embeddings release `{}` did not contain asset `{}`",
                release.tag_name,
                asset_spec.asset_name
            )
        })?;
    let expected_digest = parse_sha256_digest(asset.digest.as_deref())?;
    let archive_bytes = download_managed_embeddings_asset(&asset.browser_download_url)?;

    install_managed_embeddings_binary_from_release_bytes(
        &release.tag_name,
        &asset.name,
        asset_spec.archive_kind,
        &expected_digest,
        archive_bytes.as_ref(),
    )
}

pub(crate) fn install_managed_embeddings_binary_from_release_bytes(
    version: &str,
    asset_name: &str,
    archive_kind: ManagedEmbeddingsArchiveKind,
    expected_digest: &str,
    archive_bytes: &[u8],
) -> Result<ManagedEmbeddingsBinaryInstallOutcome> {
    let actual_digest = sha256_hex(archive_bytes);
    if actual_digest != expected_digest {
        bail!(
            "managed bitloops-embeddings asset digest mismatch for `{asset_name}`: expected {expected_digest}, got {actual_digest}"
        );
    }

    let binary_name = managed_embeddings_binary_name();
    let bundle_entries =
        extract_managed_embeddings_bundle_entries(archive_bytes, archive_kind, binary_name)
            .with_context(|| {
                format!("extracting managed bitloops-embeddings bundle from `{asset_name}`")
            })?;
    reset_managed_embeddings_install_dir()?;
    let binary_path = managed_embeddings_binary_path()?;
    let install_dir = managed_embeddings_binary_dir()?;
    for entry in bundle_entries {
        let output_path = install_dir.join(&entry.relative_path);
        write_file_atomically(&output_path, &entry.bytes, entry.executable)?;
    }
    save_managed_embeddings_install_metadata(&ManagedEmbeddingsInstallMetadata {
        version: version.to_string(),
        binary_path: binary_path.clone(),
    })?;

    Ok(ManagedEmbeddingsBinaryInstallOutcome {
        version: version.to_string(),
        binary_path,
        freshly_installed: true,
    })
}

pub(crate) fn managed_embeddings_asset_spec_for(
    os: &str,
    arch: &str,
    version: &str,
) -> Result<ManagedEmbeddingsAssetSpec> {
    let (target_triple, archive_kind, extension) = match (os, arch) {
        ("macos", "aarch64") => (
            "aarch64-apple-darwin",
            ManagedEmbeddingsArchiveKind::Zip,
            "zip",
        ),
        ("macos", "x86_64") => (
            "x86_64-apple-darwin",
            ManagedEmbeddingsArchiveKind::Zip,
            "zip",
        ),
        ("linux", "aarch64") => (
            "aarch64-unknown-linux-gnu",
            ManagedEmbeddingsArchiveKind::TarXz,
            "tar.xz",
        ),
        ("linux", "x86_64") => (
            "x86_64-unknown-linux-gnu",
            ManagedEmbeddingsArchiveKind::TarXz,
            "tar.xz",
        ),
        ("windows", "x86_64") => (
            "x86_64-pc-windows-msvc",
            ManagedEmbeddingsArchiveKind::Zip,
            "zip",
        ),
        _ => {
            bail!("managed bitloops-embeddings install is not supported on {os}/{arch}")
        }
    };

    Ok(ManagedEmbeddingsAssetSpec {
        asset_name: format!("bitloops-embeddings-{version}-{target_triple}.{extension}"),
        archive_kind,
    })
}

fn managed_embeddings_target_version() -> String {
    env::var(MANAGED_EMBEDDINGS_VERSION_OVERRIDE_ENV)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| DEFAULT_MANAGED_EMBEDDINGS_VERSION.to_string())
}

fn managed_embeddings_asset_spec(version: &str) -> Result<ManagedEmbeddingsAssetSpec> {
    managed_embeddings_asset_spec_for(env::consts::OS, env::consts::ARCH, version)
}

fn fetch_managed_embeddings_release(version: &str) -> Result<GitHubReleasePayload> {
    let url = format!("{MANAGED_EMBEDDINGS_RELEASES_API_BASE}/releases/tags/{version}");
    managed_embeddings_http_client()?
        .get(url)
        .header(ACCEPT, "application/vnd.github+json")
        .header(USER_AGENT, MANAGED_EMBEDDINGS_USER_AGENT)
        .send()
        .context("fetching managed bitloops-embeddings release metadata")?
        .error_for_status()
        .context("fetching managed bitloops-embeddings release metadata")?
        .json::<GitHubReleasePayload>()
        .context("parsing managed bitloops-embeddings release metadata")
}

fn download_managed_embeddings_asset(url: &str) -> Result<Vec<u8>> {
    managed_embeddings_http_client()?
        .get(url)
        .header(ACCEPT, "application/octet-stream")
        .header(USER_AGENT, MANAGED_EMBEDDINGS_USER_AGENT)
        .send()
        .with_context(|| format!("downloading managed bitloops-embeddings asset from {url}"))?
        .error_for_status()
        .with_context(|| format!("downloading managed bitloops-embeddings asset from {url}"))?
        .bytes()
        .context("reading managed bitloops-embeddings asset bytes")
        .map(|bytes| bytes.to_vec())
}

fn managed_embeddings_http_client() -> Result<Client> {
    Client::builder()
        .timeout(Duration::from_secs(MANAGED_EMBEDDINGS_HTTP_TIMEOUT_SECS))
        .build()
        .context("building managed bitloops-embeddings HTTP client")
}

fn parse_sha256_digest(digest: Option<&str>) -> Result<String> {
    let digest = digest
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .context("managed bitloops-embeddings release asset is missing a digest")?;
    let digest = digest.strip_prefix("sha256:").unwrap_or(digest);
    if digest.len() != 64 || !digest.chars().all(|char| char.is_ascii_hexdigit()) {
        bail!("managed bitloops-embeddings release asset digest is malformed");
    }
    Ok(digest.to_ascii_lowercase())
}

#[cfg(test)]
pub(crate) fn with_managed_embeddings_install_hook<T>(
    hook: impl Fn(&Path) -> Result<ManagedEmbeddingsBinaryInstallOutcome> + 'static,
    f: impl FnOnce() -> T,
) -> T {
    MANAGED_EMBEDDINGS_INSTALL_HOOK.with(|cell| {
        assert!(
            cell.borrow().is_none(),
            "managed embeddings install hook already installed"
        );
        *cell.borrow_mut() = Some(Rc::new(hook));
    });
    let result = f();
    MANAGED_EMBEDDINGS_INSTALL_HOOK.with(|cell| {
        *cell.borrow_mut() = None;
    });
    result
}
