use std::path::{Path, PathBuf};

use anyhow::Result;

use crate::adapters::agents::skill_install::{
    prune_empty_parents, remove_managed_file, strip_skill_frontmatter, write_managed_file,
};
use crate::host::hooks::augmentation::skill_content::DEVQL_EXPLORE_FIRST_SKILL;

pub const CURSOR_RULE_RELATIVE_PATH: &str = ".cursor/rules/bitloops-devql-explore-first.mdc";
pub const LEGACY_CURSOR_RULE_RELATIVE_PATH: &str = ".cursor/rules/bitloops-using-devql.mdc";

pub fn repo_rule_path(repo_root: &Path) -> PathBuf {
    repo_root.join(CURSOR_RULE_RELATIVE_PATH)
}

fn legacy_repo_rule_path(repo_root: &Path) -> PathBuf {
    repo_root.join(LEGACY_CURSOR_RULE_RELATIVE_PATH)
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
    let changed = write_managed_file(&path, &content)?;

    let legacy_path = legacy_repo_rule_path(repo_root);
    let removed_legacy = legacy_path.exists();
    remove_managed_file(&legacy_path)?;
    prune_empty_parents(&legacy_path, &repo_root.join(".cursor"))?;

    Ok(changed || removed_legacy)
}

pub fn uninstall_repo_rule(repo_root: &Path) -> Result<()> {
    let path = repo_rule_path(repo_root);
    remove_managed_file(&path)?;
    prune_empty_parents(&path, &repo_root.join(".cursor"))?;

    let legacy_path = legacy_repo_rule_path(repo_root);
    remove_managed_file(&legacy_path)?;
    prune_empty_parents(&legacy_path, &repo_root.join(".cursor"))
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

    #[test]
    fn install_repo_rule_replaces_legacy_using_devql_rule() {
        let dir = tempfile::tempdir().expect("tempdir");
        let legacy_path = legacy_repo_rule_path(dir.path());
        std::fs::create_dir_all(legacy_path.parent().expect("legacy parent"))
            .expect("create legacy parent");
        std::fs::write(&legacy_path, "legacy rule content").expect("write legacy rule");

        assert!(install_repo_rule(dir.path()).expect("install"));

        let rule_path = repo_rule_path(dir.path());
        assert!(rule_path.exists());
        assert!(!legacy_path.exists());
        assert!(
            std::fs::read_to_string(rule_path)
                .expect("read rule")
                .contains("alwaysApply: true")
        );
    }
}
