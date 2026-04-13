use super::DashboardState;
use super::bundle_types::{
    BundleCheckResult, BundleError, BundleInstallResult, BundleVersionEntry, BundleVersionReason,
    BundleVersionsManifest, LocalBundleVersionFile, ResolvedBundleVersion,
};
use semver::Version;
use sha2::{Digest, Sha256};
use std::env;
use std::fs;
use std::io::Cursor;
use std::path::{Component, Path, PathBuf};
use tar::Archive;
use uuid::Uuid;

const DASHBOARD_CDN_BASE_URL_ENV: &str = "BITLOOPS_DASHBOARD_CDN_BASE_URL";
const DASHBOARD_MANIFEST_URL_ENV: &str = "BITLOOPS_DASHBOARD_MANIFEST_URL";
const MANIFEST_FILE_NAME: &str = "bundle_versions.json";

mod dashboard_env {
    include!(concat!(env!("OUT_DIR"), "/dashboard_env.rs"));
}

pub(super) async fn check_bundle_version(
    state: &DashboardState,
) -> Result<BundleCheckResult, BundleError> {
    let current_cli_version = current_cli_version()?;
    let local_version = read_local_bundle_version(&state.bundle_dir)?;
    let manifest = fetch_manifest_for_state(state).await?;
    let resolved =
        resolve_latest_applicable_for_state(&manifest.versions, &current_cli_version, state)?;

    let result = match resolved {
        Some(latest) => match local_version.as_deref() {
            None => BundleCheckResult {
                current_version: None,
                latest_applicable_version: Some(latest.version),
                install_available: true,
                reason: BundleVersionReason::NotInstalled,
            },
            Some(current) if current == latest.version => BundleCheckResult {
                current_version: Some(current.to_string()),
                latest_applicable_version: Some(latest.version),
                install_available: false,
                reason: BundleVersionReason::UpToDate,
            },
            Some(current) => BundleCheckResult {
                current_version: Some(current.to_string()),
                latest_applicable_version: Some(latest.version),
                install_available: true,
                reason: BundleVersionReason::UpdateAvailable,
            },
        },
        None => BundleCheckResult {
            current_version: local_version,
            latest_applicable_version: None,
            install_available: false,
            reason: BundleVersionReason::NoCompatibleVersion,
        },
    };

    Ok(result)
}

pub(super) async fn fetch_bundle(
    state: &DashboardState,
) -> Result<BundleInstallResult, BundleError> {
    let current_cli_version = current_cli_version()?;
    let manifest = fetch_manifest_for_state(state).await?;
    let Some(resolved) =
        resolve_latest_applicable_for_state(&manifest.versions, &current_cli_version, state)?
    else {
        return Err(BundleError::NoCompatibleVersion);
    };

    let archive_bytes = download_bytes(&resolved.download_url).await?;
    let checksum_payload = download_text(&resolved.checksum_url).await?;
    verify_sha256(&archive_bytes, &checksum_payload)?;

    install_archive_atomically(&archive_bytes, &state.bundle_dir)?;

    Ok(BundleInstallResult {
        installed_version: resolved.version,
        bundle_dir: state.bundle_dir.display().to_string(),
        status: "installed".to_string(),
        checksum_verified: true,
    })
}

fn current_cli_version() -> Result<Version, BundleError> {
    Version::parse(env!("CARGO_PKG_VERSION"))
        .map_err(|err| BundleError::Internal(format!("invalid CLI version: {err}")))
}

fn read_local_bundle_version(bundle_dir: &Path) -> Result<Option<String>, BundleError> {
    let version_path = bundle_dir.join("version.json");
    let content = match fs::read_to_string(&version_path) {
        Ok(content) => content,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => {
            return Err(BundleError::Internal(format!(
                "failed reading local bundle version metadata: {err}"
            )));
        }
    };

    let parsed: LocalBundleVersionFile = match serde_json::from_str(&content) {
        Ok(parsed) => parsed,
        Err(_) => return Ok(None),
    };

    let _ = parsed.source_url;

    let normalized = parsed.version.trim().to_string();
    if normalized.is_empty() {
        Ok(None)
    } else {
        Ok(Some(normalized))
    }
}

