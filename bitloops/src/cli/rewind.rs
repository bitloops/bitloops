use std::env;
use std::fs;
use std::io::{self, Write};
use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result, anyhow, bail};
use clap::Args;
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::cli::enable;
use crate::cli::explain::{RewindPoint, get_branch_checkpoints_real};
use crate::config::settings;
use crate::git::{hard_reset_with_protection, has_uncommitted_changes};
use crate::host::checkpoints::session::create_session_backend_or_local;
use crate::host::checkpoints::session::state::SessionState;
use crate::host::checkpoints::strategy::manual_commit::lookup_session_id_for_commit;
use crate::host::checkpoints::strategy::manual_commit::{
    read_latest_session_content, read_session_content_by_id,
};
use crate::utils::paths;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

#[derive(Args, Debug, Clone, Default)]
pub struct RewindArgs {
    /// List available rewind points (JSON output)
    #[arg(long, conflicts_with_all = ["to", "logs_only", "reset"], default_value_t = false)]
    pub list: bool,

    /// Rewind to specific commit/checkpoint ID (non-interactive)
    #[arg(long, value_name = "id")]
    pub to: Option<String>,

    /// Only restore logs; do not modify working directory
    #[arg(long, requires = "to", default_value_t = false)]
    pub logs_only: bool,

    /// Hard reset branch to commit (destructive)
    #[arg(long, requires = "to", default_value_t = false)]
    pub reset: bool,
}

#[derive(Debug, Clone, Serialize)]
struct RewindListPoint {
    id: String,
    message: String,
    date: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    checkpoint_id: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    session_id: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    session_prompt: String,
    is_logs_only: bool,
    is_task_checkpoint: bool,
    #[serde(skip_serializing_if = "String::is_empty")]
    tool_use_id: String,
}

pub fn run(args: &RewindArgs) -> Result<()> {
    let repo_root = paths::repo_root()?;

    {
        let mut out = io::stdout().lock();
        let policy_start = env::current_dir().unwrap_or_else(|_| repo_root.clone());
        if enable::check_disabled_guard(&policy_start, &mut out) {
            return Ok(());
        }
    }

    if args.list {
        return run_list(&repo_root);
    }

    if let Some(target) = &args.to {
        return run_to(&repo_root, target, args.logs_only, args.reset);
    }

    run_interactive(&repo_root)
}

fn run_list(repo_root: &Path) -> Result<()> {
    let points = get_branch_checkpoints_real(repo_root, 20)?;
    let payload: Vec<RewindListPoint> = points
        .into_iter()
        .map(|point| RewindListPoint {
            id: point.id,
            message: point.message,
            date: point.date,
            checkpoint_id: point.checkpoint_id,
            session_id: point.session_id,
            session_prompt: point.session_prompt,
            is_logs_only: point.is_logs_only,
            is_task_checkpoint: point.is_task_checkpoint,
            tool_use_id: point.tool_use_id,
        })
        .collect();

    let mut out = io::stdout().lock();
    serde_json::to_writer_pretty(&mut out, &payload).context("serializing rewind list")?;
    writeln!(out)?;
    Ok(())
}

fn run_interactive(repo_root: &Path) -> Result<()> {
    if requires_clean_worktree_for_rewind(repo_root) && has_uncommitted_changes()? {
        bail!("you have uncommitted changes. Please commit or stash them first");
    }

    let points = get_branch_checkpoints_real(repo_root, 20)?;
    if points.is_empty() {
        println!("No rewind points found.");
        println!("Rewind points are created automatically when agent sessions end.");
        return Ok(());
    }

    println!("Select a checkpoint to restore:");
    for (idx, point) in points.iter().enumerate() {
        let marker = if point.is_logs_only {
            "logs"
        } else if point.is_task_checkpoint {
            "task"
        } else {
            "shadow"
        };
        let id_display = point
            .id
            .chars()
            .take(7)
            .collect::<String>()
            .if_empty_then("-------");
        println!(
            "  {}. [{}] {} {}",
            idx + 1,
            marker,
            id_display,
            point.message
        );
    }
    println!("  0. Cancel");
    print!("Enter selection: ");
    io::stdout().flush()?;

    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    let idx = input.trim().parse::<usize>().unwrap_or(0);
    if idx == 0 || idx > points.len() {
        println!("Rewind cancelled.");
        return Ok(());
    }

    let point = &points[idx - 1];
    if point.is_logs_only {
        return handle_logs_only_restore(repo_root, point, true);
    }
    perform_full_rewind(repo_root, point)
}

