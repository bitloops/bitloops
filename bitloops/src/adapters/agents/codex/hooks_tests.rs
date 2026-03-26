use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;

use super::*;

fn init_repo(path: &Path) {
    let output = std::process::Command::new("git")
        .args(["init", "-q"])
        .current_dir(path)
        .output()
        .expect("git init");
    assert!(output.status.success(), "git init should succeed");
}

fn hooks_file(path: &Path) -> PathBuf {
    path.join(".codex").join("hooks.json")
}

fn read_hooks(path: &Path) -> String {
    fs::read_to_string(hooks_file(path)).expect("read hooks.json")
}

fn read_hooks_json(path: &Path) -> Value {
    serde_json::from_str(&read_hooks(path)).expect("parse hooks.json")
}

fn commands_for_hook(doc: &Value, hook_key: &str) -> Vec<String> {
    doc.get("hooks")
        .and_then(|hooks| hooks.get(hook_key))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .flat_map(|matcher| {
            matcher
                .get("hooks")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|hook| {
                    hook.get("command")
                        .and_then(Value::as_str)
                        .map(|command| command.to_string())
                })
        })
        .collect()
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
fn install_hooks_fresh_and_idempotent() {
    let dir = tempfile::tempdir().expect("tempdir");
    init_repo(dir.path());

    let installed = install_hooks_at(dir.path(), false, false).expect("install");
    assert_eq!(installed, 2);
    assert!(are_hooks_installed_at(dir.path()));

    let second = install_hooks_at(dir.path(), false, false).expect("install second");
    assert_eq!(second, 0);

    let output = read_hooks_json(dir.path());
    let session_commands = commands_for_hook(&output, "SessionStart");
    let stop_commands = commands_for_hook(&output, "Stop");

    assert_eq!(
        session_commands,
        vec!["bitloops hooks codex session-start".to_string()]
    );
    assert_eq!(stop_commands, vec!["bitloops hooks codex stop".to_string()]);

    let start_hook = output
        .get("hooks")
        .and_then(|hooks| hooks.get("SessionStart"))
        .and_then(Value::as_array)
        .and_then(|entries| entries.first())
        .and_then(|entry| entry.get("hooks"))
        .and_then(Value::as_array)
        .and_then(|entries| entries.first())
        .expect("SessionStart hook command");

    assert_eq!(
        start_hook.get("type").and_then(Value::as_str),
        Some("command")
    );
    assert_eq!(
        start_hook.get("statusMessage").and_then(Value::as_str),
        Some("Initializing session...")
    );
    assert_eq!(start_hook.get("timeout").and_then(Value::as_i64), Some(10));
}

#[test]
fn install_hooks_local_dev_writes_cargo_run_commands() {
    let dir = tempfile::tempdir().expect("tempdir");
    init_repo(dir.path());

    let installed = install_hooks_at(dir.path(), true, false).expect("install local-dev");
    assert_eq!(installed, 2);

    let output = read_hooks(dir.path());
    assert!(output.contains("cargo run -- hooks codex session-start"));
    assert!(output.contains("cargo run -- hooks codex stop"));
    assert!(!output.contains("bitloops hooks codex session-start"));
}

#[test]
fn install_hooks_force_reinstalls_managed_hooks() {
    let dir = tempfile::tempdir().expect("tempdir");
    init_repo(dir.path());

    install_hooks_at(dir.path(), false, false).expect("initial install");

    let installed = install_hooks_at(dir.path(), false, true).expect("force install");
    assert_eq!(installed, 2);

    let output = read_hooks(dir.path());
    let count = output.matches("bitloops hooks codex stop").count();
    assert_eq!(count, 1, "force should keep one managed stop hook");
}

#[test]
fn install_hooks_preserves_unknown_fields() {
    let dir = tempfile::tempdir().expect("tempdir");
    init_repo(dir.path());

    let codex_dir = dir.path().join(".codex");
    fs::create_dir_all(&codex_dir).expect("create .codex");
    fs::write(
        codex_dir.join("hooks.json"),
        r#"
{
  "profile": "strict",
  "hooks": {
    "FutureEvent": [
      {
        "hooks": [
          {
            "type": "command",
            "command": "echo future"
          }
        ]
      }
    ]
  }
}
"#,
    )
    .expect("seed config");

    install_hooks_at(dir.path(), false, false).expect("install");

    let output = read_hooks(dir.path());
    assert!(output.contains("\"profile\": \"strict\""));
    assert!(output.contains("echo future"));
}

