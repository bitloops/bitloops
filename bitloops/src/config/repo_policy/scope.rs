use std::collections::HashSet;
use std::fs;
use std::path::Path;

use anyhow::{Context, Result, bail};
use serde_json::Value;

use super::discovery::resolve_import_path;
use super::types::{RepoPolicyExclusionFileReference, RepoPolicyScopeExclusions};

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

pub fn parse_exclusion_patterns(content: &str) -> Vec<String> {
    content
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(str::to_string)
        .collect()
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
