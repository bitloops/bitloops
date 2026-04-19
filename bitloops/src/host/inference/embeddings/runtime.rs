use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use sha2::{Digest, Sha256};

pub(crate) const BITLOOPS_EMBEDDINGS_CACHE_DIR_ENV: &str = "BITLOOPS_EMBEDDINGS_CACHE_DIR";
pub(crate) const HF_HUB_OFFLINE_ENV: &str = "HF_HUB_OFFLINE";
pub(crate) const TRANSFORMERS_OFFLINE_ENV: &str = "TRANSFORMERS_OFFLINE";
pub(crate) const SHARED_EMBEDDINGS_REQUEST_TIMEOUT_GUARD_SECS: u64 = 5;

pub(crate) fn process_environment_fingerprint() -> String {
    let mut vars = std::env::vars_os()
        .map(|(key, value)| format!("{}={}", key.to_string_lossy(), value.to_string_lossy()))
        .collect::<Vec<_>>();
    vars.sort();
    sha256_hex(vars.join("\n").as_bytes())
}

pub(crate) fn embeddings_runtime_launch_artifact_fingerprint(
    command: &str,
    args: &[String],
) -> String {
    let command_path = Path::new(command);
    let mut candidates = vec![command];
    if runtime_command_uses_script_argument(command_path)
        && let Some(script_path) = args.first()
    {
        candidates.push(script_path.as_str());
    }

    let mut artefacts = candidates
        .into_iter()
        .filter_map(|candidate| {
            let path = Path::new(candidate);
            if !path.is_file() {
                return None;
            }
            let canonical = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
            let metadata = std::fs::metadata(&canonical).ok()?;
            let modified = metadata
                .modified()
                .ok()
                .and_then(|timestamp| timestamp.duration_since(UNIX_EPOCH).ok())
                .map(|duration| format!("{}:{}", duration.as_secs(), duration.subsec_nanos()))
                .unwrap_or_else(|| "unknown".to_string());
            Some(format!(
                "{}|{}|{}",
                canonical.display(),
                metadata.len(),
                modified
            ))
        })
        .collect::<Vec<_>>();
    artefacts.sort();
    sha256_hex(artefacts.join("\n").as_bytes())
}

fn runtime_command_uses_script_argument(command: &Path) -> bool {
    command
        .file_name()
        .and_then(|value| value.to_str())
        .map(|value| {
            matches!(
                value.to_ascii_lowercase().as_str(),
                "sh" | "bash"
                    | "zsh"
                    | "python"
                    | "python3"
                    | "python3.11"
                    | "python3.12"
                    | "python3.13"
                    | "node"
                    | "ruby"
                    | "perl"
                    | "pwsh"
                    | "powershell"
            )
        })
        .unwrap_or(false)
}

fn sha256_hex(data: &[u8]) -> String {
    let digest = Sha256::digest(data);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

pub(crate) fn embeddings_runtime_request_timeout_secs(configured_timeout_secs: u64) -> u64 {
    if configured_timeout_secs > SHARED_EMBEDDINGS_REQUEST_TIMEOUT_GUARD_SECS {
        configured_timeout_secs - SHARED_EMBEDDINGS_REQUEST_TIMEOUT_GUARD_SECS
    } else {
        configured_timeout_secs
    }
}

pub(crate) fn embeddings_runtime_error_is_timeout(err: &anyhow::Error) -> bool {
    format!("{err:#}").contains("timed out after")
}

pub(crate) fn resolve_effective_embeddings_cache_dir(
    explicit_cache_dir: Option<&Path>,
) -> Option<PathBuf> {
    explicit_cache_dir
        .map(Path::to_path_buf)
        .or_else(|| std::env::var_os(BITLOOPS_EMBEDDINGS_CACHE_DIR_ENV).map(PathBuf::from))
        .or_else(|| {
            dirs::cache_dir()
                .or_else(|| dirs::home_dir().map(|home| home.join(".cache")))
                .map(|dir| dir.join("bitloops-embeddings"))
        })
}

pub(crate) fn cache_contains_requested_model(cache_dir: &Path, model: &str) -> bool {
    let Ok(entries) = std::fs::read_dir(cache_dir) else {
        return false;
    };
    let normalized_model = model.replace('/', "--");
    let exact = format!("models--{normalized_model}");
    let suffix = format!("--{normalized_model}");
    entries
        .flatten()
        .filter_map(|entry| {
            entry
                .file_type()
                .ok()
                .filter(|file_type| file_type.is_dir())
                .map(|_| entry)
        })
        .any(|entry| {
            let file_name = entry.file_name();
            let Some(file_name) = file_name.to_str() else {
                return false;
            };
            if !file_name.starts_with("models--") {
                return false;
            }
            if file_name != exact && !file_name.ends_with(&suffix) {
                return false;
            }
            let model_dir = entry.path();
            model_dir.join("refs").exists()
                || model_dir.join("snapshots").exists()
                || model_dir.join("blobs").exists()
        })
}
