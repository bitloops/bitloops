use std::fs;
use std::io::{self, Write};
use std::path::Path;
use std::process::Command;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};

use crate::engine::agent::agent_display_name;
use crate::engine::session::local_backend::LocalFileBackend;
use crate::engine::session::phase::SessionPhase as RuntimeSessionPhase;
use crate::engine::session::state::SessionState as RuntimeSessionState;
use crate::engine::settings;
use crate::engine::strategy::manual_commit::ManualCommitStrategy;

pub const STALENESS_THRESHOLD: Duration = Duration::from_secs(60 * 60);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionPhase {
    Active,
    Ended,
    Idle,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionState {
    pub session_id: String,
    pub base_commit: String,
    pub worktree_id: String,
    pub phase: SessionPhase,
    pub step_count: i32,
    pub files_touched_count: i32,
    pub last_interaction_time: Option<SystemTime>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StuckSession {
    pub reason: String,
    pub shadow_branch: String,
    pub has_shadow_branch: bool,
    pub checkpoint_count: i32,
    pub files_touched_count: i32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DoctorRepo {
    pub existing_shadow_branches: Vec<String>,
}

pub fn shadow_branch_name_for_commit(base_commit: &str, worktree_id: &str) -> String {
    if base_commit.is_empty() {
        return String::new();
    }
    let short = &base_commit[..base_commit.len().min(7)];
    let wt_hash = {
        let mut hasher = Sha256::new();
        hasher.update(worktree_id.as_bytes());
        let digest = hasher.finalize();
        digest
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    };
    format!("bitloops/{short}-{}", &wt_hash[..6])
}

pub fn classify_session(
    state: &SessionState,
    _repo: &DoctorRepo,
    _now: SystemTime,
) -> Option<StuckSession> {
    let shadow_branch = shadow_branch_name_for_commit(&state.base_commit, &state.worktree_id);
    let has_shadow_branch = _repo
        .existing_shadow_branches
        .iter()
        .any(|branch| branch == &shadow_branch);

    match state.phase {
        SessionPhase::Active => {
            let reason = match state.last_interaction_time {
                None => "active, no recorded interaction time".to_string(),
                Some(last) => match _now.duration_since(last) {
                    Ok(elapsed) if elapsed > STALENESS_THRESHOLD => {
                        let minutes = elapsed.as_secs() / 60;
                        format!("active, last interaction {minutes}m ago")
                    }
                    _ => return None,
                },
            };

            Some(StuckSession {
                reason,
                shadow_branch,
                has_shadow_branch,
                checkpoint_count: state.step_count,
                files_touched_count: state.files_touched_count,
            })
        }
        SessionPhase::Ended => {
            if state.step_count <= 0 || !has_shadow_branch {
                return None;
            }

            Some(StuckSession {
                reason: "ended with uncondensed checkpoint data".to_string(),
                shadow_branch,
                has_shadow_branch,
                checkpoint_count: state.step_count,
                files_touched_count: state.files_touched_count,
            })
        }
        SessionPhase::Idle => None,
    }
}

pub fn run_doctor(force: bool) -> Result<()> {
    let repo_root = crate::engine::paths::repo_root()?;
    let backend = LocalFileBackend::new(&repo_root);
    let states = backend.list_sessions()?;
    let mut out = io::stdout().lock();
    let mut err = io::stderr().lock();

    if states.is_empty() {
        writeln!(out, "No stuck sessions found.")?;
        return Ok(());
    }

    let repo = DoctorRepo {
        existing_shadow_branches: list_shadow_branches(&repo_root),
    };
    let now = SystemTime::now();
    let strategy_name = settings::load_settings(&repo_root)
        .map(|s| s.strategy)
        .unwrap_or_else(|_| settings::DEFAULT_STRATEGY.to_string());
    let can_condense = strategy_name == "manual-commit";

    let mut stuck = Vec::<(RuntimeSessionState, StuckSession)>::new();
    for state in states {
        let doctor_state = map_runtime_state(&state);
        if let Some(ss) = classify_session(&doctor_state, &repo, now) {
            stuck.push((state, ss));
        }
    }

    if stuck.is_empty() {
        writeln!(out, "No stuck sessions found.")?;
        return Ok(());
    }

    writeln!(out, "Found {} stuck session(s):\n", stuck.len())?;

    for (state, stuck_session) in stuck {
        display_stuck_session(&mut out, &state, &stuck_session)?;

        if force {
            if can_condense && stuck_session.has_shadow_branch && stuck_session.checkpoint_count > 0
            {
                match condense_session(&repo_root, &state.session_id) {
                    Ok(()) => writeln!(out, "  -> Condensed session {}\n", state.session_id)?,
                    Err(e) => writeln!(
                        err,
                        "Warning: failed to condense session {}: {}",
                        state.session_id, e
                    )?,
                }
            } else {
                match discard_session(&repo_root, &backend, &state, &stuck_session) {
                    Ok(()) => writeln!(out, "  -> Discarded session {}\n", state.session_id)?,
                    Err(e) => writeln!(
                        err,
                        "Warning: failed to discard session {}: {}",
                        state.session_id, e
                    )?,
                }
            }
            continue;
        }

        let action = prompt_action(&state.session_id, &stuck_session, can_condense)?;
        match action.as_str() {
            "condense" => {
                if let Err(e) = condense_session(&repo_root, &state.session_id) {
                    writeln!(
                        err,
                        "Warning: failed to condense session {}: {}",
                        state.session_id, e
                    )?;
                } else {
                    writeln!(out, "  -> Condensed session {}\n", state.session_id)?;
                }
            }
            "discard" => {
                if let Err(e) = discard_session(&repo_root, &backend, &state, &stuck_session) {
                    writeln!(
                        err,
                        "Warning: failed to discard session {}: {}",
                        state.session_id, e
                    )?;
                } else {
                    writeln!(out, "  -> Discarded session {}\n", state.session_id)?;
                }
            }
            _ => {
                writeln!(out, "  -> Skipped\n")?;
            }
        }
    }

    Ok(())
}

fn map_runtime_state(state: &RuntimeSessionState) -> SessionState {
    SessionState {
        session_id: state.session_id.clone(),
        base_commit: state.base_commit.clone(),
        worktree_id: state.worktree_id.clone(),
        phase: match state.phase {
            RuntimeSessionPhase::Active => SessionPhase::Active,
            RuntimeSessionPhase::Ended => SessionPhase::Ended,
            RuntimeSessionPhase::Idle => SessionPhase::Idle,
        },
        step_count: state.step_count as i32,
        files_touched_count: state.files_touched.len() as i32,
        last_interaction_time: state
            .last_interaction_time
            .as_deref()
            .and_then(parse_time_string),
    }
}

fn parse_time_string(value: &str) -> Option<SystemTime> {
    let s = value.trim();
    if s.is_empty() {
        return None;
    }
    if let Ok(unix) = s.parse::<u64>() {
        return Some(UNIX_EPOCH + Duration::from_secs(unix));
    }
    parse_rfc3339_basic(s).map(|unix| UNIX_EPOCH + Duration::from_secs(unix))
}

fn parse_rfc3339_basic(input: &str) -> Option<u64> {
    let (date, time_with_tz) = input.split_once('T')?;
    let mut date_parts = date.split('-');
    let year: i32 = date_parts.next()?.parse().ok()?;
    let month: u32 = date_parts.next()?.parse().ok()?;
    let day: u32 = date_parts.next()?.parse().ok()?;

    let (time_part, offset_seconds) = if let Some(base) = time_with_tz.strip_suffix('Z') {
        (base, 0_i64)
    } else if let Some(idx) = time_with_tz.rfind('+') {
        let (base, off) = time_with_tz.split_at(idx);
        (base, parse_tz_offset(off)?)
    } else if let Some(idx) = time_with_tz.rfind('-') {
        if idx > 2 {
            let (base, off) = time_with_tz.split_at(idx);
            (base, parse_tz_offset(off)?)
        } else {
            (time_with_tz, 0_i64)
        }
    } else {
        (time_with_tz, 0_i64)
    };

    let mut time_parts = time_part.split(':');
    let hour: u32 = time_parts.next()?.parse().ok()?;
    let minute: u32 = time_parts.next()?.parse().ok()?;
    let second_str = time_parts.next()?;
    let second: u32 = second_str.split('.').next()?.parse().ok()?;

    let days = days_since_unix_epoch(year, month, day)?;
    let secs = days
        .checked_mul(86_400)?
        .checked_add(hour as i64 * 3_600)?
        .checked_add(minute as i64 * 60)?
        .checked_add(second as i64)?
        .checked_sub(offset_seconds)?;
    if secs < 0 {
        return None;
    }
    Some(secs as u64)
}

fn parse_tz_offset(raw: &str) -> Option<i64> {
    if raw.len() != 6 {
        return None;
    }
    let sign = match &raw[0..1] {
        "+" => 1_i64,
        "-" => -1_i64,
        _ => return None,
    };
    if &raw[3..4] != ":" {
        return None;
    }
    let hours: i64 = raw[1..3].parse().ok()?;
    let minutes: i64 = raw[4..6].parse().ok()?;
    Some(sign * (hours * 3600 + minutes * 60))
}

fn days_since_unix_epoch(year: i32, month: u32, day: u32) -> Option<i64> {
    if !(1..=12).contains(&month) || day == 0 || day > days_in_month(year, month) {
        return None;
    }
    let mut days = 0_i64;

    if year >= 1970 {
        for y in 1970..year {
            days += if is_leap_year(y) { 366 } else { 365 };
        }
    } else {
        for y in year..1970 {
            days -= if is_leap_year(y) { 366 } else { 365 };
        }
    }

    for m in 1..month {
        days += days_in_month(year, m) as i64;
    }
    days += (day - 1) as i64;
    Some(days)
}

fn days_in_month(year: i32, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            if is_leap_year(year) {
                29
            } else {
                28
            }
        }
        _ => 0,
    }
}

fn is_leap_year(year: i32) -> bool {
    (year % 4 == 0 && year % 100 != 0) || (year % 400 == 0)
}

fn list_shadow_branches(repo_root: &Path) -> Vec<String> {
    let mut all = Vec::<String>::new();
    for pattern in ["bitloops/*"] {
        let output = Command::new("git")
            .args(["branch", "--list", pattern])
            .current_dir(repo_root)
            .output();
        let Ok(output) = output else { continue };
        if !output.status.success() {
            continue;
        }
        for line in String::from_utf8_lossy(&output.stdout).lines() {
            let branch = line.trim().trim_start_matches('*').trim();
            if !branch.is_empty() {
                all.push(branch.to_string());
            }
        }
    }
    all.sort();
    all.dedup();
    all
}

fn display_stuck_session(
    out: &mut dyn Write,
    state: &RuntimeSessionState,
    stuck: &StuckSession,
) -> io::Result<()> {
    writeln!(out, "  Session: {}", state.session_id)?;
    writeln!(out, "  Phase:   {}", state.phase.as_str())?;
    writeln!(out, "  Reason:  {}", stuck.reason)?;
    let agent = agent_display_name(&state.agent_type);
    if !agent.is_empty() {
        writeln!(out, "  Agent:   {}", agent)?;
    }
    if let Some(last) = &state.last_interaction_time {
        writeln!(out, "  Last interaction: {}", last)?;
    }
    if stuck.has_shadow_branch {
        writeln!(out, "  Shadow branch: exists ({})", stuck.shadow_branch)?;
    } else {
        writeln!(out, "  Shadow branch: not found")?;
    }
    writeln!(
        out,
        "  Checkpoints: {}, Files touched: {}",
        stuck.checkpoint_count, stuck.files_touched_count
    )?;
    Ok(())
}

fn prompt_action(session_id: &str, stuck: &StuckSession, can_condense: bool) -> Result<String> {
    let mut prompt = String::from("Choose action [d=discard, s=skip");
    let condense_allowed = can_condense && stuck.has_shadow_branch && stuck.checkpoint_count > 0;
    if condense_allowed {
        prompt = String::from("Choose action [c=condense, d=discard, s=skip");
    }
    prompt.push_str("] (default: s): ");

    let mut out = io::stdout().lock();
    write!(out, "Fix session {session_id}? {prompt}")?;
    out.flush()?;

    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    let action = input.trim().to_ascii_lowercase();
    if condense_allowed && (action == "c" || action == "condense") {
        return Ok("condense".to_string());
    }
    if action == "d" || action == "discard" {
        return Ok("discard".to_string());
    }
    Ok("skip".to_string())
}

fn condense_session(repo_root: &Path, session_id: &str) -> Result<()> {
    let strategy = ManualCommitStrategy::new(repo_root);
    strategy.condense_session_by_id(session_id)
}

fn discard_session(
    repo_root: &Path,
    backend: &LocalFileBackend,
    state: &RuntimeSessionState,
    stuck: &StuckSession,
) -> Result<()> {
    let state_path = backend
        .sessions_dir()
        .join(format!("{}.json", state.session_id));
    if state_path.exists() {
        fs::remove_file(&state_path)
            .with_context(|| format!("removing session state {}", state_path.display()))?;
    }

    if stuck.has_shadow_branch
        && can_delete_shadow_branch(backend, &stuck.shadow_branch, &state.session_id)?
    {
        let output = Command::new("git")
            .args(["branch", "-D", &stuck.shadow_branch])
            .current_dir(repo_root)
            .output()
            .context("deleting shadow branch")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            if !stderr.contains("not found") && !stderr.contains("not exist") {
                bail!(
                    "failed to delete shadow branch {}: {}",
                    stuck.shadow_branch,
                    stderr.trim()
                );
            }
        }
    }

    Ok(())
}

