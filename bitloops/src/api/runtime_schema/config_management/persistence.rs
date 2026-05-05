use std::fs;
use std::path::Path;

use anyhow::{Context as _, Result as AnyhowResult, anyhow};

pub(super) fn write_atomic(path: &Path, bytes: &[u8]) -> AnyhowResult<()> {
    let parent = path
        .parent()
        .ok_or_else(|| anyhow!("config target has no parent directory: {}", path.display()))?;
    let tmp = parent.join(format!(
        ".{}.tmp.{}",
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("bitloops-config"),
        std::process::id()
    ));
    fs::write(&tmp, bytes)
        .with_context(|| format!("writing temporary config file {}", tmp.display()))?;
    fs::rename(&tmp, path).with_context(|| {
        format!(
            "renaming temporary config file {} to {}",
            tmp.display(),
            path.display()
        )
    })?;
    Ok(())
}
