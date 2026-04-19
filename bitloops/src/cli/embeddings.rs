mod args;
pub(crate) mod managed;
mod profiles;

#[cfg(test)]
mod tests;

pub(crate) use args::enqueue_embeddings_bootstrap_task;
pub(crate) use args::platform_embeddings_gateway_url_override;
pub(crate) use args::run_async;
pub use args::{
    EmbeddingsArgs, EmbeddingsClearCacheArgs, EmbeddingsCommand, EmbeddingsDoctorArgs,
    EmbeddingsInstallArgs, EmbeddingsPullArgs, EmbeddingsRuntime, run,
};
#[allow(unused_imports)]
pub(crate) use managed::{
    ensure_managed_embeddings_runtime_with_progress,
    install_managed_platform_embeddings_binary_with_progress, install_or_bootstrap_embeddings,
    install_or_configure_platform_embeddings, managed_embeddings_binary_dir,
    managed_embeddings_binary_path, managed_embeddings_metadata_path,
    managed_platform_runtime_command_is_eligible, managed_platform_runtime_version_for_command,
    managed_runtime_command_is_eligible, managed_runtime_version_for_command,
};
pub(crate) use profiles::{
    EmbeddingsInstallState, PulledEmbeddingProfileOutcome, embedding_capability_for_config_path,
    inspect_embeddings_install_state, pull_profile_with_config_path_and_progress,
    selected_inference_profile_name,
};

#[cfg(test)]
pub(crate) use managed::{
    ManagedEmbeddingsBinaryInstallOutcome, ManagedPlatformEmbeddingsBinaryInstallOutcome,
    with_managed_embeddings_install_hook, with_managed_platform_embeddings_install_hook,
};

#[cfg(test)]
pub(crate) use profiles::{clear_cache_for_profile, doctor_profile, pull_profile};
