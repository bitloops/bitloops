use std::path::Path;

use anyhow::{Context, Result, bail};
use regex::Regex;

use crate::config::{discover_repo_policy_optional, resolve_repo_policy_scope_exclusions};

#[derive(Debug, Clone)]
pub(crate) struct RepoExclusionMatcher {
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
    let anchored_to_root = normalized.starts_with("./") || normalized.starts_with('/');
    while normalized.starts_with("./") {
        normalized = normalized[2..].to_string();
    }
    while normalized.starts_with('/') {
        normalized.remove(0);
    }
    if normalized.ends_with('/') {
        normalized.push_str("**");
    }
    if anchored_to_root && !normalized.is_empty() {
        normalized.insert(0, '/');
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

fn compile_exclusion_pattern(pattern: &str) -> Result<Regex> {
    let (anchored_to_root, pattern) = split_root_anchor(pattern);
    if pattern.is_empty() {
        return Regex::new(r"^$")
            .with_context(|| format!("compiling exclusion pattern `{pattern}`"));
    }

    if is_literal_pattern(pattern) {
        let escaped = regex::escape(pattern);
        let prefix = if !anchored_to_root && is_basename_pattern(pattern) {
            "(?:.*/)?"
        } else {
            ""
        };
        let regex = format!("^{prefix}{escaped}(?:/.*)?$");
        return Regex::new(&regex)
            .with_context(|| format!("compiling exclusion pattern `{pattern}`"));
    }

    let mut regex = String::with_capacity(pattern.len() * 2 + 8);
    regex.push('^');
    if !anchored_to_root
        && (is_basename_pattern(pattern) || is_single_dir_descendant_pattern(pattern))
    {
        regex.push_str("(?:.*/)?");
    }
    let chars: Vec<char> = pattern.chars().collect();
    let mut index = 0usize;
    while index < chars.len() {
        if chars[index] == '*' {
            if index + 1 < chars.len() && chars[index + 1] == '*' {
                while index + 1 < chars.len() && chars[index + 1] == '*' {
                    index += 1;
                }
                if index + 1 < chars.len() && chars[index + 1] == '/' {
                    regex.push_str("(?:.*/)?");
                    index += 2;
                    continue;
                }
                regex.push_str(".*");
                index += 1;
                continue;
            }
            regex.push_str("[^/]*");
            index += 1;
            continue;
        }

        let ch = chars[index];
        match ch {
            '?' => regex.push_str("[^/]"),
            '.' | '+' | '(' | ')' | '|' | '^' | '$' | '{' | '}' | '[' | ']' | '\\' => {
                regex.push('\\');
                regex.push(ch);
            }
            other => regex.push(other),
        }
        index += 1;
    }
    if should_match_folder_descendants(pattern) {
        regex.push_str("(?:/.*)?");
    }
    regex.push('$');
    Regex::new(&regex).with_context(|| format!("compiling exclusion pattern `{pattern}`"))
}

fn is_literal_pattern(pattern: &str) -> bool {
    !pattern.contains('*') && !pattern.contains('?')
}

fn should_match_folder_descendants(pattern: &str) -> bool {
    if pattern.ends_with("/**") {
        return false;
    }
    let Some(last_segment) = pattern.rsplit('/').next() else {
        return false;
    };
    !last_segment.is_empty() && !last_segment.contains('*') && !last_segment.contains('?')
}

fn is_basename_pattern(pattern: &str) -> bool {
    !pattern.contains('/')
}

fn is_single_dir_descendant_pattern(pattern: &str) -> bool {
    let Some(base) = pattern.strip_suffix("/**") else {
        return false;
    };
    !base.contains('/')
}

fn split_root_anchor(pattern: &str) -> (bool, &str) {
    if let Some(stripped) = pattern.strip_prefix('/') {
        return (true, stripped);
    }
    (false, pattern)
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

    #[test]
    fn matcher_excludes_plain_folder_patterns() {
        let repo = tempfile::tempdir().expect("temp dir");
        std::fs::create_dir_all(repo.path().join(".git")).expect("create .git");
        std::fs::write(
            repo.path().join(REPO_POLICY_FILE_NAME),
            r#"
[scope]
exclude = ["docs"]
"#,
        )
        .expect("write policy");

        let matcher = RepoExclusionMatcher::discover(repo.path()).expect("discover matcher");
        assert!(matcher.excludes_repo_relative_path("docs"));
        assert!(matcher.excludes_repo_relative_path("docs/readme.md"));
        assert!(matcher.excludes_repo_relative_path("src/docs/readme.md"));
    }

    #[test]
    fn matcher_double_star_folder_pattern_matches_root_and_nested_dirs() {
        let repo = tempfile::tempdir().expect("temp dir");
        std::fs::create_dir_all(repo.path().join(".git")).expect("create .git");
        std::fs::write(
            repo.path().join(REPO_POLICY_FILE_NAME),
            r#"
[scope]
exclude = ["**/third_party/**"]
"#,
        )
        .expect("write policy");

        let matcher = RepoExclusionMatcher::discover(repo.path()).expect("discover matcher");
        assert!(matcher.excludes_repo_relative_path("third_party/lib.rs"));
        assert!(matcher.excludes_repo_relative_path("packages/api/third_party/lib.rs"));
        assert!(!matcher.excludes_repo_relative_path("packages/api/thirdparty/lib.rs"));
    }

    #[test]
    fn matcher_regex_escaping_keeps_special_chars_literal() {
        let pattern = compile_exclusion_pattern("docs.v1").expect("compile pattern");
        assert!(pattern.is_match("docs.v1/readme.md"));
        assert!(!pattern.is_match("docsXv1/readme.md"));
    }

    #[test]
    fn matcher_supports_gitignore_like_folder_pattern_variants() {
        let repo = tempfile::tempdir().expect("temp dir");
        std::fs::create_dir_all(repo.path().join(".git")).expect("create .git");
        std::fs::write(
            repo.path().join(REPO_POLICY_FILE_NAME),
            r#"
[scope]
exclude = ["oti", "/oti", "./oti", "*/oti"]
"#,
        )
        .expect("write policy");

        let matcher = RepoExclusionMatcher::discover(repo.path()).expect("discover matcher");
        assert!(matcher.excludes_repo_relative_path("oti"));
        assert!(matcher.excludes_repo_relative_path("oti/file.rs"));
        assert!(matcher.excludes_repo_relative_path("src/oti"));
        assert!(matcher.excludes_repo_relative_path("src/oti/file.rs"));
        assert!(matcher.excludes_repo_relative_path("src/nested/oti/file.rs"));
    }

    #[test]
    fn matcher_applies_gitignore_basename_rules_from_exclude_from_file() {
        let repo = tempfile::tempdir().expect("temp dir");
        std::fs::create_dir_all(repo.path().join(".git")).expect("create .git");
        std::fs::write(
            repo.path().join(REPO_POLICY_FILE_NAME),
            r#"
[scope]
exclude_from = [".gitignore"]
"#,
        )
        .expect("write policy");
        std::fs::write(
            repo.path().join(".gitignore"),
            r#"
fasoules/
*.bak
"#,
        )
        .expect("write .gitignore");

        let matcher = RepoExclusionMatcher::discover(repo.path()).expect("discover matcher");
        assert!(matcher.excludes_repo_relative_path("fasoules/file.ts"));
        assert!(matcher.excludes_repo_relative_path("src/fasoules/file.ts"));
        assert!(matcher.excludes_repo_relative_path("src/tmp.bak"));
        assert!(!matcher.excludes_repo_relative_path("src/fasoules_keep/file.ts"));
    }

    #[test]
    fn matcher_respects_root_anchored_prefixes() {
        let repo = tempfile::tempdir().expect("temp dir");
        std::fs::create_dir_all(repo.path().join(".git")).expect("create .git");
        std::fs::write(
            repo.path().join(REPO_POLICY_FILE_NAME),
            r#"
[scope]
exclude = ["/oti", "./pies.ts"]
"#,
        )
        .expect("write policy");

        let matcher = RepoExclusionMatcher::discover(repo.path()).expect("discover matcher");
        assert!(matcher.excludes_repo_relative_path("oti/file.ts"));
        assert!(!matcher.excludes_repo_relative_path("src/oti/file.ts"));
        assert!(matcher.excludes_repo_relative_path("pies.ts"));
        assert!(!matcher.excludes_repo_relative_path("src/pies.ts"));
    }
}
