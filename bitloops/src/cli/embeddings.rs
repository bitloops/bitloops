mod args;
mod managed;
mod profiles;

#[cfg(test)]
mod tests;

pub use args::{
    EmbeddingsArgs, EmbeddingsClearCacheArgs, EmbeddingsCommand, EmbeddingsDoctorArgs,
    EmbeddingsInstallArgs, EmbeddingsPullArgs, run,
};
pub(crate) use managed::{
    install_or_bootstrap_embeddings, managed_embeddings_binary_dir, managed_embeddings_binary_path,
    managed_embeddings_metadata_path,
};
pub(crate) use profiles::{EmbeddingsInstallState, inspect_embeddings_install_state};

#[cfg(test)]
pub(crate) use managed::{
    ManagedEmbeddingsBinaryInstallOutcome, with_managed_embeddings_install_hook,
};

#[cfg(test)]
pub(crate) use profiles::{clear_cache_for_profile, doctor_profile, pull_profile};
