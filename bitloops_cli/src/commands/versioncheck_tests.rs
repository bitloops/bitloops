use super::*;
use crate::test_support::process_state::with_env_vars;
use std::fs;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::Path;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tempfile::TempDir;

fn with_test_overrides<T>(
    config_dir: Option<&Path>,
    server_url: Option<&str>,
    f: impl FnOnce() -> T,
) -> T {
    let config_dir_str = config_dir.map(|path| path.to_string_lossy().into_owned());
    with_env_vars(
        &[
            (CONFIG_DIR_OVERRIDE_ENV, config_dir_str.as_deref()),
            (GITHUB_API_URL_OVERRIDE_ENV, server_url),
        ],
        f,
    )
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock should be after unix epoch")
        .as_secs()
}

struct MockServer {
    url: String,
    hits: Arc<AtomicUsize>,
    request: Arc<Mutex<Option<String>>>,
    handle: Option<thread::JoinHandle<()>>,
}

impl MockServer {
    fn start(status_code: u16, body: &str) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock server");
        listener.set_nonblocking(true).expect("set nonblocking");
        let addr = listener.local_addr().expect("get local addr");
        let url = format!("http://{}", addr);

        let hits = Arc::new(AtomicUsize::new(0));
        let request = Arc::new(Mutex::new(None));

        let hits_for_thread = Arc::clone(&hits);
        let request_for_thread = Arc::clone(&request);
        let response_body = body.to_string();

        let handle = thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_millis(250);

            loop {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        hits_for_thread.fetch_add(1, Ordering::SeqCst);

                        let mut buf = [0u8; 4096];
                        let n = stream.read(&mut buf).unwrap_or(0);
                        let req = String::from_utf8_lossy(&buf[..n]).to_string();
                        *request_for_thread
                            .lock()
                            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(req);

                        let status_text = match status_code {
                            200 => "OK",
                            500 => "Internal Server Error",
                            _ => "Status",
                        };
                        let response = format!(
                            "HTTP/1.1 {status_code} {status_text}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                            response_body.len(),
                            response_body
                        );
                        let _ = stream.write_all(response.as_bytes());
                        break;
                    }
                    Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                        if Instant::now() >= deadline {
                            break;
                        }
                        thread::sleep(Duration::from_millis(10));
                    }
                    Err(_) => break,
                }
            }
        });

        Self {
            url,
            hits,
            request,
            handle: Some(handle),
        }
    }

    fn hits(&self) -> usize {
        self.hits.load(Ordering::SeqCst)
    }

    fn request_text(&self) -> String {
        self.request
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
            .unwrap_or_default()
    }
}

