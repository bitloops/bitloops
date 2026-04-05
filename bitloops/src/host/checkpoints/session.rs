use std::path::PathBuf;

use anyhow::Result;

pub mod backend;
pub mod db_backend;
pub mod local_backend;
pub mod phase;
pub mod state;

pub use backend::SessionBackend;
pub use db_backend::DbSessionBackend;

pub fn create_session_backend(repo_root: impl Into<PathBuf>) -> Result<Box<dyn SessionBackend>> {
    let root = repo_root.into();
    let backend = DbSessionBackend::for_repo_root(&root)?;
    Ok(Box::new(backend))
}

pub fn create_session_backend_or_local(repo_root: impl Into<PathBuf>) -> Box<dyn SessionBackend> {
    let root = repo_root.into();
    match create_session_backend(root.clone()) {
        Ok(backend) => backend,
        Err(err) => {
            let reason = format!("failed to initialise DbSessionBackend: {err:#}");
            log::warn!("{reason}; falling back to LocalFileBackend");
            Box::new(local_backend::LocalFileBackend::new(root))
        }
    }
}
