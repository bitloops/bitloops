use std::path::{Path, PathBuf};

use anyhow::Result;

use crate::adapters::agents::skill_install::{
    install_canonical_repo_skill, prune_empty_parents, read_or_install_canonical_repo_skill,
    remove_canonical_repo_skill, remove_managed_file, write_managed_file,
};

pub const OPEN_CODE_SKILL_RELATIVE_PATH: &str =
    ".opencode/skills/bitloops/devql-explore-first/SKILL.md";

pub fn repo_skill_path(repo_root: &Path) -> PathBuf {
    repo_root.join(OPEN_CODE_SKILL_RELATIVE_PATH)
}

pub fn install_repo_skill(repo_root: &Path) -> Result<bool> {
    let canonical_changed = install_canonical_repo_skill(repo_root)?;
    let skill = read_or_install_canonical_repo_skill(repo_root)?;
    let path = repo_skill_path(repo_root);
    let changed = write_managed_file(&path, &skill)?;
    Ok(changed || canonical_changed)
}

pub fn uninstall_repo_skill(repo_root: &Path) -> Result<()> {
    let path = repo_skill_path(repo_root);
    remove_managed_file(&path)?;
    prune_empty_parents(&path, &repo_root.join(".opencode"))?;
    remove_canonical_repo_skill(repo_root)
}

#[cfg(test)]
mod tests {
    use crate::adapters::agents::skill_install::read_or_install_canonical_repo_skill;

    use super::*;

    #[test]
    fn repo_skill_path_points_to_open_code_skill_tree() {
        let repo_root = Path::new("/repo");
        assert_eq!(
            repo_skill_path(repo_root),
            PathBuf::from("/repo").join(OPEN_CODE_SKILL_RELATIVE_PATH)
        );
    }

    #[test]
    fn install_and_uninstall_repo_skill_prune_empty_directories() {
        let dir = tempfile::tempdir().expect("tempdir");
        let repo_root = dir.path();

        assert!(install_repo_skill(repo_root).expect("install should succeed"));
        let path = repo_skill_path(repo_root);
        assert!(path.exists(), "skill file should exist after install");
        assert_eq!(
            std::fs::read_to_string(&path).expect("read skill"),
            read_or_install_canonical_repo_skill(repo_root).expect("read canonical skill")
        );
        assert!(
            path.to_string_lossy()
                .contains(OPEN_CODE_SKILL_RELATIVE_PATH)
        );

        uninstall_repo_skill(repo_root).expect("uninstall should succeed");

        assert!(!path.exists(), "skill file should be removed");
        assert!(
            repo_root.join(".opencode").exists(),
            ".opencode should be preserved"
        );
        assert!(
            !repo_root.join(".opencode/skills").exists(),
            "skills directory should be pruned when empty"
        );
    }

    #[test]
    fn uninstall_repo_skill_preserves_non_empty_parent_directories() {
        let dir = tempfile::tempdir().expect("tempdir");
        let repo_root = dir.path();

        install_repo_skill(repo_root).expect("install should succeed");
        let sibling_file = repo_root.join(".opencode/skills/keep.txt");
        std::fs::write(&sibling_file, "keep").expect("write sibling file");

        uninstall_repo_skill(repo_root).expect("uninstall should succeed");

        assert!(
            sibling_file.exists(),
            "uninstall should not remove unrelated files"
        );
        assert!(
            repo_root.join(".opencode/skills").exists(),
            "non-empty skills directory should be preserved"
        );
        assert!(
            !repo_root.join(".opencode/skills/bitloops").exists(),
            "managed bitloops directory should still be pruned when empty"
        );
        assert!(
            !repo_root
                .join(".opencode/skills/bitloops/devql-explore-first")
                .exists(),
            "managed skill directory should be removed"
        );
    }
}
