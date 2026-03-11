use std::env;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Mutex, MutexGuard, OnceLock};

#[allow(dead_code)]
pub(crate) const GIT_ENV_KEYS: [&str; 6] = [
    "GIT_DIR",
    "GIT_WORK_TREE",
    "GIT_INDEX_FILE",
    "GIT_OBJECT_DIRECTORY",
    "GIT_ALTERNATE_OBJECT_DIRECTORIES",
    "GIT_COMMON_DIR",
];

fn process_state_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

#[allow(dead_code)]
pub(crate) fn strip_inherited_git_env(cmd: &mut Command) {
    for key in GIT_ENV_KEYS {
        cmd.env_remove(key);
    }
}

#[allow(dead_code)]
pub(crate) fn git_command() -> Command {
    let mut cmd = Command::new("git");
    strip_inherited_git_env(&mut cmd);
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
            crate::engine::paths::clear_repo_root_cache();
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
    let previous_env = apply_env_vars(env_vars);

    if let Some(path) = cwd {
        env::set_current_dir(path).expect("set cwd");
        crate::engine::paths::clear_repo_root_cache();
    }

    ProcessStateGuard {
        _lock_guard: lock_guard,
        previous_env,
        original_cwd,
    }
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

#[allow(dead_code)]
pub(crate) fn with_env_var<T>(key: &str, value: Option<&str>, f: impl FnOnce() -> T) -> T {
    with_process_state(None, &[(key, value)], f)
}

pub(crate) fn with_env_vars<T>(vars: &[(&str, Option<&str>)], f: impl FnOnce() -> T) -> T {
    with_process_state(None, vars, f)
}

#[allow(dead_code)]
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
}
