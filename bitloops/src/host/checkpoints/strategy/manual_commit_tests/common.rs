use super::*;
use crate::adapters::agents::AGENT_TYPE_CLAUDE_CODE;
use crate::host::checkpoints::session::backend::SessionBackend;
use crate::host::checkpoints::session::create_session_backend_or_local;
pub(crate) use crate::host::checkpoints::session::local_backend::LocalFileBackend;
use crate::host::checkpoints::session::state::{PrePromptState, PreTaskState, SessionState};
pub(crate) use crate::storage::SqliteConnectionPool;
pub(crate) use crate::test_support::process_state::{
    ALLOW_HOST_GIT_CONFIG_ENV, git_command, isolated_git_command, with_env_vars,
    with_git_env_cleared,
};
use rusqlite::OptionalExtension;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
pub(crate) use tempfile::TempDir;
pub(crate) const HIGH_ENTROPY_SECRET: &str = "sk-ant-api03-xK9mZ2vL8nQ5rT1wY4bC7dF0gH3jE6pA";

pub(crate) fn ensure_test_store_backends(repo_root: &Path) {
    let sqlite_path = paths::default_relational_db_path(repo_root);
    if let Some(parent) = sqlite_path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    let sqlite = SqliteConnectionPool::connect(sqlite_path).unwrap();
    sqlite.initialise_checkpoint_schema().unwrap();

    let duckdb_path = paths::default_events_db_path(repo_root);
    if let Some(parent) = duckdb_path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    let duckdb = duckdb::Connection::open(duckdb_path).unwrap();
    duckdb
        .execute_batch(
            "CREATE TABLE IF NOT EXISTS checkpoint_events (
            event_id VARCHAR PRIMARY KEY,
            event_time VARCHAR,
            repo_id VARCHAR,
            checkpoint_id VARCHAR,
            session_id VARCHAR,
            commit_sha VARCHAR,
            branch VARCHAR,
            event_type VARCHAR,
            agent VARCHAR,
            strategy VARCHAR,
            files_touched VARCHAR,
            payload VARCHAR
        );",
        )
        .unwrap();

    fs::create_dir_all(paths::default_blob_store_path(repo_root)).unwrap();
}

