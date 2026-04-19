//! Serial download paths used when range requests are unavailable, when the
//! asset is too small to benefit from parallelism, or when the probe response
//! itself can be reused as the body.

use anyhow::{Context, Result};
use reqwest::blocking::{Client, Response};
use sha2::{Digest, Sha256};
use std::fs::File;
use std::io::{Read, Write};
use std::time::Instant;

use super::config::MANAGED_RELEASE_DOWNLOAD_BUFFER_BYTES;
use super::http::{managed_download_request, temporary_download_path};
use super::types::DownloadedManagedAsset;

/// Re-fetch the asset using a plain `GET` and stream the body to a temporary
/// file. Used when the server signalled that range requests are supported but
/// the asset is too small to make parallel ranges worthwhile.
pub(crate) fn download_release_asset_to_temp_file_serial(
    client: &Client,
    url: &str,
    user_agent: &str,
    asset_label: &str,
    progress: impl FnMut(u64, Option<u64>) -> Result<()>,
) -> Result<DownloadedManagedAsset> {
    let transfer_started = Instant::now();
    let response = managed_download_request(client, url, user_agent)
        .send()
        .with_context(|| format!("downloading {asset_label} from {url}"))?
        .error_for_status()
        .with_context(|| format!("downloading {asset_label} from {url}"))?;
    let download = download_response_to_temp_file(response, asset_label, progress)?;
    log::info!(
        "managed runtime download complete: asset_label={asset_label} mode=serial_refetch bytes_downloaded={} total_bytes={} transfer_ms={}",
        download.bytes_downloaded,
        download.bytes_total.unwrap_or(download.bytes_downloaded),
        transfer_started.elapsed().as_millis()
    );
    Ok(download)
}

/// Stream an already-issued response body to a temporary file while updating
/// the SHA-256 hasher and the caller-supplied progress callback.
pub(crate) fn download_response_to_temp_file(
    mut response: Response,
    asset_label: &str,
    mut progress: impl FnMut(u64, Option<u64>) -> Result<()>,
) -> Result<DownloadedManagedAsset> {
    let mut download = DownloadedManagedAsset {
        path: temporary_download_path(asset_label),
        bytes_downloaded: 0,
        bytes_total: response.content_length(),
        sha256_hex: String::new(),
    };
    let mut file = File::create(download.path()).with_context(|| {
        format!(
            "creating temporary download file {}",
            download.path().display()
        )
    })?;
    let mut hasher = Sha256::new();
    let mut chunk = [0_u8; MANAGED_RELEASE_DOWNLOAD_BUFFER_BYTES];
    progress(download.bytes_downloaded, download.bytes_total)?;
    loop {
        let read = response
            .read(&mut chunk)
            .with_context(|| format!("reading {asset_label} bytes"))?;
        if read == 0 {
            break;
        }
        file.write_all(&chunk[..read]).with_context(|| {
            format!(
                "writing temporary download file {}",
                download.path().display()
            )
        })?;
        hasher.update(&chunk[..read]);
        download.bytes_downloaded = download.bytes_downloaded.saturating_add(read as u64);
        progress(download.bytes_downloaded, download.bytes_total)?;
    }
    file.flush().with_context(|| {
        format!(
            "flushing temporary download file {}",
            download.path().display()
        )
    })?;
    download.sha256_hex = hex::encode(hasher.finalize());
    Ok(download)
}
