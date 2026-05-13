use std::path::PathBuf;
use std::pin::Pin;

use async_graphql::futures_util::{Stream, stream};
use async_graphql::{Context, ID, Object, Result, SimpleObject, Subscription};

use super::config::{map_runtime_api_error, resolve_runtime_devql_config};
use super::config_management::{
    RuntimeConfigSnapshotObject, RuntimeConfigTargetObject, UpdateRuntimeConfigInput,
    UpdateRuntimeConfigResult, list_config_targets, load_config_snapshot,
    update_config as update_runtime_config,
};
use super::debug::{RuntimeDebugSnapshotObject, load_runtime_debug_snapshot};
use super::events::RuntimeEventObject;
use super::snapshot::RuntimeSnapshotObject;
use super::start_init::{StartInitInput, StartInitResult};
use super::util::{current_unix_timestamp, to_graphql_i64};
use super::watchers::{RuntimeWatcherReconcileResultObject, reconcile_runtime_watcher};
use crate::api::DashboardState;
use crate::graphql::{TaskObject, bad_user_input_error, graphql_error};

#[derive(Debug, Clone, Default)]
pub(crate) struct RuntimeRequestContext {
    pub(crate) bound_repo_root: Option<PathBuf>,
}

#[derive(Default)]
pub(crate) struct RuntimeQueryRoot;

#[Object]
impl RuntimeQueryRoot {
    #[graphql(name = "configTargets")]
    async fn config_targets(&self, ctx: &Context<'_>) -> Result<Vec<RuntimeConfigTargetObject>> {
        list_config_targets(ctx.data_unchecked::<DashboardState>()).await
    }

    #[graphql(name = "configSnapshot")]
    async fn config_snapshot(
        &self,
        ctx: &Context<'_>,
        #[graphql(name = "targetId")] target_id: ID,
    ) -> Result<RuntimeConfigSnapshotObject> {
        load_config_snapshot(ctx.data_unchecked::<DashboardState>(), &target_id).await
    }

    #[graphql(name = "runtimeSnapshot")]
    async fn runtime_snapshot(
        &self,
        ctx: &Context<'_>,
        #[graphql(name = "repoId")] repo_id: String,
    ) -> Result<RuntimeSnapshotObject> {
        let state = ctx.data_unchecked::<DashboardState>();
        let request_context = ctx
            .data_opt::<RuntimeRequestContext>()
            .cloned()
            .unwrap_or_default();
        let cfg = resolve_runtime_devql_config(state, &request_context, repo_id.as_str())
            .await
            .map_err(map_runtime_api_error)?;
        crate::daemon::shared_init_runtime_coordinator()
            .overview_snapshot_for_repo(&cfg)
            .map(|snapshot| RuntimeSnapshotObject::from_overview(cfg, snapshot))
            .map_err(|err| {
                graphql_error(
                    "internal",
                    format!("failed to load runtime snapshot overview: {err:#}"),
                )
            })
    }

    #[graphql(name = "runtimeDebugSnapshot")]
    async fn runtime_debug_snapshot(
        &self,
        ctx: &Context<'_>,
        #[graphql(name = "repoId")] repo_id: String,
    ) -> Result<RuntimeDebugSnapshotObject> {
        let state = ctx.data_unchecked::<DashboardState>();
        let request_context = ctx
            .data_opt::<RuntimeRequestContext>()
            .cloned()
            .unwrap_or_default();
        load_runtime_debug_snapshot(state, request_context, repo_id.as_str()).await
    }
}

#[derive(Default)]
pub(crate) struct RuntimeMutationRoot;

#[derive(Debug, Clone, SimpleObject)]
pub(crate) struct RuntimeTaskEnqueueResultObject {
    pub task: TaskObject,
    pub merged: bool,
}

#[Object]
impl RuntimeMutationRoot {
    #[graphql(name = "updateConfig")]
    async fn update_config(
        &self,
        ctx: &Context<'_>,
        input: UpdateRuntimeConfigInput,
    ) -> Result<UpdateRuntimeConfigResult> {
        update_runtime_config(ctx.data_unchecked::<DashboardState>(), input).await
    }

