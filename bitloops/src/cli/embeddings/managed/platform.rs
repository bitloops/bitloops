use anyhow::{Context, Result, anyhow, bail};
use reqwest::blocking::Client;
use reqwest::header::{ACCEPT, USER_AGENT};
use serde::{Deserialize, Serialize};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

#[cfg(test)]
use std::cell::RefCell;
#[cfg(test)]
use std::rc::Rc;
use toml_edit::{DocumentMut, Item};

use crate::config::settings::settings_local_path;
use crate::config::{
    RepoSemanticEmbeddingPolicy, prepare_daemon_platform_embeddings_install,
    resolve_bound_daemon_config_path_for_repo, resolve_daemon_config_path_for_repo,
    set_repo_semantic_embedding_policy,
};
use crate::host::inference::BITLOOPS_PLATFORM_EMBEDDINGS_RUNTIME_ID;
use crate::utils::platform_dirs::bitloops_data_dir;

use super::archive::{
    ManagedEmbeddingsArchiveKind, ManagedEmbeddingsBundleEntry,
    extract_managed_embeddings_bundle_entries_from_file, write_file_atomically,
};
use super::download::download_release_asset_to_temp_file;

const MANAGED_PLATFORM_EMBEDDINGS_RELEASES_API_BASE: &str =
    "https://api.github.com/repos/bitloops/bitloops-embeddings";
const MANAGED_PLATFORM_EMBEDDINGS_HTTP_TIMEOUT_SECS: u64 = 300;
const MANAGED_PLATFORM_EMBEDDINGS_USER_AGENT: &str = "bitloops-cli";
const MANAGED_PLATFORM_EMBEDDINGS_VERSION_OVERRIDE_ENV: &str =
    "BITLOOPS_PLATFORM_EMBEDDINGS_VERSION_OVERRIDE";
const MANAGED_PLATFORM_EMBEDDINGS_INSTALL_PARENT_DIR: &str = "tools";
const MANAGED_PLATFORM_EMBEDDINGS_INSTALL_DIR_NAME: &str = "bitloops-platform-embeddings";
const MANAGED_PLATFORM_EMBEDDINGS_METADATA_FILE_NAME: &str =
    "bitloops-platform-embeddings-install.json";

