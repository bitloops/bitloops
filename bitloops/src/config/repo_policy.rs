use anyhow::{Context, Result, bail};
use serde::Deserialize;
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use std::collections::HashSet;
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
    pub daemon_config_path: Option<PathBuf>,
    pub capture: Value,
    pub watch: Value,
    pub scope: Value,
    pub contexts: Value,
    pub agents: Value,
    pub knowledge_import_paths: Vec<String>,
    pub imported_knowledge: Vec<ImportedKnowledgeConfig>,
    pub fingerprint: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoPolicyScopeExclusions {
    pub exclude: Vec<String>,
    pub exclude_from: Vec<String>,
    pub referenced_files: Vec<RepoPolicyExclusionFileReference>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoPolicyExclusionFileReference {
    pub configured_path: String,
    pub resolved_path: PathBuf,
    pub content: String,
    pub patterns: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct ImportedKnowledgeConfig {
    pub path: PathBuf,
    pub value: Value,
}

struct RepoPolicyFingerprintInputs<'a> {
    capture: &'a Value,
    watch: &'a Value,
    scope: &'a Value,
    scope_exclusions: &'a RepoPolicyScopeExclusions,
    contexts: &'a Value,
    agents: &'a Value,
    knowledge_import_paths: &'a [String],
    imported_knowledge: &'a [ImportedKnowledgeConfig],
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
    contexts: Option<Value>,
    #[serde(default)]
    agents: Option<Value>,
    #[serde(default)]
    daemon: RepoPolicyDaemon,
    #[serde(default)]
    imports: RepoPolicyImports,
}

#[derive(Debug, Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct RepoPolicyDaemon {
    #[serde(default)]
    config_path: Option<String>,
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

fn resolve_import_path(root: &Path, import_path: &str) -> PathBuf {
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

fn normalize_scope_exclusion_array_literals(raw: &str) -> Option<String> {
    let mut changed = false;
    let mut lines = Vec::new();

    for line in raw.lines() {
        let normalized = normalize_scope_exclusion_array_line(line);
        if normalized != line {
            changed = true;
        }
        lines.push(normalized);
    }

    if !changed {
        return None;
    }

    let mut out = lines.join("\n");
    if raw.ends_with('\n') {
        out.push('\n');
    }
    Some(out)
}

fn normalize_scope_exclusion_array_line(line: &str) -> String {
    if line.contains('#') {
        return line.to_string();
    }

    let Some((lhs, rhs)) = line.split_once('=') else {
        return line.to_string();
    };
    let key = lhs.trim();
    if key != "exclude" && key != "exclude_from" {
        return line.to_string();
    }

    let rhs = rhs.trim();
    if !(rhs.starts_with('[') && rhs.ends_with(']')) {
        return line.to_string();
    }

    let body = &rhs[1..rhs.len().saturating_sub(1)];
    let values = split_array_values(body);
    if values.is_empty() {
        return line.to_string();
    }

    let mut changed = false;
    let mut normalized_values = Vec::new();
    for value in values {
        let value = value.trim();
        if value.is_empty() {
            continue;
        }
        if is_quoted_string_literal(value) {
            normalized_values.push(value.to_string());
            continue;
        }

        normalized_values.push(format!("\"{}\"", escape_toml_basic_string(value)));
        changed = true;
    }

    if !changed {
        return line.to_string();
    }

    let indent_len = line.len() - line.trim_start().len();
    let indent = &line[..indent_len];
    format!("{indent}{key} = [{}]", normalized_values.join(", "))
}

fn split_array_values(input: &str) -> Vec<String> {
    let mut values = Vec::new();
    let mut current = String::new();
    let mut in_single_quotes = false;
    let mut in_double_quotes = false;
    let mut escaped = false;

    for ch in input.chars() {
        if escaped {
            current.push(ch);
            escaped = false;
            continue;
        }

        if in_double_quotes && ch == '\\' {
            current.push(ch);
            escaped = true;
            continue;
        }

        match ch {
            '\'' if !in_double_quotes => {
                in_single_quotes = !in_single_quotes;
                current.push(ch);
            }
            '"' if !in_single_quotes => {
                in_double_quotes = !in_double_quotes;
                current.push(ch);
            }
            ',' if !in_single_quotes && !in_double_quotes => {
                values.push(std::mem::take(&mut current));
            }
            _ => current.push(ch),
        }
    }

    values.push(current);
    values
}

fn is_quoted_string_literal(value: &str) -> bool {
    value.len() >= 2
        && ((value.starts_with('"') && value.ends_with('"'))
            || (value.starts_with('\'') && value.ends_with('\'')))
}

fn escape_toml_basic_string(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

fn merge_optional_values(base: Option<Value>, overlay: Option<Value>) -> Value {
    match (base, overlay) {
        (Some(base), Some(overlay)) => deep_merge_value(base, overlay),
        (Some(base), None) => base,
        (None, Some(overlay)) => overlay,
        (None, None) => Value::Object(Map::new()),
    }
}

fn merge_scope_values(base: Option<Value>, overlay: Option<Value>) -> Value {
    match (base, overlay) {
        (Some(base), Some(overlay)) => {
            if scope_overlay_replaces_exclusions(&overlay) {
                deep_merge_value(remove_scope_exclusion_keys(base), overlay)
            } else {
                deep_merge_value(base, overlay)
            }
        }
        (Some(base), None) => base,
        (None, Some(overlay)) => overlay,
        (None, None) => Value::Object(Map::new()),
    }
}

fn scope_overlay_replaces_exclusions(value: &Value) -> bool {
    value
        .as_object()
        .is_some_and(|map| map.contains_key("exclude") || map.contains_key("exclude_from"))
}

fn remove_scope_exclusion_keys(value: Value) -> Value {
    if let Value::Object(mut map) = value {
        map.remove("exclude");
        map.remove("exclude_from");
        Value::Object(map)
    } else {
        value
    }
}

pub fn resolve_repo_policy_scope_exclusions(
    scope: &Value,
    root: &Path,
) -> Result<RepoPolicyScopeExclusions> {
    let exclude = parse_scope_string_list(scope, "exclude")?;
    let exclude_from = parse_scope_string_list(scope, "exclude_from")?;
    let referenced_files = load_scope_exclusion_file_references(root, &exclude_from)?;
    Ok(RepoPolicyScopeExclusions {
        exclude,
        exclude_from,
        referenced_files,
    })
}

fn parse_scope_string_list(scope: &Value, key: &str) -> Result<Vec<String>> {
    let Some(scope_map) = scope.as_object() else {
        return Ok(Vec::new());
    };
    let Some(raw) = scope_map.get(key) else {
        return Ok(Vec::new());
    };

    let raw_values = raw
        .as_array()
        .with_context(|| format!("`scope.{key}` must be an array of strings"))?;
    let mut values = Vec::new();
    for item in raw_values {
        let value = item
            .as_str()
            .with_context(|| format!("`scope.{key}` values must be strings"))?
            .trim()
            .to_string();
        if !value.is_empty() {
            values.push(value);
        }
    }
    Ok(values)
}

fn load_scope_exclusion_file_references(
    root: &Path,
    configured_paths: &[String],
) -> Result<Vec<RepoPolicyExclusionFileReference>> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    let canonical_root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    for configured_path in configured_paths {
        let resolved = resolve_import_path(root, configured_path);
        let canonical = resolved.canonicalize().unwrap_or(resolved.clone());
        if !canonical.starts_with(&canonical_root) {
            bail!(
                "scope.exclude_from path `{}` resolves outside repo-policy root {}",
                configured_path,
                canonical_root.display()
            );
        }
        if !seen.insert(canonical.clone()) {
            continue;
        }

        let content = fs::read_to_string(&resolved).with_context(|| {
            format!(
                "reading scope.exclude_from patterns from {}",
                resolved.display()
            )
        })?;
        let patterns = parse_exclusion_patterns(&content);
        out.push(RepoPolicyExclusionFileReference {
            configured_path: configured_path.clone(),
            resolved_path: canonical,
            content,
            patterns,
        });
    }
    Ok(out)
}

pub fn parse_exclusion_patterns(content: &str) -> Vec<String> {
    content
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(str::to_string)
        .collect()
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

fn fingerprint_repo_policy(inputs: RepoPolicyFingerprintInputs<'_>) -> Result<String> {
    let RepoPolicyFingerprintInputs {
        capture,
        watch,
        scope,
        scope_exclusions,
        contexts,
        agents,
        knowledge_import_paths,
        imported_knowledge,
    } = inputs;
    let mut root = Map::new();
    root.insert("capture".into(), canonicalize_value(capture));
    root.insert("watch".into(), canonicalize_value(watch));
    root.insert("scope".into(), canonicalize_value(scope));
    root.insert(
        "scope_exclusions".into(),
        Value::Object(Map::from_iter([
            (
                "exclude".into(),
                Value::Array(
                    scope_exclusions
                        .exclude
                        .iter()
                        .map(|value| Value::String(value.clone()))
                        .collect(),
                ),
            ),
            (
                "exclude_from".into(),
                Value::Array(
                    scope_exclusions
                        .exclude_from
                        .iter()
                        .map(|value| Value::String(value.clone()))
                        .collect(),
                ),
            ),
            (
                "exclude_from_files".into(),
                Value::Array(
                    scope_exclusions
                        .referenced_files
                        .iter()
                        .map(|entry| {
                            Value::Object(Map::from_iter([
                                (
                                    "configured_path".into(),
                                    Value::String(entry.configured_path.clone()),
                                ),
                                (
                                    "resolved_path".into(),
                                    Value::String(
                                        entry.resolved_path.to_string_lossy().to_string(),
                                    ),
                                ),
                                ("content".into(), Value::String(entry.content.clone())),
                                (
                                    "patterns".into(),
                                    Value::Array(
                                        entry
                                            .patterns
                                            .iter()
                                            .map(|value| Value::String(value.clone()))
                                            .collect(),
                                    ),
                                ),
                            ]))
                        })
                        .collect(),
                ),
            ),
        ])),
    );
    root.insert("contexts".into(), canonicalize_value(contexts));
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

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn discover_repo_policy_reads_local_daemon_binding() {
        let repo = tempdir().expect("temp dir");
        let local_policy = repo.path().join(REPO_POLICY_LOCAL_FILE_NAME);
        fs::write(
            &local_policy,
            r#"
[daemon]
config_path = "/tmp/daemon/config.toml"
"#,
        )
        .expect("write local repo policy");

        let snapshot = discover_repo_policy(repo.path()).expect("discover repo policy");

        assert_eq!(
            snapshot.daemon_config_path,
            Some(PathBuf::from("/tmp/daemon/config.toml"))
        );
    }

    #[test]
    fn discover_repo_policy_rejects_shared_daemon_binding() {
        let repo = tempdir().expect("temp dir");
        let shared_policy = repo.path().join(REPO_POLICY_FILE_NAME);
        fs::write(
            &shared_policy,
            r#"
[daemon]
config_path = "/tmp/daemon/config.toml"
"#,
        )
        .expect("write shared repo policy");

        let err = discover_repo_policy(repo.path()).expect_err("shared daemon binding must fail");

        assert!(
            err.to_string()
                .contains("Bitloops daemon binding must be local-only"),
            "unexpected error: {err:#}"
        );
    }

    #[test]
    fn daemon_binding_does_not_change_repo_policy_fingerprint() {
        let repo = tempdir().expect("temp dir");
        let shared_policy = repo.path().join(REPO_POLICY_FILE_NAME);
        let local_policy = repo.path().join(REPO_POLICY_LOCAL_FILE_NAME);
        fs::write(
            &shared_policy,
            r#"
[capture]
enabled = true
"#,
        )
        .expect("write shared repo policy");
        fs::write(
            &local_policy,
            r#"
[daemon]
config_path = "/tmp/daemon-a/config.toml"
"#,
        )
        .expect("write first local repo policy");

        let first = discover_repo_policy(repo.path())
            .expect("discover first repo policy")
            .fingerprint;

        fs::write(
            &local_policy,
            r#"
[daemon]
config_path = "/tmp/daemon-b/config.toml"
"#,
        )
        .expect("write second local repo policy");

        let second = discover_repo_policy(repo.path())
            .expect("discover second repo policy")
            .fingerprint;

        assert_eq!(first, second);
    }

    #[test]
    fn local_scope_exclusions_replace_shared_values() {
        let temp = tempfile::tempdir().expect("temp dir");
        std::fs::create_dir_all(temp.path().join(".git")).expect("create .git");
        std::fs::write(
            temp.path().join(REPO_POLICY_FILE_NAME),
            r#"
[scope]
project_root = "packages/api"
include = ["src/**"]
exclude = ["dist/**"]
exclude_from = ["shared.ignore"]
"#,
        )
        .expect("write shared policy");
        std::fs::write(
            temp.path().join(REPO_POLICY_LOCAL_FILE_NAME),
            r#"
[scope]
exclude_from = ["local.ignore"]
"#,
        )
        .expect("write local policy");
        std::fs::write(temp.path().join("shared.ignore"), "vendor/**\n").expect("write shared");
        std::fs::write(temp.path().join("local.ignore"), "tmp/**\n").expect("write local");

        let snapshot = discover_repo_policy(temp.path()).expect("discover policy");
        let scope = snapshot.scope.as_object().expect("scope object");
        assert_eq!(
            scope.get("include"),
            Some(&serde_json::json!(["src/**"])),
            "non-exclusion keys should still inherit from shared"
        );
        assert!(
            scope.get("exclude").is_none(),
            "shared scope.exclude should be cleared when local defines exclusion keys"
        );
        assert_eq!(
            scope.get("exclude_from"),
            Some(&serde_json::json!(["local.ignore"]))
        );
    }

    #[test]
    fn shared_scope_exclusions_apply_when_local_exclusion_keys_absent() {
        let temp = tempfile::tempdir().expect("temp dir");
        std::fs::create_dir_all(temp.path().join(".git")).expect("create .git");
        std::fs::write(
            temp.path().join(REPO_POLICY_FILE_NAME),
            r#"
[scope]
exclude = ["dist/**"]
exclude_from = ["shared.ignore"]
"#,
        )
        .expect("write shared policy");
        std::fs::write(
            temp.path().join(REPO_POLICY_LOCAL_FILE_NAME),
            r#"
[scope]
project_root = "packages/app"
"#,
        )
        .expect("write local policy");
        std::fs::write(temp.path().join("shared.ignore"), "vendor/**\n").expect("write shared");

        let snapshot = discover_repo_policy(temp.path()).expect("discover policy");
        let scope = snapshot.scope.as_object().expect("scope object");
        assert_eq!(scope.get("exclude"), Some(&serde_json::json!(["dist/**"])));
        assert_eq!(
            scope.get("exclude_from"),
            Some(&serde_json::json!(["shared.ignore"]))
        );
    }

    #[test]
    fn policy_fingerprint_changes_when_exclude_from_file_content_changes() {
        let temp = tempfile::tempdir().expect("temp dir");
        std::fs::create_dir_all(temp.path().join(".git")).expect("create .git");
        std::fs::write(
            temp.path().join(REPO_POLICY_FILE_NAME),
            r#"
[scope]
exclude_from = [".bitloopsignore"]
"#,
        )
        .expect("write shared policy");
        let ignore_path = temp.path().join(".bitloopsignore");
        std::fs::write(&ignore_path, "vendor/**\n").expect("write ignore");
        let first = discover_repo_policy(temp.path())
            .expect("discover policy")
            .fingerprint;

        std::fs::write(&ignore_path, "vendor/**\nbuild/**\n").expect("rewrite ignore");
        let second = discover_repo_policy(temp.path())
            .expect("discover policy")
            .fingerprint;
        assert_ne!(first, second);
    }

    #[test]
    fn exclude_from_paths_outside_policy_root_are_rejected() {
        let temp = tempfile::tempdir().expect("temp dir");
        let outside = tempfile::tempdir().expect("outside temp dir");
        std::fs::create_dir_all(temp.path().join(".git")).expect("create .git");
        std::fs::write(outside.path().join("outside.ignore"), "vendor/**\n")
            .expect("write outside ignore");
        std::fs::write(
            temp.path().join(REPO_POLICY_FILE_NAME),
            format!(
                r#"
[scope]
exclude_from = ["{}"]
"#,
                outside.path().join("outside.ignore").display()
            ),
        )
        .expect("write policy");

        let err = discover_repo_policy(temp.path()).expect_err("outside-root paths should fail");
        assert!(
            err.to_string().contains("outside repo-policy root"),
            "unexpected error: {err:#}"
        );
    }

    #[test]
    fn discover_policy_accepts_unquoted_scope_exclusion_array_values() {
        let temp = tempfile::tempdir().expect("temp dir");
        std::fs::create_dir_all(temp.path().join(".git")).expect("create .git");
        std::fs::write(
            temp.path().join(REPO_POLICY_LOCAL_FILE_NAME),
            r#"
[scope]
exclude = [docs/**]
exclude_from = [.bitloopsignore]
"#,
        )
        .expect("write local policy");
        std::fs::write(temp.path().join(".bitloopsignore"), "vendor/**\n")
            .expect("write ignore file");

        let snapshot = discover_repo_policy(temp.path()).expect("discover policy");
        let scope = snapshot.scope.as_object().expect("scope object");
        assert_eq!(scope.get("exclude"), Some(&serde_json::json!(["docs/**"])));
        assert_eq!(
            scope.get("exclude_from"),
            Some(&serde_json::json!([".bitloopsignore"]))
        );
    }
}
