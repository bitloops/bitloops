use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::host::hooks::augmentation::skill_content::DEVQL_EXPLORE_FIRST_SKILL;

pub const CANONICAL_DEVQL_SKILL_RELATIVE_PATH: &str =
    ".bitloops/skills/bitloops/devql-explore-first/SKILL.md";

pub fn canonical_repo_skill_path(repo_root: &Path) -> PathBuf {
    repo_root.join(CANONICAL_DEVQL_SKILL_RELATIVE_PATH)
}

pub fn write_managed_file(path: &Path, content: &str) -> Result<bool> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
    }

    let existing = fs::read_to_string(path).ok();
    if existing.as_deref() == Some(content) {
        return Ok(false);
    }

    fs::write(path, content).with_context(|| format!("writing {}", path.display()))?;
    Ok(true)
}

pub fn install_canonical_repo_skill(repo_root: &Path) -> Result<bool> {
    write_managed_file(
        &canonical_repo_skill_path(repo_root),
        DEVQL_EXPLORE_FIRST_SKILL,
    )
}

pub fn read_or_install_canonical_repo_skill(repo_root: &Path) -> Result<String> {
    install_canonical_repo_skill(repo_root)?;
    let path = canonical_repo_skill_path(repo_root);
    fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))
}

pub fn remove_canonical_repo_skill(repo_root: &Path) -> Result<()> {
    let path = canonical_repo_skill_path(repo_root);
    remove_managed_file(&path)?;
    prune_empty_parents(&path, &repo_root.join(".bitloops"))
}

pub fn strip_skill_frontmatter(skill: &str) -> &str {
    skill
        .splitn(3, "---")
        .nth(2)
        .map(str::trim_start)
        .unwrap_or(skill)
}

pub fn remove_managed_file(path: &Path) -> Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err).with_context(|| format!("removing {}", path.display())),
    }
}

pub fn prune_empty_parents(path: &Path, stop_at: &Path) -> Result<()> {
    let mut current = path.parent();
    while let Some(dir) = current {
        if dir == stop_at {
            break;
        }

        match fs::remove_dir(dir) {
            Ok(()) => {}
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(err) if err.kind() == std::io::ErrorKind::DirectoryNotEmpty => break,
            Err(err) => return Err(err).with_context(|| format!("removing {}", dir.display())),
        }

        current = dir.parent();
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_managed_file_is_idempotent() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir
            .path()
            .join(".agent/skills/bitloops/using-devql/SKILL.md");

        assert!(write_managed_file(&path, "hello").expect("initial write"));
        assert!(!write_managed_file(&path, "hello").expect("idempotent write"));
        assert_eq!(fs::read_to_string(&path).expect("read"), "hello");
    }

    #[test]
    fn canonical_repo_skill_path_uses_repo_root() {
        let dir = tempfile::tempdir().expect("tempdir");
        assert_eq!(
            canonical_repo_skill_path(dir.path()),
            dir.path().join(CANONICAL_DEVQL_SKILL_RELATIVE_PATH)
        );
    }

    #[test]
    fn canonical_repo_skill_is_written_and_rewritten_from_builtin_content() {
        let dir = tempfile::tempdir().expect("tempdir");
        assert!(install_canonical_repo_skill(dir.path()).expect("initial install"));

        let path = canonical_repo_skill_path(dir.path());
        assert_eq!(
            fs::read_to_string(&path).expect("read canonical skill"),
            DEVQL_EXPLORE_FIRST_SKILL
        );

        fs::write(&path, "mutated").expect("mutate canonical skill");
        assert!(install_canonical_repo_skill(dir.path()).expect("rewrite canonical skill"));
        assert_eq!(
            fs::read_to_string(&path).expect("read rewritten canonical skill"),
            DEVQL_EXPLORE_FIRST_SKILL
        );
    }

    #[test]
    fn remove_canonical_repo_skill_prunes_managed_subtree_but_keeps_bitloops_root() {
        let dir = tempfile::tempdir().expect("tempdir");
        install_canonical_repo_skill(dir.path()).expect("install canonical skill");
        remove_canonical_repo_skill(dir.path()).expect("remove canonical skill");

        assert!(!canonical_repo_skill_path(dir.path()).exists());
        assert!(dir.path().join(".bitloops").exists());
        assert!(!dir.path().join(".bitloops/skills").exists());
    }

    #[test]
    fn strip_skill_frontmatter_returns_body() {
        let body = strip_skill_frontmatter(DEVQL_EXPLORE_FIRST_SKILL);
        assert!(!body.starts_with("---"));
        assert!(body.contains("primary discovery tool"));
    }

    #[test]
    fn prune_empty_parents_stops_at_boundary() {
        let dir = tempfile::tempdir().expect("tempdir");
        let stop_at = dir.path().join(".agent");
        let file = stop_at.join("skills/bitloops/using-devql/SKILL.md");
        write_managed_file(&file, "hello").expect("write managed file");

        remove_managed_file(&file).expect("remove file");
        prune_empty_parents(&file, &stop_at).expect("prune empty parents");

        assert!(stop_at.exists(), "stop boundary should be preserved");
        assert!(
            !stop_at.join("skills").exists(),
            "empty managed subtree should be pruned"
        );
    }

    #[test]
    fn prune_empty_parents_preserves_non_empty_directories() {
        let dir = tempfile::tempdir().expect("tempdir");
        let stop_at = dir.path().join(".agent");
        let managed = stop_at.join("skills/bitloops/using-devql/SKILL.md");
        let sibling = stop_at.join("skills/bitloops/other-skill/SKILL.md");
        write_managed_file(&managed, "hello").expect("write managed file");
        write_managed_file(&sibling, "keep").expect("write sibling");

        remove_managed_file(&managed).expect("remove managed file");
        prune_empty_parents(&managed, &stop_at).expect("prune empty parents");

        assert!(stop_at.join("skills/bitloops").exists());
        assert!(sibling.exists());
    }
}