/// Creates a real git repository with an initial commit for testing.
pub(crate) fn setup_git_repo(dir: &TempDir) -> String {
    let run = |args: &[&str]| {
        let out = isolated_git_command(dir.path())
            .args(args)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "git {:?} failed\nstdout:\n{}\nstderr:\n{}",
            args,
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
    };
    run(&["init"]);
    run(&["config", "user.email", "t@t.com"]);
    run(&["config", "user.name", "Test"]);
    run(&["config", "commit.gpgsign", "false"]);
    ensure_test_store_backends(dir.path());
    fs::write(dir.path().join("README.md"), "initial content").unwrap();
    run(&["add", "."]);
    run(&["commit", "--allow-empty", "-m", "initial"]);
    // Return HEAD hash.
    let out = isolated_git_command(dir.path())
        .args(["rev-parse", "HEAD"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git [\"rev-parse\", \"HEAD\"] failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// Creates a git repo with no commits.
pub(crate) fn setup_empty_git_repo(dir: &TempDir) {
    let run = |args: &[&str]| {
        let out = isolated_git_command(dir.path())
            .args(args)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "git {:?} failed\nstdout:\n{}\nstderr:\n{}",
            args,
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
    };
    run(&["init"]);
    run(&["config", "user.email", "t@t.com"]);
    run(&["config", "user.name", "Test"]);
    run(&["config", "commit.gpgsign", "false"]);
    ensure_test_store_backends(dir.path());
}

pub(crate) fn session_backend(repo_root: &Path) -> Box<dyn SessionBackend> {
    create_session_backend_or_local(repo_root)
}

pub(crate) struct CountingBackend {
    inner: LocalFileBackend,
    save_calls: Arc<AtomicUsize>,
}

impl SessionBackend for CountingBackend {
    fn list_sessions(&self) -> anyhow::Result<Vec<SessionState>> {
        self.inner.list_sessions()
    }

    fn load_session(&self, session_id: &str) -> anyhow::Result<Option<SessionState>> {
        self.inner.load_session(session_id)
    }

    fn save_session(&self, state: &SessionState) -> anyhow::Result<()> {
        self.save_calls.fetch_add(1, Ordering::SeqCst);
        self.inner.save_session(state)
    }

    fn delete_session(&self, session_id: &str) -> anyhow::Result<()> {
        self.inner.delete_session(session_id)
    }

    fn load_pre_prompt(&self, session_id: &str) -> anyhow::Result<Option<PrePromptState>> {
        self.inner.load_pre_prompt(session_id)
    }

    fn save_pre_prompt(&self, state: &PrePromptState) -> anyhow::Result<()> {
        self.inner.save_pre_prompt(state)
    }

    fn delete_pre_prompt(&self, session_id: &str) -> anyhow::Result<()> {
        self.inner.delete_pre_prompt(session_id)
    }

    fn create_pre_task_marker(&self, state: &PreTaskState) -> anyhow::Result<()> {
        self.inner.create_pre_task_marker(state)
    }

    fn load_pre_task_marker(&self, tool_use_id: &str) -> anyhow::Result<Option<PreTaskState>> {
        self.inner.load_pre_task_marker(tool_use_id)
    }

    fn delete_pre_task_marker(&self, tool_use_id: &str) -> anyhow::Result<()> {
        self.inner.delete_pre_task_marker(tool_use_id)
    }

    fn find_active_pre_task(&self) -> anyhow::Result<Option<String>> {
        self.inner.find_active_pre_task()
    }
}

#[test]
pub(crate) fn initialize_session_uses_injected_session_backend() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    let save_calls = Arc::new(AtomicUsize::new(0));

    let strategy = ManualCommitStrategy::with_backend(
        dir.path(),
        Box::new(CountingBackend {
            inner: LocalFileBackend::new(dir.path()),
            save_calls: Arc::clone(&save_calls),
        }),
    );

    strategy
        .initialize_session(
            "injected-backend-session",
            AGENT_TYPE_CLAUDE_CODE,
            "/tmp/injected-transcript.jsonl",
            "test prompt",
        )
        .unwrap();

    assert!(
        save_calls.load(Ordering::SeqCst) >= 1,
        "expected injected backend to record at least one save"
    );

    let verify_backend = LocalFileBackend::new(dir.path());
    let state = verify_backend
        .load_session("injected-backend-session")
        .unwrap()
        .unwrap();
    assert_eq!(state.phase, SessionPhase::Active);
}

pub(crate) fn git_ok(repo_root: &Path, args: &[&str]) -> String {
    run_git(repo_root, args).unwrap_or_else(|e| panic!("git {:?} failed: {e}", args))
}

pub(crate) fn commit_files(repo_root: &Path, files: &[(&str, &str)], message: &str) -> String {
    for (path, content) in files {
        let full_path = repo_root.join(path);
        if let Some(parent) = full_path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(full_path, content).unwrap();
    }

    let mut args = vec!["add"];
    args.extend(files.iter().map(|(path, _)| *path));
    git_ok(repo_root, &args);
    git_ok(repo_root, &["commit", "-m", message]);
    git_ok(repo_root, &["rev-parse", "HEAD"])
}

pub(crate) fn temporary_checkpoints_db_path(repo_root: &Path) -> PathBuf {
    crate::host::checkpoints::strategy::manual_commit::resolve_temporary_checkpoint_sqlite_path(
        repo_root,
    )
    .unwrap_or_else(|_| paths::default_relational_db_path(repo_root))
}

pub(crate) fn latest_temporary_tree_hash(repo_root: &Path, session_id: &str) -> Option<String> {
    let sqlite = SqliteConnectionPool::connect(temporary_checkpoints_db_path(repo_root)).ok()?;
    sqlite.initialise_checkpoint_schema().ok()?;
    let repo_id = crate::host::devql::resolve_repo_identity(repo_root)
        .ok()?
        .repo_id;

    sqlite
        .with_connection(|conn| {
            conn.query_row(
                "SELECT tree_hash
                 FROM temporary_checkpoints
                 WHERE session_id = ?1 AND repo_id = ?2
                 ORDER BY id DESC
                 LIMIT 1",
                rusqlite::params![session_id, repo_id],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(anyhow::Error::from)
        })
        .ok()
        .flatten()
}

pub(crate) fn temporary_checkpoint_count(repo_root: &Path, session_id: &str) -> i64 {
    let sqlite = SqliteConnectionPool::connect(temporary_checkpoints_db_path(repo_root)).unwrap();
    sqlite.initialise_checkpoint_schema().unwrap();
    let repo_id = crate::host::devql::resolve_repo_identity(repo_root)
        .unwrap()
        .repo_id;

    sqlite
        .with_connection(|conn| {
            let count: i64 = conn.query_row(
                "SELECT COUNT(*)
                 FROM temporary_checkpoints
                 WHERE session_id = ?1 AND repo_id = ?2",
                rusqlite::params![session_id, repo_id],
                |row| row.get(0),
            )?;
            Ok(count)
        })
        .unwrap()
}

pub(crate) fn query_commit_checkpoint_id(repo_root: &Path, commit_sha: &str) -> Option<String> {
    let sqlite = SqliteConnectionPool::connect(temporary_checkpoints_db_path(repo_root)).ok()?;
    sqlite.initialise_checkpoint_schema().ok()?;
    let repo_id = crate::host::devql::resolve_repo_identity(repo_root)
        .ok()?
        .repo_id;

    sqlite
        .with_connection(|conn| {
            conn.query_row(
                "SELECT checkpoint_id
                 FROM commit_checkpoints
                 WHERE commit_sha = ?1 AND repo_id = ?2
                 ORDER BY created_at DESC, checkpoint_id DESC
                 LIMIT 1",
                rusqlite::params![commit_sha, repo_id],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(anyhow::Error::from)
        })
        .ok()
        .flatten()
}

pub(crate) fn query_commit_checkpoint_count(repo_root: &Path, commit_sha: &str) -> i64 {
    let sqlite = SqliteConnectionPool::connect(temporary_checkpoints_db_path(repo_root)).unwrap();
    sqlite.initialise_checkpoint_schema().unwrap();
    let repo_id = crate::host::devql::resolve_repo_identity(repo_root)
        .unwrap()
        .repo_id;

    sqlite
        .with_connection(|conn| {
            let count: i64 = conn.query_row(
                "SELECT COUNT(*)
                 FROM commit_checkpoints
                 WHERE commit_sha = ?1 AND repo_id = ?2",
                rusqlite::params![commit_sha, repo_id],
                |row| row.get(0),
            )?;
            Ok(count)
        })
        .unwrap()
}

#[derive(Debug)]
pub(crate) struct CheckpointBlobRow {
    pub(crate) storage_backend: String,
    pub(crate) storage_path: String,
    pub(crate) content_hash: String,
}

pub(crate) fn committed_checkpoint_blob_root(repo_root: &Path) -> PathBuf {
    paths::default_blob_store_path(repo_root)
}

pub(crate) fn read_blob_payload_from_storage(repo_root: &Path, storage_path: &str) -> Vec<u8> {
    let disk_path = committed_checkpoint_blob_root(repo_root).join(storage_path);
    std::fs::read(&disk_path).unwrap_or_else(|err| {
        panic!(
            "failed reading blob payload at {}: {err}",
            disk_path.display()
        )
    })
}

pub(crate) fn query_checkpoint_session_content_hash(
    repo_root: &Path,
    checkpoint_id: &str,
    session_id: &str,
) -> Option<String> {
    let sqlite = SqliteConnectionPool::connect(temporary_checkpoints_db_path(repo_root)).ok()?;
    sqlite.initialise_checkpoint_schema().ok()?;
    sqlite
        .with_connection(|conn| {
            conn.query_row(
                "SELECT content_hash
                 FROM checkpoint_sessions
                 WHERE checkpoint_id = ?1 AND session_id = ?2
                 LIMIT 1",
                rusqlite::params![checkpoint_id, session_id],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(anyhow::Error::from)
        })
        .ok()
        .flatten()
}

pub(crate) fn query_checkpoint_subagent_transcript_path(
    repo_root: &Path,
    checkpoint_id: &str,
    session_id: &str,
) -> Option<String> {
    let sqlite = SqliteConnectionPool::connect(temporary_checkpoints_db_path(repo_root)).ok()?;
    sqlite.initialise_checkpoint_schema().ok()?;
    sqlite
        .with_connection(|conn| {
            conn.query_row(
                "SELECT subagent_transcript_path
                 FROM checkpoint_sessions
                 WHERE checkpoint_id = ?1 AND session_id = ?2
                 LIMIT 1",
                rusqlite::params![checkpoint_id, session_id],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(anyhow::Error::from)
        })
        .ok()
        .flatten()
}

pub(crate) fn query_checkpoint_session_content_hash_by_index(
    repo_root: &Path,
    checkpoint_id: &str,
    session_index: i64,
) -> Option<String> {
    let sqlite = SqliteConnectionPool::connect(temporary_checkpoints_db_path(repo_root)).ok()?;
    sqlite.initialise_checkpoint_schema().ok()?;
    sqlite
        .with_connection(|conn| {
            conn.query_row(
                "SELECT content_hash
                 FROM checkpoint_sessions
                 WHERE checkpoint_id = ?1 AND session_index = ?2
                 LIMIT 1",
                rusqlite::params![checkpoint_id, session_index],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(anyhow::Error::from)
        })
        .ok()
        .flatten()
}

pub(crate) fn query_checkpoint_blob_row(
    repo_root: &Path,
    checkpoint_id: &str,
    session_index: i64,
    blob_type: &str,
) -> Option<CheckpointBlobRow> {
    let sqlite = SqliteConnectionPool::connect(temporary_checkpoints_db_path(repo_root)).ok()?;
    sqlite.initialise_checkpoint_schema().ok()?;
    sqlite
        .with_connection(|conn| {
            conn.query_row(
                "SELECT storage_backend, storage_path, content_hash
                 FROM checkpoint_blobs
                 WHERE checkpoint_id = ?1 AND session_index = ?2 AND blob_type = ?3
                 LIMIT 1",
                rusqlite::params![checkpoint_id, session_index, blob_type],
                |row| {
                    Ok(CheckpointBlobRow {
                        storage_backend: row.get(0)?,
                        storage_path: row.get(1)?,
                        content_hash: row.get(2)?,
                    })
                },
            )
            .optional()
            .map_err(anyhow::Error::from)
        })
        .ok()
        .flatten()
}

pub(crate) fn query_checkpoint_file_session_ids(
    repo_root: &Path,
    checkpoint_id: &str,
) -> Vec<String> {
    let sqlite = SqliteConnectionPool::connect(temporary_checkpoints_db_path(repo_root)).unwrap();
    sqlite.initialise_checkpoint_schema().unwrap();
    let repo_id = crate::host::devql::resolve_repo_identity(repo_root)
        .unwrap()
        .repo_id;

    sqlite
        .with_connection(|conn| {
            let mut stmt = conn.prepare(
                "SELECT DISTINCT session_id
                 FROM checkpoint_files
                 WHERE checkpoint_id = ?1 AND repo_id = ?2
                 ORDER BY session_id ASC",
            )?;
            let mut rows = stmt.query(rusqlite::params![checkpoint_id, repo_id])?;
            let mut session_ids = Vec::new();
            while let Some(row) = rows.next()? {
                session_ids.push(row.get::<_, String>(0)?);
            }
            Ok(session_ids)
        })
        .unwrap()
}

pub(crate) fn init_sequence_worktree_repo() -> (TempDir, PathBuf, PathBuf) {
    let parent = tempfile::tempdir().unwrap();
    let main_repo = parent.path().join("main");
    let worktree_dir = parent.path().join("worktree");
    fs::create_dir_all(&main_repo).unwrap();

    git_ok(&main_repo, &["init"]);
    git_ok(&main_repo, &["config", "user.email", "t@t.com"]);
    git_ok(&main_repo, &["config", "user.name", "Test"]);
    fs::write(main_repo.join("README.md"), "initial content").unwrap();
    git_ok(&main_repo, &["add", "."]);
    git_ok(&main_repo, &["commit", "--allow-empty", "-m", "initial"]);
    git_ok(
        &main_repo,
        &[
            "worktree",
            "add",
            worktree_dir.to_string_lossy().as_ref(),
            "-b",
            "feature",
        ],
    );

    (parent, main_repo, worktree_dir)
}

pub(crate) fn create_shadow_branch_with_content(
    repo_root: &Path,
    branch: &str,
    files: &[(&str, &str)],
) {
    let current = git_ok(repo_root, &["rev-parse", "--abbrev-ref", "HEAD"]);
    git_ok(repo_root, &["checkout", "-b", branch]);

    for (path, content) in files {
        let full_path = repo_root.join(path);
        if let Some(parent) = full_path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(full_path, content).unwrap();
    }
    git_ok(repo_root, &["add", "-A"]);
    git_ok(
        repo_root,
        &["commit", "--allow-empty", "-m", "shadow checkpoint"],
    );
    git_ok(repo_root, &["checkout", &current]);
}
