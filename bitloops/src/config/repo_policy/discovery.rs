use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde_json::{Map, Value};
use toml_edit::de::from_str;

use super::fingerprint::fingerprint_repo_policy;
use super::merge::{
    merge_optional_values, merge_scope_values, normalize_scope_exclusion_array_literals,
};
use super::scope::resolve_repo_policy_scope_exclusions;
use super::types::{
    ImportedKnowledgeConfig, REPO_POLICY_FILE_NAME, REPO_POLICY_LOCAL_FILE_NAME,
    RepoPolicyFingerprintInputs, RepoPolicyLocation, RepoPolicyScopeExclusions, RepoPolicySnapshot,
    RepoPolicyTomlFile,
};

pub fn discover_repo_policy(start: &Path) -> Result<RepoPolicySnapshot> {
    discover_repo_policy_with_mode(start, true)
}

pub fn discover_repo_policy_optional(start: &Path) -> Result<RepoPolicySnapshot> {
    discover_repo_policy_with_mode(start, false)
}

fn discover_repo_policy_with_mode(start: &Path, strict: bool) -> Result<RepoPolicySnapshot> {
    let start = if start.is_dir() {
        start.to_path_buf()
    } else {
        start.parent().unwrap_or(start).to_path_buf()
    };
    let start = start.canonicalize().unwrap_or(start);
    let git_root = find_git_root(&start);
    let Some(location) = find_policy_location(&start, git_root.as_deref()) else {
        if strict {
            return missing_repo_policy_error(git_root.as_deref(), &start);
        }
        return Ok(default_repo_policy_snapshot());
    };

    let shared = location
        .shared_path
        .as_deref()
        .map(load_policy_file)
        .transpose()?;
    let local = location
        .local_path
        .as_deref()
        .map(load_policy_file)
        .transpose()?;

    if shared
        .as_ref()
        .and_then(|value| value.daemon.config_path.as_deref())
        .is_some_and(|value| !value.trim().is_empty())
    {
        let shared_path = location
            .shared_path
            .as_deref()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| REPO_POLICY_FILE_NAME.to_string());
        anyhow::bail!(
            "Bitloops daemon binding must be local-only. Move `[daemon].config_path` from {} into `{}`.",
            shared_path,
            REPO_POLICY_LOCAL_FILE_NAME
        );
    }

    let daemon_config_path = local
        .as_ref()
        .and_then(|value| value.daemon.config_path.as_deref())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from);

    let capture = merge_optional_values(
        shared.as_ref().and_then(|value| value.capture.clone()),
        local.as_ref().and_then(|value| value.capture.clone()),
    );
    let watch = merge_optional_values(
        shared.as_ref().and_then(|value| value.watch.clone()),
        local.as_ref().and_then(|value| value.watch.clone()),
    );
    let scope = merge_scope_values(
        shared.as_ref().and_then(|value| value.scope.clone()),
        local.as_ref().and_then(|value| value.scope.clone()),
    );
    let contexts = merge_optional_values(
        shared.as_ref().and_then(|value| value.contexts.clone()),
        local.as_ref().and_then(|value| value.contexts.clone()),
    );
    let agents = merge_optional_values(
        shared.as_ref().and_then(|value| value.agents.clone()),
        local.as_ref().and_then(|value| value.agents.clone()),
    );
    let knowledge_import_paths = if let Some(local) = &local {
        if !local.imports.knowledge.is_empty() {
            local.imports.knowledge.clone()
        } else {
            shared
                .as_ref()
                .map(|value| value.imports.knowledge.clone())
                .unwrap_or_default()
        }
    } else {
        shared
            .as_ref()
            .map(|value| value.imports.knowledge.clone())
            .unwrap_or_default()
    };

    let imported_knowledge = knowledge_import_paths
        .iter()
        .map(|import_path| load_imported_knowledge(&location.root, import_path))
        .collect::<Result<Vec<_>>>()?;
    let scope_exclusions = resolve_repo_policy_scope_exclusions(&scope, &location.root)?;

    let fingerprint = fingerprint_repo_policy(RepoPolicyFingerprintInputs {
        capture: &capture,
        watch: &watch,
        scope: &scope,
        scope_exclusions: &scope_exclusions,
        contexts: &contexts,
        agents: &agents,
        knowledge_import_paths: &knowledge_import_paths,
        imported_knowledge: &imported_knowledge,
    })?;

    Ok(RepoPolicySnapshot {
        root: Some(location.root),
        shared_path: location.shared_path,
        local_path: location.local_path,
        daemon_config_path,
        capture,
        watch,
        scope,
        contexts,
        agents,
        knowledge_import_paths,
        imported_knowledge,
        fingerprint,
    })
}

