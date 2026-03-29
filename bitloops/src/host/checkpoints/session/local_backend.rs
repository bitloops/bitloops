//! `LocalFileBackend` — file-system implementation of `SessionBackend`.
//!
//! Storage layout:
//!   `<git-common-dir>/bitloops-sessions/<session_id>.json`   — session state
//!   `<state-dir>/daemon/repos/<repo-hash>/tmp/pre-prompt-<session_id>.json` — pre-prompt state
//!   `<state-dir>/daemon/repos/<repo-hash>/tmp/pre-task-<tool_use_id>.json`   — pre-task marker
//!
//! Legacy compatibility backend. Non-test runtime only falls back to this backend
//! when `BITLOOPS_ENABLE_LEGACY_LOCAL_BACKEND=1` is set.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context, Result};

use crate::utils::paths;

use super::backend::SessionBackend;
use super::state::{PrePromptState, PreTaskState, SessionState};

pub struct LocalFileBackend {
    /// Repository root.
    repo_root: PathBuf,
}

impl LocalFileBackend {
    pub fn new(repo_root: impl Into<PathBuf>) -> Self {
        Self {
            repo_root: repo_root.into(),
        }
    }

    /// `<git-common-dir>/bitloops-sessions/`
    pub fn sessions_dir(&self) -> PathBuf {
        self.git_common_dir().join("bitloops-sessions")
    }

    /// Repo-scoped session scratch directory.
    fn tmp_dir(&self) -> PathBuf {
        paths::default_session_tmp_dir(&self.repo_root)
    }

    fn session_path(&self, session_id: &str) -> PathBuf {
        self.sessions_dir().join(format!("{session_id}.json"))
    }

    fn pre_prompt_path(&self, session_id: &str) -> PathBuf {
        self.tmp_dir().join(format!("pre-prompt-{session_id}.json"))
    }

    fn pre_task_path(&self, tool_use_id: &str) -> PathBuf {
        self.tmp_dir().join(format!("pre-task-{tool_use_id}.json"))
    }

    /// Resolves the git common dir, falling back to `<repo_root>/.git` on failure.
    fn git_common_dir(&self) -> PathBuf {
        let output = Command::new("git")
            .args(["rev-parse", "--git-common-dir"])
            .current_dir(&self.repo_root)
            .stdin(Stdio::null())
            .output();

        let Ok(output) = output else {
            return self.repo_root.join(".git");
        };
        if !output.status.success() {
            return self.repo_root.join(".git");
        }

        let dir = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if dir.is_empty() {
            return self.repo_root.join(".git");
        }
        let path = Path::new(&dir);
        if path.is_absolute() {
            path.to_path_buf()
        } else {
            self.repo_root.join(path)
        }
    }

    /// Returns all session states found in `.git/bitloops-sessions/`.
    /// Skips files that cannot be parsed (resilient to partial writes).
    pub fn list_sessions(&self) -> Result<Vec<SessionState>> {
        let dir = self.sessions_dir();
        match fs::read_dir(&dir) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(vec![]),
            Err(e) => Err(e).context("reading sessions directory"),
            Ok(entries) => {
                let mut sessions = vec![];
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.extension().and_then(|e| e.to_str()) == Some("json")
                        && let Ok(data) = fs::read_to_string(&path)
                        && let Ok(mut state) = serde_json::from_str::<SessionState>(&data)
                    {
                        state.normalize_after_load();
                        sessions.push(state);
                    }
                }
                Ok(sessions)
            }
        }
    }
}

impl SessionBackend for LocalFileBackend {
    fn list_sessions(&self) -> Result<Vec<SessionState>> {
        LocalFileBackend::list_sessions(self)
    }

    fn load_session(&self, session_id: &str) -> Result<Option<SessionState>> {
        validate_session_id(session_id)?;
        let path = self.session_path(session_id);
        match fs::read(&path) {
            Ok(data) => {
                let mut state: SessionState = serde_json::from_slice(&data)
                    .with_context(|| format!("parsing session state: {}", path.display()))?;
                state.normalize_after_load();
                Ok(Some(state))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e).with_context(|| format!("reading session state: {}", path.display())),
        }
    }

    fn save_session(&self, state: &SessionState) -> Result<()> {
        validate_session_id(&state.session_id)?;
        let path = self.session_path(&state.session_id);
        write_json(&path, state)
    }

    fn delete_session(&self, session_id: &str) -> Result<()> {
        validate_session_id(session_id)?;
        let path = self.session_path(session_id);
        remove_if_exists(&path)
            .with_context(|| format!("deleting session state: {}", path.display()))
    }

