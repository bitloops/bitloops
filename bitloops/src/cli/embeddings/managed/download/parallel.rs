//! Parallel range-based downloading and the helpers it needs to plan, fetch,
//! retry and merge byte-range chunks back into a single file.

use anyhow::{Context, Result};
use reqwest::StatusCode;
use reqwest::blocking::Client;
use reqwest::header::{CONTENT_RANGE, RANGE};
use sha2::{Digest, Sha256};
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use super::config::{
    MANAGED_RELEASE_DOWNLOAD_BUFFER_BYTES, MAX_PARALLEL_DOWNLOAD_PARTS,
    MIN_PARALLEL_DOWNLOAD_BYTES, TARGET_PARALLEL_DOWNLOAD_PART_BYTES,
};
#[cfg(not(test))]
use super::config::{PARALLEL_RANGE_TIMEOUT_GRACE_SECS, PARALLEL_RANGE_TIMEOUT_MIN_BYTES_PER_SEC};
use super::http::{
    managed_download_request, temporary_download_path, validate_content_range_header,
};
use super::types::{DownloadByteRange, DownloadedManagedAsset};

/// A single byte-range chunk that has been written to disk; the temporary file
/// is removed when the value is dropped so failed downloads do not leak.
#[derive(Debug)]
struct DownloadedManagedChunk {
    path: PathBuf,
    bytes_downloaded: u64,
}

