use super::*;

pub(super) fn resolve_daemon_config(
    explicit_config_path: Option<&Path>,
) -> Result<ResolvedDaemonConfig> {
    let config_path = match explicit_config_path {
        Some(path) => expand_user_path(path)?,
        None => env::current_dir()
            .context("resolving current directory for Bitloops daemon config")?
            .join(BITLOOPS_CONFIG_RELATIVE_PATH),
    };
    if !config_path.is_file() {
        bail!(
            "Bitloops daemon config not found at {}. Pass `--config <path>` or run the command from a directory containing `./{}`.",
            config_path.display(),
            BITLOOPS_CONFIG_RELATIVE_PATH
        );
    }

    let config_path = config_path
        .canonicalize()
        .unwrap_or_else(|_| config_path.to_path_buf());
    let config_root = derive_config_root(&config_path)?;
    let backend_config = resolve_store_backend_config_for_repo(&config_root)
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
    let config_dir = config_path
        .parent()
        .context("resolving Bitloops daemon config directory")?;
    if config_dir
        .file_name()
        .and_then(|value| value.to_str())
        .is_some_and(|value| value == ".bitloops")
    {
        return config_dir
            .parent()
            .map(Path::to_path_buf)
            .context("resolving Bitloops daemon config root");
    }
    Ok(config_dir.to_path_buf())
}

fn expand_user_path(path: &Path) -> Result<PathBuf> {
    let raw = path.to_string_lossy();
    if raw == "~" {
        return user_home_dir();
    }
    if let Some(rest) = raw.strip_prefix("~/").or_else(|| raw.strip_prefix("~\\")) {
        return Ok(user_home_dir()?.join(rest));
    }
    Ok(path.to_path_buf())
}
