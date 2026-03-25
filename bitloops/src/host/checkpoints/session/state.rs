//! Session state structs persisted to `.git/bitloops-sessions/<id>.json`.

use crate::adapters::agents::TokenUsage;
use serde::{Deserialize, Serialize};

use super::phase::SessionPhase;

pub const PRE_PROMPT_SOURCE_CURSOR_SHELL: &str = "cursor-shell";

fn is_zero_i64(value: &i64) -> bool {
    *value == 0
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct PromptAttribution {
    #[serde(default)]
    pub checkpoint_number: i32,
    #[serde(default)]
    pub user_lines_added: i32,
    #[serde(default)]
    pub user_lines_removed: i32,
    #[serde(default)]
    pub agent_lines_added: i32,
    #[serde(default)]
    pub agent_lines_removed: i32,
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub user_added_per_file: std::collections::BTreeMap<String, i32>,
}

/// Full session state.
///
/// Fields not yet actively used are included for forward compatibility with
/// the ManualCommit strategy implementation (shadow branches, attributions).
///
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SessionState {
    /// Unique session identifier from Claude Code.
    pub session_id: String,

    /// Version of the CLI that created this session.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub cli_version: String,

    /// Current shadow branch base commit hash.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub base_commit: String,

    /// Absolute path to the worktree root.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub worktree_path: String,

    /// Git worktree identifier (empty for main worktree).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub worktree_id: String,

    /// When the session started (RFC 3339).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub started_at: String,

    /// When the session closed (RFC 3339). None if still active.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ended_at: Option<String>,

    /// Current lifecycle stage.
    #[serde(default)]
    pub phase: SessionPhase,

    /// Unique identifier for the current agent turn.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub turn_id: String,

    /// Checkpoint IDs created during the current turn (for HandleTurnEnd finalization).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub turn_checkpoint_ids: Vec<String>,

    /// Updated on every hook invocation (RFC 3339).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_interaction_time: Option<String>,

    /// Number of checkpoints/steps created in this session.
    #[serde(default, rename = "checkpoint_count")]
    pub step_count: u32,

    /// Transcript line offset where the current checkpoint cycle began.
    #[serde(default)]
    pub checkpoint_transcript_start: i64,

    #[serde(default, skip_serializing_if = "is_zero_i64")]
    pub condensed_transcript_lines: i64,

    #[serde(default, skip_serializing_if = "is_zero_i64")]
    pub transcript_lines_at_start: i64,

    /// Files that existed (untracked) when the session started.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub untracked_files_at_start: Vec<String>,

    /// Files modified, created, or deleted during this session.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub files_touched: Vec<String>,

    /// Path to the live Claude Code transcript file.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub transcript_path: String,

    /// First user prompt (truncated to 100 chars for display).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub first_prompt: String,

    /// Agent type (e.g., "claude-code").
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub agent_type: String,

    /// Checkpoint ID from the most recent condensation.
    /// Used by `prepare-commit-msg` on `git commit --amend`.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub last_checkpoint_id: String,

    /// Accumulated token usage for pending checkpoints in this session.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_usage: Option<TokenUsage>,

    /// Base commit at the start of the current attribution window.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub attribution_base_commit: String,

    /// Transcript identifier recorded at the first step of the current checkpoint cycle
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub transcript_identifier_at_start: String,

    /// User/agent attribution snapshots captured at each prompt start.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub prompt_attributions: Vec<PromptAttribution>,

    /// Attribution captured at prompt start and consumed on the next checkpoint save.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending_prompt_attribution: Option<PromptAttribution>,
}

/// State captured at `user-prompt-submit` time; consumed by `stop`.
///
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PrePromptState {
    pub session_id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub timestamp: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub source: String,
    /// User prompt, truncated to 100 chars.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub prompt: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub transcript_path: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub untracked_files: Vec<String>,
    #[serde(default)]
    pub transcript_offset: i64,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub last_transcript_identifier: String,
    #[serde(default, skip_serializing_if = "is_zero_i64")]
    pub start_message_index: i64,
    #[serde(default, skip_serializing_if = "is_zero_i64")]
    pub step_transcript_start: i64,
    #[serde(default, skip_serializing_if = "is_zero_i64")]
    pub last_transcript_line_count: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub devql_prefetch: Option<crate::host::checkpoints::history::devql_prefetch::PrefetchResult>,
}

/// Marker written by `pre-task`; checked by `post-todo` and `post-task`.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PreTaskState {
    pub tool_use_id: String,
    pub session_id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub timestamp: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub untracked_files: Vec<String>,
}

