use std::path::{Component, Path, PathBuf};

use anyhow::{Context, Result, bail};
use regex::Regex;

use crate::config::{discover_repo_policy_optional, resolve_repo_policy_scope_exclusions};

#[derive(Debug, Clone)]
pub(crate) struct RepoExclusionMatcher {
    policy_root: PathBuf,
    #[cfg_attr(not(test), allow(dead_code))]
    patterns: Vec<String>,
    compiled_patterns: Vec<Regex>,
}

impl RepoExclusionMatcher {
    pub(crate) fn discover(repo_root: &Path) -> Result<Self> {
        let policy = discover_repo_policy_optional(repo_root)
            .with_context(|| format!("loading repo policy from {}", repo_root.display()))?;
        let policy_root = policy.root.unwrap_or_else(|| repo_root.to_path_buf());
        let policy_root = policy_root.canonicalize().unwrap_or(policy_root);
        let exclusions = resolve_repo_policy_scope_exclusions(&policy.scope, &policy_root)
            .context("resolving [scope] exclusions for DevQL runtime")?;

        let mut patterns = exclusions.exclude;
        for reference in exclusions.referenced_files {
            let resolved_path = reference
                .resolved_path
                .canonicalize()
                .unwrap_or(reference.resolved_path.clone());
            if !resolved_path.starts_with(&policy_root) {
                bail!(
                    "scope.exclude_from path `{}` resolves outside repo-policy root {}",
                    reference.configured_path,
                    policy_root.display()
                );
            }
            patterns.extend(reference.patterns);
        }
        patterns = normalize_patterns(patterns);
        let compiled_patterns = patterns
            .iter()
            .map(|pattern| compile_exclusion_pattern(pattern))
            .collect::<Result<Vec<_>>>()?;

        Ok(Self {
            policy_root,
            patterns,
            compiled_patterns,
        })
    }

    #[cfg(test)]
    pub(crate) fn has_rules(&self) -> bool {
        !self.compiled_patterns.is_empty()
    }

    pub(crate) fn excludes_repo_relative_path(&self, path: &str) -> bool {
        if self.compiled_patterns.is_empty() {
            return false;
        }
        let normalized_path = normalize_relative_path(path);
        if normalized_path.is_empty() {
            return false;
        }
        self.compiled_patterns
            .iter()
            .any(|pattern| pattern.is_match(&normalized_path))
    }

    pub(crate) fn excludes_path(&self, path: &Path) -> bool {
        let normalized = if path.is_absolute() {
            let normalized_absolute = normalize_lexical_path(path);
            if let Ok(relative) = normalized_absolute.strip_prefix(&self.policy_root) {
                relative.to_string_lossy().to_string()
            } else {
                return false;
            }
        } else {
            path.to_string_lossy().to_string()
        };

        self.excludes_repo_relative_path(&normalized)
    }

    #[cfg(test)]
    pub(crate) fn patterns(&self) -> &[String] {
        &self.patterns
    }
}

pub(crate) fn load_repo_exclusion_matcher(repo_root: &Path) -> Result<RepoExclusionMatcher> {
    RepoExclusionMatcher::discover(repo_root)
}

fn normalize_patterns(patterns: Vec<String>) -> Vec<String> {
    let mut normalized = patterns
        .into_iter()
        .map(|pattern| normalize_pattern(&pattern))
        .filter(|pattern| !pattern.is_empty())
        .collect::<Vec<_>>();
    normalized.sort();
    normalized.dedup();
    normalized
}

fn normalize_pattern(pattern: &str) -> String {
    let mut normalized = pattern.trim().replace('\\', "/");
    while normalized.starts_with("./") {
        normalized = normalized[2..].to_string();
    }
    while normalized.starts_with('/') {
        normalized.remove(0);
    }
    if normalized.ends_with('/') {
        normalized.push_str("**");
    }
    normalized
}

fn normalize_relative_path(path: &str) -> String {
    let mut normalized = path.trim().replace('\\', "/");
    while normalized.starts_with("./") {
        normalized = normalized[2..].to_string();
    }
    while normalized.starts_with('/') {
        normalized.remove(0);
    }

    let mut segments: Vec<&str> = Vec::new();
    for segment in normalized.split('/') {
        match segment {
            "" | "." => {}
            ".." => {
                segments.pop();
            }
            value => segments.push(value),
        }
    }

    segments.join("/")
}

fn normalize_lexical_path(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

fn compile_exclusion_pattern(pattern: &str) -> Result<Regex> {
    let mut regex = String::with_capacity(pattern.len() * 2 + 8);
    regex.push('^');
    let chars: Vec<char> = pattern.chars().collect();
    let mut index = 0usize;
    while index < chars.len() {
        let ch = chars[index];
        match ch {
            '*' => {
                if index + 1 < chars.len() && chars[index + 1] == '*' {
                    while index + 1 < chars.len() && chars[index + 1] == '*' {
                        index += 1;
                    }
                    regex.push_str(".*");
                } else {
                    regex.push_str("[^/]*");
                }
            }
            '?' => regex.push_str("[^/]"),
            '.' | '+' | '(' | ')' | '|' | '^' | '$' | '{' | '}' | '[' | ']' | '\\' => {
                regex.push('\\');
                regex.push(ch);
            }
            other => regex.push(other),
        }
        index += 1;
    }
    regex.push('$');
    Regex::new(&regex).with_context(|| format!("compiling exclusion pattern `{pattern}`"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::REPO_POLICY_FILE_NAME;

    #[test]
    fn matcher_combines_inline_and_exclude_from_patterns() {
        let repo = tempfile::tempdir().expect("temp dir");
        std::fs::create_dir_all(repo.path().join(".git")).expect("create .git");
        std::fs::write(
            repo.path().join(REPO_POLICY_FILE_NAME),
            r#"
[scope]
exclude = ["docs/**"]
exclude_from = [".bitloopsignore"]
"#,
        )
        .expect("write policy");
        std::fs::write(
            repo.path().join(".bitloopsignore"),
            "tmp/**\n# comment\n\nvendor/**\n",
        )
        .expect("write ignore");

        let matcher = RepoExclusionMatcher::discover(repo.path()).expect("discover matcher");
        assert!(matcher.has_rules());
        assert_eq!(
            matcher.patterns(),
            &[
                "docs/**".to_string(),
                "tmp/**".to_string(),
                "vendor/**".to_string()
            ]
        );
        assert!(matcher.excludes_repo_relative_path("docs/guide.md"));
        assert!(matcher.excludes_repo_relative_path("tmp/file.txt"));
        assert!(matcher.excludes_repo_relative_path("vendor/a/b.rs"));
        assert!(!matcher.excludes_repo_relative_path("src/lib.rs"));
    }

    #[test]
    fn matcher_fails_for_missing_exclude_from_file() {
        let repo = tempfile::tempdir().expect("temp dir");
        std::fs::create_dir_all(repo.path().join(".git")).expect("create .git");
        std::fs::write(
            repo.path().join(REPO_POLICY_FILE_NAME),
            r#"
[scope]
exclude_from = [".bitloopsignore"]
"#,
        )
        .expect("write policy");

        let err = RepoExclusionMatcher::discover(repo.path()).expect_err("missing file should err");
        let err_chain = format!("{err:#}");
        assert!(
            err_chain.contains("scope.exclude_from"),
            "expected scope.exclude_from read error, got: {err_chain}"
        );
    }
}
