use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::host::hooks::augmentation::skill_content::USING_DEVQL_SKILL;

pub fn repo_skill_path(repo_root: &Path) -> PathBuf {
    repo_root
        .join(".claude")
        .join("skills")
        .join("bitloops")
        .join("using-devql")
        .join("SKILL.md")
}

pub fn install_repo_skill(repo_root: &Path) -> Result<bool> {
    let path = repo_skill_path(repo_root);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
    }

    let existing = fs::read_to_string(&path).ok();
    if existing.as_deref() == Some(USING_DEVQL_SKILL) {
        return Ok(false);
    }

    fs::write(&path, USING_DEVQL_SKILL).with_context(|| format!("writing {}", path.display()))?;
    Ok(true)
}

pub fn uninstall_repo_skill(repo_root: &Path) -> Result<()> {
    let path = repo_skill_path(repo_root);
    match fs::remove_file(&path) {
        Ok(()) => {}
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
        Err(err) => return Err(err).with_context(|| format!("removing {}", path.display())),
    }

    for dir in [
        path.parent(),
        path.parent().and_then(|p| p.parent()),
        path.parent()
            .and_then(|p| p.parent())
            .and_then(|p| p.parent()),
    ]
    .into_iter()
    .flatten()
    {
        match fs::remove_dir(dir) {
            Ok(()) => {}
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(err) if err.kind() == std::io::ErrorKind::DirectoryNotEmpty => {}
            Err(err) => return Err(err).with_context(|| format!("removing {}", dir.display())),
        }
    }

    Ok(())
}
