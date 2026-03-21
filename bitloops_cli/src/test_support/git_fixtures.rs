use std::path::{Path, PathBuf};

use crate::test_support::process_state::git_command;
use crate::utils::paths;

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
    paths::default_blob_store_path(repo_root)
}

#[allow(dead_code)]
pub(crate) fn ensure_test_store_backends(repo_root: &Path) {
    let sqlite_path = paths::default_relational_db_path(repo_root);
    let sqlite = crate::engine::db::SqliteConnectionPool::connect(sqlite_path)
        .expect("create relational sqlite file");
    sqlite
        .initialise_checkpoint_schema()
        .expect("initialise checkpoint schema");

    let duckdb_path = paths::default_events_db_path(repo_root);
    if let Some(parent) = duckdb_path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent).expect("create duckdb parent");
    }
    let _conn = duckdb::Connection::open(duckdb_path).expect("create events duckdb file");

    std::fs::create_dir_all(paths::default_blob_store_path(repo_root))
        .expect("create local blob store directory");
}
