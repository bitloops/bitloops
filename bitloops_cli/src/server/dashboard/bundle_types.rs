use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize)]
pub(super) struct BundleVersionsManifest {
    pub(super) versions: Vec<BundleVersionEntry>,
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct BundleVersionEntry {
    pub(super) version: String,
    pub(super) min_required_cli_version: String,
    pub(super) max_required_cli_version: String,
    pub(super) download_url: String,
    pub(super) checksum_url: String,
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct LocalBundleVersionFile {
    pub(super) version: String,
    pub(super) source_url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum BundleVersionReason {
    NotInstalled,
    UpdateAvailable,
    UpToDate,
    NoCompatibleVersion,
}

impl BundleVersionReason {
    pub(super) fn as_str(&self) -> &'static str {
        match self {
            Self::NotInstalled => "not_installed",
            Self::UpdateAvailable => "update_available",
            Self::UpToDate => "up_to_date",
            Self::NoCompatibleVersion => "no_compatible_version",
        }
    }
}

#[derive(Debug, Clone)]
pub(super) struct BundleCheckResult {
    pub(super) current_version: Option<String>,
    pub(super) latest_applicable_version: Option<String>,
    pub(super) install_available: bool,
    pub(super) reason: BundleVersionReason,
}

#[derive(Debug, Clone)]
pub(super) struct BundleInstallResult {
    pub(super) installed_version: String,
    pub(super) bundle_dir: String,
    pub(super) status: String,
    pub(super) checksum_verified: bool,
}

#[derive(Debug, Clone)]
pub(super) struct ResolvedBundleVersion {
    pub(super) version: String,
    pub(super) download_url: String,
    pub(super) checksum_url: String,
}

#[derive(Debug, Clone)]
pub(super) enum BundleError {
    ManifestFetchFailed(String),
    ManifestParseFailed(String),
    NoCompatibleVersion,
    BundleDownloadFailed(String),
    ChecksumMismatch,
    BundleInstallFailed(String),
    Internal(String),
}
