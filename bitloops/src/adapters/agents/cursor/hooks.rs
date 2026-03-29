use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Result, anyhow};
use serde_json::{Map, Value, json};

const HOOKS_FILE_NAME: &str = "hooks.json";

const BITLOOPS_HOOK_PREFIX: &str = "bitloops hooks cursor ";
const LOCAL_DEV_HOOK_PREFIX: &str = "cargo run -- hooks cursor ";
const MANAGED_HOOK_PREFIXES: [&str; 2] = [BITLOOPS_HOOK_PREFIX, LOCAL_DEV_HOOK_PREFIX];

const HOOK_TYPE_SESSION_START: &str = "sessionStart";
const HOOK_TYPE_SESSION_END: &str = "sessionEnd";
const HOOK_TYPE_BEFORE_SUBMIT_PROMPT: &str = "beforeSubmitPrompt";
const HOOK_TYPE_BEFORE_SHELL_EXECUTION: &str = "beforeShellExecution";
const HOOK_TYPE_AFTER_SHELL_EXECUTION: &str = "afterShellExecution";
const HOOK_TYPE_STOP: &str = "stop";
const HOOK_TYPE_PRE_COMPACT: &str = "preCompact";
const HOOK_TYPE_SUBAGENT_START: &str = "subagentStart";
const HOOK_TYPE_SUBAGENT_STOP: &str = "subagentStop";

fn managed_hook_types() -> [&'static str; 9] {
    [
        HOOK_TYPE_SESSION_START,
        HOOK_TYPE_SESSION_END,
        HOOK_TYPE_BEFORE_SUBMIT_PROMPT,
        HOOK_TYPE_BEFORE_SHELL_EXECUTION,
        HOOK_TYPE_AFTER_SHELL_EXECUTION,
        HOOK_TYPE_STOP,
        HOOK_TYPE_PRE_COMPACT,
        HOOK_TYPE_SUBAGENT_START,
        HOOK_TYPE_SUBAGENT_STOP,
    ]
}

