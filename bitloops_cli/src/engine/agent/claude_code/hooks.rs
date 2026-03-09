//! Claude Code hook installation and management.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::fs;
use std::path::{Path, PathBuf};

pub const SETTINGS_FILE_NAME: &str = "settings.json";

/// Deny rule that blocks Claude from reading Bitloops session metadata.
pub const METADATA_DENY_RULE: &str = "Read(./.bitloops/metadata/**)";

/// Prefix that identifies a Bitloops-managed hook command.
const BITLOOPS_HOOK_PREFIX: &str = "bitloops ";

// Hook commands — subcommands of `bitloops hooks claude-code`
const CMD_SESSION_START: &str = "bitloops hooks claude-code session-start";
const CMD_SESSION_END: &str = "bitloops hooks claude-code session-end";
const CMD_STOP: &str = "bitloops hooks claude-code stop";
const CMD_USER_PROMPT_SUBMIT: &str = "bitloops hooks claude-code user-prompt-submit";
const CMD_PRE_TASK: &str = "bitloops hooks claude-code pre-task";
const CMD_POST_TASK: &str = "bitloops hooks claude-code post-task";
const CMD_POST_TODO: &str = "bitloops hooks claude-code post-todo";

/// A single hook entry within a matcher.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClaudeHookEntry {
    #[serde(rename = "type")]
    pub kind: String,
    pub command: String,
}

/// Groups hooks by tool matcher pattern (e.g., "Task", "TodoWrite", or "" for session hooks).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClaudeHookMatcher {
    pub matcher: String,
    pub hooks: Vec<ClaudeHookEntry>,
}

fn claude_settings_path(repo_root: &Path) -> PathBuf {
    repo_root.join(".claude").join(SETTINGS_FILE_NAME)
}

/// Returns true if the command is a Bitloops-managed hook.
fn is_bitloops_hook(command: &str) -> bool {
    command.starts_with(BITLOOPS_HOOK_PREFIX)
}

/// Returns true if `command` appears in any hook of any matcher.
fn hook_command_exists(matchers: &[ClaudeHookMatcher], command: &str) -> bool {
    matchers
        .iter()
        .flat_map(|m| &m.hooks)
        .any(|h| h.command == command)
}

/// Returns true if `command` appears in a matcher with the given `matcher_name`.
fn hook_command_exists_with_matcher(
    matchers: &[ClaudeHookMatcher],
    matcher_name: &str,
    command: &str,
) -> bool {
    matchers
        .iter()
        .filter(|m| m.matcher == matcher_name)
        .flat_map(|m| &m.hooks)
        .any(|h| h.command == command)
}

/// Adds `command` to the matcher with `matcher_name`, creating it if needed.
fn add_hook_to_matcher(
    mut matchers: Vec<ClaudeHookMatcher>,
    matcher_name: &str,
    command: &str,
) -> Vec<ClaudeHookMatcher> {
    let entry = ClaudeHookEntry {
        kind: "command".into(),
        command: command.into(),
    };
    for m in &mut matchers {
        if m.matcher == matcher_name {
            m.hooks.push(entry);
            return matchers;
        }
    }
    matchers.push(ClaudeHookMatcher {
        matcher: matcher_name.into(),
        hooks: vec![entry],
    });
    matchers
}

/// Removes all Bitloops-managed hooks from matchers. Drops matchers that become empty.
fn remove_bitloops_hooks(matchers: Vec<ClaudeHookMatcher>) -> Vec<ClaudeHookMatcher> {
    matchers
        .into_iter()
        .filter_map(|mut m| {
            m.hooks.retain(|h| !is_bitloops_hook(&h.command));
            if m.hooks.is_empty() { None } else { Some(m) }
        })
        .collect()
}

/// Reads a named hook type from the raw hooks map, returning an empty vec if absent.
fn parse_hook_type(raw_hooks: &Map<String, Value>, hook_type: &str) -> Vec<ClaudeHookMatcher> {
    raw_hooks
        .get(hook_type)
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default()
}