fn can_delete_shadow_branch(
    backend: &LocalFileBackend,
    shadow_branch: &str,
    exclude_session_id: &str,
) -> Result<bool> {
    let states = backend.list_sessions()?;
    for state in states {
        if state.session_id == exclude_session_id {
            continue;
        }
        if state.step_count == 0 {
            continue;
        }
        let expected = shadow_branch_name_for_commit(&state.base_commit, &state.worktree_id);
        if expected == shadow_branch {
            return Ok(false);
        }
    }
    Ok(true)
}

#[cfg(test)]
#[allow(non_snake_case)]
mod tests {
    use super::{
        DoctorRepo, STALENESS_THRESHOLD, SessionPhase, SessionState, classify_session,
        shadow_branch_name_for_commit,
    };
    use std::time::{Duration, SystemTime};

    const TEST_BASE_COMMIT: &str = "abcdef1234567890abcdef1234567890abcdef12";

    fn empty_repo() -> DoctorRepo {
        DoctorRepo {
            existing_shadow_branches: vec![],
        }
    }

    // CLI-547
    #[test]
    fn TestClassifySession_ActiveStale_NilInteractionTime() {
        let state = SessionState {
            session_id: "test-active-nil-time".to_string(),
            base_commit: TEST_BASE_COMMIT.to_string(),
            worktree_id: String::new(),
            phase: SessionPhase::Active,
            step_count: 3,
            files_touched_count: 0,
            last_interaction_time: None,
        };
        let result = classify_session(&state, &empty_repo(), SystemTime::now());

        assert!(
            result.is_some(),
            "active session with nil LastInteractionTime should be stuck"
        );
        let result = result.expect("expected stuck session");
        assert_eq!(result.reason, "active, no recorded interaction time");
        assert_eq!(result.checkpoint_count, 3);
        assert!(!result.has_shadow_branch);
    }

