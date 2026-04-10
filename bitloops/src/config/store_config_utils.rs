use anyhow::{Context, Result, anyhow, bail};
use serde_json::{Map, Value};
use std::env;
use std::path::{Path, PathBuf};

use crate::utils::paths;

pub(super) fn current_repo_root_or_cwd_result() -> Result<PathBuf> {
    paths::repo_root()
        .or_else(|_| env::current_dir().context("resolving current directory for repo config"))
}

pub(super) fn current_repo_root_or_cwd() -> PathBuf {
    current_repo_root_or_cwd_result().unwrap_or_else(|_| PathBuf::from("."))
}

pub(super) fn user_home_dir() -> Option<PathBuf> {
    env::var_os("HOME")
        .or_else(|| env::var_os("USERPROFILE"))
        .map(PathBuf::from)
}

pub(super) fn normalize_sqlite_path(raw_path: &str, repo_root: &Path) -> Result<PathBuf> {
    normalize_repo_scoped_path(
        raw_path,
        repo_root,
        "sqlite path is empty; set `stores.relational.sqlite_path`",
    )
}

pub(super) fn normalize_blob_path(raw_path: &str, repo_root: &Path) -> Result<PathBuf> {
    normalize_repo_scoped_path(
        raw_path,
        repo_root,
        "blob local path is empty; set `stores.blob.local_path`",
    )
}

fn normalize_repo_scoped_path(
    raw_path: &str,
    repo_root: &Path,
    empty_err: &str,
) -> Result<PathBuf> {
    let trimmed = raw_path.trim();
    if trimmed.is_empty() {
        bail!("{empty_err}");
    }

    let expanded = expand_home_prefix(trimmed)?;
    let candidate = Path::new(&expanded).to_path_buf();
    if candidate.is_absolute() {
        Ok(candidate)
    } else {
        Ok(repo_root.join(candidate))
    }
}

pub(super) fn resolve_configured_path(raw_path: &str, repo_root: &Path) -> PathBuf {
    let expanded = expand_tilde_path(raw_path);
    if expanded.is_absolute() {
        expanded
    } else {
        repo_root.join(expanded)
    }
}

fn expand_home_prefix(path: &str) -> Result<String> {
    let home = user_home_dir();
    expand_home_prefix_with(path, home.as_deref())
}

pub(super) fn expand_home_prefix_with(path: &str, home: Option<&Path>) -> Result<String> {
    if path == "~" {
        let Some(home) = home else {
            bail!("unable to resolve home directory for `~` path");
        };
        return Ok(home.to_string_lossy().to_string());
    }

    if let Some(rest) = path.strip_prefix("~/").or_else(|| path.strip_prefix("~\\")) {
        let Some(home) = home else {
            bail!("unable to resolve home directory for `~` path");
        };
        return Ok(home.join(rest).to_string_lossy().to_string());
    }

    Ok(path.to_string())
}

pub(super) fn read_any_string(root: &Map<String, Value>, keys: &[&str]) -> Option<String> {
    for key in keys {
        if let Some(value) = root.get(*key).and_then(Value::as_str) {
            let trimmed = value.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
    }
    None
}

pub(super) fn read_any_string_opt(
    root: Option<&Map<String, Value>>,
    keys: &[&str],
) -> Option<String> {
    root.and_then(|map| read_any_string(map, keys))
}

pub(super) fn read_any_bool(root: &Map<String, Value>, keys: &[&str]) -> Option<bool> {
    for key in keys {
        let Some(value) = root.get(*key) else {
            continue;
        };
        if let Some(boolean) = value.as_bool() {
            return Some(boolean);
        }
        if let Some(raw) = value.as_str() {
            match raw.trim().to_ascii_lowercase().as_str() {
                "true" | "1" | "yes" | "on" => return Some(true),
                "false" | "0" | "no" | "off" => return Some(false),
                _ => {}
            }
        }
    }
    None
}

pub(super) fn read_any_u64(root: &Map<String, Value>, keys: &[&str]) -> Option<u64> {
    for key in keys {
        let Some(value) = root.get(*key) else {
            continue;
        };

        if let Some(number) = value.as_u64() {
            return Some(number);
        }
        if let Some(number) = value.as_i64().filter(|number| *number >= 0) {
            return Some(number as u64);
        }
        if let Some(raw) = value.as_str()
            && let Ok(number) = raw.trim().parse::<u64>()
        {
            return Some(number);
        }
    }
    None
}

pub(super) fn read_any_u64_opt(root: Option<&Map<String, Value>>, keys: &[&str]) -> Option<u64> {
    root.and_then(|map| read_any_u64(map, keys))
}

pub(super) fn read_non_empty_env<F>(env_lookup: &F, key: &str) -> Option<String>
where
    F: Fn(&str) -> Option<String>,
{
    env_lookup(key).and_then(|value| {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    })
}

#[allow(dead_code)]
pub(super) fn resolve_optional_env_indirection<F>(
    raw: Option<String>,
    env_lookup: &F,
) -> Option<String>
where
    F: Fn(&str) -> Option<String>,
{
    raw.and_then(|value| resolve_optional_env_indirection_str(&value, env_lookup))
}

#[allow(dead_code)]
fn resolve_optional_env_indirection_str<F>(raw: &str, env_lookup: &F) -> Option<String>
where
    F: Fn(&str) -> Option<String>,
{
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }

    if let Some(key) = trimmed
        .strip_prefix("${")
        .and_then(|value| value.strip_suffix('}'))
    {
        return read_non_empty_env(env_lookup, key);
    }

    Some(trimmed.to_string())
}

fn expand_tilde_path(raw: &str) -> PathBuf {
    let trimmed = raw.trim();
    if trimmed == "~" {
        return env::var_os("HOME")
            .or_else(|| env::var_os("USERPROFILE"))
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(trimmed));
    }

    if let Some(rest) = trimmed
        .strip_prefix("~/")
        .or_else(|| trimmed.strip_prefix("~\\"))
        && let Some(home) = env::var_os("HOME").or_else(|| env::var_os("USERPROFILE"))
    {
        return PathBuf::from(home).join(rest);
    }

    PathBuf::from(trimmed)
}

pub(super) fn resolve_required_provider_string<F>(
    map: &Map<String, Value>,
    key: &str,
    env_lookup: &F,
    section: &str,
) -> Result<String>
where
    F: Fn(&str) -> Option<String>,
{
    let raw = map
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("missing `{section}.{key}`"))?;
    resolve_provider_string(raw, env_lookup).with_context(|| format!("resolving `{section}.{key}`"))
}

fn resolve_provider_string<F>(raw: &str, env_lookup: &F) -> Result<String>
where
    F: Fn(&str) -> Option<String>,
{
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        bail!("provider value must not be empty");
    }

    if let Some(key) = trimmed
        .strip_prefix("${")
        .and_then(|value| value.strip_suffix('}'))
    {
        let env_value =
            env_lookup(key).ok_or_else(|| anyhow!("environment variable `{key}` is not set"))?;
        let env_trimmed = env_value.trim();
        if env_trimmed.is_empty() {
            bail!("environment variable `{key}` is empty");
        }
        Ok(env_trimmed.to_string())
    } else {
        Ok(trimmed.to_string())
    }
}
