use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Result, anyhow};
use serde_json::{Map, Value, json};

use super::types::CopilotHooksFile;

const HOOKS_DIR: &str = ".github/hooks";
const HOOKS_FILE_NAME: &str = "bitloops.json";
const BITLOOPS_HOOK_PREFIX: &str = "bitloops hooks copilot ";
const LOCAL_DEV_HOOK_PREFIX: &str = "cargo run -- hooks copilot ";
const MANAGED_HOOK_PREFIXES: [&str; 2] = [BITLOOPS_HOOK_PREFIX, LOCAL_DEV_HOOK_PREFIX];

const HOOK_TYPE_USER_PROMPT_SUBMITTED: &str = "userPromptSubmitted";
const HOOK_TYPE_SESSION_START: &str = "sessionStart";
const HOOK_TYPE_AGENT_STOP: &str = "agentStop";
const HOOK_TYPE_SESSION_END: &str = "sessionEnd";
const HOOK_TYPE_SUBAGENT_STOP: &str = "subagentStop";
const HOOK_TYPE_PRE_TOOL_USE: &str = "preToolUse";
const HOOK_TYPE_POST_TOOL_USE: &str = "postToolUse";
const HOOK_TYPE_ERROR_OCCURRED: &str = "errorOccurred";

fn required_hook_types() -> [&'static str; 4] {
    [
        HOOK_TYPE_USER_PROMPT_SUBMITTED,
        HOOK_TYPE_SESSION_START,
        HOOK_TYPE_AGENT_STOP,
        HOOK_TYPE_SESSION_END,
    ]
}

fn managed_hook_types() -> [&'static str; 8] {
    [
        HOOK_TYPE_USER_PROMPT_SUBMITTED,
        HOOK_TYPE_SESSION_START,
        HOOK_TYPE_AGENT_STOP,
        HOOK_TYPE_SESSION_END,
        HOOK_TYPE_SUBAGENT_STOP,
        HOOK_TYPE_PRE_TOOL_USE,
        HOOK_TYPE_POST_TOOL_USE,
        HOOK_TYPE_ERROR_OCCURRED,
    ]
}

fn daemon_config_override_from_env() -> Option<PathBuf> {
    std::env::var_os(crate::config::ENV_DAEMON_CONFIG_PATH_OVERRIDE).map(PathBuf::from)
}