#[test]
fn uninstall_preserves_non_bitloops_hooks() {
    let dir = tempfile::tempdir().expect("tempdir");
    init_repo(dir.path());

    let codex_dir = dir.path().join(".codex");
    fs::create_dir_all(&codex_dir).expect("create .codex");
    fs::write(
        codex_dir.join("hooks.json"),
        r#"
{
  "hooks": {
    "SessionStart": [
      {
        "hooks": [
          {
            "type": "command",
            "command": "bitloops hooks codex session-start"
          },
          {
            "type": "command",
            "command": "echo custom"
          }
        ]
      }
    ],
    "Stop": [
      {
        "hooks": [
          {
            "type": "command",
            "command": "bitloops hooks codex stop"
          }
        ]
      }
    ]
  }
}
"#,
    )
    .expect("seed config");

    uninstall_hooks_at(dir.path()).expect("uninstall");
    let output = read_hooks(dir.path());
    assert!(output.contains("echo custom"));
    assert!(!output.contains("bitloops hooks codex session-start"));
    assert!(!output.contains("bitloops hooks codex stop"));
}

#[test]
fn install_hooks_migrates_local_dev_commands_without_force() {
    let dir = tempfile::tempdir().expect("tempdir");
    init_repo(dir.path());

    let codex_dir = dir.path().join(".codex");
    fs::create_dir_all(&codex_dir).expect("create .codex");
    fs::write(
        codex_dir.join("hooks.json"),
        r#"
{
  "hooks": {
    "SessionStart": [
      {
        "hooks": [
          {
            "type": "command",
            "command": "cargo run -- hooks codex session-start"
          }
        ]
      }
    ],
    "Stop": [
      {
        "hooks": [
          {
            "type": "command",
            "command": "cargo run -- hooks codex stop"
          }
        ]
      }
    ]
  }
}
"#,
    )
    .expect("seed config");

    let installed = install_hooks_at(dir.path(), false, false).expect("install");
    assert_eq!(installed, 2);

    let output = read_hooks(dir.path());
    assert!(!output.contains("cargo run -- hooks codex session-start"));
    assert!(!output.contains("cargo run -- hooks codex stop"));
    assert!(output.contains("bitloops hooks codex session-start"));
    assert!(output.contains("bitloops hooks codex stop"));
}

#[test]
fn uninstall_hooks_without_config_is_noop() {
    let dir = tempfile::tempdir().expect("tempdir");
    init_repo(dir.path());

    uninstall_hooks_at(dir.path()).expect("uninstall noop");
}

#[test]
fn are_hooks_installed_requires_session_start_and_stop() {
    let dir = tempfile::tempdir().expect("tempdir");
    init_repo(dir.path());

    let codex_dir = dir.path().join(".codex");
    fs::create_dir_all(&codex_dir).expect("create .codex");
    fs::write(
        codex_dir.join("hooks.json"),
        r#"
{
  "hooks": {
    "SessionStart": [
      {
        "hooks": [
          {
            "type": "command",
            "command": "bitloops hooks codex session-start"
          }
        ]
      }
    ]
  }
}
"#,
    )
    .expect("seed config");

    assert!(
        !are_hooks_installed_at(dir.path()),
        "missing stop hook should be treated as incomplete install"
    );
}

#[test]
fn legacy_bitloops_hooks_codex_usage_is_allowlisted() {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let src_root = manifest_dir.join("src");
    let mut files = Vec::new();
    collect_rs_files(&src_root, &mut files);

    let allowlist = [
        "src/adapters/agents/codex/hooks.rs",
        "src/adapters/agents/codex/hooks_tests.rs",
        "src/cli/init.rs",
        "src/cli/init/tests.rs",
        "src/cli/root_test.rs",
    ];
    let mut violations = Vec::new();

    for file in files {
        let Ok(content) = fs::read_to_string(&file) else {
            continue;
        };
        if !content.contains("bitloops hooks codex") {
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
        "unexpected `bitloops hooks codex` usage outside allowlist: {:?}",
        violations
    );
}
