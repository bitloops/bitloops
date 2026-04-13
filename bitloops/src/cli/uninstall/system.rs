use std::collections::BTreeSet;
use std::env;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use super::{BinaryCandidatesFn, ServiceUninstaller};
use crate::cli::embeddings::{
    managed_embeddings_binary_dir, managed_embeddings_binary_path, managed_embeddings_metadata_path,
};
use crate::config::settings::SETTINGS_DIR;
use crate::utils::platform_dirs::{
    bitloops_cache_dir, bitloops_config_dir, bitloops_data_dir, bitloops_home_dir,
    bitloops_state_dir,
};

pub(super) fn uninstall_data(repo_roots: &[PathBuf], out: &mut dyn Write) -> Result<()> {
    let mut removed_any = false;

    let data_dir = bitloops_data_dir()?;
    if remove_dir_if_exists(&data_dir)? {
        writeln!(out, "  Removed data directory {}", data_dir.display())?;
        removed_any = true;
    }

    for repo_root in repo_roots {
        let bitloops_dir = repo_root.join(SETTINGS_DIR);
        if remove_dir_if_exists(&bitloops_dir)? {
            writeln!(out, "  Removed repo data {}", bitloops_dir.display())?;
            removed_any = true;
        }
    }

    if !removed_any {
        writeln!(out, "  No Bitloops data found.")?;
    }

    Ok(())
}

pub(super) fn uninstall_cache(out: &mut dyn Write) -> Result<()> {
    let cache_dir = bitloops_cache_dir()?;
    if remove_dir_if_exists(&cache_dir)? {
        writeln!(out, "  Removed cache directory {}", cache_dir.display())?;
    } else {
        writeln!(out, "  No Bitloops cache found.")?;
    }

    Ok(())
}

pub(super) fn uninstall_config(out: &mut dyn Write) -> Result<()> {
    let mut removed_any = false;

    let config_dir = bitloops_config_dir()?;
    if remove_dir_if_exists(&config_dir)? {
        writeln!(out, "  Removed config directory {}", config_dir.display())?;
        removed_any = true;
    }

    let tls_certs_dir = bitloops_home_dir()?.join(".bitloops").join("certs");
    if remove_dir_if_exists(&tls_certs_dir)? {
        writeln!(out, "  Removed TLS artefacts {}", tls_certs_dir.display())?;
        removed_any = true;
    }

    if !removed_any {
        writeln!(out, "  No Bitloops config found.")?;
    }

    Ok(())
}

pub(super) fn uninstall_service(
    out: &mut dyn Write,
    service_uninstaller: &ServiceUninstaller,
) -> Result<()> {
    service_uninstaller()?;

    let state_dir = bitloops_state_dir()?;
    if remove_dir_if_exists(&state_dir)? {
        writeln!(out, "  Removed service state {}", state_dir.display())?;
    } else {
        writeln!(out, "  No Bitloops service state found.")?;
    }

    Ok(())
}

pub(super) fn uninstall_binaries(
    out: &mut dyn Write,
    binary_candidates: &BinaryCandidatesFn,
) -> Result<()> {
    let candidates = binary_candidates()?;
    let mut removed = 0usize;

    for candidate in candidates {
        if !candidate.exists() {
            continue;
        }

        fs::remove_file(&candidate)
            .with_context(|| format!("removing binary {}", candidate.display()))?;
        writeln!(out, "  Removed binary {}", candidate.display())?;
        removed += 1;
    }

    if removed == 0 {
        writeln!(out, "  No recognised Bitloops binaries found.")?;
    }

    let managed_bundle_dir = managed_embeddings_binary_dir()?;
    if managed_bundle_dir.exists() {
        fs::remove_dir_all(&managed_bundle_dir).with_context(|| {
            format!(
                "removing managed embeddings install directory {}",
                managed_bundle_dir.display()
            )
        })?;
        writeln!(
            out,
            "  Removed managed embeddings install directory {}",
            managed_bundle_dir.display()
        )?;
    }

    let metadata_path = managed_embeddings_metadata_path()?;
    if metadata_path.exists() {
        fs::remove_file(&metadata_path).with_context(|| {
            format!(
                "removing managed embeddings metadata {}",
                metadata_path.display()
            )
        })?;
        writeln!(
            out,
            "  Removed managed embeddings metadata {}",
            metadata_path.display()
        )?;
    }

    Ok(())
}

pub(super) fn default_service_uninstaller() -> Result<()> {
    crate::daemon::uninstall_supervisor_service()
}

pub(super) fn known_binary_candidates() -> Result<Vec<PathBuf>> {
    let binary_name = if cfg!(windows) {
        "bitloops.exe"
    } else {
        "bitloops"
    };

    let mut candidates = BTreeSet::new();

    if let Ok(current_exe) = env::current_exe()
        && current_exe
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name == binary_name)
    {
        candidates.insert(current_exe);
    }

    if let Ok(install_dir) = env::var("BITLOOPS_INSTALL_DIR") {
        candidates.insert(PathBuf::from(install_dir).join(binary_name));
    }

    if let Some(home) = dirs::home_dir() {
        candidates.insert(home.join(".local").join("bin").join(binary_name));
        candidates.insert(home.join(".cargo").join("bin").join(binary_name));
        candidates.insert(home.join(".bitloops").join("bin").join(binary_name));
    }
    if let Ok(managed_binary) = managed_embeddings_binary_path() {
        candidates.insert(managed_binary);
    }

    candidates.insert(PathBuf::from("/usr/local/bin").join(binary_name));
    if cfg!(target_os = "macos") {
        candidates.insert(PathBuf::from("/opt/homebrew/bin").join(binary_name));
    }

    Ok(candidates.into_iter().collect())
}

fn remove_dir_if_exists(path: &Path) -> Result<bool> {
    if !path.exists() {
        return Ok(false);
    }

    fs::remove_dir_all(path).with_context(|| format!("removing {}", path.display()))?;
    Ok(true)
}
