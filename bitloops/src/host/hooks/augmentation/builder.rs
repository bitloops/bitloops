use std::path::Path;

use crate::adapters::agents::{
    AGENT_NAME_CLAUDE_CODE, AGENT_NAME_CODEX, AGENT_NAME_COPILOT, AGENT_NAME_CURSOR,
    AGENT_NAME_GEMINI, AGENT_NAME_OPEN_CODE,
};

use super::devql_guidance::build_turn_guidance;
use super::skill_content::USING_DEVQL_SKILL;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HookAugmentation {
    pub additional_context: String,
    pub targeted: bool,
}

pub fn build_devql_session_start_augmentation(agent_name: &str) -> HookAugmentation {
    HookAugmentation {
        additional_context: session_bootstrap_text(agent_name),
        targeted: false,
    }
}

pub fn build_devql_hook_augmentation(repo_root: &Path, prompt: &str) -> HookAugmentation {
    HookAugmentation {
        additional_context: build_turn_guidance(repo_root, prompt),
        targeted: true,
    }
}

fn session_bootstrap_text(agent_name: &str) -> String {
    let surface_label = match agent_name {
        AGENT_NAME_CLAUDE_CODE => {
            "The repo includes a Bitloops-managed skill at `.claude/skills/bitloops/using-devql/SKILL.md`."
        }
        AGENT_NAME_CODEX => {
            "The repo includes a Bitloops-managed skill at `.agents/skills/bitloops/using-devql/SKILL.md`."
        }
        AGENT_NAME_COPILOT => {
            "The repo includes a Bitloops-managed skill at `.github/skills/bitloops/using-devql/SKILL.md`."
        }
        AGENT_NAME_CURSOR => {
            "The repo includes a Bitloops-managed Cursor rule at `.cursor/rules/bitloops-using-devql.mdc`, and Bitloops also provides Cursor session bootstrap guidance for this repo."
        }
        AGENT_NAME_GEMINI => {
            "The repo includes Bitloops-managed Gemini instructions via `GEMINI.md` and `.gemini/skills/bitloops/using-devql/SKILL.md`."
        }
        AGENT_NAME_OPEN_CODE => {
            "Bitloops provides DevQL guidance through the repo-local OpenCode plugin and `.opencode/skills/bitloops/using-devql/SKILL.md`."
        }
        _ => "Bitloops provides DevQL guidance for this repo.",
    };

    format!(
        "<EXTREMELY_IMPORTANT>\n\
You have DevQL in this repo.\n\
\n\
{}\n\
Follow the `using-devql` guidance below for code understanding and exploration.\n\
\n\
{}\n\
</EXTREMELY_IMPORTANT>",
        surface_label, USING_DEVQL_SKILL
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_start_guidance_for_claude_uses_canonical_skill_content() {
        let augmentation = build_devql_session_start_augmentation(AGENT_NAME_CLAUDE_CODE);

        assert!(!augmentation.targeted);
        assert!(
            augmentation
                .additional_context
                .contains("<EXTREMELY_IMPORTANT>")
        );
        assert!(
            augmentation
                .additional_context
                .contains("You have DevQL in this repo.")
        );
        assert!(
            augmentation
                .additional_context
                .contains(".claude/skills/bitloops/using-devql/SKILL.md")
        );
        assert!(augmentation.additional_context.contains(USING_DEVQL_SKILL));
        assert!(!augmentation.additional_context.contains("tracked.txt"));
    }

    #[test]
    fn session_start_guidance_is_agent_specific() {
        let codex = build_devql_session_start_augmentation(AGENT_NAME_CODEX);
        let gemini = build_devql_session_start_augmentation(AGENT_NAME_GEMINI);
        let cursor = build_devql_session_start_augmentation(AGENT_NAME_CURSOR);
        let copilot = build_devql_session_start_augmentation(AGENT_NAME_COPILOT);

        assert!(
            codex
                .additional_context
                .contains(".agents/skills/bitloops/using-devql/SKILL.md")
        );
        assert!(
            !codex
                .additional_context
                .contains("~/.agents/skills/bitloops/using-devql/SKILL.md")
        );
        assert!(
            gemini.additional_context.contains("GEMINI.md")
                && gemini
                    .additional_context
                    .contains(".gemini/skills/bitloops/using-devql/SKILL.md")
        );
        assert!(
            cursor
                .additional_context
                .contains(".cursor/rules/bitloops-using-devql.mdc")
        );
        assert!(
            cursor
                .additional_context
                .contains("Cursor session bootstrap")
        );
        assert!(
            copilot
                .additional_context
                .contains(".github/skills/bitloops/using-devql/SKILL.md")
        );
        assert!(
            !codex
                .additional_context
                .contains(".claude/skills/bitloops/using-devql/SKILL.md")
        );
    }

    #[test]
    fn turn_guidance_uses_prompt_target_for_line_scoped_command() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("tracked.txt"), "one\n").expect("write tracked file");

        let augmentation = build_devql_hook_augmentation(dir.path(), "Explain tracked.txt:1");

        assert!(augmentation.targeted);
        assert!(
            augmentation
                .additional_context
                .contains("Use DevQL first for this request.")
        );
        assert!(augmentation.additional_context.contains("tracked.txt"));
        assert!(augmentation.additional_context.contains("start: 1"));
        assert!(augmentation.additional_context.contains("end: 1"));
        assert!(
            !augmentation
                .additional_context
                .contains("<repo-relative-path>")
        );
        assert!(!augmentation.additional_context.contains("<symbol-fqn>"));
    }
}
