use std::path::Path;

use anyhow::Result;

use crate::host::checkpoints::session::backend::SessionBackend;
use crate::host::checkpoints::strategy::Strategy;
use crate::host::hooks::runtime::agent_runtime::{
    CODEX_HOOK_AGENT_PROFILE, SessionInfoInput, handle_session_start_with_profile,
    handle_stop_with_profile,
};

pub fn handle_session_start_codex(
    input: SessionInfoInput,
    backend: &dyn SessionBackend,
    repo_root: Option<&Path>,
) -> Result<()> {
    handle_session_start_with_profile(input, backend, repo_root, Some(CODEX_HOOK_AGENT_PROFILE))
}

pub fn handle_stop_codex(
    input: SessionInfoInput,
    backend: &dyn SessionBackend,
    strategy: &dyn Strategy,
    repo_root: Option<&Path>,
) -> Result<()> {
    handle_stop_with_profile(
        input,
        backend,
        strategy,
        repo_root,
        CODEX_HOOK_AGENT_PROFILE,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::agents::{AGENT_NAME_CODEX, AGENT_TYPE_CODEX};
    use crate::host::checkpoints::session::backend::SessionBackend;
    use crate::host::checkpoints::session::local_backend::LocalFileBackend;
    use crate::host::checkpoints::session::phase::SessionPhase;
    use crate::host::checkpoints::session::state::PrePromptState;
    use crate::host::checkpoints::strategy::{StepContext, TaskStepContext};
    use crate::test_support::git_fixtures::write_test_daemon_config;
    use crate::test_support::process_state::git_command;
    use std::fs;
    use std::path::Path;
    use std::sync::Mutex;
    use tempfile::TempDir;

    #[derive(Default)]
    struct RecordingStrategy {
        step_calls: Mutex<Vec<StepContext>>,
    }

    impl Strategy for RecordingStrategy {
        fn name(&self) -> &str {
            "recording"
        }

        fn save_step(&self, ctx: &StepContext) -> Result<()> {
            self.step_calls
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .push(ctx.clone());
            Ok(())
        }

        fn save_task_step(&self, _ctx: &TaskStepContext) -> Result<()> {
            Ok(())
        }

        fn prepare_commit_msg(&self, _commit_msg_file: &Path, _source: Option<&str>) -> Result<()> {
            Ok(())
        }

        fn commit_msg(&self, _commit_msg_file: &Path) -> Result<()> {
            Ok(())
        }

        fn post_commit(&self) -> Result<()> {
            Ok(())
        }

        fn pre_push(&self, _remote: &str, _stdin_lines: &[String]) -> Result<()> {
            Ok(())
        }
    }

    fn git_ok(repo_root: &Path, args: &[&str]) {
        let out = git_command()
            .args(args)
            .current_dir(repo_root)
            .output()
            .unwrap_or_else(|err| panic!("failed to start git {:?}: {err}", args));
        assert!(
            out.status.success(),
            "git {:?} failed\nstdout:\n{}\nstderr:\n{}",
            args,
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
    }

    fn seed_git_repo() -> TempDir {
        let dir = TempDir::new().expect("temp dir");
        let repo_root = dir.path();

        git_ok(repo_root, &["init"]);
        git_ok(repo_root, &["checkout", "-B", "main"]);
        git_ok(repo_root, &["config", "user.name", "Bitloops Test"]);
        git_ok(
            repo_root,
            &["config", "user.email", "bitloops-test@example.com"],
        );

        fs::write(repo_root.join("tracked.txt"), "one\n").expect("write tracked file");
        git_ok(repo_root, &["add", "tracked.txt"]);
        git_ok(repo_root, &["commit", "-m", "initial"]);

        dir
    }

    #[test]
    fn session_start_codex_persists_shared_session_state() {
        let dir = TempDir::new().expect("temp dir");
        fs::create_dir_all(dir.path().join(".git")).expect("create git dir");
        let backend = LocalFileBackend::new(dir.path());

        handle_session_start_codex(
            SessionInfoInput {
                session_id: "codex-session".to_string(),
                transcript_path: "/tmp/codex.jsonl".to_string(),
            },
            &backend,
            None,
        )
        .expect("session-start should succeed");

        let state = backend
            .load_session("codex-session")
            .expect("load session")
            .expect("saved session");
        assert_eq!(state.phase, SessionPhase::Idle);
        assert_eq!(state.transcript_path, "/tmp/codex.jsonl");
    }

    #[test]
    fn stop_codex_uses_codex_profile_when_checkpointing() {
        let repo = seed_git_repo();
        let backend = LocalFileBackend::new(repo.path());
        let strategy = RecordingStrategy::default();
        let session_id = "codex-session";
        let transcript_path = repo.path().join("codex.jsonl");
        write_test_daemon_config(repo.path());
        fs::write(&transcript_path, "").expect("write transcript");

        backend
            .save_pre_prompt(&PrePromptState {
                session_id: session_id.to_string(),
                prompt: "Refactor tracked file".to_string(),
                transcript_path: transcript_path.to_string_lossy().to_string(),
                ..Default::default()
            })
            .expect("save pre-prompt");

        fs::write(repo.path().join("tracked.txt"), "two\n").expect("modify tracked file");

        handle_stop_codex(
            SessionInfoInput {
                session_id: session_id.to_string(),
                transcript_path: transcript_path.to_string_lossy().to_string(),
            },
            &backend,
            &strategy,
            Some(repo.path()),
        )
        .expect("stop should succeed");

        let step_calls = strategy
            .step_calls
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        assert_eq!(step_calls.len(), 1);
        assert_eq!(step_calls[0].agent_type, AGENT_NAME_CODEX);
        assert_eq!(
            step_calls[0].modified_files,
            vec!["tracked.txt".to_string()]
        );
        drop(step_calls);

        let state = backend
            .load_session(session_id)
            .expect("load session")
            .expect("saved session");
        assert_eq!(state.agent_type, AGENT_TYPE_CODEX);
    }
}