async fn fetch_manifest_for_state(
    state: &DashboardState,
) -> Result<BundleVersionsManifest, BundleError> {
    let manifest_url = manifest_url_for_state(state)?;
    let body = download_text(&manifest_url)
        .await
        .map_err(|err| match err {
            BundleError::BundleDownloadFailed(message) => BundleError::ManifestFetchFailed(message),
            other => other,
        })?;

    serde_json::from_str::<BundleVersionsManifest>(&body)
        .map_err(|err| BundleError::ManifestParseFailed(format!("invalid manifest JSON: {err}")))
}

fn manifest_url_for_state(state: &DashboardState) -> Result<String, BundleError> {
    manifest_url_from_overrides(Some(&state.bundle_source_overrides))
}

fn manifest_url_from_overrides(
    overrides: Option<&crate::api::DashboardBundleSourceOverrides>,
) -> Result<String, BundleError> {
    if let Some(explicit_manifest_url) = overrides.and_then(|item| item.manifest_url.as_deref()) {
        let trimmed = explicit_manifest_url.trim();
        if !trimmed.is_empty() {
            return Ok(trimmed.to_string());
        }
    }

    if let Some(explicit_base_url) = overrides.and_then(|item| item.cdn_base_url.as_deref()) {
        let trimmed = explicit_base_url.trim();
        if !trimmed.is_empty() {
            return join_url(trimmed, MANIFEST_FILE_NAME);
        }
    }

    if let Ok(explicit_manifest_url) = env::var(DASHBOARD_MANIFEST_URL_ENV) {
        let trimmed = explicit_manifest_url.trim();
        if !trimmed.is_empty() {
            return Ok(trimmed.to_string());
        }
    }

    if let Ok(explicit_base_url) = env::var(DASHBOARD_CDN_BASE_URL_ENV) {
        let trimmed = explicit_base_url.trim();
        if !trimmed.is_empty() {
            return join_url(trimmed, MANIFEST_FILE_NAME);
        }
    }

    let compiled_manifest = dashboard_env::DASHBOARD_MANIFEST_URL.trim();
    if !compiled_manifest.is_empty() {
        return Ok(compiled_manifest.to_string());
    }

    let compiled_base = dashboard_env::DASHBOARD_CDN_BASE_URL.trim();
    if compiled_base.is_empty() {
        return Err(BundleError::ManifestFetchFailed(
            "missing configured dashboard URLs (manifest and CDN base)".to_string(),
        ));
    }

    join_url(compiled_base, MANIFEST_FILE_NAME)
}

fn join_url(base: &str, path: &str) -> Result<String, BundleError> {
    let parsed = reqwest::Url::parse(base)
        .map_err(|err| BundleError::ManifestFetchFailed(format!("invalid CDN base URL: {err}")))?;
    parsed
        .join(path)
        .map(|url| url.to_string())
        .map_err(|err| BundleError::ManifestFetchFailed(format!("invalid manifest URL: {err}")))
}

async fn download_text(url: &str) -> Result<String, BundleError> {
    if let Some(path) = file_url_to_path(url)? {
        return fs::read_to_string(path).map_err(|err| {
            BundleError::BundleDownloadFailed(format!("reading {url} failed: {err}"))
        });
    }

    let response = reqwest::get(url)
        .await
        .map_err(|err| BundleError::BundleDownloadFailed(format!("GET {url} failed: {err}")))?;

    if !response.status().is_success() {
        return Err(BundleError::BundleDownloadFailed(format!(
            "GET {url} returned {}",
            response.status()
        )));
    }

    response.text().await.map_err(|err| {
        BundleError::BundleDownloadFailed(format!("reading body {url} failed: {err}"))
    })
}

async fn download_bytes(url: &str) -> Result<Vec<u8>, BundleError> {
    if let Some(path) = file_url_to_path(url)? {
        return fs::read(path).map_err(|err| {
            BundleError::BundleDownloadFailed(format!("reading {url} failed: {err}"))
        });
    }

    let response = reqwest::get(url)
        .await
        .map_err(|err| BundleError::BundleDownloadFailed(format!("GET {url} failed: {err}")))?;

    if !response.status().is_success() {
        return Err(BundleError::BundleDownloadFailed(format!(
            "GET {url} returned {}",
            response.status()
        )));
    }

    response
        .bytes()
        .await
        .map(|bytes| bytes.to_vec())
        .map_err(|err| {
            BundleError::BundleDownloadFailed(format!("reading body {url} failed: {err}"))
        })
}

