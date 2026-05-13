use std::path::{Path, PathBuf};

use anyhow::Result;

use crate::adapters::agents::skill_install::{
    prune_empty_parents, remove_managed_file, strip_skill_frontmatter, write_managed_file,
};
use crate::host::hooks::augmentation::skill_content::DEVQL_EXPLORE_FIRST_SKILL;

pub const CURSOR_RULE_RELATIVE_PATH: &str = ".cursor/rules/devql-explore-first.mdc";

pub fn repo_rule_path(repo_root: &Path) -> PathBuf {
    repo_root.join(CURSOR_RULE_RELATIVE_PATH)
}

fn cursor_rule_content() -> String {
    let body = strip_skill_frontmatter(DEVQL_EXPLORE_FIRST_SKILL).trim();
    format!(
        "---\n\
description: Use DevQL before codebase exploration, symbol lookup, or any source-file reads. DevQL is the primary discovery tool — use `bitloops devql query` whenever locating symbols, files, tests, or implementations.\n\
alwaysApply: true\n\
---\n\n\
{body}\n"
    )
}

pub fn install_repo_rule(repo_root: &Path) -> Result<bool> {
    let path = repo_rule_path(repo_root);
    let content = cursor_rule_content();
    write_managed_file(&path, &content)
}

pub fn uninstall_repo_rule(repo_root: &Path) -> Result<()> {
    let path = repo_rule_path(repo_root);
    remove_managed_file(&path)?;
    prune_empty_parents(&path, &repo_root.join(".cursor"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repo_rule_path_uses_repo_root() {
        let dir = tempfile::tempdir().expect("tempdir");
        assert_eq!(
            repo_rule_path(dir.path()),
            dir.path().join(CURSOR_RULE_RELATIVE_PATH)
        );
    }

    #[test]
    fn cursor_rule_content_uses_frontmatter_and_devql_body() {
        let content = cursor_rule_content();
        assert!(content.starts_with("---\n"));
        assert!(content.contains("alwaysApply: true"));
        assert!(content.contains("primary discovery tool"));
        assert!(content.contains("bitloops devql query"));
        assert!(content.contains("searchMode: LEXICAL"));
        assert!(content.contains("fall back when DevQL fails"));
        assert!(content.contains("symbolFqn"));
        assert!(!content.contains("fuzzyName"));
        assert!(!content.contains("naturalLanguage"));
        assert!(!content.contains("semanticQuery"));
    }

    #[test]
    fn install_and_uninstall_repo_rule_are_idempotent() {
        let dir = tempfile::tempdir().expect("tempdir");
        assert!(install_repo_rule(dir.path()).expect("install"));
        assert!(!install_repo_rule(dir.path()).expect("idempotent install"));
        let rule_path = repo_rule_path(dir.path());
        assert!(rule_path.exists());

        uninstall_repo_rule(dir.path()).expect("uninstall");
        assert!(!rule_path.exists());
        assert!(dir.path().join(".cursor").exists());
    }
}
