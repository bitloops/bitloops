use std::path::Path;

use crate::config::settings::{self, BitloopsSettings};

/// Returns true when the executed command or any ancestor is hidden.
///
/// `hidden_chain` order must be leaf -> ... -> root.
#[cfg(test)]
pub(crate) fn has_hidden_in_chain(hidden_chain: &[bool]) -> bool {
    hidden_chain.iter().copied().any(|is_hidden| is_hidden)
}

/// Loads settings once for root post-run side effects.
///
/// Settings load failures are tolerated and
/// downstream telemetry/version logic simply proceeds with partial data.
pub(crate) fn load_settings_once(repo_root: &Path) -> Option<BitloopsSettings> {
    settings::load_settings(repo_root).ok()
}
