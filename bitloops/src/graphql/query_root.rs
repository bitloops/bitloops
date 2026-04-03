use super::backend_error;
use super::context::DevqlGraphqlContext;
use super::types::{HealthStatus, Repository, SyncTaskObject};
use async_graphql::{Context, Object, Result};

#[derive(Default)]
pub struct QueryRoot;

#[Object]
impl QueryRoot {
    async fn health(&self, ctx: &Context<'_>) -> HealthStatus {
        ctx.data_unchecked::<DevqlGraphqlContext>()
            .health_status()
            .await
    }

    async fn repo(&self, ctx: &Context<'_>, name: String) -> Result<Repository> {
        ctx.data_unchecked::<DevqlGraphqlContext>()
            .repository_for_name(&name)
            .await
            .map_err(|err| backend_error(format!("failed to resolve repository: {err:#}")))
    }

    #[graphql(name = "syncTask")]
    async fn sync_task(&self, _ctx: &Context<'_>, id: String) -> Result<Option<SyncTaskObject>> {
        crate::daemon::sync_task(id.as_str())
            .map(|task| task.map(Into::into))
            .map_err(|err| backend_error(format!("failed to load sync task: {err:#}")))
    }

    #[graphql(name = "syncTasks")]
    async fn sync_tasks(
        &self,
        _ctx: &Context<'_>,
        #[graphql(name = "repoId")] repo_id: Option<String>,
        limit: Option<i32>,
    ) -> Result<Vec<SyncTaskObject>> {
        let limit = limit
            .map(|limit| usize::try_from(limit.max(0)).unwrap_or(usize::MAX))
            .or(Some(25));
        crate::daemon::sync_tasks(repo_id.as_deref(), limit)
            .map(|tasks| tasks.into_iter().map(Into::into).collect())
            .map_err(|err| backend_error(format!("failed to list sync tasks: {err:#}")))
    }
}
