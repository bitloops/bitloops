//! `bitloops hooks git <verb>` — hidden subcommand dispatcher for git hooks.
//!
//! Git hook scripts installed by `bitloops enable` call this subcommand.
//! All handlers exit 0 on success so git hooks don't block the user.

use std::path::PathBuf;
use std::time::SystemTime;

use anyhow::Result;
use clap::{Args, Subcommand};

use crate::engine::logging;
use crate::engine::paths;
use crate::engine::session::local_backend::LocalFileBackend;
use crate::engine::session::state::find_most_recent_session;
use crate::engine::settings;

use crate::engine::strategy::Strategy;
use crate::engine::strategy::manual_commit::ManualCommitStrategy;
use crate::engine::strategy::registry::{self, StrategyRegistry};

fn init_hook_logging(repo_root: &std::path::Path) -> Box<dyn FnOnce()> {
    let backend = LocalFileBackend::new(repo_root);
    let sessions = backend.list_sessions().unwrap_or_default();
    let session_id = find_most_recent_session(&sessions, &repo_root.to_string_lossy())
        .map(|s| s.session_id)
        .unwrap_or_default();
    let _ = logging::init(&session_id);
    Box::new(logging::close)
}

fn run_git_hook_with_logging<F>(
    repo_root: &std::path::Path,
    hook_name: &str,
    strategy_name: &str,
    handler: F,
) -> Result<()>
where
    F: FnOnce() -> Result<()>,
{
    let cleanup = init_hook_logging(repo_root);
    let start = SystemTime::now();
    let ctx = logging::with_component(logging::background(), "hooks");

    logging::debug(
        &ctx,
        "hook invoked",
        &[
            logging::string_attr("hook", hook_name),
            logging::string_attr("hook_type", "git"),
            logging::string_attr("strategy", strategy_name),
        ],
    );
    logging::info(
        &ctx,
        "hook invoked",
        &[
            logging::string_attr("hook", hook_name),
            logging::string_attr("hook_type", "git"),
            logging::string_attr("strategy", strategy_name),
        ],
    );

    let result = handler();

    logging::log_duration(
        &ctx,
        logging::LogLevel::Debug,
        "hook completed",
        start,
        &[
            logging::string_attr("hook", hook_name),
            logging::string_attr("hook_type", "git"),
            logging::string_attr("strategy", strategy_name),
            logging::bool_attr("success", result.is_ok()),
        ],
    );
    logging::log_duration(
        &ctx,
        logging::LogLevel::Info,
        "hook completed",
        start,
        &[
            logging::string_attr("hook", hook_name),
            logging::string_attr("hook_type", "git"),
            logging::string_attr("strategy", strategy_name),
            logging::bool_attr("success", result.is_ok()),
        ],
    );

    cleanup();
    result
}

// ── Clap types ────────────────────────────────────────────────────────────────

#[derive(Args)]
pub struct GitHooksArgs {
    #[command(subcommand)]
    pub verb: GitHookVerb,
}

#[derive(Subcommand)]
pub enum GitHookVerb {
    /// Handle the prepare-commit-msg git hook.
    #[command(name = "prepare-commit-msg")]
    PrepareCommitMsg {
        /// Path to the commit message file (provided by git).
        commit_msg_file: PathBuf,
        /// Commit source (template/message/merge/squash/commit).
        source: Option<String>,
    },

    /// Handle the commit-msg git hook.
    #[command(name = "commit-msg")]
    CommitMsg {
        /// Path to the commit message file (provided by git).
        commit_msg_file: PathBuf,
    },

    /// Handle the post-commit git hook.
    #[command(name = "post-commit")]
    PostCommit,

    /// Handle the pre-push git hook.
    #[command(name = "pre-push")]
    PrePush {
        /// Remote name (e.g., "origin"), provided by git as $1.
        remote: String,
    },
}

// ── Entry point ───────────────────────────────────────────────────────────────

