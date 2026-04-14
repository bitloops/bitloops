use anyhow::{Context, Result, anyhow, bail};
use reqwest::blocking::Client;
use reqwest::header::{ACCEPT, USER_AGENT};
use serde::{Deserialize, Serialize};
use std::env;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;

#[cfg(test)]
use std::cell::RefCell;
#[cfg(test)]
use std::rc::Rc;

use crate::cli::embeddings::managed::archive::{
    ManagedEmbeddingsArchiveKind, extract_managed_embeddings_bundle_entries, sha256_hex,
    write_file_atomically,
};
use crate::config::{prepare_daemon_inference_install, resolve_daemon_config_path_for_repo};

use super::config::{
    MANAGED_INFERENCE_VERSION_OVERRIDE_ENV, ManagedInferenceInstallMetadata,
    ManagedInferenceMetadataError, load_managed_inference_install_metadata,
    managed_inference_binary_dir, managed_inference_binary_name, managed_inference_binary_path,
    managed_inference_bundle_is_complete, reset_managed_inference_install_dir,
    rewrite_managed_runtime_command_if_eligible, save_managed_inference_install_metadata,
};

const MANAGED_INFERENCE_RELEASES_API_BASE: &str =
    "https://api.github.com/repos/bitloops/bitloops-inference";
const MANAGED_INFERENCE_HTTP_TIMEOUT_SECS: u64 = 300;
const MANAGED_INFERENCE_USER_AGENT: &str = "bitloops-cli";

