// ── Git utilities ─────────────────────────────────────────────────────────────

#[cfg(test)]
use crate::test_support::process_state::git_command;

fn new_git_command() -> Command {
    #[cfg(test)]
    {
        git_command()
    }

    #[cfg(not(test))]
    {
        Command::new("git")
    }
}

/// Runs a git command and returns trimmed stdout. Errors if the command fails.
pub fn run_git(repo_root: &Path, args: &[&str]) -> Result<String> {
    run_git_env(repo_root, args, &[])
}

/// Runs a git command with extra environment variables and returns trimmed stdout.
pub fn run_git_env(repo_root: &Path, args: &[&str], env: &[(&str, &str)]) -> Result<String> {
    let mut cmd = new_git_command();
    cmd.args(args).current_dir(repo_root).stdin(Stdio::null());

    for (k, v) in env {
        cmd.env(k, v);
    }

    let output = cmd
        .output()
        .with_context(|| format!("running git {}", args.join(" ")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!(
            "git {} failed ({}): {}",
            args.join(" "),
            output.status,
            stderr.trim()
        );
    }

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    if should_preserve_stdout(args) {
        return Ok(stdout);
    }
    Ok(stdout.trim().to_string())
}

fn should_preserve_stdout(args: &[&str]) -> bool {
    if args.first() != Some(&"show") {
        return false;
    }
    args.iter()
        .skip(1)
        .any(|arg| !arg.starts_with('-') && arg.contains(':'))
}

/// Returns the full SHA of HEAD.
fn head_hash(repo_root: &Path) -> Result<String> {
    run_git(repo_root, &["rev-parse", "HEAD"])
}

/// Returns `Some(HEAD)` when available, `None` when the repo has no commits yet.
/// Any other git error is returned.
fn try_head_hash(repo_root: &Path) -> Result<Option<String>> {
    match head_hash(repo_root) {
        Ok(h) => Ok(Some(h)),
        Err(e) if is_missing_head_error(&e) => Ok(None),
        Err(e) => Err(e),
    }
}

/// Detects git errors that mean "HEAD does not exist yet" (fresh repo, no commits).
fn is_missing_head_error(err: &anyhow::Error) -> bool {
    let msg = err.to_string();
    msg.contains("ambiguous argument 'HEAD'")
        || msg.contains("unknown revision or path not in the working tree")
        || msg.contains("Needed a single revision")
}

/// Returns the shadow branch ref: `refs/heads/bitloops/<hash[:7]>-<sha256(worktree_id)[:6]>`.
///
/// Includes the worktree hash suffix even for the main worktree
/// (`worktree_id == ""`), producing the deterministic empty-string hash `e3b0c4`.
///
fn shadow_branch_ref(base_commit: &str, worktree_id: &str) -> String {
    let short = &base_commit[..base_commit.len().min(7)];
    let wt_hash = sha256_hex(worktree_id.as_bytes());
    format!("refs/heads/bitloops/{short}-{}", &wt_hash[..6])
}

/// Parses shadow branch names into `(commit_prefix, worktree_hash, ok)`.
///
/// Supports both fully qualified refs and short names:
/// - `refs/heads/bitloops/<commit>`
/// - `refs/heads/bitloops/<commit>-<worktree_hash>`
/// - `bitloops/<commit>`
/// - `bitloops/<commit>-<worktree_hash>`
fn parse_shadow_branch_name(branch_name: &str) -> (String, String, bool) {
    let suffix = if let Some(s) = branch_name.strip_prefix("refs/heads/bitloops/") {
        s
    } else if let Some(s) = branch_name.strip_prefix("bitloops/") {
        s
    } else {
        return (String::new(), String::new(), false);
    };

    if let Some((commit, worktree)) = suffix.rsplit_once('-') {
        (commit.to_string(), worktree.to_string(), true)
    } else {
        (suffix.to_string(), String::new(), true)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CleanupItem {
    pub id: String,
    pub reason: String,
}

const SESSION_GRACE_PERIOD_SECS: u64 = 10 * 60;

/// Matches bitloops/* shadow branches for cleanup.
fn is_shadow_branch(branch_name: &str) -> bool {
    if branch_name == paths::METADATA_BRANCH_NAME {
        return false;
    }
    let (commit_prefix, worktree_hash, ok) = parse_shadow_branch_name(branch_name);
    if !ok {
        return false;
    }
    if commit_prefix.len() < 7 || !commit_prefix.chars().all(|c| c.is_ascii_hexdigit()) {
        return false;
    }
    worktree_hash.is_empty()
        || (worktree_hash.len() == 6 && worktree_hash.chars().all(|c| c.is_ascii_hexdigit()))
}

fn list_shadow_branches(repo_root: &Path) -> Result<Vec<String>> {
    let refs = run_git(
        repo_root,
        &["for-each-ref", "--format=%(refname:short)", "refs/heads"],
    )?;
    let mut branches: Vec<String> = refs
        .lines()
        .map(str::trim)
        .filter(|b| !b.is_empty() && is_shadow_branch(b))
        .map(ToString::to_string)
        .collect();
    branches.sort();
    Ok(branches)
}

fn delete_shadow_branches(repo_root: &Path, branches: &[String]) -> (Vec<String>, Vec<String>) {
    if branches.is_empty() {
        return (vec![], vec![]);
    }

    let mut deleted = vec![];
    let mut failed = vec![];
    for branch in branches {
        let status = new_git_command()
            .args(["branch", "-D", branch])
            .current_dir(repo_root)
            .stdin(Stdio::null())
            .status();
        if matches!(status, Ok(s) if s.success()) {
            deleted.push(branch.clone());
        } else {
            failed.push(branch.clone());
        }
    }
    (deleted, failed)
}

fn list_orphaned_session_states(repo_root: &Path) -> Result<Vec<CleanupItem>> {
    let backend = create_session_backend_or_local(repo_root.to_path_buf());
    let states = backend.list_sessions()?;
    if states.is_empty() {
        return Ok(vec![]);
    }

    let sessions_with_checkpoints: std::collections::HashSet<String> = list_committed(repo_root)
        .unwrap_or_default()
        .into_iter()
        .map(|cp| cp.session_id)
        .filter(|sid| !sid.is_empty())
        .collect();

    let legacy_shadow_branches_enabled = crate::engine::session::legacy_local_backend_enabled();
    let shadow_branch_set: std::collections::HashSet<String> = if legacy_shadow_branches_enabled {
        list_shadow_branches(repo_root)
            .unwrap_or_default()
            .into_iter()
            .collect()
    } else {
        std::collections::HashSet::new()
    };

    let now_secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let mut orphaned = vec![];
    for state in states {
        if started_recently(&state.started_at, now_secs, SESSION_GRACE_PERIOD_SECS) {
            continue;
        }

        let has_checkpoints = sessions_with_checkpoints.contains(&state.session_id);
        let has_shadow_branch = if legacy_shadow_branches_enabled {
            let expected_branch =
                expected_shadow_branch_short_name(&state.base_commit, &state.worktree_id);
            !expected_branch.is_empty() && shadow_branch_set.contains(&expected_branch)
        } else {
            false
        };

        if !has_checkpoints && !has_shadow_branch {
            orphaned.push(CleanupItem {
                id: state.session_id,
                reason: if legacy_shadow_branches_enabled {
                    "no checkpoints or shadow branch found".to_string()
                } else {
                    "no checkpoints found".to_string()
                },
            });
        }
    }

    Ok(orphaned)
}

pub fn list_shadow_branches_for_cleanup(repo_root: &Path) -> Result<Vec<String>> {
    if !crate::engine::session::legacy_local_backend_enabled() {
        return Ok(vec![]);
    }
    list_shadow_branches(repo_root)
}

pub fn delete_shadow_branches_for_cleanup(
    repo_root: &Path,
    branches: &[String],
) -> (Vec<String>, Vec<String>) {
    if !crate::engine::session::legacy_local_backend_enabled() {
        return (vec![], vec![]);
    }
    delete_shadow_branches(repo_root, branches)
}

pub fn list_orphaned_session_states_for_cleanup(repo_root: &Path) -> Result<Vec<CleanupItem>> {
    list_orphaned_session_states(repo_root)
}

fn expected_shadow_branch_short_name(base_commit: &str, worktree_id: &str) -> String {
    if base_commit.is_empty() {
        return String::new();
    }
    shadow_branch_ref(base_commit, worktree_id)
        .strip_prefix("refs/heads/")
        .unwrap_or_default()
        .to_string()
}

fn started_recently(started_at: &str, now_secs: u64, grace_period_secs: u64) -> bool {
    let Some(started_secs) = parse_timestamp_to_unix(started_at) else {
        return false;
    };
    now_secs.saturating_sub(started_secs) < grace_period_secs
}

fn parse_timestamp_to_unix(input: &str) -> Option<u64> {
    let s = input.trim();
    if s.is_empty() {
        return None;
    }
    if let Ok(v) = s.parse::<u64>() {
        return Some(v);
    }
    parse_rfc3339_basic(s)
}

fn parse_rfc3339_basic(input: &str) -> Option<u64> {
    let (date, time_with_tz) = input.split_once('T')?;
    let mut date_parts = date.split('-');
    let year: i32 = date_parts.next()?.parse().ok()?;
    let month: u32 = date_parts.next()?.parse().ok()?;
    let day: u32 = date_parts.next()?.parse().ok()?;
    if date_parts.next().is_some() {
        return None;
    }

    let (time_part, offset_seconds) = if let Some(base) = time_with_tz.strip_suffix('Z') {
        (base, 0_i64)
    } else {
        let tz_idx = time_with_tz.rfind(['+', '-']).filter(|idx| *idx >= 8)?;
        let (base, tz) = time_with_tz.split_at(tz_idx);
        let sign = if tz.starts_with('+') { 1_i64 } else { -1_i64 };
        let tz = &tz[1..];
        let (h, m) = tz.split_once(':')?;
        let hours: i64 = h.parse().ok()?;
        let mins: i64 = m.parse().ok()?;
        if hours > 23 || mins > 59 {
            return None;
        }
        (base, sign * (hours * 3600 + mins * 60))
    };

    let time_no_frac = time_part.split('.').next()?;
    let mut time_parts = time_no_frac.split(':');
    let hour: u32 = time_parts.next()?.parse().ok()?;
    let minute: u32 = time_parts.next()?.parse().ok()?;
    let second: u32 = time_parts.next()?.parse().ok()?;
    if time_parts.next().is_some() {
        return None;
    }

    let local_secs = ymd_hms_to_unix(year, month, day, hour, minute, second)?;
    if offset_seconds >= 0 {
        local_secs.checked_sub(offset_seconds as u64)
    } else {
        local_secs.checked_add(offset_seconds.unsigned_abs())
    }
}

fn ymd_hms_to_unix(
    year: i32,
    month: u32,
    day: u32,
    hour: u32,
    minute: u32,
    second: u32,
) -> Option<u64> {
    if year < 1970 || !(1..=12).contains(&month) || hour > 23 || minute > 59 || second > 59 {
        return None;
    }
    let dim = days_in_month(year, month)?;
    if day == 0 || day > dim {
        return None;
    }

    let mut days: u64 = 0;
    for y in 1970..year {
        days += if is_leap_year(y) { 366 } else { 365 };
    }
    for m in 1..month {
        days += days_in_month(year, m)? as u64;
    }
    days += (day - 1) as u64;

    days.checked_mul(86_400)?
        .checked_add(hour as u64 * 3600)?
        .checked_add(minute as u64 * 60)?
        .checked_add(second as u64)
}

fn days_in_month(year: i32, month: u32) -> Option<u32> {
    let days = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap_year(year) => 29,
        2 => 28,
        _ => return None,
    };
    Some(days)
}

fn is_leap_year(year: i32) -> bool {
    (year % 4 == 0 && year % 100 != 0) || (year % 400 == 0)
}

/// Computes a lowercase hex SHA256 digest of `data`.
fn sha256_hex(data: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let hash = Sha256::digest(data);
    format!("{hash:x}")
}

/// Returns a 12-char lowercase hex checkpoint ID derived from a UUID v4.
fn generate_checkpoint_id() -> String {
    let id = uuid::Uuid::new_v4().simple().to_string();
    id[..12].to_string()
}

/// Returns the two-level directory parts for a checkpoint ID.
/// e.g., `"ab1234567890"` → `("ab", "1234567890")`
pub(crate) fn checkpoint_dir_parts(id: &str) -> (String, String) {
    if id.len() >= 2 {
        (id[..2].to_string(), id[2..].to_string())
    } else {
        (id.to_string(), String::new())
    }
}

/// Wrapper that owns a temp-index path and deletes the file on drop.
struct TempIndexPath(PathBuf);

impl TempIndexPath {
    fn new() -> Self {
        let id = uuid::Uuid::new_v4().simple().to_string();
        let path = std::env::temp_dir().join(format!("bitloops-idx-{}.idx", &id[..12]));
        Self(path)
    }
    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempIndexPath {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

/// Builds a git tree using a temporary index file (GIT_INDEX_FILE approach).
///
/// `parent_tree`: existing tree to start from (incremental update)
/// `modified` / `new_files`: files to add/update from the working directory
/// `deleted`: files to remove from the tree
///
/// Returns the new tree hash.
pub(crate) fn build_tree(
    repo_root: &Path,
    parent_tree: Option<&str>,
    modified: &[String],
    new_files: &[String],
    deleted: &[String],
) -> Result<String> {
    let tmp = TempIndexPath::new();
    let idx_path = tmp.path().to_string_lossy().to_string();

    // Populate temp index from parent tree.
    if let Some(tree) = parent_tree {
        run_git_env(
            repo_root,
            &["read-tree", tree],
            &[("GIT_INDEX_FILE", &idx_path)],
        )?;
    }

    // Add/update files — only those that actually exist on disk.
    // Files listed as modified/new but missing on disk are treated as deleted.
    let mut extra_deleted: Vec<String> = vec![];
    let to_add: Vec<String> = modified
        .iter()
        .chain(new_files.iter())
        .filter(|f| {
            if repo_root.join(f).exists() {
                true
            } else {
                extra_deleted.push((*f).clone());
                false
            }
        })
        .cloned()
        .collect();
    if !to_add.is_empty() {
        let mut args: Vec<String> = vec!["update-index".into(), "--add".into(), "--".into()];
        args.extend(to_add);
        let str_args: Vec<&str> = args.iter().map(String::as_str).collect();
        run_git_env(repo_root, &str_args, &[("GIT_INDEX_FILE", &idx_path)])?;
    }

    // Remove deleted files (plus any from the add list that were missing on disk).
    let all_deleted: Vec<String> = deleted
        .iter()
        .chain(extra_deleted.iter())
        .cloned()
        .collect();
    if !all_deleted.is_empty() {
        let mut seen: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
        for path in all_deleted {
            if path.is_empty() || !seen.insert(path.clone()) {
                continue;
            }
            // --remove is a no-op for files not in the index. Run one-by-one so a
            // malformed path does not prevent valid deletions from being applied.
            let _ = run_git_env(
                repo_root,
                &["update-index", "--remove", "--", &path],
                &[("GIT_INDEX_FILE", &idx_path)],
            );
        }
    }

    // Write the new tree object.
    let tree = run_git_env(repo_root, &["write-tree"], &[("GIT_INDEX_FILE", &idx_path)])?;

    Ok(tree)
}

/// Builds a git tree using explicit `(disk_path → tree_path)` file pairs.
///
/// Unlike `build_tree`, this function allows files to be stored at arbitrary tree
/// paths regardless of their actual on-disk location. Useful for the checkpoints
/// branch where files live in `.bitloops/tmp/` but must appear at sharded paths
/// like `<cp[:2]>/<cp[2:]>/metadata.json` in the tree.
///
/// Uses `git hash-object -w` to create blob objects, then
/// `git update-index --cacheinfo` to register them at the desired tree paths.
pub(crate) fn build_tree_with_explicit_paths(
    repo_root: &Path,
    parent_tree: Option<&str>,
    files: &[(PathBuf, String)],
) -> Result<String> {
    let tmp = TempIndexPath::new();
    let idx_path = tmp.path().to_string_lossy().to_string();

    // Start from parent tree if provided.
    if let Some(tree) = parent_tree {
        run_git_env(
            repo_root,
            &["read-tree", tree],
            &[("GIT_INDEX_FILE", &idx_path)],
        )?;
    }

    // Hash each file and register it at the desired tree path.
    for (disk_path, tree_path) in files {
        let hash = run_git(
            repo_root,
            &["hash-object", "-w", &disk_path.to_string_lossy()],
        )?;
        run_git_env(
            repo_root,
            &[
                "update-index",
                "--add",
                "--cacheinfo",
                &format!("100644,{hash},{tree_path}"),
            ],
            &[("GIT_INDEX_FILE", &idx_path)],
        )?;
    }

    let tree = run_git_env(repo_root, &["write-tree"], &[("GIT_INDEX_FILE", &idx_path)])?;
    Ok(tree)
}

/// Returns `(modified, new_files, deleted)` from `git status --porcelain`.
fn working_tree_changes(repo_root: &Path) -> Result<(Vec<String>, Vec<String>, Vec<String>)> {
    let output = new_git_command()
        .args(["status", "--porcelain=v1", "-z", "--untracked-files=all"])
        .current_dir(repo_root)
        .stdin(Stdio::null())
        .output()
        .context("running git status --porcelain=v1 -z")?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!(
            "git status --porcelain=v1 -z failed ({}): {}",
            output.status,
            stderr.trim()
        );
    }

    let mut modified = vec![];
    let mut new_files = vec![];
    let mut deleted = vec![];

    let mut i = 0usize;
    let bytes = output.stdout;
    while i < bytes.len() {
        let Some(end) = bytes[i..].iter().position(|b| *b == 0).map(|v| i + v) else {
            break;
        };
        let entry = &bytes[i..end];
        i = end + 1;

        if entry.len() < 4 || entry[2] != b' ' {
            continue;
        }

        let x = entry[0] as char;
        let y = entry[1] as char;
        let mut old_name: Option<String> = None;
        let mut file = String::from_utf8_lossy(&entry[3..]).to_string();

        if x == 'R' || y == 'R' || x == 'C' || y == 'C' {
            // In porcelain v1 -z mode, rename/copy entries are:
            // "XY <new-path>\0<old-path>\0"
            let new_path = file;
            let Some(end2) = bytes[i..].iter().position(|b| *b == 0).map(|v| i + v) else {
                break;
            };
            let old_path = String::from_utf8_lossy(&bytes[i..end2]).to_string();
            i = end2 + 1;
            old_name = Some(old_path);
            file = new_path;
        }

        if file.is_empty() || paths::is_infrastructure_path(&file) {
            continue;
        }
        // Skip directory entries (defensive), git update-index --add cannot handle them.
        if file.ends_with('/') {
            continue;
        }

        if x == '?' && y == '?' {
            new_files.push(file);
            continue;
        }

        if x == 'C' || y == 'C' {
            new_files.push(file);
            continue;
        }

        if x == 'R' || y == 'R' {
            if let Some(old) = old_name
                && !old.is_empty()
                && !paths::is_infrastructure_path(&old)
            {
                deleted.push(old);
            }
            modified.push(file);
            continue;
        }

        if x == 'D' || y == 'D' {
            deleted.push(file);
            continue;
        }

        if x == 'A' || y == 'A' {
            new_files.push(file);
            continue;
        }

        if x == 'M' || y == 'M' || x == 'T' || y == 'T' {
            modified.push(file);
        }
    }

    Ok((modified, new_files, deleted))
}

/// Merges `new_files` into `existing`, deduplicating.
fn merge_files_touched(existing: &mut Vec<String>, new_files: &[String]) {
    use std::collections::HashSet;
    let mut seen: HashSet<String> = existing.iter().cloned().collect();
    for f in new_files {
        if !seen.contains(f) {
            seen.insert(f.clone());
            existing.push(f.clone());
        }
    }
}

fn collect_untracked_files_at_start(repo_root: &Path) -> Vec<String> {
    run_git(repo_root, &["ls-files", "--others", "--exclude-standard"])
        .map(|out| {
            out.lines()
                .map(str::trim)
                .filter(|line| !line.is_empty() && !paths::is_infrastructure_path(line))
                .map(ToString::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn truncate_prompt_for_storage(prompt: &str) -> String {
    strings::truncate_runes(&strings::collapse_whitespace(prompt), 100, "...")
}

fn generate_context_from_prompts(prompts: &[String]) -> String {
    if prompts.is_empty() {
        return String::new();
    }
    let mut out = String::from("# Session Context\n\n## User Prompts\n\n");
    for (idx, prompt) in prompts.iter().enumerate() {
        let mut display = prompt.clone();
        if display.chars().count() > 500 {
            display = strings::truncate_runes(&display, 500, "...");
        }
        out.push_str(&format!("### Prompt {}\n\n{}\n\n", idx + 1, display));
    }
    out
}