fn hook_commands_with_daemon_config_override(
    local_dev: bool,
    daemon_config_override: Option<&Path>,
) -> [(&'static str, String); 8] {
    let prefix = if local_dev {
        LOCAL_DEV_HOOK_PREFIX
    } else {
        BITLOOPS_HOOK_PREFIX
    };

    [
        (
            HOOK_TYPE_USER_PROMPT_SUBMITTED,
            crate::adapters::agents::managed_hook_command_with_daemon_config(
                &format!(
                    "{prefix}{}",
                    crate::adapters::agents::copilot::lifecycle::HOOK_NAME_USER_PROMPT_SUBMITTED
                ),
                daemon_config_override,
            ),
        ),
        (
            HOOK_TYPE_SESSION_START,
            crate::adapters::agents::managed_hook_command_with_daemon_config(
                &format!(
                    "{prefix}{}",
                    crate::adapters::agents::copilot::lifecycle::HOOK_NAME_SESSION_START
                ),
                daemon_config_override,
            ),
        ),
        (
            HOOK_TYPE_AGENT_STOP,
            crate::adapters::agents::managed_hook_command_with_daemon_config(
                &format!(
                    "{prefix}{}",
                    crate::adapters::agents::copilot::lifecycle::HOOK_NAME_AGENT_STOP
                ),
                daemon_config_override,
            ),
        ),
        (
            HOOK_TYPE_SESSION_END,
            crate::adapters::agents::managed_hook_command_with_daemon_config(
                &format!(
                    "{prefix}{}",
                    crate::adapters::agents::copilot::lifecycle::HOOK_NAME_SESSION_END
                ),
                daemon_config_override,
            ),
        ),
        (
            HOOK_TYPE_SUBAGENT_STOP,
            crate::adapters::agents::managed_hook_command_with_daemon_config(
                &format!(
                    "{prefix}{}",
                    crate::adapters::agents::copilot::lifecycle::HOOK_NAME_SUBAGENT_STOP
                ),
                daemon_config_override,
            ),
        ),
        (
            HOOK_TYPE_PRE_TOOL_USE,
            crate::adapters::agents::managed_hook_command_with_daemon_config(
                &format!(
                    "{prefix}{}",
                    crate::adapters::agents::copilot::lifecycle::HOOK_NAME_PRE_TOOL_USE
                ),
                daemon_config_override,
            ),
        ),
        (
            HOOK_TYPE_POST_TOOL_USE,
            crate::adapters::agents::managed_hook_command_with_daemon_config(
                &format!(
                    "{prefix}{}",
                    crate::adapters::agents::copilot::lifecycle::HOOK_NAME_POST_TOOL_USE
                ),
                daemon_config_override,
            ),
        ),
        (
            HOOK_TYPE_ERROR_OCCURRED,
            crate::adapters::agents::managed_hook_command_with_daemon_config(
                &format!(
                    "{prefix}{}",
                    crate::adapters::agents::copilot::lifecycle::HOOK_NAME_ERROR_OCCURRED
                ),
                daemon_config_override,
            ),
        ),
    ]
}

fn hooks_file_path_at(repo_root: &Path) -> PathBuf {
    repo_root.join(HOOKS_DIR).join(HOOKS_FILE_NAME)
}

fn hooks_file_path() -> Result<PathBuf> {
    let repo_root = crate::utils::paths::repo_root().or_else(|_| {
        std::env::current_dir().map_err(|err| anyhow!("failed to get current directory: {err}"))
    })?;
    Ok(hooks_file_path_at(&repo_root))
}

fn parse_top_level_map(data: &[u8]) -> Result<Map<String, Value>> {
    let value: Value = serde_json::from_slice(data)
        .map_err(|err| anyhow!("failed to parse {HOOKS_FILE_NAME}: {err}"))?;
    let Some(map) = value.as_object() else {
        return Err(anyhow!(
            "failed to parse {HOOKS_FILE_NAME}: expected JSON object"
        ));
    };
    Ok(map.clone())
}

fn bash_of(entry: &Value) -> Option<&str> {
    entry
        .as_object()
        .and_then(|obj| obj.get("bash"))
        .and_then(Value::as_str)
}

fn is_bitloops_hook(command: &str) -> bool {
    crate::adapters::agents::is_managed_hook_command(command, &MANAGED_HOOK_PREFIXES)
}

fn remove_managed_hooks(entries: Vec<Value>) -> Vec<Value> {
    entries
        .into_iter()
        .filter(|entry| !bash_of(entry).is_some_and(is_bitloops_hook))
        .collect()
}

fn normalize_hook_entries_for_install(
    entries: Vec<Value>,
    command: &str,
    force: bool,
) -> Vec<Value> {
    let mut normalized = Vec::with_capacity(entries.len());
    let mut kept_target = false;

    for entry in entries {
        let Some(entry_command) = bash_of(&entry) else {
            normalized.push(entry);
            continue;
        };

        if !is_bitloops_hook(entry_command) {
            normalized.push(entry);
            continue;
        }

        if force {
            continue;
        }

        if entry_command == command && !kept_target {
            kept_target = true;
            normalized.push(entry);
        }
    }

    normalized
}

fn parse_hook_entries(raw_hooks: &Map<String, Value>, hook_type: &str) -> Vec<Value> {
    raw_hooks
        .get(hook_type)
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

fn marshal_hook_entries(raw_hooks: &mut Map<String, Value>, hook_type: &str, entries: Vec<Value>) {
    if entries.is_empty() {
        raw_hooks.remove(hook_type);
    } else {
        raw_hooks.insert(hook_type.to_string(), Value::Array(entries));
    }
}

fn has_command(entries: &[Value], command: &str) -> bool {
    entries.iter().any(|entry| bash_of(entry) == Some(command))
}

fn has_managed_hook(entries: &[super::types::CopilotHookEntry]) -> bool {
    entries.iter().any(|entry| is_bitloops_hook(&entry.bash))
}

pub fn install_hooks(local_dev: bool, force: bool) -> Result<usize> {
    let path = hooks_file_path()?;
    install_hooks_at_path(&path, local_dev, force, true)
}

pub fn install_hooks_at(repo_root: &Path, local_dev: bool, force: bool) -> Result<usize> {
    install_hooks_at_with_bitloops_skill(repo_root, local_dev, force, true)
}

pub fn install_hooks_at_with_bitloops_skill(
    repo_root: &Path,
    local_dev: bool,
    force: bool,
    install_bitloops_skill: bool,
) -> Result<usize> {
    install_hooks_at_path(
        &hooks_file_path_at(repo_root),
        local_dev,
        force,
        install_bitloops_skill,
    )
}

fn install_hooks_at_path(
    path: &Path,
    local_dev: bool,
    force: bool,
    install_bitloops_skill: bool,
) -> Result<usize> {
    let repo_root = path
        .parent()
        .and_then(|parent| parent.parent())
        .and_then(|parent| parent.parent())
        .unwrap_or_else(|| Path::new("."));
    if install_bitloops_skill {
        crate::adapters::agents::copilot::skills::install_repo_skill(repo_root)?;
    } else {
        crate::adapters::agents::copilot::skills::uninstall_repo_skill(repo_root)?;
    }

    let daemon_config_override = daemon_config_override_from_env();
    install_hooks_at_path_with_daemon_config_override(
        path,
        local_dev,
        force,
        daemon_config_override.as_deref(),
    )
}

fn install_hooks_at_path_with_daemon_config_override(
    path: &Path,
    local_dev: bool,
    force: bool,
    daemon_config_override: Option<&Path>,
) -> Result<usize> {
    let existing_data = fs::read(path).ok();
    let mut raw_file = match existing_data {
        Some(data) => parse_top_level_map(&data)?,
        None => Map::new(),
    };

    raw_file
        .entry("version".to_string())
        .or_insert_with(|| Value::Number(1.into()));

    let mut raw_hooks = raw_file
        .get("hooks")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();

    let mut installed = 0usize;
    let mut changed = false;

    for (hook_type, command) in
        hook_commands_with_daemon_config_override(local_dev, daemon_config_override)
    {
        let existing = parse_hook_entries(&raw_hooks, hook_type);
        let mut normalized = normalize_hook_entries_for_install(existing.clone(), &command, force);
        if !has_command(&normalized, &command) {
            installed += 1;
            normalized.push(json!({
                "type": "command",
                "bash": command,
                "comment": "Bitloops CLI"
            }));
        }
        if normalized != existing {
            changed = true;
        }
        marshal_hook_entries(&mut raw_hooks, hook_type, normalized);
    }

    if !changed {
        return Ok(installed);
    }

    raw_file.insert("hooks".to_string(), Value::Object(raw_hooks));

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|err| anyhow!("failed to create {HOOKS_DIR}: {err}"))?;
    }

    let mut output = serde_json::to_string_pretty(&Value::Object(raw_file))
        .map_err(|err| anyhow!("failed to marshal {HOOKS_FILE_NAME}: {err}"))?;
    output.push('\n');
    fs::write(path, output).map_err(|err| anyhow!("failed to write {HOOKS_FILE_NAME}: {err}"))?;
    Ok(installed)
}

