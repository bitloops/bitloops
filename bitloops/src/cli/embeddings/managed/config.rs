use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use toml_edit::{Array, DocumentMut, Item, Value as TomlValue};

use crate::host::inference::BITLOOPS_EMBEDDINGS_RUNTIME_ID;
use crate::utils::platform_dirs::{bitloops_data_dir, ensure_dir};

use super::archive::write_file_atomically;

pub(crate) const DEFAULT_MANAGED_EMBEDDINGS_VERSION: &str = "v0.1.0";
const MANAGED_EMBEDDINGS_INSTALL_PARENT_DIR: &str = "tools";
const MANAGED_EMBEDDINGS_INSTALL_DIR_NAME: &str = "bitloops-embeddings";
const MANAGED_EMBEDDINGS_METADATA_FILE_NAME: &str = "bitloops-embeddings-install.json";

pub(crate) const MANAGED_EMBEDDINGS_VERSION_OVERRIDE_ENV: &str =
    "BITLOOPS_EMBEDDINGS_VERSION_OVERRIDE";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct ManagedEmbeddingsInstallMetadata {
    pub(crate) version: String,
    pub(crate) binary_path: PathBuf,
}

pub(crate) fn managed_embeddings_binary_name() -> &'static str {
    if cfg!(windows) {
        "bitloops-embeddings.exe"
    } else {
        "bitloops-embeddings"
    }
}

pub(crate) fn managed_embeddings_binary_dir() -> Result<PathBuf> {
    Ok(bitloops_data_dir()?
        .join(MANAGED_EMBEDDINGS_INSTALL_PARENT_DIR)
        .join(MANAGED_EMBEDDINGS_INSTALL_DIR_NAME))
}

pub(crate) fn managed_embeddings_binary_path() -> Result<PathBuf> {
    Ok(managed_embeddings_binary_dir()?.join(managed_embeddings_binary_name()))
}

pub(crate) fn managed_embeddings_metadata_path() -> Result<PathBuf> {
    Ok(bitloops_data_dir()?.join(MANAGED_EMBEDDINGS_METADATA_FILE_NAME))
}

pub(crate) fn load_managed_embeddings_install_metadata()
-> Result<Option<ManagedEmbeddingsInstallMetadata>> {
    let path = managed_embeddings_metadata_path()?;
    let contents = match fs::read_to_string(&path) {
        Ok(contents) => contents,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => {
            return Err(err).with_context(|| {
                format!("reading managed embeddings metadata {}", path.display())
            });
        }
    };

    Ok(serde_json::from_str(&contents).ok())
}

pub(crate) fn save_managed_embeddings_install_metadata(
    metadata: &ManagedEmbeddingsInstallMetadata,
) -> Result<()> {
    let path = managed_embeddings_metadata_path()?;
    let bytes =
        serde_json::to_vec_pretty(metadata).context("serialising managed embeddings metadata")?;
    write_file_atomically(&path, &bytes, false)
}

pub(crate) fn managed_runtime_version_for_command(command: &str) -> Option<String> {
    let metadata = load_managed_embeddings_install_metadata().ok().flatten()?;
    let command = Path::new(command.trim());
    (command == metadata.binary_path).then_some(metadata.version)
}

pub(crate) fn managed_embeddings_bundle_is_complete(binary_path: &Path) -> bool {
    binary_path.is_file()
        && binary_path
            .parent()
            .is_some_and(|parent| parent.join("_internal").is_dir())
}

pub(crate) fn managed_runtime_command_is_eligible(config_path: &Path) -> Result<bool> {
    let Some(command) = raw_managed_runtime_command(config_path)? else {
        return Ok(true);
    };

    let command = command.trim();
    if command.is_empty() {
        return Ok(true);
    }
    if command == "bitloops-embeddings" || command == managed_embeddings_binary_name() {
        return Ok(true);
    }

    let candidate = Path::new(command);
    Ok(candidate.is_absolute() && candidate.starts_with(managed_embeddings_binary_dir()?))
}

pub(crate) fn raw_managed_runtime_command(config_path: &Path) -> Result<Option<String>> {
    let contents = match fs::read_to_string(config_path) {
        Ok(contents) => contents,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => {
            return Err(err).with_context(|| {
                format!(
                    "reading Bitloops daemon config for managed embeddings runtime {}",
                    config_path.display()
                )
            });
        }
    };
    let doc = contents.parse::<DocumentMut>().with_context(|| {
        format!(
            "parsing Bitloops daemon config for managed embeddings runtime {}",
            config_path.display()
        )
    })?;

    Ok(doc
        .as_table()
        .get("inference")
        .and_then(Item::as_table)
        .and_then(|table| table.get("runtimes"))
        .and_then(Item::as_table)
        .and_then(|table| table.get(BITLOOPS_EMBEDDINGS_RUNTIME_ID))
        .and_then(Item::as_table)
        .and_then(|table| table.get("command"))
        .and_then(Item::as_value)
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned))
}

pub(crate) fn rewrite_managed_runtime_command_if_eligible(
    config_path: &Path,
    binary_path: &Path,
) -> Result<bool> {
    let contents = match fs::read_to_string(config_path) {
        Ok(contents) => contents,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(err) => {
            return Err(err).with_context(|| {
                format!(
                    "reading Bitloops daemon config for managed embeddings runtime {}",
                    config_path.display()
                )
            });
        }
    };
    let mut doc = contents.parse::<DocumentMut>().with_context(|| {
        format!(
            "parsing Bitloops daemon config for managed embeddings runtime {}",
            config_path.display()
        )
    })?;

    let Some(inference) = doc.get_mut("inference").and_then(Item::as_table_mut) else {
        return Ok(false);
    };
    let Some(runtimes) = inference.get_mut("runtimes").and_then(Item::as_table_mut) else {
        return Ok(false);
    };
    let Some(runtime) = runtimes
        .get_mut(BITLOOPS_EMBEDDINGS_RUNTIME_ID)
        .and_then(Item::as_table_mut)
    else {
        return Ok(false);
    };

    let current_command = runtime
        .get("command")
        .and_then(Item::as_value)
        .and_then(|value| value.as_str())
        .map(str::trim);
    let current_args_are_empty = runtime
        .get("args")
        .and_then(Item::as_value)
        .and_then(|value| value.as_array())
        .is_some_and(|value| value.is_empty());
    if !managed_runtime_command_is_eligible(config_path)? {
        return Ok(false);
    }

    let desired = binary_path.to_string_lossy().to_string();
    if current_command == Some(desired.as_str()) && current_args_are_empty {
        return Ok(false);
    }

    runtime["command"] = Item::Value(desired.into());
    runtime["args"] = Item::Value(TomlValue::Array(Array::new()));
    fs::write(config_path, doc.to_string()).with_context(|| {
        format!(
            "writing Bitloops daemon config for managed embeddings runtime {}",
            config_path.display()
        )
    })?;
    Ok(true)
}

pub(crate) fn reset_managed_embeddings_install_dir() -> Result<()> {
    let install_dir = managed_embeddings_binary_dir()?;
    if install_dir.exists() {
        fs::remove_dir_all(&install_dir).with_context(|| {
            format!(
                "removing existing managed embeddings install directory {}",
                install_dir.display()
            )
        })?;
    }
    ensure_dir(&install_dir)?;

    Ok(())
}
