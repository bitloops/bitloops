pub(crate) mod config;
pub(crate) mod install;

pub(crate) use config::{
    managed_inference_binary_dir, managed_inference_binary_path, managed_inference_metadata_path,
    managed_runtime_command_is_eligible, managed_runtime_version_for_command,
};
#[allow(unused_imports)]
pub(crate) use install::{
    ManagedInferenceInstallPhase, ManagedInferenceInstallProgress,
    ensure_managed_inference_runtime, install_or_bootstrap_inference,
    install_or_bootstrap_inference_with_progress,
};

#[cfg(test)]
pub(crate) use install::{
    ManagedInferenceBinaryInstallOutcome, with_managed_inference_install_hook,
};
