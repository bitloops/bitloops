use std::path::{Path, PathBuf};

use anyhow::{Result, anyhow};
use serde_json::{Map, Value};

use super::cli_agent::GeminiCliAgent;
use super::config::{
    BITLOOPS_HOOK_PREFIXES, GEMINI_SETTINGS_FILE_NAME, GeminiHookEntry, GeminiHookMatcher,
    GeminiHooksConfig, GeminiSettings,
};
use crate::adapters::agents::HookSupport;

impl HookSupport for GeminiCliAgent {
    fn install_hooks(&self, local_dev: bool, force: bool) -> Result<usize> {
        let repo_root = crate::utils::paths::repo_root().or_else(|_| {
            std::env::current_dir().map_err(|err| anyhow!("failed to get current directory: {err}"))
        })?;
        self.install_hooks_at(&repo_root, local_dev, force)
    }

    fn uninstall_hooks(&self) -> Result<()> {
        let repo_root = crate::utils::paths::repo_root().or_else(|_| {
            std::env::current_dir().map_err(|err| anyhow!("failed to get current directory: {err}"))
        })?;
        self.uninstall_hooks_at(&repo_root)
    }

    fn are_hooks_installed(&self) -> bool {
        let repo_root = match crate::utils::paths::repo_root().or_else(|_| {
            std::env::current_dir().map_err(|err| anyhow!("failed to get current directory: {err}"))
        }) {
            Ok(repo_root) => repo_root,
            Err(_) => return false,
        };
        self.are_hooks_installed_at(&repo_root)
    }
}

impl GeminiCliAgent {
    pub(crate) fn detect_presence_at(&self, repo_root: &Path) -> Result<bool> {
        let gemini_dir = repo_root.join(".gemini");
        if gemini_dir.exists() {
            return Ok(true);
        }

        let settings_file = gemini_dir.join(GEMINI_SETTINGS_FILE_NAME);
        if settings_file.exists() {
            return Ok(true);
        }

        Ok(false)
    }

    pub(crate) fn install_hooks_at(
        &self,
        repo_root: &Path,
        local_dev: bool,
        force: bool,
    ) -> Result<usize> {
        self.install_hooks_at_with_bitloops_skill(repo_root, local_dev, force, true)
    }