impl Drop for DownloadedManagedChunk {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

/// Messages sent from worker threads back to the orchestrator.
enum WorkerEvent {
    Progress(usize, u64),
    Done(usize, Result<DownloadedManagedChunk, String>),
}

/// Shared parameters cloned into each parallel range worker.
#[derive(Clone)]
struct DownloadByteRangeRequest {
    client: Client,
    url: String,
    user_agent: String,
    asset_label: String,
    total_bytes: u64,
    abort: Arc<AtomicBool>,
}

impl DownloadByteRangeRequest {
    fn new(
        client: &Client,
        url: &str,
        user_agent: &str,
        asset_label: &str,
        total_bytes: u64,
        abort: Arc<AtomicBool>,
    ) -> Self {
        Self {
            client: client.clone(),
            url: url.to_string(),
            user_agent: user_agent.to_string(),
            asset_label: asset_label.to_string(),
            total_bytes,
            abort,
        }
    }
}

/// Drive a parallel range-based download: spawn a worker per part, feed the
/// caller progress updates, retry stragglers serially and finally merge the
/// chunks into a single temporary file.
pub(crate) fn download_release_asset_to_temp_file_in_parallel(
    client: &Client,
    url: &str,
    user_agent: &str,
    asset_label: &str,
    total_bytes: u64,
    part_ranges: &[DownloadByteRange],
    mut progress: impl FnMut(u64, Option<u64>) -> Result<()>,
) -> Result<DownloadedManagedAsset> {
    let transfer_started = Instant::now();
    let mut download = DownloadedManagedAsset {
        path: temporary_download_path(asset_label),
        bytes_downloaded: 0,
        bytes_total: Some(total_bytes),
        sha256_hex: String::new(),
    };
    progress(download.bytes_downloaded, download.bytes_total)?;
    let abort = Arc::new(AtomicBool::new(false));
    let (tx, rx) = mpsc::channel();
    let mut handles = Vec::with_capacity(part_ranges.len());
    for (index, range) in part_ranges.iter().copied().enumerate() {
        let tx = tx.clone();
        let request = DownloadByteRangeRequest::new(
            client,
            url,
            user_agent,
            asset_label,
            total_bytes,
            Arc::clone(&abort),
        );
        handles.push(thread::spawn(move || {
            let result = download_byte_range_to_temp_file(
                &request,
                range,
                Some(parallel_range_request_timeout(range)),
                |delta| {
                    let _ = tx.send(WorkerEvent::Progress(index, delta));
                    Ok(())
                },
            )
            .map_err(|err| err.to_string());
            let _ = tx.send(WorkerEvent::Done(index, result));
        }));
    }
    drop(tx);

    let mut progress_error = None;
    let mut worker_error = None;
    let mut completed_workers = 0_usize;
    let mut worker_bytes_downloaded = vec![0_u64; part_ranges.len()];
    let mut chunks = std::iter::repeat_with(|| None)
        .take(part_ranges.len())
        .collect::<Vec<_>>();
    while completed_workers < part_ranges.len() {
        match rx.recv() {
            Ok(WorkerEvent::Progress(index, delta)) => {
                if progress_error.is_none() && worker_error.is_none() {
                    worker_bytes_downloaded[index] =
                        worker_bytes_downloaded[index].saturating_add(delta);
                    download.bytes_downloaded = worker_bytes_downloaded.iter().sum();
                    if let Err(err) = progress(download.bytes_downloaded, download.bytes_total) {
                        progress_error = Some(err);
                        abort.store(true, Ordering::SeqCst);
                    }
                }
            }
            Ok(WorkerEvent::Done(index, result)) => {
                completed_workers = completed_workers.saturating_add(1);
                match result {
                    Ok(chunk) => {
                        worker_bytes_downloaded[index] = chunk.bytes_downloaded;
                        chunks[index] = Some(chunk);
                    }
                    Err(err) => {
                        if worker_error.is_none() {
                            worker_error = Some(format!(
                                "range {}-{} failed: {err}",
                                part_ranges[index].start, part_ranges[index].end
                            ));
                            abort.store(true, Ordering::SeqCst);
                        }
                    }
                }
            }
            Err(_) => {
                if worker_error.is_none() {
                    worker_error = Some(
                        "parallel managed runtime download channel closed unexpectedly".to_string(),
                    );
                    abort.store(true, Ordering::SeqCst);
                }
                break;
            }
        }
    }

    for handle in handles {
        handle
            .join()
            .map_err(|_| anyhow::anyhow!("parallel managed runtime download worker panicked"))?;
    }
    if let Some(err) = progress_error {
        return Err(err);
    }

    let transfer_elapsed = transfer_started.elapsed();
    let mut retried_parts = 0_usize;
    if let Some(err) = worker_error {
        let completed_part_count = chunks.iter().filter(|chunk| chunk.is_some()).count();
        log::warn!(
            "managed runtime parallel download degraded to serial retries: asset_label={asset_label} total_bytes={} completed_parts={} part_count={} transfer_ms={} reason={}",
            total_bytes,
            completed_part_count,
            part_ranges.len(),
            transfer_elapsed.as_millis(),
            err
        );
        download.bytes_downloaded = chunks
            .iter()
            .flatten()
            .map(|chunk| chunk.bytes_downloaded)
            .sum();
        progress(download.bytes_downloaded, download.bytes_total)?;
        let retry_request = DownloadByteRangeRequest::new(
            client,
            url,
            user_agent,
            asset_label,
            total_bytes,
            Arc::new(AtomicBool::new(false)),
        );
        retried_parts = retry_missing_parallel_ranges_serially(
            &retry_request,
            part_ranges,
            &mut chunks,
            &mut download.bytes_downloaded,
            &mut progress,
        )?;
    }

    let merge_started = Instant::now();
    let chunks = chunks
        .into_iter()
        .map(|chunk| chunk.context("parallel managed runtime download finished without a chunk"))
        .collect::<Result<Vec<_>>>()?;
    download.sha256_hex = merge_downloaded_chunks_into_file(download.path(), &chunks)?;
    let merge_elapsed = merge_started.elapsed();
    download.bytes_downloaded = total_bytes;
    progress(download.bytes_downloaded, download.bytes_total)?;
    log::info!(
        "managed runtime download complete: asset_label={asset_label} mode=parallel bytes_downloaded={} total_bytes={} part_count={} retried_parts={} transfer_ms={} merge_ms={}",
        download.bytes_downloaded,
        download.bytes_total.unwrap_or(download.bytes_downloaded),
        part_ranges.len(),
        retried_parts,
        transfer_elapsed.as_millis(),
        merge_elapsed.as_millis()
    );
    Ok(download)
}

/// Download a single byte range into a temporary chunk file, validating that
/// the server honoured the requested range and respecting both the abort flag
/// and an optional overall timeout.
fn download_byte_range_to_temp_file(
    request: &DownloadByteRangeRequest,
    range: DownloadByteRange,
    request_timeout: Option<Duration>,
    mut on_progress: impl FnMut(u64) -> Result<()>,
) -> Result<DownloadedManagedChunk> {
    let path = temporary_download_path(&format!(
        "{}-{}-{}",
        request.asset_label, range.start, range.end
    ));
    let result = (|| {
        let mut request_builder =
            managed_download_request(&request.client, &request.url, &request.user_agent)
                .header(RANGE, format!("bytes={}-{}", range.start, range.end));
        if let Some(timeout) = request_timeout {
            request_builder = request_builder.timeout(timeout);
        }
        let mut response = request_builder
            .send()
            .with_context(|| {
                format!(
                    "downloading {} range {}-{}",
                    request.asset_label, range.start, range.end
                )
            })?
            .error_for_status()
            .with_context(|| {
                format!(
                    "downloading {} range {}-{}",
                    request.asset_label, range.start, range.end
                )
            })?;
        if response.status() != StatusCode::PARTIAL_CONTENT {
            anyhow::bail!(
                "managed runtime range request {}-{} returned HTTP {} instead of 206",
                range.start,
                range.end,
                response.status()
            );
        }
        validate_content_range_header(
            response.headers().get(CONTENT_RANGE),
            range,
            request.total_bytes,
        )?;

        let mut file = File::create(&path)
            .with_context(|| format!("creating temporary chunk file {}", path.display()))?;
        let mut bytes_downloaded = 0_u64;
        let mut chunk = [0_u8; MANAGED_RELEASE_DOWNLOAD_BUFFER_BYTES];
        let started = Instant::now();
        loop {
            if request.abort.load(Ordering::SeqCst) {
                anyhow::bail!("parallel managed runtime download aborted");
            }
            if let Some(timeout) = request_timeout
                && started.elapsed() > timeout
            {
                anyhow::bail!(
                    "managed runtime range request {}-{} exceeded {} ms after downloading {} of {} bytes",
                    range.start,
                    range.end,
                    timeout.as_millis(),
                    bytes_downloaded,
                    range.len()
                );
            }
            let read = response.read(&mut chunk).with_context(|| {
                format!(
                    "reading {} range {}-{} bytes",
                    request.asset_label, range.start, range.end
                )
            })?;
            if read == 0 {
                break;
            }
            file.write_all(&chunk[..read])
                .with_context(|| format!("writing temporary chunk file {}", path.display()))?;
            bytes_downloaded = bytes_downloaded.saturating_add(read as u64);
            on_progress(read as u64)?;
        }
        file.flush()
            .with_context(|| format!("flushing temporary chunk file {}", path.display()))?;
        if bytes_downloaded != range.len() {
            anyhow::bail!(
                "managed runtime range request {}-{} downloaded {} bytes instead of {}",
                range.start,
                range.end,
                bytes_downloaded,
                range.len()
            );
        }

        Ok(DownloadedManagedChunk {
            path: path.clone(),
            bytes_downloaded,
        })
    })();
    if result.is_err() {
        let _ = fs::remove_file(&path);
    }
    result
}

/// Retry any byte ranges whose parallel attempt failed by fetching them
/// sequentially without an artificial per-request timeout.
fn retry_missing_parallel_ranges_serially(
    request: &DownloadByteRangeRequest,
    part_ranges: &[DownloadByteRange],
    chunks: &mut [Option<DownloadedManagedChunk>],
    bytes_downloaded: &mut u64,
    progress: &mut impl FnMut(u64, Option<u64>) -> Result<()>,
) -> Result<usize> {
    let mut retried_parts = 0_usize;
    for (index, range) in part_ranges.iter().copied().enumerate() {
        if chunks[index].is_some() {
            continue;
        }
        retried_parts = retried_parts.saturating_add(1);
        let chunk = download_byte_range_to_temp_file(request, range, None, |delta| {
            *bytes_downloaded = (*bytes_downloaded).saturating_add(delta);
            progress(*bytes_downloaded, Some(request.total_bytes))
        })
        .with_context(|| {
            format!(
                "retrying {} range {}-{} serially",
                request.asset_label, range.start, range.end
            )
        })?;
        chunks[index] = Some(chunk);
    }
    Ok(retried_parts)
}

/// Concatenate downloaded chunks into the final asset file in part order,
/// hashing the combined bytes and ensuring each chunk on disk matches its
/// recorded length.
fn merge_downloaded_chunks_into_file(
    output_path: &Path,
    chunks: &[DownloadedManagedChunk],
) -> Result<String> {
    let mut output = File::create(output_path)
        .with_context(|| format!("creating merged download file {}", output_path.display()))?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; MANAGED_RELEASE_DOWNLOAD_BUFFER_BYTES];
    let mut merged_bytes = 0_u64;
    for chunk in chunks {
        let mut input = File::open(&chunk.path)
            .with_context(|| format!("opening downloaded chunk {}", chunk.path.display()))?;
        let mut chunk_bytes = 0_u64;
        loop {
            let read = input
                .read(&mut buffer)
                .with_context(|| format!("reading downloaded chunk {}", chunk.path.display()))?;
            if read == 0 {
                break;
            }
            output.write_all(&buffer[..read]).with_context(|| {
                format!("writing merged download file {}", output_path.display())
            })?;
            hasher.update(&buffer[..read]);
            chunk_bytes = chunk_bytes.saturating_add(read as u64);
            merged_bytes = merged_bytes.saturating_add(read as u64);
        }
        if chunk_bytes != chunk.bytes_downloaded {
            anyhow::bail!(
                "downloaded chunk {} contained {} bytes on disk instead of {}",
                chunk.path.display(),
                chunk_bytes,
                chunk.bytes_downloaded
            );
        }
    }
    output
        .flush()
        .with_context(|| format!("flushing merged download file {}", output_path.display()))?;
    if merged_bytes == 0 {
        anyhow::bail!("merged managed runtime download is empty");
    }
    Ok(hex::encode(hasher.finalize()))
}

