use anyhow::{Context, Result, bail};
use std::path::PathBuf;

const APP_DIR_NAME: &str = "bitloops";
#[cfg(test)]
use std::cell::RefCell;
#[cfg(test)]
const TEST_CONFIG_DIR_OVERRIDE_ENV: &str = "BITLOOPS_TEST_CONFIG_DIR_OVERRIDE";
#[cfg(test)]
const TEST_DATA_DIR_OVERRIDE_ENV: &str = "BITLOOPS_TEST_DATA_DIR_OVERRIDE";
#[cfg(test)]
const TEST_CACHE_DIR_OVERRIDE_ENV: &str = "BITLOOPS_TEST_CACHE_DIR_OVERRIDE";
#[cfg(test)]
const TEST_STATE_DIR_OVERRIDE_ENV: &str = "BITLOOPS_TEST_STATE_DIR_OVERRIDE";

#[cfg(test)]
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct TestPlatformDirOverrides {
    pub(crate) config_root: Option<PathBuf>,
    pub(crate) data_root: Option<PathBuf>,
    pub(crate) cache_root: Option<PathBuf>,
    pub(crate) state_root: Option<PathBuf>,
}

#[cfg(test)]
thread_local! {
    static TEST_PLATFORM_DIR_OVERRIDES: RefCell<Option<TestPlatformDirOverrides>> =
        const { RefCell::new(None) };
}

#[cfg(test)]
pub(crate) fn with_test_platform_dir_overrides<T>(
    overrides: TestPlatformDirOverrides,
    f: impl FnOnce() -> T,
) -> T {
    TEST_PLATFORM_DIR_OVERRIDES.with(|cell| {
        assert!(
            cell.borrow().is_none(),
            "test platform dir overrides already installed"
        );
        *cell.borrow_mut() = Some(overrides);
    });
    let result = f();
    TEST_PLATFORM_DIR_OVERRIDES.with(|cell| {
        *cell.borrow_mut() = None;
    });
    result
}

#[cfg(test)]
fn test_platform_dir_override<F>(project: F) -> Option<PathBuf>
where
    F: FnOnce(&TestPlatformDirOverrides) -> Option<PathBuf>,
{
    TEST_PLATFORM_DIR_OVERRIDES.with(|cell| cell.borrow().as_ref().and_then(project))
}

#[cfg(test)]
pub(crate) fn explicit_test_state_dir() -> Option<PathBuf> {
    if let Some(path) = test_platform_dir_override(|overrides| overrides.state_root.clone()) {
        return Some(path.join(APP_DIR_NAME));
    }
    std::env::var_os(TEST_STATE_DIR_OVERRIDE_ENV)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .map(|path| path.join(APP_DIR_NAME))
}

fn xdg_style_fallback(segments: &[&str]) -> Option<PathBuf> {
    let mut path = dirs::home_dir()?;
    for segment in segments {
        path.push(segment);
    }
    Some(path)
}

pub fn bitloops_config_dir() -> Result<PathBuf> {
    #[cfg(test)]
    if let Some(path) = test_platform_dir_override(|overrides| overrides.config_root.clone()) {
        return Ok(path.join(APP_DIR_NAME));
    }
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
    if let Some(path) = test_platform_dir_override(|overrides| overrides.data_root.clone()) {
        return Ok(path.join(APP_DIR_NAME));
    }
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
    if let Some(path) = test_platform_dir_override(|overrides| overrides.cache_root.clone()) {
        return Ok(path.join(APP_DIR_NAME));
    }
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
    if let Some(path) = explicit_test_state_dir() {
        return Ok(path);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_platform_dir_overrides_scope_without_process_env() {
        let temp = tempfile::tempdir().expect("tempdir");

        with_test_platform_dir_overrides(
            TestPlatformDirOverrides {
                config_root: Some(temp.path().join("config-root")),
                data_root: Some(temp.path().join("data-root")),
                cache_root: Some(temp.path().join("cache-root")),
                state_root: Some(temp.path().join("state-root")),
            },
            || {
                assert_eq!(
                    bitloops_config_dir().expect("config dir"),
                    temp.path().join("config-root").join(APP_DIR_NAME)
                );
                assert_eq!(
                    bitloops_data_dir().expect("data dir"),
                    temp.path().join("data-root").join(APP_DIR_NAME)
                );
                assert_eq!(
                    bitloops_cache_dir().expect("cache dir"),
                    temp.path().join("cache-root").join(APP_DIR_NAME)
                );
                assert_eq!(
                    bitloops_state_dir().expect("state dir"),
                    temp.path().join("state-root").join(APP_DIR_NAME)
                );
            },
        );
    }
}
