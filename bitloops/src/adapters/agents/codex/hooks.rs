use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Result, anyhow};
use serde_json::{Map, Value};

use super::config::{codex_hooks_feature_enabled_at, ensure_codex_hooks_feature_enabled_at};

const HOOKS_FILE_NAME: &str = "hooks.json";

const HOOK_KEY_SESSION_START: &str = "SessionStart";
const HOOK_KEY_USER_PROMPT_SUBMIT: &str = "UserPromptSubmit";
const HOOK_KEY_PRE_TOOL_USE: &str = "PreToolUse";
const HOOK_KEY_POST_TOOL_USE: &str = "PostToolUse";
const HOOK_KEY_STOP: &str = "Stop";

const BITLOOPS_HOOK_PREFIX: &str = "bitloops hooks codex ";
const LOCAL_DEV_HOOK_PREFIX: &str = "cargo run -- hooks codex ";
const MANAGED_HOOK_PREFIXES: [&str; 2] = [BITLOOPS_HOOK_PREFIX, LOCAL_DEV_HOOK_PREFIX];

const STATUS_MESSAGE_SESSION_START: &str = "Initializing session...";
const STATUS_MESSAGE_USER_PROMPT_SUBMIT: &str = "Submitting prompt...";
const STATUS_MESSAGE_PRE_TOOL_USE: &str = "Preparing tool call...";
const STATUS_MESSAGE_POST_TOOL_USE: &str = "Processing tool response...";
const STATUS_MESSAGE_STOP: &str = "Wrapping up turn...";
const TOOL_MATCHER_BASH: &str = "Bash";
const HOOK_TIMEOUT_SECONDS: i64 = 10;

fn managed_hook_keys() -> [&'static str; 5] {
    [
        HOOK_KEY_SESSION_START,
        HOOK_KEY_USER_PROMPT_SUBMIT,
        HOOK_KEY_PRE_TOOL_USE,
        HOOK_KEY_POST_TOOL_USE,
        HOOK_KEY_STOP,
    ]
}

fn hook_commands(
    local_dev: bool,
) -> [(&'static str, String, &'static str, Option<&'static str>); 5] {
    let prefix = if local_dev {
        LOCAL_DEV_HOOK_PREFIX
    } else {
        BITLOOPS_HOOK_PREFIX
    };
    [
        (
            HOOK_KEY_SESSION_START,
            crate::adapters::agents::managed_hook_command(&format!(
                "{prefix}{}",
                crate::adapters::agents::codex::lifecycle::HOOK_NAME_SESSION_START
            )),
            STATUS_MESSAGE_SESSION_START,
            None,
        ),
        (
            HOOK_KEY_USER_PROMPT_SUBMIT,
            crate::adapters::agents::managed_hook_command(&format!("{prefix}user-prompt-submit")),
            STATUS_MESSAGE_USER_PROMPT_SUBMIT,
            None,
        ),
        (
            HOOK_KEY_PRE_TOOL_USE,
            crate::adapters::agents::managed_hook_command(&format!("{prefix}pre-tool-use")),
            STATUS_MESSAGE_PRE_TOOL_USE,
            Some(TOOL_MATCHER_BASH),
        ),
        (
            HOOK_KEY_POST_TOOL_USE,
            crate::adapters::agents::managed_hook_command(&format!("{prefix}post-tool-use")),
            STATUS_MESSAGE_POST_TOOL_USE,
            Some(TOOL_MATCHER_BASH),
        ),
        (
            HOOK_KEY_STOP,
            crate::adapters::agents::managed_hook_command(&format!(
                "{prefix}{}",
                crate::adapters::agents::codex::lifecycle::HOOK_NAME_STOP
            )),
            STATUS_MESSAGE_STOP,
            None,
        ),
    ]
}

fn resolve_repo_root() -> Result<PathBuf> {
    crate::utils::paths::repo_root().or_else(|_| {
        std::env::current_dir().map_err(|err| anyhow!("failed to get current directory: {err}"))
    })
}

fn hooks_file_path_for(repo_root: &Path) -> PathBuf {
    repo_root.join(".codex").join(HOOKS_FILE_NAME)
}

fn parse_top_level_map(data: &[u8]) -> Result<Map<String, Value>> {
    let value: Value =
        serde_json::from_slice(data).map_err(|err| anyhow!("failed to parse hooks.json: {err}"))?;
    let Some(map) = value.as_object() else {
        return Err(anyhow!("failed to parse hooks.json: expected JSON object"));
    };
    Ok(map.clone())
}

fn write_hooks_file(path: &Path, root: &Map<String, Value>) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|err| anyhow!("failed to create .codex directory: {err}"))?;
    }

    let mut output = serde_json::to_string_pretty(&Value::Object(root.clone()))
        .map_err(|err| anyhow!("failed to marshal hooks.json: {err}"))?;
    output.push('\n');
    fs::write(path, output).map_err(|err| anyhow!("failed to write hooks.json: {err}"))?;
    Ok(())
}

