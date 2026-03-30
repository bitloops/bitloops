use anyhow::{Context, Result};
use serde::Deserialize;
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};
use toml_edit::de::from_str;

pub const REPO_POLICY_FILE_NAME: &str = ".bitloops.toml";
pub const REPO_POLICY_LOCAL_FILE_NAME: &str = ".bitloops.local.toml";

#[derive(Debug, Clone)]
pub struct RepoPolicySnapshot {
    pub root: Option<PathBuf>,
    pub shared_path: Option<PathBuf>,
    pub local_path: Option<PathBuf>,
    pub capture: Value,
    pub watch: Value,
    pub scope: Value,
    pub agents: Value,
    pub knowledge_import_paths: Vec<String>,
    pub imported_knowledge: Vec<ImportedKnowledgeConfig>,
    pub fingerprint: String,
}

#[derive(Debug, Clone)]
pub struct ImportedKnowledgeConfig {
    pub path: PathBuf,
    pub value: Value,
}

#[derive(Debug, Clone)]
struct RepoPolicyLocation {
    root: PathBuf,
    shared_path: Option<PathBuf>,
    local_path: Option<PathBuf>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct RepoPolicyTomlFile {
    #[serde(default)]
    capture: Option<Value>,
    #[serde(default)]
    watch: Option<Value>,
    #[serde(default)]
    scope: Option<Value>,
    #[serde(default)]
    agents: Option<Value>,
    #[serde(default)]
    imports: RepoPolicyImports,
}

#[derive(Debug, Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct RepoPolicyImports {
    #[serde(default)]
    knowledge: Vec<String>,
}

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

    let capture = merge_optional_values(
        shared.as_ref().and_then(|value| value.capture.clone()),
        local.as_ref().and_then(|value| value.capture.clone()),
    );
    let watch = merge_optional_values(
        shared.as_ref().and_then(|value| value.watch.clone()),
        local.as_ref().and_then(|value| value.watch.clone()),
    );
    let scope = merge_optional_values(
        shared.as_ref().and_then(|value| value.scope.clone()),
        local.as_ref().and_then(|value| value.scope.clone()),
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

    let fingerprint = fingerprint_repo_policy(
        &capture,
        &watch,
        &scope,
        &agents,
        &knowledge_import_paths,
        &imported_knowledge,
    )?;

    Ok(RepoPolicySnapshot {
        root: Some(location.root),
        shared_path: location.shared_path,
        local_path: location.local_path,
        capture,
        watch,
        scope,
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
    let agents = Value::Object(Map::new());
    let imported_knowledge = Vec::new();
    let knowledge_import_paths = Vec::new();
    let fingerprint = fingerprint_repo_policy(
        &capture,
        &watch,
        &scope,
        &agents,
        &knowledge_import_paths,
        &imported_knowledge,
    )
    .unwrap_or_else(|_| "default".to_string());

    RepoPolicySnapshot {
        root: None,
        shared_path: None,
        local_path: None,
        capture,
        watch,
        scope,
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

fn resolve_import_path(root: &Path, import_path: &str) -> PathBuf {
    let candidate = PathBuf::from(import_path);
    if candidate.is_absolute() {
        candidate
    } else {
        root.join(candidate)
    }
}

fn parse_policy_text(raw: &str, path: &Path) -> Result<RepoPolicyTomlFile> {
    from_str::<RepoPolicyTomlFile>(raw)
        .with_context(|| format!("parsing Bitloops repo policy {}", path.display()))
}

fn merge_optional_values(base: Option<Value>, overlay: Option<Value>) -> Value {
    match (base, overlay) {
        (Some(base), Some(overlay)) => deep_merge_value(base, overlay),
        (Some(base), None) => base,
        (None, Some(overlay)) => overlay,
        (None, None) => Value::Object(Map::new()),
    }
}

fn deep_merge_value(base: Value, overlay: Value) -> Value {
    match (base, overlay) {
        (Value::Object(base_map), Value::Object(overlay_map)) => {
            let mut merged = base_map;
            for (key, overlay_value) in overlay_map {
                match (merged.remove(&key), overlay_value) {
                    (_, Value::Null) => {}
                    (Some(existing), overlay_value) => {
                        merged.insert(key, deep_merge_value(existing, overlay_value));
                    }
                    (None, overlay_value) => {
                        merged.insert(key, overlay_value);
                    }
                }
            }
            Value::Object(merged)
        }
        (_, overlay) => overlay,
    }
}

fn fingerprint_repo_policy(
    capture: &Value,
    watch: &Value,
    scope: &Value,
    agents: &Value,
    knowledge_import_paths: &[String],
    imported_knowledge: &[ImportedKnowledgeConfig],
) -> Result<String> {
    let mut root = Map::new();
    root.insert("capture".into(), canonicalize_value(capture));
    root.insert("watch".into(), canonicalize_value(watch));
    root.insert("scope".into(), canonicalize_value(scope));
    root.insert("agents".into(), canonicalize_value(agents));
    root.insert(
        "imports".into(),
        Value::Object(Map::from_iter([(
            "knowledge".into(),
            Value::Array(
                knowledge_import_paths
                    .iter()
                    .map(|path| Value::String(path.clone()))
                    .collect(),
            ),
        )])),
    );
    root.insert(
        "knowledge".into(),
        Value::Array(
            imported_knowledge
                .iter()
                .map(|knowledge| {
                    Value::Object(Map::from_iter([
                        (
                            "path".into(),
                            Value::String(knowledge.path.to_string_lossy().to_string()),
                        ),
                        ("config".into(), canonicalize_value(&knowledge.value)),
                    ]))
                })
                .collect(),
        ),
    );

    let bytes = serde_json::to_vec(&Value::Object(root)).context("serialising repo policy")?;
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    Ok(hex::encode(hasher.finalize()))
}

fn canonicalize_value(value: &Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut keys: Vec<_> = map.keys().cloned().collect();
            keys.sort();
            let mut out = Map::new();
            for key in keys {
                if let Some(value) = map.get(&key) {
                    out.insert(key, canonicalize_value(value));
                }
            }
            Value::Object(out)
        }
        Value::Array(values) => Value::Array(values.iter().map(canonicalize_value).collect()),
        _ => value.clone(),
    }
}
