use super::*;

fn slim_scope_headers(repo_root: &Path) -> Vec<(String, String)> {
    let repo = crate::host::devql::resolve_repo_identity(repo_root).expect("resolve repo identity");
    vec![
        (
            crate::devql_transport::HEADER_SCOPE_REPO_ID.to_string(),
            repo.repo_id,
        ),
        (
            crate::devql_transport::HEADER_SCOPE_REPO_NAME.to_string(),
            repo.name,
        ),
        (
            crate::devql_transport::HEADER_SCOPE_REPO_PROVIDER.to_string(),
            repo.provider,
        ),
        (
            crate::devql_transport::HEADER_SCOPE_REPO_ORGANISATION.to_string(),
            repo.organization,
        ),
        (
            crate::devql_transport::HEADER_SCOPE_REPO_IDENTITY.to_string(),
            repo.identity,
        ),
        (
            crate::devql_transport::HEADER_SCOPE_REPO_ROOT.to_string(),
            repo_root.to_string_lossy().to_string(),
        ),
        (
            crate::devql_transport::HEADER_SCOPE_BRANCH.to_string(),
            "main".to_string(),
        ),
        (
            crate::devql_transport::HEADER_SCOPE_GIT_DIR_RELATIVE_PATH.to_string(),
            ".git".to_string(),
        ),
    ]
}

#[tokio::test]
async fn devql_playground_route_serves_explorer() {
    let temp = TempDir::new().expect("temp dir");
    let app = build_dashboard_router(test_state(
        temp.path().to_path_buf(),
        ServeMode::HelloWorld,
        temp.path().to_path_buf(),
    ));

    let (status, body) = request_text(app, "/devql/playground").await;

    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("DevQL Slim Explorer"));
    assert!(body.contains("/devql"));
}

#[tokio::test]
async fn devql_sdl_route_returns_schema_text() {
    let temp = TempDir::new().expect("temp dir");
    let app = build_dashboard_router(test_state(
        temp.path().to_path_buf(),
        ServeMode::HelloWorld,
        temp.path().to_path_buf(),
    ));

    let (status, body) = request_text(app, "/devql/sdl").await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, crate::graphql::slim_schema_sdl());
    assert!(body.contains("health: HealthStatus!"));
    assert!(body.contains("type SlimQueryRoot"));
    assert!(body.contains("type MutationRoot"));
    assert!(body.contains("defaultBranch: String!"));
    assert!(body.contains("checkpoints(agent: String, since: DateTime"));
    assert!(body.contains("telemetry(eventType: String, agent: String"));
    assert!(body.contains("knowledge(provider: KnowledgeProvider"));
    assert!(body.contains("clones(filter:"));
    assert!(body.contains("chatHistory"));
    assert!(body.contains("asOf(input: AsOfInput!): TemporalScope!"));
    assert!(!body.contains("repo(name: String!): Repository!"));
}

#[tokio::test]
async fn devql_global_routes_serve_full_schema_and_playground() {
    let temp = TempDir::new().expect("temp dir");
    let app = build_dashboard_router(test_state(
        temp.path().to_path_buf(),
        ServeMode::HelloWorld,
        temp.path().to_path_buf(),
    ));

    let (playground_status, playground_body) = request_text(app.clone(), "/devql/global/playground").await;
    assert_eq!(playground_status, StatusCode::OK);
    assert!(playground_body.contains("DevQL Global Explorer"));
    assert!(playground_body.contains("/devql/global"));

    let (sdl_status, sdl_body) = request_text(app, "/devql/global/sdl").await;
    assert_eq!(sdl_status, StatusCode::OK);
    assert_eq!(sdl_body, crate::graphql::schema_sdl());
    assert!(sdl_body.contains("type QueryRoot"));
    assert!(sdl_body.contains("repo(name: String!): Repository!"));
    assert!(sdl_body.contains("branch(name: String!): Repository!"));
    assert!(sdl_body.contains("project(path: String!): Project!"));
}

#[test]
fn checked_in_schema_file_matches_runtime_sdl() {
    let expected = crate::graphql::schema_sdl();
    let schema_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("schema.graphql");
    let actual = fs::read_to_string(&schema_path).expect("read checked-in schema.graphql");
    assert_eq!(actual, expected);
}

#[test]
fn checked_in_slim_schema_file_matches_runtime_sdl() {
    let expected = crate::graphql::slim_schema_sdl();
    let schema_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("schema.slim.graphql");
    let actual = fs::read_to_string(&schema_path).expect("read checked-in schema.slim.graphql");
    assert_eq!(actual, expected);
}

