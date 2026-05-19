use std::path::{Path, PathBuf};

pub(super) fn canonical_config_path(config_path: &Path) -> PathBuf {
    config_path
        .canonicalize()
        .unwrap_or_else(|_| config_path.to_path_buf())
}

pub(super) fn config_path_key(config_path: &Path) -> String {
    canonical_config_path(config_path).display().to_string()
}
