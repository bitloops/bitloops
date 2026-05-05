use anyhow::Result as AnyhowResult;

use crate::config::{validate_daemon_config_text, validate_repo_policy_text};

use super::types::{ConfigTarget, ConfigTargetKind};

pub(super) fn validate_target_text(target: &ConfigTarget, text: &str) -> AnyhowResult<()> {
    match target.kind {
        ConfigTargetKind::Daemon => validate_daemon_config_text(text, &target.path),
        ConfigTargetKind::RepoShared | ConfigTargetKind::RepoLocal => {
            validate_repo_policy_text(text, &target.path)
        }
    }
}
