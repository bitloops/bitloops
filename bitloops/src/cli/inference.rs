mod args;
mod managed;
mod setup;

#[cfg(test)]
mod tests;

pub use args::{InferenceArgs, InferenceCommand, InferenceInstallArgs, run};
#[allow(unused_imports)]
pub(crate) use managed::{
    ManagedInferenceInstallPhase, ManagedInferenceInstallProgress,
    ensure_managed_inference_runtime, install_or_bootstrap_inference,
    install_or_bootstrap_inference_with_progress, managed_inference_binary_dir,
    managed_inference_binary_path, managed_inference_metadata_path,
    managed_runtime_command_is_eligible, managed_runtime_version_for_command,
};
pub(crate) use setup::{
    PreparedSummarySetupPlan, SummarySetupExecutionResult, SummarySetupOutcome, SummarySetupPhase,
    SummarySetupProgress, SummarySetupSelection, configure_cloud_summary_generation,
    configure_local_summary_generation, execute_prepared_summary_setup_with_progress,
    platform_summary_gateway_url_override, prepare_local_summary_generation_plan,
    prompt_summary_setup_selection, summary_generation_configured,
};

#[cfg(test)]
pub(crate) use managed::{
    ManagedInferenceBinaryInstallOutcome, with_managed_inference_install_hook,
};

#[cfg(test)]
pub(crate) use setup::{OllamaAvailability, with_ollama_probe_hook};

#[cfg(test)]
pub(crate) use setup::with_summary_generation_configured_hook;
