use anyhow::{Context, Result, bail};
use std::path::PathBuf;

const APP_DIR_NAME: &str = "bitloops";
#[cfg(test)]
const TEST_CONFIG_DIR_OVERRIDE_ENV: &str = "BITLOOPS_TEST_CONFIG_DIR_OVERRIDE";
#[cfg(test)]
const TEST_DATA_DIR_OVERRIDE_ENV: &str = "BITLOOPS_TEST_DATA_DIR_OVERRIDE";
#[cfg(test)]
const TEST_CACHE_DIR_OVERRIDE_ENV: &str = "BITLOOPS_TEST_CACHE_DIR_OVERRIDE";
#[cfg(test)]
const TEST_STATE_DIR_OVERRIDE_ENV: &str = "BITLOOPS_TEST_STATE_DIR_OVERRIDE";

fn xdg_style_fallback(segments: &[&str]) -> Option<PathBuf> {
    let mut path = dirs::home_dir()?;
    for segment in segments {
        path.push(segment);
    }
    Some(path)
}

pub fn bitloops_config_dir() -> Result<PathBuf> {
    #[cfg(test)]
    if let Some(path) = std::env::var_os(TEST_CONFIG_DIR_OVERRIDE_ENV).filter(|v| !v.is_empty()) {
        return Ok(PathBuf::from(path).join(APP_DIR_NAME));
    }
    dirs::config_dir()
        .or_else(|| xdg_style_fallback(&[".config"]))
        .map(|dir| dir.join(APP_DIR_NAME))
        .context("resolving Bitloops config directory")
}

pub fn bitloops_data_dir() -> Result<PathBuf> {
    #[cfg(test)]
    if let Some(path) = std::env::var_os(TEST_DATA_DIR_OVERRIDE_ENV).filter(|v| !v.is_empty()) {
        return Ok(PathBuf::from(path).join(APP_DIR_NAME));
    }
    dirs::data_dir()
        .or_else(|| xdg_style_fallback(&[".local", "share"]))
        .map(|dir| dir.join(APP_DIR_NAME))
        .context("resolving Bitloops data directory")
}

pub fn bitloops_cache_dir() -> Result<PathBuf> {
    #[cfg(test)]
    if let Some(path) = std::env::var_os(TEST_CACHE_DIR_OVERRIDE_ENV).filter(|v| !v.is_empty()) {
        return Ok(PathBuf::from(path).join(APP_DIR_NAME));
    }
    dirs::cache_dir()
        .or_else(|| xdg_style_fallback(&[".cache"]))
        .map(|dir| dir.join(APP_DIR_NAME))
        .context("resolving Bitloops cache directory")
}

pub fn bitloops_state_dir() -> Result<PathBuf> {
    #[cfg(test)]
    if let Some(path) = std::env::var_os(TEST_STATE_DIR_OVERRIDE_ENV).filter(|v| !v.is_empty()) {
        return Ok(PathBuf::from(path).join(APP_DIR_NAME));
    }
    if let Some(dir) = dirs::state_dir() {
        return Ok(dir.join(APP_DIR_NAME));
    }
    if let Some(dir) = xdg_style_fallback(&[".local", "state"]) {
        return Ok(dir.join(APP_DIR_NAME));
    }
    bitloops_data_dir()
}

pub fn bitloops_home_dir() -> Result<PathBuf> {
    dirs::home_dir().context("resolving home directory")
}

pub fn bitloops_config_file_path() -> Result<PathBuf> {
    Ok(bitloops_config_dir()?.join("config.toml"))
}

pub fn ensure_parent_dir(path: &std::path::Path) -> Result<()> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .context("resolving parent directory")?;
    std::fs::create_dir_all(parent)
        .with_context(|| format!("creating directory {}", parent.display()))
}

pub fn ensure_dir(path: &std::path::Path) -> Result<()> {
    if path.as_os_str().is_empty() {
        bail!("cannot create empty directory path");
    }
    std::fs::create_dir_all(path).with_context(|| format!("creating directory {}", path.display()))
}
