pub mod augmentation;
pub mod dispatcher;
pub mod git;
pub mod runtime;

pub const BITLOOPS_SUPPRESS_AGENT_HOOKS_ENV: &str = "BITLOOPS_SUPPRESS_AGENT_HOOKS";

pub(crate) fn agent_hooks_suppressed_by_env() -> bool {
    std::env::var(BITLOOPS_SUPPRESS_AGENT_HOOKS_ENV)
        .map(|value| {
            let trimmed = value.trim();
            !trimmed.is_empty()
                && !matches!(
                    trimmed.to_ascii_lowercase().as_str(),
                    "0" | "false" | "no" | "off"
                )
        })
        .unwrap_or(false)
}