/// Choose the byte ranges for a parallel download, returning `None` when the
/// asset is too small or only one part would be produced.
pub(crate) fn choose_parallel_download_ranges(total_bytes: u64) -> Option<Vec<DownloadByteRange>> {
    if total_bytes < MIN_PARALLEL_DOWNLOAD_BYTES {
        return None;
    }

    let part_count = total_bytes
        .div_ceil(TARGET_PARALLEL_DOWNLOAD_PART_BYTES)
        .clamp(2, MAX_PARALLEL_DOWNLOAD_PARTS as u64) as usize;
    if part_count < 2 {
        return None;
    }

    let base_len = total_bytes / part_count as u64;
    let remainder = total_bytes % part_count as u64;
    let mut start = 0_u64;
    let mut ranges = Vec::with_capacity(part_count);
    for index in 0..part_count {
        let part_len = base_len + u64::from((index as u64) < remainder);
        let end = start + part_len.saturating_sub(1);
        ranges.push(DownloadByteRange { start, end });
        start = end.saturating_add(1);
    }
    Some(ranges)
}

#[cfg(not(test))]
fn parallel_range_request_timeout(range: DownloadByteRange) -> Duration {
    Duration::from_secs(
        PARALLEL_RANGE_TIMEOUT_GRACE_SECS
            + range
                .len()
                .div_ceil(PARALLEL_RANGE_TIMEOUT_MIN_BYTES_PER_SEC)
                .max(1),
    )
}

#[cfg(test)]
fn parallel_range_request_timeout(range: DownloadByteRange) -> Duration {
    Duration::from_millis(80 + range.len().div_ceil(128) * 10)
}
