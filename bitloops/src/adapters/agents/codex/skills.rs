use std::path::PathBuf;

use anyhow::{Context, Result, anyhow};

use crate::adapters::agents::skill_install::{
    prune_empty_parents, remove_managed_file, write_managed_file,
};
use crate::host::hooks::augmentation::skill_content::USING_DEVQL_SKILL;

pub fn codex_global_skill_path() -> Result<PathBuf> {
    let home = dirs::home_dir().ok_or_else(|| anyhow!("failed to resolve home directory"))?;
    Ok(home
        .join(".agents")
        .join("skills")
        .join("bitloops")
        .join("using-devql")
        .join("SKILL.md"))
}

pub fn install_global_skill() -> Result<bool> {
    let path = codex_global_skill_path()?;
    write_managed_file(&path, USING_DEVQL_SKILL)
}

pub fn uninstall_global_skill() -> Result<()> {
    let path = codex_global_skill_path()?;
    let stop_at = path
        .parent()
        .and_then(|parent| parent.parent())
        .and_then(|parent| parent.parent())
        .context("failed to resolve Codex global skills root")?
        .to_path_buf();
    remove_managed_file(&path)?;
    prune_empty_parents(&path, &stop_at)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::process_state::with_env_vars;

    #[test]
    fn codex_global_skill_path_uses_home_directory() {
        let dir = tempfile::tempdir().expect("tempdir");
        with_env_vars(
            &[
                ("HOME", Some(dir.path().to_string_lossy().as_ref())),
                ("USERPROFILE", Some(dir.path().to_string_lossy().as_ref())),
            ],
            || {
                assert_eq!(
                    codex_global_skill_path().expect("path"),
                    dir.path()
                        .join(".agents")
                        .join("skills")
                        .join("bitloops")
                        .join("using-devql")
                        .join("SKILL.md")
                );
            },
        );
    }

    #[test]
    fn install_and_uninstall_global_skill_are_idempotent() {
        let dir = tempfile::tempdir().expect("tempdir");
        with_env_vars(
            &[
                ("HOME", Some(dir.path().to_string_lossy().as_ref())),
                ("USERPROFILE", Some(dir.path().to_string_lossy().as_ref())),
            ],
            || {
                assert!(install_global_skill().expect("install"));
                assert!(!install_global_skill().expect("idempotent install"));
                let skill_path = codex_global_skill_path().expect("path");
                assert!(skill_path.exists());

                uninstall_global_skill().expect("uninstall");
                assert!(!skill_path.exists());
                assert!(dir.path().join(".agents/skills").exists());
            },
        );
    }
}
