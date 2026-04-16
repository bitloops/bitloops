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
    ManagedEmbeddingsArchiveKind, ManagedEmbeddingsBundleEntry,
    extract_managed_embeddings_bundle_entries, extract_managed_embeddings_bundle_entries_from_file,
    sha256_hex, write_file_atomically,
};
use super::config::{
    MANAGED_EMBEDDINGS_VERSION_OVERRIDE_ENV, ManagedEmbeddingsInstallMetadata,
    ManagedEmbeddingsMetadataError, load_managed_embeddings_install_metadata,
    managed_embeddings_binary_dir, managed_embeddings_binary_name, managed_embeddings_binary_path,
    managed_embeddings_bundle_is_complete, reset_managed_embeddings_install_dir,
    rewrite_managed_runtime_command_if_eligible, save_managed_embeddings_install_metadata,
};
use super::download::download_release_asset_to_temp_file;

const MANAGED_EMBEDDINGS_RELEASES_API_BASE: &str =
    "https://api.github.com/repos/bitloops/bitloops-embeddings";
const MANAGED_EMBEDDINGS_HTTP_TIMEOUT_SECS: u64 = 300;
const MANAGED_EMBEDDINGS_USER_AGENT: &str = "bitloops-cli";

// TODO: replace latest-resolution with compatibility-range negotiation once the
// managed embeddings runtime exposes an explicit compatibility contract.
#[derive(Debug, Clone, PartialEq, Eq)]
enum ManagedEmbeddingsReleaseRequest {
    Latest,
    Tag(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ManagedEmbeddingsBinaryInstallOutcome {
    pub(crate) version: String,
    pub(crate) binary_path: PathBuf,
    pub(crate) freshly_installed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ManagedEmbeddingsRuntimeEnsureOutcome {
    pub(crate) install: ManagedEmbeddingsBinaryInstallOutcome,
    pub(crate) command_rewritten: bool,
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

#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn ensure_managed_embeddings_runtime(
    repo_root: &Path,
    config_path: Option<&Path>,
) -> Result<Vec<String>> {
    let outcome =
        ensure_managed_embeddings_runtime_with_progress(repo_root, config_path, |_| Ok(()))?;
    Ok(format_managed_embeddings_runtime_lines(
        &outcome,
        config_path,
    ))
}

pub(crate) fn ensure_managed_embeddings_runtime_with_progress<R>(
    repo_root: &Path,
    config_path: Option<&Path>,
    mut report: R,
) -> Result<ManagedEmbeddingsRuntimeEnsureOutcome>
where
    R: FnMut(crate::daemon::EmbeddingsBootstrapProgress) -> Result<()>,
{
    report(crate::daemon::EmbeddingsBootstrapProgress {
        phase: crate::daemon::EmbeddingsBootstrapPhase::ResolvingRelease,
        message: Some("Checking managed embeddings runtime".to_string()),
        ..Default::default()
    })?;
    let install = install_managed_embeddings_binary_with_progress(repo_root, &mut report)?;
    let mut command_rewritten = false;
    if let Some(config_path) = config_path {
        report(crate::daemon::EmbeddingsBootstrapProgress {
            phase: crate::daemon::EmbeddingsBootstrapPhase::RewritingRuntime,
            version: Some(install.version.clone()),
            message: Some(format!(
                "Updating runtime command in {}",
                config_path.display()
            )),
            ..Default::default()
        })?;
        command_rewritten =
            rewrite_managed_runtime_command_if_eligible(config_path, &install.binary_path)?;
    }
    Ok(ManagedEmbeddingsRuntimeEnsureOutcome {
        install,
        command_rewritten,
    })
}

fn format_managed_embeddings_runtime_lines(
    outcome: &ManagedEmbeddingsRuntimeEnsureOutcome,
    config_path: Option<&Path>,
) -> Vec<String> {
    let mut lines = vec![if outcome.install.freshly_installed {
        format!(
            "Installed managed standalone `bitloops-local-embeddings` runtime {}.",
            outcome.install.version
        )
    } else {
        format!(
            "Managed standalone `bitloops-local-embeddings` runtime {} already installed.",
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
            "Updated embeddings runtime command and args in {}.",
            config_path.display()
        ));
    }

    lines
}

#[cfg_attr(not(test), allow(dead_code))]
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

    if matches!(plan.mode, DaemonEmbeddingsInstallMode::WarmExisting) {
        let capability = embedding_capability_for_config_path(&config_path)?;
        let result =
            pull_profile_with_config_path(repo_root, &config_path, &capability, &plan.profile_name);
        return match result {
            Ok(mut lines) => {
                lines.insert(
                    0,
                    format!(
                        "Embeddings already configured via profile `{}`; warming local cache.",
                        plan.profile_name
                    ),
                );
                Ok(lines)
            }
            Err(err) => {
                if plan.config_modified {
                    plan.rollback()?;
                }
                Err(err)
            }
        };
    }

    let ensure = ensure_managed_embeddings_runtime_with_progress(repo_root, None, |_| Ok(()))?;
    plan.apply_with_managed_runtime_path(&ensure.install.binary_path)?;
    let capability = embedding_capability_for_config_path(&config_path)?;
    let result =
        pull_profile_with_config_path(repo_root, &config_path, &capability, &plan.profile_name);
    match result {
        Ok(mut lines) => {
            lines.splice(
                0..0,
                format_managed_embeddings_runtime_lines(&ensure, Some(&config_path)),
            );
            lines.insert(
                0,
                format!("Configured embeddings in {}.", plan.config_path.display()),
            );
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

#[allow(dead_code)]
fn install_managed_embeddings_binary(
    repo_root: &Path,
) -> Result<ManagedEmbeddingsBinaryInstallOutcome> {
    install_managed_embeddings_binary_with_progress(repo_root, &mut |_| Ok(()))
}

fn install_managed_embeddings_binary_with_progress<R>(
    repo_root: &Path,
    report: &mut R,
) -> Result<ManagedEmbeddingsBinaryInstallOutcome>
where
    R: FnMut(crate::daemon::EmbeddingsBootstrapProgress) -> Result<()>,
{
    #[cfg(test)]
    if let Some(hook) = MANAGED_EMBEDDINGS_INSTALL_HOOK.with(|cell| cell.borrow().clone()) {
        return hook(repo_root);
    }

    let _ = repo_root;
    let binary_path = managed_embeddings_binary_path()?;
    let install_metadata = load_managed_embeddings_install_metadata_for_install()?;
    let release_request = managed_embeddings_release_request();
    let required_version = match &release_request {
        ManagedEmbeddingsReleaseRequest::Latest => None,
        ManagedEmbeddingsReleaseRequest::Tag(version) => Some(version.as_str()),
    };
    if let Some(outcome) = installed_managed_embeddings_outcome(
        install_metadata.as_ref(),
        &binary_path,
        required_version,
    ) {
        report(crate::daemon::EmbeddingsBootstrapProgress {
            phase: crate::daemon::EmbeddingsBootstrapPhase::ResolvingRelease,
            version: Some(outcome.version.clone()),
            message: Some("Managed embeddings runtime already installed".to_string()),
            ..Default::default()
        })?;
        return Ok(outcome);
    }

    let release = fetch_managed_embeddings_release(&release_request)?;

    install_managed_embeddings_binary_from_release(&release, report)
}

fn load_managed_embeddings_install_metadata_for_install()
-> Result<Option<ManagedEmbeddingsInstallMetadata>> {
    match load_managed_embeddings_install_metadata() {
        Ok(metadata) => Ok(metadata),
        Err(ManagedEmbeddingsMetadataError::Parse { path, source }) => {
            eprintln!(
                "[bitloops] Warning: invalid managed embeddings metadata at {}: {}. Ignoring cached metadata and reinstalling the managed runtime.",
                path.display(),
                source
            );
            Ok(None)
        }
        Err(err) => Err(err.into()),
    }
}

#[cfg_attr(not(test), allow(dead_code))]
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
            "managed bitloops-local-embeddings asset digest mismatch for `{asset_name}`: expected {expected_digest}, got {actual_digest}"
        );
    }

    let binary_name = managed_embeddings_binary_name();
    let bundle_entries =
        extract_managed_embeddings_bundle_entries(archive_bytes, archive_kind, binary_name)
            .with_context(|| {
                format!("extracting managed bitloops-local-embeddings bundle from `{asset_name}`")
            })?;
    install_managed_embeddings_bundle_entries(version, bundle_entries)
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
            bail!("managed bitloops-local-embeddings install is not supported on {os}/{arch}")
        }
    };

