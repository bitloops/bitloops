use std::pin::Pin;

use async_graphql::futures_util::{Stream, stream};
use async_graphql::{Context, Subscription};

use super::context::DevqlGraphqlContext;
use super::types::{Checkpoint, IngestionProgressEvent, SyncProgressEvent};

#[derive(Default)]
pub struct SubscriptionRoot;

type CheckpointStream = Pin<Box<dyn Stream<Item = Checkpoint> + Send>>;
type IngestionProgressStream = Pin<Box<dyn Stream<Item = IngestionProgressEvent> + Send>>;
type SyncProgressStream = Pin<Box<dyn Stream<Item = SyncProgressEvent> + Send>>;

#[Subscription]
impl SubscriptionRoot {
    async fn checkpoint_ingested(
        &self,
        ctx: &Context<'_>,
        #[graphql(name = "repoName")] repo_name: String,
    ) -> CheckpointStream {
        let context = ctx.data_unchecked::<DevqlGraphqlContext>();
        let receiver = context.subscriptions().subscribe_checkpoints();

        Box::pin(stream::unfold(
            (receiver, repo_name),
            |(mut receiver, repo_name)| async move {
                loop {
                    match receiver.recv().await {
                        Ok(event) => {
                            let event_repo_name = event.repo_name;
                            let checkpoint = event.checkpoint;
                            if event_repo_name != repo_name {
                                continue;
                            }
                            return Some((checkpoint, (receiver, repo_name)));
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                            continue;
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => return None,
                    }
                }
            },
        ))
    }

    async fn ingestion_progress(
        &self,
        ctx: &Context<'_>,
        #[graphql(name = "repoName")] repo_name: String,
    ) -> IngestionProgressStream {
        let context = ctx.data_unchecked::<DevqlGraphqlContext>();
        let receiver = context.subscriptions().subscribe_progress();

        Box::pin(stream::unfold(
            (receiver, repo_name),
            |(mut receiver, repo_name)| async move {
                loop {
                    match receiver.recv().await {
                        Ok(event) => {
                            let event_repo_name = event.repo_name;
                            let progress = event.event;
                            if event_repo_name != repo_name {
                                continue;
                            }
                            return Some((progress, (receiver, repo_name)));
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                            continue;
                        }
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
                            continue;
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => return None,
                    }
                }
            },
        ))
    }
}
