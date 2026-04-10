use super::backend_error;
use super::context::DevqlGraphqlContext;
use super::types::{HealthStatus, Repository, TaskKind, TaskObject, TaskQueueStatusObject, TaskStatus};
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

    async fn task(&self, _ctx: &Context<'_>, id: String) -> Result<Option<TaskObject>> {
        crate::daemon::devql_task(id.as_str())
            .map(|task| task.map(Into::into))
            .map_err(|err| backend_error(format!("failed to load task: {err:#}")))
    }

    async fn tasks(
        &self,
        _ctx: &Context<'_>,
        #[graphql(name = "repoId")] repo_id: Option<String>,
        kind: Option<TaskKind>,
        status: Option<TaskStatus>,
        limit: Option<i32>,
    ) -> Result<Vec<TaskObject>> {
        let limit = limit
            .map(|limit| usize::try_from(limit.max(0)).unwrap_or(usize::MAX))
            .or(Some(25));
        crate::daemon::devql_tasks(
            repo_id.as_deref(),
            kind.map(Into::into),
            status.map(Into::into),
            limit,
        )
            .map(|tasks| tasks.into_iter().map(Into::into).collect())
            .map_err(|err| backend_error(format!("failed to list tasks: {err:#}")))
    }

    #[graphql(name = "taskQueue")]
    async fn task_queue(
        &self,
        _ctx: &Context<'_>,
        #[graphql(name = "repoId")] repo_id: Option<String>,
    ) -> Result<TaskQueueStatusObject> {
        crate::daemon::devql_task_status(repo_id.as_deref())
            .map(Into::into)
            .map_err(|err| backend_error(format!("failed to load task queue status: {err:#}")))
    }
}
