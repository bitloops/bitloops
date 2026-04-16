use std::time::Instant;

use async_graphql::{Context, EmptySubscription, Object, Result, Schema};
use async_graphql_axum::GraphQLRequest;
use axum::{
    extract::State,
    http::HeaderMap,
    response::{IntoResponse, Response as AxumResponse},
};

use super::DashboardState;
use super::dashboard_params::{parse_commit_checkpoint_filter, parse_dashboard_commit_query};
use super::dashboard_service::{
    check_dashboard_bundle_version, fetch_dashboard_bundle, load_dashboard_agents,
    load_dashboard_branches, load_dashboard_checkpoint, load_dashboard_commits,
    load_dashboard_health, load_dashboard_interaction_actors, load_dashboard_interaction_agents,
    load_dashboard_interaction_commit_authors, load_dashboard_interaction_kpis,
    load_dashboard_interaction_session, load_dashboard_interaction_sessions, load_dashboard_kpis,
    load_dashboard_repositories, load_dashboard_users, search_dashboard_interaction_sessions,
    search_dashboard_interaction_turns,
};
use super::dashboard_types::{
    DashboardAgent, DashboardBranchSummary, DashboardBundleVersion, DashboardCheckpointDetail,
    DashboardCommitRow, DashboardFetchBundleResult, DashboardInteractionActorBucket,
    DashboardInteractionAgentBucket, DashboardInteractionCommitAuthorBucket,
    DashboardInteractionFilterInput, DashboardInteractionKpis, DashboardInteractionSearchInput,
    DashboardInteractionSession, DashboardInteractionSessionDetail,
    DashboardInteractionSessionSearchHit, DashboardInteractionTurnSearchHit, DashboardKpis,
    DashboardRepository, DashboardUser,
};
use crate::graphql::{
    GraphqlActionTelemetry, HealthStatus, MAX_DEVQL_QUERY_COMPLEXITY, MAX_DEVQL_QUERY_DEPTH,
    bad_user_input_error, execute_graphql_request, graphql_error, graphql_playground_response,
    graphql_request_signature, track_graphql_action,
};

pub(crate) type DashboardGraphqlSchema =
    Schema<DashboardQueryRoot, DashboardMutationRoot, EmptySubscription>;

#[derive(Default)]
pub(crate) struct DashboardQueryRoot;

#[Object]
impl DashboardQueryRoot {
    async fn health(&self, ctx: &Context<'_>) -> HealthStatus {
        load_dashboard_health(ctx.data_unchecked::<DashboardState>()).await
    }