fn default_repo_policy_snapshot() -> RepoPolicySnapshot {
    let capture = Value::Object(Map::new());
    let watch = Value::Object(Map::new());
    let scope = Value::Object(Map::new());
    let contexts = Value::Array(Vec::new());
    let agents = Value::Object(Map::new());
    let imported_knowledge = Vec::new();
    let knowledge_import_paths = Vec::new();
    let exclusions = RepoPolicyScopeExclusions {
        exclude: Vec::new(),
        exclude_from: Vec::new(),
        referenced_files: Vec::new(),
    };
    let fingerprint = fingerprint_repo_policy(RepoPolicyFingerprintInputs {
        capture: &capture,
        watch: &watch,
        scope: &scope,
        scope_exclusions: &exclusions,
        contexts: &contexts,
        agents: &agents,
        knowledge_import_paths: &knowledge_import_paths,
        imported_knowledge: &imported_knowledge,
    })
    .unwrap_or_else(|_| "default".to_string());

    RepoPolicySnapshot {
        root: None,
        shared_path: None,
        local_path: None,
        daemon_config_path: None,
        capture,
        watch,
        scope,
        contexts,
        agents,
        knowledge_import_paths,
        imported_knowledge,
        fingerprint,
    }
}

fn find_git_root(start: &Path) -> Option<PathBuf> {
    for directory in start.ancestors() {
        if directory.join(".git").exists() {
            return Some(directory.to_path_buf());
        }
    }
    None
}

fn find_policy_location(start: &Path, git_root: Option<&Path>) -> Option<RepoPolicyLocation> {
    for directory in start.ancestors() {
        let local_path = directory.join(REPO_POLICY_LOCAL_FILE_NAME);
        let shared_path = directory.join(REPO_POLICY_FILE_NAME);

        if local_path.is_file() {
            return Some(RepoPolicyLocation {
                root: directory.to_path_buf(),
                shared_path: shared_path.is_file().then_some(shared_path),
                local_path: Some(local_path),
            });
        }

        if shared_path.is_file() {
            return Some(RepoPolicyLocation {
                root: directory.to_path_buf(),
                shared_path: Some(shared_path),
                local_path: None,
            });
        }

        if Some(directory) == git_root {
            break;
        }
    }

    None
}

fn missing_repo_policy_error(git_root: Option<&Path>, start: &Path) -> Result<RepoPolicySnapshot> {
    if let Some(git_root) = git_root {
        anyhow::bail!(
            "Bitloops project config not found from {} up to git root {}. Run `bitloops init` in this directory or a parent project directory.",
            start.display(),
            git_root.display()
        );
    }

    anyhow::bail!(
        "No git repository found above {}. Run Bitloops inside a git repository and use `bitloops init`.",
        start.display()
    )
}

fn load_policy_file(path: &Path) -> Result<RepoPolicyTomlFile> {
    let raw = fs::read_to_string(path)
        .with_context(|| format!("reading Bitloops repo policy {}", path.display()))?;
    parse_policy_text(&raw, path)
}

fn load_imported_knowledge(root: &Path, import_path: &str) -> Result<ImportedKnowledgeConfig> {
    let resolved_path = resolve_import_path(root, import_path);
    let raw = fs::read_to_string(&resolved_path).with_context(|| {
        format!(
            "reading imported knowledge config {}",
            resolved_path.display()
        )
    })?;
    let value = from_str::<Value>(&raw).with_context(|| {
        format!(
            "parsing imported knowledge config {}",
            resolved_path.display()
        )
    })?;
    Ok(ImportedKnowledgeConfig {
        path: resolved_path.canonicalize().unwrap_or(resolved_path),
        value,
    })
}

pub(super) fn resolve_import_path(root: &Path, import_path: &str) -> PathBuf {
    let candidate = PathBuf::from(import_path);
    if candidate.is_absolute() {
        candidate
    } else {
        root.join(candidate)
    }
}

fn parse_policy_text(raw: &str, path: &Path) -> Result<RepoPolicyTomlFile> {
    match from_str::<RepoPolicyTomlFile>(raw) {
        Ok(parsed) => Ok(parsed),
        Err(primary_err) => {
            if let Some(normalized) = normalize_scope_exclusion_array_literals(raw)
                && let Ok(parsed) = from_str::<RepoPolicyTomlFile>(&normalized)
            {
                return Ok(parsed);
            }
            Err(primary_err)
                .with_context(|| format!("parsing Bitloops repo policy {}", path.display()))
        }
    }
}
