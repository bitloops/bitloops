mod constants;
mod execution;
mod ollama;
mod profiles;
mod prompt;
mod types;

pub(crate) use execution::{
    configure_cloud_summary_generation, configure_local_summary_generation,
    execute_prepared_summary_setup_with_progress, prepare_cloud_summary_generation_plan,
    prepare_local_summary_generation_plan,
};
#[cfg(test)]
pub(crate) use ollama::with_ollama_probe_hook;
#[cfg(test)]
pub(crate) use profiles::with_summary_generation_configured_hook;
pub(crate) use profiles::{platform_summary_gateway_url_override, summary_generation_configured};
pub(crate) use prompt::prompt_summary_setup_selection;
#[cfg(test)]
pub(crate) use types::OllamaAvailability;
pub(crate) use types::{
    PreparedSummarySetupPlan, SummarySetupExecutionResult, SummarySetupOutcome, SummarySetupPhase,
    SummarySetupProgress, SummarySetupSelection,
};
