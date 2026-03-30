use super::*;
use crate::config::load_daemon_settings;
use crate::config::resolve_blob_local_path_for_repo;
use crate::config::unified_config::resolve_store_backend_from_unified;

pub(super) fn resolve_daemon_config(
    explicit_config_path: Option<&Path>,
) -> Result<ResolvedDaemonConfig> {
    let loaded =
        load_daemon_settings(explicit_config_path).context("resolving Bitloops daemon config")?;
    let config_path = loaded
        .path
        .canonicalize()
        .unwrap_or_else(|_| loaded.path.clone());
    let config_root = derive_config_root(&config_path)?;
    let backend_config = resolve_store_backend_from_unified(&loaded.settings, &config_root)
        .with_context(|| format!("resolving store backends from {}", config_path.display()))?;
    let relational_db_path = backend_config
        .relational
        .resolve_sqlite_db_path_for_repo(&config_root)
        .context("resolving SQLite path for Bitloops daemon")?;
    let events_db_path = backend_config
        .events
        .resolve_duckdb_db_path_for_repo(&config_root);
    let blob_store_path =
        resolve_blob_local_path_for_repo(&config_root, backend_config.blobs.local_path.as_deref())
            .context("resolving blob store path for Bitloops daemon")?;

    Ok(ResolvedDaemonConfig {
        config_path,
        config_root,
        relational_db_path,
        events_db_path,
        blob_store_path,
        repo_registry_path: global_daemon_dir()?.join("repo-path-registry.json"),
    })
}

fn derive_config_root(config_path: &Path) -> Result<PathBuf> {
    config_path
        .parent()
        .map(Path::to_path_buf)
        .context("resolving Bitloops daemon config directory")
}
