mod code_city;
mod errors;
mod inputs;
mod knowledge;
mod results;
mod schema;
mod task_queue;
mod telemetry;
mod validation;

use async_graphql::{Context, Object, Result};

use super::types::{TaskObject, TaskQueueControlResultObject};

#[allow(unused_imports)]
pub use inputs::{
    AddKnowledgeInput, AssociateKnowledgeInput, CodeCityRefreshInput, EnqueueIngestTaskInput,
    EnqueueSyncTaskInput, EnqueueTaskInput, RefreshKnowledgeInput,
};
#[allow(unused_imports)]
pub use results::{
    AddKnowledgeMutationResult, ApplyMigrationsMutationResult, AssociateKnowledgeMutationResult,
    CodeCityRefreshResultObject, EnqueueTaskResult, IngestResult, InitSchemaResult,
    MigrationRecord, RefreshKnowledgeMutationResult, SyncResult, SyncValidationFileDriftResult,
    SyncValidationResult, UpdateCliTelemetryConsentResult,
};

#[derive(Default)]
pub struct MutationRoot;

#[Object]
impl MutationRoot {
    async fn update_cli_telemetry_consent(
        &self,
        ctx: &Context<'_>,
        cli_version: String,
        telemetry: Option<bool>,
    ) -> Result<UpdateCliTelemetryConsentResult> {
        telemetry::update_cli_telemetry_consent(ctx, cli_version, telemetry).await
    }

    async fn init_schema(&self, ctx: &Context<'_>) -> Result<InitSchemaResult> {
        schema::init_schema(ctx).await
    }

    #[graphql(name = "enqueueTask")]
    async fn enqueue_task(
        &self,
        ctx: &Context<'_>,
        input: EnqueueTaskInput,
    ) -> Result<EnqueueTaskResult> {
        task_queue::enqueue_task(ctx, input).await
    }

    #[graphql(name = "refreshCodeCity")]
    async fn refresh_code_city(
        &self,
        ctx: &Context<'_>,
        input: CodeCityRefreshInput,
    ) -> Result<CodeCityRefreshResultObject> {
        code_city::refresh_code_city(ctx, input).await
    }

    #[graphql(name = "pauseTaskQueue")]
    async fn pause_task_queue(
        &self,
        ctx: &Context<'_>,
        reason: Option<String>,
    ) -> Result<TaskQueueControlResultObject> {
        task_queue::pause_task_queue(ctx, reason).await
    }

    #[graphql(name = "resumeTaskQueue")]
    async fn resume_task_queue(
        &self,
        ctx: &Context<'_>,
        #[graphql(name = "repoId")] repo_id: Option<String>,
    ) -> Result<TaskQueueControlResultObject> {
        task_queue::resume_task_queue(ctx, repo_id).await
    }

    #[graphql(name = "cancelTask")]
    async fn cancel_task(&self, ctx: &Context<'_>, id: String) -> Result<TaskObject> {
        task_queue::cancel_task(ctx, id).await
    }

    async fn add_knowledge(
        &self,
        ctx: &Context<'_>,
        input: AddKnowledgeInput,
    ) -> Result<AddKnowledgeMutationResult> {
        knowledge::add_knowledge(ctx, input).await
    }

    async fn associate_knowledge(
        &self,
        ctx: &Context<'_>,
        input: AssociateKnowledgeInput,
    ) -> Result<AssociateKnowledgeMutationResult> {
        knowledge::associate_knowledge(ctx, input).await
    }

    async fn refresh_knowledge(
        &self,
        ctx: &Context<'_>,
        input: RefreshKnowledgeInput,
    ) -> Result<RefreshKnowledgeMutationResult> {
        knowledge::refresh_knowledge(ctx, input).await
    }

    async fn apply_migrations(&self, ctx: &Context<'_>) -> Result<ApplyMigrationsMutationResult> {
        schema::apply_migrations(ctx).await
    }
}