fn run_to(repo_root: &Path, target: &str, logs_only: bool, reset: bool) -> Result<()> {
    if !reset && requires_clean_worktree_for_rewind(repo_root) && has_uncommitted_changes()? {
        bail!("you have uncommitted changes. Please commit or stash them first");
    }

    let points = get_branch_checkpoints_real(repo_root, 100)?;
    let selected = points
        .iter()
        .find(|point| point_matches(point, target))
        .ok_or_else(|| anyhow!("rewind point not found: {target}"))?;

    if reset {
        return handle_logs_only_reset(repo_root, selected);
    }
    if selected.is_logs_only || logs_only {
        return handle_logs_only_restore(repo_root, selected, true);
    }
    perform_full_rewind(repo_root, selected)
}

fn requires_clean_worktree_for_rewind(repo_root: &Path) -> bool {
    let policy_start = env::current_dir()
        .ok()
        .filter(|cwd| cwd.starts_with(repo_root))
        .unwrap_or_else(|| repo_root.to_path_buf());
    let strategy_name = settings::load_settings(&policy_start)
        .map(|settings| settings.strategy)
        .or_else(|_| {
            crate::config::discover_repo_policy(&policy_start).map(|policy| {
                policy
                    .capture
                    .as_object()
                    .and_then(|capture| capture.get("strategy"))
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or(settings::DEFAULT_STRATEGY)
                    .to_string()
            })
        })
        .unwrap_or_else(|_| settings::DEFAULT_STRATEGY.to_string());
    strategy_name != "manual-commit"
}

fn point_matches(point: &RewindPoint, target: &str) -> bool {
    if !point.id.is_empty()
        && (point.id == target || (target.len() >= 7 && point.id.starts_with(target)))
    {
        return true;
    }
    if !point.checkpoint_id.is_empty() && point.checkpoint_id.starts_with(target) {
        return true;
    }
    false
}

fn handle_logs_only_restore(repo_root: &Path, point: &RewindPoint, print_note: bool) -> Result<()> {
    if point.checkpoint_id.is_empty() {
        bail!("logs-only checkpoint metadata is missing");
    }

    let content = if !point.session_id.is_empty() {
        match read_session_content_by_id(repo_root, &point.checkpoint_id, &point.session_id) {
            Ok(content) => content,
            Err(err) if is_session_not_found_error(&err) => {
                read_latest_session_content(repo_root, &point.checkpoint_id)?
            }
            Err(err) => return Err(err),
        }
    } else {
        read_latest_session_content(repo_root, &point.checkpoint_id)?
    };
    let session_id = if !point.session_id.is_empty() {
        point.session_id.clone()
    } else {
        content
            .metadata
            .get("session_id")
            .and_then(|value| value.as_str())
            .unwrap_or_default()
            .to_string()
    };
    if session_id.is_empty() {
        bail!("checkpoint {} has no session id", point.checkpoint_id);
    }

    restore_claude_transcript(repo_root, &session_id, content.transcript.as_bytes())?;
    println!(
        "Restored logs for session {session_id}. Run `claude --resume {session_id}` to continue."
    );
    if print_note {
        println!("Note: Working directory unchanged. Use --reset for a hard rewind.");
    }
    Ok(())
}

fn is_session_not_found_error(err: &anyhow::Error) -> bool {
    let msg = format!("{:#}", err).to_ascii_lowercase();
    msg.contains("session") && msg.contains("not found")
}

fn handle_logs_only_reset(repo_root: &Path, point: &RewindPoint) -> Result<()> {
    let previous_head = current_head(repo_root).unwrap_or_default();

    let _ = handle_logs_only_restore(repo_root, point, false);

    if point.id.is_empty() {
        bail!("cannot reset: selected rewind point has no commit id");
    }
    let short = hard_reset_with_protection(&point.id)?;
    println!("Reset branch to {short}.");

    if !previous_head.is_empty() && previous_head != point.id {
        let undo_short = previous_head.chars().take(7).collect::<String>();
        println!("\nTo undo this reset: git reset --hard {undo_short}");
    }
    Ok(())
}