    Ok(ManagedEmbeddingsAssetSpec {
        asset_name: format!("bitloops-local-embeddings-{version}-{target_triple}.{extension}"),
        archive_kind,
    })
}

fn managed_embeddings_target_version() -> String {
    env::var(MANAGED_EMBEDDINGS_VERSION_OVERRIDE_ENV)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_default()
}

fn managed_embeddings_release_request() -> ManagedEmbeddingsReleaseRequest {
    let version = managed_embeddings_target_version();
    if version.is_empty() {
        ManagedEmbeddingsReleaseRequest::Latest
    } else {
        ManagedEmbeddingsReleaseRequest::Tag(version)
    }
}

fn managed_embeddings_asset_spec(version: &str) -> Result<ManagedEmbeddingsAssetSpec> {
    managed_embeddings_asset_spec_for(env::consts::OS, env::consts::ARCH, version)
}

fn installed_managed_embeddings_outcome(
    metadata: Option<&ManagedEmbeddingsInstallMetadata>,
    binary_path: &Path,
    required_version: Option<&str>,
) -> Option<ManagedEmbeddingsBinaryInstallOutcome> {
    let metadata = metadata?;
    if metadata.binary_path != binary_path || !managed_embeddings_bundle_is_complete(binary_path) {
        return None;
    }
    if let Some(required_version) = required_version
        && metadata.version != required_version
    {
        return None;
    }

    Some(ManagedEmbeddingsBinaryInstallOutcome {
        version: metadata.version.clone(),
        binary_path: binary_path.to_path_buf(),
        freshly_installed: false,
    })
}

