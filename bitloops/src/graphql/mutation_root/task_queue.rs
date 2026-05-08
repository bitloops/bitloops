use async_graphql::{Context, Result};

use crate::graphql::DevqlGraphqlContext;
use crate::graphql::types::{TaskKind, TaskObject, TaskQueueControlResultObject};

use super::errors::operation_error;
use super::inputs::EnqueueTaskInput;
use super::results::EnqueueTaskResult;
use super::validation::{normalise_optional_input, require_non_empty_input};

pub(super) async fn enqueue_task(
    ctx: &Context<'_>,
    input: EnqueueTaskInput,
) -> Result<EnqueueTaskResult> {
    let context = ctx.data_unchecked::<DevqlGraphqlContext>();
    context
        .require_repo_write_scope()
        .map_err(|err| operation_error("BAD_USER_INPUT", "validation", "enqueueTask", err))?;
    let cfg = context
        .devql_config()
        .map_err(|err| operation_error("BACKEND_ERROR", "configuration", "enqueueTask", err))?;
    let (source, spec) = resolve_enqueue_task_input(input, "enqueueTask")?;

    crate::daemon::shared_devql_task_coordinator()
        .register_subscription_hub(context.subscriptions());
    let queued = crate::daemon::enqueue_task_for_config(&cfg, source, spec)
        .map_err(|err| operation_error("BACKEND_ERROR", "task", "enqueueTask", err))?;
    Ok(EnqueueTaskResult {
        task: queued.task.into(),
        merged: queued.merged,
    })
}

pub(super) async fn pause_task_queue(
    ctx: &Context<'_>,
    reason: Option<String>,
) -> Result<TaskQueueControlResultObject> {
    let context = ctx.data_unchecked::<DevqlGraphqlContext>();
    context
        .require_repo_write_scope()
        .map_err(|err| operation_error("BAD_USER_INPUT", "validation", "pauseTaskQueue", err))?;
    let cfg = context
        .devql_config()
        .map_err(|err| operation_error("BACKEND_ERROR", "configuration", "pauseTaskQueue", err))?;
    let reason = normalise_optional_input(reason, "reason", "pauseTaskQueue")?;
    crate::daemon::pause_devql_tasks(cfg.repo.repo_id.as_str(), reason)
        .map(Into::into)
        .map_err(|err| operation_error("BACKEND_ERROR", "task", "pauseTaskQueue", err))
}

pub(super) async fn resume_task_queue(
    ctx: &Context<'_>,
    repo_id: Option<String>,
) -> Result<TaskQueueControlResultObject> {
    let context = ctx.data_unchecked::<DevqlGraphqlContext>();
    context
        .require_repo_write_scope()
        .map_err(|err| operation_error("BAD_USER_INPUT", "validation", "resumeTaskQueue", err))?;
    let cfg = context
        .devql_config()
        .map_err(|err| operation_error("BACKEND_ERROR", "configuration", "resumeTaskQueue", err))?;
    let requested_repo_id = repo_id
        .map(|value| require_non_empty_input(value, "repoId", "resumeTaskQueue"))
        .transpose()?;
    if let Some(requested_repo_id) = requested_repo_id
        && requested_repo_id != cfg.repo.repo_id
    {
        return Err(operation_error(
            "BAD_USER_INPUT",
            "validation",
            "resumeTaskQueue",
            format!(
                "repoId `{requested_repo_id}` does not match the current repository `{}`",
                cfg.repo.repo_id
            ),
        ));
    }

    crate::daemon::resume_devql_tasks(cfg.repo.repo_id.as_str())
        .map(Into::into)
        .map_err(|err| operation_error("BACKEND_ERROR", "task", "resumeTaskQueue", err))
}

pub(super) async fn cancel_task(ctx: &Context<'_>, id: String) -> Result<TaskObject> {
    let context = ctx.data_unchecked::<DevqlGraphqlContext>();
    context
        .require_repo_write_scope()
        .map_err(|err| operation_error("BAD_USER_INPUT", "validation", "cancelTask", err))?;
    let cfg = context
        .devql_config()
        .map_err(|err| operation_error("BACKEND_ERROR", "configuration", "cancelTask", err))?;
    let task = crate::daemon::devql_task(id.as_str())
        .map_err(|err| operation_error("BACKEND_ERROR", "task", "cancelTask", err))?
        .ok_or_else(|| {
            operation_error(
                "BAD_USER_INPUT",
                "validation",
                "cancelTask",
                format!("unknown task `{id}`"),
            )
        })?;
    if task.repo_id != cfg.repo.repo_id {
        return Err(operation_error(
            "BAD_USER_INPUT",
            "validation",
            "cancelTask",
            format!(
                "task `{id}` belongs to repository `{}` and is outside the current repo scope",
                task.repo_id
            ),
        ));
    }

    crate::daemon::cancel_devql_task(id.as_str())
        .map(Into::into)
        .map_err(|err| operation_error("BACKEND_ERROR", "task", "cancelTask", err))
}

