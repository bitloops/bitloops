use std::pin::Pin;

use async_graphql::futures_util::{Stream, stream};
use async_graphql::{Context, Subscription};

use super::context::DevqlGraphqlContext;
use super::types::{Checkpoint, IngestionProgressEvent, SyncProgressEvent};

#[derive(Default)]
pub struct SlimSubscriptionRoot;

type CheckpointStream = Pin<Box<dyn Stream<Item = Checkpoint> + Send>>;
type IngestionProgressStream = Pin<Box<dyn Stream<Item = IngestionProgressEvent> + Send>>;
type SyncProgressStream = Pin<Box<dyn Stream<Item = SyncProgressEvent> + Send>>;

#[Subscription]
impl SlimSubscriptionRoot {
    async fn checkpoint_ingested(&self, ctx: &Context<'_>) -> CheckpointStream {
        let context = ctx.data_unchecked::<DevqlGraphqlContext>();
        let receiver = context.subscriptions().subscribe_checkpoints();
        let repo_name = context.repo_name_for_scope(&context.slim_root_scope()).ok();

        Box::pin(stream::unfold(
            (receiver, repo_name),
            |(mut receiver, repo_name)| async move {
                let repo_name = repo_name.clone()?;
                loop {
                    match receiver.recv().await {
                        Ok(event) => {
                            if event.repo_name != repo_name {
                                continue;
                            }
                            return Some((event.checkpoint, (receiver, Some(repo_name))));
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => return None,
                    }
                }
            },
        ))
    }

    async fn ingestion_progress(&self, ctx: &Context<'_>) -> IngestionProgressStream {
        let context = ctx.data_unchecked::<DevqlGraphqlContext>();
        let receiver = context.subscriptions().subscribe_progress();
        let repo_name = context.repo_name_for_scope(&context.slim_root_scope()).ok();

        Box::pin(stream::unfold(
            (receiver, repo_name),
            |(mut receiver, repo_name)| async move {
                let repo_name = repo_name.clone()?;
                loop {
                    match receiver.recv().await {
                        Ok(event) => {
                            if event.repo_name != repo_name {
                                continue;
                            }
                            return Some((event.event, (receiver, Some(repo_name))));
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => return None,
                    }
                }
            },
        ))
    }

    async fn sync_progress(
        &self,
        ctx: &Context<'_>,
        #[graphql(name = "taskId")] task_id: String,
    ) -> SyncProgressStream {
        let context = ctx.data_unchecked::<DevqlGraphqlContext>();
        let receiver = context.subscriptions().subscribe_sync_progress();

        Box::pin(stream::unfold(
            (receiver, task_id),
            |(mut receiver, task_id)| async move {
                loop {
                    match receiver.recv().await {
                        Ok(event) => {
                            if event.task_id != task_id {
                                continue;
                            }
                            return Some((event.task.into(), (receiver, task_id)));
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                            if let Ok(Some(task)) = crate::daemon::sync_task(task_id.as_str()) {
                                return Some((task.into(), (receiver, task_id)));
                            }
                            continue;
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => return None,
                    }
                }
            },
        ))
    }
}
