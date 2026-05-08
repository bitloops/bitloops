use std::path::{Path, PathBuf};

use anyhow::Result;

use crate::adapters::agents::skill_install::{
    install_canonical_repo_skill, prune_empty_parents, read_or_install_canonical_repo_skill,
    remove_canonical_repo_skill, remove_managed_file, write_managed_file,
};

pub const CLAUDE_CODE_SKILL_RELATIVE_PATH: &str =
    ".claude/skills/bitloops/devql-explore-first/SKILL.md";
pub const LEGACY_CLAUDE_CODE_SKILL_RELATIVE_PATH: &str =
    ".claude/skills/bitloops/using-devql/SKILL.md";

pub fn repo_skill_path(repo_root: &Path) -> PathBuf {
    repo_root.join(CLAUDE_CODE_SKILL_RELATIVE_PATH)
}

fn legacy_repo_skill_path(repo_root: &Path) -> PathBuf {
    repo_root.join(LEGACY_CLAUDE_CODE_SKILL_RELATIVE_PATH)
}

pub fn install_repo_skill(repo_root: &Path) -> Result<bool> {
    let canonical_changed = install_canonical_repo_skill(repo_root)?;
    let skill = read_or_install_canonical_repo_skill(repo_root)?;
    let path = repo_skill_path(repo_root);
    let changed = write_managed_file(&path, &skill)?;
    let legacy_path = legacy_repo_skill_path(repo_root);
    let removed_legacy = legacy_path.exists();
    remove_managed_file(&legacy_path)?;
    prune_empty_parents(&legacy_path, &repo_root.join(".claude"))?;

    Ok(changed || removed_legacy || canonical_changed)
}

pub fn uninstall_repo_skill(repo_root: &Path) -> Result<()> {
    let path = repo_skill_path(repo_root);
    remove_managed_file(&path)?;
    prune_empty_parents(&path, &repo_root.join(".claude"))?;

    let legacy_path = legacy_repo_skill_path(repo_root);
    remove_managed_file(&legacy_path)?;
    prune_empty_parents(&legacy_path, &repo_root.join(".claude"))?;

    remove_canonical_repo_skill(repo_root)
}

#[cfg(test)]
mod tests {
    use crate::adapters::agents::skill_install::read_or_install_canonical_repo_skill;

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
        assert!(!legacy_repo_skill_path(dir.path()).exists());

        uninstall_repo_skill(dir.path()).expect("uninstall");
        assert!(!skill_path.exists());
        assert!(dir.path().join(".claude").exists());
    }

    #[test]
    fn install_repo_skill_replaces_legacy_using_devql_skill() {
        let dir = tempfile::tempdir().expect("tempdir");
        let legacy_path = legacy_repo_skill_path(dir.path());
        std::fs::create_dir_all(legacy_path.parent().expect("legacy parent"))
            .expect("create legacy parent");
        std::fs::write(&legacy_path, "legacy").expect("write legacy skill");

        assert!(install_repo_skill(dir.path()).expect("install"));

        let skill_path = repo_skill_path(dir.path());
        assert!(skill_path.exists());
        assert!(!legacy_path.exists());
        assert_eq!(
            std::fs::read_to_string(skill_path).expect("read skill"),
            read_or_install_canonical_repo_skill(dir.path()).expect("read canonical skill")
        );
    }
}