fn resolve_latest_applicable_for_state(
    versions: &[BundleVersionEntry],
    current_cli_version: &Version,
    state: &DashboardState,
) -> Result<Option<ResolvedBundleVersion>, BundleError> {
    resolve_latest_applicable_from_overrides(
        versions,
        current_cli_version,
        Some(&state.bundle_source_overrides),
    )
}

fn resolve_latest_applicable_from_overrides(
    versions: &[BundleVersionEntry],
    current_cli_version: &Version,
    overrides: Option<&crate::api::DashboardBundleSourceOverrides>,
) -> Result<Option<ResolvedBundleVersion>, BundleError> {
    let mut best: Option<(Version, ResolvedBundleVersion)> = None;

    for entry in versions {
        if !is_entry_compatible(entry, current_cli_version)? {
            continue;
        }

        let parsed_version = Version::parse(entry.version.trim()).map_err(|err| {
            BundleError::ManifestParseFailed(format!(
                "invalid manifest version {}: {err}",
                entry.version
            ))
        })?;

        let download_url = resolve_entry_url_from_overrides(&entry.download_url, overrides)?;
        let checksum_url = resolve_entry_url_from_overrides(&entry.checksum_url, overrides)?;

        let resolved = ResolvedBundleVersion {
            version: parsed_version.to_string(),
            download_url,
            checksum_url,
        };

        match &best {
            Some((best_version, _)) if parsed_version <= *best_version => {}
            _ => {
                best = Some((parsed_version, resolved));
            }
        }
    }

    Ok(best.map(|(_, resolved)| resolved))
}

fn resolve_entry_url_from_overrides(
    raw: &str,
    overrides: Option<&crate::api::DashboardBundleSourceOverrides>,
) -> Result<String, BundleError> {
    if raw.starts_with("http://") || raw.starts_with("https://") {
        return Ok(raw.to_string());
    }

    if let Some(explicit_base_url) = overrides.and_then(|item| item.cdn_base_url.as_deref()) {
        let trimmed = explicit_base_url.trim();
        if !trimmed.is_empty() {
            return join_url(trimmed, raw);
        }
    }

    if let Ok(explicit_base_url) = env::var(DASHBOARD_CDN_BASE_URL_ENV) {
        let trimmed = explicit_base_url.trim();
        if !trimmed.is_empty() {
            return join_url(trimmed, raw);
        }
    }

    let compiled_base = dashboard_env::DASHBOARD_CDN_BASE_URL.trim();
    if compiled_base.is_empty() {
        return Err(BundleError::ManifestFetchFailed(
            "missing configured dashboard CDN base URL".to_string(),
        ));
    }

    join_url(compiled_base, raw)
}

fn is_entry_compatible(
    entry: &BundleVersionEntry,
    current_cli_version: &Version,
) -> Result<bool, BundleError> {
    let min_version = Version::parse(entry.min_required_cli_version.trim()).map_err(|err| {
        BundleError::ManifestParseFailed(format!(
            "invalid min_required_cli_version {}: {err}",
            entry.min_required_cli_version
        ))
    })?;

    if current_cli_version < &min_version {
        return Ok(false);
    }

    matches_max(entry.max_required_cli_version.trim(), current_cli_version)
}

fn matches_max(max: &str, current_cli_version: &Version) -> Result<bool, BundleError> {
    if max.eq_ignore_ascii_case("latest") {
        return Ok(true);
    }

    if let Some(prefix) = max.strip_suffix(".x") {
        let mut parts = prefix.split('.');
        let Some(major_raw) = parts.next() else {
            return Err(BundleError::ManifestParseFailed(format!(
                "invalid max_required_cli_version {max}"
            )));
        };
        let Some(minor_raw) = parts.next() else {
            return Err(BundleError::ManifestParseFailed(format!(
                "invalid max_required_cli_version {max}"
            )));
        };
        if parts.next().is_some() {
            return Err(BundleError::ManifestParseFailed(format!(
                "invalid max_required_cli_version {max}"
            )));
        }

        let major = major_raw.parse::<u64>().map_err(|_| {
            BundleError::ManifestParseFailed(format!("invalid max_required_cli_version {max}"))
        })?;
        let minor = minor_raw.parse::<u64>().map_err(|_| {
            BundleError::ManifestParseFailed(format!("invalid max_required_cli_version {max}"))
        })?;

        return Ok((current_cli_version.major, current_cli_version.minor) <= (major, minor));
    }

    let max_version = Version::parse(max).map_err(|err| {
        BundleError::ManifestParseFailed(format!("invalid max_required_cli_version {max}: {err}"))
    })?;
    Ok(current_cli_version <= &max_version)
}

