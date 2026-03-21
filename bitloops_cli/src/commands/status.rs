use crate::engine::agent::agent_display_name;
use crate::utils::strings;
use anyhow::Result;
use clap::Args;
use serde_json::Value;
use std::fs;
use std::io::Write;
use std::path::Path;
use std::process::Command;
use std::time::SystemTime;

#[derive(Args, Debug, Clone)]
pub struct StatusArgs {
    #[arg(long, default_value_t = false)]
    pub detailed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActiveSession {
    pub session_id: String,
    pub worktree_path: String,
    pub started_at: SystemTime,
    pub last_interaction_time: Option<SystemTime>,
    pub first_prompt: Option<String>,
    pub agent_type: Option<String>,
    pub ended_at: Option<SystemTime>,
    pub branch: Option<String>,
}

#[derive(Debug, Clone)]
struct CliSettings {
    strategy: String,
    enabled: bool,
}

impl Default for CliSettings {
    fn default() -> Self {
        Self {
            strategy: "manual-commit".to_string(),
            enabled: true,
        }
    }
}

#[derive(Debug, Default, Clone)]
struct PartialSettings {
    strategy: Option<String>,
    enabled: Option<bool>,
}

pub fn run_status(w: &mut dyn Write, detailed: bool) -> Result<()> {
    if !is_git_repository() {
        writeln!(w, "✕ not a git repository")?;
        return Ok(());
    }

    let project_path = Path::new(".bitloops").join("settings.json");
    let local_path = Path::new(".bitloops").join("settings.local.json");
    let project_exists = project_path.exists();
    let local_exists = local_path.exists();

    if !project_exists && !local_exists {
        writeln!(w, "○ not set up (run `bitloops enable` to get started)")?;
        return Ok(());
    }

    let project_partial = if project_exists {
        Some(load_partial_settings(&project_path)?)
    } else {
        None
    };
    let local_partial = if local_exists {
        Some(load_partial_settings(&local_path)?)
    } else {
        None
    };

    let effective = merged_settings(project_partial.as_ref(), local_partial.as_ref());

    if detailed {
        writeln!(w, "{}", format_settings_status_short(&effective))?;
        writeln!(w)?;

        if let Some(partial) = &project_partial {
            let settings = settings_from_partial(partial);
            writeln!(w, "{}", format_settings_status("Project", &settings))?;
        }

        if let Some(partial) = &local_partial {
            let settings = settings_from_partial(partial);
            writeln!(w, "{}", format_settings_status("Local", &settings))?;
        }
    } else {
        writeln!(w, "{}", format_settings_status_short(&effective))?;
    }

    if effective.enabled {
        write_active_sessions(w, &[])?;
    }

    Ok(())
}

pub fn time_ago(at: SystemTime) -> String {
    let Ok(elapsed) = SystemTime::now().duration_since(at) else {
        return "just now".to_string();
    };
    let secs = elapsed.as_secs();
    if secs < 60 {
        "just now".to_string()
    } else if secs < 60 * 60 {
        format!("{}m ago", secs / 60)
    } else if secs < 24 * 60 * 60 {
        format!("{}h ago", secs / (60 * 60))
    } else {
        format!("{}d ago", secs / (24 * 60 * 60))
    }
}

pub fn write_active_sessions(w: &mut dyn Write, sessions: &[ActiveSession]) -> Result<()> {
    let mut active: Vec<&ActiveSession> =
        sessions.iter().filter(|s| s.ended_at.is_none()).collect();
    if active.is_empty() {
        return Ok(());
    }

    active.sort_by(|a, b| match a.worktree_path.cmp(&b.worktree_path) {
        std::cmp::Ordering::Equal => b.started_at.cmp(&a.started_at),
        other => other,
    });

    writeln!(w)?;
    writeln!(w, "Active Sessions:")?;

    let mut idx = 0usize;
    while idx < active.len() {
        let path = if active[idx].worktree_path.is_empty() {
            "(unknown)"
        } else {
            active[idx].worktree_path.as_str()
        };
        writeln!(w, "  {path}")?;

        while idx < active.len() {
            let current_path = if active[idx].worktree_path.is_empty() {
                "(unknown)"
            } else {
                active[idx].worktree_path.as_str()
            };
            if current_path != path {
                break;
            }

            let session = active[idx];
            let agent = session
                .agent_type
                .as_deref()
                .map(agent_display_name)
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "(unknown)".to_string());
            let short_id = truncate_chars(&session.session_id, 7);
            let mut line = format!(
                "    [{}] {:<9} started {}",
                agent,
                short_id,
                time_ago(session.started_at)
            );

            if let Some(last) = session.last_interaction_time
                && last.duration_since(session.started_at).unwrap_or_default()
                    > std::time::Duration::from_secs(60)
            {
                line.push_str(&format!(", active {}", time_ago(last)));
            }

            writeln!(w, "{line}")?;

            if let Some(prompt) = &session.first_prompt
                && !prompt.is_empty()
            {
                writeln!(w, "      \"{}\"", truncate_prompt(prompt, 60))?;
            }

            idx += 1;
        }

        if idx < active.len() {
            writeln!(w)?;
        }
    }