    async fn repositories(&self, ctx: &Context<'_>) -> Result<Vec<DashboardRepository>> {
        load_dashboard_repositories(ctx.data_unchecked::<DashboardState>())
            .await
            .map_err(map_dashboard_error)
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "The published dashboard GraphQL SDL keeps these filter arguments flat"
    )]
    async fn kpis(
        &self,
        ctx: &Context<'_>,
        #[graphql(name = "repoId")] repo_id: Option<String>,
        branch: String,
        from: Option<String>,
        to: Option<String>,
        user: Option<String>,
        agent: Option<String>,
    ) -> Result<DashboardKpis> {
        let filter = parse_commit_checkpoint_filter(Some(branch), from, to, user, agent)
            .map_err(map_dashboard_error)?;
        load_dashboard_kpis(ctx.data_unchecked::<DashboardState>(), repo_id, filter)
            .await
            .map_err(map_dashboard_error)
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "The published dashboard GraphQL SDL keeps these filter arguments flat"
    )]
    async fn commits(
        &self,
        ctx: &Context<'_>,
        #[graphql(name = "repoId")] repo_id: Option<String>,
        branch: String,
        from: Option<String>,
        to: Option<String>,
        user: Option<String>,
        agent: Option<String>,
        limit: Option<i32>,
        offset: Option<i32>,
    ) -> Result<Vec<DashboardCommitRow>> {
        let filter = parse_dashboard_commit_query(branch, from, to, user, agent, limit, offset)
            .map_err(map_dashboard_error)?;
        load_dashboard_commits(ctx.data_unchecked::<DashboardState>(), repo_id, filter)
            .await
            .map_err(map_dashboard_error)
    }

    async fn branches(
        &self,
        ctx: &Context<'_>,
        #[graphql(name = "repoId")] repo_id: Option<String>,
        from: Option<String>,
        to: Option<String>,
    ) -> Result<Vec<DashboardBranchSummary>> {
        load_dashboard_branches(ctx.data_unchecked::<DashboardState>(), repo_id, from, to)
            .await
            .map_err(map_dashboard_error)
    }

    async fn users(
        &self,
        ctx: &Context<'_>,
        #[graphql(name = "repoId")] repo_id: Option<String>,
        branch: String,
        from: Option<String>,
        to: Option<String>,
        agent: Option<String>,
    ) -> Result<Vec<DashboardUser>> {
        load_dashboard_users(
            ctx.data_unchecked::<DashboardState>(),
            repo_id,
            branch,
            from,
            to,
            agent,
        )
        .await
        .map_err(map_dashboard_error)
    }

    async fn agents(
        &self,
        ctx: &Context<'_>,
        #[graphql(name = "repoId")] repo_id: Option<String>,
        branch: String,
        from: Option<String>,
        to: Option<String>,
        user: Option<String>,
    ) -> Result<Vec<DashboardAgent>> {
        load_dashboard_agents(
            ctx.data_unchecked::<DashboardState>(),
            repo_id,
            branch,
            from,
            to,
            user,
        )
        .await
        .map_err(map_dashboard_error)
    }

    async fn checkpoint(
        &self,
        ctx: &Context<'_>,
        #[graphql(name = "repoId")] repo_id: Option<String>,
        #[graphql(name = "checkpointId")] checkpoint_id: String,
    ) -> Result<DashboardCheckpointDetail> {
        load_dashboard_checkpoint(
            ctx.data_unchecked::<DashboardState>(),
            repo_id,
            checkpoint_id,
        )
        .await
        .map_err(map_dashboard_error)
    }

    #[graphql(name = "interactionKpis")]
    async fn interaction_kpis(
        &self,
        ctx: &Context<'_>,
        #[graphql(name = "repoId")] repo_id: Option<String>,
        filter: Option<DashboardInteractionFilterInput>,
    ) -> Result<DashboardInteractionKpis> {
        load_dashboard_interaction_kpis(ctx.data_unchecked::<DashboardState>(), repo_id, filter)
            .await
            .map_err(map_dashboard_error)
    }

    #[graphql(name = "interactionSessions")]
    async fn interaction_sessions(
        &self,
        ctx: &Context<'_>,
        #[graphql(name = "repoId")] repo_id: Option<String>,
        filter: Option<DashboardInteractionFilterInput>,
        limit: Option<i32>,
        offset: Option<i32>,
    ) -> Result<Vec<DashboardInteractionSession>> {
        load_dashboard_interaction_sessions(
            ctx.data_unchecked::<DashboardState>(),
            repo_id,
            filter,
            limit,
            offset,
        )
        .await
        .map_err(map_dashboard_error)
    }

    #[graphql(name = "interactionSession")]
    async fn interaction_session(
        &self,
        ctx: &Context<'_>,
        #[graphql(name = "sessionId")] session_id: String,
        #[graphql(name = "repoId")] repo_id: Option<String>,
    ) -> Result<DashboardInteractionSessionDetail> {
        load_dashboard_interaction_session(
            ctx.data_unchecked::<DashboardState>(),
            repo_id,
            session_id,
        )
        .await
        .map_err(map_dashboard_error)
    }

    #[graphql(name = "interactionActors")]
    async fn interaction_actors(
        &self,
        ctx: &Context<'_>,
        #[graphql(name = "repoId")] repo_id: Option<String>,
        filter: Option<DashboardInteractionFilterInput>,
    ) -> Result<Vec<DashboardInteractionActorBucket>> {
        load_dashboard_interaction_actors(ctx.data_unchecked::<DashboardState>(), repo_id, filter)
            .await
            .map_err(map_dashboard_error)
    }

    #[graphql(name = "interactionCommitAuthors")]
    async fn interaction_commit_authors(
        &self,
        ctx: &Context<'_>,
        #[graphql(name = "repoId")] repo_id: Option<String>,
        filter: Option<DashboardInteractionFilterInput>,
    ) -> Result<Vec<DashboardInteractionCommitAuthorBucket>> {
        load_dashboard_interaction_commit_authors(
            ctx.data_unchecked::<DashboardState>(),
            repo_id,
            filter,
        )
        .await
        .map_err(map_dashboard_error)
    }

    #[graphql(name = "interactionAgents")]
    async fn interaction_agents(
        &self,
        ctx: &Context<'_>,
        #[graphql(name = "repoId")] repo_id: Option<String>,
        filter: Option<DashboardInteractionFilterInput>,
    ) -> Result<Vec<DashboardInteractionAgentBucket>> {
        load_dashboard_interaction_agents(ctx.data_unchecked::<DashboardState>(), repo_id, filter)
            .await
            .map_err(map_dashboard_error)
    }

    #[graphql(name = "searchInteractionSessions")]
    async fn search_interaction_sessions(
        &self,
        ctx: &Context<'_>,
        #[graphql(name = "repoId")] repo_id: Option<String>,
        input: DashboardInteractionSearchInput,
    ) -> Result<Vec<DashboardInteractionSessionSearchHit>> {
        search_dashboard_interaction_sessions(
            ctx.data_unchecked::<DashboardState>(),
            repo_id,
            input,
        )
        .await
        .map_err(map_dashboard_error)
    }

    #[graphql(name = "searchInteractionTurns")]
    async fn search_interaction_turns(
        &self,
        ctx: &Context<'_>,
        #[graphql(name = "repoId")] repo_id: Option<String>,
        input: DashboardInteractionSearchInput,
    ) -> Result<Vec<DashboardInteractionTurnSearchHit>> {
        search_dashboard_interaction_turns(ctx.data_unchecked::<DashboardState>(), repo_id, input)
            .await
            .map_err(map_dashboard_error)
    }

    #[graphql(name = "checkBundleVersion")]
    async fn check_bundle_version(&self, ctx: &Context<'_>) -> Result<DashboardBundleVersion> {
        check_dashboard_bundle_version(ctx.data_unchecked::<DashboardState>())
            .await
            .map_err(map_dashboard_error)
    }
}

