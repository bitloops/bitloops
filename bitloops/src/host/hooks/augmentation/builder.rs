use std::path::Path;

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
    Some(HookAugmentation {
        additional_context: session_bootstrap_text(surface_path),
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
        additional_context: format!(
            "Use DevQL first for this repo-aware request. Prefer `path`, `path + lines`, `symbolFqn`, `fuzzyName`, or a distilled `semanticQuery` before broad repo search. Detailed repo-local DevQL guidance: `{surface_path}`."
        ),
        targeted: true,
    })
}

fn session_bootstrap_text(surface_path: &str) -> String {
    format!(
        "<EXTREMELY_IMPORTANT>\n\
This repo has DevQL.\n\
For code understanding, repo exploration, architecture questions, symbol lookup, path/line resolution, and callers/usages/imports/tests/dependencies, use DevQL first before broad repo search or directory crawling.\n\
Start with the most specific selector available: `path`, `path + lines`, `symbolFqn`, `fuzzyName`, or a distilled `semanticQuery`.\n\
Fall back to targeted repo search only if DevQL returns no useful artefacts or rows.\n\
Detailed repo-local DevQL guidance is installed at `{surface_path}`.\n\
</EXTREMELY_IMPORTANT>"
    )
}

fn agent_supports_turn_guidance(agent_name: &str) -> bool {
    matches!(
        agent_name,
        crate::adapters::agents::AGENT_NAME_CLAUDE_CODE | crate::adapters::agents::AGENT_NAME_CODEX
    )
}

fn prompt_warrants_devql(prompt: &str) -> bool {
    let lower = prompt.to_ascii_lowercase();
    let repo_understanding_terms = [
        "understand",
        "explain",
        "architecture",
        "where is",
        "find",
        "inspect",
        "caller",
        "usage",
        "import",
        "dependency",
        "test covering",
    ];
    let execution_terms = [
        "fix ",
        "implement ",
        "edit ",
        "write ",
        "run ",
        "build ",
        "test ",
        "format ",
    ];
    let looks_like_code_reference = prompt.contains('/')
        || prompt.contains("::")
        || prompt.contains('`')
        || prompt.contains(':');
    let looks_like_edit_or_execution = execution_terms.iter().any(|needle| lower.contains(needle));
    let looks_like_repo_understanding = repo_understanding_terms
        .iter()
        .any(|needle| lower.contains(needle));

    (looks_like_code_reference || looks_like_repo_understanding) && !looks_like_edit_or_execution
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::agents::{AGENT_NAME_CODEX, AGENT_NAME_GEMINI};

    #[test]
    fn session_start_guidance_includes_direct_devql_first_instructions_when_skill_exists() {
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
                .contains("This repo has DevQL")
        );
        assert!(
            augmentation
                .additional_context
                .contains("use DevQL first before broad repo search or directory crawling")
        );
        assert!(augmentation.additional_context.contains("path"));
        assert!(augmentation.additional_context.contains("symbolFqn"));
        assert!(augmentation.additional_context.contains("fuzzyName"));
        assert!(augmentation.additional_context.contains("semanticQuery"));
        assert!(
            augmentation
                .additional_context
                .contains(crate::adapters::agents::codex::skills::CODEX_SKILL_RELATIVE_PATH)
        );
        assert!(!augmentation.additional_context.contains("# Using DevQL"));
        assert!(
            !augmentation
                .additional_context
                .contains("bitloops devql query '{")
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
                .contains(crate::adapters::agents::codex::skills::CODEX_SKILL_RELATIVE_PATH)
        );
        assert!(
            gemini
                .additional_context
                .contains(crate::adapters::agents::gemini::skills::GEMINI_SKILL_RELATIVE_PATH)
        );
        assert!(
            !codex
                .additional_context
                .contains(crate::adapters::agents::gemini::skills::GEMINI_SKILL_RELATIVE_PATH)
        );
    }

    #[test]
    fn turn_guidance_is_present_for_repo_understanding_prompt_when_skill_exists() {
        let dir = tempfile::tempdir().expect("tempdir");
        crate::adapters::agents::codex::skills::install_repo_skill(dir.path())
            .expect("install codex repo skill");

        let augmentation = build_devql_hook_augmentation(
            dir.path(),
            AGENT_NAME_CODEX,
            "Help me understand src/payments/invoice.ts:42",
        )
        .expect("augmentation");

        assert!(augmentation.targeted);
        assert!(augmentation.additional_context.contains("Use DevQL first"));
        assert!(augmentation.additional_context.contains("path + lines"));
    }

    #[test]
    fn turn_guidance_is_absent_for_non_repo_understanding_prompt() {
        let dir = tempfile::tempdir().expect("tempdir");
        crate::adapters::agents::codex::skills::install_repo_skill(dir.path())
            .expect("install codex repo skill");

        assert_eq!(
            build_devql_hook_augmentation(dir.path(), AGENT_NAME_CODEX, "Run cargo fmt"),
            None
        );
    }
}
