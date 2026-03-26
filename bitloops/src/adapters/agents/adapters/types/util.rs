use anyhow::{Result, bail};

pub(crate) fn normalise_key(value: &str) -> Result<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        bail!("missing agent name");
    }
    Ok(trimmed.to_ascii_lowercase())
}