#[derive(Debug, Clone, PartialEq, Eq)]
enum ManagedInferenceReleaseRequest {
    Latest,
    Tag(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ManagedInferenceBinaryInstallOutcome {
    pub(crate) version: String,
    pub(crate) binary_path: PathBuf,
    pub(crate) freshly_installed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ManagedInferenceRuntimeEnsureOutcome {
    install: ManagedInferenceBinaryInstallOutcome,
    command_rewritten: bool,
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
struct ManagedInferenceAssetSpec {
    asset_name: String,
    archive_kind: ManagedEmbeddingsArchiveKind,
}

#[cfg(test)]
type ManagedInferenceInstallHook =
    dyn Fn(&Path) -> Result<ManagedInferenceBinaryInstallOutcome> + 'static;

#[cfg(test)]
thread_local! {
    static MANAGED_INFERENCE_INSTALL_HOOK: RefCell<Option<Rc<ManagedInferenceInstallHook>>> =
        RefCell::new(None);
}

#[allow(dead_code)]
pub(crate) fn ensure_managed_inference_runtime(
    repo_root: &Path,
    config_path: Option<&Path>,
) -> Result<Vec<String>> {
    let outcome = ensure_managed_inference_runtime_with_config(repo_root, config_path)?;
    Ok(format_managed_inference_runtime_lines(
        &outcome,
        config_path,
    ))
}

fn ensure_managed_inference_runtime_with_config(
    repo_root: &Path,
    config_path: Option<&Path>,
) -> Result<ManagedInferenceRuntimeEnsureOutcome> {
    let install = install_managed_inference_binary(repo_root)?;
    let command_rewritten = if let Some(config_path) = config_path {
        rewrite_managed_runtime_command_if_eligible(config_path, &install.binary_path)?
    } else {
        false
    };
    Ok(ManagedInferenceRuntimeEnsureOutcome {
        install,
        command_rewritten,
    })
}

fn format_managed_inference_runtime_lines(
    outcome: &ManagedInferenceRuntimeEnsureOutcome,
    config_path: Option<&Path>,
) -> Vec<String> {
    let mut lines = vec![if outcome.install.freshly_installed {
        format!(
            "Installed managed standalone `bitloops-inference` runtime {}.",
            outcome.install.version
        )
    } else {
        format!(
            "Managed standalone `bitloops-inference` runtime {} already installed.",
            outcome.install.version
        )
    }];
    lines.push(format!(
        "Binary path: {}",
        outcome.install.binary_path.display()
    ));

    if let Some(config_path) = config_path
        && outcome.command_rewritten
    {
        lines.push(format!(
            "Updated inference runtime command and args in {}.",
            config_path.display()
        ));
    }

    lines
}

pub(crate) fn install_or_bootstrap_inference(repo_root: &Path) -> Result<Vec<String>> {
    let config_path = resolve_daemon_config_path_for_repo(repo_root)?;
    let plan = prepare_daemon_inference_install(&config_path)?;
    if plan.config_modified {
        plan.apply()?;
    }

    let ensure = ensure_managed_inference_runtime_with_config(repo_root, Some(&config_path));
    match ensure {
        Ok(outcome) => {
            let mut lines = format_managed_inference_runtime_lines(&outcome, Some(&config_path));
            if plan.config_modified {
                lines.insert(
                    0,
                    format!("Configured inference runtime in {}.", config_path.display()),
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

fn install_managed_inference_binary(
    repo_root: &Path,
) -> Result<ManagedInferenceBinaryInstallOutcome> {
    #[cfg(test)]
    if let Some(hook) = MANAGED_INFERENCE_INSTALL_HOOK.with(|cell| cell.borrow().clone()) {
        return hook(repo_root);
    }

    let _ = repo_root;
    let binary_path = managed_inference_binary_path()?;
    let install_metadata = load_managed_inference_install_metadata_for_install()?;
    let release_request = managed_inference_release_request();
    let required_version = match &release_request {
        ManagedInferenceReleaseRequest::Latest => None,
        ManagedInferenceReleaseRequest::Tag(version) => Some(version.as_str()),
    };
    if let Some(outcome) = installed_managed_inference_outcome(
        install_metadata.as_ref(),
        &binary_path,
        required_version,
    ) {
        return Ok(outcome);
    }

    let release = fetch_managed_inference_release(&release_request)?;
    install_managed_inference_binary_from_release(&release)
}

fn load_managed_inference_install_metadata_for_install()
-> Result<Option<ManagedInferenceInstallMetadata>> {
    match load_managed_inference_install_metadata() {
        Ok(metadata) => Ok(metadata),
        Err(ManagedInferenceMetadataError::Parse { path, source }) => {
            eprintln!(
                "[bitloops] Warning: invalid managed inference metadata at {}: {}. Ignoring cached metadata and reinstalling the managed runtime.",
                path.display(),
                source
            );
            Ok(None)
        }
        Err(err) => Err(err.into()),
    }
}

fn install_managed_inference_binary_from_release(
    release: &GitHubReleasePayload,
) -> Result<ManagedInferenceBinaryInstallOutcome> {
    let asset_spec = managed_inference_asset_spec(&release.tag_name)?;
    let asset = release
        .assets
        .iter()
        .find(|asset| asset.name == asset_spec.asset_name)
        .ok_or_else(|| {
            anyhow!(
                "managed bitloops-inference release `{}` did not contain asset `{}`",
                release.tag_name,
                asset_spec.asset_name
            )
        })?;
    let expected_digest = parse_sha256_digest(asset.digest.as_deref())?;
    let archive_bytes = download_managed_inference_asset(&asset.browser_download_url)?;

    let actual_digest = sha256_hex(&archive_bytes);
    if actual_digest != expected_digest {
        bail!(
            "managed bitloops-inference asset digest mismatch for `{}`: expected {}, got {}",
            asset.name,
            expected_digest,
            actual_digest
        );
    }

    let binary_name = managed_inference_binary_name();
    let bundle_entries = extract_managed_embeddings_bundle_entries(
        archive_bytes.as_ref(),
        asset_spec.archive_kind,
        binary_name,
    )
    .with_context(|| {
        format!(
            "extracting managed bitloops-inference bundle from `{}`",
            asset.name
        )
    })?;
    reset_managed_inference_install_dir()?;
    let binary_path = managed_inference_binary_path()?;
    let install_dir = managed_inference_binary_dir()?;
    for entry in bundle_entries {
        let output_path = install_dir.join(&entry.relative_path);
        write_file_atomically(&output_path, &entry.bytes, entry.executable)?;
    }
    save_managed_inference_install_metadata(&ManagedInferenceInstallMetadata {
        version: release.tag_name.clone(),
        binary_path: binary_path.clone(),
    })?;

    Ok(ManagedInferenceBinaryInstallOutcome {
        version: release.tag_name.clone(),
        binary_path,
        freshly_installed: true,
    })
}

fn managed_inference_target_version() -> String {
    env::var(MANAGED_INFERENCE_VERSION_OVERRIDE_ENV)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_default()
}

fn managed_inference_release_request() -> ManagedInferenceReleaseRequest {
    let version = managed_inference_target_version();
    if version.is_empty() {
        ManagedInferenceReleaseRequest::Latest
    } else {
        ManagedInferenceReleaseRequest::Tag(version)
    }
}

fn managed_inference_asset_spec(version: &str) -> Result<ManagedInferenceAssetSpec> {
    managed_inference_asset_spec_for(env::consts::OS, env::consts::ARCH, version)
}

fn managed_inference_asset_spec_for(
    os: &str,
    arch: &str,
    version: &str,
) -> Result<ManagedInferenceAssetSpec> {
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
        _ => bail!("managed bitloops-inference install is not supported on {os}/{arch}"),
    };

    Ok(ManagedInferenceAssetSpec {
        asset_name: format!("bitloops-inference-{version}-{target_triple}.{extension}"),
        archive_kind,
    })
}

fn installed_managed_inference_outcome(
    metadata: Option<&ManagedInferenceInstallMetadata>,
    binary_path: &Path,
    required_version: Option<&str>,
) -> Option<ManagedInferenceBinaryInstallOutcome> {
    let metadata = metadata?;
    if metadata.binary_path != binary_path || !managed_inference_bundle_is_complete(binary_path) {
        return None;
    }
    if let Some(required_version) = required_version
        && metadata.version != required_version
    {
        return None;
    }

    Some(ManagedInferenceBinaryInstallOutcome {
        version: metadata.version.clone(),
        binary_path: binary_path.to_path_buf(),
        freshly_installed: false,
    })
}

fn fetch_managed_inference_release(
    request: &ManagedInferenceReleaseRequest,
) -> Result<GitHubReleasePayload> {
    let release_path = match request {
        ManagedInferenceReleaseRequest::Latest => "latest".to_string(),
        ManagedInferenceReleaseRequest::Tag(version) => format!("tags/{version}"),
    };
    let url = format!("{MANAGED_INFERENCE_RELEASES_API_BASE}/releases/{release_path}");
    managed_inference_http_client()?
        .get(url)
        .header(ACCEPT, "application/vnd.github+json")
        .header(USER_AGENT, MANAGED_INFERENCE_USER_AGENT)
        .send()
        .context("fetching managed bitloops-inference release metadata")?
        .error_for_status()
        .context("fetching managed bitloops-inference release metadata")?
        .json::<GitHubReleasePayload>()
        .context("parsing managed bitloops-inference release metadata")
}

fn download_managed_inference_asset(url: &str) -> Result<Vec<u8>> {
    let mut response = managed_inference_http_client()?
        .get(url)
        .header(ACCEPT, "application/octet-stream")
        .header(USER_AGENT, MANAGED_INFERENCE_USER_AGENT)
        .send()
        .with_context(|| format!("downloading managed bitloops-inference asset from {url}"))?
        .error_for_status()
        .with_context(|| format!("downloading managed bitloops-inference asset from {url}"))?;
    let mut bytes = Vec::new();
    let mut chunk = [0_u8; 64 * 1024];
    loop {
        let read = response
            .read(&mut chunk)
            .context("reading managed bitloops-inference asset bytes")?;
        if read == 0 {
            break;
        }
        bytes.extend_from_slice(&chunk[..read]);
    }
    Ok(bytes)
}

fn managed_inference_http_client() -> Result<Client> {
    Client::builder()
        .timeout(Duration::from_secs(MANAGED_INFERENCE_HTTP_TIMEOUT_SECS))
        .build()
        .context("building managed bitloops-inference HTTP client")
}

fn parse_sha256_digest(digest: Option<&str>) -> Result<String> {
    let digest = digest
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .context("managed bitloops-inference release asset is missing a digest")?;
    let digest = digest.strip_prefix("sha256:").unwrap_or(digest);
    if digest.len() != 64 || !digest.chars().all(|char| char.is_ascii_hexdigit()) {
        bail!("managed bitloops-inference release asset digest is malformed");
    }
    Ok(digest.to_ascii_lowercase())
}

#[cfg(test)]
pub(crate) fn with_managed_inference_install_hook<T>(
    hook: impl Fn(&Path) -> Result<ManagedInferenceBinaryInstallOutcome> + 'static,
    f: impl FnOnce() -> T,
) -> T {
    MANAGED_INFERENCE_INSTALL_HOOK.with(|cell| {
        assert!(
            cell.borrow().is_none(),
            "managed inference install hook already installed"
        );
        *cell.borrow_mut() = Some(Rc::new(hook));
    });
    let result = f();
    MANAGED_INFERENCE_INSTALL_HOOK.with(|cell| {
        *cell.borrow_mut() = None;
    });
    result
}