/// Writes a hook type back into the raw hooks map.
/// If `matchers` is empty the key is removed (to avoid empty arrays in JSON).
fn set_hook_type(
    raw_hooks: &mut Map<String, Value>,
    hook_type: &str,
    matchers: Vec<ClaudeHookMatcher>,
) {
    if matchers.is_empty() {
        raw_hooks.remove(hook_type);
    } else {
        raw_hooks.insert(
            hook_type.into(),
            serde_json::to_value(matchers).unwrap_or(Value::Null),
        );
    }
}

fn write_settings_file(path: &Path, settings: &Map<String, Value>) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("creating directory: {}", parent.display()))?;
    }
    let mut data = serde_json::to_string_pretty(settings).context("serializing settings")?;
    data.push('\n');
    fs::write(path, data).with_context(|| format!("writing {}", path.display()))?;
    Ok(())
}

/// Installs Bitloops hooks into `.claude/settings.json` (relative to `repo_root`).
/// Returns the number of new hooks added (0 if all already installed).
/// Idempotent: safe to call multiple times.
///
pub fn install_hooks(repo_root: &Path, force: bool) -> Result<usize> {
    let settings_path = claude_settings_path(repo_root);

    // Parse existing settings as a raw map — preserves ALL unknown fields and hook types.
    let mut raw_settings: Map<String, Value> = match fs::read(&settings_path) {
        Ok(data) => serde_json::from_slice(&data).context("parsing existing settings.json")?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Map::new(),
        Err(e) => return Err(e).context("reading settings.json"),
    };

    // Preserve unknown hook types (e.g., "Notification", "SubagentStop").
    let mut raw_hooks: Map<String, Value> = raw_settings
        .get("hooks")
        .and_then(|v| v.as_object().cloned())
        .unwrap_or_default();

    // Preserve unknown permission fields (e.g., "ask", "customField").
    let mut raw_permissions: Map<String, Value> = raw_settings
        .get("permissions")
        .and_then(|v| v.as_object().cloned())
        .unwrap_or_default();

    // Parse only the 6 hook types we manage.
    let mut session_start = parse_hook_type(&raw_hooks, "SessionStart");
    let mut session_end = parse_hook_type(&raw_hooks, "SessionEnd");
    let mut stop = parse_hook_type(&raw_hooks, "Stop");
    let mut user_prompt_submit = parse_hook_type(&raw_hooks, "UserPromptSubmit");
    let mut pre_tool_use = parse_hook_type(&raw_hooks, "PreToolUse");
    let mut post_tool_use = parse_hook_type(&raw_hooks, "PostToolUse");

    if force {
        session_start = remove_bitloops_hooks(session_start);
        session_end = remove_bitloops_hooks(session_end);
        stop = remove_bitloops_hooks(stop);
        user_prompt_submit = remove_bitloops_hooks(user_prompt_submit);
        pre_tool_use = remove_bitloops_hooks(pre_tool_use);
        post_tool_use = remove_bitloops_hooks(post_tool_use);
    }

    let mut count = 0usize;

    // Session hooks use empty matcher; tool-use hooks use named matcher.
    if !hook_command_exists(&session_start, CMD_SESSION_START) {
        session_start = add_hook_to_matcher(session_start, "", CMD_SESSION_START);
        count += 1;
    }
    if !hook_command_exists(&session_end, CMD_SESSION_END) {
        session_end = add_hook_to_matcher(session_end, "", CMD_SESSION_END);
        count += 1;
    }
    if !hook_command_exists(&stop, CMD_STOP) {
        stop = add_hook_to_matcher(stop, "", CMD_STOP);
        count += 1;
    }
    if !hook_command_exists(&user_prompt_submit, CMD_USER_PROMPT_SUBMIT) {
        user_prompt_submit = add_hook_to_matcher(user_prompt_submit, "", CMD_USER_PROMPT_SUBMIT);
        count += 1;
    }
    if !hook_command_exists_with_matcher(&pre_tool_use, "Task", CMD_PRE_TASK) {
        pre_tool_use = add_hook_to_matcher(pre_tool_use, "Task", CMD_PRE_TASK);
        count += 1;
    }
    if !hook_command_exists_with_matcher(&post_tool_use, "Task", CMD_POST_TASK) {
        post_tool_use = add_hook_to_matcher(post_tool_use, "Task", CMD_POST_TASK);
        count += 1;
    }
    if !hook_command_exists_with_matcher(&post_tool_use, "TodoWrite", CMD_POST_TODO) {
        post_tool_use = add_hook_to_matcher(post_tool_use, "TodoWrite", CMD_POST_TODO);
        count += 1;
    }

    // Add metadata deny rule to permissions.deny if not already present.
    let mut deny_rules: Vec<String> = raw_permissions
        .get("deny")
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default();

    let permissions_changed = if !deny_rules.contains(&METADATA_DENY_RULE.to_string()) {
        deny_rules.push(METADATA_DENY_RULE.to_string());
        raw_permissions.insert("deny".into(), serde_json::to_value(&deny_rules).unwrap());
        true
    } else {
        false
    };

    if count == 0 && !permissions_changed {
        return Ok(0); // Nothing changed — skip writing.
    }

    set_hook_type(&mut raw_hooks, "SessionStart", session_start);
    set_hook_type(&mut raw_hooks, "SessionEnd", session_end);
    set_hook_type(&mut raw_hooks, "Stop", stop);
    set_hook_type(&mut raw_hooks, "UserPromptSubmit", user_prompt_submit);
    set_hook_type(&mut raw_hooks, "PreToolUse", pre_tool_use);
    set_hook_type(&mut raw_hooks, "PostToolUse", post_tool_use);

    raw_settings.insert("hooks".into(), Value::Object(raw_hooks));
    raw_settings.insert("permissions".into(), Value::Object(raw_permissions));

    write_settings_file(&settings_path, &raw_settings)?;
    Ok(count)
}

