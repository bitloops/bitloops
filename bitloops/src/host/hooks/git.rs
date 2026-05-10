//! `bitloops hooks git <verb>` — hidden subcommand dispatcher for git hooks.
//!
//! Git hook scripts installed by `bitloops enable` call this subcommand.
//! All handlers exit 0 on success so git hooks don't block the user.

use std::path::PathBuf;
use std::time::SystemTime;
use std::{io, io::Read};

use anyhow::Result;
use clap::{Args, Subcommand};

use crate::config::settings;
use crate::host::checkpoints::session::create_session_backend_or_local;
use crate::host::checkpoints::session::state::find_most_recent_session;
use crate::telemetry::logging;
use crate::utils::paths;

use crate::host::checkpoints::strategy::Strategy;
use crate::host::checkpoints::strategy::manual_commit::ManualCommitStrategy;
use crate::host::checkpoints::strategy::registry::{self, StrategyRegistry};

fn init_hook_logging(repo_root: &std::path::Path) -> Box<dyn FnOnce()> {
    let backend = create_session_backend_or_local(repo_root);
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

    if let Err(err) = result.as_ref() {
        logging::warn(
            &ctx,
            "hook failed",
            &[
                logging::string_attr("hook", hook_name),
                logging::string_attr("hook_type", "git"),
                logging::string_attr("strategy", strategy_name),
                logging::string_attr("error", &format!("{err:#}")),
            ],
        );
    }

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

    /// Handle the post-merge git hook.
    #[command(name = "post-merge")]
    PostMerge {
        /// `1` when merge was a squash merge, `0` otherwise (provided by git as $1).
        is_squash: i32,
    },

    /// Handle the post-checkout git hook.
    #[command(name = "post-checkout")]
    PostCheckout {
        /// Previous HEAD commit (provided by git as $1).
        previous_head: String,
        /// New HEAD commit (provided by git as $2).
        new_head: String,
        /// `1` when switching branches, `0` otherwise (provided by git as $3).
        is_branch_checkout: i32,
    },

    /// Handle the reference-transaction git hook.
    #[command(name = "reference-transaction")]
    ReferenceTransaction {
        /// Hook state (provided by git as $1): prepared, committed, or aborted.
        state: String,
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
    let config_start = std::env::current_dir().unwrap_or_else(|_| repo_root.clone());

    // Skip silently when Bitloops is disabled.
    if !settings::is_enabled_for_hooks(&config_start) {
        return Ok(());
    }

    let strategy_name = settings::load_settings(&config_start)
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
                let stdin_lines = read_pre_push_stdin_lines();
                strategy.pre_push(&remote, &stdin_lines)
            })
        }
        GitHookVerb::PostMerge { is_squash } => {
            run_git_hook_with_logging(&repo_root, "post-merge", &strategy_name, || {
                strategy.post_merge(is_squash != 0)
            })
        }
        GitHookVerb::PostCheckout {
            previous_head,
            new_head,
            is_branch_checkout,
        } => run_git_hook_with_logging(&repo_root, "post-checkout", &strategy_name, || {
            strategy.post_checkout(&previous_head, &new_head, is_branch_checkout != 0)
        }),
        GitHookVerb::ReferenceTransaction { state } => {
            run_git_hook_with_logging(&repo_root, "reference-transaction", &strategy_name, || {
                let stdin_lines = read_reference_transaction_stdin_lines();
                strategy.reference_transaction(&state, &stdin_lines)
            })
        }
    };

    if let Err(e) = result {
        eprintln!("[bitloops] Warning: git hook error: {e:#}");
    }

    Ok(())
}

fn read_reference_transaction_stdin_lines() -> Vec<String> {
    let mut raw = String::new();
    if io::stdin().read_to_string(&mut raw).is_err() {
        return Vec::new();
    }
    raw.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(ToString::to_string)
        .collect()
}

