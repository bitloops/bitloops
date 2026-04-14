use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};
use std::fs;
use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use xz2::read::XzDecoder;
use zip::ZipArchive;

use crate::utils::platform_dirs::{ensure_dir, ensure_parent_dir};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ManagedEmbeddingsArchiveKind {
    Zip,
    TarXz,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ManagedEmbeddingsBundleEntry {
    pub(crate) relative_path: PathBuf,
    pub(crate) bytes: Vec<u8>,
    pub(crate) executable: bool,
}

pub(crate) fn extract_managed_embeddings_bundle_entries(
    archive_bytes: &[u8],
    archive_kind: ManagedEmbeddingsArchiveKind,
    binary_name: &str,
) -> Result<Vec<ManagedEmbeddingsBundleEntry>> {
    let mut bundle_entries = Vec::new();
    match archive_kind {
        ManagedEmbeddingsArchiveKind::Zip => {
            let mut archive = ZipArchive::new(Cursor::new(archive_bytes))
                .context("opening managed bitloops-embeddings zip archive")?;
            for index in 0..archive.len() {
                let mut entry = archive
                    .by_index(index)
                    .with_context(|| format!("reading zip entry {index}"))?;
                if entry.is_dir() {
                    continue;
                }
                let relative_path = sanitise_archive_entry_path(Path::new(entry.name()))?;
                let executable = zip_entry_is_executable(&entry);
                let mut bytes = Vec::new();
                entry
                    .read_to_end(&mut bytes)
                    .context("reading managed bitloops-embeddings zip entry")?;
                bundle_entries.push(ManagedEmbeddingsBundleEntry {
                    relative_path,
                    bytes,
                    executable,
                });
            }
        }
        ManagedEmbeddingsArchiveKind::TarXz => {
            let decoder = XzDecoder::new(Cursor::new(archive_bytes));
            let mut archive = tar::Archive::new(decoder);
            for entry in archive
                .entries()
                .context("reading managed bitloops-embeddings tar archive entries")?
            {
                let mut entry = entry.context("reading managed bitloops-embeddings tar entry")?;
                if !entry.header().entry_type().is_file() {
                    continue;
                }
                let archive_path = entry
                    .path()
                    .context("reading managed bitloops-embeddings tar entry path")?
                    .into_owned();
                let relative_path = sanitise_archive_entry_path(&archive_path)?;
                let executable = tar_entry_is_executable(&entry);
                let mut bytes = Vec::new();
                entry
                    .read_to_end(&mut bytes)
                    .context("reading managed bitloops-embeddings tar entry")?;
                bundle_entries.push(ManagedEmbeddingsBundleEntry {
                    relative_path,
                    bytes,
                    executable,
                });
            }
        }
    }

    bundle_entries_for_binary(bundle_entries, binary_name)
}

pub(crate) fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

pub(crate) fn write_file_atomically(path: &Path, bytes: &[u8], executable: bool) -> Result<()> {
    ensure_parent_dir(path)?;
    if let Some(parent) = path.parent() {
        ensure_dir(parent)?;
    }

    let temp_path = unique_temp_path(path);
    fs::write(&temp_path, bytes)
        .with_context(|| format!("writing temporary file {}", temp_path.display()))?;
    if executable {
        set_executable_permissions(&temp_path)?;
    }
    replace_file_atomically(&temp_path, path)?;
    Ok(())
}

fn archive_entry_matches_binary(path: &Path, binary_name: &str) -> bool {
    path.file_name().and_then(|value| value.to_str()) == Some(binary_name)
}

fn bundle_entries_for_binary(
    entries: Vec<ManagedEmbeddingsBundleEntry>,
    binary_name: &str,
) -> Result<Vec<ManagedEmbeddingsBundleEntry>> {
    let Some(binary_entry) = entries
        .iter()
        .find(|entry| archive_entry_matches_binary(&entry.relative_path, binary_name))
    else {
        bail!("managed bitloops-embeddings archive did not contain `{binary_name}`");
    };

    let bundle_root = binary_entry
        .relative_path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_default();
    let mut bundle_entries = Vec::new();
    for entry in entries {
        if !bundle_root.as_os_str().is_empty() && !entry.relative_path.starts_with(&bundle_root) {
            continue;
        }
        let relative_path = if bundle_root.as_os_str().is_empty() {
            entry.relative_path
        } else {
            entry
                .relative_path
                .strip_prefix(&bundle_root)
                .context("stripping managed embeddings bundle root")?
                .to_path_buf()
        };
        let is_binary = archive_entry_matches_binary(&relative_path, binary_name);
        bundle_entries.push(ManagedEmbeddingsBundleEntry {
            relative_path,
            bytes: entry.bytes,
            executable: entry.executable || is_binary,
        });
    }

    Ok(bundle_entries)
}

fn sanitise_archive_entry_path(path: &Path) -> Result<PathBuf> {
    let mut relative = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::Normal(value) => relative.push(value),
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir
            | std::path::Component::RootDir
            | std::path::Component::Prefix(_) => {
                bail!(
                    "managed bitloops-embeddings archive contained unsafe path `{}`",
                    path.display()
                );
            }
        }
    }
    if relative.as_os_str().is_empty() {
        bail!("managed bitloops-embeddings archive contained an empty path entry");
    }
    Ok(relative)
}

fn zip_entry_is_executable(entry: &zip::read::ZipFile<'_>) -> bool {
    entry.unix_mode().is_some_and(|mode| mode & 0o111 != 0)
}

fn tar_entry_is_executable<R: Read>(entry: &tar::Entry<'_, R>) -> bool {
    entry.header().mode().is_ok_and(|mode| mode & 0o111 != 0)
}

fn unique_temp_path(path: &Path) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    let file_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("bitloops-temp");
    path.with_file_name(format!(".{file_name}.tmp.{}.{}", std::process::id(), nanos))
}

fn replace_file_atomically(temp_path: &Path, final_path: &Path) -> Result<()> {
    #[cfg(windows)]
    {
        if !final_path.exists() {
            return fs::rename(temp_path, final_path).with_context(|| {
                format!(
                    "moving temporary file {} into {}",
                    temp_path.display(),
                    final_path.display()
                )
            });
        }

        let backup_path = unique_temp_path(final_path);
        fs::rename(final_path, &backup_path).with_context(|| {
            format!(
                "moving existing file {} aside before replacement",
                final_path.display()
            )
        })?;
        match fs::rename(temp_path, final_path) {
            Ok(_) => {
                let _ = fs::remove_file(&backup_path);
                return Ok(());
            }
            Err(err) => {
                let _ = fs::rename(&backup_path, final_path);
                let _ = fs::remove_file(temp_path);
                return Err(err).with_context(|| {
                    format!(
                        "moving temporary file {} into {}",
                        temp_path.display(),
                        final_path.display()
                    )
                });
            }
        }
    }

    #[cfg(not(windows))]
    {
        fs::rename(temp_path, final_path).with_context(|| {
            format!(
                "moving temporary file {} into {}",
                temp_path.display(),
                final_path.display()
            )
        })
    }
}

#[cfg(unix)]
fn set_executable_permissions(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;

    let mut permissions = fs::metadata(path)
        .with_context(|| format!("reading metadata for {}", path.display()))?
        .permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions)
        .with_context(|| format!("setting executable permissions on {}", path.display()))
}

#[cfg(not(unix))]
fn set_executable_permissions(_path: &Path) -> Result<()> {
    Ok(())
}