    fn load_pre_prompt(&self, session_id: &str) -> Result<Option<PrePromptState>> {
        validate_session_id(session_id)?;
        let path = self.pre_prompt_path(session_id);
        match fs::read(&path) {
            Ok(data) => {
                let mut state: PrePromptState = serde_json::from_slice(&data)
                    .with_context(|| format!("parsing pre-prompt state: {}", path.display()))?;
                state.normalize_after_load();
                Ok(Some(state))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => {
                Err(e).with_context(|| format!("reading pre-prompt state: {}", path.display()))
            }
        }
    }

    fn save_pre_prompt(&self, state: &PrePromptState) -> Result<()> {
        validate_session_id(&state.session_id)?;
        let path = self.pre_prompt_path(&state.session_id);
        write_json(&path, state)
    }

    fn delete_pre_prompt(&self, session_id: &str) -> Result<()> {
        validate_session_id(session_id)?;
        let path = self.pre_prompt_path(session_id);
        remove_if_exists(&path).with_context(|| format!("deleting pre-prompt: {}", path.display()))
    }

    fn create_pre_task_marker(&self, state: &PreTaskState) -> Result<()> {
        validate_tool_use_id(&state.tool_use_id)?;
        let path = self.pre_task_path(&state.tool_use_id);
        write_json(&path, state)
    }

    fn load_pre_task_marker(&self, tool_use_id: &str) -> Result<Option<PreTaskState>> {
        validate_tool_use_id(tool_use_id)?;
        let path = self.pre_task_path(tool_use_id);
        match fs::read(&path) {
            Ok(data) => {
                let state: PreTaskState = serde_json::from_slice(&data)
                    .with_context(|| format!("parsing pre-task state: {}", path.display()))?;
                Ok(Some(state))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e).with_context(|| format!("reading pre-task state: {}", path.display())),
        }
    }

    fn delete_pre_task_marker(&self, tool_use_id: &str) -> Result<()> {
        validate_tool_use_id(tool_use_id)?;
        let path = self.pre_task_path(tool_use_id);
        remove_if_exists(&path)
            .with_context(|| format!("deleting pre-task marker: {}", path.display()))
    }

    fn find_active_pre_task(&self) -> Result<Option<String>> {
        let dir = self.tmp_dir();
        match fs::read_dir(&dir) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e).context("reading tmp dir for pre-task markers"),
            Ok(entries) => {
                let mut latest: Option<(std::time::SystemTime, String)> = None;
                for entry in entries.flatten() {
                    let name_str = entry.file_name().to_string_lossy().to_string();
                    if !name_str.starts_with("pre-task-") || !name_str.ends_with(".json") {
                        continue;
                    }
                    let tool_use_id = name_str
                        .trim_start_matches("pre-task-")
                        .trim_end_matches(".json")
                        .to_string();
                    let modified = entry
                        .metadata()
                        .ok()
                        .and_then(|m| m.modified().ok())
                        .unwrap_or(std::time::SystemTime::UNIX_EPOCH);

                    match &latest {
                        None => latest = Some((modified, tool_use_id)),
                        Some((ts, _)) if modified > *ts => latest = Some((modified, tool_use_id)),
                        _ => {}
                    }
                }
                Ok(latest.map(|(_, id)| id))
            }
        }
    }
}

// ── helpers ──────────────────────────────────────────────────────────────────

fn write_json<T: serde::Serialize>(path: &Path, value: &T) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("creating directory: {}", parent.display()))?;
    }
    let mut data = serde_json::to_string_pretty(value).context("serializing to JSON")?;
    data.push('\n');
    fs::write(path, data).with_context(|| format!("writing file: {}", path.display()))
}

fn remove_if_exists(path: &Path) -> Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.into()),
    }
}

fn validate_session_id(session_id: &str) -> Result<()> {
    if session_id.is_empty() {
        anyhow::bail!("session ID cannot be empty");
    }
    if session_id.contains('/') || session_id.contains('\\') {
        anyhow::bail!(
            "invalid session ID {:?}: contains path separators",
            session_id
        );
    }
    Ok(())
}

