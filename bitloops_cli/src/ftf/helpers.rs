use super::world::FtfWorld;
use crate::engine::agent::AGENT_NAME_CLAUDE_CODE;
use crate::engine::session::create_session_backend_or_local;
use crate::engine::strategy::manual_commit::{read_commit_checkpoint_mappings, read_committed};
use anyhow::{Context, Result, anyhow, bail, ensure};
use serde::Serialize;
use std::ffi::OsString;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::process::{Command, Output};
use time::format_description::well_known::Rfc3339;
use time::{Duration, OffsetDateTime, PrimitiveDateTime, Time, UtcOffset};
use uuid::Uuid;

pub const BITLOOPS_REPO_NAME: &str = "bitloops";
const DEFAULT_CLAUDE_CODE_COMMAND: &str =
    "claude --model haiku --permission-mode bypassPermissions -p";
const FIRST_CLAUDE_PROMPT: &str =
    "Remove the Vite example code from the project and replace it with a simple hello world page";
const SECOND_CLAUDE_PROMPT: &str = "Change the hello world color to blue";

#[derive(Debug, Serialize)]
struct RunMetadata<'a> {
    scenario_name: &'a str,
    scenario_slug: &'a str,
    flow_name: &'a str,
    run_dir: String,
    repo_dir: String,
    terminal_log: String,
    binary_path: String,
    created_at: String,
}

pub fn sanitize_name(input: &str) -> String {
    let mut slug = String::with_capacity(input.len());
    let mut last_was_dash = false;

    for ch in input.chars() {
        let normalized = match ch {
            'a'..='z' | '0'..='9' => Some(ch),
            'A'..='Z' => Some(ch.to_ascii_lowercase()),
            _ if ch.is_ascii_whitespace() || matches!(ch, '-' | '_' | '/' | ':') => Some('-'),
            _ => None,
        };

        if let Some(value) = normalized {
            if value == '-' {
                if slug.is_empty() || last_was_dash {
                    continue;
                }
                last_was_dash = true;
            } else {
                last_was_dash = false;
            }
            slug.push(value);
        }
    }

    slug.trim_matches('-').to_string()
}

pub fn ensure_bitloops_repo_name(repo_name: &str) -> Result<()> {
    ensure!(
        repo_name == BITLOOPS_REPO_NAME,
        "unsupported repository `{repo_name}`; only `bitloops` is supported by ftf"
    );
    Ok(())
}

pub fn run_clean_start(world: &mut FtfWorld, flow_name: &str) -> Result<()> {
    let config = world.run_config().clone();
    let flow_slug = sanitize_name(flow_name);
    ensure!(
        !flow_slug.is_empty(),
        "flow name must produce a non-empty slug"
    );

    let scenario_slug = world
        .scenario_slug
        .clone()
        .unwrap_or_else(|| "scenario".to_string());
    let run_dir = config
        .suite_root
        .join(format!("{scenario_slug}-{flow_slug}-{}", short_run_id()));
    let repo_dir = run_dir.join(BITLOOPS_REPO_NAME);
    let terminal_log_path = run_dir.join("terminal.log");
    let metadata_path = run_dir.join("run.json");

    fs::create_dir_all(&repo_dir).context("creating ftf repo directory")?;

    world.flow_name = Some(flow_name.to_string());
    world.run_dir = Some(run_dir);
    world.repo_dir = Some(repo_dir);
    world.terminal_log_path = Some(terminal_log_path);
    world.metadata_path = Some(metadata_path);

    let init_output = run_command_capture(
        world,
        "git init",
        build_git_command(world.repo_dir(), &["init", "-q"], &[]),
    )?;
    ensure_success(&init_output, "git init")?;
    configure_git_identity(world)?;
    write_run_metadata(world)?;
    append_world_log(
        world,
        &format!(
            "Initialized clean run directory at {}\n",
            world.run_dir().display()
        ),
    )?;
    Ok(())
}

pub fn run_init_commit_for_repo(world: &mut FtfWorld, repo_name: &str) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    if repo_has_head(world)? {
        append_world_log(world, "InitCommit skipped because HEAD already exists.\n")?;
        return Ok(());
    }

    let readme_path = world.repo_dir().join("README.md");
    fs::write(
        &readme_path,
        format!("# {repo_name}\n\nInitial repo for Bitloops foundation tests.\n"),
    )
    .with_context(|| format!("writing {}", readme_path.display()))?;
    run_git_success(world, &["add", "README.md"], &[], "git add README.md")?;
    run_git_success(
        world,
        &["commit", "-m", "chore: initial commit"],
        &[],
        "git commit initial",
    )?;
    Ok(())
}

