//! Per-repo telemetry session management.
//!
//! Sessions are daemon-driven with 60-minute idle timeout. Each repo has its own
//! active session. When a session expires, a new one is created on the next event.

use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

/// Session timeout: 60 minutes of inactivity
const SESSION_TIMEOUT: Duration = Duration::from_secs(60 * 60);

/// Session state file name in daemon state dir
const SESSIONS_FILE: &str = "telemetry_sessions.json";

#[derive(Debug, Clone)]
pub struct SessionResult {
    pub session_id: String,
    pub started_at: u64,
    pub is_new_session: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoSession {
    /// Unique session ID (UUID v4 format)
    pub session_id: String,
    /// Unix timestamp when session started
    pub started_at: u64,
    /// Unix timestamp of last event in this session
    pub last_event_at: u64,
}

impl RepoSession {
    pub fn new() -> Self {
        let now = now_secs();
        Self {
            session_id: uuid_v4(),
            started_at: now,
            last_event_at: now,
        }
    }

    pub fn is_expired(&self) -> bool {
        now_secs().saturating_sub(self.last_event_at) > SESSION_TIMEOUT.as_secs()
    }

    pub fn touch(&mut self) {
        self.last_event_at = now_secs();
    }

    pub fn session_duration_secs(&self) -> u64 {
        now_secs().saturating_sub(self.started_at)
    }
}

impl Default for RepoSession {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SessionStore {
    /// Map of repo_root_path -> active session
    sessions: HashMap<String, RepoSession>,
}

impl SessionStore {
    pub fn load(state_dir: &Path) -> Self {
        let path = state_dir.join(SESSIONS_FILE);
        match fs::read_to_string(&path) {
            Ok(content) => serde_json::from_str(&content).unwrap_or_default(),
            Err(_) => Self::default(),
        }
    }

    /// Load store and return any sessions that have expired since last check
    pub fn load_with_expired(state_dir: &Path) -> (Self, Vec<EndedSession>) {
        let path = state_dir.join(SESSIONS_FILE);
        let mut store: Self = match fs::read_to_string(&path) {
            Ok(content) => serde_json::from_str(&content).unwrap_or_default(),
            Err(_) => Self::default(),
        };

        let expired: Vec<EndedSession> = store
            .sessions
            .iter()
            .filter(|(_, s)| s.is_expired())
            .map(|(key, s)| {
                let duration = s.session_duration_secs();
                EndedSession {
                    session_id: s.session_id.clone(),
                    repo_root: key.clone(),
                    started_at: s.started_at,
                    ended_at: now_secs(),
                    duration_secs: duration,
                }
            })
            .collect();

        // Remove expired sessions
        for ended in &expired {
            store.sessions.remove(&ended.repo_root);
        }

        (store, expired)
    }

    pub fn save(&self, state_dir: &Path) -> std::io::Result<()> {
        let path = state_dir.join(SESSIONS_FILE);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let content = serde_json::to_string(self).unwrap_or_default();
        fs::write(path, content)
    }

    pub fn get_or_create_session(&mut self, repo_root: &Path) -> SessionResult {
        let key = repo_root.to_string_lossy().to_string();

        // Check if we have an existing non-expired session
        let has_valid_session = self.sessions.get(&key).is_some_and(|s| !s.is_expired());

        let is_new_session = !has_valid_session;

        if !has_valid_session {
            let new_session = RepoSession::new();
            self.sessions.insert(key.clone(), new_session);
        }

        // Touch the session to update last_event_at
        if let Some(session) = self.sessions.get_mut(&key) {
            session.touch();
        }

        let session = self.sessions.get(&key).unwrap();
        SessionResult {
            session_id: session.session_id.clone(),
            started_at: session.started_at,
            is_new_session,
        }
    }

    pub fn get_session_id(&mut self, repo_root: &Path) -> Option<String> {
        self.get_or_create_session(repo_root)
            .session_id
            .clone()
            .into()
    }

    pub fn end_session(&mut self, repo_root: &Path) -> Option<EndedSession> {
        let key = repo_root.to_string_lossy().to_string();
        self.sessions.remove(&key).map(|s| {
            let duration = s.session_duration_secs();
            EndedSession {
                session_id: s.session_id,
                repo_root: key,
                started_at: s.started_at,
                ended_at: now_secs(),
                duration_secs: duration,
            }
        })
    }

    pub fn sessions(&self) -> impl Iterator<Item = (&String, &RepoSession)> {
        self.sessions.iter()
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct EndedSession {
    pub session_id: String,
    pub repo_root: String,
    pub started_at: u64,
    pub ended_at: u64,
    pub duration_secs: u64,
}

pub fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn uuid_v4() -> String {
    use uuid::Uuid;
    Uuid::new_v4().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_session_creation() {
        let session = RepoSession::new();
        assert!(!session.session_id.is_empty());
        assert!(session.started_at > 0);
        assert!(session.last_event_at >= session.started_at);
    }

    #[test]
    fn test_session_expiration() {
        let mut session = RepoSession::new();
        assert!(!session.is_expired());

        // Manually set old timestamp to test expiration
        session.last_event_at = 0;
        assert!(session.is_expired());
    }

    #[test]
    fn test_session_store_persistence() {
        let tmp = tempdir().unwrap();
        let mut store = SessionStore::default();

        let repo = Path::new("/test/repo");
        let session_id = store.get_session_id(repo).unwrap();

        store.save(tmp.path()).unwrap();

        // Load fresh store
        let loaded = SessionStore::load(tmp.path());
        assert_eq!(
            loaded.sessions.get("/test/repo").unwrap().session_id,
            session_id
        );
    }

    #[test]
    fn test_session_timeout() {
        let tmp = tempdir().unwrap();
        let mut store = SessionStore::default();
        let repo = Path::new("/test/repo");

        // Create session
        let session_id = store.get_session_id(repo).unwrap();
        store.save(tmp.path()).unwrap();

        // Load and manually expire
        let mut loaded = SessionStore::load(tmp.path());
        loaded.sessions.get_mut("/test/repo").unwrap().last_event_at = 0;
        loaded.save(tmp.path()).unwrap();

        // Reload and get session - should create new one since expired
        let mut reloaded = SessionStore::load(tmp.path());
        let new_session_id = reloaded.get_session_id(repo).unwrap();
        assert_ne!(new_session_id, session_id);
    }

    #[test]
    fn test_session_expiration_saturates_when_last_event_is_in_future() {
        let now = now_secs();
        let session = RepoSession {
            session_id: "session-1".to_string(),
            started_at: now,
            last_event_at: now + 60,
        };

        assert!(!session.is_expired());
    }

    #[test]
    fn test_session_duration_saturates_when_started_at_is_in_future() {
        let now = now_secs();
        let session = RepoSession {
            session_id: "session-1".to_string(),
            started_at: now + 60,
            last_event_at: now,
        };

        assert_eq!(session.session_duration_secs(), 0);
    }
}
