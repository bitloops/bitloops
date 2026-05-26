use std::env;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use async_graphql::SimpleObject;

#[derive(Debug, Clone, SimpleObject)]
pub(crate) struct RuntimeExecutableResolutionObject {
    pub(crate) command: String,
    pub(crate) path: Option<String>,
    pub(crate) found: bool,
}

pub(crate) fn resolve_runtime_executable_resolutions(
    commands: Vec<String>,
) -> Vec<RuntimeExecutableResolutionObject> {
    commands
        .into_iter()
        .map(|command| {
            let path = resolve_executable(&command);
            RuntimeExecutableResolutionObject {
                command,
                path: path.as_ref().map(|path| path.display().to_string()),
                found: path.is_some(),
            }
        })
        .collect()
}

fn resolve_executable(command: &str) -> Option<PathBuf> {
    let command = command.trim();
    if command.is_empty() {
        return None;
    }

    if is_path_like_command(command) {
        let candidate = PathBuf::from(command);
        return executable_path_if_file(&candidate);
    }

    let path_env = env::var_os("PATH");
    resolve_command_on_path(command, path_env.as_deref())
}

fn is_path_like_command(command: &str) -> bool {
    Path::new(command).is_absolute() || command.contains('/') || command.contains('\\')
}

fn resolve_command_on_path(command: &str, path_env: Option<&OsStr>) -> Option<PathBuf> {
    let path_env = path_env?;
    env::split_paths(path_env).find_map(|dir| {
        executable_candidates(&dir, command)
            .into_iter()
            .find_map(|candidate| executable_path_if_file(&candidate))
    })
}

fn executable_candidates(dir: &Path, command: &str) -> [PathBuf; 4] {
    [
        dir.join(command),
        dir.join(format!("{command}.exe")),
        dir.join(format!("{command}.cmd")),
        dir.join(format!("{command}.bat")),
    ]
}

fn executable_path_if_file(path: &Path) -> Option<PathBuf> {
    if !is_executable_file(path) {
        return None;
    }

    Some(path.canonicalize().unwrap_or_else(|_| path.to_path_buf()))
}

#[cfg(unix)]
fn is_executable_file(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;

    path.metadata()
        .map(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

#[cfg(not(unix))]
fn is_executable_file(path: &Path) -> bool {
    path.is_file()
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::TempDir;

    use super::*;

    #[cfg(unix)]
    fn make_executable(path: &Path) {
        use std::os::unix::fs::PermissionsExt;

        let mut permissions = path.metadata().expect("metadata").permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(path, permissions).expect("set executable permissions");
    }

    #[cfg(not(unix))]
    fn make_executable(_path: &Path) {}

    #[test]
    fn resolves_bare_command_from_supplied_path() {
        let temp = TempDir::new().expect("temp dir");
        let executable = temp.path().join("codex");
        fs::write(&executable, b"#!/bin/sh\n").expect("write executable");
        make_executable(&executable);
        let path_env = env::join_paths([temp.path()]).expect("join path");

        let resolved = resolve_command_on_path("codex", Some(&path_env)).expect("resolved path");

        assert_eq!(resolved, executable.canonicalize().expect("canonical path"));
    }

    #[test]
    fn resolves_windows_command_extensions_from_supplied_path() {
        let temp = TempDir::new().expect("temp dir");
        let executable = temp.path().join("codex.cmd");
        fs::write(&executable, b"@echo off\n").expect("write executable");
        make_executable(&executable);
        let path_env = env::join_paths([temp.path()]).expect("join path");

        let resolved = resolve_command_on_path("codex", Some(&path_env)).expect("resolved path");

        assert_eq!(resolved, executable.canonicalize().expect("canonical path"));
    }
}