impl Drop for MockServer {
    fn drop(&mut self) {
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

fn with_test_paths<T>(tmp_home: &TempDir, server_url: &str, f: impl FnOnce() -> T) -> T {
    let config_dir = tmp_home.path().join(GLOBAL_CONFIG_DIR_NAME);
    with_test_overrides(Some(&config_dir), Some(server_url), f)
}

#[test]
fn test_is_outdated() {
    struct TestCase {
        current: &'static str,
        latest: &'static str,
        want: bool,
        desc: &'static str,
    }

    let tests = [
        TestCase {
            current: "1.0.0",
            latest: "1.0.1",
            want: true,
            desc: "patch version bump",
        },
        TestCase {
            current: "1.0.0",
            latest: "1.1.0",
            want: true,
            desc: "minor version bump",
        },
        TestCase {
            current: "1.0.0",
            latest: "2.0.0",
            want: true,
            desc: "major version bump",
        },
        TestCase {
            current: "1.0.1",
            latest: "1.0.0",
            want: false,
            desc: "current is newer",
        },
        TestCase {
            current: "2.0.0",
            latest: "1.9.9",
            want: false,
            desc: "current major is higher",
        },
        TestCase {
            current: "1.0.0",
            latest: "1.0.0",
            want: false,
            desc: "same version",
        },
        TestCase {
            current: "v1.0.0",
            latest: "v1.0.1",
            want: true,
            desc: "with v prefix",
        },
        TestCase {
            current: "v1.0.0",
            latest: "1.0.1",
            want: true,
            desc: "mixed v prefix",
        },
        TestCase {
            current: "1.0.0",
            latest: "v1.0.1",
            want: true,
            desc: "mixed v prefix reversed",
        },
        TestCase {
            current: "1.0.0-rc1",
            latest: "1.0.0",
            want: true,
            desc: "prerelease in current",
        },
        TestCase {
            current: "1.0.0",
            latest: "1.0.1-rc1",
            want: true,
            desc: "prerelease in latest is still newer",
        },
    ];

    for tc in tests {
        let got = is_outdated(tc.current, tc.latest);
        assert_eq!(
            got, tc.want,
            "is_outdated({:?}, {:?}) mismatch for {}",
            tc.current, tc.latest, tc.desc
        );
    }
}

#[test]
fn test_cache_read_write() {
    let tmp_home = tempfile::tempdir().expect("create temp dir");
    let config_dir = tmp_home.path().join(GLOBAL_CONFIG_DIR_NAME);
    with_test_overrides(Some(&config_dir), None, || {
        fs::create_dir_all(&config_dir).expect("create config dir");

        let original_cache = VersionCache {
            last_check_time_secs: now_secs(),
        };

        save_cache(&original_cache).expect("save cache");
        let loaded = load_cache().expect("load cache");

        let file_path = cache_file_path().expect("resolve cache file path");

        let diff = loaded
            .last_check_time_secs
            .abs_diff(original_cache.last_check_time_secs);
        assert!(
            diff <= 1,
            "last_check_time_secs mismatch: got {}, want {}",
            loaded.last_check_time_secs,
            original_cache.last_check_time_secs
        );
        assert!(file_path.exists(), "cache file should exist");
    });
}

#[test]
fn test_ensure_global_config_dir() {
    let tmp_dir = tempfile::tempdir().expect("create temp dir");
    let config_dir = tmp_dir.path().join(GLOBAL_CONFIG_DIR_NAME);
    with_test_overrides(Some(&config_dir), None, || {
        assert!(!config_dir.exists(), "directory already exists before test");

        ensure_global_config_dir().expect("ensure_global_config_dir should not error");

        let metadata = fs::metadata(&config_dir).expect("config directory should be created");
        assert!(metadata.is_dir(), "path should be a directory");

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = metadata.permissions().mode();
            assert!(
                mode & 0o700 == 0o700,
                "directory permissions {mode:o} should include owner rwx"
            );
        }
    });
}

#[test]
fn test_fetch_latest_version() {
    let server = MockServer::start(200, r#"{"tag_name":"v1.2.3","prerelease":false}"#);
    with_test_overrides(None, Some(server.url.as_str()), || {
        let got = fetch_latest_version().expect("fetch_latest_version should succeed");

        assert!(server.hits() > 0, "expected HTTP request to be made");
        let req = server.request_text();
        assert!(
            req.contains("Accept: application/vnd.github+json"),
            "missing Accept header in request: {req}"
        );
        assert!(
            req.contains("User-Agent: bitloops-cli"),
            "missing User-Agent header in request: {req}"
        );
        assert_eq!(got, "v1.2.3");
    });
}

#[test]
fn test_fetch_latest_version_prerelease() {
    let server = MockServer::start(200, r#"{"tag_name":"v2.0.0-rc1","prerelease":true}"#);
    with_test_overrides(None, Some(server.url.as_str()), || {
        let result = fetch_latest_version();
        assert!(
            result.is_err(),
            "expected error for prerelease response, got {result:?}"
        );
    });
}

#[test]
fn test_fetch_latest_version_server_error() {
    let server = MockServer::start(500, "");
    with_test_overrides(None, Some(server.url.as_str()), || {
        let result = fetch_latest_version();
        assert!(
            result.is_err(),
            "expected error for 500 response, got {result:?}"
        );
    });
}

#[test]
fn test_parse_github_release() {
    struct TestCase {
        name: &'static str,
        body: &'static str,
        want: &'static str,
        want_err: bool,
    }

    let tests = [
        TestCase {
            name: "valid release",
            body: r#"{"tag_name":"v1.2.3","prerelease":false}"#,
            want: "v1.2.3",
            want_err: false,
        },
        TestCase {
            name: "bitloops latest json",
            body: r#"{"version":"1.2.3"}"#,
            want: "1.2.3",
            want_err: false,
        },
        TestCase {
            name: "prerelease",
            body: r#"{"tag_name":"v2.0.0-rc1","prerelease":true}"#,
            want: "",
            want_err: true,
        },
        TestCase {
            name: "empty tag",
            body: r#"{"tag_name":"","prerelease":false}"#,
            want: "",
            want_err: true,
        },
        TestCase {
            name: "invalid json",
            body: "not json",
            want: "",
            want_err: true,
        },
    ];

    for tc in tests {
        let got = parse_github_release(tc.body.as_bytes());
        assert_eq!(
            got.is_err(),
            tc.want_err,
            "parse_github_release error state mismatch for {}",
            tc.name
        );
        if !tc.want_err {
            assert_eq!(
                got.expect("expected successful parse"),
                tc.want,
                "parse_github_release value mismatch for {}",
                tc.name
            );
        }
    }
}

#[test]
fn test_update_command() {
    let cmd = update_command();
    let valid = [
        "brew upgrade bitloops",
        "curl -fsSL https://bitloops.io/install.sh | bash",
    ];
    assert!(
        valid.contains(&cmd.as_str()),
        "update_command returned unexpected value: {cmd}"
    );
}

#[test]
fn test_check_and_notify_skips_dev_version() {
    let server = MockServer::start(200, r#"{"tag_name":"v9.9.9","prerelease":false}"#);
    let tmp_home = tempfile::tempdir().expect("create temp dir");
    with_test_paths(&tmp_home, &server.url, || {
        let mut buf = Vec::new();
        check_and_notify(&mut buf, "dev");
        assert!(
            buf.is_empty(),
            "expected no output for dev version, got: {}",
            String::from_utf8_lossy(&buf)
        );
    });
}

#[test]
fn test_check_and_notify_skips_empty_version() {
    let server = MockServer::start(200, r#"{"tag_name":"v9.9.9","prerelease":false}"#);
    let tmp_home = tempfile::tempdir().expect("create temp dir");
    with_test_paths(&tmp_home, &server.url, || {
        let mut buf = Vec::new();
        check_and_notify(&mut buf, "");
        assert!(
            buf.is_empty(),
            "expected no output for empty version, got: {}",
            String::from_utf8_lossy(&buf)
        );
    });
}

#[test]
fn test_check_and_notify_skips_when_cache_is_fresh() {
    let server = MockServer::start(200, r#"{"tag_name":"v9.9.9","prerelease":false}"#);
    let tmp_home = tempfile::tempdir().expect("create temp dir");
    with_test_paths(&tmp_home, &server.url, || {
        let config_dir = global_config_dir_path().expect("resolve config path");
        fs::create_dir_all(&config_dir).expect("create config dir");
        let cache = VersionCache {
            last_check_time_secs: now_secs(),
        };
        save_cache(&cache).expect("save cache");

        let mut buf = Vec::new();
        check_and_notify(&mut buf, "1.0.0");
        assert!(
            buf.is_empty(),
            "expected no output when cache is fresh, got: {}",
            String::from_utf8_lossy(&buf)
        );
    });
}

#[test]
fn test_check_and_notify_prints_notification_when_outdated() {
    let server = MockServer::start(200, r#"{"tag_name":"v2.0.0","prerelease":false}"#);
    let tmp_home = tempfile::tempdir().expect("create temp dir");
    with_test_paths(&tmp_home, &server.url, || {
        let mut buf = Vec::new();
        check_and_notify(&mut buf, "1.0.0");

        let output = String::from_utf8_lossy(&buf).to_string();
        assert!(
            output.contains("v2.0.0"),
            "expected output to include latest version, got: {output}"
        );
        assert!(
            output.contains("1.0.0"),
            "expected output to include current version, got: {output}"
        );
    });
}

#[test]
fn test_check_and_notify_no_notification_when_up_to_date() {
    let server = MockServer::start(200, r#"{"tag_name":"v1.0.0","prerelease":false}"#);
    let tmp_home = tempfile::tempdir().expect("create temp dir");
    with_test_paths(&tmp_home, &server.url, || {
        let mut buf = Vec::new();
        check_and_notify(&mut buf, "1.0.0");

        assert!(
            buf.is_empty(),
            "expected no output when up to date, got: {}",
            String::from_utf8_lossy(&buf)
        );
    });
}

#[test]
fn test_check_and_notify_fetch_failure_updates_cache_to_prevent_retry() {
    let server = MockServer::start(500, "");
    let tmp_home = tempfile::tempdir().expect("create temp dir");
    with_test_paths(&tmp_home, &server.url, || {
        let mut buf = Vec::new();
        check_and_notify(&mut buf, "1.0.0");

        assert!(
            buf.is_empty(),
            "expected no output on fetch failure, got: {}",
            String::from_utf8_lossy(&buf)
        );

        let cache = load_cache().expect("cache should be readable after fetch failure");
        let age_secs = now_secs().saturating_sub(cache.last_check_time_secs);
        assert!(
            age_secs <= 60,
            "expected cache timestamp to be updated recently, age_secs={age_secs}"
        );
    });
}