fn parse_hook_entries(raw_hooks: &Map<String, Value>, hook_key: &str) -> Vec<Value> {
    raw_hooks
        .get(hook_key)
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

fn marshal_hook_entries(raw_hooks: &mut Map<String, Value>, hook_key: &str, entries: Vec<Value>) {
    if entries.is_empty() {
        raw_hooks.remove(hook_key);
    } else {
        raw_hooks.insert(hook_key.to_string(), Value::Array(entries));
    }
}

fn is_bitloops_hook(command: &str) -> bool {
    crate::adapters::agents::is_managed_hook_command(command, &MANAGED_HOOK_PREFIXES)
}

fn command_of(entry: &Value) -> Option<&str> {
    entry
        .as_object()
        .and_then(|obj| obj.get("command"))
        .and_then(Value::as_str)
}

fn managed_hook_value(command: &str, status_message: &str, matcher: Option<&str>) -> Value {
    let mut hook = Map::new();
    hook.insert("type".to_string(), Value::String("command".to_string()));
    hook.insert("command".to_string(), Value::String(command.to_string()));
    hook.insert(
        "statusMessage".to_string(),
        Value::String(status_message.to_string()),
    );
    hook.insert(
        "timeout".to_string(),
        Value::Number(HOOK_TIMEOUT_SECONDS.into()),
    );

    let mut entry = Map::new();
    if let Some(matcher) = matcher {
        entry.insert("matcher".to_string(), Value::String(matcher.to_string()));
    }
    entry.insert("hooks".to_string(), Value::Array(vec![Value::Object(hook)]));
    Value::Object(entry)
}

fn strip_managed_from_entries(
    entries: &mut Vec<Value>,
    desired_command: &str,
    keep_one_desired: bool,
    desired_kept: &mut bool,
) {
    let mut idx = 0usize;
    while idx < entries.len() {
        let should_remove = strip_managed_from_value(
            &mut entries[idx],
            desired_command,
            keep_one_desired,
            desired_kept,
        );
        if should_remove {
            entries.remove(idx);
        } else {
            idx += 1;
        }
    }
}

fn strip_managed_from_value(
    value: &mut Value,
    desired_command: &str,
    keep_one_desired: bool,
    desired_kept: &mut bool,
) -> bool {
    if let Some(command) = value.as_str()
        && is_bitloops_hook(command)
    {
        if keep_one_desired && command == desired_command && !*desired_kept {
            *desired_kept = true;
            return false;
        }
        return true;
    }

    if let Some(command) = command_of(value)
        && is_bitloops_hook(command)
    {
        if keep_one_desired && command == desired_command && !*desired_kept {
            *desired_kept = true;
            return false;
        }
        return true;
    }

    let Some(obj) = value.as_object_mut() else {
        return false;
    };

    let Some(hooks_value) = obj.get_mut("hooks") else {
        return false;
    };

    let Some(hooks_array) = hooks_value.as_array_mut() else {
        return false;
    };

    let mut hooks = hooks_array.clone();
    strip_managed_from_entries(&mut hooks, desired_command, keep_one_desired, desired_kept);
    *hooks_array = hooks;

    if hooks_array.is_empty() {
        let removable_matcher = obj.keys().all(|key| key == "hooks" || key == "matcher");
        if removable_matcher {
            return true;
        }
    }

    false
}

fn normalize_entries_for_install(
    entries: &mut Vec<Value>,
    desired_command: &str,
    status_message: &str,
    matcher: Option<&'static str>,
    force: bool,
) -> (bool, bool) {
    let before = entries.clone();
    let mut desired_kept = false;
    let mut inserted = false;

    strip_managed_from_entries(entries, desired_command, !force, &mut desired_kept);

    if !desired_kept {
        entries.push(managed_hook_value(desired_command, status_message, matcher));
        inserted = true;
    }

    (*entries != before, inserted)
}

pub fn install_hooks_at(repo_root: &Path, local_dev: bool, force: bool) -> Result<usize> {
    ensure_codex_hooks_feature_enabled_at(repo_root)?;
    let path = hooks_file_path_for(repo_root);
    let existing_data = fs::read(&path).ok();

    let mut raw_file = match existing_data {
        Some(data) => parse_top_level_map(&data)?,
        None => Map::new(),
    };

    let mut raw_hooks = raw_file
        .get("hooks")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();

    let mut installed = 0usize;
    let mut changed = false;

    for (hook_key, command, status_message, matcher) in hook_commands(local_dev) {
        let mut entries = parse_hook_entries(&raw_hooks, hook_key);
        let (hook_changed, inserted) =
            normalize_entries_for_install(&mut entries, &command, status_message, matcher, force);
        if hook_changed {
            changed = true;
        }
        if inserted {
            installed += 1;
        }

        marshal_hook_entries(&mut raw_hooks, hook_key, entries);
    }

    if !changed {
        return Ok(installed);
    }

    raw_file.insert("hooks".to_string(), Value::Object(raw_hooks));
    write_hooks_file(&path, &raw_file)?;

    Ok(installed)
}

pub fn install_hooks(local_dev: bool, force: bool) -> Result<usize> {
    let repo_root = resolve_repo_root()?;
    install_hooks_at(&repo_root, local_dev, force)
}

pub fn uninstall_hooks_at(repo_root: &Path) -> Result<()> {
    let path = hooks_file_path_for(repo_root);
    let data = match fs::read(&path) {
        Ok(data) => data,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(err) => return Err(anyhow!("failed to read hooks.json: {err}")),
    };

    let mut raw_file = parse_top_level_map(&data)?;
    let mut raw_hooks = raw_file
        .get("hooks")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();

    let mut changed = false;

    for hook_key in managed_hook_keys() {
        let mut entries = parse_hook_entries(&raw_hooks, hook_key);
        let before = entries.clone();
        let mut desired_kept = false;
        strip_managed_from_entries(&mut entries, "", false, &mut desired_kept);

        if entries != before {
            changed = true;
        }
        marshal_hook_entries(&mut raw_hooks, hook_key, entries);
    }

    if raw_hooks.is_empty() {
        if raw_file.remove("hooks").is_some() {
            changed = true;
        }
    } else {
        raw_file.insert("hooks".to_string(), Value::Object(raw_hooks));
    }

    if !changed {
        return Ok(());
    }

    write_hooks_file(&path, &raw_file)
}

pub fn uninstall_hooks() -> Result<()> {
    let repo_root = resolve_repo_root()?;
    uninstall_hooks_at(&repo_root)
}

pub fn are_hooks_installed_at(repo_root: &Path) -> bool {
    if !codex_hooks_feature_enabled_at(repo_root) {
        return false;
    }

    let path = hooks_file_path_for(repo_root);
    let Ok(data) = fs::read(&path) else {
        return false;
    };

    let Ok(parsed) = serde_json::from_slice::<super::types::CodexHooksFile>(&data) else {
        return false;
    };

    [
        (parsed.hooks.session_start.as_slice(), None),
        (parsed.hooks.user_prompt_submit.as_slice(), None),
        (
            parsed.hooks.pre_tool_use.as_slice(),
            Some(TOOL_MATCHER_BASH),
        ),
        (
            parsed.hooks.post_tool_use.as_slice(),
            Some(TOOL_MATCHER_BASH),
        ),
        (parsed.hooks.stop.as_slice(), None),
    ]
    .into_iter()
    .all(|(entries, matcher)| {
        entries.iter().any(|entry| {
            let matcher_matches = match matcher {
                Some(expected) => entry.matcher == expected,
                None => true,
            };
            matcher_matches
                && entry
                    .hooks
                    .iter()
                    .any(|hook| is_bitloops_hook(hook.command.as_str()))
        })
    })
}

pub fn are_hooks_installed() -> bool {
    let repo_root = match resolve_repo_root() {
        Ok(repo_root) => repo_root,
        Err(_) => return false,
    };
    are_hooks_installed_at(&repo_root)
}

#[cfg(test)]
#[path = "hooks_tests.rs"]
mod tests;
