use super::*;
use crate::engine::agent::AGENT_TYPE_CLAUDE_CODE;
use crate::engine::db::SqliteConnectionPool;
use crate::engine::session::backend::SessionBackend;
use crate::engine::session::local_backend::LocalFileBackend;
use crate::engine::session::state::{PrePromptState, PreTaskState, SessionState};
use crate::test_support::process_state::{git_command, with_env_var, with_git_env_cleared};
use rusqlite::OptionalExtension;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use tempfile::TempDir;
const HIGH_ENTROPY_SECRET: &str = "sk-ant-api03-xK9mZ2vL8nQ5rT1wY4bC7dF0gH3jE6pA";

/// Creates a real git repository with an initial commit for testing.
fn setup_git_repo(dir: &TempDir) -> String {
    let run = |args: &[&str]| {
        let out = git_command()
            .args(args)
            .current_dir(dir.path())
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
    fs::write(dir.path().join("README.md"), "initial content").unwrap();
    run(&["add", "."]);
    run(&["commit", "--allow-empty", "-m", "initial"]);
    // Return HEAD hash.
    let out = git_command()
        .args(["rev-parse", "HEAD"])
        .current_dir(dir.path())
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
fn setup_empty_git_repo(dir: &TempDir) {
    let run = |args: &[&str]| {
        let out = git_command()
            .args(args)
            .current_dir(dir.path())
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
}

struct CountingBackend {
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
fn initialize_session_uses_injected_session_backend() {
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

fn git_ok(repo_root: &Path, args: &[&str]) -> String {
    run_git(repo_root, args).unwrap_or_else(|e| panic!("git {:?} failed: {e}", args))
}

fn temporary_checkpoints_db_path(repo_root: &Path) -> PathBuf {
    repo_root
        .join(paths::BITLOOPS_DIR)
        .join("devql")
        .join("relational.db")
}

fn latest_temporary_tree_hash(repo_root: &Path, session_id: &str) -> Option<String> {
    let sqlite = SqliteConnectionPool::connect(temporary_checkpoints_db_path(repo_root)).ok()?;
    sqlite.initialise_checkpoint_schema().ok()?;
    let repo_id = crate::engine::devql::resolve_repo_identity(repo_root)
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

fn temporary_checkpoint_count(repo_root: &Path, session_id: &str) -> i64 {
    let sqlite = SqliteConnectionPool::connect(temporary_checkpoints_db_path(repo_root)).unwrap();
    sqlite.initialise_checkpoint_schema().unwrap();
    let repo_id = crate::engine::devql::resolve_repo_identity(repo_root)
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

fn query_commit_checkpoint_id(repo_root: &Path, commit_sha: &str) -> Option<String> {
    let sqlite = SqliteConnectionPool::connect(temporary_checkpoints_db_path(repo_root)).ok()?;
    sqlite.initialise_checkpoint_schema().ok()?;
    let repo_id = crate::engine::devql::resolve_repo_identity(repo_root)
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

fn query_commit_checkpoint_count(repo_root: &Path, commit_sha: &str) -> i64 {
    let sqlite = SqliteConnectionPool::connect(temporary_checkpoints_db_path(repo_root)).unwrap();
    sqlite.initialise_checkpoint_schema().unwrap();
    let repo_id = crate::engine::devql::resolve_repo_identity(repo_root)
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
struct CheckpointBlobRow {
    storage_backend: String,
    storage_path: String,
    content_hash: String,
}

fn committed_checkpoint_blob_root(repo_root: &Path) -> PathBuf {
    repo_root.join(paths::BITLOOPS_DIR).join("blobs")
}

fn read_blob_payload_from_storage(repo_root: &Path, storage_path: &str) -> Vec<u8> {
    let disk_path = committed_checkpoint_blob_root(repo_root).join(storage_path);
    std::fs::read(&disk_path)
        .unwrap_or_else(|err| panic!("failed reading blob payload at {}: {err}", disk_path.display()))
}

fn query_checkpoint_session_content_hash(
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

fn query_checkpoint_subagent_transcript_path(
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

fn query_checkpoint_session_content_hash_by_index(
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

fn query_checkpoint_blob_row(
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

fn init_sequence_worktree_repo() -> (TempDir, PathBuf, PathBuf) {
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

fn create_shadow_branch_with_content(repo_root: &Path, branch: &str, files: &[(&str, &str)]) {
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
