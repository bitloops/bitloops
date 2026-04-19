pub(crate) mod archive;
pub(crate) mod config;
pub(crate) mod download;
pub(crate) mod install;
pub(crate) mod platform;

pub(crate) use config::{
    managed_embeddings_binary_dir, managed_embeddings_binary_path,
    managed_embeddings_metadata_path, managed_runtime_command_is_eligible,
    managed_runtime_version_for_command,
};
#[allow(unused_imports)]
pub(crate) use install::{
    ensure_managed_embeddings_runtime, ensure_managed_embeddings_runtime_with_progress,
    install_or_bootstrap_embeddings,
};
pub(crate) use platform::{
    install_managed_platform_embeddings_binary_with_progress,
    install_or_configure_platform_embeddings, managed_platform_runtime_command_is_eligible,
    managed_platform_runtime_version_for_command,
};

#[cfg(test)]
pub(crate) use install::{
    ManagedEmbeddingsBinaryInstallOutcome, with_managed_embeddings_install_hook,
};
#[cfg(test)]
pub(crate) use platform::{
    ManagedPlatformEmbeddingsBinaryInstallOutcome, with_managed_platform_embeddings_install_hook,
};
