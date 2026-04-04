use axum::{Json, extract::State, http::StatusCode};
use serde_json::Value;
use std::collections::HashMap;
use std::time::Instant;

use super::super::dto::{
    ApiCheckBundleVersionResponse, ApiError, ApiErrorEnvelope, ApiFetchBundleResponse,
};
use super::super::{DashboardState, bundle, bundle_types::BundleError};

#[utoipa::path(
    get,
    path = "/api/check_bundle_version",
    responses(
        (status = 200, description = "Dashboard bundle install/update availability", body = ApiCheckBundleVersionResponse),
        (status = 502, description = "Manifest fetch failure", body = ApiErrorEnvelope),
        (status = 500, description = "Internal server error", body = ApiErrorEnvelope)
    )
)]
pub(crate) async fn handle_api_check_bundle_version(
    State(state): State<DashboardState>,
) -> std::result::Result<Json<ApiCheckBundleVersionResponse>, ApiError> {
    let started = Instant::now();
    let bundle_dir = state.bundle_dir.display().to_string();
    log::info!(
        "event=dashboard.bundle.check.started operation=check_bundle_version status=started bundle_dir={bundle_dir}"
    );

    let response = match bundle::check_bundle_version(&state).await {
        Ok(result) => {
            let latest_applicable_version = result
                .latest_applicable_version
                .clone()
                .unwrap_or_else(|| "null".to_string());
            log::info!(
                "event=dashboard.bundle.check.succeeded operation=check_bundle_version status=succeeded bundle_dir={} install_available={} latest_applicable_version={}",
                bundle_dir,
                result.install_available,
                latest_applicable_version
            );
            Ok(Json(ApiCheckBundleVersionResponse {
                current_version: result.current_version,
                latest_applicable_version: result.latest_applicable_version,
                install_available: result.install_available,
                reason: result.reason.as_str().to_string(),
            }))
        }
        Err(error) => {
            let error_code = bundle_error_code(&error);
            let api_error = map_bundle_error(error);
            log::error!(
                "event=dashboard.bundle.check.failed operation=check_bundle_version status=failed bundle_dir={} error_code={} error_message={}",
                bundle_dir,
                error_code,
                api_error.message
            );
            Err(api_error)
        }
    };
    track_bundle_action(
        &state,
        "bitloops dashboard api bundle-check",
        "GET",
        started.elapsed(),
        &response,
    );
    response
}

#[utoipa::path(
    post,
    path = "/api/fetch_bundle",
    request_body = inline(serde_json::Value),
    responses(
        (status = 200, description = "Bundle fetched and installed", body = ApiFetchBundleResponse),
        (status = 409, description = "No compatible version", body = ApiErrorEnvelope),
        (status = 422, description = "Checksum mismatch", body = ApiErrorEnvelope),
        (status = 502, description = "Download/manifest fetch failure", body = ApiErrorEnvelope),
        (status = 500, description = "Install failure", body = ApiErrorEnvelope)
    )
)]
pub(crate) async fn handle_api_fetch_bundle(
    State(state): State<DashboardState>,
) -> std::result::Result<Json<ApiFetchBundleResponse>, ApiError> {
    let started = Instant::now();
    let bundle_dir = state.bundle_dir.display().to_string();
    log::info!(
        "event=dashboard.bundle.install.started operation=fetch_bundle status=started bundle_dir={bundle_dir}"
    );

    let response = match bundle::fetch_bundle(&state).await {
        Ok(result) => {
            log::info!(
                "event=dashboard.bundle.install.succeeded operation=fetch_bundle status=succeeded bundle_dir={} installed_version={} checksum_verified={}",
                result.bundle_dir,
                result.installed_version,
                result.checksum_verified
            );
            Ok(Json(ApiFetchBundleResponse {
                installed_version: result.installed_version,
                bundle_dir: result.bundle_dir,
                status: result.status,
                checksum_verified: result.checksum_verified,
            }))
        }
        Err(BundleError::ChecksumMismatch) => {
            log::warn!(
                "event=dashboard.bundle.install.checksum_mismatch operation=fetch_bundle status=failed bundle_dir={} error_code=checksum_mismatch",
                bundle_dir
            );
            let api_error = map_bundle_error(BundleError::ChecksumMismatch);
            log::error!(
                "event=dashboard.bundle.install.failed operation=fetch_bundle status=failed bundle_dir={} error_code=checksum_mismatch error_message={}",
                bundle_dir,
                api_error.message
            );
            Err(api_error)
        }
        Err(error) => {
            let error_code = bundle_error_code(&error);
            let api_error = map_bundle_error(error);
            log::error!(
                "event=dashboard.bundle.install.failed operation=fetch_bundle status=failed bundle_dir={} error_code={} error_message={}",
                bundle_dir,
                error_code,
                api_error.message
            );
            Err(api_error)
        }
    };
    track_bundle_action(
        &state,
        "bitloops dashboard api bundle-fetch",
        "POST",
        started.elapsed(),
        &response,
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

fn track_bundle_action<T>(
    state: &DashboardState,
    event: &str,
    method: &str,
    duration: std::time::Duration,
    result: &std::result::Result<Json<T>, ApiError>,
) {
    let status = match result {
        Ok(_) => StatusCode::OK,
        Err(err) => err.status_code(),
    };
    let mut properties = HashMap::new();
    properties.insert("http_method".to_string(), Value::String(method.to_string()));
    properties.insert(
        "status_code_class".to_string(),
        Value::String(super::super::status_code_class(status).to_string()),
    );
    super::super::track_repo_action(
        &state.repo_root,
        crate::telemetry::analytics::ActionDescriptor {
            event: event.to_string(),
            surface: "dashboard",
            properties,
        },
        status.is_success(),
        duration,
    );
}
