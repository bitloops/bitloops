use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use rusqlite::{OptionalExtension, params};
use serde::de::DeserializeOwned;

use super::backend::SessionBackend;
use super::phase::SessionPhase;
use super::state::{PrePromptState, PreTaskState, SessionState};
use crate::devql_config::{resolve_devql_backend_config, resolve_sqlite_db_path};
use crate::engine::db::SqliteConnectionPool;
use crate::engine::paths;
use crate::engine::validation::validators::{validate_session_id, validate_tool_use_id};

pub struct DbSessionBackend {
    repo_id: String,
    sqlite: SqliteConnectionPool,
}

impl DbSessionBackend {
    pub fn new(repo_id: impl Into<String>, sqlite: SqliteConnectionPool) -> Result<Self> {
        sqlite
            .initialise_checkpoint_schema()
            .context("initialising checkpoint schema for DbSessionBackend")?;
        let backend = Self {
            repo_id: repo_id.into(),
            sqlite,
        };
        Ok(backend)
    }

    pub fn from_sqlite_path(repo_id: impl Into<String>, sqlite_path: PathBuf) -> Result<Self> {
        let sqlite = SqliteConnectionPool::connect(sqlite_path)?;
        Self::new(repo_id, sqlite)
    }

    pub fn for_repo_root(repo_root: &Path) -> Result<Self> {
        let sqlite_path = resolve_repo_scoped_sqlite_path(repo_root)?;
        let repo_identity = crate::engine::devql::resolve_repo_identity(repo_root)
            .context("resolving repo identity for DbSessionBackend")?;
        Self::from_sqlite_path(repo_identity.repo_id, sqlite_path)
    }

    #[cfg(test)]
    fn sqlite(&self) -> &SqliteConnectionPool {
        &self.sqlite
    }

    fn map_session_row(row: &rusqlite::Row<'_>) -> Result<SessionState> {
        let step_count_i64: i64 = row.get("step_count").context("reading step_count")?;
        let step_count = u32::try_from(step_count_i64).unwrap_or_default();

        let token_usage_raw: Option<String> =
            row.get("token_usage").context("reading token_usage")?;
        let pending_prompt_raw: Option<String> = row
            .get("pending_prompt_attribution")
            .context("reading pending_prompt_attribution")?;

        let mut state = SessionState {
            session_id: row.get("session_id").context("reading session_id")?,
            cli_version: row.get("cli_version").context("reading cli_version")?,
            base_commit: row.get("base_commit").context("reading base_commit")?,
            attribution_base_commit: row
                .get("attribution_base_commit")
                .context("reading attribution_base_commit")?,
            worktree_path: row.get("worktree_path").context("reading worktree_path")?,
            worktree_id: row.get("worktree_id").context("reading worktree_id")?,
            started_at: row.get("started_at").unwrap_or_default(),
            ended_at: None,
            phase: SessionPhase::from_string(
                &row.get::<_, String>("phase").context("reading phase")?,
            ),
            turn_id: row.get("turn_id").context("reading turn_id")?,
            turn_checkpoint_ids: parse_json_column(
                &row.get::<_, String>("turn_checkpoint_ids")
                    .context("reading turn_checkpoint_ids")?,
                "turn_checkpoint_ids",
            )?,
            last_interaction_time: row.get("last_interaction_time").unwrap_or(None),
            step_count,
            checkpoint_transcript_start: row
                .get("checkpoint_transcript_start")
                .context("reading checkpoint_transcript_start")?,
            condensed_transcript_lines: 0,
            transcript_lines_at_start: 0,
            untracked_files_at_start: parse_json_column(
                &row.get::<_, String>("untracked_files_at_start")
                    .context("reading untracked_files_at_start")?,
                "untracked_files_at_start",
            )?,
            files_touched: parse_json_column(
                &row.get::<_, String>("files_touched")
                    .context("reading files_touched")?,
                "files_touched",
            )?,
            transcript_path: row
                .get("transcript_path")
                .context("reading transcript_path")?,
            first_prompt: row.get("first_prompt").context("reading first_prompt")?,
            agent_type: row.get("agent_type").context("reading agent_type")?,
            last_checkpoint_id: row
                .get("last_checkpoint_id")
                .context("reading last_checkpoint_id")?,
            token_usage: parse_optional_json_column(token_usage_raw.as_deref(), "token_usage")?,
            transcript_identifier_at_start: row
                .get("transcript_identifier_at_start")
                .context("reading transcript_identifier_at_start")?,
            prompt_attributions: parse_json_column(
                &row.get::<_, String>("prompt_attributions")
                    .context("reading prompt_attributions")?,
                "prompt_attributions",
            )?,
            pending_prompt_attribution: parse_optional_json_column(
                pending_prompt_raw.as_deref(),
                "pending_prompt_attribution",
            )?,
        };
        state.normalize_after_load();
        Ok(state)
    }
}