fn verify_sha256(archive_bytes: &[u8], checksum_payload: &str) -> Result<(), BundleError> {
    let expected = parse_checksum(checksum_payload).ok_or(BundleError::ChecksumMismatch)?;

    let mut hasher = Sha256::new();
    hasher.update(archive_bytes);
    let actual = hex::encode(hasher.finalize());

    if actual == expected {
        Ok(())
    } else {
        Err(BundleError::ChecksumMismatch)
    }
}

fn parse_checksum(payload: &str) -> Option<String> {
    let first_token = payload.split_whitespace().next()?;
    if first_token.len() != 64 {
        return None;
    }
    if !first_token.chars().all(|ch| ch.is_ascii_hexdigit()) {
        return None;
    }
    Some(first_token.to_ascii_lowercase())
}

fn install_archive_atomically(archive_bytes: &[u8], bundle_dir: &Path) -> Result<(), BundleError> {
    let parent = bundle_dir
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));

    fs::create_dir_all(&parent).map_err(|err| {
        BundleError::BundleInstallFailed(format!("failed creating parent dir: {err}"))
    })?;

    let staging_root = parent.join(format!(".bundle-staging-{}", Uuid::new_v4()));
    let extract_dir = staging_root.join("extract");
    fs::create_dir_all(&extract_dir).map_err(|err| {
        BundleError::BundleInstallFailed(format!("failed creating staging dir: {err}"))
    })?;

    unpack_archive(archive_bytes, &extract_dir)?;

    let bundle_root = locate_bundle_root(&extract_dir)?;
    let prepared_dir = staging_root.join("bundle-ready");

    if bundle_root != prepared_dir {
        fs::rename(&bundle_root, &prepared_dir).map_err(|err| {
            BundleError::BundleInstallFailed(format!("failed preparing bundle root: {err}"))
        })?;
    }

    let backup_dir = parent.join(format!(".bundle-backup-{}", Uuid::new_v4()));
    let target_exists = bundle_dir.exists();

    if target_exists {
        fs::rename(bundle_dir, &backup_dir).map_err(|err| {
            BundleError::BundleInstallFailed(format!(
                "failed moving existing bundle to backup: {err}"
            ))
        })?;
    }

    if let Err(err) = fs::rename(&prepared_dir, bundle_dir) {
        if target_exists {
            let _ = fs::rename(&backup_dir, bundle_dir);
        }
        let _ = fs::remove_dir_all(&staging_root);
        return Err(BundleError::BundleInstallFailed(format!(
            "failed moving prepared bundle into place: {err}"
        )));
    }

    if target_exists {
        let _ = fs::remove_dir_all(&backup_dir);
    }

    let _ = fs::remove_dir_all(&staging_root);
    Ok(())
}

fn unpack_archive(archive_bytes: &[u8], extract_dir: &Path) -> Result<(), BundleError> {
    let cursor = Cursor::new(archive_bytes);
    let decoder = zstd::stream::read::Decoder::new(cursor).map_err(|err| {
        BundleError::BundleInstallFailed(format!("zstd decode init failed: {err}"))
    })?;
    let mut archive = Archive::new(decoder);

    let entries = archive.entries().map_err(|err| {
        BundleError::BundleInstallFailed(format!("reading tar archive failed: {err}"))
    })?;

    for entry in entries {
        let mut entry = entry.map_err(|err| {
            BundleError::BundleInstallFailed(format!("reading tar entry failed: {err}"))
        })?;
        let entry_path = entry.path().map_err(|err| {
            BundleError::BundleInstallFailed(format!("reading tar entry path failed: {err}"))
        })?;
        if !is_safe_archive_path(&entry_path) {
            return Err(BundleError::BundleInstallFailed(format!(
                "unsafe tar entry path: {}",
                entry_path.display()
            )));
        }
        entry.unpack_in(extract_dir).map_err(|err| {
            BundleError::BundleInstallFailed(format!("extracting tar entry failed: {err}"))
        })?;
    }

    Ok(())
}

fn is_safe_archive_path(path: &Path) -> bool {
    !path.components().any(|component| {
        matches!(
            component,
            Component::ParentDir | Component::RootDir | Component::Prefix(_)
        )
    })
}