fn hook_commands(local_dev: bool) -> [(&'static str, String); 9] {
    let prefix = if local_dev {
        LOCAL_DEV_HOOK_PREFIX
    } else {
        BITLOOPS_HOOK_PREFIX
    };
    [
        (
            HOOK_TYPE_SESSION_START,
            format!(
                "{prefix}{}",
                crate::adapters::agents::cursor::lifecycle::HOOK_NAME_SESSION_START
            ),
        ),
        (
            HOOK_TYPE_SESSION_END,
            format!(
                "{prefix}{}",
                crate::adapters::agents::cursor::lifecycle::HOOK_NAME_SESSION_END
            ),
        ),
        (
            HOOK_TYPE_BEFORE_SUBMIT_PROMPT,
            format!(
                "{prefix}{}",
                crate::adapters::agents::cursor::lifecycle::HOOK_NAME_BEFORE_SUBMIT_PROMPT
            ),
        ),
        (
            HOOK_TYPE_BEFORE_SHELL_EXECUTION,
            format!(
                "{prefix}{}",
                crate::adapters::agents::cursor::lifecycle::HOOK_NAME_BEFORE_SHELL_EXECUTION
            ),
        ),
        (
            HOOK_TYPE_AFTER_SHELL_EXECUTION,
            format!(
                "{prefix}{}",
                crate::adapters::agents::cursor::lifecycle::HOOK_NAME_AFTER_SHELL_EXECUTION
            ),
        ),
        (
            HOOK_TYPE_STOP,
            format!(
                "{prefix}{}",
                crate::adapters::agents::cursor::lifecycle::HOOK_NAME_STOP
            ),
        ),
        (
            HOOK_TYPE_PRE_COMPACT,
            format!(
                "{prefix}{}",
                crate::adapters::agents::cursor::lifecycle::HOOK_NAME_PRE_COMPACT
            ),
        ),
        (
            HOOK_TYPE_SUBAGENT_START,
            format!(
                "{prefix}{}",
                crate::adapters::agents::cursor::lifecycle::HOOK_NAME_SUBAGENT_START
            ),
        ),
        (
            HOOK_TYPE_SUBAGENT_STOP,
            format!(
                "{prefix}{}",
                crate::adapters::agents::cursor::lifecycle::HOOK_NAME_SUBAGENT_STOP
            ),
        ),
    ]
}

fn hooks_file_path_at(repo_root: &Path) -> PathBuf {
    repo_root.join(".cursor").join(HOOKS_FILE_NAME)
}

fn hooks_file_path() -> Result<PathBuf> {
    let repo_root = crate::utils::paths::repo_root().or_else(|_| {
        std::env::current_dir().map_err(|err| anyhow!("failed to get current directory: {err}"))
    })?;
    Ok(hooks_file_path_at(&repo_root))
}

fn parse_top_level_map(data: &[u8]) -> Result<Map<String, Value>> {
    let value: Value =
        serde_json::from_slice(data).map_err(|err| anyhow!("failed to parse hooks.json: {err}"))?;
    let Some(map) = value.as_object() else {
        return Err(anyhow!("failed to parse hooks.json: expected JSON object"));
    };
    Ok(map.clone())
}

fn command_of(entry: &Value) -> Option<&str> {
    entry
        .as_object()
        .and_then(|obj| obj.get("command"))
        .and_then(Value::as_str)
}

fn is_bitloops_hook(command: &str) -> bool {
    MANAGED_HOOK_PREFIXES
        .iter()
        .any(|prefix| command.starts_with(prefix))
}

fn is_bitloops_hook_entry(entry: &Value) -> bool {
    command_of(entry).is_some_and(is_bitloops_hook)
}

fn remove_managed_hooks(entries: Vec<Value>) -> Vec<Value> {
    entries
        .into_iter()
        .filter(|entry| !is_bitloops_hook_entry(entry))
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
        let Some(entry_command) = command_of(&entry) else {
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

fn has_command(entries: &[Value], command: &str) -> bool {
    entries
        .iter()
        .any(|entry| command_of(entry) == Some(command))
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

pub fn install_hooks(local_dev: bool, force: bool) -> Result<usize> {
    let path = hooks_file_path()?;
    install_hooks_at_path(&path, local_dev, force)
}

pub fn install_hooks_at(repo_root: &Path, local_dev: bool, force: bool) -> Result<usize> {
    install_hooks_at_path(&hooks_file_path_at(repo_root), local_dev, force)
}

fn install_hooks_at_path(path: &Path, local_dev: bool, force: bool) -> Result<usize> {
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
    for (hook_type, command) in hook_commands(local_dev) {
        let existing = parse_hook_entries(&raw_hooks, hook_type);
        let mut normalized = normalize_hook_entries_for_install(existing.clone(), &command, force);
        if !has_command(&normalized, &command) {
            installed += 1;
            normalized.push(json!({ "command": command }));
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
        fs::create_dir_all(parent)
            .map_err(|err| anyhow!("failed to create .cursor directory: {err}"))?;
    }

    let mut output = serde_json::to_string_pretty(&Value::Object(raw_file))
        .map_err(|err| anyhow!("failed to marshal hooks.json: {err}"))?;
    output.push('\n');
    fs::write(path, output).map_err(|err| anyhow!("failed to write hooks.json: {err}"))?;
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
    let data = match fs::read(path) {
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
        .map_err(|err| anyhow!("failed to marshal hooks.json: {err}"))?;
    output.push('\n');
    fs::write(path, output).map_err(|err| anyhow!("failed to write hooks.json: {err}"))?;
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
    let Ok(parsed) = serde_json::from_slice::<super::types::CursorHooksFile>(&data) else {
        return false;
    };

    [
        parsed.hooks.session_start.as_slice(),
        parsed.hooks.session_end.as_slice(),
        parsed.hooks.before_submit_prompt.as_slice(),
        parsed.hooks.before_shell_execution.as_slice(),
        parsed.hooks.after_shell_execution.as_slice(),
        parsed.hooks.stop.as_slice(),
        parsed.hooks.pre_compact.as_slice(),
        parsed.hooks.subagent_start.as_slice(),
        parsed.hooks.subagent_stop.as_slice(),
    ]
    .into_iter()
    .all(|entries| entries.iter().any(|entry| is_bitloops_hook(&entry.command)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::process_state::with_cwd;
    use std::path::{Path, PathBuf};

    fn init_repo(path: &std::path::Path) {
        let output = std::process::Command::new("git")
            .args(["init", "-q"])
            .current_dir(path)
            .output()
            .expect("git init");
        assert!(output.status.success(), "git init failed");
    }

    fn collect_rs_files(dir: &Path, out: &mut Vec<PathBuf>) {
        let Ok(entries) = fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                collect_rs_files(&path, out);
                continue;
            }
            if path.extension().and_then(|ext| ext.to_str()) == Some("rs") {
                out.push(path);
            }
        }
    }

    #[test]
    fn install_hooks_canonical_fresh_and_idempotent() {
        let dir = tempfile::tempdir().expect("tempdir");
        init_repo(dir.path());
        with_cwd(dir.path(), || {
            let count = install_hooks(false, false).expect("install");
            assert_eq!(count, 9);
            assert!(are_hooks_installed());

            let second = install_hooks(false, false).expect("install second");
            assert_eq!(second, 0);
        });
    }

    #[test]
    fn install_hooks_local_dev_writes_cargo_run_commands() {
        let dir = tempfile::tempdir().expect("tempdir");
        init_repo(dir.path());
        with_cwd(dir.path(), || {
            let installed = install_hooks(true, false).expect("install local-dev");
            assert_eq!(installed, 9);

            let output = fs::read_to_string(dir.path().join(".cursor").join("hooks.json"))
                .expect("read written hooks.json");
            assert!(output.contains("cargo run -- hooks cursor session-start"));
            assert!(output.contains("cargo run -- hooks cursor stop"));
            assert!(!output.contains("bitloops hooks cursor session-start"));
        });
    }

    #[test]
    fn install_hooks_force_reinstalls_managed_hooks() {
        let dir = tempfile::tempdir().expect("tempdir");
        init_repo(dir.path());
        with_cwd(dir.path(), || {
            install_hooks(false, false).expect("initial install");

            let installed = install_hooks(false, true).expect("force install");
            assert_eq!(installed, 9);

            let output = fs::read_to_string(dir.path().join(".cursor").join("hooks.json"))
                .expect("read written hooks.json");
            let parsed: Value = serde_json::from_str(&output).expect("json parse");
            let hooks = parsed
                .get("hooks")
                .and_then(Value::as_object)
                .expect("hooks object");
            let stop = hooks
                .get("stop")
                .and_then(Value::as_array)
                .expect("stop hooks");
            let stop_count = stop
                .iter()
                .filter_map(command_of)
                .filter(|command| *command == "bitloops hooks cursor stop")
                .count();
            assert_eq!(stop_count, 1, "force should keep one managed stop hook");
        });
    }

    #[test]
    fn are_hooks_installed_false_when_shell_hooks_missing() {
        let dir = tempfile::tempdir().expect("tempdir");
        init_repo(dir.path());
        with_cwd(dir.path(), || {
            let cursor_dir = dir.path().join(".cursor");
            fs::create_dir_all(&cursor_dir).expect("create .cursor");
            fs::write(
                cursor_dir.join("hooks.json"),
                r#"{
  "version": 1,
  "hooks": {
    "sessionStart": [{"command": "bitloops hooks cursor session-start"}],
    "sessionEnd": [{"command": "bitloops hooks cursor session-end"}],
    "beforeSubmitPrompt": [{"command": "bitloops hooks cursor before-submit-prompt"}],
    "stop": [{"command": "bitloops hooks cursor stop"}],
    "preCompact": [{"command": "bitloops hooks cursor pre-compact"}],
    "subagentStart": [{"command": "bitloops hooks cursor subagent-start"}],
    "subagentStop": [{"command": "bitloops hooks cursor subagent-stop"}]
  }
}
"#,
            )
            .expect("seed hooks");

            assert!(
                !are_hooks_installed(),
                "legacy 7-hook install should be treated as incomplete"
            );

            let installed = install_hooks(false, false).expect("install");
            assert_eq!(installed, 2, "should add missing shell hooks only");
            assert!(are_hooks_installed());
        });
    }

    #[test]
    fn install_hooks_preserves_unknown_fields() {
        let dir = tempfile::tempdir().expect("tempdir");
        init_repo(dir.path());
        with_cwd(dir.path(), || {
            let cursor_dir = dir.path().join(".cursor");
            fs::create_dir_all(&cursor_dir).expect("create .cursor");
            fs::write(
                cursor_dir.join("hooks.json"),
                r#"{
  "version": 1,
  "cursorSettings": {"foo": true},
  "hooks": {"customHook": [{"command": "echo custom"}]}
}
"#,
            )
            .expect("seed hooks");

            install_hooks(false, false).expect("install");
            let output =
                fs::read_to_string(cursor_dir.join("hooks.json")).expect("read written hooks.json");
            let parsed: Value = serde_json::from_str(&output).expect("json parse");
            assert!(parsed.get("cursorSettings").is_some());
            assert!(
                parsed
                    .get("hooks")
                    .and_then(Value::as_object)
                    .and_then(|h| h.get("customHook"))
                    .is_some()
            );
        });
    }

    #[test]
    fn uninstall_preserves_non_bitloops_hooks() {
        let dir = tempfile::tempdir().expect("tempdir");
        init_repo(dir.path());
        with_cwd(dir.path(), || {
            install_hooks(false, false).expect("install");
            let hooks_path = dir.path().join(".cursor").join("hooks.json");
            let seeded = r#"{
  "version": 1,
  "hooks": {
    "sessionStart": [{"command": "bitloops hooks cursor session-start"}, {"command": "echo custom"}]
  }
}
"#;
            fs::write(&hooks_path, seeded).expect("seed");

            uninstall_hooks().expect("uninstall");
            let output = fs::read_to_string(hooks_path).expect("read");
            assert!(output.contains("echo custom"));
            assert!(!output.contains("bitloops hooks cursor session-start"));
        });
    }

    #[test]
    fn install_hooks_migrates_local_dev_commands_without_force() {
        let dir = tempfile::tempdir().expect("tempdir");
        init_repo(dir.path());
        with_cwd(dir.path(), || {
            let cursor_dir = dir.path().join(".cursor");
            fs::create_dir_all(&cursor_dir).expect("create .cursor");
            fs::write(
                cursor_dir.join("hooks.json"),
                r#"{
  "version": 1,
  "hooks": {
    "sessionStart": [{"command": "cargo run -- hooks cursor session-start"}]
  }
}
"#,
            )
            .expect("seed hooks");

            let installed = install_hooks(false, false).expect("install");
            assert_eq!(installed, 9);

            let output =
                fs::read_to_string(cursor_dir.join("hooks.json")).expect("read written hooks.json");
            assert!(!output.contains("cargo run -- hooks cursor session-start"));
            assert!(output.contains("bitloops hooks cursor session-start"));
        });
    }

    #[test]
    fn install_hooks_removes_non_target_managed_when_canonical_already_exists() {
        let dir = tempfile::tempdir().expect("tempdir");
        init_repo(dir.path());
        with_cwd(dir.path(), || {
            let cursor_dir = dir.path().join(".cursor");
            fs::create_dir_all(&cursor_dir).expect("create .cursor");
            fs::write(
                cursor_dir.join("hooks.json"),
                r#"{
  "version": 1,
  "hooks": {
    "stop": [
      {"command": "cargo run -- hooks cursor stop"},
      {"command": "bitloops hooks cursor stop"}
    ]
  }
}
"#,
            )
            .expect("seed hooks");

            let installed = install_hooks(false, false).expect("install");
            assert_eq!(installed, 8);

            let output =
                fs::read_to_string(cursor_dir.join("hooks.json")).expect("read written hooks.json");
            assert!(!output.contains("cargo run -- hooks cursor stop"));

            let parsed: Value = serde_json::from_str(&output).expect("json parse");
            let hooks = parsed
                .get("hooks")
                .and_then(Value::as_object)
                .expect("hooks object");
            let stop = hooks
                .get("stop")
                .and_then(Value::as_array)
                .expect("stop hooks");
            let stop_count = stop
                .iter()
                .filter_map(command_of)
                .filter(|command| *command == "bitloops hooks cursor stop")
                .count();
            assert_eq!(stop_count, 1);
        });
    }

    #[test]
    fn legacy_bitloops_hooks_cursor_usage_is_allowlisted() {
        let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
        let src_root = manifest_dir.join("src");
        let mut files = Vec::new();
        collect_rs_files(&src_root, &mut files);

        let allowlist = [
            "src/adapters/agents/cursor/hooks.rs",
            "src/cli/enable.rs",
            "src/cli/init.rs",
            "src/cli/init/tests.rs",
            "src/cli/root_test.rs",
        ];
        let mut violations = Vec::new();

        for file in files {
            let Ok(content) = fs::read_to_string(&file) else {
                continue;
            };
            if !content.contains("bitloops hooks cursor") {
                continue;
            }
            let rel = file
                .strip_prefix(manifest_dir)
                .ok()
                .and_then(|p| p.to_str())
                .unwrap_or_default()
                .replace('\\', "/");
            if !allowlist.contains(&rel.as_str()) {
                violations.push(rel);
            }
        }

        assert!(
            violations.is_empty(),
            "unexpected `bitloops hooks cursor` usage outside allowlist: {:?}",
            violations
        );
    }
}