fn parse_task_source(
    raw: Option<&str>,
) -> std::result::Result<crate::daemon::DevqlTaskSource, String> {
    match raw.map(str::trim).filter(|value| !value.is_empty()) {
        None => Ok(crate::daemon::DevqlTaskSource::ManualCli),
        Some("init") => Ok(crate::daemon::DevqlTaskSource::Init),
        Some("manual_cli") | Some("manual-cli") | Some("manual") => {
            Ok(crate::daemon::DevqlTaskSource::ManualCli)
        }
        Some("watcher") => Ok(crate::daemon::DevqlTaskSource::Watcher),
        Some("post_commit") | Some("post-commit") => Ok(crate::daemon::DevqlTaskSource::PostCommit),
        Some("post_merge") | Some("post-merge") => Ok(crate::daemon::DevqlTaskSource::PostMerge),
        Some("post_checkout") | Some("post-checkout") => {
            Ok(crate::daemon::DevqlTaskSource::PostCheckout)
        }
        Some(other) => Err(format!(
            "unsupported task source `{other}`; expected one of: init, manual_cli, watcher, post_commit, post_merge, post_checkout"
        )),
    }
}

fn resolve_enqueue_task_input(
    input: EnqueueTaskInput,
    operation: &'static str,
) -> Result<(crate::daemon::DevqlTaskSource, crate::daemon::DevqlTaskSpec)> {
    match input.kind {
        TaskKind::Sync => {
            let sync = input.sync.ok_or_else(|| {
                operation_error(
                    "BAD_USER_INPUT",
                    "validation",
                    operation,
                    "`sync` input is required when kind is SYNC",
                )
            })?;
            if input.ingest.is_some() {
                return Err(operation_error(
                    "BAD_USER_INPUT",
                    "validation",
                    operation,
                    "`ingest` must not be provided when kind is SYNC",
                ));
            }
            let mode = resolve_sync_mode_input(
                sync.full,
                sync.paths,
                sync.repair,
                sync.validate,
                operation,
            )?;
            let source = parse_task_source(sync.source.as_deref())
                .map_err(|err| operation_error("BAD_USER_INPUT", "validation", operation, err))?;
            Ok((
                source,
                crate::daemon::DevqlTaskSpec::Sync(crate::daemon::SyncTaskSpec {
                    mode: match mode {
                        crate::host::devql::SyncMode::Auto => crate::daemon::SyncTaskMode::Auto,
                        crate::host::devql::SyncMode::Full => crate::daemon::SyncTaskMode::Full,
                        crate::host::devql::SyncMode::Paths(paths) => {
                            crate::daemon::SyncTaskMode::Paths { paths }
                        }
                        crate::host::devql::SyncMode::Repair => crate::daemon::SyncTaskMode::Repair,
                        crate::host::devql::SyncMode::Validate => {
                            crate::daemon::SyncTaskMode::Validate
                        }
                    },
                    post_commit_snapshot: None,
                }),
            ))
        }
        TaskKind::Ingest => {
            let ingest = input.ingest.ok_or_else(|| {
                operation_error(
                    "BAD_USER_INPUT",
                    "validation",
                    operation,
                    "`ingest` input is required when kind is INGEST",
                )
            })?;
            if input.sync.is_some() {
                return Err(operation_error(
                    "BAD_USER_INPUT",
                    "validation",
                    operation,
                    "`sync` must not be provided when kind is INGEST",
                ));
            }
            let backfill = match ingest.backfill {
                Some(backfill) if backfill <= 0 => {
                    return Err(operation_error(
                        "BAD_USER_INPUT",
                        "validation",
                        operation,
                        "`backfill` must be greater than zero",
                    ));
                }
                Some(backfill) => Some(usize::try_from(backfill).map_err(|_| {
                    operation_error(
                        "BAD_USER_INPUT",
                        "validation",
                        operation,
                        "`backfill` must be greater than zero",
                    )
                })?),
                None => None,
            };
            Ok((
                crate::daemon::DevqlTaskSource::ManualCli,
                crate::daemon::DevqlTaskSpec::Ingest(crate::daemon::IngestTaskSpec {
                    commits: Vec::new(),
                    backfill,
                }),
            ))
        }
        TaskKind::EmbeddingsBootstrap => Err(operation_error(
            "BAD_USER_INPUT",
            "validation",
            operation,
            "`enqueueTask` does not support EMBEDDINGS_BOOTSTRAP; bootstrap tasks are enqueued internally by the daemon-aware CLI flows",
        )),
        TaskKind::SummaryBootstrap => Err(operation_error(
            "BAD_USER_INPUT",
            "validation",
            operation,
            "`enqueueTask` does not support SUMMARY_BOOTSTRAP; bootstrap tasks are enqueued internally by the daemon-aware CLI flows",
        )),
    }
}