pub fn run_init_commit_with_relative_day_for_repo(
    world: &mut FtfWorld,
    repo_name: &str,
    days_ago: i64,
) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    if repo_has_head(world)? {
        append_world_log(
            world,
            "InitCommit with relative day skipped because HEAD already exists.\n",
        )?;
        return Ok(());
    }

    let readme_path = world.repo_dir().join("README.md");
    fs::write(
        &readme_path,
        format!("# {repo_name}\n\nInitial repo for Bitloops foundation tests.\n"),
    )
    .with_context(|| format!("writing {}", readme_path.display()))?;
    let git_date = git_date_for_relative_day(days_ago)?;
    let env = [
        ("GIT_AUTHOR_DATE", OsString::from(git_date.clone())),
        ("GIT_COMMITTER_DATE", OsString::from(git_date)),
    ];
    run_git_success(world, &["add", "README.md"], &env, "git add README.md")?;
    run_git_success(
        world,
        &["commit", "-m", "chore: initial commit"],
        &env,
        "git commit initial",
    )?;
    Ok(())
}

pub fn run_create_vite_app_project_for_repo(world: &mut FtfWorld, repo_name: &str) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    create_offline_vite_react_ts_scaffold(world.repo_dir())
}

pub fn run_init_bitloops_for_repo(world: &mut FtfWorld, repo_name: &str) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    run_bitloops_success(world, &["init", "--agent", "claude-code"], "bitloops init")
}

pub fn run_enable_cli_for_repo(world: &mut FtfWorld, repo_name: &str) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    run_bitloops_success(world, &["enable"], "bitloops enable")
}

pub fn run_first_change_using_claude_code_for_repo(
    world: &mut FtfWorld,
    repo_name: &str,
) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    run_claude_code_prompt(world, FIRST_CLAUDE_PROMPT)
}

pub fn run_second_change_using_claude_code_for_repo(
    world: &mut FtfWorld,
    repo_name: &str,
) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    run_claude_code_prompt(world, SECOND_CLAUDE_PROMPT)
}

pub fn commit_for_relative_day_for_repo(
    world: &mut FtfWorld,
    repo_name: &str,
    days_ago: i64,
    label: &str,
) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    let git_date = git_date_for_relative_day(days_ago)?;
    let env = [
        ("GIT_AUTHOR_DATE", OsString::from(git_date.clone())),
        ("GIT_COMMITTER_DATE", OsString::from(git_date)),
    ];

    run_git_success(world, &["add", "-A"], &env, "git add -A")?;

    let diff_output = run_command_capture(
        world,
        "git diff --cached --quiet",
        build_git_command(world.repo_dir(), &["diff", "--cached", "--quiet"], &env),
    )?;

    let diff_code = diff_output.status.code().unwrap_or_default();
    ensure!(
        diff_code <= 1,
        "git diff --cached --quiet failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&diff_output.stdout),
        String::from_utf8_lossy(&diff_output.stderr)
    );

    let mut args = vec!["commit", "-m", label];
    if diff_code == 0 {
        args.insert(1, "--allow-empty");
    }
    run_git_success(world, &args, &env, "git commit relative day")?;
    Ok(())
}

pub fn assert_bitloops_stores_exist_for_repo(world: &FtfWorld, repo_name: &str) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    let stores_dir = world.repo_dir().join(".bitloops").join("stores");
    ensure!(
        stores_dir.exists(),
        "expected stores directory to exist at {}",
        stores_dir.display()
    );
    let relational = stores_dir.join("relational").join("relational.db");
    let events = stores_dir.join("event").join("events.duckdb");
    ensure!(
        relational.exists(),
        "expected relational store at {}",
        relational.display()
    );
    ensure!(
        events.exists(),
        "expected events store at {}",
        events.display()
    );
    Ok(())
}

pub fn assert_claude_session_exists_for_repo(world: &FtfWorld, repo_name: &str) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    let backend = create_session_backend_or_local(world.repo_dir());
    let sessions = backend
        .list_sessions()
        .context("listing persisted Bitloops sessions")?;

    let Some(session) = sessions
        .iter()
        .find(|session| session.agent_type == AGENT_NAME_CLAUDE_CODE)
    else {
        bail!("expected at least one persisted claude-code session");
    };

    ensure!(
        !session.session_id.is_empty(),
        "expected claude-code session to have a session id"
    );
    ensure!(
        !session.transcript_path.is_empty(),
        "expected claude-code session to record a transcript path"
    );
    Ok(())
}