/// Removes all Bitloops hooks from `.claude/settings.json`.
///
pub fn uninstall_hooks(repo_root: &Path) -> Result<()> {
    let settings_path = claude_settings_path(repo_root);

    let data = match fs::read(&settings_path) {
        Ok(d) => d,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e).context("reading settings.json"),
    };

    let mut raw_settings: Map<String, Value> =
        serde_json::from_slice(&data).context("parsing settings.json")?;

    // Preserve unknown hook types.
    let mut raw_hooks: Map<String, Value> = raw_settings
        .get("hooks")
        .and_then(|v| v.as_object().cloned())
        .unwrap_or_default();

    // Remove our hooks from the 6 managed types; unknown types are untouched.
    for hook_type in &[
        "SessionStart",
        "SessionEnd",
        "Stop",
        "UserPromptSubmit",
        "PreToolUse",
        "PostToolUse",
    ] {
        let matchers = parse_hook_type(&raw_hooks, hook_type);
        set_hook_type(&mut raw_hooks, hook_type, remove_bitloops_hooks(matchers));
    }

    if raw_hooks.is_empty() {
        raw_settings.remove("hooks");
    } else {
        raw_settings.insert("hooks".into(), Value::Object(raw_hooks));
    }

    // Remove our deny rule from permissions.deny.
    let perms_is_empty = if let Some(Value::Object(perms)) = raw_settings.get_mut("permissions") {
        if let Some(deny_val) = perms.get("deny")
            && let Ok(mut rules) = serde_json::from_value::<Vec<String>>(deny_val.clone())
        {
            rules.retain(|r| r != METADATA_DENY_RULE);
            if rules.is_empty() {
                perms.remove("deny");
            } else {
                perms.insert("deny".into(), serde_json::to_value(&rules).unwrap());
            }
        }
        perms.is_empty()
    } else {
        false
    };

    if perms_is_empty {
        raw_settings.remove("permissions");
    }

    write_settings_file(&settings_path, &raw_settings)?;
    Ok(())
}

