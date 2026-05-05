use std::path::Path;

use super::devql_guidance::{build_session_bootstrap, build_turn_guidance, prompt_warrants_devql};
use super::prompt_surface_presence::installed_prompt_surface_relative_path;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HookAugmentation {
    pub additional_context: String,
    pub targeted: bool,
}

pub fn build_devql_session_start_augmentation(
    repo_root: &Path,
    agent_name: &str,
) -> Option<HookAugmentation> {
    let surface_path = installed_prompt_surface_relative_path(repo_root, agent_name)?;
    if agent_name == crate::adapters::agents::AGENT_NAME_CODEX {
        return None;
    }

    Some(HookAugmentation {
        additional_context: build_session_bootstrap(surface_path),
        targeted: false,
    })
}

pub fn build_devql_hook_augmentation(
    repo_root: &Path,
    agent_name: &str,
    prompt: &str,
) -> Option<HookAugmentation> {
    let surface_path = installed_prompt_surface_relative_path(repo_root, agent_name)?;
    if !agent_supports_turn_guidance(agent_name) || !prompt_warrants_devql(prompt) {
        return None;
    }

    Some(HookAugmentation {
        additional_context: build_turn_guidance(surface_path),
        targeted: true,
    })
}

fn agent_supports_turn_guidance(agent_name: &str) -> bool {
    matches!(agent_name, crate::adapters::agents::AGENT_NAME_CLAUDE_CODE)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::agents::{AGENT_NAME_CLAUDE_CODE, AGENT_NAME_CODEX, AGENT_NAME_GEMINI};

    fn write_repo_policy(dir: &tempfile::TempDir, body: &str) {
        std::fs::write(dir.path().join(".bitloops.toml"), body).expect("write repo policy");
    }

    #[test]
    fn session_start_guidance_includes_search_overview_and_response_hint_guidance() {
        let dir = tempfile::tempdir().expect("tempdir");
        write_repo_policy(
            &dir,
            r#"
[capture]
enabled = true
"#,
        );
        crate::adapters::agents::claude_code::skills::install_repo_skill(dir.path())
            .expect("install claude repo skill");

        let augmentation =
            build_devql_session_start_augmentation(dir.path(), AGENT_NAME_CLAUDE_CODE)
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
                .contains("DevQL-capable guidance surface")
        );
        assert!(
            augmentation
                .additional_context
                .contains("When DevQL is available in this session")
        );
        assert!(augmentation.additional_context.contains("path"));
        assert!(augmentation.additional_context.contains("symbolFqn"));
        assert!(augmentation.additional_context.contains("search"));
        assert!(augmentation.additional_context.contains("overview"));
        assert!(augmentation.additional_context.contains("expandHint"));
        assert!(!augmentation.additional_context.contains("fuzzyName"));
        assert!(!augmentation.additional_context.contains("naturalLanguage"));
        assert!(!augmentation.additional_context.contains("semanticQuery"));
        assert!(augmentation.additional_context.contains(
            crate::adapters::agents::claude_code::skills::CLAUDE_CODE_SKILL_RELATIVE_PATH
        ));
        assert!(!augmentation.additional_context.contains("# Using DevQL"));
        assert!(
            !augmentation
                .additional_context
                .contains("bitloops devql query '{")
        );
        assert!(
            augmentation
                .additional_context
                .contains("fall back to targeted repo search or file reads")
        );
    }

    #[test]
    fn codex_session_start_validates_skill_but_emits_no_bootstrap() {
        let dir = tempfile::tempdir().expect("tempdir");
        write_repo_policy(
            &dir,
            r#"
[capture]
enabled = true
"#,
        );
        crate::adapters::agents::codex::skills::install_repo_skill(dir.path())
            .expect("install codex repo skill");

        assert_eq!(
            build_devql_session_start_augmentation(dir.path(), AGENT_NAME_CODEX),
            None
        );
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
    #[should_panic(expected = "expected minimal skill is missing")]
    fn session_start_guidance_panics_when_codex_guidance_enabled_but_skill_missing() {
        let dir = tempfile::tempdir().expect("tempdir");
        write_repo_policy(
            &dir,
            r#"
[agents]
supported = ["codex"]
devql_guidance_enabled = true
"#,
        );

        let _ = build_devql_session_start_augmentation(dir.path(), AGENT_NAME_CODEX);
    }

    #[test]
    fn session_start_guidance_is_absent_when_policy_disables_guidance_even_if_skill_exists() {
        let dir = tempfile::tempdir().expect("tempdir");
        write_repo_policy(
            &dir,
            r#"
[agents]
supported = ["codex"]
devql_guidance_enabled = false
"#,
        );
        crate::adapters::agents::codex::skills::install_repo_skill(dir.path())
            .expect("install codex repo skill");

        assert_eq!(
            build_devql_session_start_augmentation(dir.path(), AGENT_NAME_CODEX),
            None
        );
    }

    #[test]
    fn session_start_guidance_is_agent_specific() {
        let dir = tempfile::tempdir().expect("tempdir");
        write_repo_policy(
            &dir,
            r#"
[capture]
enabled = true
"#,
        );
        crate::adapters::agents::codex::skills::install_repo_skill(dir.path())
            .expect("install codex repo skill");
        crate::adapters::agents::gemini::skills::install_repo_skill(dir.path())
            .expect("install gemini repo skill");

        let gemini = build_devql_session_start_augmentation(dir.path(), AGENT_NAME_GEMINI)
            .expect("gemini augmentation");

        assert_eq!(
            build_devql_session_start_augmentation(dir.path(), AGENT_NAME_CODEX),
            None
        );
        assert!(
            gemini
                .additional_context
                .contains(crate::adapters::agents::gemini::skills::GEMINI_SKILL_RELATIVE_PATH)
        );
    }

    #[test]
    fn codex_turn_guidance_is_absent_for_repo_understanding_prompt_when_skill_exists() {
        let dir = tempfile::tempdir().expect("tempdir");
        write_repo_policy(
            &dir,
            r#"
[capture]
enabled = true
"#,
        );
        crate::adapters::agents::codex::skills::install_repo_skill(dir.path())
            .expect("install codex repo skill");

        assert_eq!(
            build_devql_hook_augmentation(
                dir.path(),
                AGENT_NAME_CODEX,
                "Help me understand src/payments/invoice.ts:42",
            ),
            None
        );
    }

    #[test]
    fn claude_turn_guidance_is_present_for_repo_understanding_prompt_when_skill_exists() {
        let dir = tempfile::tempdir().expect("tempdir");
        write_repo_policy(
            &dir,
            r#"
[capture]
enabled = true
"#,
        );
        crate::adapters::agents::claude_code::skills::install_repo_skill(dir.path())
            .expect("install claude repo skill");

        let augmentation = build_devql_hook_augmentation(
            dir.path(),
            AGENT_NAME_CLAUDE_CODE,
            "Help me understand src/payments/invoice.ts:42",
        )
        .expect("augmentation");

        assert!(augmentation.targeted);
        assert!(
            augmentation
                .additional_context
                .contains("when it is available in this session")
        );
        assert!(augmentation.additional_context.contains("path + lines"));
        assert!(augmentation.additional_context.contains("search"));
        assert!(augmentation.additional_context.contains("overview"));
        assert!(augmentation.additional_context.contains("expandHint"));
        assert!(!augmentation.additional_context.contains("fuzzyName"));
        assert!(!augmentation.additional_context.contains("naturalLanguage"));
        assert!(!augmentation.additional_context.contains("semanticQuery"));
        assert!(
            augmentation
                .additional_context
                .contains("fall back to targeted repo search or file reads")
        );
    }

    #[test]
    fn codex_turn_guidance_is_absent_for_repo_overview_prompt_when_skill_exists() {
        let dir = tempfile::tempdir().expect("tempdir");
        write_repo_policy(
            &dir,
            r#"
[capture]
enabled = true
"#,
        );
        crate::adapters::agents::codex::skills::install_repo_skill(dir.path())
            .expect("install codex repo skill");

        assert_eq!(
            build_devql_hook_augmentation(dir.path(), AGENT_NAME_CODEX, "What does this repo do?"),
            None
        );
    }

    #[test]
    fn turn_guidance_is_absent_for_non_repo_understanding_prompt() {
        let dir = tempfile::tempdir().expect("tempdir");
        write_repo_policy(
            &dir,
            r#"
[capture]
enabled = true
"#,
        );
        crate::adapters::agents::codex::skills::install_repo_skill(dir.path())
            .expect("install codex repo skill");

        assert_eq!(
            build_devql_hook_augmentation(dir.path(), AGENT_NAME_CODEX, "Run cargo fmt"),
            None
        );
    }

    #[test]
    fn turn_guidance_is_absent_when_policy_disables_guidance_even_if_skill_exists() {
        let dir = tempfile::tempdir().expect("tempdir");
        write_repo_policy(
            &dir,
            r#"
[agents]
supported = ["codex"]
devql_guidance_enabled = false
"#,
        );
        crate::adapters::agents::codex::skills::install_repo_skill(dir.path())
            .expect("install codex repo skill");

        assert_eq!(
            build_devql_hook_augmentation(
                dir.path(),
                AGENT_NAME_CODEX,
                "Help me understand src/payments/invoice.ts:42",
            ),
            None
        );
    }
}