    Ok(())
}

fn is_git_repository() -> bool {
    matches!(
        Command::new("git").args(["rev-parse", "--git-dir"]).output(),
        Ok(out) if out.status.success()
    )
}

fn load_partial_settings(path: &Path) -> Result<PartialSettings> {
    let raw = fs::read_to_string(path)?;
    let value: Value = serde_json::from_str(&raw)?;
    let strategy = value
        .get("strategy")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(ToOwned::to_owned);
    let enabled = value.get("enabled").and_then(Value::as_bool);
    Ok(PartialSettings { strategy, enabled })
}

fn settings_from_partial(partial: &PartialSettings) -> CliSettings {
    let mut settings = CliSettings::default();
    if let Some(strategy) = &partial.strategy {
        settings.strategy = strategy.clone();
    }
    if let Some(enabled) = partial.enabled {
        settings.enabled = enabled;
    }
    settings
}

fn merged_settings(
    project: Option<&PartialSettings>,
    local: Option<&PartialSettings>,
) -> CliSettings {
    let mut settings = CliSettings::default();
    if let Some(project) = project {
        if let Some(strategy) = &project.strategy {
            settings.strategy = strategy.clone();
        }
        if let Some(enabled) = project.enabled {
            settings.enabled = enabled;
        }
    }
    if let Some(local) = local {
        if let Some(strategy) = &local.strategy {
            settings.strategy = strategy.clone();
        }
        if let Some(enabled) = local.enabled {
            settings.enabled = enabled;
        }
    }
    settings
}

fn format_settings_status_short(settings: &CliSettings) -> String {
    if settings.enabled {
        format!("Enabled ({})", settings.strategy)
    } else {
        format!("Disabled ({})", settings.strategy)
    }
}

fn format_settings_status(prefix: &str, settings: &CliSettings) -> String {
    if settings.enabled {
        format!("{prefix}, enabled ({})", settings.strategy)
    } else {
        format!("{prefix}, disabled ({})", settings.strategy)
    }
}

fn truncate_chars(input: &str, max_chars: usize) -> String {
    input.chars().take(max_chars).collect()
}

fn truncate_prompt(input: &str, max_chars: usize) -> String {
    strings::truncate_runes(input, max_chars, "...")
}

pub async fn run(args: StatusArgs) -> Result<()> {
    let mut out = std::io::stdout();
    run_status(&mut out, args.detailed)
}

#[cfg(test)]
#[allow(non_snake_case)]
mod tests {
    use super::{ActiveSession, run_status, time_ago, write_active_sessions};
    use crate::test_support::process_state::with_cwd;
    use std::fs;
    use std::io::Cursor;
    use std::path::Path;
    use std::process::Command;
    use std::time::{Duration, SystemTime};
    use tempfile::TempDir;

    fn run_git(dir: &Path, args: &[&str]) -> (bool, String, String) {
        let out = Command::new("git")
            .current_dir(dir)
            .args(args)
            .output()
            .expect("git command should run");
        (
            out.status.success(),
            String::from_utf8_lossy(&out.stdout).to_string(),
            String::from_utf8_lossy(&out.stderr).to_string(),
        )
    }

    fn setup_status_test_repo() -> TempDir {
        let dir = TempDir::new().expect("temp dir");
        let root = dir.path();
        let (ok, _, err) = run_git(root, &["init", "-b", "master"]);
        assert!(ok, "git init failed: {err}");
        fs::write(root.join("test.txt"), "test content").expect("write file");
        let (ok, _, err) = run_git(root, &["add", "test.txt"]);
        assert!(ok, "git add failed: {err}");
        let (ok, _, err) = run_git(
            root,
            &[
                "-c",
                "user.name=Test User",
                "-c",
                "user.email=test@example.com",
                "-c",
                "commit.gpgsign=false",
                "commit",
                "-m",
                "initial commit",
            ],
        );
        assert!(ok, "initial commit failed: {err}");
        dir
    }

