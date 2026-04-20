use std::path::{Path, PathBuf};

use anyhow::Result;

use crate::adapters::agents::skill_install::{
    prune_empty_parents, remove_managed_file, write_managed_file,
};
use crate::host::hooks::augmentation::skill_content::USING_DEVQL_SKILL;

pub const CLAUDE_CODE_SKILL_RELATIVE_PATH: &str = ".claude/skills/bitloops/using-devql/SKILL.md";

pub fn repo_skill_path(repo_root: &Path) -> PathBuf {
    repo_root.join(CLAUDE_CODE_SKILL_RELATIVE_PATH)
}

pub fn install_repo_skill(repo_root: &Path) -> Result<bool> {
    let path = repo_skill_path(repo_root);
    write_managed_file(&path, USING_DEVQL_SKILL)
}

pub fn uninstall_repo_skill(repo_root: &Path) -> Result<()> {
    let path = repo_skill_path(repo_root);
    remove_managed_file(&path)?;
    prune_empty_parents(&path, &repo_root.join(".claude"))
}
