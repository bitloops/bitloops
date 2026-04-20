use std::path::Path;

use crate::adapters::agents::{
    AGENT_NAME_CLAUDE_CODE, AGENT_NAME_CODEX, AGENT_NAME_COPILOT, AGENT_NAME_CURSOR,
    AGENT_NAME_GEMINI, AGENT_NAME_OPEN_CODE,
};

pub fn installed_prompt_surface_label(repo_root: &Path, agent_name: &str) -> Option<&'static str> {
    match agent_name {
        AGENT_NAME_CLAUDE_CODE => {
            crate::adapters::agents::claude_code::skills::repo_skill_path(repo_root)
                .is_file()
                .then_some("`.claude/skills/bitloops/using-devql/SKILL.md`")
        }
        AGENT_NAME_CODEX => crate::adapters::agents::codex::skills::repo_skill_path(repo_root)
            .is_file()
            .then_some("`.agents/skills/bitloops/using-devql/SKILL.md`"),
        AGENT_NAME_COPILOT => crate::adapters::agents::copilot::skills::repo_skill_path(repo_root)
            .is_file()
            .then_some("`.github/skills/bitloops/using-devql/SKILL.md`"),
        AGENT_NAME_CURSOR => crate::adapters::agents::cursor::rules::repo_rule_path(repo_root)
            .is_file()
            .then_some("`.cursor/rules/bitloops-using-devql.mdc`"),
        AGENT_NAME_GEMINI => crate::adapters::agents::gemini::skills::repo_skill_path(repo_root)
            .is_file()
            .then_some("`.gemini/skills/bitloops/using-devql/SKILL.md`"),
        AGENT_NAME_OPEN_CODE => {
            crate::adapters::agents::open_code::skills::repo_skill_path(repo_root)
                .is_file()
                .then_some("`.opencode/skills/bitloops/using-devql/SKILL.md`")
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn installed_prompt_surface_label_is_none_when_surface_is_absent() {
        let dir = tempfile::tempdir().expect("tempdir");

        assert_eq!(
            installed_prompt_surface_label(dir.path(), AGENT_NAME_CODEX),
            None
        );
    }

    #[test]
    fn installed_prompt_surface_label_returns_codex_skill_path_when_installed() {
        let dir = tempfile::tempdir().expect("tempdir");
        crate::adapters::agents::codex::skills::install_repo_skill(dir.path())
            .expect("install codex repo skill");

        assert_eq!(
            installed_prompt_surface_label(dir.path(), AGENT_NAME_CODEX),
            Some("`.agents/skills/bitloops/using-devql/SKILL.md`")
        );
    }
}
