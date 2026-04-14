use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::path::Path;

use anyhow::{Context, Result, anyhow, bail};
use regex::Regex;
use serde_json::{Map, Value};

use crate::host::devql::core_extension_host;
use crate::host::extension_host::LanguagePackResolutionInput;

use super::path_rules::{
    ancestor_dirs, auto_ignored_dirs_for_kind, compute_sha256_json, display_root, is_jsconfig_file,
    is_requirements_file, is_tsconfig_file, join_dir_file, matching_entries, path_depth,
    pathbuf_to_repo_relative, relative_to_root, resolve_policy_relative_path, sorted_dedup,
};
use super::patterns::PathPatternMatcher;
use super::repo_view::RepoContentView;
use super::types::ProjectContext;

#[derive(Debug, Clone)]
pub(super) struct AutoScopePolicy {
    root_rel: Option<String>,
    include_matcher: Option<PathPatternMatcher>,
}

impl AutoScopePolicy {
    pub(super) fn from_scope(
        repo_root_abs: &Path,
        policy_root_abs: &Path,
        scope: &Value,
    ) -> Result<Self> {
        let project_root = scope
            .as_object()
            .and_then(|map| map.get("project_root"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let include = parse_scope_string_list(scope, "include")?;
        let root_rel = project_root
            .map(|value| resolve_policy_relative_path(repo_root_abs, policy_root_abs, value))
            .transpose()?
            .map(|path| {
                path.strip_prefix(repo_root_abs)
                    .unwrap_or(path.as_path())
                    .to_path_buf()
            })
            .map(|path| pathbuf_to_repo_relative(&path));
        let include_matcher = if include.is_empty() {
            None
        } else {
            Some(PathPatternMatcher::new(include)?)
        };
        Ok(Self {
            root_rel,
            include_matcher,
        })
    }

    pub(super) fn allows_path(&self, path: &str) -> bool {
        let Some(relative_to_root) = self.relative_to_auto_root(path) else {
            return false;
        };
        self.include_matcher
            .as_ref()
            .is_none_or(|matcher| matcher.is_match(&relative_to_root))
    }

    pub(super) fn allows_context_root(&self, path: &str) -> bool {
        self.relative_to_auto_root(path).is_some()
    }

    fn relative_to_auto_root(&self, path: &str) -> Option<String> {
        let path = crate::host::devql::normalize_repo_path(path);
        if let Some(root_rel) = &self.root_rel {
            if root_rel.is_empty() {
                return Some(path);
            }
            if path == *root_rel {
                return Some(String::new());
            }
            let prefix = format!("{root_rel}/");
            return path.strip_prefix(&prefix).map(str::to_string);
        }
        Some(path)
    }
}

#[derive(Debug, Clone)]
pub(super) struct ResolvedContext {
    pub(super) context: ProjectContext,
    pub(super) root_rel: String,
    pub(super) root_depth: usize,
    pub(super) manual: bool,
    include_matcher: Option<PathPatternMatcher>,
    exclude_matcher: Option<PathPatternMatcher>,
    pub(super) code_extensions: HashSet<String>,
    pub(super) profile_id: Option<String>,
    auto_ignored_dirs: Vec<String>,
}

impl ResolvedContext {
    pub(super) fn matches_code_path(&self, path: &str) -> bool {
        if !self.matches_affiliated_path(path) {
            return false;
        }
        let extension = super::path_rules::lower_extension(path);
        extension
            .as_deref()
            .is_some_and(|extension| self.code_extensions.contains(extension))
    }

    pub(super) fn matches_affiliated_path(&self, path: &str) -> bool {
        let Some(relative) = relative_to_root(path, &self.root_rel) else {
            return false;
        };
        if self.is_auto_ignored(path) {
            return false;
        }
        if self
            .include_matcher
            .as_ref()
            .is_some_and(|matcher| !matcher.is_match(&relative))
        {
            return false;
        }
        if self
            .exclude_matcher
            .as_ref()
            .is_some_and(|matcher| matcher.is_match(&relative))
        {
            return false;
        }
        true
    }

    pub(super) fn is_auto_ignored(&self, path: &str) -> bool {
        let Some(relative) = relative_to_root(path, &self.root_rel) else {
            return false;
        };
        self.auto_ignored_dirs
            .iter()
            .any(|dir| relative == *dir || relative.starts_with(&format!("{dir}/")))
    }
}

#[derive(Debug, Clone)]
struct ManualContextSpec {
    id: String,
    kind: String,
    root_rel: String,
    include: Vec<String>,
    exclude: Vec<String>,
    frameworks: Vec<String>,
    runtime_profile: Option<String>,
    source_version_override: Option<String>,
    profile: Option<String>,
}

struct AutoContextBuildSpec<'a> {
    kind: &'a str,
    dir: &'a str,
    config_files: Vec<String>,
    base_languages: Vec<String>,
    frameworks: Vec<String>,
    runtime_profile: Option<String>,
    source_versions: BTreeMap<String, String>,
    code_extensions: HashSet<String>,
    profile_id: Option<String>,
}

pub(super) fn detect_auto_contexts<V: RepoContentView>(
    view: &V,
    candidate_paths: &[String],
    auto_scope: &AutoScopePolicy,
) -> Result<Vec<ResolvedContext>> {
    let mut directories = BTreeSet::new();
    for path in candidate_paths {
        for ancestor in ancestor_dirs(path) {
            directories.insert(ancestor);
        }
    }
    directories.insert(String::new());

    let mut contexts = Vec::new();
    for dir in directories {
        let entries = view.list_dir_entries(&dir)?;
        if entries.is_empty() && !dir.is_empty() {
            continue;
        }

        if let Some(context) = build_rust_context(view, auto_scope, &dir, &entries)? {
            contexts.push(context);
        }
        if let Some(context) = build_node_context(view, auto_scope, &dir, &entries)? {
            contexts.push(context);
        }
        if let Some(context) = build_typescript_context(view, auto_scope, &dir, &entries)? {
            contexts.push(context);
        }
        if let Some(context) = build_python_context(view, auto_scope, &dir, &entries)? {
            contexts.push(context);
        }
        if let Some(context) = build_go_context(view, auto_scope, &dir, &entries)? {
            contexts.push(context);
        }
        if let Some(context) = build_java_context(view, auto_scope, &dir, &entries)? {
            contexts.push(context);
        }
    }

    Ok(contexts)
}

pub(super) fn parse_manual_context_specs(
    contexts: &Value,
    repo_root_abs: &Path,
    policy_root_abs: &Path,
) -> Result<Vec<ResolvedContext>> {
    let Some(items) = contexts.as_array() else {
        if contexts.is_null() || contexts.as_object().is_some_and(serde_json::Map::is_empty) {
            return Ok(Vec::new());
        }
        bail!("`contexts` must be an array of tables");
    };
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for item in items {
        let Some(map) = item.as_object() else {
            bail!("`contexts` entries must be objects");
        };
        let id = required_string(map, "id")?;
        if !seen.insert(id.clone()) {
            bail!("duplicate manual context id `{id}`");
        }
        let kind = required_string(map, "kind")?;
        let root_raw = required_string(map, "root")?;
        let root_abs = resolve_policy_relative_path(repo_root_abs, policy_root_abs, &root_raw)?;
        let root_rel =
            pathbuf_to_repo_relative(root_abs.strip_prefix(repo_root_abs).unwrap_or(&root_abs));
        out.push(build_manual_context_from_spec(&ManualContextSpec {
            id,
            kind,
            root_rel,
            include: optional_string_list(map, "include")?,
            exclude: optional_string_list(map, "exclude")?,
            frameworks: optional_string_list(map, "frameworks")?,
            runtime_profile: optional_string(map, "runtime_profile")?,
            source_version_override: optional_string(map, "source_version_override")?,
            profile: optional_string(map, "profile")?,
        })?);
    }
    Ok(out)
}

pub(super) fn parse_scope_string_list(scope: &Value, key: &str) -> Result<Vec<String>> {
    let Some(scope_map) = scope.as_object() else {
        return Ok(Vec::new());
    };
    let Some(raw) = scope_map.get(key) else {
        return Ok(Vec::new());
    };
    let raw_values = raw
        .as_array()
        .with_context(|| format!("`scope.{key}` must be an array of strings"))?;
    Ok(raw_values
        .iter()
        .filter_map(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .collect())
}

fn build_manual_context_from_spec(spec: &ManualContextSpec) -> Result<ResolvedContext> {
    let (base_language, code_extensions) = match spec.kind.as_str() {
        "rust" => ("rust".to_string(), vec!["rs".to_string()]),
        "node" => (
            "javascript".to_string(),
            ["js", "jsx", "mjs", "cjs"]
                .into_iter()
                .map(str::to_string)
                .collect(),
        ),
        "python" => ("python".to_string(), vec!["py".to_string()]),
        "go" => ("go".to_string(), vec!["go".to_string()]),
        "java" => ("java".to_string(), vec!["java".to_string()]),
        "standalone" => {
            let profile = spec.profile.as_deref().ok_or_else(|| {
                anyhow!("manual standalone context `{}` requires `profile`", spec.id)
            })?;
            let host = core_extension_host()?;
            let resolved = host
                .language_packs()
                .resolve(LanguagePackResolutionInput::for_profile(profile))
                .with_context(|| format!("resolving standalone context profile `{profile}`"))?;
            (
                resolved.profile.language_id.to_string(),
                resolved
                    .profile
                    .file_extensions
                    .iter()
                    .map(|value| value.to_ascii_lowercase())
                    .collect(),
            )
        }
        other => bail!("unsupported manual context kind `{other}`"),
    };

    let mut source_versions = BTreeMap::new();
    if let Some(source_version) = spec.source_version_override.as_deref() {
        source_versions.insert(base_language.clone(), source_version.to_string());
    }
    let config_value = Value::Object(Map::from_iter([
        ("id".into(), Value::String(spec.id.clone())),
        ("kind".into(), Value::String(spec.kind.clone())),
        ("root".into(), Value::String(spec.root_rel.clone())),
        (
            "include".into(),
            Value::Array(spec.include.iter().cloned().map(Value::String).collect()),
        ),
        (
            "exclude".into(),
            Value::Array(spec.exclude.iter().cloned().map(Value::String).collect()),
        ),
        (
            "frameworks".into(),
            Value::Array(spec.frameworks.iter().cloned().map(Value::String).collect()),
        ),
        (
            "runtime_profile".into(),
            spec.runtime_profile
                .clone()
                .map(Value::String)
                .unwrap_or(Value::Null),
        ),
        (
            "source_versions".into(),
            Value::Object(Map::from_iter(
                source_versions
                    .iter()
                    .map(|(key, value)| (key.clone(), Value::String(value.clone()))),
            )),
        ),
        (
            "profile".into(),
            spec.profile
                .clone()
                .map(Value::String)
                .unwrap_or(Value::Null),
        ),
    ]));

    Ok(ResolvedContext {
        context: ProjectContext {
            context_id: spec.id.clone(),
            root: display_root(&spec.root_rel),
            kind: spec.kind.clone(),
            detection_source: "manual".to_string(),
            config_files: Vec::new(),
            config_fingerprint: compute_sha256_json(&config_value),
            base_languages: vec![base_language],
            frameworks: sorted_dedup(spec.frameworks.clone()),
            runtime_profile: spec.runtime_profile.clone(),
            source_versions,
        },
        root_rel: spec.root_rel.clone(),
        root_depth: path_depth(&spec.root_rel),
        manual: true,
        include_matcher: (!spec.include.is_empty())
            .then(|| PathPatternMatcher::new(spec.include.clone()))
            .transpose()?,
        exclude_matcher: (!spec.exclude.is_empty())
            .then(|| PathPatternMatcher::new(spec.exclude.clone()))
            .transpose()?,
        code_extensions: code_extensions.into_iter().collect(),
        profile_id: spec.profile.clone(),
        auto_ignored_dirs: auto_ignored_dirs_for_kind(&spec.kind),
    })
}

fn build_rust_context<V: RepoContentView>(
    view: &V,
    auto_scope: &AutoScopePolicy,
    dir: &str,
    entries: &[String],
) -> Result<Option<ResolvedContext>> {
    if !entries.iter().any(|entry| entry == "Cargo.toml") {
        return Ok(None);
    }
    let cargo_path = join_dir_file(dir, "Cargo.toml");
    if !auto_scope.allows_context_root(dir) {
        return Ok(None);
    }
    let cargo_content = view.read_text(&cargo_path)?.unwrap_or_default();
    let toolchain_path = join_dir_file(dir, "rust-toolchain.toml");
    let toolchain_content = view.read_text(&toolchain_path)?;
    let mut config_files = vec![cargo_path.clone()];
    let mut source_versions = BTreeMap::new();
    if let Some(version) = toolchain_content
        .as_deref()
        .and_then(parse_rust_toolchain_channel)
        .or_else(|| parse_cargo_rust_version(&cargo_content))
    {
        source_versions.insert("rust".to_string(), version);
    }
    if toolchain_content.is_some() {
        config_files.push(toolchain_path);
    }
    Ok(Some(build_auto_context(AutoContextBuildSpec {
        kind: "rust",
        dir,
        config_files,
        base_languages: vec!["rust".to_string()],
        frameworks: Vec::new(),
        runtime_profile: None,
        source_versions,
        code_extensions: HashSet::from(["rs".to_string()]),
        profile_id: None,
    })))
}

fn build_node_context<V: RepoContentView>(
    view: &V,
    auto_scope: &AutoScopePolicy,
    dir: &str,
    entries: &[String],
) -> Result<Option<ResolvedContext>> {
    let jsconfigs = matching_entries(entries, is_jsconfig_file);
    let has_package_json = entries.iter().any(|entry| entry == "package.json");
    if !has_package_json && jsconfigs.is_empty() {
        return Ok(None);
    }
    let package_path = join_dir_file(dir, "package.json");
    let jsconfig_paths = jsconfigs
        .iter()
        .map(|entry| join_dir_file(dir, entry))
        .collect::<Vec<_>>();
    let seed_path = if has_package_json {
        package_path.as_str()
    } else {
        jsconfig_paths
            .first()
            .map(String::as_str)
            .unwrap_or_default()
    };
    if seed_path.is_empty() || !auto_scope.allows_context_root(dir) {
        return Ok(None);
    }
    let package_content = if has_package_json {
        view.read_text(&package_path)?
    } else {
        None
    };
    let next_configs = matching_entries(entries, |entry| entry.starts_with("next.config."));
    let next_config_paths = next_configs
        .iter()
        .map(|entry| join_dir_file(dir, entry))
        .collect::<Vec<_>>();
    let frameworks = package_content
        .as_deref()
        .map(parse_package_frameworks)
        .transpose()?
        .unwrap_or_default();
    let runtime_profile = frameworks
        .iter()
        .find(|value| value.starts_with("next"))
        .map(|_| "next".to_string())
        .or_else(|| {
            frameworks
                .iter()
                .find(|value| value.starts_with("react"))
                .map(|_| "react".to_string())
        });
    let mut config_files = Vec::new();
    if has_package_json {
        config_files.push(package_path);
    }
    config_files.extend(jsconfig_paths);
    config_files.extend(next_config_paths);
    Ok(Some(build_auto_context(AutoContextBuildSpec {
        kind: "node",
        dir,
        config_files,
        base_languages: vec!["javascript".to_string()],
        frameworks,
        runtime_profile,
        source_versions: BTreeMap::new(),
        code_extensions: HashSet::from_iter(
            ["js", "jsx", "mjs", "cjs"].into_iter().map(str::to_string),
        ),
        profile_id: None,
    })))
}

fn build_typescript_context<V: RepoContentView>(
    view: &V,
    auto_scope: &AutoScopePolicy,
    dir: &str,
    entries: &[String],
) -> Result<Option<ResolvedContext>> {
    let tsconfigs = matching_entries(entries, |entry| {
        is_tsconfig_file(entry) || is_jsconfig_file(entry)
    });
    let package_path = join_dir_file(dir, "package.json");
    let package_content = view.read_text(&package_path)?;
    let has_typescript_dependency = package_content
        .as_deref()
        .and_then(parse_package_dependency_version)
        .is_some();
    if tsconfigs.is_empty() && !has_typescript_dependency {
        return Ok(None);
    }
    let seed_path = tsconfigs
        .first()
        .map(|entry| join_dir_file(dir, entry))
        .or_else(|| has_typescript_dependency.then_some(package_path.clone()))
        .unwrap_or_default();
    if seed_path.is_empty() || !auto_scope.allows_context_root(dir) {
        return Ok(None);
    }
    let next_configs = matching_entries(entries, |entry| entry.starts_with("next.config."));
    let mut config_files = tsconfigs
        .iter()
        .map(|entry| join_dir_file(dir, entry))
        .collect::<Vec<_>>();
    if package_content.is_some() {
        config_files.push(package_path);
    }
    config_files.extend(next_configs.iter().map(|entry| join_dir_file(dir, entry)));
    let frameworks = package_content
        .as_deref()
        .map(parse_package_frameworks)
        .transpose()?
        .unwrap_or_default();
    let runtime_profile = frameworks
        .iter()
        .find(|value| value.starts_with("next"))
        .map(|_| "next".to_string())
        .or_else(|| {
            frameworks
                .iter()
                .find(|value| value.starts_with("react"))
                .map(|_| "react".to_string())
        });
    let mut source_versions = BTreeMap::new();
    if let Some(version) = package_content
        .as_deref()
        .and_then(parse_package_dependency_version)
    {
        source_versions.insert("typescript".to_string(), version);
    }
    Ok(Some(build_auto_context(AutoContextBuildSpec {
        kind: "typescript",
        dir,
        config_files,
        base_languages: vec!["typescript".to_string()],
        frameworks,
        runtime_profile,
        source_versions,
        code_extensions: HashSet::from_iter(
            ["ts", "tsx", "mts", "cts"].into_iter().map(str::to_string),
        ),
        profile_id: None,
    })))
}

fn build_python_context<V: RepoContentView>(
    view: &V,
    auto_scope: &AutoScopePolicy,
    dir: &str,
    entries: &[String],
) -> Result<Option<ResolvedContext>> {
    let seeds = matching_entries(entries, |entry| {
        entry == "pyproject.toml"
            || entry == "setup.py"
            || entry == "setup.cfg"
            || entry == "Pipfile"
            || is_requirements_file(entry)
    });
    if seeds.is_empty() {
        return Ok(None);
    }
    let seed_path = join_dir_file(dir, &seeds[0]);
    if seed_path.is_empty() || !auto_scope.allows_context_root(dir) {
        return Ok(None);
    }
    let mut config_files = seeds
        .iter()
        .map(|entry| join_dir_file(dir, entry))
        .collect::<Vec<_>>();
    let python_version_path = join_dir_file(dir, ".python-version");
    let python_version_content = view.read_text(&python_version_path)?;
    if python_version_content.is_some() {
        config_files.push(python_version_path);
    }
    let pyproject_path = join_dir_file(dir, "pyproject.toml");
    let pyproject_content = view.read_text(&pyproject_path)?;
    let mut source_versions = BTreeMap::new();
    if let Some(version) = pyproject_content
        .as_deref()
        .and_then(parse_pyproject_requires_python)
        .or_else(|| {
            python_version_content
                .as_deref()
                .map(str::trim)
                .map(str::to_string)
        })
        .filter(|value| !value.is_empty())
    {
        source_versions.insert("python".to_string(), version);
    }
    Ok(Some(build_auto_context(AutoContextBuildSpec {
        kind: "python",
        dir,
        config_files,
        base_languages: vec!["python".to_string()],
        frameworks: Vec::new(),
        runtime_profile: None,
        source_versions,
        code_extensions: HashSet::from(["py".to_string()]),
        profile_id: None,
    })))
}

fn build_go_context<V: RepoContentView>(
    view: &V,
    auto_scope: &AutoScopePolicy,
    dir: &str,
    entries: &[String],
) -> Result<Option<ResolvedContext>> {
    if !entries.iter().any(|entry| entry == "go.mod") {
        return Ok(None);
    }
    let go_mod_path = join_dir_file(dir, "go.mod");
    if !auto_scope.allows_context_root(dir) {
        return Ok(None);
    }
    let go_mod = view.read_text(&go_mod_path)?.unwrap_or_default();
    let mut source_versions = BTreeMap::new();
    if let Some(version) = parse_go_mod_version(&go_mod) {
        source_versions.insert("go".to_string(), version);
    }
    Ok(Some(build_auto_context(AutoContextBuildSpec {
        kind: "go",
        dir,
        config_files: vec![go_mod_path],
        base_languages: vec!["go".to_string()],
        frameworks: Vec::new(),
        runtime_profile: None,
        source_versions,
        code_extensions: HashSet::from(["go".to_string()]),
        profile_id: None,
    })))
}

fn build_java_context<V: RepoContentView>(
    view: &V,
    auto_scope: &AutoScopePolicy,
    dir: &str,
    entries: &[String],
) -> Result<Option<ResolvedContext>> {
    let seeds = matching_entries(entries, |entry| {
        matches!(
            entry,
            "pom.xml"
                | "build.gradle"
                | "build.gradle.kts"
                | "settings.gradle"
                | "settings.gradle.kts"
        )
    });
    if seeds.is_empty() {
        return Ok(None);
    }
    let seed_path = join_dir_file(dir, &seeds[0]);
    if seed_path.is_empty() || !auto_scope.allows_context_root(dir) {
        return Ok(None);
    }
    let config_files = seeds
        .iter()
        .map(|entry| join_dir_file(dir, entry))
        .collect::<Vec<_>>();
    let mut source_versions = BTreeMap::new();
    for config_file in &config_files {
        let Some(content) = view.read_text(config_file)? else {
            continue;
        };
        if let Some(version) = parse_java_version(&content).filter(|value| !value.trim().is_empty())
        {
            source_versions.insert("java".to_string(), version);
            break;
        }
    }
    if !source_versions.contains_key("java") {
        source_versions.insert("java".to_string(), "unknown".to_string());
    }
    Ok(Some(build_auto_context(AutoContextBuildSpec {
        kind: "java",
        dir,
        config_files,
        base_languages: vec!["java".to_string()],
        frameworks: Vec::new(),
        runtime_profile: None,
        source_versions,
        code_extensions: HashSet::from(["java".to_string()]),
        profile_id: None,
    })))
}

fn build_auto_context(spec: AutoContextBuildSpec<'_>) -> ResolvedContext {
    let AutoContextBuildSpec {
        kind,
        dir,
        config_files,
        base_languages,
        frameworks,
        runtime_profile,
        source_versions,
        code_extensions,
        profile_id,
    } = spec;
    let config_files = sorted_dedup(config_files);
    let frameworks = sorted_dedup(frameworks);
    let config_value = Value::Object(Map::from_iter([
        ("kind".into(), Value::String(kind.to_string())),
        ("root".into(), Value::String(display_root(dir))),
        (
            "config_files".into(),
            Value::Array(config_files.iter().cloned().map(Value::String).collect()),
        ),
        (
            "frameworks".into(),
            Value::Array(frameworks.iter().cloned().map(Value::String).collect()),
        ),
        (
            "runtime_profile".into(),
            runtime_profile
                .clone()
                .map(Value::String)
                .unwrap_or(Value::Null),
        ),
        (
            "source_versions".into(),
            Value::Object(Map::from_iter(
                source_versions
                    .iter()
                    .map(|(key, value)| (key.clone(), Value::String(value.clone()))),
            )),
        ),
    ]));
    let context_id = format!("auto:{kind}:{}", display_root(dir));
    let context_fingerprint = compute_sha256_json(&config_value);
    ResolvedContext {
        context: ProjectContext {
            context_id,
            root: display_root(dir),
            kind: kind.to_string(),
            detection_source: "auto".to_string(),
            config_files,
            config_fingerprint: context_fingerprint,
            base_languages,
            frameworks,
            runtime_profile,
            source_versions,
        },
        root_rel: dir.to_string(),
        root_depth: path_depth(dir),
        manual: false,
        include_matcher: None,
        exclude_matcher: None,
        code_extensions,
        profile_id,
        auto_ignored_dirs: auto_ignored_dirs_for_kind(kind),
    }
}

fn required_string(map: &Map<String, Value>, key: &str) -> Result<String> {
    map.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .with_context(|| format!("`contexts.{key}` must be a non-empty string"))
}

fn optional_string(map: &Map<String, Value>, key: &str) -> Result<Option<String>> {
    let Some(value) = map.get(key) else {
        return Ok(None);
    };
    Ok(value
        .as_str()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string))
}

fn optional_string_list(map: &Map<String, Value>, key: &str) -> Result<Vec<String>> {
    let Some(value) = map.get(key) else {
        return Ok(Vec::new());
    };
    let Some(values) = value.as_array() else {
        bail!("`contexts.{key}` must be an array of strings");
    };
    Ok(values
        .iter()
        .filter_map(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .collect())
}

fn parse_package_dependency_version(content: &str) -> Option<String> {
    let parsed = serde_json::from_str::<Value>(content).ok()?;
    parsed
        .get("dependencies")
        .and_then(Value::as_object)
        .and_then(|deps| deps.get("typescript"))
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| {
            parsed
                .get("devDependencies")
                .and_then(Value::as_object)
                .and_then(|deps| deps.get("typescript"))
                .and_then(Value::as_str)
                .map(str::to_string)
        })
}

fn parse_package_frameworks(content: &str) -> Result<Vec<String>> {
    let parsed = serde_json::from_str::<Value>(content).context("parsing package.json")?;
    let mut frameworks = Vec::new();
    for dependency in ["react", "next"] {
        let version = parsed
            .get("dependencies")
            .and_then(Value::as_object)
            .and_then(|deps| deps.get(dependency))
            .and_then(Value::as_str)
            .or_else(|| {
                parsed
                    .get("devDependencies")
                    .and_then(Value::as_object)
                    .and_then(|deps| deps.get(dependency))
                    .and_then(Value::as_str)
            });
        if let Some(version) = version {
            frameworks.push(format!("{dependency}@{version}"));
        }
    }
    Ok(sorted_dedup(frameworks))
}

fn parse_rust_toolchain_channel(content: &str) -> Option<String> {
    Regex::new(r#"(?m)^\s*channel\s*=\s*"([^"]+)""#)
        .ok()?
        .captures(content)?
        .get(1)
        .map(|value| value.as_str().to_string())
}

fn parse_cargo_rust_version(content: &str) -> Option<String> {
    Regex::new(r#"(?m)^\s*rust-version\s*=\s*"([^"]+)""#)
        .ok()?
        .captures(content)?
        .get(1)
        .map(|value| value.as_str().to_string())
}

fn parse_pyproject_requires_python(content: &str) -> Option<String> {
    Regex::new(r#"(?m)^\s*requires-python\s*=\s*"([^"]+)""#)
        .ok()?
        .captures(content)?
        .get(1)
        .map(|value| value.as_str().to_string())
}

fn parse_go_mod_version(content: &str) -> Option<String> {
    Regex::new(r#"(?m)^\s*go\s+([^\s]+)\s*$"#)
        .ok()?
        .captures(content)?
        .get(1)
        .map(|value| value.as_str().to_string())
}

fn parse_java_version(content: &str) -> Option<String> {
    for pattern in [
        r#"(?s)<maven\.compiler\.release>\s*([^<\s]+)\s*</maven\.compiler\.release>"#,
        r#"(?s)<maven\.compiler\.source>\s*([^<\s]+)\s*</maven\.compiler\.source>"#,
        r#"(?s)<java\.version>\s*([^<\s]+)\s*</java\.version>"#,
        r#"(?m)^\s*sourceCompatibility\s*=\s*['"]?([^'"\s]+)"#,
        r#"(?m)^\s*targetCompatibility\s*=\s*['"]?([^'"\s]+)"#,
    ] {
        if let Some(version) = Regex::new(pattern)
            .ok()
            .and_then(|regex| regex.captures(content))
            .and_then(|captures| captures.get(1))
            .map(|value| value.as_str().to_string())
        {
            return Some(version);
        }
    }
    None
}