fn install_managed_embeddings_binary_from_release<R>(
    release: &GitHubReleasePayload,
    report: &mut R,
) -> Result<ManagedEmbeddingsBinaryInstallOutcome>
where
    R: FnMut(crate::daemon::EmbeddingsBootstrapProgress) -> Result<()>,
{
    let asset_spec = managed_embeddings_asset_spec(&release.tag_name)?;
    let asset = release
        .assets
        .iter()
        .find(|asset| asset.name == asset_spec.asset_name)
        .ok_or_else(|| {
            anyhow!(
                "managed bitloops-local-embeddings release `{}` did not contain asset `{}`",
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
    let client = managed_embeddings_http_client()?;
    let download = download_release_asset_to_temp_file(
        &client,
        &asset.browser_download_url,
        MANAGED_EMBEDDINGS_USER_AGENT,
        "managed bitloops-local-embeddings asset",
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
            "managed bitloops-local-embeddings asset digest mismatch for `{}`: expected {}, got {}",
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
    let bundle_entries = extract_managed_embeddings_bundle_entries_from_file(
        download.path(),
        asset_spec.archive_kind,
        managed_embeddings_binary_name(),
    )
    .with_context(|| {
        format!(
            "extracting managed bitloops-local-embeddings bundle from `{}`",
            asset.name
        )
    })?;

    install_managed_embeddings_bundle_entries(&release.tag_name, bundle_entries)
}

fn fetch_managed_embeddings_release(
    request: &ManagedEmbeddingsReleaseRequest,
) -> Result<GitHubReleasePayload> {
    let release_path = match request {
        ManagedEmbeddingsReleaseRequest::Latest => "latest".to_string(),
        ManagedEmbeddingsReleaseRequest::Tag(version) => format!("tags/{version}"),
    };
    let url = format!("{MANAGED_EMBEDDINGS_RELEASES_API_BASE}/releases/{release_path}");
    managed_embeddings_http_client()?
        .get(url)
        .header(ACCEPT, "application/vnd.github+json")
        .header(USER_AGENT, MANAGED_EMBEDDINGS_USER_AGENT)
        .send()
        .context("fetching managed bitloops-local-embeddings release metadata")?
        .error_for_status()
        .context("fetching managed bitloops-local-embeddings release metadata")?
        .json::<GitHubReleasePayload>()
        .context("parsing managed bitloops-local-embeddings release metadata")
}

fn managed_embeddings_http_client() -> Result<Client> {
    Client::builder()
        .timeout(Duration::from_secs(MANAGED_EMBEDDINGS_HTTP_TIMEOUT_SECS))
        .build()
        .context("building managed bitloops-local-embeddings HTTP client")
}

fn install_managed_embeddings_bundle_entries(
    version: &str,
    bundle_entries: Vec<ManagedEmbeddingsBundleEntry>,
) -> Result<ManagedEmbeddingsBinaryInstallOutcome> {
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

fn parse_sha256_digest(digest: Option<&str>) -> Result<String> {
    let digest = digest
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .context("managed bitloops-local-embeddings release asset is missing a digest")?;
    let digest = digest.strip_prefix("sha256:").unwrap_or(digest);
    if digest.len() != 64 || !digest.chars().all(|char| char.is_ascii_hexdigit()) {
        bail!("managed bitloops-local-embeddings release asset digest is malformed");
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

#[cfg(test)]
mod tests {
    use super::{ManagedEmbeddingsReleaseRequest, managed_embeddings_release_request};
    use crate::test_support::process_state::enter_process_state;
    use tempfile::TempDir;

    #[test]
    fn managed_embeddings_release_request_defaults_to_latest() {
        let repo = TempDir::new().expect("tempdir");
        let _guard = enter_process_state(
            Some(repo.path()),
            &[("BITLOOPS_LOCAL_EMBEDDINGS_VERSION_OVERRIDE", None)],
        );

        assert_eq!(
            managed_embeddings_release_request(),
            ManagedEmbeddingsReleaseRequest::Latest
        );
    }

    #[test]
    fn managed_embeddings_release_request_uses_override_tag() {
        let repo = TempDir::new().expect("tempdir");
        let _guard = enter_process_state(
            Some(repo.path()),
            &[("BITLOOPS_LOCAL_EMBEDDINGS_VERSION_OVERRIDE", Some("v1.2.3"))],
        );

        assert_eq!(
            managed_embeddings_release_request(),
            ManagedEmbeddingsReleaseRequest::Tag("v1.2.3".to_string())
        );
    }
}
