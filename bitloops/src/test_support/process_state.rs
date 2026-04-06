use std::collections::hash_map::DefaultHasher;
use std::env;
use std::ffi::OsString;
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Mutex, MutexGuard, OnceLock};

pub(crate) const GIT_ENV_KEYS: [&str; 12] = [
    "GIT_DIR",
    "GIT_WORK_TREE",
    "GIT_INDEX_FILE",
    "GIT_OBJECT_DIRECTORY",
    "GIT_ALTERNATE_OBJECT_DIRECTORIES",
    "GIT_COMMON_DIR",
    "GIT_CONFIG",
    "GIT_CONFIG_PARAMETERS",
    "GIT_CONFIG_COUNT",
    "GIT_CONFIG_GLOBAL",
    "GIT_CONFIG_SYSTEM",
    "GIT_CONFIG_NOSYSTEM",
];
pub(crate) const ALLOW_HOST_GIT_CONFIG_ENV: &str = "BITLOOPS_TEST_ALLOW_HOST_GIT_CONFIG";
pub(crate) const SUPPRESS_HOST_DAEMON_CONFIG_ENV: &str =
    "BITLOOPS_TEST_SUPPRESS_HOST_DAEMON_CONFIG";

fn process_state_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

pub(crate) fn strip_inherited_git_env(cmd: &mut Command) {
    for key in GIT_ENV_KEYS {
        cmd.env_remove(key);
    }
}

fn isolated_git_config_path(scope: &str) -> PathBuf {
    let config_dir = env::temp_dir().join("bitloops-test-git-config");
    fs::create_dir_all(&config_dir).expect("create isolated git config dir");
    let path = config_dir.join(format!("{scope}-{}.gitconfig", std::process::id()));
    if !path.exists() {
        fs::write(&path, "").expect("create isolated git config");
    }
    path
}

pub(crate) fn git_command() -> Command {
    let mut cmd = Command::new("git");
    strip_inherited_git_env(&mut cmd);
    if env::var_os(ALLOW_HOST_GIT_CONFIG_ENV).is_none() {
        let global_config = isolated_git_config_path("default");
        cmd.env("GIT_CONFIG_GLOBAL", global_config)
            .env("GIT_CONFIG_NOSYSTEM", "1");
    }
    cmd
}

pub(crate) fn isolated_git_command(repo_root: &Path) -> Command {
    let mut hasher = DefaultHasher::new();
    repo_root.hash(&mut hasher);
    let repo_hash = hasher.finish();

    let global_config = isolated_git_config_path(&format!("repo-{repo_hash:016x}"));

    let mut cmd = git_command();
    cmd.current_dir(repo_root)
        .env("GIT_CONFIG_GLOBAL", &global_config)
        .env("GIT_CONFIG_NOSYSTEM", "1");
    cmd
}

fn apply_env_vars(vars: &[(&str, Option<&str>)]) -> Vec<(String, Option<OsString>)> {
    let mut previous = Vec::with_capacity(vars.len());

    for (key, value) in vars {
        if previous.iter().all(|(seen, _)| seen != key) {
            previous.push(((*key).to_string(), env::var_os(key)));
        }

        // SAFETY: callers serialize mutation through process_state_lock().
        unsafe {
            match value {
                Some(v) => env::set_var(key, v),
                None => env::remove_var(key),
            }
        }
    }

    previous
}

fn restore_env_vars(previous: &[(String, Option<OsString>)]) {
    for (key, value) in previous.iter().rev() {
        // SAFETY: callers serialize mutation through process_state_lock().
        unsafe {
            match value {
                Some(v) => env::set_var(key, v),
                None => env::remove_var(key),
            }
        }
    }
}

fn fallback_cwd() -> PathBuf {
    env::current_dir().unwrap_or_else(|_| PathBuf::from(env!("CARGO_MANIFEST_DIR")))
}

// Shared test helper for process-global cwd/env mutation.
pub(crate) struct ProcessStateGuard {
    _lock_guard: MutexGuard<'static, ()>,
    previous_env: Vec<(String, Option<OsString>)>,
    original_cwd: Option<PathBuf>,
}

impl Drop for ProcessStateGuard {
    fn drop(&mut self) {
        if let Some(old) = &self.original_cwd {
            if env::set_current_dir(old).is_err() {
                let _ = env::set_current_dir(env!("CARGO_MANIFEST_DIR"));
            }
            crate::utils::paths::clear_repo_root_cache();
        }

        restore_env_vars(&self.previous_env);
    }
}