fn graphql_parent_depth_limit_query(parent_depth: usize) -> String {
    let mut query = String::from(
        r#"{ repo(name: "demo") { file(path: "src/caller.ts") { artefacts(first: 1) { edges { node {"#,
    );
    for _ in 0..parent_depth {
        query.push_str(" parent {");
    }
    query.push_str(" id");
    for _ in 0..parent_depth {
        query.push_str(" }");
    }
    query.push_str(" } } } } } }");
    query
}

fn graphql_complexity_limit_query(alias_count: usize) -> String {
    let mut query = String::from("{");
    for index in 0..alias_count {
        write!(
            &mut query,
            r#" q{index}: health {{ relational {{ backend connected }} }}"#
        )
        .expect("writing GraphQL query should succeed");
    }
    query.push('}');
    query
}

#[tokio::test]
async fn devql_graphql_rejects_queries_over_the_depth_limit() {
    let repo = seed_graphql_devql_repo();
    let schema = crate::graphql::build_schema(crate::graphql::DevqlGraphqlContext::new(
        repo.path().to_path_buf(),
        super::super::db::DashboardDbPools::default(),
    ));
    let query = graphql_parent_depth_limit_query(crate::graphql::MAX_DEVQL_QUERY_DEPTH);

    let response = schema.execute(async_graphql::Request::new(query)).await;

    assert_eq!(response.errors.len(), 1, "expected one graphql error");
    assert!(
        response.errors[0]
            .message
            .to_ascii_lowercase()
            .contains("nested too deep"),
        "expected depth-limit error, got {:?}",
        response.errors
    );
}

#[tokio::test]
async fn devql_graphql_rejects_queries_over_the_complexity_limit() {
    let temp = TempDir::new().expect("temp dir");
    let schema = crate::graphql::build_schema(crate::graphql::DevqlGraphqlContext::new(
        temp.path().to_path_buf(),
        super::super::db::DashboardDbPools::default(),
    ));
    let query = graphql_complexity_limit_query(
        crate::graphql::MAX_DEVQL_QUERY_COMPLEXITY.saturating_add(1),
    );

    let response = schema.execute(async_graphql::Request::new(query)).await;

    assert_eq!(response.errors.len(), 1, "expected one graphql error");
    assert!(
        response.errors[0]
            .message
            .to_ascii_lowercase()
            .contains("too complex"),
        "expected complexity-limit error, got {:?}",
        response.errors
    );
}

#[tokio::test]
async fn devql_post_route_executes_graphql_requests() {
    let repo = seed_graphql_devql_repo();
    let app = build_dashboard_router(test_state(
        repo.path().to_path_buf(),
        ServeMode::HelloWorld,
        repo.path().to_path_buf(),
    ));

    let (global_status, global_payload) = request_json_with_method_and_content_type(
        app.clone(),
        Method::POST,
        "/devql/global",
        "application/json",
        Body::from(
            r#"{"query":"{ repo(name: \"demo\") { name provider } health { blob { backend connected } } }"}"#,
        ),
    )
    .await;

    assert_eq!(global_status, StatusCode::OK);
    assert_eq!(global_payload["data"]["repo"]["name"], "demo");
    assert_eq!(global_payload["data"]["repo"]["provider"], "local");
    assert_eq!(global_payload["data"]["health"]["blob"]["backend"], "local");
    assert_eq!(global_payload["data"]["health"]["blob"]["connected"], true);

    let slim_headers = slim_scope_headers(repo.path());
    let slim_headers_ref = slim_headers
        .iter()
        .map(|(name, value)| (name.as_str(), value.as_str()))
        .collect::<Vec<_>>();
    let (slim_status, slim_payload) = request_json_with_method_content_type_and_headers(
        app,
        Method::POST,
        "/devql",
        "application/json",
        &slim_headers_ref,
        Body::from(
            r#"{"query":"{ defaultBranch health { blob { backend connected } } }"}"#,
        ),
    )
    .await;

    assert_eq!(slim_status, StatusCode::OK);
    assert_eq!(slim_payload["data"]["defaultBranch"], "main");
    assert_eq!(slim_payload["data"]["health"]["blob"]["backend"], "local");
    assert_eq!(slim_payload["data"]["health"]["blob"]["connected"], true);
}