    // CLI-548
    #[test]
    fn TestClassifySession_ActiveStale_OldInteractionTime() {
        let state = SessionState {
            session_id: "test-active-stale".to_string(),
            base_commit: TEST_BASE_COMMIT.to_string(),
            worktree_id: String::new(),
            phase: SessionPhase::Active,
            step_count: 2,
            files_touched_count: 2,
            last_interaction_time: Some(SystemTime::now() - Duration::from_secs(2 * 60 * 60)),
        };
        let now = SystemTime::now();
        let result = classify_session(&state, &empty_repo(), now);

        assert!(
            result.is_some(),
            "active session with old interaction time should be stuck"
        );
        let result = result.expect("expected stuck session");
        assert!(
            result.reason.contains("active, last interaction"),
            "reason should mention stale interaction"
        );
        assert_eq!(result.checkpoint_count, 2);
        assert_eq!(result.files_touched_count, 2);
    }

    // CLI-549
    #[test]
    fn TestClassifySession_ActiveRecent_Healthy() {
        let state = SessionState {
            session_id: "test-active-healthy".to_string(),
            base_commit: TEST_BASE_COMMIT.to_string(),
            worktree_id: String::new(),
            phase: SessionPhase::Active,
            step_count: 1,
            files_touched_count: 0,
            last_interaction_time: Some(SystemTime::now() - Duration::from_secs(5 * 60)),
        };
        let result = classify_session(&state, &empty_repo(), SystemTime::now());
        assert!(
            result.is_none(),
            "active session with recent interaction should be healthy"
        );
    }