pub(crate) fn enter_process_state(
    cwd: Option<&Path>,
    env_vars: &[(&str, Option<&str>)],
) -> ProcessStateGuard {
    let lock_guard = process_state_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());

    let original_cwd = cwd.map(|_| fallback_cwd());
    let mut effective_env = env_vars.to_vec();
    if should_suppress_host_daemon_config(env_vars)
        && !effective_env
            .iter()
            .any(|(key, _)| *key == SUPPRESS_HOST_DAEMON_CONFIG_ENV)
    {
        effective_env.push((SUPPRESS_HOST_DAEMON_CONFIG_ENV, Some("1")));
    }

    let previous_env = apply_env_vars(&effective_env);

    if let Some(path) = cwd {
        env::set_current_dir(path).expect("set cwd");
        crate::utils::paths::clear_repo_root_cache();
    }

    ProcessStateGuard {
        _lock_guard: lock_guard,
        previous_env,
        original_cwd,
    }
}

fn should_suppress_host_daemon_config(env_vars: &[(&str, Option<&str>)]) -> bool {
    let exposes_config_root = env_vars.iter().any(|(key, value)| {
        value.is_some()
            && matches!(
                *key,
                "BITLOOPS_TEST_CONFIG_DIR_OVERRIDE" | "HOME" | "XDG_CONFIG_HOME" | "APPDATA"
            )
    });
    !exposes_config_root
}

pub(crate) fn enter_env_vars(env_vars: &[(&str, Option<&str>)]) -> ProcessStateGuard {
    enter_process_state(None, env_vars)
}

pub(crate) fn with_process_state<T>(
    cwd: Option<&Path>,
    env_vars: &[(&str, Option<&str>)],
    f: impl FnOnce() -> T,
) -> T {
    let _guard = enter_process_state(cwd, env_vars);
    f()
}

pub(crate) fn with_cwd<T>(path: &Path, f: impl FnOnce() -> T) -> T {
    with_process_state(Some(path), &[], f)
}

pub(crate) fn with_env_var<T>(key: &str, value: Option<&str>, f: impl FnOnce() -> T) -> T {
    with_process_state(None, &[(key, value)], f)
}

pub(crate) fn with_env_vars<T>(vars: &[(&str, Option<&str>)], f: impl FnOnce() -> T) -> T {
    with_process_state(None, vars, f)
}

pub(crate) fn with_git_env_cleared<T>(f: impl FnOnce() -> T) -> T {
    let mut updates = Vec::with_capacity(GIT_ENV_KEYS.len());
    for key in GIT_ENV_KEYS {
        updates.push((key, None));
    }
    with_process_state(None, &updates, f)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsStr;
    use std::sync::atomic::{AtomicBool, Ordering};

    #[test]
    fn strip_inherited_git_env_removes_poisoned_git_dir_from_child() {
        let dir = tempfile::tempdir().unwrap();

        let init = Command::new("git")
            .args(["init"])
            .current_dir(dir.path())
            .output()
            .unwrap();
        assert!(init.status.success(), "git init should succeed");

        let mut cmd = Command::new("git");
        cmd.args(["rev-parse", "--git-dir"])
            .current_dir(dir.path())
            .env("GIT_DIR", "/tmp/not-a-repo/.git");
        strip_inherited_git_env(&mut cmd);

        let out = cmd.output().unwrap();
        assert!(
            out.status.success(),
            "expected sanitized child git command to succeed"
        );
        assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), ".git");
    }

    #[test]
    fn git_command_sets_program_to_git() {
        let cmd = git_command();
        assert_eq!(cmd.get_program(), OsStr::new("git"));
    }

    #[test]
    fn isolated_git_command_sets_empty_global_config_and_disables_system_config() {
        let dir = tempfile::tempdir().unwrap();
        let cmd = isolated_git_command(dir.path());
        let envs = cmd
            .get_envs()
            .filter_map(|(key, value)| Some((key.to_str()?, value?.to_str()?)))
            .collect::<Vec<_>>();

        let global_config = envs
            .iter()
            .find_map(|(key, value)| (*key == "GIT_CONFIG_GLOBAL").then_some(*value))
            .expect("GIT_CONFIG_GLOBAL should be set");
        let global_config_path = PathBuf::from(global_config);

        assert!(global_config_path.exists());
        assert!(global_config_path.starts_with(env::temp_dir()));
        assert!(!global_config_path.starts_with(dir.path()));
        assert!(
            global_config_path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("repo-") && name.ends_with(".gitconfig"))
        );
        assert!(
            envs.iter()
                .any(|(key, value)| *key == "GIT_CONFIG_NOSYSTEM" && *value == "1")
        );
    }

    #[test]
    fn with_git_env_cleared_runs_closure() {
        let called = AtomicBool::new(false);
        with_git_env_cleared(|| {
            called.store(true, Ordering::SeqCst);
        });
        assert!(called.load(Ordering::SeqCst));
    }
}
