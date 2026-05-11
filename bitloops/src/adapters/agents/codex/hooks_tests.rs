use std::fs;
use std::path::{Path, PathBuf};

use crate::test_support::process_state::{with_env_var, with_env_vars};
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

fn config_file(path: &Path) -> PathBuf {
    path.join(".codex").join("config.toml")
}

fn skill_file(path: &Path) -> PathBuf {
    path.join(crate::adapters::agents::codex::skills::CODEX_SKILL_RELATIVE_PATH)
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

fn matchers_for_hook(doc: &Value, hook_key: &str) -> Vec<String> {
    doc.get("hooks")
        .and_then(|hooks| hooks.get(hook_key))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|matcher| {
            matcher
                .get("matcher")
                .and_then(Value::as_str)
                .map(|matcher| matcher.to_string())
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

fn with_managed_hook_env_cleared<T>(f: impl FnOnce() -> T) -> T {
    with_env_var(crate::config::ENV_DAEMON_CONFIG_PATH_OVERRIDE, None, f)
}

fn with_codex_test_env<T>(home: &Path, f: impl FnOnce() -> T) -> T {
    let home = home.to_string_lossy().to_string();
    with_env_vars(
        &[
            (crate::config::ENV_DAEMON_CONFIG_PATH_OVERRIDE, None),
            ("HOME", Some(home.as_str())),
            ("USERPROFILE", Some(home.as_str())),
        ],
        f,
    )
}

#[test]
fn install_hooks_fresh_and_idempotent() {
    let dir = tempfile::tempdir().expect("tempdir");
    init_repo(dir.path());
    with_codex_test_env(dir.path(), || {
        let installed = install_hooks_at(dir.path(), false, false).expect("install");
        assert_eq!(installed, 5);
        assert!(are_hooks_installed_at(dir.path()));
        assert!(config_file(dir.path()).exists());

        let second = install_hooks_at(dir.path(), false, false).expect("install second");
        assert_eq!(second, 0);

        let output = read_hooks_json(dir.path());
        let session_commands = commands_for_hook(&output, "SessionStart");
        let user_prompt_commands = commands_for_hook(&output, "UserPromptSubmit");
        let pre_tool_commands = commands_for_hook(&output, "PreToolUse");
        let post_tool_commands = commands_for_hook(&output, "PostToolUse");
        let stop_commands = commands_for_hook(&output, "Stop");

        assert_eq!(
            session_commands,
            vec!["bitloops hooks codex session-start".to_string()]
        );
        assert_eq!(
            user_prompt_commands,
            vec!["bitloops hooks codex user-prompt-submit".to_string()]
        );
        assert_eq!(
            pre_tool_commands,
            vec!["bitloops hooks codex pre-tool-use".to_string()]
        );
        assert_eq!(
            post_tool_commands,
            vec!["bitloops hooks codex post-tool-use".to_string()]
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
        let user_prompt_hook = output
            .get("hooks")
            .and_then(|hooks| hooks.get("UserPromptSubmit"))
            .and_then(Value::as_array)
            .and_then(|entries| entries.first())
            .and_then(|entry| entry.get("hooks"))
            .and_then(Value::as_array)
            .and_then(|entries| entries.first())
            .expect("UserPromptSubmit hook command");

        assert_eq!(
            start_hook.get("type").and_then(Value::as_str),
            Some("command")
        );
        assert_eq!(
            start_hook.get("statusMessage").and_then(Value::as_str),
            Some("Initializing session...")
        );
        assert_eq!(start_hook.get("timeout").and_then(Value::as_i64), Some(10));
        assert_eq!(
            user_prompt_hook
                .get("statusMessage")
                .and_then(Value::as_str),
            Some("Submitting prompt...")
        );
        assert_eq!(
            user_prompt_hook.get("timeout").and_then(Value::as_i64),
            Some(30)
        );
    });
}

#[test]
fn install_hooks_writes_the_minimal_repo_skill() {
    let repo = tempfile::tempdir().expect("repo tempdir");
    let home = tempfile::tempdir().expect("home tempdir");
    init_repo(repo.path());
    with_codex_test_env(home.path(), || {
        install_hooks_at(repo.path(), false, false).expect("install");

        let skill = fs::read_to_string(skill_file(repo.path())).expect("read repo skill");
        assert_eq!(
            skill,
            crate::host::hooks::augmentation::skill_content::DEVQL_EXPLORE_FIRST_SKILL
        );
        assert!(
            !skill_file(home.path()).exists(),
            "must not write Codex skill into HOME"
        );
    });
}

#[test]
fn installed_hooks_require_repo_local_codex_feature_config() {
    let dir = tempfile::tempdir().expect("tempdir");
    init_repo(dir.path());
    with_codex_test_env(dir.path(), || {
        install_hooks_at(dir.path(), false, false).expect("install");
        fs::remove_file(config_file(dir.path())).expect("remove config");

        assert!(
            !are_hooks_installed_at(dir.path()),
            "hooks should not be considered installed without .codex/config.toml enabling codex_hooks"
        );
    });
}

#[test]
fn installed_hooks_require_enabled_repo_local_codex_feature_config() {
    let dir = tempfile::tempdir().expect("tempdir");
    init_repo(dir.path());
    with_codex_test_env(dir.path(), || {
        install_hooks_at(dir.path(), false, false).expect("install");
        fs::write(config_file(dir.path()), "[features]\ncodex_hooks = false\n")
            .expect("disable codex hooks");

        assert!(
            !are_hooks_installed_at(dir.path()),
            "hooks should not be considered installed when codex_hooks is disabled"
        );
    });
}

#[test]
fn install_hooks_local_dev_writes_cargo_run_commands() {
    let dir = tempfile::tempdir().expect("tempdir");
    init_repo(dir.path());
    with_codex_test_env(dir.path(), || {
        let installed = install_hooks_at(dir.path(), true, false).expect("install local-dev");
        assert_eq!(installed, 5);

        let output = read_hooks(dir.path());
        assert!(output.contains("cargo run -- hooks codex session-start"));
        assert!(output.contains("cargo run -- hooks codex user-prompt-submit"));
        assert!(output.contains("cargo run -- hooks codex pre-tool-use"));
        assert!(output.contains("cargo run -- hooks codex post-tool-use"));
        assert!(output.contains("cargo run -- hooks codex stop"));
        assert!(!output.contains("bitloops hooks codex session-start"));
    });
}

#[test]
fn install_hooks_force_reinstalls_managed_hooks() {
    let dir = tempfile::tempdir().expect("tempdir");
    init_repo(dir.path());
    with_codex_test_env(dir.path(), || {
        install_hooks_at(dir.path(), false, false).expect("initial install");

        let installed = install_hooks_at(dir.path(), false, true).expect("force install");
        assert_eq!(installed, 5);

        let output = read_hooks(dir.path());
        let count = output.matches("bitloops hooks codex stop").count();
        assert_eq!(count, 1, "force should keep one managed stop hook");
    });
}

#[test]
fn install_hooks_preserves_unknown_fields() {
    let dir = tempfile::tempdir().expect("tempdir");
    init_repo(dir.path());
    with_codex_test_env(dir.path(), || {
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
        assert!(config_file(dir.path()).exists());
    });
}

#[test]
fn uninstall_preserves_non_bitloops_hooks() {
    let dir = tempfile::tempdir().expect("tempdir");
    init_repo(dir.path());
    with_codex_test_env(dir.path(), || {
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
    ],
    "UserPromptSubmit": [
      {
        "hooks": [
          {
            "type": "command",
            "command": "bitloops hooks codex user-prompt-submit"
          }
        ]
      }
    ],
    "PreToolUse": [
      {
        "matcher": "Bash",
        "hooks": [
          {
            "type": "command",
            "command": "bitloops hooks codex pre-tool-use"
          }
        ]
      }
    ],
    "PostToolUse": [
      {
        "matcher": "Bash",
        "hooks": [
          {
            "type": "command",
            "command": "bitloops hooks codex post-tool-use"
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
        assert!(!output.contains("bitloops hooks codex user-prompt-submit"));
        assert!(!output.contains("bitloops hooks codex pre-tool-use"));
        assert!(!output.contains("bitloops hooks codex post-tool-use"));
        assert!(!output.contains("bitloops hooks codex stop"));
    });
}

#[test]
fn uninstall_removes_the_repo_skill() {
    let repo = tempfile::tempdir().expect("repo tempdir");
    let home = tempfile::tempdir().expect("home tempdir");
    init_repo(repo.path());
    with_codex_test_env(home.path(), || {
        install_hooks_at(repo.path(), false, false).expect("install");
        assert!(
            skill_file(repo.path()).exists(),
            "repo-local skill should exist after install"
        );
        assert!(
            !skill_file(home.path()).exists(),
            "HOME should not receive a Codex skill"
        );

        uninstall_hooks_at(repo.path()).expect("uninstall");

        assert!(
            !skill_file(repo.path()).exists(),
            "repo-local skill should be removed by uninstall"
        );
        assert!(
            !skill_file(home.path()).exists(),
            "HOME should remain clean after uninstall"
        );
    });
}

#[test]
fn install_hooks_migrates_local_dev_commands_without_force() {
    let dir = tempfile::tempdir().expect("tempdir");
    init_repo(dir.path());
    with_codex_test_env(dir.path(), || {
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
    ],
    "UserPromptSubmit": [
      {
        "hooks": [
          {
            "type": "command",
            "command": "cargo run -- hooks codex user-prompt-submit"
          }
        ]
      }
    ],
    "PreToolUse": [
      {
        "matcher": "Bash",
        "hooks": [
          {
            "type": "command",
            "command": "cargo run -- hooks codex pre-tool-use"
          }
        ]
      }
    ],
    "PostToolUse": [
      {
        "matcher": "Bash",
        "hooks": [
          {
            "type": "command",
            "command": "cargo run -- hooks codex post-tool-use"
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
        assert_eq!(installed, 5);

        let output = read_hooks(dir.path());
        assert!(!output.contains("cargo run -- hooks codex session-start"));
        assert!(!output.contains("cargo run -- hooks codex user-prompt-submit"));
        assert!(!output.contains("cargo run -- hooks codex pre-tool-use"));
        assert!(!output.contains("cargo run -- hooks codex post-tool-use"));
        assert!(!output.contains("cargo run -- hooks codex stop"));
        assert!(output.contains("bitloops hooks codex session-start"));
        assert!(output.contains("bitloops hooks codex user-prompt-submit"));
        assert!(output.contains("bitloops hooks codex pre-tool-use"));
        assert!(output.contains("bitloops hooks codex post-tool-use"));
        assert!(output.contains("bitloops hooks codex stop"));
    });
}

#[test]
fn uninstall_hooks_without_config_is_noop() {
    let dir = tempfile::tempdir().expect("tempdir");
    init_repo(dir.path());
    with_codex_test_env(dir.path(), || {
        uninstall_hooks_at(dir.path()).expect("uninstall noop");
    });
}

#[test]
fn install_hooks_sets_bash_matcher_for_tool_hooks() {
    let dir = tempfile::tempdir().expect("tempdir");
    init_repo(dir.path());
    with_codex_test_env(dir.path(), || {
        install_hooks_at(dir.path(), false, false).expect("install");

        let output = read_hooks_json(dir.path());
        assert_eq!(
            matchers_for_hook(&output, "PreToolUse"),
            vec!["Bash".to_string()]
        );
        assert_eq!(
            matchers_for_hook(&output, "PostToolUse"),
            vec!["Bash".to_string()]
        );
    });
}

#[test]
fn are_hooks_installed_requires_all_five_managed_hooks() {
    with_managed_hook_env_cleared(|| {
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
    ],
    "UserPromptSubmit": [
      {
        "hooks": [
          {
            "type": "command",
            "command": "bitloops hooks codex user-prompt-submit"
          }
        ]
      }
    ],
    "PreToolUse": [
      {
        "matcher": "Bash",
        "hooks": [
          {
            "type": "command",
            "command": "bitloops hooks codex pre-tool-use"
          }
        ]
      }
    ],
    "PostToolUse": [
      {
        "matcher": "Bash",
        "hooks": [
          {
            "type": "command",
            "command": "echo custom post"
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
    });
}

#[test]
fn legacy_bitloops_hooks_codex_usage_is_allowlisted() {
    with_managed_hook_env_cleared(|| {
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
    });
}