pub fn uninstall_hooks() -> Result<()> {
    let path = hooks_file_path()?;
    uninstall_hooks_at_path(&path)
}

pub fn uninstall_hooks_at(repo_root: &Path) -> Result<()> {
    uninstall_hooks_at_path(&hooks_file_path_at(repo_root))
}

fn uninstall_hooks_at_path(path: &Path) -> Result<()> {
    let repo_root = path
        .parent()
        .and_then(|parent| parent.parent())
        .and_then(|parent| parent.parent())
        .unwrap_or_else(|| Path::new("."));
    crate::adapters::agents::copilot::skills::uninstall_repo_skill(repo_root)?;

    let data = match fs::read(path) {
        Ok(data) => data,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(err) => return Err(anyhow!("failed to read {HOOKS_FILE_NAME}: {err}")),
    };

    let mut raw_file = parse_top_level_map(&data)?;
    let mut raw_hooks = raw_file
        .get("hooks")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();

    for hook_type in managed_hook_types() {
        let entries = parse_hook_entries(&raw_hooks, hook_type);
        marshal_hook_entries(&mut raw_hooks, hook_type, remove_managed_hooks(entries));
    }

    if raw_hooks.is_empty() {
        raw_file.remove("hooks");
    } else {
        raw_file.insert("hooks".to_string(), Value::Object(raw_hooks));
    }

    let mut output = serde_json::to_string_pretty(&Value::Object(raw_file))
        .map_err(|err| anyhow!("failed to marshal {HOOKS_FILE_NAME}: {err}"))?;
    output.push('\n');
    fs::write(path, output).map_err(|err| anyhow!("failed to write {HOOKS_FILE_NAME}: {err}"))?;
    Ok(())
}