pub fn assert_checkpoint_mapping_exists_for_repo(world: &FtfWorld, repo_name: &str) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    let mappings = read_commit_checkpoint_mappings(world.repo_dir())
        .context("reading Bitloops checkpoint mappings")?;
    let Some(checkpoint_id) = mappings.values().next() else {
        bail!("expected at least one Bitloops checkpoint mapping");
    };

    let summary = read_committed(world.repo_dir(), checkpoint_id)
        .with_context(|| format!("reading committed checkpoint summary for {checkpoint_id}"))?;
    ensure!(
        summary.is_some(),
        "expected committed checkpoint summary for {checkpoint_id}"
    );
    Ok(())
}

pub fn assert_checkpoint_mapping_count_at_least_for_repo(
    world: &FtfWorld,
    repo_name: &str,
    min_count: usize,
) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    let mappings = read_commit_checkpoint_mappings(world.repo_dir())
        .context("reading Bitloops checkpoint mappings")?;
    ensure!(
        mappings.len() >= min_count,
        "expected at least {min_count} Bitloops checkpoint mappings, got {}",
        mappings.len()
    );
    Ok(())
}

pub fn assert_init_yesterday_and_final_today_commit_checkpoints_for_repo(
    world: &FtfWorld,
    repo_name: &str,
) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    let output = run_command_capture(
        world,
        "git log timeline",
        build_git_command(
            world.repo_dir(),
            &["log", "--pretty=format:%s|%aI", "-n", "30"],
            &[],
        ),
    )?;
    ensure_success(&output, "git log timeline")?;
    let log = String::from_utf8_lossy(&output.stdout);
    let commits = log
        .lines()
        .filter_map(|line| {
            let (subject, author_iso) = line.split_once('|')?;
            Some((subject.to_string(), author_iso.to_string()))
        })
        .collect::<Vec<_>>();
    ensure!(
        commits.len() >= 3,
        "expected at least 3 commits, got {}",
        commits.len()
    );

    let yesterday = expected_date_for_relative_day(1)?;
    let today = expected_date_for_relative_day(0)?;

    ensure!(
        commits.iter().any(|(subject, iso)| {
            subject == "chore: initial commit" && iso.starts_with(&yesterday)
        }),
        "missing initial commit dated {yesterday}"
    );
    ensure!(
        commits.iter().any(|(subject, iso)| {
            subject == "test: committed yesterday" && iso.starts_with(&yesterday)
        }),
        "missing yesterday checkpoint commit dated {yesterday}"
    );
    ensure!(
        commits.iter().any(|(subject, iso)| {
            subject == "test: committed today" && iso.starts_with(&today)
        }),
        "missing today checkpoint commit dated {today}"
    );

    Ok(())
}

fn repo_has_head(world: &FtfWorld) -> Result<bool> {
    let output = run_command_capture(
        world,
        "git rev-parse HEAD",
        build_git_command(world.repo_dir(), &["rev-parse", "--verify", "HEAD"], &[]),
    )?;
    Ok(output.status.success())
}

fn configure_git_identity(world: &FtfWorld) -> Result<()> {
    let commands = [
        ["config", "user.name", "Bitloops FTF"],
        ["config", "user.email", "bitloops-ftf@example.com"],
        ["config", "commit.gpgsign", "false"],
    ];

    for args in commands {
        run_git_success(world, &args, &[], "git config")?;
    }
    Ok(())
}

fn run_claude_code_prompt(world: &FtfWorld, prompt: &str) -> Result<()> {
    let command_spec = std::env::var("BITLOOPS_FTF_CLAUDE_CMD")
        .unwrap_or_else(|_| DEFAULT_CLAUDE_CODE_COMMAND.to_string());
    let output = run_command_capture(
        world,
        "claude prompt",
        build_host_shell_command(
            world,
            &format!("{command_spec} {}", shell_single_quote(prompt)),
        )?,
    )
    .context("running external Claude Code prompt")?;
    ensure_success(&output, "claude prompt")
}

fn run_bitloops_success(world: &FtfWorld, args: &[&str], label: &str) -> Result<()> {
    let output = run_command_capture(world, label, build_bitloops_command(world, args)?)
        .with_context(|| format!("running {label}"))?;
    ensure_success(&output, label)
}

