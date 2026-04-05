#[path = "test_command_support.rs"]
mod test_command_support;

use bitloops::cli::versioncheck::DISABLE_VERSION_CHECK_ENV;
use bitloops::host::devql::watch::DISABLE_WATCHER_AUTOSTART_ENV;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::env;
use std::fs;
use std::io::{Cursor, Read};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::Duration;
use tempfile::TempDir;

fn bitloops_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_bitloops"))
}

fn run_git(repo: &Path, args: &[&str]) {
    let out = Command::new("git")
        .args(args)
        .current_dir(repo)
        .output()
        .expect("failed to run git");
    assert!(
        out.status.success(),
        "git {:?} failed\nstdout:\n{}\nstderr:\n{}",
        args,
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

fn init_repo(repo: &Path) {
    run_git(repo, &["init"]);
    run_git(repo, &["config", "user.name", "Test"]);
    run_git(repo, &["config", "user.email", "test@example.com"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);
    run_git(repo, &["config", "tag.gpgsign", "false"]);
    fs::write(repo.join("README.md"), "dashboard e2e\n").expect("write readme");
    run_git(repo, &["add", "README.md"]);
    run_git(repo, &["commit", "-m", "init"]);
    ensure_dashboard_store_files(repo);
}

fn ensure_dashboard_store_files(repo_root: &Path) {
    test_command_support::with_repo_app_env(repo_root, || {
        let cfg = bitloops::config::resolve_store_backend_config_for_repo(repo_root)
            .expect("resolve backend config");

        if !cfg.relational.has_postgres() {
            let sqlite_path = if let Some(path) = cfg.relational.sqlite_path.as_deref() {
                bitloops::config::resolve_sqlite_db_path_for_repo(repo_root, Some(path))
                    .expect("resolve configured sqlite path")
            } else {
                bitloops::utils::paths::default_relational_db_path(repo_root)
            };
            let sqlite = bitloops::storage::SqliteConnectionPool::connect(sqlite_path)
                .expect("create relational sqlite file");
            sqlite
                .initialise_checkpoint_schema()
                .expect("initialise checkpoint schema");
        }

        if !cfg.events.has_clickhouse() {
            let duckdb_path = if let Some(path) = cfg.events.duckdb_path.as_deref() {
                bitloops::config::resolve_duckdb_db_path_for_repo(repo_root, Some(path))
            } else {
                bitloops::utils::paths::default_events_db_path(repo_root)
            };
            if let Some(parent) = duckdb_path.parent()
                && !parent.as_os_str().is_empty()
            {
                fs::create_dir_all(parent).expect("create duckdb parent");
            }
            let _conn = duckdb::Connection::open(duckdb_path).expect("create events duckdb file");
        }
    });
}

fn pick_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind ephemeral port");
    let port = listener.local_addr().expect("local addr").port();
    drop(listener);
    port
}

fn localhost_bind_available(test_name: &str) -> bool {
    match TcpListener::bind("127.0.0.1:0") {
        Ok(listener) => {
            drop(listener);
            true
        }
        Err(err) if err.kind() == std::io::ErrorKind::PermissionDenied => {
            eprintln!(
                "skipping {test_name}: loopback sockets are unavailable in this environment ({err})"
            );
            false
        }
        Err(err) => panic!("bind localhost for {test_name}: {err}"),
    }
}

fn build_bundle_archive(version: &str) -> Vec<u8> {
    let mut tar_builder = tar::Builder::new(Vec::new());

    let index = b"<html><body>installed bundle</body></html>".to_vec();
    let version_json =
        format!(r#"{{"version":"{version}","source_url":"https://cdn.test/bundle.tar.zst"}}"#)
            .into_bytes();

    let mut index_header = tar::Header::new_gnu();
    index_header.set_size(index.len() as u64);
    index_header.set_mode(0o644);
    index_header.set_cksum();
    tar_builder
        .append_data(&mut index_header, "index.html", Cursor::new(index))
        .expect("append index");

    let mut version_header = tar::Header::new_gnu();
    version_header.set_size(version_json.len() as u64);
    version_header.set_mode(0o644);
    version_header.set_cksum();
    tar_builder
        .append_data(
            &mut version_header,
            "version.json",
            Cursor::new(version_json),
        )
        .expect("append version");

    let tar_bytes = tar_builder.into_inner().expect("finalize tar");
    zstd::stream::encode_all(Cursor::new(tar_bytes), 0).expect("compress archive")
}

fn checksum_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex::encode(hasher.finalize())
}

fn setup_local_bundle_cdn(archive_bytes: &[u8], checksum: &str, manifest_version: &str) -> TempDir {
    let temp = TempDir::new().expect("local cdn temp dir");
    let root = temp.path();

    fs::write(root.join("bundle.tar.zst"), archive_bytes).expect("write bundle archive");
    fs::write(
        root.join("bundle.tar.zst.sha256"),
        format!("{checksum}  bundle.tar.zst\n"),
    )
    .expect("write checksum");

    let manifest = format!(
        r#"{{"versions":[{{"version":"{version}","min_required_cli_version":"0.0.1","max_required_cli_version":"latest","download_url":"bundle.tar.zst","checksum_url":"bundle.tar.zst.sha256"}}]}}"#,
        version = manifest_version
    );
    fs::write(root.join("bundle_versions.json"), manifest).expect("write manifest");
    temp
}

struct ChildGuard {
    child: Child,
}

impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn read_child_stderr(child: &mut Child) -> String {
    let Some(mut stderr) = child.stderr.take() else {
        return "<stderr unavailable>".to_string();
    };

    let mut output = String::new();
    match stderr.read_to_string(&mut output) {
        Ok(_) => {
            if output.trim().is_empty() {
                "<no stderr output>".to_string()
            } else {
                output
            }
        }
        Err(err) => format!("<failed reading stderr: {err}>"),
    }
}

async fn wait_until_ready(url: &str, child: &mut Child) {
    let client = reqwest::Client::new();
    for _ in 0..300 {
        if let Ok(response) = client.get(url).send().await
            && response.status().is_success()
        {
            return;
        }

        match child.try_wait() {
            Ok(Some(status)) => {
                let stderr = read_child_stderr(child);
                panic!(
                    "daemon process exited before readiness check succeeded at {url}\nchild status: {status}\nchild stderr:\n{stderr}"
                );
            }
            Ok(None) => {}
            Err(err) => {
                panic!("failed to inspect daemon process status while waiting for {url}: {err}")
            }
        }

        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    let (child_status, child_stderr) = match child.try_wait() {
        Ok(Some(status)) => (status.to_string(), read_child_stderr(child)),
        Ok(None) => (
            "still running".to_string(),
            "<child still running; stderr cannot be drained without stopping it>".to_string(),
        ),
        Err(err) => (
            format!("<failed to inspect status: {err}>"),
            "<stderr unavailable>".to_string(),
        ),
    };
    panic!(
        "daemon server did not become ready at {url}\nchild status: {child_status}\nchild stderr:\n{child_stderr}"
    );
}

#[tokio::test]
async fn e2e_dashboard_bundle_lifecycle_missing_install_served() {
    if !localhost_bind_available("e2e_dashboard_bundle_lifecycle_missing_install_served") {
        return;
    }
    let repo = TempDir::new().expect("repo temp dir");
    init_repo(repo.path());

    let bundle_dir = repo.path().join("bundle");
    let archive = build_bundle_archive("4.0.0");
    let checksum = checksum_hex(&archive);
    let cdn = setup_local_bundle_cdn(&archive, &checksum, "4.0.0");

    let port = pick_port();
    let base_url = format!("file://{}/", cdn.path().display());

    let child = test_command_support::new_isolated_bitloops_command(
        &bitloops_bin(),
        repo.path(),
        &[
            "daemon",
            "start",
            "--create-default-config",
            "--no-telemetry",
            "--http",
            "--host",
            "127.0.0.1",
            "--port",
            &port.to_string(),
            "--bundle-dir",
            bundle_dir.to_str().expect("bundle dir str"),
        ],
    )
    .env("BITLOOPS_DASHBOARD_CDN_BASE_URL", &base_url)
    .env(DISABLE_WATCHER_AUTOSTART_ENV, "1")
    .env(DISABLE_VERSION_CHECK_ENV, "1")
    .env_remove("BITLOOPS_DEVQL_PG_DSN")
    .env_remove("BITLOOPS_DEVQL_CH_URL")
    .env_remove("BITLOOPS_DEVQL_CH_DATABASE")
    .env_remove("BITLOOPS_DEVQL_CH_USER")
    .env_remove("BITLOOPS_DEVQL_CH_PASSWORD")
    .stdout(Stdio::null())
    .stderr(Stdio::piped())
    .spawn()
    .expect("start daemon process");
    let mut guard = ChildGuard { child };

    wait_until_ready(&format!("http://127.0.0.1:{port}/api"), &mut guard.child).await;

    let client = reqwest::Client::new();

    let before = client
        .get(format!("http://127.0.0.1:{port}/"))
        .send()
        .await
        .expect("get before install")
        .text()
        .await
        .expect("read before install body");
    assert!(before.contains("Install dashboard bundle"));

    let fetch_response = client
        .post(format!("http://127.0.0.1:{port}/api/fetch_bundle"))
        .header("content-type", "application/json")
        .body("{}")
        .send()
        .await
        .expect("post fetch bundle");
    assert_eq!(fetch_response.status(), reqwest::StatusCode::OK);
    let fetch_payload = fetch_response
        .json::<Value>()
        .await
        .expect("parse fetch payload");
    assert_eq!(fetch_payload["status"], "installed");
    assert_eq!(fetch_payload["installedVersion"], "4.0.0");

    let after = client
        .get(format!("http://127.0.0.1:{port}/"))
        .send()
        .await
        .expect("get after install")
        .text()
        .await
        .expect("read after install body");
    assert!(after.contains("installed bundle"));

    let api_root = client
        .get(format!("http://127.0.0.1:{port}/api"))
        .send()
        .await
        .expect("get api root");
    assert_eq!(api_root.status(), reqwest::StatusCode::OK);
    let api_root_payload = api_root
        .json::<Value>()
        .await
        .expect("parse api root payload");
    assert_eq!(api_root_payload["name"], "bitloops-dashboard-api");

    assert!(bundle_dir.join("index.html").is_file());
    assert!(bundle_dir.join("version.json").is_file());
}