#[derive(Debug, Clone, PartialEq, Eq)]
enum ManagedPlatformEmbeddingsReleaseRequest {
    Latest,
    Tag(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ManagedPlatformEmbeddingsBinaryInstallOutcome {
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct ManagedPlatformEmbeddingsInstallMetadata {
    version: String,
    binary_path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ManagedPlatformEmbeddingsAssetSpec {
    asset_name: String,
    archive_kind: ManagedEmbeddingsArchiveKind,
}

#[cfg(test)]
type ManagedPlatformEmbeddingsInstallHook =
    dyn Fn() -> Result<ManagedPlatformEmbeddingsBinaryInstallOutcome> + 'static;

#[cfg(test)]
thread_local! {
    static MANAGED_PLATFORM_EMBEDDINGS_INSTALL_HOOK:
        RefCell<Option<Rc<ManagedPlatformEmbeddingsInstallHook>>> = RefCell::new(None);
}

pub(crate) fn install_or_configure_platform_embeddings(
    repo_root: &Path,
    gateway_url: Option<&str>,
    api_key_env: &str,
) -> Result<Vec<String>> {
    let config_path = resolve_bound_daemon_config_path_for_repo(repo_root)
        .or_else(|_| resolve_daemon_config_path_for_repo(repo_root))?;
    let plan = prepare_daemon_platform_embeddings_install(&config_path, gateway_url, api_key_env)?;
    plan.apply()?;

    let install = match install_managed_platform_embeddings_binary() {
        Ok(install) => install,
        Err(err) => {
            plan.rollback()?;
            return Err(err);
        }
    };
    let apply_result = plan.apply_with_managed_runtime_path(&install.binary_path);
    match apply_result {
        Ok(()) => {
            set_repo_semantic_embedding_policy(
                &settings_local_path(repo_root),
                &RepoSemanticEmbeddingPolicy::enabled_with_profile(&plan.profile_name),
            )?;
            let mut lines = vec![format!(
                "Configured platform embeddings in {}.",
                config_path.display()
            )];
            lines.push(if install.freshly_installed {
                format!(
                    "Installed managed standalone `bitloops-platform-embeddings` runtime {}.",
                    install.version
                )
            } else {
                format!(
                    "Managed standalone `bitloops-platform-embeddings` runtime {} already installed.",
                    install.version
                )
            });
            lines.push(format!("Binary path: {}", install.binary_path.display()));
            Ok(lines)
        }
        Err(err) => {
            plan.rollback()?;
            Err(err)
        }
    }
}

pub(crate) fn managed_platform_runtime_version_for_command(
    command: &str,
) -> Result<Option<String>> {
    let command = Path::new(command.trim());
    if !command.is_absolute() {
        return Ok(None);
    }

    if !command.starts_with(managed_platform_embeddings_binary_dir()?) {
        return Ok(None);
    }

    let Some(metadata) = load_managed_platform_embeddings_install_metadata()? else {
        return Ok(None);
    };
    Ok((command == metadata.binary_path).then_some(metadata.version))
}

pub(crate) fn managed_platform_runtime_command_is_eligible(config_path: &Path) -> Result<bool> {
    let Some(command) = raw_managed_platform_runtime_command(config_path)? else {
        return Ok(true);
    };

    let command = command.trim();
    if command.is_empty() {
        return Ok(true);
    }
    if command == "bitloops-platform-embeddings"
        || command == managed_platform_embeddings_binary_name()
    {
        return Ok(true);
    }

    let candidate = Path::new(command);
    Ok(candidate.is_absolute() && candidate.starts_with(managed_platform_embeddings_binary_dir()?))
}

fn install_managed_platform_embeddings_binary()
-> Result<ManagedPlatformEmbeddingsBinaryInstallOutcome> {
    install_managed_platform_embeddings_binary_with_progress(&mut |_| Ok(()))
}

pub(crate) fn install_managed_platform_embeddings_binary_with_progress<R>(
    report: &mut R,
) -> Result<ManagedPlatformEmbeddingsBinaryInstallOutcome>
where
    R: FnMut(crate::daemon::EmbeddingsBootstrapProgress) -> Result<()>,
{
    report(crate::daemon::EmbeddingsBootstrapProgress {
        phase: crate::daemon::EmbeddingsBootstrapPhase::ResolvingRelease,
        message: Some("Checking managed platform embeddings runtime".to_string()),
        ..Default::default()
    })?;

    #[cfg(test)]
    if let Some(hook) = MANAGED_PLATFORM_EMBEDDINGS_INSTALL_HOOK.with(|cell| cell.borrow().clone())
    {
        return hook();
    }

    let binary_path = managed_platform_embeddings_binary_path()?;
    let install_metadata = load_managed_platform_embeddings_install_metadata()?;
    let release_request = managed_platform_embeddings_release_request();
    let required_version = match &release_request {
        ManagedPlatformEmbeddingsReleaseRequest::Latest => None,
        ManagedPlatformEmbeddingsReleaseRequest::Tag(version) => Some(version.as_str()),
    };
    if let Some(metadata) = install_metadata.as_ref()
        && metadata.binary_path == binary_path
        && binary_path.is_file()
        && required_version.is_none_or(|required| required == metadata.version)
    {
        return Ok(ManagedPlatformEmbeddingsBinaryInstallOutcome {
            version: metadata.version.clone(),
            binary_path,
            freshly_installed: false,
        });
    }

    let release = fetch_managed_platform_embeddings_release(&release_request)?;
    install_managed_platform_embeddings_binary_from_release(&release, report)
}

#[cfg(test)]
pub(crate) fn with_managed_platform_embeddings_install_hook<T>(
    hook: impl Fn() -> Result<ManagedPlatformEmbeddingsBinaryInstallOutcome> + 'static,
    f: impl FnOnce() -> T,
) -> T {
    MANAGED_PLATFORM_EMBEDDINGS_INSTALL_HOOK.with(|cell| {
        assert!(
            cell.borrow().is_none(),
            "managed platform embeddings install hook already installed"
        );
        *cell.borrow_mut() = Some(Rc::new(hook));
    });
    let result = f();
    MANAGED_PLATFORM_EMBEDDINGS_INSTALL_HOOK.with(|cell| {
        *cell.borrow_mut() = None;
    });
    result
}

fn managed_platform_embeddings_binary_name() -> &'static str {
    if cfg!(windows) {
        "bitloops-platform-embeddings.exe"
    } else {
        "bitloops-platform-embeddings"
    }
}

fn managed_platform_embeddings_binary_dir() -> Result<PathBuf> {
    Ok(bitloops_data_dir()?
        .join(MANAGED_PLATFORM_EMBEDDINGS_INSTALL_PARENT_DIR)
        .join(MANAGED_PLATFORM_EMBEDDINGS_INSTALL_DIR_NAME))
}

fn managed_platform_embeddings_binary_path() -> Result<PathBuf> {
    Ok(managed_platform_embeddings_binary_dir()?.join(managed_platform_embeddings_binary_name()))
}

fn managed_platform_embeddings_metadata_path() -> Result<PathBuf> {
    Ok(bitloops_data_dir()?.join(MANAGED_PLATFORM_EMBEDDINGS_METADATA_FILE_NAME))
}

fn load_managed_platform_embeddings_install_metadata()
-> Result<Option<ManagedPlatformEmbeddingsInstallMetadata>> {
    let path = managed_platform_embeddings_metadata_path()?;
    let contents = match fs::read_to_string(&path) {
        Ok(contents) => contents,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => {
            return Err(err).with_context(|| {
                format!(
                    "reading managed platform embeddings metadata {}",
                    path.display()
                )
            });
        }
    };

    serde_json::from_str(&contents).map(Some).with_context(|| {
        format!(
            "parsing managed platform embeddings metadata {}",
            path.display()
        )
    })
}

fn save_managed_platform_embeddings_install_metadata(
    metadata: &ManagedPlatformEmbeddingsInstallMetadata,
) -> Result<()> {
    let path = managed_platform_embeddings_metadata_path()?;
    let bytes = serde_json::to_vec_pretty(metadata)
        .context("serialising managed platform embeddings metadata")?;
    write_file_atomically(&path, &bytes, false)
}

fn reset_managed_platform_embeddings_install_dir() -> Result<()> {
    let dir = managed_platform_embeddings_binary_dir()?;
    if dir.exists() {
        fs::remove_dir_all(&dir).with_context(|| {
            format!(
                "removing existing managed platform embeddings install directory {}",
                dir.display()
            )
        })?;
    }
    fs::create_dir_all(&dir).with_context(|| {
        format!(
            "creating managed platform embeddings install directory {}",
            dir.display()
        )
    })?;
    Ok(())
}

fn managed_platform_embeddings_target_version() -> String {
    env::var(MANAGED_PLATFORM_EMBEDDINGS_VERSION_OVERRIDE_ENV)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_default()
}

fn managed_platform_embeddings_release_request() -> ManagedPlatformEmbeddingsReleaseRequest {
    let version = managed_platform_embeddings_target_version();
    if version.is_empty() {
        ManagedPlatformEmbeddingsReleaseRequest::Latest
    } else {
        ManagedPlatformEmbeddingsReleaseRequest::Tag(version)
    }
}

fn managed_platform_embeddings_asset_spec(
    version: &str,
) -> Result<ManagedPlatformEmbeddingsAssetSpec> {
    let (target_triple, archive_kind, extension) = match (env::consts::OS, env::consts::ARCH) {
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
        (os, arch) => {
            bail!("managed bitloops-platform-embeddings install is not supported on {os}/{arch}")
        }
    };

    Ok(ManagedPlatformEmbeddingsAssetSpec {
        asset_name: format!("bitloops-platform-embeddings-{version}-{target_triple}.{extension}"),
        archive_kind,
    })
}

fn install_managed_platform_embeddings_binary_from_release(
    release: &GitHubReleasePayload,
    report: &mut impl FnMut(crate::daemon::EmbeddingsBootstrapProgress) -> Result<()>,
) -> Result<ManagedPlatformEmbeddingsBinaryInstallOutcome> {
    let asset_spec = managed_platform_embeddings_asset_spec(&release.tag_name)?;
    let asset = release
        .assets
        .iter()
        .find(|asset| asset.name == asset_spec.asset_name)
        .ok_or_else(|| {
            anyhow!(
                "managed bitloops-platform-embeddings release `{}` did not contain asset `{}`",
                release.tag_name,
                asset_spec.asset_name
            )
        })?;
    let expected_digest = parse_sha256_digest(asset.digest.as_deref())?;
    report(crate::daemon::EmbeddingsBootstrapProgress {
        phase: crate::daemon::EmbeddingsBootstrapPhase::DownloadingRuntime,
        asset_name: Some(asset.name.clone()),
        version: Some(release.tag_name.clone()),
        message: Some(format!("Downloading `{}`", asset.name)),
        ..Default::default()
    })?;
    let client = managed_platform_embeddings_http_client()?;
    let download = download_release_asset_to_temp_file(
        &client,
        &asset.browser_download_url,
        MANAGED_PLATFORM_EMBEDDINGS_USER_AGENT,
        "managed bitloops-platform-embeddings asset",
        |downloaded, total| {
            report(crate::daemon::EmbeddingsBootstrapProgress {
                phase: crate::daemon::EmbeddingsBootstrapPhase::DownloadingRuntime,
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
            "managed bitloops-platform-embeddings asset digest mismatch for `{}`: expected {}, got {}",
            asset.name,
            expected_digest,
            download.sha256_hex
        );
    }
    report(crate::daemon::EmbeddingsBootstrapProgress {
        phase: crate::daemon::EmbeddingsBootstrapPhase::ExtractingRuntime,
        asset_name: Some(asset.name.clone()),
        bytes_downloaded: download.bytes_downloaded,
        bytes_total: Some(download.bytes_downloaded),
        version: Some(release.tag_name.clone()),
        message: Some(format!("Extracting `{}`", asset.name)),
    })?;
    let extract_started = Instant::now();

    let bundle_entries = extract_managed_embeddings_bundle_entries_from_file(
        download.path(),
        asset_spec.archive_kind,
        managed_platform_embeddings_binary_name(),
    )
    .with_context(|| {
        format!(
            "extracting managed platform embeddings bundle from `{}`",
            asset.name
        )
    })?;
    log::info!(
        "managed runtime extraction complete: runtime=bitloops-platform-embeddings version={} asset_name={} archive_bytes={} extract_ms={}",
        release.tag_name,
        asset.name,
        download.bytes_downloaded,
        extract_started.elapsed().as_millis()
    );

    install_managed_platform_embeddings_bundle_entries(&release.tag_name, bundle_entries)
}

fn fetch_managed_platform_embeddings_release(
    request: &ManagedPlatformEmbeddingsReleaseRequest,
) -> Result<GitHubReleasePayload> {
    let release_path = match request {
        ManagedPlatformEmbeddingsReleaseRequest::Latest => "latest".to_string(),
        ManagedPlatformEmbeddingsReleaseRequest::Tag(version) => format!("tags/{version}"),
    };
    let url = format!("{MANAGED_PLATFORM_EMBEDDINGS_RELEASES_API_BASE}/releases/{release_path}");
    let started = Instant::now();
    let release = managed_platform_embeddings_http_client()?
        .get(url)
        .header(ACCEPT, "application/vnd.github+json")
        .header(USER_AGENT, MANAGED_PLATFORM_EMBEDDINGS_USER_AGENT)
        .send()
        .context("fetching managed bitloops-platform-embeddings release metadata")?
        .error_for_status()
        .context("fetching managed bitloops-platform-embeddings release metadata")?
        .json::<GitHubReleasePayload>()
        .context("parsing managed bitloops-platform-embeddings release metadata")?;
    log::info!(
        "managed runtime release resolved: runtime=bitloops-platform-embeddings requested_release={} resolved_version={} asset_count={} resolve_ms={}",
        release_path,
        release.tag_name,
        release.assets.len(),
        started.elapsed().as_millis()
    );
    Ok(release)
}

fn managed_platform_embeddings_http_client() -> Result<Client> {
    Client::builder()
        .timeout(Duration::from_secs(
            MANAGED_PLATFORM_EMBEDDINGS_HTTP_TIMEOUT_SECS,
        ))
        .build()
        .context("building managed bitloops-platform-embeddings HTTP client")
}

fn install_managed_platform_embeddings_bundle_entries(
    version: &str,
    bundle_entries: Vec<ManagedEmbeddingsBundleEntry>,
) -> Result<ManagedPlatformEmbeddingsBinaryInstallOutcome> {
    reset_managed_platform_embeddings_install_dir()?;
    let install_dir = managed_platform_embeddings_binary_dir()?;
    let binary_path = managed_platform_embeddings_binary_path()?;
    for entry in bundle_entries {
        let output_path = install_dir.join(&entry.relative_path);
        write_file_atomically(&output_path, &entry.bytes, entry.executable)?;
    }

    save_managed_platform_embeddings_install_metadata(&ManagedPlatformEmbeddingsInstallMetadata {
        version: version.to_string(),
        binary_path: binary_path.clone(),
    })?;

    Ok(ManagedPlatformEmbeddingsBinaryInstallOutcome {
        version: version.to_string(),
        binary_path,
        freshly_installed: true,
    })
}

fn parse_sha256_digest(digest: Option<&str>) -> Result<String> {
    let digest = digest
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .context("managed bitloops-platform-embeddings release asset is missing a digest")?;
    let digest = digest.strip_prefix("sha256:").unwrap_or(digest);
    if digest.len() != 64 || !digest.chars().all(|char| char.is_ascii_hexdigit()) {
        bail!("managed bitloops-platform-embeddings release asset digest is malformed");
    }
    Ok(digest.to_ascii_lowercase())
}

fn raw_managed_platform_runtime_command(config_path: &Path) -> Result<Option<String>> {
    let contents = match fs::read_to_string(config_path) {
        Ok(contents) => contents,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => {
            return Err(err).with_context(|| {
                format!(
                    "reading Bitloops daemon config for managed platform embeddings runtime {}",
                    config_path.display()
                )
            });
        }
    };
    let doc = contents.parse::<DocumentMut>().with_context(|| {
        format!(
            "parsing Bitloops daemon config for managed platform embeddings runtime {}",
            config_path.display()
        )
    })?;

    Ok(doc
        .as_table()
        .get("inference")
        .and_then(Item::as_table)
        .and_then(|table| table.get("runtimes"))
        .and_then(Item::as_table)
        .and_then(|table| table.get(BITLOOPS_PLATFORM_EMBEDDINGS_RUNTIME_ID))
        .and_then(Item::as_table)
        .and_then(|table| table.get("command"))
        .and_then(Item::as_value)
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned))
}
