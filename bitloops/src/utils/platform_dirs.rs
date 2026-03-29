use anyhow::{Context, Result, bail};
use std::path::PathBuf;

const APP_DIR_NAME: &str = "bitloops";

pub fn bitloops_config_dir() -> Result<PathBuf> {
    dirs::config_dir()
        .map(|dir| dir.join(APP_DIR_NAME))
        .context("resolving Bitloops config directory")
}

pub fn bitloops_data_dir() -> Result<PathBuf> {
    dirs::data_dir()
        .map(|dir| dir.join(APP_DIR_NAME))
        .context("resolving Bitloops data directory")
}

pub fn bitloops_state_dir() -> Result<PathBuf> {
    if let Some(dir) = dirs::state_dir() {
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