    fn write_project_settings(repo_root: &Path, json: &str) {
        let settings_dir = repo_root.join(".bitloops");
        fs::create_dir_all(&settings_dir).expect("settings dir");
        fs::write(settings_dir.join("settings.json"), json).expect("project settings");
    }

    fn write_local_settings(repo_root: &Path, json: &str) {
        let settings_dir = repo_root.join(".bitloops");
        fs::create_dir_all(&settings_dir).expect("settings dir");
        fs::write(settings_dir.join("settings.local.json"), json).expect("local settings");
    }

    // CLI-576
    #[test]
    fn TestRunStatus_Enabled() {
        let repo = setup_status_test_repo();
        write_project_settings(
            repo.path(),
            r#"{"strategy":"manual-commit","enabled":true}"#,
        );

        let mut stdout = Cursor::new(Vec::new());
        let err = with_cwd(repo.path(), || run_status(&mut stdout, false));
        assert!(err.is_ok(), "run_status returned error: {err:?}");

        let output = String::from_utf8(stdout.into_inner()).expect("utf8");
        assert!(
            output.contains("Enabled"),
            "expected output to show Enabled, got: {output}"
        );
    }

    // CLI-577
    #[test]
    fn TestRunStatus_Disabled() {
        let repo = setup_status_test_repo();
        write_project_settings(
            repo.path(),
            r#"{"strategy":"manual-commit","enabled":false}"#,
        );

        let mut stdout = Cursor::new(Vec::new());
        let err = with_cwd(repo.path(), || run_status(&mut stdout, false));
        assert!(err.is_ok(), "run_status returned error: {err:?}");

        let output = String::from_utf8(stdout.into_inner()).expect("utf8");
        assert!(
            output.contains("Disabled"),
            "expected output to show Disabled, got: {output}"
        );
    }

    // CLI-578
    #[test]
    fn TestRunStatus_NotSetUp() {
        let repo = setup_status_test_repo();

        let mut stdout = Cursor::new(Vec::new());
        let err = with_cwd(repo.path(), || run_status(&mut stdout, false));
        assert!(err.is_ok(), "run_status returned error: {err:?}");

        let output = String::from_utf8(stdout.into_inner()).expect("utf8");
        assert!(
            output.contains("not set up"),
            "expected not set up message, got: {output}"
        );
        assert!(
            output.contains("bitloops enable"),
            "expected enable hint, got: {output}"
        );
    }

    // CLI-579
    #[test]
    fn TestRunStatus_NotGitRepository() {
        let dir = TempDir::new().expect("temp dir");
        let mut stdout = Cursor::new(Vec::new());
        let err = with_cwd(dir.path(), || run_status(&mut stdout, false));
        assert!(
            err.is_ok(),
            "run_status should not hard-fail outside git repo: {err:?}"
        );

        let output = String::from_utf8(stdout.into_inner()).expect("utf8");
        assert!(
            output.contains("not a git repository"),
            "expected non-git message, got: {output}"
        );
    }

