use std::time::Instant;

use axum::http::StatusCode;

use crate::api::bundle_types::BundleError;
use crate::api::dashboard_types::{DashboardBundleVersion, DashboardFetchBundleResult};
use crate::api::{ApiError, DashboardState};

pub(in crate::api) async fn check_dashboard_bundle_version(
    state: &DashboardState,
) -> std::result::Result<DashboardBundleVersion, ApiError> {
    let started = Instant::now();
    let bundle_dir = state.bundle_dir.display().to_string();
    log::info!(
        "event=dashboard.bundle.check.started operation=checkBundleVersion status=started bundle_dir={bundle_dir}"
    );

    let response = match crate::api::bundle::check_bundle_version(state).await {
        Ok(result) => {
            let latest_applicable_version = result
                .latest_applicable_version
                .clone()
                .unwrap_or_else(|| "null".to_string());
            log::info!(
                "event=dashboard.bundle.check.succeeded operation=checkBundleVersion status=succeeded bundle_dir={} install_available={} latest_applicable_version={}",
                bundle_dir,
                result.install_available,
                latest_applicable_version
            );
            Ok(DashboardBundleVersion {
                current_version: result.current_version,
                latest_applicable_version: result.latest_applicable_version,
                install_available: result.install_available,
                reason: result.reason.as_str().to_string(),
            })
        }
        Err(error) => {
            let error_code = bundle_error_code(&error);
            let api_error = map_bundle_error(error);
            log::error!(
                "event=dashboard.bundle.check.failed operation=checkBundleVersion status=failed bundle_dir={} error_code={} error_message={}",
                bundle_dir,
                error_code,
                api_error.message
            );
            Err(api_error)
        }
    };

    log::debug!(
        "dashboard checkBundleVersion completed in {}ms",
        started.elapsed().as_millis()
    );
    response
}

pub(in crate::api) async fn fetch_dashboard_bundle(
    state: &DashboardState,
) -> std::result::Result<DashboardFetchBundleResult, ApiError> {
    let started = Instant::now();
    let bundle_dir = state.bundle_dir.display().to_string();
    log::info!(
        "event=dashboard.bundle.install.started operation=fetchBundle status=started bundle_dir={bundle_dir}"
    );

    let response = match crate::api::bundle::fetch_bundle(state).await {
        Ok(result) => {
            log::info!(
                "event=dashboard.bundle.install.succeeded operation=fetchBundle status=succeeded bundle_dir={} installed_version={} checksum_verified={}",
                result.bundle_dir,
                result.installed_version,
                result.checksum_verified
            );
            Ok(DashboardFetchBundleResult {
                installed_version: result.installed_version,
                bundle_dir: result.bundle_dir,
                status: result.status,
                checksum_verified: result.checksum_verified,
            })
        }
        Err(BundleError::ChecksumMismatch) => {
            log::warn!(
                "event=dashboard.bundle.install.checksum_mismatch operation=fetchBundle status=failed bundle_dir={} error_code=checksum_mismatch",
                bundle_dir
            );
            let api_error = map_bundle_error(BundleError::ChecksumMismatch);
            log::error!(
                "event=dashboard.bundle.install.failed operation=fetchBundle status=failed bundle_dir={} error_code=checksum_mismatch error_message={}",
                bundle_dir,
                api_error.message
            );
            Err(api_error)
        }
        Err(error) => {
            let error_code = bundle_error_code(&error);
            let api_error = map_bundle_error(error);
            log::error!(
                "event=dashboard.bundle.install.failed operation=fetchBundle status=failed bundle_dir={} error_code={} error_message={}",
                bundle_dir,
                error_code,
                api_error.message
            );
            Err(api_error)
        }
    };

    log::debug!(
        "dashboard fetchBundle completed in {}ms",
        started.elapsed().as_millis()
    );
    response
}

fn map_bundle_error(error: BundleError) -> ApiError {
    match error {
        BundleError::ManifestFetchFailed(message) => {
            ApiError::with_code(StatusCode::BAD_GATEWAY, "manifest_fetch_failed", message)
        }
        BundleError::ManifestParseFailed(message) => {
            ApiError::with_code(StatusCode::INTERNAL_SERVER_ERROR, "internal", message)
        }
        BundleError::NoCompatibleVersion => ApiError::with_code(
            StatusCode::CONFLICT,
            "no_compatible_version",
            "no compatible dashboard bundle version is available for this CLI version",
        ),
        BundleError::BundleDownloadFailed(message) => {
            ApiError::with_code(StatusCode::BAD_GATEWAY, "bundle_download_failed", message)
        }
        BundleError::ChecksumMismatch => ApiError::with_code(
            StatusCode::UNPROCESSABLE_ENTITY,
            "checksum_mismatch",
            "downloaded bundle checksum did not match",
        ),
        BundleError::BundleInstallFailed(message) => ApiError::with_code(
            StatusCode::INTERNAL_SERVER_ERROR,
            "bundle_install_failed",
            message,
        ),
        BundleError::Internal(message) => {
            ApiError::with_code(StatusCode::INTERNAL_SERVER_ERROR, "internal", message)
        }
    }
}

fn bundle_error_code(error: &BundleError) -> &'static str {
    match error {
        BundleError::ManifestFetchFailed(_) => "manifest_fetch_failed",
        BundleError::ManifestParseFailed(_) => "internal",
        BundleError::NoCompatibleVersion => "no_compatible_version",
        BundleError::BundleDownloadFailed(_) => "bundle_download_failed",
        BundleError::ChecksumMismatch => "checksum_mismatch",
        BundleError::BundleInstallFailed(_) => "bundle_install_failed",
        BundleError::Internal(_) => "internal",
    }
}