    // CLI-550
    #[test]
    fn TestClassifySession_EndedWithUncondensedData() {
        let shadow = shadow_branch_name_for_commit(TEST_BASE_COMMIT, "");
        let repo = DoctorRepo {
            existing_shadow_branches: vec![shadow.clone()],
        };
        let state = SessionState {
            session_id: "test-ended-uncondensed".to_string(),
            base_commit: TEST_BASE_COMMIT.to_string(),
            worktree_id: String::new(),
            phase: SessionPhase::Ended,
            step_count: 3,
            files_touched_count: 1,
            last_interaction_time: None,
        };
        let result = classify_session(&state, &repo, SystemTime::now());
        assert!(
            result.is_some(),
            "ended session with checkpoints and shadow branch should be stuck"
        );
        let result = result.expect("expected stuck session");
        assert_eq!(result.reason, "ended with uncondensed checkpoint data");
        assert!(result.has_shadow_branch);
        assert_eq!(result.checkpoint_count, 3);
        assert_eq!(result.files_touched_count, 1);
    }

    // CLI-551
    #[test]
    fn TestClassifySession_EndedNoShadowBranch_Healthy() {
        let state = SessionState {
            session_id: "test-ended-no-shadow".to_string(),
            base_commit: TEST_BASE_COMMIT.to_string(),
            worktree_id: String::new(),
            phase: SessionPhase::Ended,
            step_count: 3,
            files_touched_count: 0,
            last_interaction_time: None,
        };
        let result = classify_session(&state, &empty_repo(), SystemTime::now());
        assert!(
            result.is_none(),
            "ended session without shadow branch should be healthy"
        );
    }