#[tokio::test]
async fn devql_ws_route_is_registered() {
    let temp = TempDir::new().expect("temp dir");
    let app = build_dashboard_router(test_state(
        temp.path().to_path_buf(),
        ServeMode::HelloWorld,
        temp.path().to_path_buf(),
    ));

    let (status, _) = request_text_with_method(app, Method::GET, "/devql/ws").await;

    assert_ne!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn devql_graphql_checkpoint_ingested_subscription_receives_published_checkpoint_events() {
    let repo = seed_dashboard_repo();
    let context = crate::graphql::DevqlGraphqlContext::new(
        repo.path().to_path_buf(),
        super::super::db::DashboardDbPools::default(),
    );
    let schema = crate::graphql::build_schema(context.clone());
    let (event_tx, mut event_rx) = tokio::sync::mpsc::unbounded_channel();
    let stream = schema.execute_stream(async_graphql::Request::new(
        r#"
        subscription {
          checkpointIngested(repoName: "demo") {
            id
            commitSha
            sessionId
          }
        }
        "#,
    ));
    let _collector = tokio::spawn(async move {
        let mut stream = stream;
        if let Some(event) = stream.next().await {
            let _ = event_tx.send(event);
        }
    });
    tokio::task::yield_now().await;

    let checkpoint = crate::host::checkpoints::strategy::manual_commit::read_committed_info(
        repo.path(),
        "aabbccddeeff",
    )
    .expect("read committed checkpoint")
    .expect("seeded checkpoint info");
    let mut other_repo_checkpoint =
        crate::graphql::Checkpoint::from_ingested(&checkpoint, Some("wrong-repo-sha"));
    other_repo_checkpoint.session_id = "other-repo-session".to_string();
    context
        .subscriptions()
        .publish_checkpoint("other-repo", other_repo_checkpoint);
    context.subscriptions().publish_checkpoint(
        "demo",
        crate::graphql::Checkpoint::from_ingested(
            &checkpoint,
            Some(&git_ok(repo.path(), &["rev-parse", "HEAD"])),
        ),
    );

    let first_event = tokio::time::timeout(std::time::Duration::from_secs(5), event_rx.recv())
        .await
        .expect("checkpoint subscription event should arrive")
        .expect("checkpoint subscription response");
    assert!(
        first_event.errors.is_empty(),
        "subscription errors: {:?}",
        first_event.errors
    );

    let event_json = first_event
        .data
        .into_json()
        .expect("subscription data to json");
    assert_eq!(
        event_json["checkpointIngested"]["commitSha"],
        Value::String(git_ok(repo.path(), &["rev-parse", "HEAD"]))
    );
    assert!(
        event_json["checkpointIngested"]["id"]
            .as_str()
            .is_some_and(|value| !value.trim().is_empty())
    );
    assert!(event_json["checkpointIngested"]["sessionId"].is_string());
}

#[tokio::test]
async fn devql_graphql_ingestion_progress_subscription_receives_published_progress_events() {
    let repo = seed_dashboard_repo();
    let context = crate::graphql::DevqlGraphqlContext::new(
        repo.path().to_path_buf(),
        super::super::db::DashboardDbPools::default(),
    );
    let schema = crate::graphql::build_schema(context.clone());
    let (event_tx, mut event_rx) = tokio::sync::mpsc::unbounded_channel();
    let stream = schema.execute_stream(async_graphql::Request::new(
        r#"
        subscription {
          ingestionProgress(repoName: "demo") {
            phase
            checkpointsTotal
            checkpointsProcessed
            currentCheckpointId
            currentCommitSha
          }
        }
        "#,
    ));
    let _collector = tokio::spawn(async move {
        let mut stream = stream;
        while let Some(event) = stream.next().await {
            if event_tx.send(event).is_err() {
                break;
            }
        }
    });
    tokio::task::yield_now().await;

    context.subscriptions().publish_progress(
        "other-repo",
        crate::graphql::IngestionProgressEvent {
            phase: crate::graphql::IngestionPhase::Failed,
            checkpoints_total: 99,
            checkpoints_processed: 13,
            current_checkpoint_id: Some("wrong-repo-checkpoint".to_string()),
            current_commit_sha: Some("wrong-repo-sha".to_string()),
            events_inserted: 8,
            artefacts_upserted: 5,
            checkpoints_without_commit: 3,
            temporary_rows_promoted: 2,
        },
    );
    context.subscriptions().publish_progress(
        "demo",
        crate::graphql::IngestionProgressEvent {
            phase: crate::graphql::IngestionPhase::Initializing,
            checkpoints_total: 1,
            checkpoints_processed: 0,
            current_checkpoint_id: None,
            current_commit_sha: None,
            events_inserted: 0,
            artefacts_upserted: 0,
            checkpoints_without_commit: 0,
            temporary_rows_promoted: 0,
        },
    );
    context.subscriptions().publish_progress(
        "demo",
        crate::graphql::IngestionProgressEvent {
            phase: crate::graphql::IngestionPhase::Complete,
            checkpoints_total: 1,
            checkpoints_processed: 1,
            current_checkpoint_id: Some("aabbccddeeff".to_string()),
            current_commit_sha: Some(git_ok(repo.path(), &["rev-parse", "HEAD"])),
            events_inserted: 1,
            artefacts_upserted: 2,
            checkpoints_without_commit: 0,
            temporary_rows_promoted: 0,
        },
    );

    let mut phases = Vec::new();
    let mut last_payload = Value::Null;
    for _ in 0..2 {
        let event = tokio::time::timeout(std::time::Duration::from_secs(5), event_rx.recv())
            .await
            .expect("progress subscription should emit")
            .expect("progress response");
        assert!(
            event.errors.is_empty(),
            "subscription errors: {:?}",
            event.errors
        );
        let json = event.data.into_json().expect("progress event json");
        let payload = json["ingestionProgress"].clone();
        let phase = payload["phase"]
            .as_str()
            .expect("progress phase string")
            .to_string();
        phases.push(phase.clone());
        last_payload = payload;
        if phase == "COMPLETE" {
            break;
        }
    }
    assert!(phases.iter().any(|phase| phase == "INITIALIZING"));
    assert_eq!(phases.last(), Some(&"COMPLETE".to_string()));
    assert_eq!(last_payload["checkpointsTotal"], Value::from(1));
    assert_eq!(last_payload["checkpointsProcessed"], Value::from(1));
}

#[tokio::test]
async fn devql_ingest_mutation_publishes_progress_and_checkpoint_events_to_subscription_hub() {
    let repo = seed_dashboard_repo();
    let _guard = enter_process_state(
        Some(repo.path()),
        &[
            ("BITLOOPS_DEVQL_EMBEDDING_PROVIDER", Some("disabled")),
            ("BITLOOPS_DEVQL_SEMANTIC_PROVIDER", Some("disabled")),
        ],
    );
    write_envelope_config(
        repo.path(),
        json!({
            "stores": {
                "relational": {
                    "sqlite_path": ".bitloops/stores/subscriptions.sqlite"
                },
                "events": {
                    "duckdb_path": ".bitloops/stores/subscriptions.duckdb"
                },
                "embedding_provider": "disabled"
            },
            "semantic": {
                "provider": "disabled"
            }
        }),
    );
    let context = crate::graphql::DevqlGraphqlContext::for_slim_request(
        repo.path().to_path_buf(),
        repo.path().to_path_buf(),
        Some("main".to_string()),
        None,
        None,
        true,
        super::super::db::DashboardDbPools::default(),
    );
    let mut progress_rx = context.subscriptions().subscribe_progress();
    let mut checkpoint_rx = context.subscriptions().subscribe_checkpoints();
    let schema = crate::graphql::build_slim_schema(context);

    let response = schema
        .execute(async_graphql::Request::new(
            r#"
            mutation {
              ingest(input: { init: true, maxCheckpoints: 1 }) {
                success
                checkpointsProcessed
              }
            }
            "#,
        ))
        .await;

    assert!(
        response.errors.is_empty(),
        "graphql errors: {:?}",
        response.errors
    );
    let response_json = response.data.into_json().expect("mutation data to json");
    if response_json["ingest"]["checkpointsProcessed"].as_i64() == Some(1) {
        let checkpoint =
            tokio::time::timeout(std::time::Duration::from_secs(5), checkpoint_rx.recv())
                .await
                .expect("checkpoint event should arrive")
                .expect("checkpoint subscription payload");
        assert_eq!(
            checkpoint.checkpoint.commit_sha,
            Some(git_ok(repo.path(), &["rev-parse", "HEAD"]))
        );
    }

    let mut saw_complete = false;
    for _ in 0..8 {
        let progress = tokio::time::timeout(std::time::Duration::from_secs(5), progress_rx.recv())
            .await
            .expect("progress event should arrive")
            .expect("progress subscription payload");
        if progress.event.phase == crate::graphql::IngestionPhase::Complete {
            saw_complete = true;
            break;
        }
    }
    assert!(saw_complete, "expected a COMPLETE ingestion progress event");
}
