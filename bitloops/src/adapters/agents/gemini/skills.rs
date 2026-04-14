use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::adapters::agents::skill_install::{
    prune_empty_parents, remove_managed_file, write_managed_file,
};
use crate::host::hooks::augmentation::skill_content::USING_DEVQL_SKILL;

const GEMINI_DIR_NAME: &str = ".gemini";
const SKILLS_DIR_NAME: &str = "skills";
const BITLOOPS_DIR_NAME: &str = "bitloops";
const USING_DEVQL_DIR_NAME: &str = "using-devql";
const SKILL_FILE_NAME: &str = "SKILL.md";
const GEMINI_MD_FILE_NAME: &str = "GEMINI.md";

const MANAGED_BLOCK_START: &str = "<!-- bitloops-managed-start -->";
const MANAGED_BLOCK_END: &str = "<!-- bitloops-managed-end -->";
const MANAGED_IMPORT_LINE: &str = "@./.gemini/skills/bitloops/using-devql/SKILL.md";

pub fn repo_skill_path(repo_root: &Path) -> PathBuf {
    repo_root
        .join(GEMINI_DIR_NAME)
        .join(SKILLS_DIR_NAME)
        .join(BITLOOPS_DIR_NAME)
        .join(USING_DEVQL_DIR_NAME)
        .join(SKILL_FILE_NAME)
}

pub fn gemini_md_path(repo_root: &Path) -> PathBuf {
    repo_root.join(GEMINI_MD_FILE_NAME)
}

pub fn install_repo_skill(repo_root: &Path) -> Result<bool> {
    let skill_path = repo_skill_path(repo_root);
    let mut changed = write_managed_file(&skill_path, USING_DEVQL_SKILL)?;

    let gemini_md_path = gemini_md_path(repo_root);
    let gemini_content = match std::fs::read_to_string(&gemini_md_path) {
        Ok(content) => content,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(err) => {
            return Err(err).with_context(|| format!("reading {}", gemini_md_path.display()));
        }
    };
    let managed_block = managed_block();
    let next_content = if gemini_content.contains(MANAGED_BLOCK_START)
        && gemini_content.contains(MANAGED_BLOCK_END)
        && gemini_content.contains(MANAGED_IMPORT_LINE)
    {
        gemini_content
    } else if gemini_content.trim().is_empty() {
        managed_block
    } else {
        let mut content = gemini_content;
        if !content.ends_with('\n') {
            content.push('\n');
        }
        content.push('\n');
        content.push_str(&managed_block);
        content
    };

    changed |= write_if_changed(&gemini_md_path, &next_content)
        .with_context(|| format!("writing {}", gemini_md_path.display()))?;

    Ok(changed)
}

pub fn uninstall_repo_skill(repo_root: &Path) -> Result<()> {
    let skill_path = repo_skill_path(repo_root);
    remove_managed_file(&skill_path)
        .with_context(|| format!("removing {}", skill_path.display()))?;
    prune_empty_parents(&skill_path, &repo_root.join(GEMINI_DIR_NAME))?;

    let gemini_md_path = gemini_md_path(repo_root);
    let gemini_content = match std::fs::read_to_string(&gemini_md_path) {
        Ok(content) => content,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(err) => {
            return Err(err).with_context(|| format!("reading {}", gemini_md_path.display()));
        }
    };

    let next_content = remove_managed_block(&gemini_content);
    if next_content.trim().is_empty() {
        match std::fs::remove_file(&gemini_md_path) {
            Ok(()) => {}
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(err) => {
                return Err(err).with_context(|| format!("removing {}", gemini_md_path.display()));
            }
        }
    } else {
        write_if_changed(&gemini_md_path, &next_content)
            .with_context(|| format!("writing {}", gemini_md_path.display()))?;
    }

    Ok(())
}

fn managed_block() -> String {
    format!("{MANAGED_BLOCK_START}\n{MANAGED_IMPORT_LINE}\n{MANAGED_BLOCK_END}\n")
}

