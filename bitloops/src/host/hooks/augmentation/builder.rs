use std::path::Path;

use super::prompt_surface_presence::installed_prompt_surface_label;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HookAugmentation {
    pub additional_context: String,
    pub targeted: bool,
}

pub fn build_devql_session_start_augmentation(
    repo_root: &Path,
    agent_name: &str,
) -> Option<HookAugmentation> {
    let surface_label = installed_prompt_surface_label(repo_root, agent_name)?;
    Some(HookAugmentation {
        additional_context: session_bootstrap_text(surface_label),
        targeted: false,
    })
}

pub fn build_devql_hook_augmentation(
    _repo_root: &Path,
    _agent_name: &str,
    _prompt: &str,
) -> Option<HookAugmentation> {
    None
}

fn session_bootstrap_text(surface_label: &str) -> String {
    format!(
        "<EXTREMELY_IMPORTANT>\n\
Bitloops has installed DevQL guidance for this repo at {}.\n\
Use that repo-local guidance surface for DevQL-specific instructions.\n\
</EXTREMELY_IMPORTANT>",
        surface_label
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::agents::{AGENT_NAME_CODEX, AGENT_NAME_GEMINI};

    #[test]
    fn session_start_guidance_mentions_skill_path_without_inlining_skill_body() {
        let dir = tempfile::tempdir().expect("tempdir");
        crate::adapters::agents::codex::skills::install_repo_skill(dir.path())
            .expect("install codex repo skill");

        let augmentation = build_devql_session_start_augmentation(dir.path(), AGENT_NAME_CODEX)
            .expect("augmentation");

        assert!(!augmentation.targeted);
        assert!(
            augmentation
                .additional_context
                .contains("<EXTREMELY_IMPORTANT>")
        );
        assert!(
            augmentation
                .additional_context
                .contains(".agents/skills/bitloops/using-devql/SKILL.md")
        );
        assert!(
            !augmentation
                .additional_context
                .contains("name: using-devql")
        );
        assert!(
            !augmentation
                .additional_context
                .contains("bitloops devql query")
        );
        assert!(!augmentation.additional_context.contains("fuzzyName"));
        assert!(!augmentation.additional_context.contains("tracked.txt"));
    }

    #[test]
    fn session_start_guidance_is_absent_when_repo_skill_is_not_installed() {
        let dir = tempfile::tempdir().expect("tempdir");

        assert_eq!(
            build_devql_session_start_augmentation(dir.path(), AGENT_NAME_CODEX),
            None
        );
    }

    #[test]
    fn session_start_guidance_is_agent_specific() {
        let dir = tempfile::tempdir().expect("tempdir");
        crate::adapters::agents::codex::skills::install_repo_skill(dir.path())
            .expect("install codex repo skill");
        crate::adapters::agents::gemini::skills::install_repo_skill(dir.path())
            .expect("install gemini repo skill");

        let codex = build_devql_session_start_augmentation(dir.path(), AGENT_NAME_CODEX)
            .expect("codex augmentation");
        let gemini = build_devql_session_start_augmentation(dir.path(), AGENT_NAME_GEMINI)
            .expect("gemini augmentation");

        assert!(
            codex
                .additional_context
                .contains(".agents/skills/bitloops/using-devql/SKILL.md")
        );
        assert!(
            gemini
                .additional_context
                .contains(".gemini/skills/bitloops/using-devql/SKILL.md")
        );
        assert!(
            !codex
                .additional_context
                .contains(".gemini/skills/bitloops/using-devql/SKILL.md")
        );
    }

    #[test]
    fn turn_guidance_is_absent_even_for_targeted_prompt() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("tracked.txt"), "one\n").expect("write tracked file");

        assert_eq!(
            build_devql_hook_augmentation(dir.path(), AGENT_NAME_CODEX, "Explain tracked.txt:1"),
            None
        );
    }
}
