mod args;
mod managed;
mod setup;

#[cfg(test)]
mod tests;

pub use args::{InferenceArgs, InferenceCommand, InferenceInstallArgs, run};
#[allow(unused_imports)]
pub(crate) use managed::{
    ensure_managed_inference_runtime, install_or_bootstrap_inference, managed_inference_binary_dir,
    managed_inference_binary_path, managed_inference_metadata_path,
    managed_runtime_command_is_eligible, managed_runtime_version_for_command,
};
pub(crate) use setup::{configure_local_summary_generation, summary_generation_configured};

#[cfg(test)]
pub(crate) use managed::{
    ManagedInferenceBinaryInstallOutcome, with_managed_inference_install_hook,
};

#[cfg(test)]
pub(crate) use setup::{OllamaAvailability, SummarySetupOutcome, with_ollama_probe_hook};

#[cfg(test)]
pub(crate) use setup::with_summary_generation_configured_hook;
