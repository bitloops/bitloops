use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::error::Error as StdError;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use toml_edit::{Array, DocumentMut, Item, Value as TomlValue};

use crate::cli::embeddings::managed::archive::write_file_atomically;
use crate::host::inference::BITLOOPS_INFERENCE_RUNTIME_ID;
use crate::utils::platform_dirs::{bitloops_data_dir, ensure_dir};

const MANAGED_INFERENCE_INSTALL_PARENT_DIR: &str = "tools";
const MANAGED_INFERENCE_INSTALL_DIR_NAME: &str = "bitloops-inference";
const MANAGED_INFERENCE_METADATA_FILE_NAME: &str = "bitloops-inference-install.json";

pub(crate) const MANAGED_INFERENCE_VERSION_OVERRIDE_ENV: &str =
    "BITLOOPS_INFERENCE_VERSION_OVERRIDE";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct ManagedInferenceInstallMetadata {
    pub(crate) version: String,
    pub(crate) binary_path: PathBuf,
}

#[derive(Debug)]
pub(crate) enum ManagedInferenceMetadataError {
    ResolvePath {
        source: anyhow::Error,
    },
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
    Parse {
        path: PathBuf,
        source: serde_json::Error,
    },
}

impl fmt::Display for ManagedInferenceMetadataError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ResolvePath { source } => {
                write!(f, "resolving managed inference metadata path: {}", source)
            }
            Self::Read { path, source } => write!(
                f,
                "reading managed inference metadata {}: {}",
                path.display(),
                source
            ),
            Self::Parse { path, source } => write!(
                f,
                "parsing managed inference metadata {}: {}",
                path.display(),
                source
            ),
        }
    }
}

impl StdError for ManagedInferenceMetadataError {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        match self {
            Self::ResolvePath { source } => Some(source.root_cause()),
            Self::Read { source, .. } => Some(source),
            Self::Parse { source, .. } => Some(source),
        }
    }
}

pub(crate) fn managed_inference_binary_name() -> &'static str {
    if cfg!(windows) {
        "bitloops-inference.exe"
    } else {
        "bitloops-inference"
    }
}

pub(crate) fn managed_inference_binary_dir() -> Result<PathBuf> {
    Ok(bitloops_data_dir()?
        .join(MANAGED_INFERENCE_INSTALL_PARENT_DIR)
        .join(MANAGED_INFERENCE_INSTALL_DIR_NAME))
}

pub(crate) fn managed_inference_binary_path() -> Result<PathBuf> {
    Ok(managed_inference_binary_dir()?.join(managed_inference_binary_name()))
}

pub(crate) fn managed_inference_metadata_path() -> Result<PathBuf> {
    Ok(bitloops_data_dir()?.join(MANAGED_INFERENCE_METADATA_FILE_NAME))
}

pub(crate) fn load_managed_inference_install_metadata()
-> std::result::Result<Option<ManagedInferenceInstallMetadata>, ManagedInferenceMetadataError> {
    let path = managed_inference_metadata_path()
        .map_err(|source| ManagedInferenceMetadataError::ResolvePath { source })?;
    let contents = match fs::read_to_string(&path) {
        Ok(contents) => contents,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(ManagedInferenceMetadataError::Read { path, source: err }),
    };

    serde_json::from_str(&contents)
        .map(Some)
        .map_err(|err| ManagedInferenceMetadataError::Parse { path, source: err })
}

pub(crate) fn save_managed_inference_install_metadata(
    metadata: &ManagedInferenceInstallMetadata,
) -> Result<()> {
    let path = managed_inference_metadata_path()?;
    let bytes =
        serde_json::to_vec_pretty(metadata).context("serialising managed inference metadata")?;
    write_file_atomically(&path, &bytes, false)
}

#[allow(dead_code)]
pub(crate) fn managed_runtime_version_for_command(command: &str) -> Result<Option<String>> {
    let command = Path::new(command.trim());
    if !command.is_absolute() {
        return Ok(None);
    }

    if !command.starts_with(managed_inference_binary_dir()?) {
        return Ok(None);
    }

    let Some(metadata) = load_managed_inference_install_metadata()? else {
        return Ok(None);
    };
    Ok((command == metadata.binary_path).then_some(metadata.version))
}

pub(crate) fn managed_inference_bundle_is_complete(binary_path: &Path) -> bool {
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
    if command == "bitloops-inference" || command == managed_inference_binary_name() {
        return Ok(true);
    }

    let candidate = Path::new(command);
    Ok(candidate.is_absolute() && candidate.starts_with(managed_inference_binary_dir()?))
}

pub(crate) fn raw_managed_runtime_command(config_path: &Path) -> Result<Option<String>> {
    let contents = match fs::read_to_string(config_path) {
        Ok(contents) => contents,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => {
            return Err(err).with_context(|| {
                format!(
                    "reading Bitloops daemon config for managed inference runtime {}",
                    config_path.display()
                )
            });
        }
    };
    let doc = contents.parse::<DocumentMut>().with_context(|| {
        format!(
            "parsing Bitloops daemon config for managed inference runtime {}",
            config_path.display()
        )
    })?;

    Ok(doc
        .as_table()
        .get("inference")
        .and_then(Item::as_table)
        .and_then(|table| table.get("runtimes"))
        .and_then(Item::as_table)
        .and_then(|table| table.get(BITLOOPS_INFERENCE_RUNTIME_ID))
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
                    "reading Bitloops daemon config for managed inference runtime {}",
                    config_path.display()
                )
            });
        }
    };
    let mut doc = contents.parse::<DocumentMut>().with_context(|| {
        format!(
            "parsing Bitloops daemon config for managed inference runtime {}",
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
        .get_mut(BITLOOPS_INFERENCE_RUNTIME_ID)
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
            "writing Bitloops daemon config for managed inference runtime {}",
            config_path.display()
        )
    })?;
    Ok(true)
}

pub(crate) fn reset_managed_inference_install_dir() -> Result<()> {
    let install_dir = managed_inference_binary_dir()?;
    if install_dir.exists() {
        fs::remove_dir_all(&install_dir).with_context(|| {
            format!(
                "removing existing managed inference install directory {}",
                install_dir.display()
            )
        })?;
    }
    ensure_dir(&install_dir)?;
    Ok(())
}
