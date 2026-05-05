mod architecture_graph;
mod code_city;
mod errors;
mod inputs;
mod knowledge;
mod navigation_context;
mod results;
mod schema;
mod task_queue;
mod telemetry;
mod validation;

use async_graphql::{Context, Object, Result};

use super::types::{
    AcceptNavigationContextViewInput, AcceptNavigationContextViewResult,
    ArchitectureGraphAssertionResult, ArchitectureSystemMembershipAssertionResult,
    AssertArchitectureGraphFactInput, AssertArchitectureSystemMembershipInput,
    MaterialiseNavigationContextViewInput, MaterialiseNavigationContextViewResult,
    RevokeArchitectureGraphAssertionResult, TaskObject, TaskQueueControlResultObject,
};

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

    #[graphql(name = "assertArchitectureGraphFact")]
    async fn assert_architecture_graph_fact(
        &self,
        ctx: &Context<'_>,
        input: AssertArchitectureGraphFactInput,
    ) -> Result<ArchitectureGraphAssertionResult> {
        architecture_graph::assert_architecture_graph_fact(ctx, input).await
    }

    #[graphql(name = "revokeArchitectureGraphAssertion")]
    async fn revoke_architecture_graph_assertion(
        &self,
        ctx: &Context<'_>,
        id: String,
    ) -> Result<RevokeArchitectureGraphAssertionResult> {
        architecture_graph::revoke_architecture_graph_assertion(ctx, id).await
    }

    #[graphql(name = "assertArchitectureSystemMembership")]
    async fn assert_architecture_system_membership(
        &self,
        ctx: &Context<'_>,
        input: AssertArchitectureSystemMembershipInput,
    ) -> Result<ArchitectureSystemMembershipAssertionResult> {
        architecture_graph::assert_architecture_system_membership(ctx, input).await
    }

    #[graphql(name = "acceptNavigationContextView")]
    async fn accept_navigation_context_view(
        &self,
        ctx: &Context<'_>,
        input: AcceptNavigationContextViewInput,
    ) -> Result<AcceptNavigationContextViewResult> {
        navigation_context::accept_navigation_context_view_signature(ctx, input).await
    }

    #[graphql(name = "materialiseNavigationContextView")]
    async fn materialise_navigation_context_view(
        &self,
        ctx: &Context<'_>,
        input: MaterialiseNavigationContextViewInput,
    ) -> Result<MaterialiseNavigationContextViewResult> {
        navigation_context::materialise_navigation_context_view_snapshot(ctx, input).await
    }

    async fn apply_migrations(&self, ctx: &Context<'_>) -> Result<ApplyMigrationsMutationResult> {
        schema::apply_migrations(ctx).await
    }
}
