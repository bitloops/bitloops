use std::path::Path;

use super::devql_guidance::build_turn_guidance;
use super::skill_content::USING_DEVQL_SKILL;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HookAugmentation {
    pub additional_context: String,
    pub targeted: bool,
}

pub fn build_devql_session_start_augmentation() -> HookAugmentation {
    HookAugmentation {
        additional_context: session_bootstrap_text(),
        targeted: false,
    }
}

pub fn build_devql_hook_augmentation(repo_root: &Path, prompt: &str) -> HookAugmentation {
    HookAugmentation {
        additional_context: build_turn_guidance(repo_root, prompt),
        targeted: true,
    }
}

fn session_bootstrap_text() -> String {
    format!(
        "<EXTREMELY_IMPORTANT>\n\
You have DevQL in this repo.\n\
\n\
The repo also includes a Bitloops-managed skill at `.claude/skills/bitloops/using-devql/SKILL.md`.\n\
Follow the `using-devql` guidance below for code understanding and exploration.\n\
\n\
{}\n\
</EXTREMELY_IMPORTANT>",
        USING_DEVQL_SKILL
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_start_guidance_uses_canonical_skill_content() {
        let augmentation = build_devql_session_start_augmentation();

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
        assert!(
            augmentation
                .additional_context
                .contains("name: using-devql")
        );
        assert!(
            augmentation
                .additional_context
                .contains("bitloops devql query '{ selectArtefacts(by: { symbolFqn: \"<symbol-fqn>\" }) { summary } }'")
        );
        assert!(
            augmentation
                .additional_context
                .contains("bitloops devql schema --global")
        );
        assert!(!augmentation.additional_context.contains("tracked.txt"));
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