impl SessionState {
    pub fn normalize_after_load(&mut self) {
        self.phase = SessionPhase::from_string(self.phase.as_str());

        if self.checkpoint_transcript_start == 0 {
            if self.condensed_transcript_lines > 0 {
                self.checkpoint_transcript_start = self.condensed_transcript_lines;
            } else if self.transcript_lines_at_start > 0 {
                self.checkpoint_transcript_start = self.transcript_lines_at_start;
            }
        }

        self.condensed_transcript_lines = 0;
        self.transcript_lines_at_start = 0;

        if self.attribution_base_commit.is_empty() && !self.base_commit.is_empty() {
            self.attribution_base_commit = self.base_commit.clone();
        }
    }
}

impl PrePromptState {
    pub fn normalize_after_load(&mut self) {
        if self.step_transcript_start == 0 && self.last_transcript_line_count > 0 {
            self.step_transcript_start = self.last_transcript_line_count;
        }
        self.last_transcript_line_count = 0;

        if self.transcript_offset == 0 {
            if self.step_transcript_start > 0 {
                self.transcript_offset = self.step_transcript_start;
            } else if self.start_message_index > 0 {
                self.transcript_offset = self.start_message_index;
            }
        }

        self.step_transcript_start = 0;
        self.start_message_index = 0;
    }
}

/// Returns the most recent session, preferring an exact worktree match.
///
pub fn find_most_recent_session(
    sessions: &[SessionState],
    worktree_path: &str,
) -> Option<SessionState> {
    fn score(state: &SessionState) -> (&str, &str) {
        (
            state.last_interaction_time.as_deref().unwrap_or(""),
            state.started_at.as_str(),
        )
    }

    let matching: Vec<&SessionState> = sessions
        .iter()
        .filter(|s| !worktree_path.is_empty() && s.worktree_path == worktree_path)
        .collect();

    let pick_from = if matching.is_empty() {
        sessions.iter().collect()
    } else {
        matching
    };

    pick_from
        .into_iter()
        .max_by(|a, b| score(a).cmp(&score(b)))
        .cloned()
}

#[cfg(test)]
mod tests {
    use super::{PrePromptState, SessionState, find_most_recent_session};

    // CLI-257
    #[test]
    fn test_state_normalize_after_load() {
        let mut state = SessionState {
            condensed_transcript_lines: 150,
            ..SessionState::default()
        };
        state.normalize_after_load();
        assert_eq!(state.checkpoint_transcript_start, 150);
        assert_eq!(state.condensed_transcript_lines, 0);
        assert_eq!(state.transcript_lines_at_start, 0);

        let mut state = SessionState {
            checkpoint_transcript_start: 200,
            condensed_transcript_lines: 150,
            ..SessionState::default()
        };
        state.normalize_after_load();
        assert_eq!(state.checkpoint_transcript_start, 200);
        assert_eq!(state.condensed_transcript_lines, 0);

        let mut state = SessionState::default();
        state.normalize_after_load();
        assert_eq!(state.checkpoint_transcript_start, 0);

        let mut state = SessionState {
            transcript_lines_at_start: 42,
            ..SessionState::default()
        };
        state.normalize_after_load();
        assert_eq!(state.checkpoint_transcript_start, 42);
        assert_eq!(state.transcript_lines_at_start, 0);

        let mut state = SessionState {
            condensed_transcript_lines: 150,
            transcript_lines_at_start: 42,
            ..SessionState::default()
        };
        state.normalize_after_load();
        assert_eq!(state.checkpoint_transcript_start, 150);
        assert_eq!(state.condensed_transcript_lines, 0);
        assert_eq!(state.transcript_lines_at_start, 0);

        let mut state = SessionState {
            checkpoint_transcript_start: 200,
            transcript_lines_at_start: 42,
            ..SessionState::default()
        };
        state.normalize_after_load();
        assert_eq!(state.checkpoint_transcript_start, 200);
        assert_eq!(state.transcript_lines_at_start, 0);
    }

    // CLI-258
    #[test]
    fn test_state_normalize_after_load_json_round_trip() {
        let cases = [
            (
                "migrates old condensed_transcript_lines",
                r#"{"session_id":"s1","condensed_transcript_lines":42,"checkpoint_count":5}"#,
                42_i64,
                5_u32,
            ),
            (
                "migrates old transcript_lines_at_start",
                r#"{"session_id":"s1","transcript_lines_at_start":75}"#,
                75_i64,
                0_u32,
            ),
            (
                "preserves new field over old",
                r#"{"session_id":"s1","condensed_transcript_lines":10,"checkpoint_transcript_start":50}"#,
                50_i64,
                0_u32,
            ),
            (
                "handles clean new format",
                r#"{"session_id":"s1","checkpoint_transcript_start":25,"checkpoint_count":3}"#,
                25_i64,
                3_u32,
            ),
        ];

        for (case_name, json_text, expected_cts, expected_step_count) in cases {
            let mut state: SessionState =
                serde_json::from_str(json_text).expect("json should deserialize");
            state.normalize_after_load();

            assert_eq!(
                state.checkpoint_transcript_start, expected_cts,
                "{case_name}"
            );
            assert_eq!(state.step_count, expected_step_count, "{case_name}");
            assert_eq!(state.condensed_transcript_lines, 0, "{case_name}");
            assert_eq!(state.transcript_lines_at_start, 0, "{case_name}");
        }
    }

