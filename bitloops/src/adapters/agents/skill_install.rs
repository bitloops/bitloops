use std::fs;
use std::path::Path;

use anyhow::{Context, Result};

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
