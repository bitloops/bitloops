use std::path::Path;

use axum::{
    body::Body,
    extract::{Path as AxumPath, State},
    http::{HeaderValue, StatusCode, header},
    response::Response,
};

use super::super::dto::{ApiError, ApiErrorEnvelope};
use super::resolve_repo_root_from_repo_id;
use crate::api::DashboardState;
use crate::host::checkpoints::strategy::manual_commit::new_git_command;
use std::process::Stdio;

/// Upper bound for `GET /api/blobs/...` body size (via `git cat-file -s` before reading bytes).
#[cfg(test)]
pub(crate) const MAX_GIT_BLOB_BYTES: u64 = 64 * 1024;
#[cfg(not(test))]
pub(crate) const MAX_GIT_BLOB_BYTES: u64 = 10 * 1024 * 1024;

fn validate_git_blob_oid(blob_sha: &str) -> Result<String, ApiError> {
    let s = blob_sha.trim();
    // 40 = standard SHA-1 object ids; 64 = SHA-256 object format repositories (`extensions.objectFormat`).
    if s.len() != 40 && s.len() != 64 {
        return Err(ApiError::bad_request(
            "blob_sha must be 40 (sha1) or 64 (sha256) hex characters",
        ));
    }
    if !s.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(ApiError::bad_request(
            "blob_sha must contain only hexadecimal characters",
        ));
    }
    Ok(s.to_ascii_lowercase())
}

fn git_cat_file_blob_bytes(repo_root: &Path, blob_sha: &str) -> Result<Vec<u8>, ApiError> {
    let mut type_cmd = new_git_command();
    type_cmd
        .args(["cat-file", "-t", blob_sha])
        .current_dir(repo_root)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let type_out = type_cmd
        .output()
        .map_err(|err| ApiError::internal(format!("failed to spawn git cat-file -t: {err:#}")))?;

    if !type_out.status.success() {
        let stderr = String::from_utf8_lossy(&type_out.stderr);
        if stderr.to_ascii_lowercase().contains("not a git repository") {
            return Err(ApiError::internal(format!(
                "git cat-file -t failed: {}",
                stderr.trim()
            )));
        }
        return Err(ApiError::not_found(format!(
            "git object not found: {blob_sha}"
        )));
    }

    let kind = String::from_utf8_lossy(&type_out.stdout)
        .trim()
        .to_ascii_lowercase();
    if kind != "blob" {
        return Err(ApiError::bad_request(format!(
            "git object {blob_sha} is not a blob (type `{kind}`)"
        )));
    }

    let mut size_cmd = new_git_command();
    size_cmd
        .args(["cat-file", "-s", blob_sha])
        .current_dir(repo_root)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let size_out = size_cmd
        .output()
        .map_err(|err| ApiError::internal(format!("failed to spawn git cat-file -s: {err:#}")))?;

    if !size_out.status.success() {
        let stderr = String::from_utf8_lossy(&size_out.stderr);
        return Err(ApiError::internal(format!(
            "git cat-file -s failed: {}",
            stderr.trim()
        )));
    }

    let size_line = String::from_utf8_lossy(&size_out.stdout);
    let blob_size: u64 = size_line.trim().parse().map_err(|_| {
        ApiError::internal(format!(
            "git cat-file -s returned non-numeric size: {:?}",
            size_line.trim()
        ))
    })?;

    if blob_size > MAX_GIT_BLOB_BYTES {
        return Err(ApiError::payload_too_large(format!(
            "blob size {blob_size} bytes exceeds maximum of {MAX_GIT_BLOB_BYTES} bytes"
        )));
    }

    let mut cmd = new_git_command();
    cmd.args(["cat-file", "blob", blob_sha])
        .current_dir(repo_root)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let output = cmd
        .output()
        .map_err(|err| ApiError::internal(format!("failed to spawn git cat-file blob: {err:#}")))?;

    if output.status.success() {
        return Ok(output.stdout);
    }

    let stderr = String::from_utf8_lossy(&output.stderr);
    Err(ApiError::internal(format!(
        "git cat-file blob failed: {}",
        stderr.trim()
    )))
}

#[utoipa::path(
    get,
    path = "/api/blobs/{repo_id}/{blob_sha}",
    params(
        ("repo_id" = String, Path, description = "Repository id"),
        ("blob_sha" = String, Path, description = "Git blob object id (40 or 64 hex characters)")
    ),
    responses(
        (status = 200, description = "Raw git blob bytes", content_type = "application/octet-stream"),
        (status = 400, description = "Bad request", body = ApiErrorEnvelope),
        (status = 404, description = "Not found", body = ApiErrorEnvelope),
        (status = 413, description = "Blob larger than configured maximum", body = ApiErrorEnvelope),
        (status = 500, description = "Internal server error", body = ApiErrorEnvelope)
    )
)]
pub(crate) async fn handle_api_git_blob(
    State(state): State<DashboardState>,
    AxumPath((repo_id, blob_sha)): AxumPath<(String, String)>,
) -> Result<Response, ApiError> {
    let repo_root = resolve_repo_root_from_repo_id(&state, &repo_id).await?;
    let blob_sha = validate_git_blob_oid(&blob_sha)?;
    let bytes = tokio::task::spawn_blocking(move || git_cat_file_blob_bytes(&repo_root, &blob_sha))
        .await
        .map_err(|err| ApiError::internal(format!("git blob read task failed: {err}")))??;

    let mut response = Response::new(Body::from(bytes));
    *response.status_mut() = StatusCode::OK;
    // Raw object bytes: octet-stream is correct. Path-based MIME (e.g. text/plain for .ts) can be a follow-up if the API gains an optional filepath hint.
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/octet-stream"),
    );
    Ok(response)
}