    #[test]
    fn test_pre_prompt_state_normalize_after_load() {
        let mut state = PrePromptState {
            last_transcript_line_count: 42,
            ..PrePromptState::default()
        };
        state.normalize_after_load();
        assert_eq!(state.transcript_offset, 42);
        assert_eq!(state.last_transcript_line_count, 0);
        assert_eq!(state.step_transcript_start, 0);
        assert_eq!(state.start_message_index, 0);

        let mut state = PrePromptState {
            step_transcript_start: 100,
            last_transcript_line_count: 42,
            ..PrePromptState::default()
        };
        state.normalize_after_load();
        assert_eq!(state.transcript_offset, 100);

        let mut state = PrePromptState {
            start_message_index: 25,
            ..PrePromptState::default()
        };
        state.normalize_after_load();
        assert_eq!(state.transcript_offset, 25);

        let mut state = PrePromptState {
            transcript_offset: 200,
            step_transcript_start: 100,
            start_message_index: 50,
            last_transcript_line_count: 42,
            ..PrePromptState::default()
        };
        state.normalize_after_load();
        assert_eq!(state.transcript_offset, 200);
        assert_eq!(state.step_transcript_start, 0);
        assert_eq!(state.start_message_index, 0);
        assert_eq!(state.last_transcript_line_count, 0);
    }

    #[test]
    fn session_state_json_round_trip_preserves_fields() {
        let original = SessionState {
            session_id: "s-json".to_string(),
            cli_version: "0.0.3".to_string(),
            base_commit: "abc123".to_string(),
            started_at: "2026-01-01T00:00:00Z".to_string(),
            last_checkpoint_id: "a1b2c3d4e5f6".to_string(),
            turn_checkpoint_ids: vec!["c1".to_string(), "c2".to_string()],
            ..Default::default()
        };

        let json = serde_json::to_string(&original).expect("serialize");
        let mut restored: SessionState = serde_json::from_str(&json).expect("deserialize");
        restored.normalize_after_load();

        assert_eq!(restored.session_id, "s-json");
        assert_eq!(restored.cli_version, "0.0.3");
        assert_eq!(restored.base_commit, "abc123");
        assert_eq!(restored.last_checkpoint_id, "a1b2c3d4e5f6");
        assert_eq!(restored.turn_checkpoint_ids, vec!["c1", "c2"]);
    }

    #[test]
    fn session_state_last_checkpoint_id_roundtrip() {
        let state = SessionState {
            session_id: "s-last".to_string(),
            last_checkpoint_id: "deadbeefcafe".to_string(),
            ..Default::default()
        };

        let json = serde_json::to_string(&state).expect("serialize");
        let restored: SessionState = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(restored.last_checkpoint_id, "deadbeefcafe");
    }

    #[test]
    fn find_most_recent_session_filters_by_worktree() {
        let sessions = vec![
            SessionState {
                session_id: "s-main".to_string(),
                worktree_path: "/repo/main".to_string(),
                last_interaction_time: Some("100".to_string()),
                ..Default::default()
            },
            SessionState {
                session_id: "s-wt-old".to_string(),
                worktree_path: "/repo/wt".to_string(),
                last_interaction_time: Some("200".to_string()),
                ..Default::default()
            },
            SessionState {
                session_id: "s-wt-new".to_string(),
                worktree_path: "/repo/wt".to_string(),
                last_interaction_time: Some("300".to_string()),
                ..Default::default()
            },
        ];

        let found = find_most_recent_session(&sessions, "/repo/wt").expect("session");
        assert_eq!(found.session_id, "s-wt-new");
    }

    #[test]
    fn find_most_recent_session_falls_back_when_no_worktree_match() {
        let sessions = vec![
            SessionState {
                session_id: "s-1".to_string(),
                worktree_path: "/repo/other".to_string(),
                last_interaction_time: Some("100".to_string()),
                ..Default::default()
            },
            SessionState {
                session_id: "s-2".to_string(),
                worktree_path: "/repo/other2".to_string(),
                last_interaction_time: Some("200".to_string()),
                ..Default::default()
            },
        ];

        let found = find_most_recent_session(&sessions, "/repo/wt").expect("session");
        assert_eq!(found.session_id, "s-2");
    }
}