impl SessionBackend for DbSessionBackend {
    fn list_sessions(&self) -> Result<Vec<SessionState>> {
        self.sqlite.with_connection(|conn| {
            let mut stmt = conn.prepare(
                "SELECT session_id, cli_version, base_commit, attribution_base_commit, worktree_path,
                        worktree_id, started_at, phase, turn_id, step_count, checkpoint_transcript_start,
                        transcript_path, first_prompt, agent_type, last_checkpoint_id,
                        last_interaction_time, files_touched, untracked_files_at_start,
                        turn_checkpoint_ids, transcript_identifier_at_start, token_usage,
                        prompt_attributions, pending_prompt_attribution
                 FROM sessions
                 WHERE repo_id = ?1
                 ORDER BY updated_at DESC, created_at DESC",
            )
            .context("preparing session list query")?;
            let mut rows = stmt
                .query(params![self.repo_id.as_str()])
                .context("executing session list query")?;

            let mut sessions = Vec::new();
            while let Some(row) = rows.next().context("iterating session rows")? {
                if let Ok(state) = Self::map_session_row(row) {
                    sessions.push(state);
                }
            }

            Ok(sessions)
        })
    }

    fn load_session(&self, session_id: &str) -> Result<Option<SessionState>> {
        validate_session_id(session_id)?;
        self.sqlite.with_connection(|conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT session_id, cli_version, base_commit, attribution_base_commit, worktree_path,
                            worktree_id, started_at, phase, turn_id, step_count, checkpoint_transcript_start,
                            transcript_path, first_prompt, agent_type, last_checkpoint_id,
                            last_interaction_time, files_touched, untracked_files_at_start,
                            turn_checkpoint_ids, transcript_identifier_at_start, token_usage,
                            prompt_attributions, pending_prompt_attribution
                     FROM sessions
                     WHERE session_id = ?1 AND repo_id = ?2
                     LIMIT 1",
                )
                .context("preparing session load query")?;
            let mut rows = stmt
                .query(params![session_id, self.repo_id.as_str()])
                .context("executing session load query")?;
            let Some(row) = rows.next().context("reading session load row")? else {
                return Ok(None);
            };
            Ok(Some(Self::map_session_row(row)?))
        })
    }

    fn save_session(&self, state: &SessionState) -> Result<()> {
        validate_session_id(&state.session_id)?;
        let files_touched = serde_json::to_string(&state.files_touched)
            .context("serialising files_touched for session row")?;
        let untracked_files_at_start = serde_json::to_string(&state.untracked_files_at_start)
            .context("serialising untracked_files_at_start for session row")?;
        let turn_checkpoint_ids = serde_json::to_string(&state.turn_checkpoint_ids)
            .context("serialising turn_checkpoint_ids for session row")?;
        let token_usage = serde_json::to_string(&state.token_usage)
            .context("serialising token_usage for session row")?;
        let prompt_attributions = serde_json::to_string(&state.prompt_attributions)
            .context("serialising prompt_attributions for session row")?;
        let pending_prompt_attribution =
            serde_json::to_string(&state.pending_prompt_attribution)
                .context("serialising pending_prompt_attribution for session row")?;

        self.sqlite.with_connection(|conn| {
            conn.execute(
                "INSERT INTO sessions (
                    session_id, repo_id, cli_version, base_commit, attribution_base_commit,
                    worktree_path, worktree_id, started_at, phase, turn_id, step_count,
                    checkpoint_transcript_start, transcript_path, first_prompt, agent_type,
                    last_checkpoint_id, last_interaction_time, files_touched,
                    untracked_files_at_start, turn_checkpoint_ids, transcript_identifier_at_start,
                    token_usage, prompt_attributions, pending_prompt_attribution, updated_at
                 )
                 VALUES (
                    ?1, ?2, ?3, ?4, ?5,
                    ?6, ?7, ?8, ?9, ?10, ?11,
                    ?12, ?13, ?14, ?15,
                    ?16, ?17, ?18,
                    ?19, ?20, ?21,
                    ?22, ?23, ?24, datetime('now')
                 )
                 ON CONFLICT(session_id) DO UPDATE SET
                    repo_id = excluded.repo_id,
                    cli_version = excluded.cli_version,
                    base_commit = excluded.base_commit,
                    attribution_base_commit = excluded.attribution_base_commit,
                    worktree_path = excluded.worktree_path,
                    worktree_id = excluded.worktree_id,
                    started_at = excluded.started_at,
                    phase = excluded.phase,
                    turn_id = excluded.turn_id,
                    step_count = excluded.step_count,
                    checkpoint_transcript_start = excluded.checkpoint_transcript_start,
                    transcript_path = excluded.transcript_path,
                    first_prompt = excluded.first_prompt,
                    agent_type = excluded.agent_type,
                    last_checkpoint_id = excluded.last_checkpoint_id,
                    last_interaction_time = excluded.last_interaction_time,
                    files_touched = excluded.files_touched,
                    untracked_files_at_start = excluded.untracked_files_at_start,
                    turn_checkpoint_ids = excluded.turn_checkpoint_ids,
                    transcript_identifier_at_start = excluded.transcript_identifier_at_start,
                    token_usage = excluded.token_usage,
                    prompt_attributions = excluded.prompt_attributions,
                    pending_prompt_attribution = excluded.pending_prompt_attribution,
                    updated_at = datetime('now')",
                params![
                    state.session_id.as_str(),
                    self.repo_id.as_str(),
                    state.cli_version.as_str(),
                    state.base_commit.as_str(),
                    state.attribution_base_commit.as_str(),
                    state.worktree_path.as_str(),
                    state.worktree_id.as_str(),
                    empty_as_none(&state.started_at),
                    state.phase.as_str(),
                    state.turn_id.as_str(),
                    i64::from(state.step_count),
                    state.checkpoint_transcript_start,
                    state.transcript_path.as_str(),
                    state.first_prompt.as_str(),
                    state.agent_type.as_str(),
                    state.last_checkpoint_id.as_str(),
                    state.last_interaction_time.as_deref(),
                    files_touched,
                    untracked_files_at_start,
                    turn_checkpoint_ids,
                    state.transcript_identifier_at_start.as_str(),
                    token_usage,
                    prompt_attributions,
                    pending_prompt_attribution
                ],
            )
            .context("upserting session state row")?;
            Ok(())
        })
    }

    fn delete_session(&self, session_id: &str) -> Result<()> {
        validate_session_id(session_id)?;
        self.sqlite.with_connection(|conn| {
            conn.execute(
                "DELETE FROM sessions WHERE session_id = ?1 AND repo_id = ?2",
                params![session_id, self.repo_id.as_str()],
            )
            .context("deleting session state row")?;
            Ok(())
        })
    }

    fn load_pre_prompt(&self, session_id: &str) -> Result<Option<PrePromptState>> {
        validate_session_id(session_id)?;
        self.sqlite.with_connection(|conn| {
            let data: Option<String> = conn
                .query_row(
                    "SELECT data FROM pre_prompt_states WHERE session_id = ?1 AND repo_id = ?2 LIMIT 1",
                    params![session_id, self.repo_id.as_str()],
                    |row| row.get::<_, String>(0),
                )
                .optional()
                .context("querying pre-prompt row")?;

            let Some(data) = data else {
                return Ok(None);
            };

            let mut state: PrePromptState =
                serde_json::from_str(&data).context("deserialising pre-prompt state")?;
            state.normalize_after_load();
            Ok(Some(state))
        })
    }

    fn save_pre_prompt(&self, state: &PrePromptState) -> Result<()> {
        validate_session_id(&state.session_id)?;
        let data = serde_json::to_string(state).context("serialising pre-prompt state")?;
        self.sqlite.with_connection(|conn| {
            conn.execute(
                "INSERT INTO pre_prompt_states (session_id, repo_id, data, created_at)
                 VALUES (?1, ?2, ?3, datetime('now'))
                 ON CONFLICT(session_id) DO UPDATE SET
                    repo_id = excluded.repo_id,
                    data = excluded.data,
                    created_at = datetime('now')",
                params![state.session_id.as_str(), self.repo_id.as_str(), data],
            )
            .context("upserting pre-prompt row")?;
            Ok(())
        })
    }

    fn delete_pre_prompt(&self, session_id: &str) -> Result<()> {
        validate_session_id(session_id)?;
        self.sqlite.with_connection(|conn| {
            conn.execute(
                "DELETE FROM pre_prompt_states WHERE session_id = ?1 AND repo_id = ?2",
                params![session_id, self.repo_id.as_str()],
            )
            .context("deleting pre-prompt row")?;
            Ok(())
        })
    }

    fn create_pre_task_marker(&self, state: &PreTaskState) -> Result<()> {
        validate_tool_use_id(&state.tool_use_id)?;
        let data = serde_json::to_string(state).context("serialising pre-task state")?;
        self.sqlite.with_connection(|conn| {
            conn.execute(
                "INSERT INTO pre_task_markers (tool_use_id, session_id, repo_id, data, created_at)
                 VALUES (?1, ?2, ?3, ?4, datetime('now'))
                 ON CONFLICT(tool_use_id) DO UPDATE SET
                    session_id = excluded.session_id,
                    repo_id = excluded.repo_id,
                    data = excluded.data,
                    created_at = datetime('now')",
                params![
                    state.tool_use_id.as_str(),
                    state.session_id.as_str(),
                    self.repo_id.as_str(),
                    data
                ],
            )
            .context("upserting pre-task marker row")?;
            Ok(())
        })
    }

    fn load_pre_task_marker(&self, tool_use_id: &str) -> Result<Option<PreTaskState>> {
        validate_tool_use_id(tool_use_id)?;
        self.sqlite.with_connection(|conn| {
            let data: Option<String> = conn
                .query_row(
                    "SELECT data FROM pre_task_markers WHERE tool_use_id = ?1 AND repo_id = ?2 LIMIT 1",
                    params![tool_use_id, self.repo_id.as_str()],
                    |row| row.get::<_, String>(0),
                )
                .optional()
                .context("querying pre-task row")?;

            let Some(data) = data else {
                return Ok(None);
            };

            let state: PreTaskState =
                serde_json::from_str(&data).context("deserialising pre-task state")?;
            Ok(Some(state))
        })
    }

    fn delete_pre_task_marker(&self, tool_use_id: &str) -> Result<()> {
        validate_tool_use_id(tool_use_id)?;
        self.sqlite.with_connection(|conn| {
            conn.execute(
                "DELETE FROM pre_task_markers WHERE tool_use_id = ?1 AND repo_id = ?2",
                params![tool_use_id, self.repo_id.as_str()],
            )
            .context("deleting pre-task marker row")?;
            Ok(())
        })
    }

    fn find_active_pre_task(&self) -> Result<Option<String>> {
        self.sqlite.with_connection(|conn| {
            conn.query_row(
                "SELECT tool_use_id
                 FROM pre_task_markers
                 WHERE repo_id = ?1
                 ORDER BY created_at DESC, rowid DESC
                 LIMIT 1",
                params![self.repo_id.as_str()],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .context("querying latest pre-task marker")
        })
    }
}