    // CLI-580
    #[test]
    fn TestRunStatus_LocalSettingsOnly() {
        let repo = setup_status_test_repo();
        write_local_settings(repo.path(), r#"{"strategy":"auto-commit","enabled":true}"#);

        let mut stdout = Cursor::new(Vec::new());
        let err = with_cwd(repo.path(), || run_status(&mut stdout, true));
        assert!(err.is_ok(), "run_status detailed returned error: {err:?}");

        let output = String::from_utf8(stdout.into_inner()).expect("utf8");
        assert!(
            output.contains("Enabled (auto-commit)"),
            "expected effective Enabled (auto-commit), got: {output}"
        );
        assert!(
            output.contains("Local, enabled"),
            "expected local detail line, got: {output}"
        );
        assert!(
            !output.contains("Project,"),
            "should not show project settings when only local exists, got: {output}"
        );
    }

    // CLI-581
    #[test]
    fn TestRunStatus_BothProjectAndLocal() {
        let repo = setup_status_test_repo();
        write_project_settings(
            repo.path(),
            r#"{"strategy":"manual-commit","enabled":true}"#,
        );
        write_local_settings(repo.path(), r#"{"strategy":"auto-commit","enabled":false}"#);

        let mut stdout = Cursor::new(Vec::new());
        let err = with_cwd(repo.path(), || run_status(&mut stdout, true));
        assert!(err.is_ok(), "run_status detailed returned error: {err:?}");

        let output = String::from_utf8(stdout.into_inner()).expect("utf8");
        assert!(
            output.contains("Disabled (auto-commit)"),
            "expected effective local override status, got: {output}"
        );
        assert!(
            output.contains("Project, enabled (manual-commit)"),
            "expected project detail line, got: {output}"
        );
        assert!(
            output.contains("Local, disabled (auto-commit)"),
            "expected local detail line, got: {output}"
        );
    }

    // CLI-582
    #[test]
    fn TestRunStatus_BothProjectAndLocal_Short() {
        let repo = setup_status_test_repo();
        write_project_settings(
            repo.path(),
            r#"{"strategy":"manual-commit","enabled":true}"#,
        );
        write_local_settings(repo.path(), r#"{"strategy":"auto-commit","enabled":false}"#);

        let mut stdout = Cursor::new(Vec::new());
        let err = with_cwd(repo.path(), || run_status(&mut stdout, false));
        assert!(err.is_ok(), "run_status returned error: {err:?}");

        let output = String::from_utf8(stdout.into_inner()).expect("utf8");
        assert!(
            output.contains("Disabled (auto-commit)"),
            "expected merged effective status, got: {output}"
        );
    }

    // CLI-583
    #[test]
    fn TestRunStatus_ShowsStrategy() {
        let repo = setup_status_test_repo();
        write_project_settings(repo.path(), r#"{"strategy":"auto-commit","enabled":true}"#);

        let mut stdout = Cursor::new(Vec::new());
        let err = with_cwd(repo.path(), || run_status(&mut stdout, false));
        assert!(err.is_ok(), "run_status returned error: {err:?}");

        let output = String::from_utf8(stdout.into_inner()).expect("utf8");
        assert!(
            output.contains("(auto-commit)"),
            "expected auto-commit strategy display, got: {output}"
        );
    }

    // CLI-584
    #[test]
    fn TestRunStatus_ShowsManualCommitStrategy() {
        let repo = setup_status_test_repo();
        write_project_settings(
            repo.path(),
            r#"{"strategy":"manual-commit","enabled":false}"#,
        );

        let mut stdout = Cursor::new(Vec::new());
        let err = with_cwd(repo.path(), || run_status(&mut stdout, true));
        assert!(err.is_ok(), "run_status detailed returned error: {err:?}");

        let output = String::from_utf8(stdout.into_inner()).expect("utf8");
        assert!(
            output.contains("Disabled (manual-commit)"),
            "expected effective manual-commit status, got: {output}"
        );
        assert!(
            output.contains("Project, disabled (manual-commit)"),
            "expected project detail manual-commit status, got: {output}"
        );
    }

    // CLI-585
    #[test]
    fn TestTimeAgo() {
        let tests = vec![
            ("just now", Duration::from_secs(10), "just now"),
            ("30 seconds", Duration::from_secs(30), "just now"),
            ("1 minute", Duration::from_secs(60), "1m ago"),
            ("5 minutes", Duration::from_secs(5 * 60), "5m ago"),
            ("59 minutes", Duration::from_secs(59 * 60), "59m ago"),
            ("1 hour", Duration::from_secs(60 * 60), "1h ago"),
            ("3 hours", Duration::from_secs(3 * 60 * 60), "3h ago"),
            ("23 hours", Duration::from_secs(23 * 60 * 60), "23h ago"),
            ("1 day", Duration::from_secs(24 * 60 * 60), "1d ago"),
            ("7 days", Duration::from_secs(7 * 24 * 60 * 60), "7d ago"),
        ];

        for (name, duration, want) in tests {
            let got = time_ago(SystemTime::now() - duration);
            assert_eq!(got, want, "time_ago for {name}");
        }
    }

    // CLI-586
    #[test]
    fn TestWriteActiveSessions() {
        let now = SystemTime::now();
        let sessions = vec![
            ActiveSession {
                session_id: "abc-1234-session".to_string(),
                worktree_path: "/Users/test/repo".to_string(),
                started_at: now - Duration::from_secs(2 * 60 * 60),
                last_interaction_time: Some(now - Duration::from_secs(5 * 60)),
                first_prompt: Some("Fix auth bug in login flow".to_string()),
                agent_type: Some("claude-code".to_string()),
                ended_at: None,
                branch: None,
            },
            ActiveSession {
                session_id: "def-5678-session".to_string(),
                worktree_path: "/Users/test/repo".to_string(),
                started_at: now - Duration::from_secs(15 * 60),
                last_interaction_time: None,
                first_prompt: Some(
                    "Add dark mode support for the bitloops application and all components"
                        .to_string(),
                ),
                agent_type: Some("cursor".to_string()),
                ended_at: None,
                branch: None,
            },
            ActiveSession {
                session_id: "ghi-9012-session".to_string(),
                worktree_path: "/Users/test/repo/.worktrees/3".to_string(),
                started_at: now - Duration::from_secs(5 * 60),
                last_interaction_time: None,
                first_prompt: None,
                agent_type: None,
                ended_at: None,
                branch: None,
            },
        ];

        let mut buf = Cursor::new(Vec::new());
        let err = write_active_sessions(&mut buf, &sessions);
        assert!(err.is_ok(), "write_active_sessions returned error: {err:?}");

        let output = String::from_utf8(buf.into_inner()).expect("utf8");
        assert!(
            output.contains("Active Sessions:"),
            "expected Active Sessions header, got: {output}"
        );
        assert!(
            output.contains("/Users/test/repo"),
            "expected worktree path, got: {output}"
        );
        assert!(
            output.contains("/Users/test/repo/.worktrees/3"),
            "expected secondary worktree path, got: {output}"
        );
        assert!(
            output.contains("[Claude Code]"),
            "expected Claude label, got: {output}"
        );
        assert!(
            output.contains("[Cursor]"),
            "expected Cursor label, got: {output}"
        );
        assert!(
            output.contains("[(unknown)]"),
            "expected unknown agent label, got: {output}"
        );
        assert!(
            output.contains("abc-123"),
            "expected truncated session id abc-123, got: {output}"
        );
        assert!(
            output.contains("\"Fix auth bug in login flow\""),
            "expected first prompt line, got: {output}"
        );
        assert!(
            output.contains("active 5m ago"),
            "expected active time line, got: {output}"
        );

        for line in output.lines() {
            if line.contains("[Cursor]") {
                assert!(
                    !line.contains("active"),
                    "session with no LastInteractionTime should not show active: {line}"
                );
            }
        }
    }

    // CLI-587
    #[test]
    fn TestWriteActiveSessions_ActiveTimeOmittedWhenClose() {
        let now = SystemTime::now();
        let started_at = now - Duration::from_secs(10 * 60);
        let last_interaction = started_at + Duration::from_secs(30);

        let sessions = vec![ActiveSession {
            session_id: "close-time-session".to_string(),
            worktree_path: "/Users/test/repo".to_string(),
            started_at,
            last_interaction_time: Some(last_interaction),
            first_prompt: Some("test prompt".to_string()),
            agent_type: Some("claude-code".to_string()),
            ended_at: None,
            branch: None,
        }];

        let mut buf = Cursor::new(Vec::new());
        let err = write_active_sessions(&mut buf, &sessions);
        assert!(err.is_ok(), "write_active_sessions returned error: {err:?}");

        let output = String::from_utf8(buf.into_inner()).expect("utf8");
        assert!(
            !output.contains("active"),
            "expected active time omitted when close to start time, got: {output}"
        );
    }

    // CLI-588
    #[test]
    fn TestWriteActiveSessions_NoSessions() {
        let mut buf = Cursor::new(Vec::new());
        let err = write_active_sessions(&mut buf, &[]);
        assert!(err.is_ok(), "write_active_sessions returned error: {err:?}");
        let output = String::from_utf8(buf.into_inner()).expect("utf8");
        assert!(output.is_empty(), "expected empty output, got: {output}");
    }

    // CLI-589
    #[test]
    fn TestWriteActiveSessions_EndedSessionsExcluded() {
        let now = SystemTime::now();
        let sessions = vec![ActiveSession {
            session_id: "ended-session".to_string(),
            worktree_path: "/Users/test/repo".to_string(),
            started_at: now - Duration::from_secs(10 * 60),
            last_interaction_time: None,
            first_prompt: None,
            agent_type: None,
            ended_at: Some(now),
            branch: None,
        }];

        let mut buf = Cursor::new(Vec::new());
        let err = write_active_sessions(&mut buf, &sessions);
        assert!(err.is_ok(), "write_active_sessions returned error: {err:?}");
        let output = String::from_utf8(buf.into_inner()).expect("utf8");
        assert!(
            output.is_empty(),
            "expected no output when all sessions are ended, got: {output}"
        );
    }
}