#[derive(Default)]
pub(crate) struct DashboardMutationRoot;

#[Object]
impl DashboardMutationRoot {
    #[graphql(name = "fetchBundle")]
    async fn fetch_bundle(&self, ctx: &Context<'_>) -> Result<DashboardFetchBundleResult> {
        fetch_dashboard_bundle(ctx.data_unchecked::<DashboardState>())
            .await
            .map_err(map_dashboard_error)
    }
}

pub(crate) fn build_dashboard_schema_template() -> DashboardGraphqlSchema {
    Schema::build(DashboardQueryRoot, DashboardMutationRoot, EmptySubscription)
        .limit_depth(MAX_DEVQL_QUERY_DEPTH)
        .limit_complexity(MAX_DEVQL_QUERY_COMPLEXITY)
        .finish()
}

pub fn dashboard_schema_sdl() -> String {
    build_dashboard_schema_template().sdl()
}

pub(crate) async fn dashboard_graphql_handler(
    State(state): State<DashboardState>,
    headers: HeaderMap,
    request: GraphQLRequest,
) -> AxumResponse {
    let started = Instant::now();
    let request = request.into_inner();
    let signature = graphql_request_signature(&request);
    let (response, success) = execute_graphql_request(
        state.dashboard_graphql_schema(),
        request.data(state.clone()),
        &headers,
    )
    .await;
    let response = response.into_response();
    track_graphql_action(GraphqlActionTelemetry {
        repo_root: state.repo_root.as_path(),
        event: "bitloops devql dashboard http",
        scope: "dashboard",
        transport: "http",
        request_kind: &signature.0,
        operation_family: &signature.1,
        success,
        status: response.status(),
        duration: started.elapsed(),
    });
    response
}

pub(crate) async fn dashboard_graphql_playground_handler() -> impl IntoResponse {
    graphql_playground_response("/devql/dashboard", None, "DevQL Dashboard Explorer")
}

pub(crate) async fn dashboard_graphql_sdl_handler(
    State(state): State<DashboardState>,
) -> AxumResponse {
    (
        [("content-type", "text/plain; charset=utf-8")],
        state.dashboard_graphql_schema().sdl(),
    )
        .into_response()
}

fn map_dashboard_error(error: super::ApiError) -> async_graphql::Error {
    match error.code {
        "bad_request" => bad_user_input_error(error.message),
        other => graphql_error(other, error.message),
    }
}
