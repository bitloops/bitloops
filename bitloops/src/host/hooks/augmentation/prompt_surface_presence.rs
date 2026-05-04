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
        AGENT_NAME_CODEX => {
            let path = crate::adapters::agents::codex::skills::repo_skill_path(repo_root);
            if path.is_file() {
                Some(crate::adapters::agents::codex::skills::CODEX_SKILL_RELATIVE_PATH)
            } else {
                let legacy_path = repo_root
                    .join(crate::adapters::agents::codex::skills::LEGACY_CODEX_SKILL_RELATIVE_PATH);
                if legacy_path.is_file() {
                    panic!(
                        "Codex DevQL guidance is enabled, but only the legacy verbose skill exists at {}. Expected the minimal skill at {}. Run `bitloops init` before starting Codex.",
                        legacy_path.display(),
                        path.display()
                    );
                }
                panic!(
                    "Codex DevQL guidance is enabled, but the expected minimal skill is missing at {}. Run `bitloops init` before starting Codex.",
                    path.display()
                );
            }
        }
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

    #[test]
    #[should_panic(expected = "only the legacy verbose skill exists")]
    fn installed_prompt_surface_relative_path_panics_when_only_legacy_codex_skill_exists() {
        let dir = tempfile::tempdir().expect("tempdir");
        write_repo_policy(
            &dir,
            r#"
[agents]
supported = ["codex"]
devql_guidance_enabled = true
"#,
        );
        let legacy_path = dir
            .path()
            .join(crate::adapters::agents::codex::skills::LEGACY_CODEX_SKILL_RELATIVE_PATH);
        std::fs::create_dir_all(legacy_path.parent().expect("legacy parent"))
            .expect("create legacy parent");
        std::fs::write(legacy_path, "legacy").expect("write legacy skill");

        let _ = installed_prompt_surface_relative_path(dir.path(), AGENT_NAME_CODEX);
    }

    #[test]
    #[should_panic(expected = "expected minimal skill is missing")]
    fn installed_prompt_surface_relative_path_panics_when_enabled_codex_skill_is_missing() {
        let dir = tempfile::tempdir().expect("tempdir");
        write_repo_policy(
            &dir,
            r#"
[agents]
supported = ["codex"]
devql_guidance_enabled = true
"#,
        );

        let _ = installed_prompt_surface_relative_path(dir.path(), AGENT_NAME_CODEX);
    }
}