fn perform_full_rewind(repo_root: &Path, point: &RewindPoint) -> Result<()> {
    if point.id.is_empty() {
        bail!("cannot rewind: selected point has no commit id");
    }

    // manual-commit rewind restores files from the checkpoint tree
    // and prunes safe untracked files instead of hard-resetting HEAD.
    let short = if requires_clean_worktree_for_rewind(repo_root) {
        hard_reset_with_protection(&point.id)?
    } else {
        manual_commit_full_rewind(repo_root, point)?;
        point.id.chars().take(7).collect::<String>()
    };

    if !point.checkpoint_id.is_empty() {
        let _ = handle_logs_only_restore(repo_root, point, false);
    }

    println!("Rewound to {short}.");
    if !point.session_id.is_empty() {
        println!("Resume: claude --resume {}", point.session_id);
    }
    Ok(())
}

#[derive(Debug, Clone)]
struct TreeEntry {
    mode: String,
    hash: String,
    path: String,
}

fn manual_commit_full_rewind(repo_root: &Path, point: &RewindPoint) -> Result<()> {
    let entries = list_checkpoint_tree_entries(repo_root, &point.id)?;
    let checkpoint_files: std::collections::HashSet<String> =
        entries.iter().map(|entry| entry.path.clone()).collect();
    let tracked_files = list_head_tracked_files(repo_root).unwrap_or_default();
    let preserved_untracked = load_preserved_untracked_files(repo_root, point);
    let untracked_now = list_untracked_files(repo_root).unwrap_or_default();

    for path in untracked_now {
        if checkpoint_files.contains(&path)
            || tracked_files.contains(&path)
            || preserved_untracked.contains(&path)
        {
            continue;
        }

        let abs = repo_root.join(&path);
        if abs.is_file() || abs.is_symlink() {
            let _ = fs::remove_file(&abs);
        } else if abs.exists() {
            let _ = fs::remove_dir_all(&abs);
        }
    }

    for entry in entries {
        restore_checkpoint_entry(repo_root, &entry)?;
    }

    if let Err(err) = reset_shadow_branch_to_checkpoint(repo_root, point) {
        eprintln!("[bitloops] Warning: failed to reset shadow branch: {err}");
    }

    Ok(())
}

fn list_checkpoint_tree_entries(repo_root: &Path, commit_id: &str) -> Result<Vec<TreeEntry>> {
    let output = git_stdout(repo_root, &["ls-tree", "-r", commit_id])?;
    let mut entries = Vec::new();
    for line in output.lines() {
        let Some((meta, path)) = line.split_once('\t') else {
            continue;
        };
        if path.is_empty() || paths::is_infrastructure_path(path) {
            continue;
        }

        let mut parts = meta.split_whitespace();
        let mode = parts.next().unwrap_or_default();
        let kind = parts.next().unwrap_or_default();
        let hash = parts.next().unwrap_or_default();
        if kind != "blob" || mode.is_empty() || hash.is_empty() {
            continue;
        }

        entries.push(TreeEntry {
            mode: mode.to_string(),
            hash: hash.to_string(),
            path: path.to_string(),
        });
    }
    Ok(entries)
}

fn restore_checkpoint_entry(repo_root: &Path, entry: &TreeEntry) -> Result<()> {
    let contents = git_stdout_bytes(repo_root, &["cat-file", "-p", &entry.hash])?;
    let abs_path = repo_root.join(&entry.path);
    if let Some(parent) = abs_path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
    }
    fs::write(&abs_path, contents)
        .with_context(|| format!("writing {}", abs_path.to_string_lossy()))?;

    #[cfg(unix)]
    {
        let mode = if entry.mode == "100755" { 0o755 } else { 0o644 };
        let mut perms = fs::metadata(&abs_path)
            .with_context(|| format!("stat {}", abs_path.to_string_lossy()))?
            .permissions();
        perms.set_mode(mode);
        fs::set_permissions(&abs_path, perms)
            .with_context(|| format!("chmod {}", abs_path.to_string_lossy()))?;
    }

    Ok(())
}

