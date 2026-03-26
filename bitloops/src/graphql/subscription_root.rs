use std::pin::Pin;

use async_graphql::futures_util::{Stream, stream};
use async_graphql::{Context, Subscription};

use super::context::DevqlGraphqlContext;
use super::types::{Checkpoint, IngestionProgressEvent};

#[derive(Default)]
pub struct SubscriptionRoot;

type CheckpointStream = Pin<Box<dyn Stream<Item = Checkpoint> + Send>>;
type IngestionProgressStream = Pin<Box<dyn Stream<Item = IngestionProgressEvent> + Send>>;

#[Subscription]
impl SubscriptionRoot {
    async fn checkpoint_ingested(
        &self,
        ctx: &Context<'_>,
        #[graphql(name = "repoName")] repo_name: String,
    ) -> CheckpointStream {
        let context = ctx.data_unchecked::<DevqlGraphqlContext>();
        let _ = repo_name;
        let receiver = context.subscriptions().subscribe_checkpoints();

        Box::pin(stream::unfold(receiver, |mut receiver| async move {
            loop {
                match receiver.recv().await {
                    Ok(event) => {
                        return Some((event.checkpoint, receiver));
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                        continue;
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => return None,
                }
            }
        }))
    }

    async fn ingestion_progress(
        &self,
        ctx: &Context<'_>,
        #[graphql(name = "repoName")] repo_name: String,
    ) -> IngestionProgressStream {
        let context = ctx.data_unchecked::<DevqlGraphqlContext>();
        let _ = repo_name;
        let receiver = context.subscriptions().subscribe_progress();

        Box::pin(stream::unfold(receiver, |mut receiver| async move {
            loop {
                match receiver.recv().await {
                    Ok(event) => {
                        return Some((event.event, receiver));
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                        continue;
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => return None,
                }
            }
        }))
    }
}
