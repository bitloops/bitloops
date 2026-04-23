use std::path::Path;

use crate::adapters::agents::{
    AGENT_NAME_CLAUDE_CODE, AGENT_NAME_CODEX, AGENT_NAME_COPILOT, AGENT_NAME_CURSOR,
    AGENT_NAME_GEMINI, AGENT_NAME_OPEN_CODE,
};

pub fn installed_prompt_surface_relative_path(
    repo_root: &Path,
    agent_name: &str,
) -> Option<&'static str> {
    if !crate::config::settings::devql_guidance_enabled_or_false(repo_root) {
        return None;
    }

    match agent_name {
        AGENT_NAME_CLAUDE_CODE => {
            crate::adapters::agents::claude_code::skills::repo_skill_path(repo_root)
                .is_file()
                .then_some(
                    crate::adapters::agents::claude_code::skills::CLAUDE_CODE_SKILL_RELATIVE_PATH,
                )
        }
        AGENT_NAME_CODEX => crate::adapters::agents::codex::skills::repo_skill_path(repo_root)
            .is_file()
            .then_some(crate::adapters::agents::codex::skills::CODEX_SKILL_RELATIVE_PATH),
        AGENT_NAME_COPILOT => crate::adapters::agents::copilot::skills::repo_skill_path(repo_root)
            .is_file()
            .then_some(crate::adapters::agents::copilot::skills::COPILOT_SKILL_RELATIVE_PATH),
        AGENT_NAME_CURSOR => crate::adapters::agents::cursor::rules::repo_rule_path(repo_root)
            .is_file()
            .then_some(crate::adapters::agents::cursor::rules::CURSOR_RULE_RELATIVE_PATH),
        AGENT_NAME_GEMINI => crate::adapters::agents::gemini::skills::repo_skill_path(repo_root)
            .is_file()
            .then_some(crate::adapters::agents::gemini::skills::GEMINI_SKILL_RELATIVE_PATH),
        AGENT_NAME_OPEN_CODE => {
            crate::adapters::agents::open_code::skills::repo_skill_path(repo_root)
                .is_file()
                .then_some(
                    crate::adapters::agents::open_code::skills::OPEN_CODE_SKILL_RELATIVE_PATH,
                )
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_repo_policy(dir: &tempfile::TempDir, body: &str) {
        std::fs::write(dir.path().join(".bitloops.toml"), body).expect("write repo policy");
    }

    #[test]
    fn installed_prompt_surface_relative_path_is_none_when_surface_is_absent() {
        let dir = tempfile::tempdir().expect("tempdir");

        assert_eq!(
            installed_prompt_surface_relative_path(dir.path(), AGENT_NAME_CODEX),
            None
        );
    }

    #[test]
    fn installed_prompt_surface_relative_path_returns_codex_skill_path_when_installed() {
        let dir = tempfile::tempdir().expect("tempdir");
        write_repo_policy(
            &dir,
            r#"
[agents]
supported = ["codex"]
devql_guidance_enabled = true
"#,
        );
        crate::adapters::agents::codex::skills::install_repo_skill(dir.path())
            .expect("install codex repo skill");

        assert_eq!(
            installed_prompt_surface_relative_path(dir.path(), AGENT_NAME_CODEX),
            Some(crate::adapters::agents::codex::skills::CODEX_SKILL_RELATIVE_PATH)
        );
    }

    #[test]
    fn installed_prompt_surface_relative_path_is_none_when_policy_disables_guidance() {
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
            installed_prompt_surface_relative_path(dir.path(), AGENT_NAME_CODEX),
            None
        );
    }
}