fn list_head_tracked_files(repo_root: &Path) -> Result<std::collections::HashSet<String>> {
    let output = git_stdout(repo_root, &["ls-tree", "-r", "--name-only", "HEAD"])?;
    Ok(output
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(ToString::to_string)
        .collect())
}

fn list_untracked_files(repo_root: &Path) -> Result<Vec<String>> {
    let output = git_stdout(repo_root, &["ls-files", "--others", "--exclude-standard"])?;
    Ok(output
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !paths::is_infrastructure_path(line))
        .map(ToString::to_string)
        .collect())
}

fn load_preserved_untracked_files(
    repo_root: &Path,
    point: &RewindPoint,
) -> std::collections::HashSet<String> {
    let Some(session_id) = resolve_point_session_id(repo_root, point) else {
        return std::collections::HashSet::new();
    };

    let backend = create_session_backend_or_local(repo_root);
    match backend.load_session(&session_id) {
        Ok(Some(state)) => state.untracked_files_at_start.into_iter().collect(),
        _ => std::collections::HashSet::new(),
    }
}

fn reset_shadow_branch_to_checkpoint(repo_root: &Path, point: &RewindPoint) -> Result<()> {
    let Some(session_id) = resolve_point_session_id(repo_root, point) else {
        return Ok(());
    };

    let backend = create_session_backend_or_local(repo_root);
    let Some(state) = backend.load_session(&session_id)? else {
        return Ok(());
    };
    if state.base_commit.is_empty() {
        return Ok(());
    }

    let shadow_ref = shadow_branch_ref_for_session_state(&state);
    git_stdout(repo_root, &["update-ref", &shadow_ref, &point.id])?;
    Ok(())
}

fn resolve_point_session_id(repo_root: &Path, point: &RewindPoint) -> Option<String> {
    if !point.session_id.trim().is_empty() {
        return Some(point.session_id.clone());
    }
    // Look up session_id via the relational DB: commit_sha → checkpoint_id → session_id
    lookup_session_id_for_commit(repo_root, &point.id)
        .ok()
        .flatten()
}

fn shadow_branch_ref_for_session_state(state: &SessionState) -> String {
    let short = &state.base_commit[..state.base_commit.len().min(7)];
    let worktree_hash = sha256_hex(state.worktree_id.as_bytes());
    format!("refs/heads/bitloops/{short}-{}", &worktree_hash[..6])
}

fn sha256_hex(data: &[u8]) -> String {
    hex::encode(Sha256::digest(data))
}

