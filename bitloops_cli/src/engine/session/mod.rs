use std::path::PathBuf;

use anyhow::{Context, Result};

pub mod backend;
pub mod db_backend;
pub mod local_backend;
pub mod phase;
pub mod state;

pub use backend::SessionBackend;
pub use db_backend::DbSessionBackend;

pub fn list_legacy_local_session_ids(repo_root: impl Into<PathBuf>) -> Result<Vec<String>> {
    let backend = local_backend::LocalFileBackend::new(repo_root.into());
    let dir = backend.sessions_dir();

    let mut session_ids = Vec::new();
    match std::fs::read_dir(&dir) {
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(session_ids),
        Err(err) => {
            return Err(err).with_context(|| format!("reading sessions dir {}", dir.display()));
        }
        Ok(entries) => {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
                    continue;
                }
                let Some(stem) = path.file_stem().and_then(|stem| stem.to_str()) else {
                    continue;
                };
                if stem.is_empty() {
                    continue;
                }
                session_ids.push(stem.to_string());
            }
        }
    }

    session_ids.sort();
    session_ids.dedup();
    Ok(session_ids)
}

pub fn delete_legacy_local_session_state(
    repo_root: impl Into<PathBuf>,
    session_id: &str,
) -> Result<()> {
    let backend = local_backend::LocalFileBackend::new(repo_root.into());
    backend.delete_session(session_id)
}

pub fn create_session_backend(repo_root: impl Into<PathBuf>) -> Result<Box<dyn SessionBackend>> {
    let root = repo_root.into();
    let backend = DbSessionBackend::for_repo_root(&root)?;
    Ok(Box::new(backend))
}

#[cfg(test)]
pub fn create_session_backend_or_local(repo_root: impl Into<PathBuf>) -> Box<dyn SessionBackend> {
    Box::new(local_backend::LocalFileBackend::new(repo_root.into()))
}

#[cfg(not(test))]
pub fn create_session_backend_or_local(repo_root: impl Into<PathBuf>) -> Box<dyn SessionBackend> {
    let root = repo_root.into();
    match create_session_backend(root.clone()) {
        Ok(backend) => backend,
        Err(err) => {
            log::warn!(
                "failed to initialise DbSessionBackend; falling back to LocalFileBackend: {err:#}"
            );
            Box::new(local_backend::LocalFileBackend::new(root))
        }
    }
}
