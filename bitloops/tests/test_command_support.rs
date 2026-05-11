use std::ffi::OsString;
use std::fs;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Mutex, MutexGuard, OnceLock};

use bitloops::cli::versioncheck::DISABLE_VERSION_CHECK_ENV;
use bitloops::host::devql::watch::DISABLE_WATCHER_AUTOSTART_ENV;

const TEST_STATE_DIR_OVERRIDE_ENV: &str = "BITLOOPS_TEST_STATE_DIR_OVERRIDE";

pub fn new_isolated_bitloops_command(bin_path: &Path, repo: &Path, args: &[&str]) -> Command {
    let mut cmd = Command::new(bin_path);
    cmd.args(args)
        .current_dir(repo)
        .env_remove("BITLOOPS_DEVQL_PG_DSN")
        .env_remove("BITLOOPS_DEVQL_CH_URL")
        .env_remove("BITLOOPS_DEVQL_CH_DATABASE")
        .env_remove("BITLOOPS_DEVQL_CH_USER")
        .env_remove("BITLOOPS_DEVQL_CH_PASSWORD");
    apply_repo_app_env(&mut cmd, repo);

    cmd
}

pub fn apply_repo_app_env(cmd: &mut Command, repo: &Path) {
    let paths = repo_app_paths(repo);
    apply_repo_app_paths(cmd, &paths);
}

#[allow(dead_code)]
pub fn write_test_daemon_config(repo: &Path) {
    let config_path = repo.join(bitloops::config::BITLOOPS_CONFIG_RELATIVE_PATH);
    let daemon_state_root = repo_test_state_root(repo);
    let sqlite_path = daemon_state_root
        .join("stores")
        .join("relational")
        .join("relational.db");
    let duckdb_path = daemon_state_root
        .join("stores")
        .join("event")
        .join("events.duckdb");
    let blob_path = daemon_state_root.join("stores").join("blob");
    fs::write(
        &config_path,
        format!(
            r#"[runtime]
local_dev = false

[stores.relational]
sqlite_path = {sqlite_path:?}

[stores.events]
duckdb_path = {duckdb_path:?}

[stores.blob]
local_path = {blob_path:?}
"#,
        ),
    )
    .expect("write repo daemon config");
    bitloops::config::settings::write_repo_daemon_binding(
        &repo.join(bitloops::config::REPO_POLICY_LOCAL_FILE_NAME),
        &config_path,
    )
    .expect("write repo daemon binding");
}

#[allow(dead_code)]
pub fn ensure_repo_daemon_stores(repo: &Path) {
    write_test_daemon_config(repo);

    let cfg = bitloops::config::resolve_store_backend_config_for_repo(repo)
        .expect("resolve backend config for integration test repo");

    if !cfg.relational.has_postgres() {
        let sqlite_path = if let Some(path) = cfg.relational.sqlite_path.as_deref() {
            bitloops::config::resolve_sqlite_db_path_for_repo(repo, Some(path))
                .expect("resolve configured sqlite path")
        } else {
            bitloops::utils::paths::default_relational_db_path(repo)
        };
        let sqlite = bitloops::storage::SqliteConnectionPool::connect(sqlite_path)
            .expect("create relational sqlite file");
        sqlite
            .initialise_checkpoint_schema()
            .expect("initialise checkpoint schema");
    }

    if !cfg.events.has_clickhouse() {
        let duckdb_path = if let Some(path) = cfg.events.duckdb_path.as_deref() {
            bitloops::config::resolve_duckdb_db_path_for_repo(repo, Some(path))
        } else {
            bitloops::utils::paths::default_events_db_path(repo)
        };
        if let Some(parent) = duckdb_path.parent()
            && !parent.as_os_str().is_empty()
        {
            fs::create_dir_all(parent).expect("create duckdb parent");
        }
        let _conn = duckdb::Connection::open(duckdb_path).expect("create events duckdb file");
    }

    fs::create_dir_all(
        cfg.blobs
            .resolve_local_path_for_repo(repo)
            .expect("resolve blob store path"),
    )
    .expect("create local blob store directory");

    let runtime_path = bitloops::config::resolve_repo_runtime_db_path_for_repo(repo)
        .expect("resolve runtime sqlite path for integration test repo");
    if let Some(parent) = runtime_path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent).expect("create runtime sqlite parent");
    }
    let runtime = bitloops::storage::SqliteConnectionPool::connect(runtime_path)
        .expect("create runtime sqlite file");
    runtime
        .initialise_runtime_checkpoint_schema()
        .expect("initialise runtime checkpoint schema");
}

pub fn apply_repo_app_paths(cmd: &mut Command, paths: &RepoAppPaths) {
    cmd.env("HOME", &paths.home)
        .env("USERPROFILE", &paths.home)
        .env("XDG_CONFIG_HOME", &paths.xdg_config)
        .env("XDG_DATA_HOME", &paths.xdg_data)
        .env("XDG_CACHE_HOME", &paths.xdg_cache)
        .env("XDG_STATE_HOME", &paths.xdg_state)
        .env(TEST_STATE_DIR_OVERRIDE_ENV, &paths.test_state)
        .env(DISABLE_WATCHER_AUTOSTART_ENV, "1")
        .env(DISABLE_VERSION_CHECK_ENV, "1");
}