    // CLI-552
    #[test]
    fn TestClassifySession_EndedZeroStepCount_Healthy() {
        let base_commit = "1234567890abcdef1234567890abcdef12345678";
        let shadow = shadow_branch_name_for_commit(base_commit, "");
        let repo = DoctorRepo {
            existing_shadow_branches: vec![shadow],
        };
        let state = SessionState {
            session_id: "test-ended-zero-steps".to_string(),
            base_commit: base_commit.to_string(),
            worktree_id: String::new(),
            phase: SessionPhase::Ended,
            step_count: 0,
            files_touched_count: 0,
            last_interaction_time: None,
        };
        let result = classify_session(&state, &repo, SystemTime::now());
        assert!(
            result.is_none(),
            "ended session with zero steps should be healthy even with shadow branch"
        );
    }

    // CLI-553
    #[test]
    fn TestClassifySession_IdlePhase_Healthy() {
        let state = SessionState {
            session_id: "test-idle".to_string(),
            base_commit: TEST_BASE_COMMIT.to_string(),
            worktree_id: String::new(),
            phase: SessionPhase::Idle,
            step_count: 1,
            files_touched_count: 0,
            last_interaction_time: None,
        };
        let result = classify_session(&state, &empty_repo(), SystemTime::now());
        assert!(result.is_none(), "IDLE session should be healthy");
    }