fn build_host_shell_command(world: &FtfWorld, script: &str) -> Result<Command> {
    let mut command = Command::new("bash");
    command
        .args(["-lc", script])
        .current_dir(world.repo_dir())
        .env("PWD", world.repo_dir())
        .env("ACCESSIBLE", "1")
        .env("BITLOOPS_FTF_ACTIVE", "1");
    Ok(command)
}

fn run_git_success(
    world: &FtfWorld,
    args: &[&str],
    env: &[(&str, OsString)],
    label: &str,
) -> Result<()> {
    let output = run_command_capture(world, label, build_git_command(world.repo_dir(), args, env))?;
    ensure_success(&output, label)
}

fn build_bitloops_command(world: &FtfWorld, args: &[&str]) -> Result<Command> {
    let run_dir = world.run_dir();
    let home_dir = run_dir.join("home");
    let xdg_config_home = home_dir.join("xdg");
    fs::create_dir_all(&xdg_config_home)
        .with_context(|| format!("creating {}", xdg_config_home.display()))?;

    let mut command = Command::new(&world.run_config().binary_path);
    command
        .args(args)
        .current_dir(world.repo_dir())
        .env("HOME", &home_dir)
        .env("USERPROFILE", &home_dir)
        .env("XDG_CONFIG_HOME", &xdg_config_home)
        .env("ACCESSIBLE", "1")
        .env("BITLOOPS_FTF_ACTIVE", "1")
        .env_remove("BITLOOPS_DEVQL_PG_DSN")
        .env_remove("BITLOOPS_DEVQL_CH_URL")
        .env_remove("BITLOOPS_DEVQL_CH_DATABASE")
        .env_remove("BITLOOPS_DEVQL_CH_USER")
        .env_remove("BITLOOPS_DEVQL_CH_PASSWORD");
    Ok(command)
}

fn build_git_command(repo_dir: &Path, args: &[&str], env: &[(&str, OsString)]) -> Command {
    let mut command = Command::new("git");
    command.args(args).current_dir(repo_dir);
    for (key, value) in env {
        command.env(key, value);
    }
    command
}

fn run_command_capture(world: &FtfWorld, label: &str, mut command: Command) -> Result<Output> {
    let command_debug = format!("{command:?}");
    let output = command
        .output()
        .with_context(|| format!("executing {label}"))?;
    append_command_log(world, label, &command_debug, &output)?;
    Ok(output)
}