fn write_if_changed(path: &Path, content: &str) -> Result<bool> {
    match std::fs::read_to_string(path) {
        Ok(existing) => {
            if existing == content {
                Ok(false)
            } else {
                std::fs::write(path, content)
                    .with_context(|| format!("writing {}", path.display()))?;
                Ok(true)
            }
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            std::fs::write(path, content).with_context(|| format!("writing {}", path.display()))?;
            Ok(true)
        }
        Err(err) => Err(err).with_context(|| format!("reading {}", path.display())),
    }
}

fn remove_managed_block(content: &str) -> String {
    let mut result = content.to_string();
    while let Some(start) = result.find(MANAGED_BLOCK_START) {
        let after_start = &result[start..];
        let Some(end_rel) = after_start.find(MANAGED_BLOCK_END) else {
            break;
        };
        let mut end = start + end_rel + MANAGED_BLOCK_END.len();
        if result[end..].starts_with("\r\n") {
            end += 2;
        } else if result[end..].starts_with('\n') {
            end += 1;
        }
        result.replace_range(start..end, "");
    }
    result
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    use crate::host::hooks::augmentation::skill_content::USING_DEVQL_SKILL;

    use super::*;

    #[test]
    #[allow(non_snake_case)]
    fn TestInstallRepoSkill_WritesSkillAndSingleManagedImport() {
        let dir = tempdir().expect("failed to create temp dir");
        let root = dir.path();
        let gemini_dir = root.join(".gemini");
        std::fs::create_dir_all(&gemini_dir).expect("failed to create .gemini");
        std::fs::write(root.join("GEMINI.md"), "user context\n").expect("failed to seed GEMINI.md");

        let changed = install_repo_skill(root).expect("install_repo_skill should succeed");
        assert!(changed);

        let skill_path = repo_skill_path(root);
        assert_eq!(
            std::fs::read_to_string(&skill_path).expect("failed to read skill"),
            USING_DEVQL_SKILL
        );

        let gemini_md =
            std::fs::read_to_string(root.join("GEMINI.md")).expect("failed to read GEMINI.md");
        assert!(gemini_md.contains("user context"));
        assert!(gemini_md.contains("@./.gemini/skills/bitloops/using-devql/SKILL.md"));
        assert_eq!(
            gemini_md
                .matches("@./.gemini/skills/bitloops/using-devql/SKILL.md")
                .count(),
            1
        );

        let changed = install_repo_skill(root).expect("idempotent install should succeed");
        assert!(!changed);
        let gemini_md_again =
            std::fs::read_to_string(root.join("GEMINI.md")).expect("failed to read GEMINI.md");
        assert_eq!(gemini_md_again, gemini_md);
    }

    #[test]
    #[allow(non_snake_case)]
    fn TestUninstallRepoSkill_RemovesManagedImportAndPreservesUserContent() {
        let dir = tempdir().expect("failed to create temp dir");
        let root = dir.path();
        let gemini_dir = root.join(".gemini");
        std::fs::create_dir_all(&gemini_dir).expect("failed to create .gemini");
        std::fs::write(
            root.join("GEMINI.md"),
            "user context\n\n<!-- bitloops-managed-start -->\n@./.gemini/skills/bitloops/using-devql/SKILL.md\n<!-- bitloops-managed-end -->\n",
        )
        .expect("failed to seed GEMINI.md");
        std::fs::create_dir_all(repo_skill_path(root).parent().expect("skill parent"))
            .expect("failed to create skill directory");
        std::fs::write(repo_skill_path(root), USING_DEVQL_SKILL).expect("failed to seed skill");

        uninstall_repo_skill(root).expect("uninstall_repo_skill should succeed");

        assert!(!repo_skill_path(root).exists());
        let gemini_md =
            std::fs::read_to_string(root.join("GEMINI.md")).expect("failed to read GEMINI.md");
        assert!(gemini_md.contains("user context"));
        assert!(!gemini_md.contains("@./.gemini/skills/bitloops/using-devql/SKILL.md"));
        assert!(!gemini_md.contains("bitloops-managed-start"));
    }
}