fn read_pre_push_stdin_lines() -> Vec<String> {
    let mut raw = String::new();
    if io::stdin().read_to_string(&mut raw).is_err() {
        return Vec::new();
    }
    raw.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(ToString::to_string)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::settings::{self, BitloopsSettings};
    use crate::host::checkpoints::session::backend::SessionBackend;
    use crate::host::checkpoints::session::local_backend::LocalFileBackend;
    use crate::host::checkpoints::session::phase::SessionPhase;
    use crate::host::checkpoints::session::state::SessionState;
    use crate::host::checkpoints::strategy::registry::StrategyRegistry;
    use crate::test_support::logger_lock::with_logger_test_lock;
    use crate::test_support::process_state::{git_command, with_cwd, with_process_state};
    use std::fs;
    use std::path::Path;
    use tempfile::TempDir;

    const TEST_CONFIG_DIR_OVERRIDE_ENV: &str = "BITLOOPS_TEST_CONFIG_DIR_OVERRIDE";
    const TEST_STATE_DIR_OVERRIDE_ENV: &str = "BITLOOPS_TEST_STATE_DIR_OVERRIDE";

    fn test_runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()
            .expect("create tokio runtime")
    }

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

    fn with_hook_test_process_state<T>(
        repo_root: &Path,
        extra_env: &[(&str, Option<&str>)],
        f: impl FnOnce() -> T,
    ) -> T {
        let config_root = repo_root.join("config-root");
        let config_root_value = config_root.display().to_string();
        let mut env = Vec::with_capacity(extra_env.len() + 1);
        env.push((
            TEST_CONFIG_DIR_OVERRIDE_ENV,
            Some(config_root_value.as_str()),
        ));
        env.extend_from_slice(extra_env);
        with_process_state(Some(repo_root), &env, f)
    }

    fn with_test_logging_state<T>(repo_root: &Path, f: impl FnOnce() -> T) -> T {
        let state_root = repo_root.join("state-root");
        let state_root_value = state_root.display().to_string();
        with_hook_test_process_state(
            repo_root,
            &[(TEST_STATE_DIR_OVERRIDE_ENV, Some(state_root_value.as_str()))],
            f,
        )
    }

    fn write_strategy_config(repo_root: &Path, strategy: &str) {
        let settings = BitloopsSettings {
            strategy: strategy.to_string(),
            enabled: true,
            ..Default::default()
        };
        settings::save_settings(&settings, &settings::settings_path(repo_root))
            .expect("write repo policy");
    }

    #[test]
    fn test_init_hook_logging() {
        let dir = tempfile::tempdir().unwrap();
        setup_git_repo(&dir);

        with_test_logging_state(dir.path(), || {
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

                let log_file = logging::log_file_path();
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
        write_strategy_config(dir.path(), "manual-commit");
        let state_root_value = dir.path().join("state-root").display().to_string();

        with_hook_test_process_state(
            dir.path(),
            &[
                (TEST_STATE_DIR_OVERRIDE_ENV, Some(state_root_value.as_str())),
                (logging::LOG_LEVEL_ENV_VAR, None),
            ],
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
                    let rt = test_runtime();
                    let result = rt.block_on(run(
                        GitHooksArgs {
                            verb: GitHookVerb::PostCommit,
                        },
                        &sr,
                    ));
                    assert!(result.is_ok(), "post-commit hook should not fail");

                    let log_file = logging::log_file_path();
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
            let rt = test_runtime();
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

        with_hook_test_process_state(dir.path(), &[], || {
            fs::write(
                dir.path().join(".bitloops.toml"),
                "[capture]\nenabled = false\nstrategy = \"manual-commit\"\n",
            )
            .unwrap();

            let sr = StrategyRegistry::builtin();
            let rt = test_runtime();
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

        with_hook_test_process_state(dir.path(), &[], || {
            with_logger_test_lock(|| {
                // Non-existent nested path causes prepare-commit-msg handler to error.
                let sr = StrategyRegistry::builtin();
                let rt = test_runtime();
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

    #[test]
    fn run_swallows_strategy_errors_and_logs_git_hook_failure_details() {
        let dir = tempfile::tempdir().unwrap();
        setup_git_repo(&dir);
        with_test_logging_state(dir.path(), || {
            with_logger_test_lock(|| {
                logging::reset_logger_for_tests();

                let result = run_git_hook_with_logging(
                    dir.path(),
                    "prepare-commit-msg",
                    "manual-commit",
                    || Err(anyhow::anyhow!("simulated git hook failure")),
                );
                assert!(result.is_err(), "failing hook should return an error");

                let content = fs::read_to_string(logging::log_file_path())
                    .expect("hook log file should exist");
                assert!(
                    content.contains("\"msg\":\"hook failed\""),
                    "expected hook failure log entry, got: {content}"
                );
                assert!(
                    content.contains("simulated git hook failure"),
                    "expected hook failure details, got: {content}"
                );
            });
        });
    }
}
