//! Integration-style tests for the managed runtime download pipeline that
//! exercise the dispatcher, the parallel range workers and the serial
//! fallbacks against an in-process mock HTTP server.

use reqwest::blocking::Client;
use sha2::{Digest, Sha256};
use std::fs;
use std::io::{self, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::thread;
use std::time::Duration;
use tempfile::TempDir;

use super::config::MIN_PARALLEL_DOWNLOAD_BYTES;
use super::entry::download_release_asset_to_temp_file;
use super::parallel::choose_parallel_download_ranges;
use super::types::DownloadByteRange;
use crate::test_support::process_state::enter_process_state;

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
        match max_in_flight.compare_exchange(observed, current, Ordering::SeqCst, Ordering::SeqCst)
        {
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
    let server = MockDownloadServer::start(asset_bytes.clone(), true, Duration::from_millis(40));
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
    assert!(
        requests.iter().any(|request| {
            let request = request.to_ascii_lowercase();
            request.contains("range: bytes=") && !request.contains("range: bytes=0-0")
        }),
        "expected follow-up range requests after the probe, got: {requests:?}"
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