fn resolve_sync_mode_input(
    full: bool,
    paths: Option<Vec<String>>,
    repair: bool,
    validate: bool,
    operation: &'static str,
) -> Result<crate::host::devql::SyncMode> {
    let selected_modes = usize::from(full)
        + usize::from(paths.is_some())
        + usize::from(repair)
        + usize::from(validate);
    if selected_modes > 1 {
        return Err(operation_error(
            "BAD_USER_INPUT",
            "validation",
            operation,
            "at most one of `full`, `paths`, `repair`, or `validate` may be specified",
        ));
    }

    Ok(if validate {
        crate::host::devql::SyncMode::Validate
    } else if repair {
        crate::host::devql::SyncMode::Repair
    } else if let Some(paths) = paths {
        crate::host::devql::SyncMode::Paths(paths)
    } else if full {
        crate::host::devql::SyncMode::Full
    } else {
        crate::host::devql::SyncMode::Auto
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_task_source_accepts_default_and_aliases() {
        assert_eq!(
            parse_task_source(None).expect("default source"),
            crate::daemon::DevqlTaskSource::ManualCli
        );
        assert_eq!(
            parse_task_source(Some("   ")).expect("blank source"),
            crate::daemon::DevqlTaskSource::ManualCli
        );
        assert_eq!(
            parse_task_source(Some("manual")).expect("manual alias"),
            crate::daemon::DevqlTaskSource::ManualCli
        );
        assert_eq!(
            parse_task_source(Some("manual-cli")).expect("manual-cli alias"),
            crate::daemon::DevqlTaskSource::ManualCli
        );
        assert_eq!(
            parse_task_source(Some("init")).expect("init source"),
            crate::daemon::DevqlTaskSource::Init
        );
        assert_eq!(
            parse_task_source(Some("watcher")).expect("watcher source"),
            crate::daemon::DevqlTaskSource::Watcher
        );
        assert_eq!(
            parse_task_source(Some("post-commit")).expect("post-commit source"),
            crate::daemon::DevqlTaskSource::PostCommit
        );
        assert_eq!(
            parse_task_source(Some("post_merge")).expect("post_merge source"),
            crate::daemon::DevqlTaskSource::PostMerge
        );
        assert_eq!(
            parse_task_source(Some("post_checkout")).expect("post_checkout source"),
            crate::daemon::DevqlTaskSource::PostCheckout
        );
    }

    #[test]
    fn parse_task_source_rejects_unknown_values() {
        let err = parse_task_source(Some("cronjob")).expect_err("unknown source should fail");
        assert!(err.contains("unsupported task source `cronjob`"));
        assert!(err.contains("manual_cli"));
    }

    #[test]
    fn resolve_sync_mode_input_defaults_to_auto_when_no_selector_is_set() {
        let mode =
            resolve_sync_mode_input(false, None, false, false, "sync").expect("default mode");
        assert_eq!(mode, crate::host::devql::SyncMode::Auto);
    }

    #[test]
    fn resolve_sync_mode_input_rejects_conflicting_selectors() {
        let err = resolve_sync_mode_input(
            true,
            Some(vec!["src/lib.rs".to_string()]),
            false,
            false,
            "enqueueTask",
        )
        .expect_err("conflicting selectors should fail");
        assert!(
            err.message.contains(
                "at most one of `full`, `paths`, `repair`, or `validate` may be specified"
            )
        );
    }
}
