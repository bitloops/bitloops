use anyhow::{Context, Result};
use reqwest::StatusCode;
use reqwest::blocking::Client;
use reqwest::header::{ACCEPT, CONTENT_RANGE, RANGE, USER_AGENT};
use sha2::{Digest, Sha256};
use std::env;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const MANAGED_RELEASE_DOWNLOAD_BUFFER_BYTES: usize = 1024 * 1024;
#[cfg(not(test))]
const MIN_PARALLEL_DOWNLOAD_BYTES: u64 = 8 * 1024 * 1024;
#[cfg(test)]
const MIN_PARALLEL_DOWNLOAD_BYTES: u64 = 1024;
#[cfg(not(test))]
const TARGET_PARALLEL_DOWNLOAD_PART_BYTES: u64 = 16 * 1024 * 1024;
#[cfg(test)]
const TARGET_PARALLEL_DOWNLOAD_PART_BYTES: u64 = 1024;
#[cfg(not(test))]
const MAX_PARALLEL_DOWNLOAD_PARTS: usize = 6;
#[cfg(test)]
const MAX_PARALLEL_DOWNLOAD_PARTS: usize = 4;
#[cfg(not(test))]
const PARALLEL_RANGE_TIMEOUT_GRACE_SECS: u64 = 15;
#[cfg(not(test))]
const PARALLEL_RANGE_TIMEOUT_MIN_BYTES_PER_SEC: u64 = 1024 * 1024;

#[derive(Debug)]
pub(crate) struct DownloadedManagedAsset {
    path: PathBuf,
    pub(crate) bytes_downloaded: u64,
    pub(crate) bytes_total: Option<u64>,
    pub(crate) sha256_hex: String,
}