/// Entry point called from `engine::run` for `bitloops hooks git <verb>`.
pub async fn run(args: GitHooksArgs, strategy_registry: &StrategyRegistry) -> Result<()> {
    // All git hooks: skip silently when not inside a git repo.
    let repo_root = match paths::repo_root() {
        Ok(r) => r,
        Err(_) => return Ok(()),
    };

    // Skip silently when Bitloops is disabled.
    if !settings::is_enabled(&repo_root).unwrap_or(true) {
        return Ok(());
    }

    let strategy_name = settings::load_settings(&repo_root)
        .map(|s| s.strategy)
        .unwrap_or_else(|_| registry::STRATEGY_NAME_MANUAL_COMMIT.to_string());
    let strategy: Box<dyn Strategy> = strategy_registry
        .get(&strategy_name, &repo_root)
        .unwrap_or_else(|_| Box::new(ManualCommitStrategy::new(&repo_root)));

    // Dispatch — all handlers swallow errors (hooks must not block git).
    let result = match args.verb {
        GitHookVerb::PrepareCommitMsg {
            commit_msg_file,
            source,
        } => run_git_hook_with_logging(&repo_root, "prepare-commit-msg", &strategy_name, || {
            strategy.prepare_commit_msg(&commit_msg_file, source.as_deref())
        }),
        GitHookVerb::CommitMsg { commit_msg_file } => {
            run_git_hook_with_logging(&repo_root, "commit-msg", &strategy_name, || {
                strategy.commit_msg(&commit_msg_file)
            })
        }
        GitHookVerb::PostCommit => {
            run_git_hook_with_logging(&repo_root, "post-commit", &strategy_name, || {
                strategy.post_commit()
            })
        }
        GitHookVerb::PrePush { remote } => {
            run_git_hook_with_logging(&repo_root, "pre-push", &strategy_name, || {
                strategy.pre_push(&remote)
            })
        }
    };

    if let Err(e) = result {
        eprintln!("[bitloops] Warning: git hook error: {e:#}");
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::session::backend::SessionBackend;
    use crate::engine::session::phase::SessionPhase;
    use crate::engine::session::state::SessionState;
    use crate::engine::strategy::registry::StrategyRegistry;
    use crate::test_support::logger_lock::with_logger_test_lock;
    use crate::test_support::process_state::{git_command, with_cwd, with_process_state};
    use std::fs;
    use tempfile::TempDir;

    fn setup_git_repo(dir: &TempDir) {
        let run = |args: &[&str]| {
            let out = git_command()
                .args(args)
                .current_dir(dir.path())
                .output()
                .unwrap();
            assert!(out.status.success(), "git {:?} failed", args);
        };
        run(&["init"]);
        run(&["config", "user.email", "t@t.com"]);
        run(&["config", "user.name", "Test"]);
        fs::write(dir.path().join("README.md"), "init").unwrap();
        run(&["add", "."]);
        run(&["commit", "-m", "initial"]);
    }

    #[test]
    fn test_init_hook_logging() {
        let dir = tempfile::tempdir().unwrap();
        setup_git_repo(&dir);

        with_cwd(dir.path(), || {
            with_logger_test_lock(|| {
                let cleanup = init_hook_logging(dir.path());
                cleanup();

                let backend = LocalFileBackend::new(dir.path());
                backend
                    .save_session(&SessionState {
                        session_id: "test-session-12345".to_string(),
                        started_at: "2026-01-01T00:00:00Z".to_string(),
                        last_interaction_time: Some("2026-01-01T00:00:01Z".to_string()),
                        phase: SessionPhase::Active,
                        ..Default::default()
                    })
                    .unwrap();

                let cleanup = init_hook_logging(dir.path());
                cleanup();

                let log_file = dir.path().join(logging::LOGS_DIR).join("bitloops.log");
                assert!(
                    log_file.exists(),
                    "expected log file at {}",
                    log_file.display()
                );
            });
        });
    }

    // Default INFO-level hook execution should write non-empty session log output.
    #[test]
    fn run_post_commit_writes_non_empty_log_at_default_level() {
        let dir = tempfile::tempdir().unwrap();
        setup_git_repo(&dir);

        with_process_state(
            Some(dir.path()),
            &[(logging::LOG_LEVEL_ENV_VAR, None)],
            || {
                with_logger_test_lock(|| {
                    logging::reset_logger_for_tests();

                    let backend = LocalFileBackend::new(dir.path());
                    backend
                        .save_session(&SessionState {
                            session_id: "test-session-default-log".to_string(),
                            started_at: "2026-01-01T00:00:00Z".to_string(),
                            last_interaction_time: Some("2026-01-01T00:00:01Z".to_string()),
                            phase: SessionPhase::Active,
                            ..Default::default()
                        })
                        .unwrap();

                    let sr = StrategyRegistry::builtin();
                    let rt = tokio::runtime::Runtime::new().expect("create tokio runtime");
                    let result = rt.block_on(run(
                        GitHooksArgs {
                            verb: GitHookVerb::PostCommit,
                        },
                        &sr,
                    ));
                    assert!(result.is_ok(), "post-commit hook should not fail");

                    let log_file = dir.path().join(logging::LOGS_DIR).join("bitloops.log");
                    let content = fs::read_to_string(&log_file).expect("log file should exist");
                    assert!(
                        !content.trim().is_empty(),
                        "default log run should write at least one log entry to {}",
                        log_file.display()
                    );
                });
            },
        );
    }

    #[test]
    fn run_no_repo_is_noop() {
        let dir = tempfile::tempdir().unwrap();

        with_cwd(dir.path(), || {
            let sr = StrategyRegistry::builtin();
            let rt = tokio::runtime::Runtime::new().expect("create tokio runtime");
            let result = rt.block_on(run(
                GitHooksArgs {
                    verb: GitHookVerb::PostCommit,
                },
                &sr,
            ));
            assert!(
                result.is_ok(),
                "run should silently no-op outside git repos"
            );
        });
    }

    #[test]
    fn run_disabled_short_circuits() {
        let dir = tempfile::tempdir().unwrap();
        setup_git_repo(&dir);

        with_cwd(dir.path(), || {
            fs::create_dir_all(dir.path().join(".bitloops")).unwrap();
            fs::write(
                dir.path().join(".bitloops/settings.json"),
                r#"{"strategy":"manual-commit","enabled":false}"#,
            )
            .unwrap();

            let sr = StrategyRegistry::builtin();
            let rt = tokio::runtime::Runtime::new().expect("create tokio runtime");
            let result = rt.block_on(run(
                GitHooksArgs {
                    verb: GitHookVerb::PrepareCommitMsg {
                        commit_msg_file: dir.path().join("DOES_NOT_EXIST"),
                        source: None,
                    },
                },
                &sr,
            ));
            assert!(
                result.is_ok(),
                "disabled mode should skip git hook execution"
            );
        });
    }

    #[test]
    fn run_swallows_strategy_errors() {
        let dir = tempfile::tempdir().unwrap();
        setup_git_repo(&dir);

        with_cwd(dir.path(), || {
            with_logger_test_lock(|| {
                // Non-existent nested path causes prepare-commit-msg handler to error.
                let sr = StrategyRegistry::builtin();
                let rt = tokio::runtime::Runtime::new().expect("create tokio runtime");
                let result = rt.block_on(run(
                    GitHooksArgs {
                        verb: GitHookVerb::PrepareCommitMsg {
                            commit_msg_file: dir.path().join("missing").join("COMMIT_EDITMSG"),
                            source: None,
                        },
                    },
                    &sr,
                ));
                assert!(
                    result.is_ok(),
                    "run should swallow strategy errors and keep git hook non-blocking"
                );
            });
        });
    }
}