/// Returns true if Bitloops hooks are installed in `.claude/settings.json`.
///
pub fn are_hooks_installed(repo_root: &Path) -> bool {
    let settings_path = claude_settings_path(repo_root);
    let data = match fs::read(&settings_path) {
        Ok(d) => d,
        Err(_) => return false,
    };
    let raw_settings: Map<String, Value> = match serde_json::from_slice(&data) {
        Ok(v) => v,
        Err(_) => return false,
    };
    let raw_hooks = raw_settings
        .get("hooks")
        .and_then(|v| v.as_object().cloned())
        .unwrap_or_default();
    let stop = parse_hook_type(&raw_hooks, "Stop");
    hook_command_exists(&stop, CMD_STOP)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    // ── helpers ──────────────────────────────────────────────────────────────

    fn write_claude_settings(dir: &TempDir, content: &str) {
        let claude_dir = dir.path().join(".claude");
        fs::create_dir_all(&claude_dir).unwrap();
        fs::write(claude_dir.join("settings.json"), content).unwrap();
    }

    /// Reads `permissions.allow` and `permissions.deny` from `.claude/settings.json`.
    fn read_permissions(dir: &TempDir) -> (Vec<String>, Vec<String>) {
        let path = dir.path().join(".claude/settings.json");
        let data = fs::read_to_string(&path).unwrap();
        let v: Value = serde_json::from_str(&data).unwrap();
        let str_vec = |val: &Value| {
            val.as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|x| x.as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default()
        };
        let allow = str_vec(&v["permissions"]["allow"]);
        let deny = str_vec(&v["permissions"]["deny"]);
        (allow, deny)
    }

    /// Reads the top-level `hooks` object as a raw JSON map.
    fn read_raw_hooks(dir: &TempDir) -> Map<String, Value> {
        let path = dir.path().join(".claude/settings.json");
        let data = fs::read_to_string(&path).unwrap();
        let v: Value = serde_json::from_str(&data).unwrap();
        v["hooks"].as_object().cloned().unwrap_or_default()
    }

    /// Reads `hooks.<hook_type>` as a list of `ClaudeHookMatcher`.
    fn read_hook_type(dir: &TempDir, hook_type: &str) -> Vec<ClaudeHookMatcher> {
        let raw = read_raw_hooks(dir);
        parse_hook_type(&raw, hook_type)
    }

    fn assert_hook_exists(
        matchers: &[ClaudeHookMatcher],
        matcher_name: &str,
        command: &str,
        description: &str,
    ) {
        for m in matchers {
            if m.matcher == matcher_name {
                for h in &m.hooks {
                    if h.command == command {
                        return;
                    }
                }
            }
        }
        panic!("{description} not found (matcher={matcher_name:?}, command={command:?})");
    }

    // ── permissions.deny tests ────────────────────────────────────────────────

    #[test]
    fn install_hooks_adds_deny_rule_fresh_install() {
        let dir = tempfile::tempdir().unwrap();
        install_hooks(dir.path(), false).unwrap();

        let (_, deny) = read_permissions(&dir);
        assert!(
            deny.contains(&METADATA_DENY_RULE.to_string()),
            "deny should contain our rule, got: {deny:?}"
        );
    }

    #[test]
    fn install_hooks_deny_rule_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        install_hooks(dir.path(), false).unwrap();
        install_hooks(dir.path(), false).unwrap();

        let (_, deny) = read_permissions(&dir);
        let count = deny
            .iter()
            .filter(|r| r.as_str() == METADATA_DENY_RULE)
            .count();
        assert_eq!(
            count, 1,
            "deny rule should appear exactly once, got: {deny:?}"
        );
    }

    #[test]
    fn install_hooks_preserves_user_deny_rules() {
        let dir = tempfile::tempdir().unwrap();
        write_claude_settings(&dir, r#"{"permissions": {"deny": ["Bash(rm -rf *)"]}}"#);
        install_hooks(dir.path(), false).unwrap();

        let (_, deny) = read_permissions(&dir);
        assert!(
            deny.contains(&"Bash(rm -rf *)".to_string()),
            "user rule preserved"
        );
        assert!(
            deny.contains(&METADATA_DENY_RULE.to_string()),
            "our rule added"
        );
    }

    #[test]
    fn install_hooks_preserves_allow_rules() {
        let dir = tempfile::tempdir().unwrap();
        write_claude_settings(
            &dir,
            r#"{"permissions": {"allow": ["Read(**)", "Write(**)"]}}"#,
        );
        install_hooks(dir.path(), false).unwrap();

        let (allow, _) = read_permissions(&dir);
        assert_eq!(allow.len(), 2);
        assert!(allow.contains(&"Read(**)".to_string()));
        assert!(allow.contains(&"Write(**)".to_string()));
    }

    #[test]
    fn install_hooks_skips_existing_deny_rule() {
        let dir = tempfile::tempdir().unwrap();
        write_claude_settings(
            &dir,
            &format!(r#"{{"permissions": {{"deny": ["{METADATA_DENY_RULE}"]}}}}"#),
        );
        install_hooks(dir.path(), false).unwrap();

        let (_, deny) = read_permissions(&dir);
        assert_eq!(
            deny.len(),
            1,
            "should still have exactly 1 deny rule, got: {deny:?}"
        );
    }

    #[test]
    fn install_hooks_preserves_unknown_permission_fields() {
        let dir = tempfile::tempdir().unwrap();
        write_claude_settings(
            &dir,
            r#"{
  "permissions": {
    "allow": ["Read(**)"],
    "ask": ["Write(**)", "Bash(*)"],
    "customField": {"nested": "value"}
  }
}"#,
        );
        install_hooks(dir.path(), false).unwrap();

        let path = dir.path().join(".claude/settings.json");
        let data = fs::read_to_string(&path).unwrap();
        let v: Value = serde_json::from_str(&data).unwrap();
        let perms = v["permissions"].as_object().unwrap();

        assert!(
            perms.contains_key("ask"),
            "permissions.ask should be preserved"
        );
        assert!(
            perms.contains_key("customField"),
            "permissions.customField should be preserved"
        );

        let ask: Vec<String> = serde_json::from_value(perms["ask"].clone()).unwrap();
        assert_eq!(ask, vec!["Write(**)", "Bash(*)"]);

        let (allow, deny) = read_permissions(&dir);
        assert_eq!(allow, vec!["Read(**)"], "allow should be preserved");
        assert!(
            deny.contains(&METADATA_DENY_RULE.to_string()),
            "deny rule added"
        );
    }

    // ── uninstall tests ───────────────────────────────────────────────────────

    #[test]
    fn uninstall_hooks_removes_all_hooks() {
        let dir = tempfile::tempdir().unwrap();
        install_hooks(dir.path(), false).unwrap();
        assert!(
            are_hooks_installed(dir.path()),
            "hooks should be installed before uninstall"
        );

        uninstall_hooks(dir.path()).unwrap();
        assert!(
            !are_hooks_installed(dir.path()),
            "hooks should not be installed after uninstall"
        );
    }

    #[test]
    fn uninstall_hooks_no_settings_file_is_noop() {
        let dir = tempfile::tempdir().unwrap();
        // No .claude/settings.json — should not error
        uninstall_hooks(dir.path()).unwrap();
    }

    #[test]
    fn uninstall_hooks_preserves_user_hooks() {
        let dir = tempfile::tempdir().unwrap();
        write_claude_settings(
            &dir,
            r#"{
  "hooks": {
    "Stop": [
      {"matcher": "", "hooks": [{"type": "command", "command": "echo user hook"}]},
      {"matcher": "", "hooks": [{"type": "command", "command": "bitloops hooks claude-code stop"}]}
    ]
  }
}"#,
        );
        uninstall_hooks(dir.path()).unwrap();

        let stop = read_hook_type(&dir, "Stop");
        assert_eq!(stop.len(), 1, "only user matcher should remain");
        assert_eq!(stop[0].hooks[0].command, "echo user hook");
    }

    #[test]
    fn uninstall_hooks_removes_deny_rule() {
        let dir = tempfile::tempdir().unwrap();
        install_hooks(dir.path(), false).unwrap();

        let (_, deny) = read_permissions(&dir);
        assert!(
            deny.contains(&METADATA_DENY_RULE.to_string()),
            "deny rule should exist after install"
        );

        uninstall_hooks(dir.path()).unwrap();

        let path = dir.path().join(".claude/settings.json");
        let data = fs::read_to_string(&path).unwrap();
        let v: Value = serde_json::from_str(&data).unwrap();
        // permissions section should be gone (was only our deny rule)
        let deny_after: Vec<String> = v["permissions"]["deny"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();
        assert!(
            !deny_after.contains(&METADATA_DENY_RULE.to_string()),
            "deny rule should be removed after uninstall"
        );
    }

    #[test]
    fn uninstall_hooks_preserves_user_deny_rules() {
        let dir = tempfile::tempdir().unwrap();
        write_claude_settings(
            &dir,
            &format!(
                r#"{{
  "permissions": {{
    "deny": ["Bash(rm -rf *)", "{METADATA_DENY_RULE}"]
  }},
  "hooks": {{
    "Stop": [
      {{"matcher": "", "hooks": [{{"type": "command", "command": "bitloops hooks claude-code stop"}}]}}
    ]
  }}
}}"#
            ),
        );
        uninstall_hooks(dir.path()).unwrap();

        let (_, deny) = read_permissions(&dir);
        assert!(
            deny.contains(&"Bash(rm -rf *)".to_string()),
            "user deny rule should be preserved"
        );
        assert!(
            !deny.contains(&METADATA_DENY_RULE.to_string()),
            "our deny rule should be removed"
        );
    }

    #[test]
    fn install_hooks_preserves_user_hooks_on_same_type() {
        let dir = tempfile::tempdir().unwrap();
        write_claude_settings(
            &dir,
            r#"{
  "hooks": {
    "Stop": [
      {"matcher": "", "hooks": [{"type": "command", "command": "echo user stop hook"}]}
    ],
    "SessionStart": [
      {"matcher": "", "hooks": [{"type": "command", "command": "echo user session start"}]}
    ],
    "PostToolUse": [
      {"matcher": "Write", "hooks": [{"type": "command", "command": "echo user wrote file"}]}
    ]
  }
}"#,
        );
        install_hooks(dir.path(), false).unwrap();

        let stop = read_hook_type(&dir, "Stop");
        assert_hook_exists(&stop, "", "echo user stop hook", "user Stop hook");
        assert_hook_exists(&stop, "", CMD_STOP, "Bitloops Stop hook");

        let session_start = read_hook_type(&dir, "SessionStart");
        assert_hook_exists(
            &session_start,
            "",
            "echo user session start",
            "user SessionStart hook",
        );
        assert_hook_exists(
            &session_start,
            "",
            CMD_SESSION_START,
            "Bitloops SessionStart hook",
        );

        let post_tool_use = read_hook_type(&dir, "PostToolUse");
        assert_hook_exists(
            &post_tool_use,
            "Write",
            "echo user wrote file",
            "user Write hook",
        );
        assert_hook_exists(&post_tool_use, "Task", CMD_POST_TASK, "Bitloops Task hook");
        assert_hook_exists(
            &post_tool_use,
            "TodoWrite",
            CMD_POST_TODO,
            "Bitloops TodoWrite hook",
        );
    }

    #[test]
    fn install_hooks_preserves_unknown_hook_types() {
        let dir = tempfile::tempdir().unwrap();
        write_claude_settings(
            &dir,
            r#"{
  "hooks": {
    "Notification": [
      {"matcher": "", "hooks": [{"type": "command", "command": "echo notification received"}]}
    ],
    "SubagentStop": [
      {"matcher": ".*", "hooks": [{"type": "command", "command": "echo subagent stopped"}]}
    ]
  }
}"#,
        );
        install_hooks(dir.path(), false).unwrap();

        let raw = read_raw_hooks(&dir);
        assert!(
            raw.contains_key("Notification"),
            "Notification hook type should be preserved"
        );
        assert!(
            raw.contains_key("SubagentStop"),
            "SubagentStop hook type should be preserved"
        );

        // Check Notification content
        let notification: Vec<ClaudeHookMatcher> =
            serde_json::from_value(raw["Notification"].clone()).unwrap();
        assert_eq!(notification.len(), 1);
        assert_eq!(
            notification[0].hooks[0].command,
            "echo notification received"
        );

        // Check SubagentStop content
        let subagent_stop: Vec<ClaudeHookMatcher> =
            serde_json::from_value(raw["SubagentStop"].clone()).unwrap();
        assert_eq!(subagent_stop.len(), 1);
        assert_eq!(subagent_stop[0].matcher, ".*");
        assert_eq!(subagent_stop[0].hooks[0].command, "echo subagent stopped");

        // Our Stop hook should also have been installed
        assert!(
            raw.contains_key("Stop"),
            "Stop hook should have been installed"
        );
    }

    #[test]
    fn uninstall_hooks_preserves_unknown_hook_types() {
        let dir = tempfile::tempdir().unwrap();
        write_claude_settings(
            &dir,
            r#"{
  "hooks": {
    "Stop": [
      {"matcher": "", "hooks": [{"type": "command", "command": "bitloops hooks claude-code stop"}]}
    ],
    "Notification": [
      {"matcher": "", "hooks": [{"type": "command", "command": "echo notification received"}]}
    ],
    "SubagentStop": [
      {"matcher": ".*", "hooks": [{"type": "command", "command": "echo subagent stopped"}]}
    ]
  }
}"#,
        );
        uninstall_hooks(dir.path()).unwrap();

        let raw = read_raw_hooks(&dir);
        assert!(
            raw.contains_key("Notification"),
            "Notification should be preserved"
        );
        assert!(
            raw.contains_key("SubagentStop"),
            "SubagentStop should be preserved"
        );

        // Stop should be gone (our hook was the only one)
        if let Some(stop_val) = raw.get("Stop") {
            let stop_matchers: Vec<ClaudeHookMatcher> =
                serde_json::from_value(stop_val.clone()).unwrap_or_default();
            assert!(
                stop_matchers.is_empty(),
                "Stop hook should have been removed, got: {stop_matchers:?}"
            );
        }
    }

    #[test]
    fn install_hooks_force_replaces_stale_bitloops_hooks() {
        let dir = tempfile::tempdir().unwrap();
        write_claude_settings(
            &dir,
            r#"{
  "hooks": {
    "Stop": [
      {"matcher": "", "hooks": [{"type": "command", "command": "bitloops hooks claude-code stop --stale"}]},
      {"matcher": "", "hooks": [{"type": "command", "command": "echo user stop hook"}]}
    ],
    "PostToolUse": [
      {"matcher": "Task", "hooks": [{"type": "command", "command": "bitloops hooks claude-code post-task --stale"}]}
    ]
  }
}"#,
        );

        install_hooks(dir.path(), true).unwrap();

        let stop = read_hook_type(&dir, "Stop");
        assert_hook_exists(&stop, "", CMD_STOP, "fresh Bitloops Stop hook");
        assert_hook_exists(&stop, "", "echo user stop hook", "user Stop hook");
        assert!(
            !hook_command_exists(&stop, "bitloops hooks claude-code stop --stale"),
            "stale Bitloops stop hook should be removed"
        );

        let post_tool_use = read_hook_type(&dir, "PostToolUse");
        assert_hook_exists(
            &post_tool_use,
            "Task",
            CMD_POST_TASK,
            "fresh Bitloops Task hook",
        );
        assert!(
            !hook_command_exists_with_matcher(
                &post_tool_use,
                "Task",
                "bitloops hooks claude-code post-task --stale"
            ),
            "stale Bitloops post-task hook should be removed"
        );
    }

    #[test]
    fn are_hooks_installed_false_without_stop_hook() {
        let dir = tempfile::tempdir().unwrap();
        write_claude_settings(
            &dir,
            r#"{
  "hooks": {
    "SessionStart": [
      {"matcher": "", "hooks": [{"type": "command", "command": "bitloops hooks claude-code session-start"}]}
    ]
  }
}"#,
        );
        assert!(
            !are_hooks_installed(dir.path()),
            "are_hooks_installed should be false when Stop hook is missing"
        );
    }
}