fn ensure_success(output: &Output, label: &str) -> Result<()> {
    if output.status.success() {
        return Ok(());
    }

    bail!(
        "{label} failed\nstatus: {:?}\nstdout:\n{}\nstderr:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

fn append_world_log(world: &FtfWorld, message: &str) -> Result<()> {
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(world.terminal_log_path())
        .with_context(|| format!("opening {}", world.terminal_log_path().display()))?;
    file.write_all(message.as_bytes())
        .with_context(|| format!("writing {}", world.terminal_log_path().display()))
}

fn append_command_log(
    world: &FtfWorld,
    label: &str,
    command_debug: &str,
    output: &Output,
) -> Result<()> {
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(world.terminal_log_path())
        .with_context(|| format!("opening {}", world.terminal_log_path().display()))?;
    writeln!(file, "=== {label} ===")?;
    writeln!(file, "{command_debug}")?;
    writeln!(file, "status: {:?}", output.status)?;
    writeln!(file, "stdout:\n{}", String::from_utf8_lossy(&output.stdout))?;
    writeln!(file, "stderr:\n{}", String::from_utf8_lossy(&output.stderr))?;
    writeln!(file)?;
    Ok(())
}

fn write_run_metadata(world: &FtfWorld) -> Result<()> {
    let metadata = RunMetadata {
        scenario_name: world
            .scenario_name
            .as_deref()
            .ok_or_else(|| anyhow!("scenario name missing"))?,
        scenario_slug: world
            .scenario_slug
            .as_deref()
            .ok_or_else(|| anyhow!("scenario slug missing"))?,
        flow_name: world
            .flow_name
            .as_deref()
            .ok_or_else(|| anyhow!("flow name missing"))?,
        run_dir: world.run_dir().display().to_string(),
        repo_dir: world.repo_dir().display().to_string(),
        terminal_log: world.terminal_log_path().display().to_string(),
        binary_path: world.run_config().binary_path.display().to_string(),
        created_at: now_rfc3339()?,
    };
    let payload = serde_json::to_vec_pretty(&metadata).context("serializing ftf run metadata")?;
    fs::write(world.metadata_path(), payload)
        .with_context(|| format!("writing {}", world.metadata_path().display()))
}

fn create_offline_vite_react_ts_scaffold(repo_dir: &Path) -> Result<()> {
    let app_dir = repo_dir.join("my-app");
    let src_dir = app_dir.join("src");
    fs::create_dir_all(&src_dir).with_context(|| format!("creating {}", src_dir.display()))?;

    fs::write(
        app_dir.join("package.json"),
        "{\n  \"name\": \"my-app\",\n  \"private\": true,\n  \"version\": \"0.0.0\",\n  \"type\": \"module\",\n  \"scripts\": {\n    \"dev\": \"vite\",\n    \"build\": \"vite build\",\n    \"preview\": \"vite preview\"\n  }\n}\n",
    )
    .context("writing package.json")?;
    fs::write(
        app_dir.join("index.html"),
        "<!doctype html>\n<html lang=\"en\">\n  <head>\n    <meta charset=\"UTF-8\" />\n    <meta name=\"viewport\" content=\"width=device-width, initial-scale=1.0\" />\n    <title>Vite + React + TS</title>\n  </head>\n  <body>\n    <div id=\"root\"></div>\n    <script type=\"module\" src=\"/src/main.tsx\"></script>\n  </body>\n</html>\n",
    )
    .context("writing index.html")?;
    fs::write(
        src_dir.join("App.tsx"),
        "export function App() {\n  return <h1>Hello Vite</h1>;\n}\n",
    )
    .context("writing App.tsx")?;
    fs::write(
        src_dir.join("main.tsx"),
        "import React from 'react';\nimport ReactDOM from 'react-dom/client';\nimport { App } from './App';\n\nReactDOM.createRoot(document.getElementById('root') as HTMLElement).render(\n  <React.StrictMode>\n    <App />\n  </React.StrictMode>\n);\n",
    )
    .context("writing main.tsx")?;
    Ok(())
}

fn now_rfc3339() -> Result<String> {
    OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .context("formatting current timestamp")
}

fn git_date_for_relative_day(days_ago: i64) -> Result<String> {
    let target_date = OffsetDateTime::now_utc().date() - Duration::days(days_ago);
    let timestamp = PrimitiveDateTime::new(target_date, Time::from_hms(12, 0, 0)?)
        .assume_offset(UtcOffset::UTC);
    timestamp
        .format(&Rfc3339)
        .context("formatting git author date")
}

fn expected_date_for_relative_day(days_ago: i64) -> Result<String> {
    let timestamp = git_date_for_relative_day(days_ago)?;
    Ok(timestamp.chars().take(10).collect())
}

fn short_run_id() -> String {
    Uuid::new_v4().simple().to_string()[..8].to_string()
}

fn shell_single_quote(input: &str) -> String {
    format!("'{}'", input.replace('\'', r#"'"'"'"#))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_name_normalizes_user_input() {
        assert_eq!(
            sanitize_name("BDD Foundation: Stores"),
            "bdd-foundation-stores"
        );
        assert_eq!(sanitize_name(" Already__Slugged "), "already-slugged");
    }

    #[test]
    fn git_date_for_relative_day_uses_stable_noon_timestamp() {
        let today = git_date_for_relative_day(0).expect("today git date");
        let yesterday = git_date_for_relative_day(1).expect("yesterday git date");

        assert!(today.ends_with('Z') || today.contains("+00:00"));
        assert!(yesterday.ends_with('Z') || yesterday.contains("+00:00"));
        assert_ne!(today[..10].to_string(), yesterday[..10].to_string());
        assert!(today.contains("12:00:00"));
        assert!(yesterday.contains("12:00:00"));
    }

    #[test]
    fn offline_vite_scaffold_writes_expected_files() {
        let dir = tempfile::tempdir().expect("tempdir");
        create_offline_vite_react_ts_scaffold(dir.path()).expect("create scaffold");

        assert!(dir.path().join("my-app").join("package.json").exists());
        assert!(dir.path().join("my-app").join("index.html").exists());
        assert!(
            dir.path()
                .join("my-app")
                .join("src")
                .join("App.tsx")
                .exists()
        );
        assert!(
            dir.path()
                .join("my-app")
                .join("src")
                .join("main.tsx")
                .exists()
        );
    }

    #[test]
    fn shell_single_quote_escapes_single_quotes() {
        assert_eq!(shell_single_quote("plain"), "'plain'");
        assert_eq!(shell_single_quote("it's ok"), "'it'\"'\"'s ok'");
    }
}