fn resolve_repo_scoped_sqlite_path(repo_root: &Path) -> Result<PathBuf> {
    let cfg =
        resolve_devql_backend_config().context("resolving DevQL backend config for session DB")?;
    if let Some(path) = cfg.relational.sqlite_path.as_deref() {
        return resolve_sqlite_db_path(Some(path))
            .context("resolving configured SQLite path for session DB");
    }

    Ok(repo_root
        .join(paths::BITLOOPS_DIR)
        .join("devql")
        .join("relational.db"))
}

fn parse_json_column<T: DeserializeOwned>(raw: &str, field: &str) -> Result<T> {
    serde_json::from_str(raw).with_context(|| format!("deserialising JSON column `{field}`"))
}

fn parse_optional_json_column<T: DeserializeOwned>(
    raw: Option<&str>,
    field: &str,
) -> Result<Option<T>> {
    let Some(raw) = raw else {
        return Ok(None);
    };
    if raw.trim().is_empty() || raw.trim() == "null" {
        return Ok(None);
    }

    let parsed = serde_json::from_str(raw)
        .with_context(|| format!("deserialising JSON column `{field}`"))?;
    Ok(Some(parsed))
}

fn empty_as_none(value: &str) -> Option<&str> {
    if value.trim().is_empty() {
        None
    } else {
        Some(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::session::phase::SessionPhase;
    use tempfile::TempDir;

    fn setup(repo_id: &str) -> (TempDir, DbSessionBackend) {
        let dir = tempfile::tempdir().unwrap();
        let sqlite_path = dir.path().join("relational.sqlite");
        let backend = DbSessionBackend::from_sqlite_path(repo_id.to_string(), sqlite_path).unwrap();
        (dir, backend)
    }

    fn sample_session(session_id: &str) -> SessionState {
        SessionState {
            session_id: session_id.to_string(),
            phase: SessionPhase::Active,
            transcript_path: "/tmp/t.jsonl".to_string(),
            first_prompt: "Fix bug".to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn save_and_load_session_roundtrip() {
        let (_dir, backend) = setup("repo-a");
        let state = sample_session("sess-1");
        backend.save_session(&state).unwrap();

        let loaded = backend.load_session("sess-1").unwrap().unwrap();
        assert_eq!(loaded.session_id, "sess-1");
        assert_eq!(loaded.phase, SessionPhase::Active);
        assert_eq!(loaded.first_prompt, "Fix bug");
    }

    #[test]
    fn load_session_normalizes_legacy_phase_values() {
        let (_dir, backend) = setup("repo-a");
        backend
            .sqlite()
            .with_connection(|conn| {
                conn.execute(
                    "INSERT INTO sessions (session_id, repo_id, phase) VALUES (?1, ?2, ?3)",
                    ("sess-legacy-phase", "repo-a", "active_committed"),
                )?;
                Ok(())
            })
            .unwrap();

        let loaded = backend.load_session("sess-legacy-phase").unwrap().unwrap();
        assert_eq!(loaded.phase, SessionPhase::Active);
    }

    #[test]
    fn list_sessions_is_repo_scoped_and_skips_bad_json() {
        let (_dir, backend_a) = setup("repo-a");
        let backend_b = DbSessionBackend::new("repo-b", backend_a.sqlite().clone()).unwrap();

        backend_a
            .save_session(&SessionState {
                session_id: "sess-a".to_string(),
                ..sample_session("sess-a")
            })
            .unwrap();
        backend_b
            .save_session(&SessionState {
                session_id: "sess-b".to_string(),
                ..sample_session("sess-b")
            })
            .unwrap();

        backend_a
            .sqlite()
            .with_connection(|conn| {
                conn.execute(
                    "INSERT INTO sessions (session_id, repo_id, phase, files_touched) VALUES (?1, ?2, ?3, ?4)",
                    ("bad-json", "repo-a", "active", "{broken"),
                )?;
                Ok(())
            })
            .unwrap();

        let sessions = backend_a.list_sessions().unwrap();
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].session_id, "sess-a");
    }

    #[test]
    fn load_pre_prompt_normalizes_legacy_transcript_fields() {
        let (_dir, backend) = setup("repo-pre");
        backend
            .sqlite()
            .with_connection(|conn| {
                conn.execute(
                    "INSERT INTO pre_prompt_states (session_id, repo_id, data) VALUES (?1, ?2, ?3)",
                    (
                        "sess-pre-legacy",
                        "repo-pre",
                        r#"{"session_id":"sess-pre-legacy","last_transcript_line_count":42}"#,
                    ),
                )?;
                Ok(())
            })
            .unwrap();

        let loaded = backend.load_pre_prompt("sess-pre-legacy").unwrap().unwrap();
        assert_eq!(loaded.transcript_offset, 42);
        assert_eq!(loaded.last_transcript_line_count, 0);
    }

    #[test]
    fn pre_prompt_roundtrip_and_delete() {
        let (_dir, backend) = setup("repo-pre");
        let pre = PrePromptState {
            session_id: "sess-pre".to_string(),
            prompt: "hello".to_string(),
            ..Default::default()
        };

        backend.save_pre_prompt(&pre).unwrap();
        let loaded = backend.load_pre_prompt("sess-pre").unwrap().unwrap();
        assert_eq!(loaded.prompt, "hello");

        backend.delete_pre_prompt("sess-pre").unwrap();
        assert!(backend.load_pre_prompt("sess-pre").unwrap().is_none());
    }

    #[test]
    fn pre_task_marker_roundtrip_and_find_active() {
        let (_dir, backend) = setup("repo-task");
        backend
            .create_pre_task_marker(&PreTaskState {
                tool_use_id: "tool-old".to_string(),
                session_id: "sess-task".to_string(),
                ..Default::default()
            })
            .unwrap();
        backend
            .create_pre_task_marker(&PreTaskState {
                tool_use_id: "tool-new".to_string(),
                session_id: "sess-task".to_string(),
                ..Default::default()
            })
            .unwrap();

        let active = backend.find_active_pre_task().unwrap();
        assert_eq!(active.as_deref(), Some("tool-new"));

        let loaded = backend.load_pre_task_marker("tool-new").unwrap().unwrap();
        assert_eq!(loaded.session_id, "sess-task");

        backend.delete_pre_task_marker("tool-new").unwrap();
        assert!(backend.load_pre_task_marker("tool-new").unwrap().is_none());
    }

    #[test]
    fn delete_session_removes_row() {
        let (_dir, backend) = setup("repo-delete");
        let session = sample_session("sess-delete");
        backend.save_session(&session).unwrap();
        assert!(backend.load_session("sess-delete").unwrap().is_some());

        backend.delete_session("sess-delete").unwrap();
        assert!(backend.load_session("sess-delete").unwrap().is_none());

        // idempotent delete (no-op when missing)
        backend.delete_session("sess-delete").unwrap();
    }
}