    pub(crate) fn install_hooks_at_with_bitloops_skill(
        &self,
        repo_root: &Path,
        local_dev: bool,
        force: bool,
        install_bitloops_skill: bool,
    ) -> Result<usize> {
        if install_bitloops_skill {
            crate::adapters::agents::gemini::skills::install_repo_skill(repo_root)?;
        } else {
            crate::adapters::agents::gemini::skills::uninstall_repo_skill(repo_root)?;
        }
        let settings_path = self.settings_path_at(repo_root);

        let mut raw_settings: Map<String, Value> = match std::fs::read(&settings_path) {
            Ok(data) => serde_json::from_slice(&data)
                .map_err(|err| anyhow!("failed to parse existing settings.json: {err}"))?,
            Err(_) => Map::new(),
        };

        let mut raw_hooks: Map<String, Value> = if let Some(hooks) = raw_settings.get("hooks") {
            serde_json::from_value(hooks.clone())
                .map_err(|err| anyhow!("failed to parse hooks in settings.json: {err}"))?
        } else {
            Map::new()
        };

        let mut hooks_config: GeminiHooksConfig =
            if let Some(hooks_config_value) = raw_settings.get("hooksConfig") {
                serde_json::from_value(hooks_config_value.clone())
                    .map_err(|err| anyhow!("failed to parse hooksConfig in settings.json: {err}"))?
            } else {
                GeminiHooksConfig::default()
            };
        hooks_config.enabled = true;

        let cmd_prefix = if local_dev {
            "cargo run -- hooks gemini "
        } else {
            "bitloops hooks gemini "
        };

        let mut session_start = Self::parse_gemini_hook_type(&raw_hooks, "SessionStart");
        let mut session_end = Self::parse_gemini_hook_type(&raw_hooks, "SessionEnd");
        let mut before_agent = Self::parse_gemini_hook_type(&raw_hooks, "BeforeAgent");
        let mut after_agent = Self::parse_gemini_hook_type(&raw_hooks, "AfterAgent");
        let mut before_model = Self::parse_gemini_hook_type(&raw_hooks, "BeforeModel");
        let mut after_model = Self::parse_gemini_hook_type(&raw_hooks, "AfterModel");
        let mut before_tool_selection =
            Self::parse_gemini_hook_type(&raw_hooks, "BeforeToolSelection");
        let mut before_tool = Self::parse_gemini_hook_type(&raw_hooks, "BeforeTool");
        let mut after_tool = Self::parse_gemini_hook_type(&raw_hooks, "AfterTool");
        let mut pre_compress = Self::parse_gemini_hook_type(&raw_hooks, "PreCompress");
        let mut notification = Self::parse_gemini_hook_type(&raw_hooks, "Notification");

        if !force {
            let existing_cmd = Self::get_first_bitloops_hook_command(&session_start);
            let expected_cmd = crate::adapters::agents::managed_hook_command(&format!(
                "{cmd_prefix}session-start"
            ));
            if existing_cmd == expected_cmd {
                return Ok(0);
            }
        }

        session_start = Self::remove_bitloops_hooks(session_start);
        session_end = Self::remove_bitloops_hooks(session_end);
        before_agent = Self::remove_bitloops_hooks(before_agent);
        after_agent = Self::remove_bitloops_hooks(after_agent);
        before_model = Self::remove_bitloops_hooks(before_model);
        after_model = Self::remove_bitloops_hooks(after_model);
        before_tool_selection = Self::remove_bitloops_hooks(before_tool_selection);
        before_tool = Self::remove_bitloops_hooks(before_tool);
        after_tool = Self::remove_bitloops_hooks(after_tool);
        pre_compress = Self::remove_bitloops_hooks(pre_compress);
        notification = Self::remove_bitloops_hooks(notification);

        session_start = Self::add_gemini_hook(
            session_start,
            "",
            "bitloops-session-start",
            crate::adapters::agents::managed_hook_command(&format!("{cmd_prefix}session-start")),
        );
        session_end = Self::add_gemini_hook(
            session_end,
            "exit",
            "bitloops-session-end-exit",
            crate::adapters::agents::managed_hook_command(&format!("{cmd_prefix}session-end")),
        );
        session_end = Self::add_gemini_hook(
            session_end,
            "logout",
            "bitloops-session-end-logout",
            crate::adapters::agents::managed_hook_command(&format!("{cmd_prefix}session-end")),
        );
        before_agent = Self::add_gemini_hook(
            before_agent,
            "",
            "bitloops-before-agent",
            crate::adapters::agents::managed_hook_command(&format!("{cmd_prefix}before-agent")),
        );
        after_agent = Self::add_gemini_hook(
            after_agent,
            "",
            "bitloops-after-agent",
            crate::adapters::agents::managed_hook_command(&format!("{cmd_prefix}after-agent")),
        );
        before_model = Self::add_gemini_hook(
            before_model,
            "",
            "bitloops-before-model",
            crate::adapters::agents::managed_hook_command(&format!("{cmd_prefix}before-model")),
        );
        after_model = Self::add_gemini_hook(
            after_model,
            "",
            "bitloops-after-model",
            crate::adapters::agents::managed_hook_command(&format!("{cmd_prefix}after-model")),
        );
        before_tool_selection = Self::add_gemini_hook(
            before_tool_selection,
            "",
            "bitloops-before-tool-selection",
            crate::adapters::agents::managed_hook_command(&format!(
                "{cmd_prefix}before-tool-selection"
            )),
        );
        before_tool = Self::add_gemini_hook(
            before_tool,
            "*",
            "bitloops-before-tool",
            crate::adapters::agents::managed_hook_command(&format!("{cmd_prefix}before-tool")),
        );
        after_tool = Self::add_gemini_hook(
            after_tool,
            "*",
            "bitloops-after-tool",
            crate::adapters::agents::managed_hook_command(&format!("{cmd_prefix}after-tool")),
        );
        pre_compress = Self::add_gemini_hook(
            pre_compress,
            "",
            "bitloops-pre-compress",
            crate::adapters::agents::managed_hook_command(&format!("{cmd_prefix}pre-compress")),
        );
        notification = Self::add_gemini_hook(
            notification,
            "",
            "bitloops-notification",
            crate::adapters::agents::managed_hook_command(&format!("{cmd_prefix}notification")),
        );

        Self::marshal_gemini_hook_type(&mut raw_hooks, "SessionStart", &session_start);
        Self::marshal_gemini_hook_type(&mut raw_hooks, "SessionEnd", &session_end);
        Self::marshal_gemini_hook_type(&mut raw_hooks, "BeforeAgent", &before_agent);
        Self::marshal_gemini_hook_type(&mut raw_hooks, "AfterAgent", &after_agent);
        Self::marshal_gemini_hook_type(&mut raw_hooks, "BeforeModel", &before_model);
        Self::marshal_gemini_hook_type(&mut raw_hooks, "AfterModel", &after_model);
        Self::marshal_gemini_hook_type(
            &mut raw_hooks,
            "BeforeToolSelection",
            &before_tool_selection,
        );
        Self::marshal_gemini_hook_type(&mut raw_hooks, "BeforeTool", &before_tool);
        Self::marshal_gemini_hook_type(&mut raw_hooks, "AfterTool", &after_tool);
        Self::marshal_gemini_hook_type(&mut raw_hooks, "PreCompress", &pre_compress);
        Self::marshal_gemini_hook_type(&mut raw_hooks, "Notification", &notification);

        let hooks_config_json = serde_json::to_value(hooks_config)
            .map_err(|err| anyhow!("failed to marshal hooksConfig: {err}"))?;
        raw_settings.insert("hooksConfig".to_string(), hooks_config_json);
        raw_settings.insert("hooks".to_string(), Value::Object(raw_hooks));

        if let Some(parent) = settings_path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|err| anyhow!("failed to create .gemini directory: {err}"))?;
        }
        let output = serde_json::to_vec_pretty(&raw_settings)
            .map_err(|err| anyhow!("failed to marshal settings: {err}"))?;
        std::fs::write(&settings_path, output)
            .map_err(|err| anyhow!("failed to write settings.json: {err}"))?;

        Ok(12)
    }

    pub(crate) fn uninstall_hooks_at(&self, repo_root: &Path) -> Result<()> {
        let settings_path = self.settings_path_at(repo_root);
        let data = match std::fs::read(&settings_path) {
            Ok(data) => data,
            Err(_) => {
                crate::adapters::agents::gemini::skills::uninstall_repo_skill(repo_root)?;
                return Ok(());
            }
        };

        let mut raw_settings: Map<String, Value> = serde_json::from_slice(&data)
            .map_err(|err| anyhow!("failed to parse settings.json: {err}"))?;

        let mut raw_hooks: Map<String, Value> = if let Some(hooks) = raw_settings.get("hooks") {
            serde_json::from_value(hooks.clone())
                .map_err(|err| anyhow!("failed to parse hooks: {err}"))?
        } else {
            Map::new()
        };

        let mut session_start = Self::parse_gemini_hook_type(&raw_hooks, "SessionStart");
        let mut session_end = Self::parse_gemini_hook_type(&raw_hooks, "SessionEnd");
        let mut before_agent = Self::parse_gemini_hook_type(&raw_hooks, "BeforeAgent");
        let mut after_agent = Self::parse_gemini_hook_type(&raw_hooks, "AfterAgent");
        let mut before_model = Self::parse_gemini_hook_type(&raw_hooks, "BeforeModel");
        let mut after_model = Self::parse_gemini_hook_type(&raw_hooks, "AfterModel");
        let mut before_tool_selection =
            Self::parse_gemini_hook_type(&raw_hooks, "BeforeToolSelection");
        let mut before_tool = Self::parse_gemini_hook_type(&raw_hooks, "BeforeTool");
        let mut after_tool = Self::parse_gemini_hook_type(&raw_hooks, "AfterTool");
        let mut pre_compress = Self::parse_gemini_hook_type(&raw_hooks, "PreCompress");
        let mut notification = Self::parse_gemini_hook_type(&raw_hooks, "Notification");

        session_start = Self::remove_bitloops_hooks(session_start);
        session_end = Self::remove_bitloops_hooks(session_end);
        before_agent = Self::remove_bitloops_hooks(before_agent);
        after_agent = Self::remove_bitloops_hooks(after_agent);
        before_model = Self::remove_bitloops_hooks(before_model);
        after_model = Self::remove_bitloops_hooks(after_model);
        before_tool_selection = Self::remove_bitloops_hooks(before_tool_selection);
        before_tool = Self::remove_bitloops_hooks(before_tool);
        after_tool = Self::remove_bitloops_hooks(after_tool);
        pre_compress = Self::remove_bitloops_hooks(pre_compress);
        notification = Self::remove_bitloops_hooks(notification);

        Self::marshal_gemini_hook_type(&mut raw_hooks, "SessionStart", &session_start);
        Self::marshal_gemini_hook_type(&mut raw_hooks, "SessionEnd", &session_end);
        Self::marshal_gemini_hook_type(&mut raw_hooks, "BeforeAgent", &before_agent);
        Self::marshal_gemini_hook_type(&mut raw_hooks, "AfterAgent", &after_agent);
        Self::marshal_gemini_hook_type(&mut raw_hooks, "BeforeModel", &before_model);
        Self::marshal_gemini_hook_type(&mut raw_hooks, "AfterModel", &after_model);
        Self::marshal_gemini_hook_type(
            &mut raw_hooks,
            "BeforeToolSelection",
            &before_tool_selection,
        );
        Self::marshal_gemini_hook_type(&mut raw_hooks, "BeforeTool", &before_tool);
        Self::marshal_gemini_hook_type(&mut raw_hooks, "AfterTool", &after_tool);
        Self::marshal_gemini_hook_type(&mut raw_hooks, "PreCompress", &pre_compress);
        Self::marshal_gemini_hook_type(&mut raw_hooks, "Notification", &notification);

        if raw_hooks.is_empty() {
            raw_settings.remove("hooks");
        } else {
            raw_settings.insert("hooks".to_string(), Value::Object(raw_hooks));
        }

        let output = serde_json::to_vec_pretty(&raw_settings)
            .map_err(|err| anyhow!("failed to marshal settings: {err}"))?;
        std::fs::write(&settings_path, output)
            .map_err(|err| anyhow!("failed to write settings.json: {err}"))?;

        crate::adapters::agents::gemini::skills::uninstall_repo_skill(repo_root)?;

        Ok(())
    }

    pub(crate) fn are_hooks_installed_at(&self, repo_root: &Path) -> bool {
        let settings_path = self.settings_path_at(repo_root);
        let data = match std::fs::read(settings_path) {
            Ok(data) => data,
            Err(_) => return false,
        };

        let settings: GeminiSettings = match serde_json::from_slice(&data) {
            Ok(settings) => settings,
            Err(_) => return false,
        };

        Self::has_bitloops_hook(&settings.hooks.session_start)
            || Self::has_bitloops_hook(&settings.hooks.session_end)
            || Self::has_bitloops_hook(&settings.hooks.before_agent)
            || Self::has_bitloops_hook(&settings.hooks.after_agent)
            || Self::has_bitloops_hook(&settings.hooks.before_model)
            || Self::has_bitloops_hook(&settings.hooks.after_model)
            || Self::has_bitloops_hook(&settings.hooks.before_tool_selection)
            || Self::has_bitloops_hook(&settings.hooks.before_tool)
            || Self::has_bitloops_hook(&settings.hooks.after_tool)
            || Self::has_bitloops_hook(&settings.hooks.pre_compress)
            || Self::has_bitloops_hook(&settings.hooks.notification)
    }

    pub(crate) fn settings_path_at(&self, repo_root: &Path) -> PathBuf {
        repo_root.join(".gemini").join(GEMINI_SETTINGS_FILE_NAME)
    }

    pub(crate) fn is_bitloops_hook(command: &str) -> bool {
        crate::adapters::agents::is_managed_hook_command(command, &BITLOOPS_HOOK_PREFIXES)
    }

    pub(crate) fn parse_gemini_hook_type(
        raw_hooks: &Map<String, Value>,
        hook_type: &str,
    ) -> Vec<GeminiHookMatcher> {
        raw_hooks
            .get(hook_type)
            .and_then(|value| serde_json::from_value::<Vec<GeminiHookMatcher>>(value.clone()).ok())
            .unwrap_or_default()
    }

    pub(crate) fn marshal_gemini_hook_type(
        raw_hooks: &mut Map<String, Value>,
        hook_type: &str,
        matchers: &[GeminiHookMatcher],
    ) {
        if matchers.is_empty() {
            raw_hooks.remove(hook_type);
            return;
        }

        if let Ok(value) = serde_json::to_value(matchers) {
            raw_hooks.insert(hook_type.to_string(), value);
        }
    }

    pub(crate) fn remove_bitloops_hooks(
        matchers: Vec<GeminiHookMatcher>,
    ) -> Vec<GeminiHookMatcher> {
        let mut result = Vec::new();
        for mut matcher in matchers {
            matcher
                .hooks
                .retain(|hook| !Self::is_bitloops_hook(&hook.command));
            if !matcher.hooks.is_empty() {
                result.push(matcher);
            }
        }
        result
    }

    pub(crate) fn add_gemini_hook(
        mut matchers: Vec<GeminiHookMatcher>,
        matcher_name: &str,
        hook_name: &str,
        command: String,
    ) -> Vec<GeminiHookMatcher> {
        let entry = GeminiHookEntry {
            name: hook_name.to_string(),
            kind: "command".to_string(),
            command,
        };

        if let Some(existing) = matchers.iter_mut().find(|m| m.matcher == matcher_name) {
            existing.hooks.push(entry);
            return matchers;
        }

        matchers.push(GeminiHookMatcher {
            matcher: matcher_name.to_string(),
            hooks: vec![entry],
        });
        matchers
    }

    pub(crate) fn has_bitloops_hook(matchers: &[GeminiHookMatcher]) -> bool {
        for matcher in matchers {
            for hook in &matcher.hooks {
                if Self::is_bitloops_hook(&hook.command) {
                    return true;
                }
            }
        }
        false
    }

    pub(crate) fn get_first_bitloops_hook_command(matchers: &[GeminiHookMatcher]) -> String {
        for matcher in matchers {
            for hook in &matcher.hooks {
                if Self::is_bitloops_hook(&hook.command) {
                    return hook.command.clone();
                }
            }
        }
        String::new()
    }
}