pub fn are_hooks_installed() -> bool {
    let path = match hooks_file_path() {
        Ok(path) => path,
        Err(_) => return false,
    };
    are_hooks_installed_at_path(&path)
}

pub fn are_hooks_installed_at(repo_root: &Path) -> bool {
    are_hooks_installed_at_path(&hooks_file_path_at(repo_root))
}

fn are_hooks_installed_at_path(path: &Path) -> bool {
    let Ok(data) = fs::read(path) else {
        return false;
    };
    let Ok(parsed) = serde_json::from_slice::<CopilotHooksFile>(&data) else {
        return false;
    };

    required_hook_types()
        .into_iter()
        .all(|hook_type| match hook_type {
            HOOK_TYPE_USER_PROMPT_SUBMITTED => {
                has_managed_hook(&parsed.hooks.user_prompt_submitted)
            }
            HOOK_TYPE_SESSION_START => has_managed_hook(&parsed.hooks.session_start),
            HOOK_TYPE_AGENT_STOP => has_managed_hook(&parsed.hooks.agent_stop),
            HOOK_TYPE_SESSION_END => has_managed_hook(&parsed.hooks.session_end),
            _ => false,
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    fn init_repo(path: &std::path::Path) {
        let output = std::process::Command::new("git")
            .args(["init", "-q"])
            .current_dir(path)
            .output()
            .expect("git init");
        assert!(output.status.success(), "git init failed");
    }

    fn read_repo_skill(dir: &tempfile::TempDir) -> Option<String> {
        fs::read_to_string(
            dir.path()
                .join(".github")
                .join("skills")
                .join("bitloops")
                .join("devql-explore-first")
                .join("SKILL.md"),
        )
        .ok()
    }

    #[test]
    fn install_hooks_canonical_fresh_and_idempotent() {
        let dir = tempfile::tempdir().expect("tempdir");
        init_repo(dir.path());
        let count = install_hooks_at_path_with_daemon_config_override(
            &hooks_file_path_at(dir.path()),
            false,
            false,
            None,
        )
        .expect("install");
        assert_eq!(count, 8);
        assert!(are_hooks_installed_at(dir.path()));

        let second = install_hooks_at_path_with_daemon_config_override(
            &hooks_file_path_at(dir.path()),
            false,
            false,
            None,
        )
        .expect("install second");
        assert_eq!(second, 0);
    }

    #[test]
    fn install_hooks_writes_copilot_repo_skill() {
        let dir = tempfile::tempdir().expect("tempdir");
        init_repo(dir.path());

        install_hooks_at(dir.path(), false, false).expect("install");

        let skill = read_repo_skill(&dir).expect("repo skill should be installed");
        assert_eq!(
            skill,
            crate::host::hooks::augmentation::skill_content::DEVQL_EXPLORE_FIRST_SKILL
        );
    }

    #[test]
    fn uninstall_hooks_removes_copilot_repo_skill() {
        let dir = tempfile::tempdir().expect("tempdir");
        init_repo(dir.path());

        install_hooks_at(dir.path(), false, false).expect("install");
        assert!(
            read_repo_skill(&dir).is_some(),
            "repo skill should exist before uninstall"
        );

        uninstall_hooks_at(dir.path()).expect("uninstall");

        assert!(
            read_repo_skill(&dir).is_none(),
            "repo skill should be removed by uninstall"
        );
    }

    #[test]
    fn are_hooks_installed_accepts_missing_optional_hooks() {
        let dir = tempfile::tempdir().expect("tempdir");
        init_repo(dir.path());
        let hooks_dir = dir.path().join(".github/hooks");
        std::fs::create_dir_all(&hooks_dir).expect("hooks dir");
        let content = r#"{
  "version": 1,
  "hooks": {
    "userPromptSubmitted": [{"type":"command","bash":"bitloops hooks copilot user-prompt-submitted"}],
    "sessionStart": [{"type":"command","bash":"bitloops hooks copilot session-start"}],
    "agentStop": [{"type":"command","bash":"bitloops hooks copilot agent-stop"}],
    "sessionEnd": [{"type":"command","bash":"bitloops hooks copilot session-end"}]
  }
}
"#;
        std::fs::write(hooks_dir.join("bitloops.json"), content).expect("write");
        assert!(are_hooks_installed_at(dir.path()));
    }

    #[test]
    fn are_hooks_installed_requires_core_lifecycle_hooks() {
        let dir = tempfile::tempdir().expect("tempdir");
        init_repo(dir.path());
        let hooks_dir = dir.path().join(".github/hooks");
        std::fs::create_dir_all(&hooks_dir).expect("hooks dir");
        let content = r#"{
  "version": 1,
  "hooks": {
    "userPromptSubmitted": [{"type":"command","bash":"bitloops hooks copilot user-prompt-submitted"}],
    "sessionStart": [{"type":"command","bash":"bitloops hooks copilot session-start"}],
    "agentStop": [{"type":"command","bash":"bitloops hooks copilot agent-stop"}]
  }
}
"#;
        std::fs::write(hooks_dir.join("bitloops.json"), content).expect("write");
        assert!(!are_hooks_installed_at(dir.path()));
    }

    #[test]
    fn install_hooks_local_dev_writes_cargo_run_commands() {
        let dir = tempfile::tempdir().expect("tempdir");
        init_repo(dir.path());
        let installed = install_hooks_at_path_with_daemon_config_override(
            &hooks_file_path_at(dir.path()),
            true,
            false,
            None,
        )
        .expect("install");
        assert_eq!(installed, 8);
        let content =
            fs::read_to_string(dir.path().join(".github/hooks/bitloops.json")).expect("read");
        assert!(content.contains("cargo run -- hooks copilot session-start"));
        assert!(!content.contains("bitloops hooks copilot session-start"));
    }

    #[test]
    fn install_hooks_preserves_unknown_fields() {
        let dir = tempfile::tempdir().expect("tempdir");
        init_repo(dir.path());
        let hooks_dir = dir.path().join(".github/hooks");
        fs::create_dir_all(&hooks_dir).expect("mkdir");
        fs::write(
            hooks_dir.join("bitloops.json"),
            r#"{
  "version": 1,
  "customField": {"ok": true},
  "hooks": {
    "customHook": [{"type":"command","bash":"echo custom"}]
  }
}
"#,
        )
        .expect("write");

        install_hooks_at_path_with_daemon_config_override(
            &hooks_file_path_at(dir.path()),
            false,
            false,
            None,
        )
        .expect("install");
        let content = fs::read_to_string(hooks_dir.join("bitloops.json")).expect("read");
        let value: Value = serde_json::from_str(&content).expect("json");
        assert!(value.get("customField").is_some());
        assert!(
            value
                .get("hooks")
                .and_then(Value::as_object)
                .and_then(|hooks| hooks.get("customHook"))
                .is_some()
        );
    }

    #[test]
    fn install_hooks_preserves_user_hooks_alongside_managed_hooks() {
        let dir = tempfile::tempdir().expect("tempdir");
        init_repo(dir.path());
        let hooks_dir = dir.path().join(".github/hooks");
        fs::create_dir_all(&hooks_dir).expect("mkdir");
        fs::write(
            hooks_dir.join("bitloops.json"),
            r#"{
  "version": 1,
  "hooks": {
    "sessionStart": [{"type":"command","bash":"echo custom-session-start"}]
  }
}
"#,
        )
        .expect("write");

        install_hooks_at_path_with_daemon_config_override(
            &hooks_file_path_at(dir.path()),
            false,
            false,
            None,
        )
        .expect("install");
        let content = fs::read_to_string(hooks_dir.join("bitloops.json")).expect("read");
        assert!(content.contains("echo custom-session-start"));
        assert!(content.contains("bitloops hooks copilot session-start"));
    }

    #[test]
    fn install_hooks_recovers_missing_managed_entries_without_duplicating_existing_ones() {
        let dir = tempfile::tempdir().expect("tempdir");
        init_repo(dir.path());
        let hooks_dir = dir.path().join(".github/hooks");
        fs::create_dir_all(&hooks_dir).expect("mkdir");
        fs::write(
            hooks_dir.join("bitloops.json"),
            r#"{
  "version": 1,
  "hooks": {
    "userPromptSubmitted": [{"type":"command","bash":"bitloops hooks copilot user-prompt-submitted"}],
    "sessionStart": [{"type":"command","bash":"echo custom-session-start"}],
    "customHook": [{"type":"command","bash":"echo custom"}]
  }
}
"#,
        )
        .expect("write");

        let installed = install_hooks_at_path_with_daemon_config_override(
            &hooks_file_path_at(dir.path()),
            false,
            false,
            None,
        )
        .expect("install");
        assert_eq!(installed, 7);

        let content = fs::read_to_string(hooks_dir.join("bitloops.json")).expect("read");
        assert_eq!(
            content
                .matches("bitloops hooks copilot user-prompt-submitted")
                .count(),
            1
        );
        assert!(content.contains("echo custom-session-start"));
        assert!(content.contains("echo custom"));
        assert!(content.contains("bitloops hooks copilot session-end"));
    }

    #[test]
    fn uninstall_preserves_non_bitloops_hooks() {
        let dir = tempfile::tempdir().expect("tempdir");
        init_repo(dir.path());
        install_hooks_at_path_with_daemon_config_override(
            &hooks_file_path_at(dir.path()),
            false,
            false,
            None,
        )
        .expect("install");
        let hooks_path = dir.path().join(".github/hooks/bitloops.json");
        fs::write(
            &hooks_path,
            r#"{
  "version": 1,
  "hooks": {
    "sessionStart": [
      {"type":"command","bash":"bitloops hooks copilot session-start"},
      {"type":"command","bash":"echo custom"}
    ]
  }
}
"#,
        )
        .expect("seed");

        uninstall_hooks_at(dir.path()).expect("uninstall");
        let output = fs::read_to_string(&hooks_path).expect("read");
        assert!(output.contains("echo custom"));
        assert!(!output.contains("bitloops hooks copilot session-start"));
    }
}