fn validate_tool_use_id(tool_use_id: &str) -> Result<()> {
    if tool_use_id.is_empty() {
        return Ok(());
    }
    if tool_use_id
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        Ok(())
    } else {
        anyhow::bail!(
            "invalid tool use ID {:?}: must be alphanumeric with underscores/hyphens only",
            tool_use_id
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::checkpoints::session::phase::SessionPhase;
    use tempfile::TempDir;

    fn setup() -> (TempDir, LocalFileBackend) {
        let dir = tempfile::tempdir().unwrap();
        // Create .git/ so the backend can resolve paths
        fs::create_dir_all(dir.path().join(".git")).unwrap();
        let backend = LocalFileBackend::new(dir.path());
        (dir, backend)
    }

    fn sample_state(session_id: &str) -> SessionState {
        SessionState {
            session_id: session_id.to_string(),
            phase: SessionPhase::Active,
            transcript_path: "/tmp/t.jsonl".to_string(),
            step_count: 3,
            first_prompt: "Fix the bug".to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn load_session_returns_none_when_missing() {
        let (_dir, backend) = setup();
        let result = backend.load_session("nonexistent").unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn save_and_load_session_roundtrip() {
        let (_dir, backend) = setup();
        let state = sample_state("sess-001");
        backend.save_session(&state).unwrap();

        let loaded = backend.load_session("sess-001").unwrap().unwrap();
        assert_eq!(loaded.session_id, "sess-001");
        assert_eq!(loaded.phase, SessionPhase::Active);
        assert_eq!(loaded.step_count, 3);
        assert_eq!(loaded.first_prompt, "Fix the bug");
    }

    #[test]
    fn session_state_package_functions_roundtrip_for_manual_strategy_equivalent() {
        let (_dir, backend) = setup();
        let state = SessionState {
            session_id: "manual-s1".to_string(),
            first_prompt: "hello".to_string(),
            ..sample_state("manual-s1")
        };
        backend.save_session(&state).unwrap();
        let loaded = backend.load_session("manual-s1").unwrap().unwrap();
        assert_eq!(loaded.session_id, "manual-s1");
        assert_eq!(loaded.first_prompt, "hello");
    }

    #[test]
    fn load_session_with_ended_at_roundtrip() {
        let (_dir, backend) = setup();
        let state = SessionState {
            session_id: "sess-ended-at".to_string(),
            ended_at: Some("2026-01-01T00:00:00Z".to_string()),
            ..sample_state("sess-ended-at")
        };
        backend.save_session(&state).unwrap();

        let loaded = backend.load_session("sess-ended-at").unwrap().unwrap();
        assert_eq!(
            loaded.ended_at.as_deref(),
            Some("2026-01-01T00:00:00Z"),
            "ended_at should round-trip through package-level load/save"
        );
    }

    #[test]
    fn load_session_with_last_interaction_time_roundtrip() {
        let (_dir, backend) = setup();
        let state = SessionState {
            session_id: "sess-last-interaction".to_string(),
            last_interaction_time: Some("2026-01-01T01:23:45Z".to_string()),
            ..sample_state("sess-last-interaction")
        };
        backend.save_session(&state).unwrap();

        let loaded = backend
            .load_session("sess-last-interaction")
            .unwrap()
            .unwrap();
        assert_eq!(
            loaded.last_interaction_time.as_deref(),
            Some("2026-01-01T01:23:45Z"),
            "last_interaction_time should round-trip through package-level load/save"
        );
    }

    #[test]
    fn load_session_normalizes_legacy_phase_values() {
        let (_dir, backend) = setup();
        let sessions_dir = backend.sessions_dir();
        fs::create_dir_all(&sessions_dir).unwrap();

        fs::write(
            sessions_dir.join("sess-legacy.json"),
            r#"{"session_id":"sess-legacy","phase":"active_committed"}"#,
        )
        .unwrap();

        let loaded = backend.load_session("sess-legacy").unwrap().unwrap();
        assert_eq!(loaded.phase, SessionPhase::Active);
    }

    #[test]
    fn load_session_normalizes_unknown_or_malformed_phase_values() {
        let (_dir, backend) = setup();
        let sessions_dir = backend.sessions_dir();
        fs::create_dir_all(&sessions_dir).unwrap();

        let cases = [
            ("sess-unknown", r#""bogus""#),
            ("sess-null", "null"),
            ("sess-number", "123"),
            ("sess-object", "{}"),
        ];

        for (session_id, phase_json) in cases {
            let state_json = format!(r#"{{"session_id":"{session_id}","phase":{phase_json}}}"#);
            fs::write(sessions_dir.join(format!("{session_id}.json")), state_json).unwrap();

            let loaded = backend.load_session(session_id).unwrap().unwrap();
            assert_eq!(loaded.phase, SessionPhase::Idle, "{session_id}");
        }
    }

    #[test]
    fn save_and_load_pre_prompt_roundtrip() {
        let (_dir, backend) = setup();
        let state = PrePromptState {
            session_id: "sess-002".to_string(),
            prompt: "Hello world".to_string(),
            transcript_path: "/tmp/t.jsonl".to_string(),
            ..Default::default()
        };
        backend.save_pre_prompt(&state).unwrap();

        let loaded = backend.load_pre_prompt("sess-002").unwrap().unwrap();
        assert_eq!(loaded.session_id, "sess-002");
        assert_eq!(loaded.prompt, "Hello world");
    }

    #[test]
    fn load_pre_prompt_state_backward_compat_transcript_offset_migration() {
        let (_dir, backend) = setup();
        let state_file = backend.pre_prompt_path("test-backward-compat");
        fs::create_dir_all(state_file.parent().unwrap()).unwrap();

        // Oldest format migrates to transcript_offset.
        fs::write(
            &state_file,
            r#"{
  "session_id":"test-backward-compat",
  "timestamp":"2026-01-01T00:00:00Z",
  "untracked_files":[],
  "last_transcript_identifier":"user-5",
  "last_transcript_line_count":42
}"#,
        )
        .unwrap();
        let state = backend
            .load_pre_prompt("test-backward-compat")
            .unwrap()
            .unwrap();
        assert_eq!(state.transcript_offset, 42);
        assert_eq!(state.last_transcript_line_count, 0);
        assert_eq!(state.step_transcript_start, 0);
        assert_eq!(state.last_transcript_identifier, "user-5");

        // step_transcript_start takes precedence over oldest field.
        fs::write(
            &state_file,
            r#"{
  "session_id":"test-backward-compat",
  "timestamp":"2026-01-01T00:00:00Z",
  "untracked_files":[],
  "step_transcript_start":100,
  "last_transcript_line_count":42
}"#,
        )
        .unwrap();
        let state = backend
            .load_pre_prompt("test-backward-compat")
            .unwrap()
            .unwrap();
        assert_eq!(state.transcript_offset, 100);

        // start_message_index migrates for Gemini-style format.
        fs::write(
            &state_file,
            r#"{
  "session_id":"test-backward-compat",
  "timestamp":"2026-01-01T00:00:00Z",
  "untracked_files":[],
  "start_message_index":25,
  "last_transcript_identifier":"msg-42"
}"#,
        )
        .unwrap();
        let state = backend
            .load_pre_prompt("test-backward-compat")
            .unwrap()
            .unwrap();
        assert_eq!(state.transcript_offset, 25);
        assert_eq!(state.start_message_index, 0);

        // transcript_offset takes precedence over deprecated fields.
        fs::write(
            &state_file,
            r#"{
  "session_id":"test-backward-compat",
  "timestamp":"2026-01-01T00:00:00Z",
  "untracked_files":[],
  "transcript_offset":200,
  "step_transcript_start":100,
  "start_message_index":50,
  "last_transcript_line_count":42
}"#,
        )
        .unwrap();
        let state = backend
            .load_pre_prompt("test-backward-compat")
            .unwrap()
            .unwrap();
        assert_eq!(state.transcript_offset, 200);
    }

    #[test]
    fn delete_pre_prompt_removes_file() {
        let (_dir, backend) = setup();
        let state = PrePromptState {
            session_id: "sess-003".to_string(),
            prompt: "test".to_string(),
            transcript_path: "/tmp/t.jsonl".to_string(),
            ..Default::default()
        };
        backend.save_pre_prompt(&state).unwrap();
        assert!(backend.load_pre_prompt("sess-003").unwrap().is_some());

        backend.delete_pre_prompt("sess-003").unwrap();
        assert!(backend.load_pre_prompt("sess-003").unwrap().is_none());
    }

    #[test]
    fn create_and_find_active_pre_task() {
        let (_dir, backend) = setup();
        let marker = PreTaskState {
            tool_use_id: "tool-abc".to_string(),
            session_id: "sess-004".to_string(),
            ..Default::default()
        };
        backend.create_pre_task_marker(&marker).unwrap();

        let found = backend.find_active_pre_task().unwrap();
        assert_eq!(found, Some("tool-abc".to_string()));
        assert!(backend.load_pre_task_marker("tool-abc").unwrap().is_some());
    }

    #[test]
    fn delete_pre_task_marker_removes_file() {
        let (_dir, backend) = setup();
        let marker = PreTaskState {
            tool_use_id: "tool-xyz".to_string(),
            session_id: "sess-005".to_string(),
            ..Default::default()
        };
        backend.create_pre_task_marker(&marker).unwrap();
        assert!(backend.load_pre_task_marker("tool-xyz").unwrap().is_some());

        backend.delete_pre_task_marker("tool-xyz").unwrap();
        assert!(backend.load_pre_task_marker("tool-xyz").unwrap().is_none());
    }

    #[test]
    fn pre_task_marker_uses_go_style_filename() {
        let (dir, backend) = setup();
        let marker = PreTaskState {
            tool_use_id: "tool-style".to_string(),
            session_id: "sess-style".to_string(),
            ..Default::default()
        };
        backend.create_pre_task_marker(&marker).unwrap();

        let expected = dir
            .path()
            .join(".bitloops")
            .join("tmp")
            .join("pre-task-tool-style.json");
        assert!(
            expected.exists(),
            "expected marker file at {}",
            expected.display()
        );
    }

    #[test]
    fn pre_task_state_file_path_ends_with_expected_suffix() {
        let (_dir, backend) = setup();
        let got = backend.pre_task_path("toolu_abc123");
        let expected_suffix = std::path::Path::new(crate::utils::paths::BITLOOPS_TMP_DIR)
            .join("pre-task-toolu_abc123.json");
        assert!(got.is_absolute());
        assert!(
            got.to_string_lossy()
                .ends_with(expected_suffix.to_string_lossy().as_ref()),
            "path {} should end with {}",
            got.display(),
            expected_suffix.display()
        );
    }

    #[test]
    fn find_active_pre_task_returns_latest_marker() {
        let (_dir, backend) = setup();
        backend
            .create_pre_task_marker(&PreTaskState {
                tool_use_id: "older".to_string(),
                session_id: "sess".to_string(),
                ..Default::default()
            })
            .unwrap();
        std::thread::sleep(std::time::Duration::from_millis(5));
        backend
            .create_pre_task_marker(&PreTaskState {
                tool_use_id: "newer".to_string(),
                session_id: "sess".to_string(),
                ..Default::default()
            })
            .unwrap();

        let found = backend.find_active_pre_task().unwrap();
        assert_eq!(found.as_deref(), Some("newer"));
    }

    #[test]
    fn find_active_pre_task_returns_none_when_empty() {
        let (_dir, backend) = setup();
        let found = backend.find_active_pre_task().unwrap();
        assert!(found.is_none());
    }

    #[test]
    fn list_sessions_returns_all_saved_sessions() {
        let (_dir, backend) = setup();
        backend.save_session(&sample_state("sess-list-1")).unwrap();
        backend.save_session(&sample_state("sess-list-2")).unwrap();

        let sessions = backend.list_sessions().unwrap();
        assert_eq!(sessions.len(), 2);
    }

    #[test]
    fn list_sessions_normalizes_legacy_and_malformed_phase_values() {
        use std::collections::HashMap;

        let (_dir, backend) = setup();
        let sessions_dir = backend.sessions_dir();
        fs::create_dir_all(&sessions_dir).unwrap();

        let cases = [
            ("sess-legacy", r#""active_committed""#, SessionPhase::Active),
            ("sess-unknown", r#""bogus""#, SessionPhase::Idle),
            ("sess-null", "null", SessionPhase::Idle),
            ("sess-number", "123", SessionPhase::Idle),
            ("sess-object", "{}", SessionPhase::Idle),
        ];

        for (session_id, phase_json, _expected_phase) in cases {
            let state_json = format!(r#"{{"session_id":"{session_id}","phase":{phase_json}}}"#);
            fs::write(sessions_dir.join(format!("{session_id}.json")), state_json).unwrap();
        }

        let sessions = backend.list_sessions().unwrap();
        let phases_by_session: HashMap<String, SessionPhase> = sessions
            .into_iter()
            .map(|state| (state.session_id, state.phase))
            .collect();

        assert_eq!(phases_by_session.len(), cases.len());
        for (session_id, _phase_json, expected_phase) in cases {
            assert_eq!(phases_by_session.get(session_id), Some(&expected_phase));
        }
    }

    #[test]
    fn list_sessions_returns_empty_when_no_dir() {
        let (_dir, backend) = setup();
        // No sessions saved → directory won't exist.
        let sessions = backend.list_sessions().unwrap();
        assert!(sessions.is_empty());
    }

    #[test]
    fn delete_session_removes_saved_state() {
        let (_dir, backend) = setup();
        let state = sample_state("sess-delete");
        backend.save_session(&state).unwrap();
        assert!(backend.load_session("sess-delete").unwrap().is_some());

        backend.delete_session("sess-delete").unwrap();
        assert!(backend.load_session("sess-delete").unwrap().is_none());
    }
}