    #[graphql(name = "startInit")]
    async fn start_init(
        &self,
        ctx: &Context<'_>,
        #[graphql(name = "repoId")] repo_id: String,
        input: StartInitInput,
    ) -> Result<StartInitResult> {
        let state = ctx.data_unchecked::<DashboardState>();
        let request_context = ctx
            .data_opt::<RuntimeRequestContext>()
            .cloned()
            .unwrap_or_default();
        let cfg = resolve_runtime_devql_config(state, &request_context, repo_id.as_str())
            .await
            .map_err(map_runtime_api_error)?;
        let selections = input.into_selections().map_err(bad_user_input_error)?;
        crate::daemon::shared_init_runtime_coordinator()
            .start_session(&cfg, selections)
            .map(|handle| StartInitResult {
                init_session_id: ID::from(handle.init_session_id),
            })
            .map_err(|err| {
                graphql_error("internal", format!("failed to start init session: {err:#}"))
            })
    }

    #[graphql(name = "validateSync")]
    async fn validate_sync(
        &self,
        ctx: &Context<'_>,
        #[graphql(name = "repoId")] repo_id: String,
    ) -> Result<RuntimeTaskEnqueueResultObject> {
        let state = ctx.data_unchecked::<DashboardState>();
        let request_context = ctx
            .data_opt::<RuntimeRequestContext>()
            .cloned()
            .unwrap_or_default();
        let cfg = resolve_runtime_devql_config(state, &request_context, repo_id.as_str())
            .await
            .map_err(map_runtime_api_error)?;

        crate::daemon::shared_devql_task_coordinator()
            .register_subscription_hub(state.subscription_hub());

        crate::daemon::enqueue_sync_for_config(
            &cfg,
            crate::daemon::DevqlTaskSource::ManualCli,
            crate::host::devql::SyncMode::Validate,
        )
        .map(|queued| RuntimeTaskEnqueueResultObject {
            task: queued.task.into(),
            merged: queued.merged,
        })
        .map_err(|err| {
            graphql_error(
                "internal",
                format!("failed to enqueue validate sync task: {err:#}"),
            )
        })
    }

    #[graphql(name = "reconcileWatcher")]
    async fn reconcile_watcher(
        &self,
        ctx: &Context<'_>,
        #[graphql(name = "repoId")] repo_id: String,
    ) -> Result<RuntimeWatcherReconcileResultObject> {
        let state = ctx.data_unchecked::<DashboardState>();
        let request_context = ctx
            .data_opt::<RuntimeRequestContext>()
            .cloned()
            .unwrap_or_default();
        reconcile_runtime_watcher(state, request_context, repo_id.as_str()).await
    }
}

#[derive(Default)]
pub(crate) struct RuntimeSubscriptionRoot;

type RuntimeEventStream = Pin<Box<dyn Stream<Item = RuntimeEventObject> + Send>>;

#[Subscription]
impl RuntimeSubscriptionRoot {
    #[graphql(name = "runtimeEvents")]
    async fn runtime_events(
        &self,
        ctx: &Context<'_>,
        #[graphql(name = "repoId")] repo_id: String,
        #[graphql(name = "initSessionId")] init_session_id: Option<ID>,
    ) -> RuntimeEventStream {
        let receiver = ctx
            .data_unchecked::<DashboardState>()
            .subscription_hub()
            .subscribe_runtime_events();
        let init_session_id = init_session_id.map(|value| value.to_string());

        Box::pin(stream::unfold(
            (receiver, repo_id, init_session_id),
            |(mut receiver, repo_id, init_session_id)| async move {
                loop {
                    match receiver.recv().await {
                        Ok(event) => {
                            if event.repo_id != repo_id {
                                continue;
                            }
                            if init_session_id.as_ref().is_some_and(|session_id| {
                                event.init_session_id.as_deref() != Some(session_id.as_str())
                            }) {
                                continue;
                            }
                            return Some((event.into(), (receiver, repo_id, init_session_id)));
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                            return Some((
                                RuntimeEventObject {
                                    domain: "lagged".to_string(),
                                    repo_id: repo_id.clone(),
                                    init_session_id: init_session_id.clone().map(ID::from),
                                    updated_at_unix: to_graphql_i64(current_unix_timestamp()),
                                    task_id: None,
                                    run_id: None,
                                    mailbox_name: None,
                                },
                                (receiver, repo_id, init_session_id),
                            ));
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => return None,
                    }
                }
            },
        ))
    }
}
