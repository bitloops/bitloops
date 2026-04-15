use std::fs;

use tempfile::tempdir;

use super::GeminiCliAgent;
use crate::adapters::agents::gemini::skills::{install_repo_skill, repo_skill_path};

#[test]
#[allow(non_snake_case)]
fn TestInstallHooks_InstallsRepoSkillAndGeminiImport() {
    let dir = tempdir().expect("failed to create temp dir");
    let root = dir.path();
    fs::write(root.join("GEMINI.md"), "user context\n").expect("failed to seed GEMINI.md");

    let agent = GeminiCliAgent;
    agent
        .install_hooks_at(root, false, false)
        .expect("install_hooks_at should succeed");

    assert!(repo_skill_path(root).exists());
    let gemini_md = fs::read_to_string(root.join("GEMINI.md")).expect("failed to read GEMINI.md");
    assert!(gemini_md.contains("user context"));
    assert!(gemini_md.contains("@./.gemini/skills/bitloops/using-devql/SKILL.md"));
    assert_eq!(
        gemini_md
            .matches("@./.gemini/skills/bitloops/using-devql/SKILL.md")
            .count(),
        1
    );

    agent
        .uninstall_hooks_at(root)
        .expect("uninstall_hooks_at should succeed");

    assert!(!repo_skill_path(root).exists());
    let gemini_md = fs::read_to_string(root.join("GEMINI.md")).expect("failed to read GEMINI.md");
    assert!(gemini_md.contains("user context"));
    assert!(!gemini_md.contains("@./.gemini/skills/bitloops/using-devql/SKILL.md"));
}

#[test]
#[allow(non_snake_case)]
fn TestInstallHooks_RepoSkillHelperIsIdempotent() {
    let dir = tempdir().expect("failed to create temp dir");
    let root = dir.path();
    install_repo_skill(root).expect("first install should succeed");
    let first = fs::read_to_string(root.join("GEMINI.md")).expect("failed to read GEMINI.md");
    install_repo_skill(root).expect("second install should succeed");
    let second = fs::read_to_string(root.join("GEMINI.md")).expect("failed to read GEMINI.md");
    assert_eq!(first, second);
}
