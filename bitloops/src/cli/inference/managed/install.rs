use anyhow::{Context, Result, anyhow, bail};
use reqwest::blocking::Client;
use reqwest::header::{ACCEPT, USER_AGENT};
use serde::{Deserialize, Serialize};
use std::env;
use std::path::{Path, PathBuf};
use std::time::Duration;

#[cfg(test)]
use std::sync::{Arc, Mutex, OnceLock};

use crate::cli::embeddings::managed::archive::{
    ManagedEmbeddingsArchiveKind, ManagedEmbeddingsBundleEntry,
    extract_managed_embeddings_bundle_entries_from_file, write_file_atomically,
};
use crate::cli::embeddings::managed::download::download_release_asset_to_temp_file;
use crate::config::{
    prepare_daemon_inference_install, resolve_preferred_daemon_config_path_for_repo,
};

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ManagedInferenceInstallPhase {
    Queued,
    ResolvingRelease,
    DownloadingRuntime,
    ExtractingRuntime,
    RewritingRuntime,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ManagedInferenceInstallProgress {
    pub(crate) phase: ManagedInferenceInstallPhase,
    pub(crate) asset_name: Option<String>,
    pub(crate) bytes_downloaded: u64,
    pub(crate) bytes_total: Option<u64>,
    pub(crate) version: Option<String>,
    pub(crate) message: Option<String>,
}

impl Default for ManagedInferenceInstallProgress {
    fn default() -> Self {
        Self {
            phase: ManagedInferenceInstallPhase::Queued,
            asset_name: None,
            bytes_downloaded: 0,
            bytes_total: None,
            version: None,
            message: None,
        }
    }
}

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
    dyn Fn(&Path) -> Result<ManagedInferenceBinaryInstallOutcome> + Send + Sync + 'static;

#[cfg(test)]
fn managed_inference_install_hook_cell() -> &'static Mutex<Option<Arc<ManagedInferenceInstallHook>>>
{
    static MANAGED_INFERENCE_INSTALL_HOOK: OnceLock<
        Mutex<Option<Arc<ManagedInferenceInstallHook>>>,
    > = OnceLock::new();
    MANAGED_INFERENCE_INSTALL_HOOK.get_or_init(|| Mutex::new(None))
}

#[cfg(test)]
fn managed_inference_install_hook_lock() -> &'static Mutex<()> {
    static MANAGED_INFERENCE_INSTALL_HOOK_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    MANAGED_INFERENCE_INSTALL_HOOK_LOCK.get_or_init(|| Mutex::new(()))
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

pub(crate) fn install_or_bootstrap_inference_with_progress<R>(
    repo_root: &Path,
    mut report: R,
) -> Result<Vec<String>>
where
    R: FnMut(ManagedInferenceInstallProgress) -> Result<()>,
{
    let config_path = resolve_preferred_daemon_config_path_for_repo(repo_root)?;
    let plan = prepare_daemon_inference_install(&config_path)?;
    if plan.config_modified {
        report(ManagedInferenceInstallProgress {
            phase: ManagedInferenceInstallPhase::RewritingRuntime,
            message: Some(format!(
                "Preparing inference config in {}",
                config_path.display()
            )),
            ..Default::default()
        })?;
        plan.apply()?;
    }

    let ensure = ensure_managed_inference_runtime_with_config_and_progress(
        repo_root,
        Some(&config_path),
        &mut report,
    );
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

fn ensure_managed_inference_runtime_with_config(
    repo_root: &Path,
    config_path: Option<&Path>,
) -> Result<ManagedInferenceRuntimeEnsureOutcome> {
    ensure_managed_inference_runtime_with_config_and_progress(repo_root, config_path, |_progress| {
        Ok(())
    })
}

fn ensure_managed_inference_runtime_with_config_and_progress<R>(
    repo_root: &Path,
    config_path: Option<&Path>,
    mut report: R,
) -> Result<ManagedInferenceRuntimeEnsureOutcome>
where
    R: FnMut(ManagedInferenceInstallProgress) -> Result<()>,
{
    let install = install_managed_inference_binary_with_progress(repo_root, &mut report)?;
    let command_rewritten = if let Some(config_path) = config_path {
        let rewritten =
            rewrite_managed_runtime_command_if_eligible(config_path, &install.binary_path)?;
        if rewritten {
            report(ManagedInferenceInstallProgress {
                phase: ManagedInferenceInstallPhase::RewritingRuntime,
                version: Some(install.version.clone()),
                message: Some(format!(
                    "Updating inference runtime command in {}",
                    config_path.display()
                )),
                ..Default::default()
            })?;
        }
        rewritten
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
    install_or_bootstrap_inference_with_progress(repo_root, |_progress| Ok(()))
}

#[allow(dead_code)]
fn install_managed_inference_binary(
    repo_root: &Path,
) -> Result<ManagedInferenceBinaryInstallOutcome> {
    install_managed_inference_binary_with_progress(repo_root, |_progress| Ok(()))
}

fn install_managed_inference_binary_with_progress<R>(
    repo_root: &Path,
    mut report: R,
) -> Result<ManagedInferenceBinaryInstallOutcome>
where
    R: FnMut(ManagedInferenceInstallProgress) -> Result<()>,
{
    #[cfg(test)]
    if let Some(hook) = managed_inference_install_hook_cell()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
        .clone()
    {
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

    report(ManagedInferenceInstallProgress {
        phase: ManagedInferenceInstallPhase::ResolvingRelease,
        message: Some("Resolving managed `bitloops-inference` release".to_string()),
        ..Default::default()
    })?;
    let release = fetch_managed_inference_release(&release_request)?;
    install_managed_inference_binary_from_release_with_progress(&release, &mut report)
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

#[allow(dead_code)]
fn install_managed_inference_binary_from_release(
    release: &GitHubReleasePayload,
) -> Result<ManagedInferenceBinaryInstallOutcome> {
    install_managed_inference_binary_from_release_with_progress(release, |_progress| Ok(()))
}

fn install_managed_inference_binary_from_release_with_progress<R>(
    release: &GitHubReleasePayload,
    mut report: R,
) -> Result<ManagedInferenceBinaryInstallOutcome>
where
    R: FnMut(ManagedInferenceInstallProgress) -> Result<()>,
{
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
    report(ManagedInferenceInstallProgress {
        phase: ManagedInferenceInstallPhase::DownloadingRuntime,
        asset_name: Some(asset.name.clone()),
        version: Some(release.tag_name.clone()),
        message: Some(format!("Downloading `{}`", asset.name)),
        ..Default::default()
    })?;
    let client = managed_inference_http_client()?;
    let download = download_release_asset_to_temp_file(
        &client,
        &asset.browser_download_url,
        MANAGED_INFERENCE_USER_AGENT,
        "managed bitloops-inference asset",
        |downloaded, total| {
            report(ManagedInferenceInstallProgress {
                phase: ManagedInferenceInstallPhase::DownloadingRuntime,
                asset_name: Some(asset.name.clone()),
                bytes_downloaded: downloaded,
                bytes_total: total,
                version: Some(release.tag_name.clone()),
                message: Some(format!("Downloading `{}`", asset.name)),
            })
        },
    )?;
    if download.sha256_hex != expected_digest {
        bail!(
            "managed bitloops-inference asset digest mismatch for `{}`: expected {}, got {}",
            asset.name,
            expected_digest,
            download.sha256_hex
        );
    }

    report(ManagedInferenceInstallProgress {
        phase: ManagedInferenceInstallPhase::ExtractingRuntime,
        asset_name: Some(asset.name.clone()),
        bytes_downloaded: download.bytes_downloaded,
        bytes_total: Some(download.bytes_downloaded),
        version: Some(release.tag_name.clone()),
        message: Some(format!("Extracting `{}`", asset.name)),
    })?;

    let bundle_entries = extract_managed_embeddings_bundle_entries_from_file(
        download.path(),
        asset_spec.archive_kind,
        managed_inference_binary_name(),
    )
    .with_context(|| {
        format!(
            "extracting managed bitloops-inference bundle from `{}`",
            asset.name
        )
    })?;
    install_managed_inference_bundle_entries(&release.tag_name, bundle_entries)
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

fn managed_inference_http_client() -> Result<Client> {
    Client::builder()
        .timeout(Duration::from_secs(MANAGED_INFERENCE_HTTP_TIMEOUT_SECS))
        .build()
        .context("building managed bitloops-inference HTTP client")
}

fn install_managed_inference_bundle_entries(
    version: &str,
    bundle_entries: Vec<ManagedEmbeddingsBundleEntry>,
) -> Result<ManagedInferenceBinaryInstallOutcome> {
    reset_managed_inference_install_dir()?;
    let binary_path = managed_inference_binary_path()?;
    let install_dir = managed_inference_binary_dir()?;
    for entry in bundle_entries {
        let output_path = install_dir.join(&entry.relative_path);
        write_file_atomically(&output_path, &entry.bytes, entry.executable)?;
    }
    save_managed_inference_install_metadata(&ManagedInferenceInstallMetadata {
        version: version.to_string(),
        binary_path: binary_path.clone(),
    })?;

    Ok(ManagedInferenceBinaryInstallOutcome {
        version: version.to_string(),
        binary_path,
        freshly_installed: true,
    })
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
    hook: impl Fn(&Path) -> Result<ManagedInferenceBinaryInstallOutcome> + Send + Sync + 'static,
    f: impl FnOnce() -> T,
) -> T {
    let _hook_lock = managed_inference_install_hook_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let cell = managed_inference_install_hook_cell();
    {
        let mut guard = cell.lock().unwrap_or_else(|poison| poison.into_inner());
        assert!(
            guard.is_none(),
            "managed inference install hook already installed"
        );
        *guard = Some(Arc::new(hook));
    }
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f));
    *cell.lock().unwrap_or_else(|poison| poison.into_inner()) = None;
    match result {
        Ok(result) => result,
        Err(payload) => std::panic::resume_unwind(payload),
    }
}
