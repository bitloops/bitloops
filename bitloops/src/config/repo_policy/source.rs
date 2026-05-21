use std::path::Path;

use anyhow::Result;

use super::discovery::{discover_repo_policy_with_mode, validate_repo_policy_text};
use super::types::RepoPolicySnapshot;

pub trait RepoPolicySource {
    fn discover_required(&self, start: &Path) -> Result<RepoPolicySnapshot>;

    fn discover_optional(&self, start: &Path) -> Result<RepoPolicySnapshot>;

    fn validate_text(&self, raw: &str, path: &Path) -> Result<()>;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct FileRepoPolicySource;

impl RepoPolicySource for FileRepoPolicySource {
    fn discover_required(&self, start: &Path) -> Result<RepoPolicySnapshot> {
        discover_repo_policy_with_mode(start, true)
    }

    fn discover_optional(&self, start: &Path) -> Result<RepoPolicySnapshot> {
        discover_repo_policy_with_mode(start, false)
    }

    fn validate_text(&self, raw: &str, path: &Path) -> Result<()> {
        validate_repo_policy_text(raw, path)
    }
}
