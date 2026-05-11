use std::path::{Path, PathBuf};

use anyhow::Result;

use crate::adapters::agents::skill_install::{
    prune_empty_parents, remove_managed_file, write_managed_file,
};
use crate::host::hooks::augmentation::skill_content::DEVQL_EXPLORE_FIRST_SKILL;

pub const CLAUDE_CODE_SKILL_RELATIVE_PATH: &str = ".claude/skills/devql-explore-first/SKILL.md";

pub fn repo_skill_path(repo_root: &Path) -> PathBuf {
    repo_root.join(CLAUDE_CODE_SKILL_RELATIVE_PATH)
}

pub fn install_repo_skill(repo_root: &Path) -> Result<bool> {
    let path = repo_skill_path(repo_root);
    write_managed_file(&path, DEVQL_EXPLORE_FIRST_SKILL)
}

pub fn uninstall_repo_skill(repo_root: &Path) -> Result<()> {
    let path = repo_skill_path(repo_root);
    remove_managed_file(&path)?;
    prune_empty_parents(&path, &repo_root.join(".claude"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repo_skill_path_uses_repo_root() {
        let dir = tempfile::tempdir().expect("tempdir");
        assert_eq!(
            repo_skill_path(dir.path()),
            dir.path().join(CLAUDE_CODE_SKILL_RELATIVE_PATH)
        );
    }

    #[test]
    fn install_and_uninstall_repo_skill_are_idempotent() {
        let dir = tempfile::tempdir().expect("tempdir");
        assert!(install_repo_skill(dir.path()).expect("install"));
        assert!(!install_repo_skill(dir.path()).expect("idempotent install"));
        let skill_path = repo_skill_path(dir.path());
        assert!(skill_path.exists());

        uninstall_repo_skill(dir.path()).expect("uninstall");
        assert!(!skill_path.exists());
        assert!(dir.path().join(".claude").exists());
    }
}
