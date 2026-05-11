use std::fs;
use std::path::{Path, PathBuf};

use crate::config::resolve_repo_runtime_db_path_for_repo;
use crate::config::{BITLOOPS_CONFIG_RELATIVE_PATH, resolve_store_backend_config_for_repo};
use crate::test_support::process_state::git_command;

pub(crate) fn git_ok(repo_root: &Path, args: &[&str]) -> String {
    let out = git_command()
        .args(args)
        .current_dir(repo_root)
        .output()
        .unwrap_or_else(|err| panic!("failed to start git {:?}: {err}", args));
    assert!(
        out.status.success(),
        "git {:?} failed\nstdout:\n{}\nstderr:\n{}",
        args,
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

pub(crate) fn init_test_repo(repo_root: &Path, branch: &str, user_name: &str, user_email: &str) {
    git_ok(repo_root, &["init"]);
    git_ok(repo_root, &["checkout", "-B", branch]);
    git_ok(repo_root, &["config", "user.name", user_name]);
    git_ok(repo_root, &["config", "user.email", user_email]);
    git_ok(repo_root, &["config", "commit.gpgsign", "false"]);
}

pub(crate) fn repo_local_blob_root(repo_root: &Path) -> PathBuf {
    resolve_store_backend_config_for_repo(repo_root)
        .expect("resolve test store backends")
        .blobs
        .resolve_local_path_for_repo(repo_root)
        .expect("resolve test blob path")
}

pub(crate) fn write_test_daemon_config(config_root: &Path) -> PathBuf {
    let config_path = config_root.join(BITLOOPS_CONFIG_RELATIVE_PATH);
    ignore_repo_test_state_dir(config_root);
    let daemon_state_root = test_state_root_for_config_root(config_root);
    let sqlite_path = daemon_state_root
        .join("stores")
        .join("relational")
        .join("relational.db");
    let duckdb_path = daemon_state_root
        .join("stores")
        .join("event")
        .join("events.duckdb");
    let blob_path = daemon_state_root.join("stores").join("blob");
    let config_contents = format!(
        r#"[runtime]
local_dev = false

[stores.relational]
sqlite_path = {sqlite_path:?}

[stores.events]
duckdb_path = {duckdb_path:?}

[stores.blob]
local_path = {blob_path:?}
"#,
    );
    fs::write(&config_path, config_contents).expect("write test daemon config");
    crate::config::settings::write_repo_daemon_binding(
        &config_root.join(crate::config::REPO_POLICY_LOCAL_FILE_NAME),
        &config_path,
    )
    .expect("write test repo daemon binding");
    config_path
}

#[allow(dead_code)]
pub(crate) fn ensure_test_store_backends(repo_root: &Path) {
    write_test_daemon_config(repo_root);

    let backends = resolve_store_backend_config_for_repo(repo_root).expect("resolve test stores");

    let sqlite_path = backends
        .relational
        .resolve_sqlite_db_path_for_repo(repo_root)
        .expect("resolve relational sqlite path");
    let sqlite = crate::storage::SqliteConnectionPool::connect(sqlite_path)
        .expect("create relational sqlite file");
    sqlite
        .initialise_checkpoint_schema()
        .expect("initialise checkpoint schema");

    let duckdb_path = backends.events.resolve_duckdb_db_path_for_repo(repo_root);
    if let Some(parent) = duckdb_path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent).expect("create duckdb parent");
    }
    let _conn = duckdb::Connection::open(duckdb_path).expect("create events duckdb file");

    fs::create_dir_all(
        backends
            .blobs
            .resolve_local_path_for_repo(repo_root)
            .expect("resolve blob store path"),
    )
    .expect("create local blob store directory");

    let runtime_path = resolve_repo_runtime_db_path_for_repo(repo_root)
        .expect("resolve runtime sqlite path for test store backends");
    if let Some(parent) = runtime_path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent).expect("create runtime sqlite parent");
    }
    let runtime = crate::storage::SqliteConnectionPool::connect(runtime_path)
        .expect("create runtime sqlite file");
    runtime
        .initialise_runtime_checkpoint_schema()
        .expect("initialise runtime checkpoint schema");
}

pub(crate) fn test_state_root_for_config_root(config_root: &Path) -> PathBuf {
    config_root.join(".bitloops-test-state")
}

fn ignore_repo_test_state_dir(config_root: &Path) {
    let exclude_path = config_root.join(".git").join("info").join("exclude");
    if !exclude_path.exists() {
        return;
    }

    let mut content = fs::read_to_string(&exclude_path).unwrap_or_default();
    if content
        .lines()
        .any(|line| line.trim() == ".bitloops-test-state/")
    {
        return;
    }
    if !content.is_empty() && !content.ends_with('\n') {
        content.push('\n');
    }
    content.push_str(".bitloops-test-state/\n");
    fs::write(&exclude_path, content).expect("write git exclude for test state dir");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_test_daemon_config_keeps_state_under_config_root() {
        let temp = tempfile::tempdir().expect("temp dir");

        write_test_daemon_config(temp.path());

        let backends =
            resolve_store_backend_config_for_repo(temp.path()).expect("resolve test stores");
        let sqlite_path = backends
            .relational
            .resolve_sqlite_db_path_for_repo(temp.path())
            .expect("resolve relational sqlite path");
        let expected_root = temp.path().join(".bitloops-test-state");
        let old_sibling_root = temp
            .path()
            .parent()
            .expect("temp dir should have parent")
            .join(".bitloops-test-state");

        assert!(
            sqlite_path.starts_with(&expected_root),
            "sqlite path {} should be under {}",
            sqlite_path.display(),
            expected_root.display()
        );
        assert!(
            !sqlite_path.starts_with(&old_sibling_root),
            "sqlite path {} should not use sibling state root {}",
            sqlite_path.display(),
            old_sibling_root.display()
        );
    }
}
