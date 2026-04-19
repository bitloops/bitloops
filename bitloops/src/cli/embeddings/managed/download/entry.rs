//! Top-level dispatcher that probes the server with a single-byte range
//! request and then chooses between the parallel and serial download paths.

use anyhow::{Context, Result};
use reqwest::StatusCode;
use reqwest::blocking::Client;
use reqwest::header::{CONTENT_RANGE, RANGE};
use std::time::Instant;

use super::http::{managed_download_request, parse_content_range_header};
use super::parallel::{
    choose_parallel_download_ranges, download_release_asset_to_temp_file_in_parallel,
};
use super::serial::{download_release_asset_to_temp_file_serial, download_response_to_temp_file};
use super::types::DownloadedManagedAsset;

/// Download `asset_label` from `url` into a temporary file, picking the most
/// appropriate strategy based on a small probe request:
///
/// * a parallel range download when the server reports `Accept-Ranges` and
///   the asset is large enough to justify multiple workers;
/// * a serial re-fetch when range requests are supported but the asset is too
///   small for parallelism;
/// * reusing the probe response otherwise.
///
/// `progress` receives `(bytes_downloaded, bytes_total)` updates throughout
/// the download and may abort the transfer by returning an error.
pub(crate) fn download_release_asset_to_temp_file(
    client: &Client,
    url: &str,
    user_agent: &str,
    asset_label: &str,
    progress: impl FnMut(u64, Option<u64>) -> Result<()>,
) -> Result<DownloadedManagedAsset> {
    let probe_started = Instant::now();
    let response = managed_download_request(client, url, user_agent)
        .header(RANGE, "bytes=0-0")
        .send()
        .with_context(|| format!("downloading {asset_label} from {url}"))?
        .error_for_status()
        .with_context(|| format!("downloading {asset_label} from {url}"))?;
    let probe_elapsed = probe_started.elapsed();
    let content_range = response
        .headers()
        .get(CONTENT_RANGE)
        .and_then(parse_content_range_header);

    if response.status() == StatusCode::PARTIAL_CONTENT
        && let Some(content_range) = content_range
        && let Some(part_ranges) = choose_parallel_download_ranges(content_range.total)
    {
        log::info!(
            "managed runtime download branch: asset_label={asset_label} mode=parallel probe_ms={} total_bytes={} part_count={}",
            probe_elapsed.as_millis(),
            content_range.total,
            part_ranges.len()
        );
        drop(response);
        return download_release_asset_to_temp_file_in_parallel(
            client,
            url,
            user_agent,
            asset_label,
            content_range.total,
            &part_ranges,
            progress,
        );
    }

    if response.status() == StatusCode::PARTIAL_CONTENT {
        if let Some(content_range) = content_range {
            log::info!(
                "managed runtime download branch: asset_label={asset_label} mode=serial_refetch probe_ms={} total_bytes={} reason=below_parallel_threshold",
                probe_elapsed.as_millis(),
                content_range.total
            );
        } else {
            log::info!(
                "managed runtime download branch: asset_label={asset_label} mode=serial_refetch probe_ms={} reason=missing_content_range",
                probe_elapsed.as_millis()
            );
        }
        drop(response);
        return download_release_asset_to_temp_file_serial(
            client,
            url,
            user_agent,
            asset_label,
            progress,
        );
    }

    log::info!(
        "managed runtime download branch: asset_label={asset_label} mode=serial_reuse_probe_response probe_ms={} status={}",
        probe_elapsed.as_millis(),
        response.status().as_u16()
    );
    let transfer_started = Instant::now();
    let download = download_response_to_temp_file(response, asset_label, progress)?;
    log::info!(
        "managed runtime download complete: asset_label={asset_label} mode=serial_reuse_probe_response bytes_downloaded={} total_bytes={} transfer_ms={}",
        download.bytes_downloaded,
        download.bytes_total.unwrap_or(download.bytes_downloaded),
        transfer_started.elapsed().as_millis()
    );
    Ok(download)
}