impl DownloadedManagedAsset {
    pub(crate) fn path(&self) -> &Path {
        &self.path
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct DownloadByteRange {
    start: u64,
    end: u64,
}

impl DownloadByteRange {
    fn len(self) -> u64 {
        self.end.saturating_sub(self.start).saturating_add(1)
    }
}

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ParsedContentRange {
    start: u64,
    end: u64,
    total: u64,
}

enum WorkerEvent {
    Progress(usize, u64),
    Done(usize, Result<DownloadedManagedChunk, String>),
}

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

impl Drop for DownloadedManagedAsset {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

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

fn download_release_asset_to_temp_file_in_parallel(
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

fn download_release_asset_to_temp_file_serial(
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

fn download_response_to_temp_file(
    mut response: reqwest::blocking::Response,
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

fn managed_download_request<'a>(
    client: &'a Client,
    url: &'a str,
    user_agent: &'a str,
) -> reqwest::blocking::RequestBuilder {
    client
        .get(url)
        .header(ACCEPT, "application/octet-stream")
        .header(USER_AGENT, user_agent)
}

fn choose_parallel_download_ranges(total_bytes: u64) -> Option<Vec<DownloadByteRange>> {
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

fn validate_content_range_header(
    header: Option<&reqwest::header::HeaderValue>,
    expected_range: DownloadByteRange,
    expected_total: u64,
) -> Result<()> {
    let parsed = header
        .and_then(parse_content_range_header)
        .context("managed runtime range response is missing a valid Content-Range header")?;
    if parsed.start != expected_range.start
        || parsed.end != expected_range.end
        || parsed.total != expected_total
    {
        anyhow::bail!(
            "managed runtime range response returned bytes {}-{}/{} instead of {}-{}/{}",
            parsed.start,
            parsed.end,
            parsed.total,
            expected_range.start,
            expected_range.end,
            expected_total
        );
    }
    Ok(())
}

fn parse_content_range_header(header: &reqwest::header::HeaderValue) -> Option<ParsedContentRange> {
    let value = header.to_str().ok()?.trim();
    let range = value.strip_prefix("bytes ")?;
    let (span, total) = range.split_once('/')?;
    if total == "*" {
        return None;
    }
    let (start, end) = span.split_once('-')?;
    Some(ParsedContentRange {
        start: start.parse().ok()?,
        end: end.parse().ok()?,
        total: total.parse().ok()?,
    })
}

fn temporary_download_path(asset_label: &str) -> PathBuf {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    let safe_label: String = asset_label
        .chars()
        .map(|char| {
            if char.is_ascii_alphanumeric() {
                char.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    env::temp_dir().join(format!(
        "bitloops-{safe_label}.{}.{}.download",
        std::process::id(),
        suffix
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::process_state::enter_process_state;
    use std::io;
    use std::net::{TcpListener, TcpStream};
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;
    use tempfile::TempDir;

    #[derive(Clone, Copy)]
    enum MockResponseBodyMode {
        Immediate,
        Chunked {
            chunk_size: usize,
            chunk_delay: Duration,
        },
    }

    struct MockDownloadServer {
        url: String,
        requests: Arc<Mutex<Vec<String>>>,
        max_in_flight: Arc<AtomicUsize>,
        stop: Arc<AtomicBool>,
        accept_handle: Option<thread::JoinHandle<()>>,
        worker_handles: Arc<Mutex<Vec<thread::JoinHandle<()>>>>,
    }

    struct MockDownloadServerContext {
        asset_bytes: Arc<[u8]>,
        supports_range: bool,
        response_delay: Duration,
        range_body_modes: Arc<Vec<(DownloadByteRange, MockResponseBodyMode)>>,
        requests: Arc<Mutex<Vec<String>>>,
        max_in_flight: Arc<AtomicUsize>,
        active_requests: Arc<AtomicUsize>,
    }

    impl MockDownloadServer {
        fn start(asset_bytes: Vec<u8>, supports_range: bool, response_delay: Duration) -> Self {
            Self::start_with_body_modes(asset_bytes, supports_range, response_delay, Vec::new())
        }

        fn start_with_body_modes(
            asset_bytes: Vec<u8>,
            supports_range: bool,
            response_delay: Duration,
            range_body_modes: Vec<(DownloadByteRange, MockResponseBodyMode)>,
        ) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock download server");
            listener
                .set_nonblocking(true)
                .expect("set mock server nonblocking");
            let url = format!(
                "http://{}",
                listener.local_addr().expect("mock server addr")
            );
            let requests = Arc::new(Mutex::new(Vec::new()));
            let max_in_flight = Arc::new(AtomicUsize::new(0));
            let stop = Arc::new(AtomicBool::new(false));
            let worker_handles = Arc::new(Mutex::new(Vec::new()));
            let context = Arc::new(MockDownloadServerContext {
                asset_bytes: Arc::from(asset_bytes),
                supports_range,
                response_delay,
                range_body_modes: Arc::new(range_body_modes),
                requests: Arc::clone(&requests),
                max_in_flight: Arc::clone(&max_in_flight),
                active_requests: Arc::new(AtomicUsize::new(0)),
            });

            let context_for_thread = Arc::clone(&context);
            let stop_for_thread = Arc::clone(&stop);
            let worker_handles_for_thread = Arc::clone(&worker_handles);
            let accept_handle = thread::spawn(move || {
                while !stop_for_thread.load(Ordering::SeqCst) {
                    match listener.accept() {
                        Ok((stream, _)) => {
                            let context = Arc::clone(&context_for_thread);
                            let handle = thread::spawn(move || {
                                handle_connection(stream, context);
                            });
                            worker_handles_for_thread
                                .lock()
                                .expect("lock worker handles")
                                .push(handle);
                        }
                        Err(err) if err.kind() == io::ErrorKind::WouldBlock => {
                            thread::sleep(Duration::from_millis(10));
                        }
                        Err(_) => break,
                    }
                }
            });

            Self {
                url,
                requests,
                max_in_flight,
                stop,
                accept_handle: Some(accept_handle),
                worker_handles,
            }
        }
    }

    impl Drop for MockDownloadServer {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::SeqCst);
            let _ = TcpStream::connect(self.url.trim_start_matches("http://"));
            if let Some(handle) = self.accept_handle.take() {
                let _ = handle.join();
            }
            let mut worker_handles = self.worker_handles.lock().expect("lock worker handles");
            for handle in worker_handles.drain(..) {
                let _ = handle.join();
            }
        }
    }

    fn handle_connection(mut stream: TcpStream, context: Arc<MockDownloadServerContext>) {
        let current = context.active_requests.fetch_add(1, Ordering::SeqCst) + 1;
        update_max_active(&context.max_in_flight, current);
        let mut request = Vec::new();
        let mut buffer = [0_u8; 4096];
        loop {
            match stream.read(&mut buffer) {
                Ok(0) => break,
                Ok(read) => {
                    request.extend_from_slice(&buffer[..read]);
                    if request.windows(4).any(|window| window == b"\r\n\r\n") {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
        let request_text = String::from_utf8_lossy(&request).to_string();
        context
            .requests
            .lock()
            .expect("lock requests")
            .push(request_text.clone());
        thread::sleep(context.response_delay);

        let range_header = request_text
            .lines()
            .find_map(|line| {
                line.strip_prefix("Range: ")
                    .or_else(|| line.strip_prefix("range: "))
            })
            .map(str::trim)
            .and_then(parse_range_header);
        let (status_line, body, headers) = if context.supports_range {
            if let Some(range) = range_header {
                let start = range.start as usize;
                let end = range.end as usize;
                let body = context.asset_bytes[start..=end].to_vec();
                (
                    "HTTP/1.1 206 Partial Content",
                    body,
                    vec![
                        format!("Content-Length: {}", range.len()),
                        format!(
                            "Content-Range: bytes {}-{}/{}",
                            range.start,
                            range.end,
                            context.asset_bytes.len()
                        ),
                        "Accept-Ranges: bytes".to_string(),
                    ],
                )
            } else {
                (
                    "HTTP/1.1 200 OK",
                    context.asset_bytes.to_vec(),
                    vec![
                        format!("Content-Length: {}", context.asset_bytes.len()),
                        "Accept-Ranges: bytes".to_string(),
                    ],
                )
            }
        } else {
            (
                "HTTP/1.1 200 OK",
                context.asset_bytes.to_vec(),
                vec![format!("Content-Length: {}", context.asset_bytes.len())],
            )
        };
        let body_mode = range_header
            .and_then(|range| {
                context
                    .range_body_modes
                    .iter()
                    .find(|(candidate, _)| *candidate == range)
                    .map(|(_, mode)| *mode)
            })
            .unwrap_or(MockResponseBodyMode::Immediate);

        let response = format!(
            "{status_line}\r\n{}\r\nConnection: close\r\n\r\n",
            headers.join("\r\n")
        );
        let _ = stream.write_all(response.as_bytes());
        match body_mode {
            MockResponseBodyMode::Immediate => {
                let _ = stream.write_all(&body);
            }
            MockResponseBodyMode::Chunked {
                chunk_size,
                chunk_delay,
            } => {
                let chunk_size = chunk_size.max(1);
                for chunk in body.chunks(chunk_size) {
                    if stream.write_all(chunk).is_err() {
                        break;
                    }
                    let _ = stream.flush();
                    thread::sleep(chunk_delay);
                }
            }
        }
        context.active_requests.fetch_sub(1, Ordering::SeqCst);
    }

    fn update_max_active(max_in_flight: &AtomicUsize, current: usize) {
        let mut observed = max_in_flight.load(Ordering::SeqCst);
        while current > observed {
            match max_in_flight.compare_exchange(
                observed,
                current,
                Ordering::SeqCst,
                Ordering::SeqCst,
            ) {
                Ok(_) => return,
                Err(next) => observed = next,
            }
        }
    }

    fn parse_range_header(header: &str) -> Option<DownloadByteRange> {
        let value = header.strip_prefix("bytes=")?;
        let (start, end) = value.split_once('-')?;
        Some(DownloadByteRange {
            start: start.parse().ok()?,
            end: end.parse().ok()?,
        })
    }

    #[test]
    fn download_uses_parallel_ranges_when_server_supports_them() {
        let repo = TempDir::new().expect("tempdir");
        let _guard = enter_process_state(Some(repo.path()), &[]);
        let asset_bytes = (0..4096)
            .map(|index| (index % 251) as u8)
            .collect::<Vec<_>>();
        let server =
            MockDownloadServer::start(asset_bytes.clone(), true, Duration::from_millis(40));
        let client = Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .expect("build client");

        let download = download_release_asset_to_temp_file(
            &client,
            &server.url,
            "bitloops-test",
            "test asset",
            |_downloaded, _total| Ok(()),
        )
        .expect("download asset");

        assert_eq!(
            fs::read(download.path()).expect("read downloaded asset"),
            asset_bytes
        );
        assert_eq!(download.bytes_downloaded, asset_bytes.len() as u64);
        assert_eq!(download.bytes_total, Some(asset_bytes.len() as u64));
        assert_eq!(
            download.sha256_hex,
            hex::encode(Sha256::digest(&asset_bytes))
        );
        assert!(
            server.max_in_flight.load(Ordering::SeqCst) > 1,
            "expected concurrent range downloads, got max in-flight {}",
            server.max_in_flight.load(Ordering::SeqCst)
        );
        let requests = server.requests.lock().expect("lock requests");
        assert!(
            requests
                .iter()
                .any(|request| request.to_ascii_lowercase().contains("range: bytes=0-0")),
            "expected an initial range probe, got: {requests:?}"
        );
        assert!(
            requests
                .iter()
                .filter(|request| request.to_ascii_lowercase().contains("range: bytes="))
                .count()
                >= 3,
            "expected multiple range requests, got: {requests:?}"
        );
    }

    #[test]
    fn download_falls_back_to_serial_when_server_ignores_ranges() {
        let repo = TempDir::new().expect("tempdir");
        let _guard = enter_process_state(Some(repo.path()), &[]);
        let asset_bytes = b"serial-download-body".to_vec();
        let server = MockDownloadServer::start(asset_bytes.clone(), false, Duration::ZERO);
        let client = Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .expect("build client");

        let download = download_release_asset_to_temp_file(
            &client,
            &server.url,
            "bitloops-test",
            "test asset",
            |_downloaded, _total| Ok(()),
        )
        .expect("download asset");

        assert_eq!(
            fs::read(download.path()).expect("read downloaded asset"),
            asset_bytes
        );
        let requests = server.requests.lock().expect("lock requests");
        assert_eq!(
            requests.len(),
            1,
            "expected probe response reuse, got {requests:?}"
        );
        assert!(
            requests[0]
                .to_ascii_lowercase()
                .contains("range: bytes=0-0"),
            "expected the first request to be the range probe, got: {}",
            requests[0]
        );
    }

    #[test]
    fn download_refetches_serially_when_asset_is_too_small_for_parallel_ranges() {
        let repo = TempDir::new().expect("tempdir");
        let _guard = enter_process_state(Some(repo.path()), &[]);
        let asset_bytes = b"small-parallel-probe".to_vec();
        assert!(asset_bytes.len() < MIN_PARALLEL_DOWNLOAD_BYTES as usize);
        let server = MockDownloadServer::start(asset_bytes.clone(), true, Duration::ZERO);
        let client = Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .expect("build client");

        let download = download_release_asset_to_temp_file(
            &client,
            &server.url,
            "bitloops-test",
            "test asset",
            |_downloaded, _total| Ok(()),
        )
        .expect("download asset");

        assert_eq!(
            fs::read(download.path()).expect("read downloaded asset"),
            asset_bytes
        );
        let requests = server.requests.lock().expect("lock requests");
        assert_eq!(
            requests.len(),
            2,
            "expected probe plus serial refetch, got {requests:?}"
        );
        assert!(
            requests[0]
                .to_ascii_lowercase()
                .contains("range: bytes=0-0"),
            "expected the first request to be the range probe, got: {}",
            requests[0]
        );
        assert!(
            !requests[1].to_ascii_lowercase().contains("range: bytes="),
            "expected the second request to be a full serial fetch, got: {}",
            requests[1]
        );
    }

    #[test]
    fn choose_parallel_download_ranges_covers_full_asset() {
        let ranges = choose_parallel_download_ranges(4096).expect("parallel ranges");
        assert_eq!(ranges.first().expect("first range").start, 0);
        assert_eq!(ranges.last().expect("last range").end, 4095);
        assert_eq!(ranges.iter().map(|range| range.len()).sum::<u64>(), 4096);
        assert!(ranges.len() >= 2);
        for pair in ranges.windows(2) {
            assert_eq!(pair[0].end + 1, pair[1].start);
        }
    }

    #[test]
    fn download_retries_straggling_parallel_range_serially() {
        let repo = TempDir::new().expect("tempdir");
        let _guard = enter_process_state(Some(repo.path()), &[]);
        let asset_bytes = (0..4096)
            .map(|index| (index % 251) as u8)
            .collect::<Vec<_>>();
        let slow_range =
            choose_parallel_download_ranges(asset_bytes.len() as u64).expect("parallel ranges")[1];
        let server = MockDownloadServer::start_with_body_modes(
            asset_bytes.clone(),
            true,
            Duration::ZERO,
            vec![(
                slow_range,
                MockResponseBodyMode::Chunked {
                    chunk_size: 64,
                    chunk_delay: Duration::from_millis(20),
                },
            )],
        );
        let client = Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .expect("build client");

        let download = download_release_asset_to_temp_file(
            &client,
            &server.url,
            "bitloops-test",
            "test asset",
            |_downloaded, _total| Ok(()),
        )
        .expect("download asset");

        assert_eq!(
            fs::read(download.path()).expect("read downloaded asset"),
            asset_bytes
        );

        let slow_range_header = format!("range: bytes={}-{}", slow_range.start, slow_range.end);
        let requests = server.requests.lock().expect("lock requests");
        assert!(
            requests
                .iter()
                .filter(|request| {
                    request
                        .to_ascii_lowercase()
                        .contains(slow_range_header.as_str())
                })
                .count()
                >= 2,
            "expected the slow range to be retried serially, got: {requests:?}"
        );
    }
}