fn locate_bundle_root(extract_dir: &Path) -> Result<PathBuf, BundleError> {
    if extract_dir.join("index.html").is_file() {
        return Ok(extract_dir.to_path_buf());
    }

    let entries = fs::read_dir(extract_dir).map_err(|err| {
        BundleError::BundleInstallFailed(format!("reading extract dir failed: {err}"))
    })?;

    let mut directories: Vec<PathBuf> = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|err| {
            BundleError::BundleInstallFailed(format!("reading extract entry failed: {err}"))
        })?;
        let path = entry.path();
        if path.is_dir() {
            directories.push(path);
        }
    }

    if directories.len() == 1 {
        let candidate = &directories[0];
        if candidate.join("index.html").is_file() {
            return Ok(candidate.to_path_buf());
        }
    }

    Err(BundleError::BundleInstallFailed(
        "bundle archive does not contain index.html".to_string(),
    ))
}

fn file_url_to_path(url: &str) -> Result<Option<PathBuf>, BundleError> {
    if !url.starts_with("file://") {
        return Ok(None);
    }

    let parsed = reqwest::Url::parse(url).map_err(|err| {
        BundleError::BundleDownloadFailed(format!("invalid file URL {url}: {err}"))
    })?;
    let path = parsed.to_file_path().map_err(|_| {
        BundleError::BundleDownloadFailed(format!("unsupported file URL path for {url}"))
    })?;
    Ok(Some(path))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::process_state::with_env_vars;
    use tempfile::TempDir;

    fn state_with_bundle_overrides(
        overrides: crate::api::DashboardBundleSourceOverrides,
    ) -> DashboardState {
        DashboardState {
            config_path: PathBuf::from("."),
            config_root: PathBuf::from("."),
            repo_root: PathBuf::from("."),
            repo_registry_path: None,
            mode: crate::api::ServeMode::HelloWorld,
            db: crate::api::DashboardDbPools::default(),
            bundle_dir: PathBuf::from("."),
            bundle_source_overrides: overrides,
            subscription_hub: crate::graphql::SubscriptionHub::new_arc(),
            dashboard_graphql_schema: crate::api::dashboard_schema::build_dashboard_schema_template(
            ),
            devql_schema: crate::graphql::build_global_schema_template(),
            devql_slim_schema: crate::graphql::build_slim_schema_template(),
        }
    }

    #[test]
    fn manifest_url_uses_compiled_default_when_env_missing() {
        with_env_vars(
            &[
                (DASHBOARD_MANIFEST_URL_ENV, None),
                (DASHBOARD_CDN_BASE_URL_ENV, None),
            ],
            || {
                let actual =
                    manifest_url_from_overrides(None).expect("manifest URL should resolve");
                assert_eq!(
                    actual,
                    dashboard_env::DASHBOARD_MANIFEST_URL,
                    "manifest URL should fall back to compiled value"
                );
            },
        );
    }

    #[test]
    fn manifest_url_prefers_manifest_env_override() {
        with_env_vars(
            &[
                (
                    DASHBOARD_MANIFEST_URL_ENV,
                    Some("https://override.example.com/bundle_versions.json"),
                ),
                (
                    DASHBOARD_CDN_BASE_URL_ENV,
                    Some("https://cdn-override.example.com/"),
                ),
            ],
            || {
                let actual =
                    manifest_url_from_overrides(None).expect("manifest URL should resolve");
                assert_eq!(
                    actual, "https://override.example.com/bundle_versions.json",
                    "manifest env override should have highest priority"
                );
            },
        );
    }

    #[test]
    fn resolve_entry_url_prefers_cdn_env_override_for_relative_paths() {
        with_env_vars(
            &[
                (DASHBOARD_MANIFEST_URL_ENV, None),
                (
                    DASHBOARD_CDN_BASE_URL_ENV,
                    Some("https://cdn-override.example.com/"),
                ),
            ],
            || {
                let actual = resolve_entry_url_from_overrides("bundle.tar.zst", None)
                    .expect("relative URL should resolve");
                assert_eq!(
                    actual, "https://cdn-override.example.com/bundle.tar.zst",
                    "relative URL should use CDN env override when present"
                );
            },
        );
    }

    #[test]
    fn manifest_url_for_state_prefers_state_manifest_override() {
        let state = state_with_bundle_overrides(crate::api::DashboardBundleSourceOverrides {
            cdn_base_url: Some("https://cdn-override.example.com/".to_string()),
            manifest_url: Some("https://override.example.com/bundle_versions.json".to_string()),
        });

        let actual = manifest_url_for_state(&state).expect("manifest URL should resolve");
        assert_eq!(actual, "https://override.example.com/bundle_versions.json");
    }

    #[test]
    fn resolve_entry_url_for_state_prefers_state_cdn_override_for_relative_paths() {
        let state = state_with_bundle_overrides(crate::api::DashboardBundleSourceOverrides {
            cdn_base_url: Some("https://cdn-override.example.com/".to_string()),
            manifest_url: None,
        });

        let actual = resolve_entry_url_from_overrides(
            "bundle.tar.zst",
            Some(&state.bundle_source_overrides),
        )
        .expect("relative URL should resolve");
        assert_eq!(actual, "https://cdn-override.example.com/bundle.tar.zst");
    }

    #[test]
    fn read_local_bundle_version_returns_none_when_missing() {
        let temp = TempDir::new().expect("temp dir");
        let version = read_local_bundle_version(temp.path()).expect("read local version");
        assert!(version.is_none());
    }

    #[test]
    fn read_local_bundle_version_returns_none_when_malformed() {
        let temp = TempDir::new().expect("temp dir");
        fs::write(temp.path().join("version.json"), "{invalid-json").expect("write malformed");
        let version = read_local_bundle_version(temp.path()).expect("read local version");
        assert!(version.is_none());
    }

    #[test]
    fn read_local_bundle_version_returns_trimmed_value() {
        let temp = TempDir::new().expect("temp dir");
        fs::write(
            temp.path().join("version.json"),
            r#"{"version":" 1.2.3 ","source_url":"https://cdn.test/bundle.tar.zst"}"#,
        )
        .expect("write version json");
        let version = read_local_bundle_version(temp.path()).expect("read local version");
        assert_eq!(version, Some("1.2.3".to_string()));
    }

    fn entry(version: &str, min: &str, max: &str) -> BundleVersionEntry {
        BundleVersionEntry {
            version: version.to_string(),
            min_required_cli_version: min.to_string(),
            max_required_cli_version: max.to_string(),
            download_url: format!("https://cdn.test/{version}/bundle.tar.zst"),
            checksum_url: format!("https://cdn.test/{version}/bundle.tar.zst.sha256"),
        }
    }

    #[test]
    fn is_entry_compatible_honors_min_max_and_latest() {
        let current = Version::parse("1.2.0").expect("parse current");

        assert!(is_entry_compatible(&entry("1.0.0", "1.0.0", "1.2.0"), &current).expect("compat"));
        assert!(
            !is_entry_compatible(&entry("1.0.0", "1.2.1", "latest"), &current).expect("compat")
        );
        assert!(!is_entry_compatible(&entry("1.0.0", "1.0.0", "1.1.9"), &current).expect("compat"));
        assert!(is_entry_compatible(&entry("1.0.0", "1.0.0", "latest"), &current).expect("compat"));
    }

    #[test]
    fn resolve_latest_applicable_returns_highest_compatible_version() {
        let current = Version::parse("1.4.0").expect("parse current");
        let versions = vec![
            entry("1.3.0", "1.0.0", "latest"),
            entry("1.5.0", "1.5.0", "latest"),
            entry("1.4.1", "1.0.0", "latest"),
        ];

        let resolved = resolve_latest_applicable_from_overrides(&versions, &current, None)
            .expect("resolve latest");
        assert_eq!(resolved.map(|v| v.version), Some("1.4.1".to_string()));
    }

    #[test]
    fn resolve_latest_applicable_returns_none_when_no_compatible_entries() {
        let current = Version::parse("1.4.0").expect("parse current");
        let versions = vec![
            entry("2.0.0", "2.0.0", "latest"),
            entry("3.0.0", "3.0.0", "latest"),
        ];

        let resolved = resolve_latest_applicable_from_overrides(&versions, &current, None)
            .expect("resolve latest");
        assert!(resolved.is_none());
    }

    #[test]
    fn resolve_latest_applicable_errors_on_invalid_semver() {
        let current = Version::parse("1.4.0").expect("parse current");
        let versions = vec![entry("not-semver", "1.0.0", "latest")];

        let err = resolve_latest_applicable_from_overrides(&versions, &current, None)
            .expect_err("should fail");
        match err {
            BundleError::ManifestParseFailed(_) => {}
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn parse_checksum_accepts_sha256sum_format() {
        let parsed = parse_checksum(
            "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef  bundle.tar.zst\n",
        );
        assert_eq!(
            parsed,
            Some("0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef".to_string())
        );
    }

    #[test]
    fn parse_checksum_rejects_invalid_payload() {
        assert_eq!(parse_checksum("not-a-checksum"), None);
        assert_eq!(parse_checksum("xyz"), None);
    }

    #[test]
    fn verify_sha256_returns_mismatch_for_wrong_hash() {
        let err = verify_sha256(
            b"bundle-bytes",
            "0000000000000000000000000000000000000000000000000000000000000000",
        )
        .expect_err("checksum should mismatch");
        match err {
            BundleError::ChecksumMismatch => {}
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn file_url_to_path_resolves_valid_file_url() {
        let resolved = file_url_to_path("file:///tmp/bundle.tar.zst").expect("file URL parse");
        assert_eq!(resolved, Some(PathBuf::from("/tmp/bundle.tar.zst")));
    }

    #[test]
    fn file_url_to_path_rejects_invalid_file_url() {
        let err =
            file_url_to_path("file://bad host/path").expect_err("invalid file URL should fail");
        match err {
            BundleError::BundleDownloadFailed(_) => {}
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn locate_bundle_root_accepts_single_nested_directory() {
        let temp = TempDir::new().expect("temp dir");
        let nested = temp.path().join("dashboard");
        fs::create_dir_all(&nested).expect("create nested");
        fs::write(nested.join("index.html"), "<html></html>").expect("write index");

        let root = locate_bundle_root(temp.path()).expect("should locate nested bundle root");
        assert_eq!(root, nested);
    }

    #[test]
    fn is_safe_archive_path_rejects_parent_and_absolute_paths() {
        assert!(!is_safe_archive_path(Path::new("../escape.txt")));
        assert!(!is_safe_archive_path(Path::new("/absolute/path.txt")));
        assert!(is_safe_archive_path(Path::new("assets/index.html")));
    }

    #[test]
    fn install_archive_atomically_preserves_existing_bundle_when_install_fails() {
        let temp = TempDir::new().expect("temp dir");
        let bundle_dir = temp.path().join("bundle");
        fs::create_dir_all(&bundle_dir).expect("create bundle dir");
        fs::write(bundle_dir.join("index.html"), "old bundle").expect("write old index");

        let mut tar_builder = tar::Builder::new(Vec::new());
        let payload = b"no index".to_vec();
        let mut header = tar::Header::new_gnu();
        header.set_size(payload.len() as u64);
        header.set_mode(0o644);
        header.set_cksum();
        tar_builder
            .append_data(&mut header, "README.txt", Cursor::new(payload))
            .expect("append readme");
        let tar_bytes = tar_builder.into_inner().expect("finalize tar");
        let archive = zstd::stream::encode_all(Cursor::new(tar_bytes), 0).expect("compress");

        let err =
            install_archive_atomically(&archive, &bundle_dir).expect_err("install should fail");
        match err {
            BundleError::BundleInstallFailed(_) => {}
            other => panic!("unexpected error: {other:?}"),
        }

        assert_eq!(
            fs::read_to_string(bundle_dir.join("index.html")).expect("read existing index"),
            "old bundle"
        );
    }

    #[test]
    fn unpack_archive_returns_install_failed_for_invalid_zstd_stream() {
        let temp = TempDir::new().expect("temp dir");
        let err = unpack_archive(b"not-zstd", temp.path()).expect_err("should fail");
        match err {
            BundleError::BundleInstallFailed(_) => {}
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn unpack_archive_returns_install_failed_for_invalid_tar_payload() {
        let temp = TempDir::new().expect("temp dir");
        let compressed =
            zstd::stream::encode_all(Cursor::new(b"not-a-tar".to_vec()), 0).expect("compress");
        let err = unpack_archive(&compressed, temp.path()).expect_err("should fail");
        match err {
            BundleError::BundleInstallFailed(_) => {}
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn resolve_latest_applicable_errors_on_invalid_max_pattern() {
        let current = Version::parse("1.2.0").expect("parse current");
        let versions = vec![entry("1.3.0", "1.0.0", "1.x.x")];

        let err = resolve_latest_applicable_from_overrides(&versions, &current, None)
            .expect_err("must fail");
        match err {
            BundleError::ManifestParseFailed(_) => {}
            other => panic!("unexpected error: {other:?}"),
        }
    }
}
