use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct ResolvedDaemonConfig {
    pub config_path: PathBuf,
    pub config_root: PathBuf,
    pub relational_db_path: PathBuf,
    pub events_db_path: PathBuf,
    pub blob_store_path: PathBuf,
    pub repo_registry_path: PathBuf,
}