pub fn repo_app_paths(repo: &Path) -> RepoAppPaths {
    isolated_app_paths(repo)
}

pub fn isolated_repo_aux_dir(repo: &Path, name: &str) -> PathBuf {
    let canonical = repo.canonicalize().unwrap_or_else(|_| repo.to_path_buf());
    let mut hasher = DefaultHasher::new();
    canonical.hash(&mut hasher);
    name.hash(&mut hasher);
    let hash = hasher.finish();

    let parent = repo
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(std::env::temp_dir);
    let dir = parent.join(format!(".bitloops-test-{name}-{hash:016x}"));
    fs::create_dir_all(&dir).expect("create isolated Bitloops test helper dir");
    dir
}

#[allow(dead_code)]
pub fn with_repo_app_env<T>(repo: &Path, f: impl FnOnce() -> T) -> T {
    let _guard = enter_repo_app_env(repo);
    f()
}

pub fn enter_repo_app_env(repo: &Path) -> RepoAppEnvGuard {
    let paths = repo_app_paths(repo);
    enter_repo_app_paths(&paths)
}

pub fn enter_repo_app_paths(paths: &RepoAppPaths) -> RepoAppEnvGuard {
    let lock_guard = env_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let previous_env = apply_env_vars(&[
        ("HOME", Some(paths.home.as_os_str())),
        ("USERPROFILE", Some(paths.home.as_os_str())),
        ("XDG_CONFIG_HOME", Some(paths.xdg_config.as_os_str())),
        ("XDG_DATA_HOME", Some(paths.xdg_data.as_os_str())),
        ("XDG_CACHE_HOME", Some(paths.xdg_cache.as_os_str())),
        ("XDG_STATE_HOME", Some(paths.xdg_state.as_os_str())),
        (
            TEST_STATE_DIR_OVERRIDE_ENV,
            Some(paths.test_state.as_os_str()),
        ),
    ]);
    RepoAppEnvGuard {
        _lock_guard: lock_guard,
        previous_env,
    }
}

pub struct RepoAppEnvGuard {
    _lock_guard: MutexGuard<'static, ()>,
    previous_env: Vec<(String, Option<OsString>)>,
}

impl Drop for RepoAppEnvGuard {
    fn drop(&mut self) {
        restore_env_vars(&self.previous_env);
    }
}

#[derive(Clone, Debug)]
pub struct RepoAppPaths {
    pub home: PathBuf,
    pub xdg_config: PathBuf,
    pub xdg_data: PathBuf,
    pub xdg_cache: PathBuf,
    pub xdg_state: PathBuf,
    pub test_state: PathBuf,
}

fn isolated_app_paths(repo: &Path) -> RepoAppPaths {
    let home = isolated_repo_aux_dir(repo, "home");
    let paths = RepoAppPaths {
        xdg_config: home.join("xdg-config"),
        xdg_data: home.join("xdg-data"),
        xdg_cache: home.join("xdg-cache"),
        xdg_state: home.join("xdg-state"),
        test_state: repo_test_state_root(repo),
        home,
    };
    for dir in [
        &paths.home,
        &paths.xdg_config,
        &paths.xdg_data,
        &paths.xdg_cache,
        &paths.xdg_state,
        &paths.test_state,
    ] {
        fs::create_dir_all(dir).expect("create isolated Bitloops app dir");
    }
    paths
}

fn repo_test_state_root(repo: &Path) -> PathBuf {
    let canonical = repo.canonicalize().unwrap_or_else(|_| repo.to_path_buf());
    let mut hasher = DefaultHasher::new();
    canonical.hash(&mut hasher);
    let hash = hasher.finish();

    std::env::temp_dir()
        .join("bitloops-test-state")
        .join(format!("process-{}", std::process::id()))
        .join("repos")
        .join(format!("{hash:016x}"))
}

fn env_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

fn apply_env_vars(vars: &[(&str, Option<&std::ffi::OsStr>)]) -> Vec<(String, Option<OsString>)> {
    let mut previous = Vec::with_capacity(vars.len());
    for (key, value) in vars {
        previous.push(((*key).to_string(), std::env::var_os(key)));
        // SAFETY: integration tests serialise process env mutation through env_lock().
        unsafe {
            match value {
                Some(v) => std::env::set_var(key, v),
                None => std::env::remove_var(key),
            }
        }
    }
    previous
}

fn restore_env_vars(previous: &[(String, Option<OsString>)]) {
    for (key, value) in previous.iter().rev() {
        // SAFETY: integration tests serialise process env mutation through env_lock().
        unsafe {
            match value {
                Some(v) => std::env::set_var(key, v),
                None => std::env::remove_var(key),
            }
        }
    }
}