    // CLI-555
    #[test]
    fn TestClassifySession_StalenessThresholdBoundary() {
        let now = SystemTime::now();

        let state_over = SessionState {
            session_id: "test-boundary-over".to_string(),
            base_commit: TEST_BASE_COMMIT.to_string(),
            worktree_id: String::new(),
            phase: SessionPhase::Active,
            step_count: 1,
            files_touched_count: 0,
            last_interaction_time: Some(now - STALENESS_THRESHOLD - Duration::from_secs(1)),
        };
        let result_over = classify_session(&state_over, &empty_repo(), now);
        assert!(
            result_over.is_some(),
            "session just over staleness threshold should be stuck"
        );

        let state_under = SessionState {
            session_id: "test-boundary-under".to_string(),
            base_commit: TEST_BASE_COMMIT.to_string(),
            worktree_id: String::new(),
            phase: SessionPhase::Active,
            step_count: 1,
            files_touched_count: 0,
            last_interaction_time: Some(now - STALENESS_THRESHOLD + Duration::from_secs(60)),
        };
        let result_under = classify_session(&state_under, &empty_repo(), now);
        assert!(
            result_under.is_none(),
            "session just under staleness threshold should be healthy"
        );
    }

    // CLI-556
    #[test]
    fn TestClassifySession_ActiveWithShadowBranch() {
        let shadow = shadow_branch_name_for_commit(TEST_BASE_COMMIT, "");
        let repo = DoctorRepo {
            existing_shadow_branches: vec![shadow],
        };
        let state = SessionState {
            session_id: "test-active-shadow".to_string(),
            base_commit: TEST_BASE_COMMIT.to_string(),
            worktree_id: String::new(),
            phase: SessionPhase::Active,
            step_count: 2,
            files_touched_count: 0,
            last_interaction_time: None,
        };
        let result = classify_session(&state, &repo, SystemTime::now());
        assert!(result.is_some());
        let result = result.expect("expected stuck session");
        assert!(
            result.has_shadow_branch,
            "should detect existing shadow branch"
        );
        assert!(!result.shadow_branch.is_empty());
    }

    // CLI-557
    #[test]
    fn TestClassifySession_WorktreeIDInShadowBranch() {
        let worktree_id = "my-worktree";
        let shadow = shadow_branch_name_for_commit(TEST_BASE_COMMIT, worktree_id);
        let repo = DoctorRepo {
            existing_shadow_branches: vec![shadow.clone()],
        };
        let state = SessionState {
            session_id: "test-worktree-shadow".to_string(),
            base_commit: TEST_BASE_COMMIT.to_string(),
            worktree_id: worktree_id.to_string(),
            phase: SessionPhase::Ended,
            step_count: 1,
            files_touched_count: 1,
            last_interaction_time: None,
        };
        let result = classify_session(&state, &repo, SystemTime::now());
        assert!(
            result.is_some(),
            "ended session with worktree shadow branch should be stuck"
        );
        let result = result.expect("expected stuck session");
        assert!(result.has_shadow_branch);
        assert_eq!(result.shadow_branch, shadow);
    }
}
