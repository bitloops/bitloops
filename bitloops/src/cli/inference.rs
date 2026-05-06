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
pub use setup::TextGenerationRuntime;
pub(crate) use setup::{
    BitloopsInferenceSetupSelection, ContextGuidanceSetupSelection,
    DEFAULT_PLATFORM_CONTEXT_GUIDANCE_API_KEY_ENV, PreparedSummarySetupAction,
    PreparedSummarySetupPlan, SummarySetupExecutionResult, SummarySetupOutcome, SummarySetupPhase,
    SummarySetupProgress, SummarySetupSelection, bitloops_inference_generation_configured,
    configure_cloud_bitloops_inference, configure_cloud_context_guidance_generation,
    configure_cloud_summary_generation, configure_local_bitloops_inference,
    configure_local_context_guidance_generation, configure_local_summary_generation,
    context_guidance_generation_configured,
    execute_prepared_bitloops_inference_setup_with_progress,
    execute_prepared_summary_setup_with_progress, platform_context_guidance_gateway_url_override,
    platform_summary_gateway_url_override, prepare_cloud_bitloops_inference_plan,
    prepare_cloud_summary_generation_plan, prepare_local_bitloops_inference_plan,
    prompt_bitloops_inference_setup_selection, prompt_context_guidance_setup_selection,
    prompt_summary_setup_selection, summary_generation_configured,
};

#[cfg(test)]
pub(crate) use managed::{
    ManagedInferenceBinaryInstallOutcome, with_managed_inference_install_hook,
};

#[cfg(test)]
pub(crate) use setup::{OllamaAvailability, with_ollama_probe_hook};

#[cfg(test)]
pub(crate) use setup::{
    ContextGuidanceSetupOutcome, with_bitloops_inference_generation_configured_hook,
    with_context_guidance_generation_configured_hook, with_summary_generation_configured_hook,
};