fn git_stdout(repo_root: &Path, args: &[&str]) -> Result<String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(repo_root)
        .output()
        .with_context(|| format!("running git {}", args.join(" ")))?;
    if !output.status.success() {
        bail!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

fn git_stdout_bytes(repo_root: &Path, args: &[&str]) -> Result<Vec<u8>> {
    let output = Command::new("git")
        .args(args)
        .current_dir(repo_root)
        .output()
        .with_context(|| format!("running git {}", args.join(" ")))?;
    if !output.status.success() {
        bail!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(output.stdout)
}

fn restore_claude_transcript(repo_root: &Path, session_id: &str, transcript: &[u8]) -> Result<()> {
    let project_dir = paths::get_claude_project_dir(&repo_root.to_string_lossy())?;
    let session_file = project_dir
        .join("sessions")
        .join(format!("{session_id}.jsonl"));
    if let Some(parent) = session_file.parent() {
        fs::create_dir_all(parent).with_context(|| {
            format!("creating transcript directory {}", parent.to_string_lossy())
        })?;
    }
    fs::write(&session_file, transcript)
        .with_context(|| format!("writing transcript to {}", session_file.to_string_lossy()))?;
    Ok(())
}

fn current_head(repo_root: &Path) -> Result<String> {
    let output = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(repo_root)
        .output()
        .context("running git rev-parse HEAD")?;
    if !output.status.success() {
        bail!(
            "failed to resolve HEAD: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

trait EmptyFallback {
    fn if_empty_then(self, fallback: &str) -> String;
}

impl EmptyFallback for String {
    fn if_empty_then(self, fallback: &str) -> String {
        if self.is_empty() {
            fallback.to_string()
        } else {
            self
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::explain::RewindPoint;
    use crate::config::settings::{self, BitloopsSettings};
    use crate::test_support::process_state::with_env_var;
    use tempfile::TempDir;

    fn write_strategy_config(repo_root: &Path, strategy: &str) {
        let settings = BitloopsSettings {
            strategy: strategy.to_string(),
            ..Default::default()
        };
        settings::save_settings(&settings, &settings::settings_path(repo_root))
            .expect("write repo policy");
    }

    fn sample_point() -> RewindPoint {
        RewindPoint {
            id: "abcdef1234567890".to_string(),
            message: "sample".to_string(),
            date: "2026-03-24T00:00:00Z".to_string(),
            checkpoint_id: "chk1234567890".to_string(),
            session_id: String::new(),
            session_prompt: String::new(),
            is_logs_only: true,
            is_task_checkpoint: false,
            tool_use_id: String::new(),
        }
    }

    fn git_ok(repo_root: &Path, args: &[&str]) -> String {
        let output = Command::new("git")
            .args(args)
            .current_dir(repo_root)
            .output()
            .expect("run git");
        assert!(
            output.status.success(),
            "git {:?} failed:\nstdout:\n{}\nstderr:\n{}",
            args,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).trim().to_string()
    }

    fn init_git_repo() -> TempDir {
        let dir = TempDir::new().expect("tempdir");
        git_ok(dir.path(), &["init"]);
        git_ok(dir.path(), &["checkout", "-B", "main"]);
        git_ok(dir.path(), &["config", "user.name", "Rewind Test"]);
        git_ok(
            dir.path(),
            &["config", "user.email", "rewind-test@example.com"],
        );
        git_ok(dir.path(), &["config", "commit.gpgsign", "false"]);
        git_ok(dir.path(), &["config", "tag.gpgsign", "false"]);
        dir
    }

    fn commit_file(repo_root: &Path, file_path: &str, content: &str, message: &str) -> String {
        std::fs::write(repo_root.join(file_path), content).expect("write file");
        git_ok(repo_root, &["add", file_path]);
        git_ok(repo_root, &["commit", "-m", message]);
        git_ok(repo_root, &["rev-parse", "HEAD"])
    }

    #[test]
    fn point_matches_accepts_full_id_and_short_prefix() {
        let point = sample_point();

        assert!(point_matches(&point, "abcdef1234567890"));
        assert!(point_matches(&point, "abcdef1"));
        assert!(!point_matches(&point, "abc"));
    }

    #[test]
    fn point_matches_accepts_checkpoint_prefix() {
        let point = sample_point();
        assert!(point_matches(&point, "chk1234"));
        assert!(!point_matches(&point, "missing"));
    }

    #[test]
    fn is_session_not_found_error_detects_expected_text() {
        let not_found = anyhow::anyhow!("Session 123 not found");
        let unrelated = anyhow::anyhow!("permission denied");

        assert!(is_session_not_found_error(&not_found));
        assert!(!is_session_not_found_error(&unrelated));
    }

    #[test]
    fn handle_logs_only_restore_errors_when_checkpoint_id_missing() {
        let temp = TempDir::new().expect("tempdir");
        let point = RewindPoint {
            checkpoint_id: String::new(),
            ..sample_point()
        };

        let err = handle_logs_only_restore(temp.path(), &point, false)
            .expect_err("missing checkpoint id must fail");
        assert!(format!("{err:#}").contains("logs-only checkpoint metadata is missing"));
    }

    #[test]
    fn perform_full_rewind_errors_when_commit_id_missing() {
        let temp = TempDir::new().expect("tempdir");
        let point = RewindPoint {
            id: String::new(),
            ..sample_point()
        };

        let err =
            perform_full_rewind(temp.path(), &point).expect_err("missing commit id must fail");
        assert!(format!("{err:#}").contains("cannot rewind: selected point has no commit id"));
    }

    #[test]
    fn requires_clean_worktree_for_rewind_manual_commit_false_other_true() {
        let manual = TempDir::new().expect("tempdir");
        write_strategy_config(manual.path(), "manual-commit");
        assert!(!requires_clean_worktree_for_rewind(manual.path()));

        let auto = TempDir::new().expect("tempdir");
        write_strategy_config(auto.path(), "auto-commit");
        assert!(requires_clean_worktree_for_rewind(auto.path()));
    }

    #[test]
    fn empty_fallback_returns_fallback_only_for_empty_strings() {
        assert_eq!(String::new().if_empty_then("fallback"), "fallback");
        assert_eq!("value".to_string().if_empty_then("fallback"), "value");
    }

    #[test]
    fn sha256_and_shadow_branch_helpers_use_expected_prefixes() {
        let digest = sha256_hex(b"worktree-alpha");
        assert_eq!(digest.len(), 64);

        let state = SessionState {
            base_commit: "abcdef1234567890".to_string(),
            worktree_id: "worktree-alpha".to_string(),
            ..Default::default()
        };
        let shadow_ref = shadow_branch_ref_for_session_state(&state);
        assert!(shadow_ref.starts_with("refs/heads/bitloops/abcdef1-"));
        assert!(shadow_ref.ends_with(&digest[..6]));
    }

    #[test]
    fn resolve_point_session_id_prefers_embedded_session_id() {
        let repo = TempDir::new().expect("tempdir");
        let point = RewindPoint {
            session_id: "session-123".to_string(),
            ..sample_point()
        };
        assert_eq!(
            resolve_point_session_id(repo.path(), &point),
            Some("session-123".to_string())
        );
    }

    #[test]
    fn restore_claude_transcript_writes_to_override_project_dir() {
        let repo = TempDir::new().expect("tempdir");
        let claude_dir = TempDir::new().expect("claude tempdir");
        with_env_var(
            "BITLOOPS_TEST_CLAUDE_PROJECT_DIR",
            Some(claude_dir.path().to_string_lossy().as_ref()),
            || {
                restore_claude_transcript(repo.path(), "session-abc", b"{\"message\":\"hello\"}\n")
                    .expect("restore transcript");
            },
        );

        let transcript_path = claude_dir.path().join("sessions").join("session-abc.jsonl");
        assert_eq!(
            std::fs::read(&transcript_path).expect("read transcript"),
            b"{\"message\":\"hello\"}\n"
        );
    }

    #[test]
    fn git_stdout_helpers_surface_failures() {
        let repo = TempDir::new().expect("tempdir");
        let err = git_stdout(repo.path(), &["status"]).expect_err("non-repo status must fail");
        assert!(format!("{err:#}").contains("git status failed"));

        let err = git_stdout_bytes(repo.path(), &["cat-file", "-p", "missing"])
            .expect_err("non-repo cat-file must fail");
        assert!(format!("{err:#}").contains("git cat-file -p missing failed"));
    }

    #[test]
    fn git_tree_helpers_list_tracked_and_untracked_files() {
        let repo = init_git_repo();
        let _head = commit_file(repo.path(), "tracked.txt", "tracked", "initial commit");
        std::fs::write(repo.path().join("notes.txt"), "notes").expect("write untracked file");
        std::fs::create_dir_all(repo.path().join(".bitloops").join("metadata"))
            .expect("create metadata dir");
        std::fs::write(
            repo.path()
                .join(".bitloops")
                .join("metadata")
                .join("ignored.txt"),
            "ignored",
        )
        .expect("write metadata file");

        let tracked = list_head_tracked_files(repo.path()).expect("list tracked files");
        assert!(tracked.contains("tracked.txt"));

        let untracked = list_untracked_files(repo.path()).expect("list untracked files");
        assert_eq!(untracked, vec!["notes.txt".to_string()]);
    }

    #[test]
    fn list_checkpoint_tree_entries_filters_infrastructure_files() {
        let repo = init_git_repo();
        std::fs::write(repo.path().join("src.rs"), "fn main() {}\n").expect("write source");
        std::fs::create_dir_all(
            repo.path()
                .join(".bitloops")
                .join("checkpoint-artifacts")
                .join("sessions")
                .join("session-1"),
        )
        .expect("create metadata dir");
        std::fs::write(
            repo.path()
                .join(".bitloops")
                .join("checkpoint-artifacts")
                .join("sessions")
                .join("session-1")
                .join("ignored.txt"),
            "ignored",
        )
        .expect("write metadata file");
        git_ok(
            repo.path(),
            &[
                "add",
                "src.rs",
                ".bitloops/checkpoint-artifacts/sessions/session-1/ignored.txt",
            ],
        );
        git_ok(repo.path(), &["commit", "-m", "seed tree"]);
        let head = git_ok(repo.path(), &["rev-parse", "HEAD"]);

        let entries = list_checkpoint_tree_entries(repo.path(), &head).expect("list tree entries");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].path, "src.rs");
        assert_eq!(entries[0].mode, "100644");
        assert!(!entries[0].hash.is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn restore_checkpoint_entry_restores_content_and_permissions() {
        let repo = init_git_repo();
        let file_path = repo.path().join("script.sh");
        std::fs::write(&file_path, "#!/bin/sh\necho old\n").expect("write script");
        let mut perms = std::fs::metadata(&file_path)
            .expect("stat script")
            .permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&file_path, perms).expect("chmod script");
        git_ok(repo.path(), &["add", "script.sh"]);
        git_ok(repo.path(), &["commit", "-m", "add script"]);
        let head = git_ok(repo.path(), &["rev-parse", "HEAD"]);

        let entry = list_checkpoint_tree_entries(repo.path(), &head)
            .expect("list tree entries")
            .into_iter()
            .find(|entry| entry.path == "script.sh")
            .expect("script entry");

        std::fs::write(&file_path, "#!/bin/sh\necho mutated\n").expect("mutate script");
        let mut perms = std::fs::metadata(&file_path)
            .expect("stat mutated script")
            .permissions();
        perms.set_mode(0o644);
        std::fs::set_permissions(&file_path, perms).expect("chmod mutated script");

        restore_checkpoint_entry(repo.path(), &entry).expect("restore checkpoint entry");

        assert_eq!(
            std::fs::read_to_string(&file_path).expect("read restored script"),
            "#!/bin/sh\necho old\n"
        );
        let restored_mode = std::fs::metadata(&file_path)
            .expect("stat restored script")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(restored_mode, 0o755);
    }

    #[test]
    fn helper_paths_tolerate_missing_session_state() {
        let repo = init_git_repo();
        let sqlite_path = crate::utils::paths::default_relational_db_path(repo.path());
        let sqlite = crate::storage::SqliteConnectionPool::connect(sqlite_path)
            .expect("create sqlite database");
        sqlite
            .initialise_checkpoint_schema()
            .expect("initialise checkpoint schema");

        let point = RewindPoint {
            session_id: "session-404".to_string(),
            ..sample_point()
        };

        let preserved = load_preserved_untracked_files(repo.path(), &point);
        assert!(preserved.is_empty());

        reset_shadow_branch_to_checkpoint(repo.path(), &point)
            .expect("missing session backend state should be a no-op");
    }

    #[test]
    fn current_head_and_manual_commit_full_rewind_restore_checkpoint_tree() {
        let repo = init_git_repo();
        write_strategy_config(repo.path(), "manual-commit");

        let original_sha = commit_file(repo.path(), "app.txt", "old\n", "original");
        let head_sha = commit_file(repo.path(), "app.txt", "new\n", "updated");
        std::fs::write(repo.path().join("scratch.txt"), "remove me").expect("write scratch file");

        let point = RewindPoint {
            id: original_sha.clone(),
            checkpoint_id: String::new(),
            session_id: String::new(),
            session_prompt: String::new(),
            is_logs_only: false,
            ..sample_point()
        };

        perform_full_rewind(repo.path(), &point).expect("manual-commit rewind should succeed");

        assert_eq!(
            std::fs::read_to_string(repo.path().join("app.txt")).expect("read restored file"),
            "old\n"
        );
        assert!(
            !repo.path().join("scratch.txt").exists(),
            "untracked file should be pruned during manual-commit rewind"
        );
        assert_eq!(
            current_head(repo.path()).expect("resolve HEAD"),
            head_sha,
            "manual-commit rewind should not move HEAD"
        );
    }
}
