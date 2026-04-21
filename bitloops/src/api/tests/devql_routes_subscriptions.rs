use super::*;

#[allow(dead_code)]
fn write_current_repo_runtime_state(repo_root: &Path) {
    let runtime_path = crate::daemon::repo_local_runtime_state_path_for_tests(repo_root)
        .unwrap_or_else(|| crate::daemon::runtime_state_path(repo_root));
    let runtime_state = crate::daemon::DaemonRuntimeState {
        version: 1,
        config_path: repo_root.join(crate::config::BITLOOPS_CONFIG_RELATIVE_PATH),
        config_root: repo_root.to_path_buf(),
        pid: std::process::id(),
        mode: crate::daemon::DaemonMode::Detached,
        service_name: None,
        url: "http://127.0.0.1:5667".to_string(),
        host: "127.0.0.1".to_string(),
        port: 5667,
        bundle_dir: repo_root.join("bundle"),
        relational_db_path: repo_root.join("relational.db"),
        events_db_path: repo_root.join("events.duckdb"),
        blob_store_path: repo_root.join("blob"),
        repo_registry_path: repo_root.join("repo-registry.json"),
        binary_fingerprint: crate::daemon::current_binary_fingerprint().unwrap_or_default(),
        updated_at_unix: 0,
    };
    fs::create_dir_all(
        runtime_path
            .parent()
            .expect("runtime state should have a parent directory"),
    )
    .expect("create runtime state parent");
    let mut bytes = serde_json::to_vec_pretty(&runtime_state).expect("serialise runtime state");
    bytes.push(b'\n');
    fs::write(&runtime_path, bytes).expect("write runtime state");
}

fn slim_scope_headers(repo_root: &Path) -> Vec<(String, String)> {
    let repo = crate::host::devql::resolve_repo_identity(repo_root).expect("resolve repo identity");
    let fingerprint = crate::config::discover_repo_policy_optional(repo_root)
        .expect("discover repo policy")
        .fingerprint;
    vec![
        (
            crate::devql_transport::HEADER_SCOPE_REPO_ID.to_string(),
            repo.repo_id,
        ),
        (
            crate::devql_transport::HEADER_SCOPE_REPO_NAME.to_string(),
            crate::devql_transport::encode_scope_header_value(&repo.name),
        ),
        (
            crate::devql_transport::HEADER_SCOPE_REPO_PROVIDER.to_string(),
            repo.provider,
        ),
        (
            crate::devql_transport::HEADER_SCOPE_REPO_ORGANISATION.to_string(),
            crate::devql_transport::encode_scope_header_value(&repo.organization),
        ),
        (
            crate::devql_transport::HEADER_SCOPE_REPO_IDENTITY.to_string(),
            crate::devql_transport::encode_scope_header_value(&repo.identity),
        ),
        (
            crate::devql_transport::HEADER_SCOPE_REPO_ROOT.to_string(),
            crate::devql_transport::encode_scope_header_value(&repo_root.to_string_lossy()),
        ),
        (
            crate::devql_transport::HEADER_SCOPE_BRANCH.to_string(),
            crate::devql_transport::encode_scope_header_value("main"),
        ),
        (
            crate::devql_transport::HEADER_SCOPE_GIT_DIR_RELATIVE_PATH.to_string(),
            crate::devql_transport::encode_scope_header_value(".git"),
        ),
        (
            crate::devql_transport::HEADER_SCOPE_CONFIG_FINGERPRINT.to_string(),
            fingerprint,
        ),
        (
            crate::devql_transport::HEADER_DAEMON_BINDING.to_string(),
            crate::devql_transport::daemon_binding_identifier_for_config_path(
                &repo_root.join(crate::config::BITLOOPS_CONFIG_RELATIVE_PATH),
            ),
        ),
    ]
}

fn runtime_binding_headers(repo_root: &Path) -> Vec<(String, String)> {
    vec![
        (
            crate::devql_transport::HEADER_SCOPE_REPO_ROOT.to_string(),
            crate::devql_transport::encode_scope_header_value(&repo_root.to_string_lossy()),
        ),
        (
            crate::devql_transport::HEADER_DAEMON_BINDING.to_string(),
            crate::devql_transport::daemon_binding_identifier_for_config_path(
                &repo_root.join(crate::config::BITLOOPS_CONFIG_RELATIVE_PATH),
            ),
        ),
    ]
}

async fn request_slim_query(
    app: axum::Router,
    repo_root: &Path,
    query: &str,
) -> (StatusCode, Value) {
    let slim_headers = slim_scope_headers(repo_root);
    let slim_headers_ref = slim_headers
        .iter()
        .map(|(name, value)| (name.as_str(), value.as_str()))
        .collect::<Vec<_>>();

    request_json_with_method_content_type_and_headers(
        app,
        Method::POST,
        "/devql",
        "application/json",
        &slim_headers_ref,
        Body::from(json!({ "query": query }).to_string()),
    )
    .await
}

#[tokio::test]
async fn devql_slim_route_rejects_missing_daemon_binding_for_repo_scoped_requests() {
    let temp = TempDir::new().expect("temp dir");
    let app = build_dashboard_router(test_state(
        temp.path().to_path_buf(),
        ServeMode::HelloWorld,
        temp.path().to_path_buf(),
    ));

    let slim_headers = slim_scope_headers(temp.path())
        .into_iter()
        .filter(|(name, _)| name != crate::devql_transport::HEADER_DAEMON_BINDING)
        .collect::<Vec<_>>();
    let slim_headers_ref = slim_headers
        .iter()
        .map(|(name, value)| (name.as_str(), value.as_str()))
        .collect::<Vec<_>>();

    let (_status, body) = request_json_with_method_content_type_and_headers(
        app,
        Method::POST,
        "/devql",
        "application/json",
        &slim_headers_ref,
        Body::from(json!({ "query": "{ health { relational { backend } } }" }).to_string()),
    )
    .await;

    assert!(
        body["errors"][0]["message"]
            .as_str()
            .is_some_and(|message| message.contains("Run `bitloops init`")),
        "unexpected response body: {body}"
    );
}

#[tokio::test]
async fn devql_global_route_rejects_mismatched_daemon_binding_for_repo_scoped_requests() {
    let temp = TempDir::new().expect("temp dir");
    let app = build_dashboard_router(test_state(
        temp.path().to_path_buf(),
        ServeMode::HelloWorld,
        temp.path().to_path_buf(),
    ));
    let repo_root = temp.path().to_string_lossy().to_string();
    let headers = [
        (
            crate::devql_transport::HEADER_SCOPE_REPO_ROOT,
            crate::devql_transport::encode_scope_header_value(&repo_root),
        ),
        (
            crate::devql_transport::HEADER_DAEMON_BINDING,
            "mismatched-binding".to_string(),
        ),
    ];
    let headers_ref = headers
        .iter()
        .map(|(name, value)| (*name, value.as_str()))
        .collect::<Vec<_>>();

    let (_status, body) = request_json_with_method_content_type_and_headers(
        app,
        Method::POST,
        "/devql/global",
        "application/json",
        &headers_ref,
        Body::from(json!({ "query": "{ health { relational { backend } } }" }).to_string()),
    )
    .await;

    assert!(
        body["errors"][0]["message"]
            .as_str()
            .is_some_and(|message| message.contains("Run `bitloops init`")),
        "unexpected response body: {body}"
    );
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
    assert!(body.contains("interactionSessions("));
    assert!(body.contains("interactionTurns("));
    assert!(body.contains("interactionEvents("));
    assert!(body.contains("searchInteractionSessions(input: InteractionSearchInputObject!)"));
    assert!(body.contains("searchInteractionTurns(input: InteractionSearchInputObject!)"));
    assert!(body.contains("chatHistory"));
    assert!(body.contains("selectArtefacts(by: ArtefactSelectorInput!): ArtefactSelection!"));
    assert!(body.contains("fuzzyName: String"));
    assert!(body.contains("semanticQuery: String"));
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

    let (playground_status, playground_body) =
        request_text(app.clone(), "/devql/global/playground").await;
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
    assert!(sdl_body.contains("interactionSessions("));
    assert!(sdl_body.contains("interactionTurns("));
    assert!(sdl_body.contains("interactionEvents("));
    assert!(sdl_body.contains("searchInteractionSessions(input: InteractionSearchInputObject!)"));
    assert!(sdl_body.contains("searchInteractionTurns(input: InteractionSearchInputObject!)"));
    assert!(!sdl_body.contains("selectArtefacts(by: ArtefactSelectorInput!)"));
}

#[tokio::test]
async fn devql_runtime_routes_serve_runtime_schema_and_playground() {
    let temp = TempDir::new().expect("temp dir");
    let app = build_dashboard_router(test_state(
        temp.path().to_path_buf(),
        ServeMode::HelloWorld,
        temp.path().to_path_buf(),
    ));

    let (playground_status, playground_body) =
        request_text(app.clone(), "/devql/runtime/playground").await;
    assert_eq!(playground_status, StatusCode::OK);
    assert!(playground_body.contains("DevQL Runtime Explorer"));
    assert!(playground_body.contains("/devql/runtime"));

    let (sdl_status, sdl_body) = request_text(app, "/devql/runtime/sdl").await;
    assert_eq!(sdl_status, StatusCode::OK);
    assert_eq!(sdl_body, crate::api::runtime_schema_sdl());
    assert!(sdl_body.contains("type RuntimeQueryRoot"));
    assert!(sdl_body.contains("runtimeSnapshot(repoId: String!): RuntimeSnapshotObject!"));
    assert!(
        sdl_body.contains("startInit(repoId: String!, input: StartInitInput!): StartInitResult!")
    );
    assert!(
        sdl_body.contains("runtimeEvents(repoId: String!, initSessionId: ID): RuntimeEventObject!")
    );

    let slim_sdl = crate::graphql::slim_schema_sdl();
    assert!(!slim_sdl.contains("runtimeSnapshot("));
    assert!(!slim_sdl.contains("startInit("));
    assert!(!slim_sdl.contains("runtimeEvents("));

    let global_sdl = crate::graphql::schema_sdl();
    assert!(!global_sdl.contains("runtimeSnapshot("));
    assert!(!global_sdl.contains("startInit("));
    assert!(!global_sdl.contains("runtimeEvents("));
}

#[tokio::test]
async fn devql_runtime_route_executes_start_init_mutations() {
    let repo = seed_dashboard_repo();
    let repo_id = crate::host::devql::resolve_repo_identity(repo.path())
        .expect("resolve repo identity")
        .repo_id;
    let app = build_dashboard_router(test_state(
        repo.path().to_path_buf(),
        ServeMode::HelloWorld,
        repo.path().to_path_buf(),
    ));

    let (status, payload) = request_json_with_method_and_content_type(
        app,
        Method::POST,
        "/devql/runtime",
        "application/json",
        Body::from(
            json!({
                "query": "mutation StartInit($repoId: String!, $input: StartInitInput!) { startInit(repoId: $repoId, input: $input) { initSessionId } }",
                "variables": {
                    "repoId": repo_id,
                    "input": {
                        "runSync": false,
                        "runIngest": false,
                        "runCodeEmbeddings": false,
                        "runSummaries": false,
                        "runSummaryEmbeddings": false,
                        "ingestBackfill": serde_json::Value::Null,
                        "embeddingsBootstrap": serde_json::Value::Null,
                        "summariesBootstrap": serde_json::Value::Null,
                    }
                }
            })
            .to_string(),
        ),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert!(
        payload.get("errors").is_none(),
        "runtime graphql errors: {:?}",
        payload.get("errors")
    );
    let init_session_id = payload["data"]["startInit"]["initSessionId"]
        .as_str()
        .expect("init session id");
    assert!(init_session_id.starts_with("init-session-"));
}

#[test]
fn devql_runtime_route_accepts_the_runtime_snapshot_query_used_by_init() {
    let repo = seed_dashboard_repo();
    let repo_id = crate::host::devql::resolve_repo_identity(repo.path())
        .expect("resolve repo identity")
        .repo_id;
    let config_path = repo
        .path()
        .join(crate::config::BITLOOPS_CONFIG_RELATIVE_PATH);
    let config_path_string = config_path.to_string_lossy().to_string();

    crate::test_support::process_state::with_env_var(
        crate::config::ENV_DAEMON_CONFIG_PATH_OVERRIDE,
        Some(config_path_string.as_str()),
        || {
            let runtime = tokio::runtime::Runtime::new().expect("create runtime");
            runtime.block_on(async {
                let app = build_dashboard_router(test_state(
                    repo.path().to_path_buf(),
                    ServeMode::HelloWorld,
                    repo.path().to_path_buf(),
                ));

                let (status, payload) = request_json_with_method_and_content_type(
                    app,
                    Method::POST,
                    "/devql/runtime",
                    "application/json",
                    Body::from(
                        json!({
                            "query": crate::cli::devql::graphql::documents::RUNTIME_SNAPSHOT_QUERY,
                            "variables": {
                                "repoId": repo_id,
                            }
                        })
                        .to_string(),
                    ),
                )
                .await;

                assert_eq!(status, StatusCode::OK);
                assert!(
                    payload.get("errors").is_none(),
                    "runtime graphql errors: {:?}",
                    payload.get("errors")
                );
                assert_eq!(payload["data"]["runtimeSnapshot"]["repoId"], repo_id);
            });
        },
    );
}

#[tokio::test]
async fn devql_runtime_route_executes_start_init_for_bound_repo_without_catalog_entry() {
    let repo = TempDir::new().expect("temp dir");
    init_test_repo(repo.path(), "main", "Alice", "alice@example.com");
    crate::test_support::git_fixtures::write_test_daemon_config(repo.path());

    let repo_id = crate::host::devql::resolve_repo_identity(repo.path())
        .expect("resolve repo identity")
        .repo_id;
    let app = build_dashboard_router(test_state(
        repo.path().to_path_buf(),
        ServeMode::HelloWorld,
        repo.path().to_path_buf(),
    ));
    let headers = runtime_binding_headers(repo.path());
    let headers_ref = headers
        .iter()
        .map(|(name, value)| (name.as_str(), value.as_str()))
        .collect::<Vec<_>>();

    let (status, payload) = request_json_with_method_content_type_and_headers(
        app,
        Method::POST,
        "/devql/runtime",
        "application/json",
        &headers_ref,
        Body::from(
            json!({
                "query": "mutation StartInit($repoId: String!, $input: StartInitInput!) { startInit(repoId: $repoId, input: $input) { initSessionId } }",
                "variables": {
                    "repoId": repo_id,
                    "input": {
                        "runSync": false,
                        "runIngest": false,
                        "runCodeEmbeddings": false,
                        "runSummaries": false,
                        "runSummaryEmbeddings": false,
                        "ingestBackfill": serde_json::Value::Null,
                        "embeddingsBootstrap": serde_json::Value::Null,
                        "summariesBootstrap": serde_json::Value::Null,
                    }
                }
            })
            .to_string(),
        ),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert!(
        payload.get("errors").is_none(),
        "runtime graphql errors: {:?}",
        payload.get("errors")
    );
    let init_session_id = payload["data"]["startInit"]["initSessionId"]
        .as_str()
        .expect("init session id");
    assert!(init_session_id.starts_with("init-session-"));
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
        Body::from(r#"{"query":"{ defaultBranch health { blob { backend connected } } }"}"#),
    )
    .await;

    assert_eq!(slim_status, StatusCode::OK);
    assert_eq!(slim_payload["data"]["defaultBranch"], "main");
    assert_eq!(slim_payload["data"]["health"]["blob"]["backend"], "local");
    assert_eq!(slim_payload["data"]["health"]["blob"]["connected"], true);
}

#[tokio::test]
async fn devql_post_route_executes_slim_repository_file_and_dependency_queries() {
    let repo = seed_graphql_devql_repo();
    let app = build_dashboard_router(test_state(
        repo.path().to_path_buf(),
        ServeMode::HelloWorld,
        repo.path().to_path_buf(),
    ));

    let (status, payload) = request_slim_query(
        app,
        repo.path(),
        r#"
        {
          defaultBranch
          commits(first: 10) {
            totalCount
            pageInfo {
              hasNextPage
              hasPreviousPage
            }
            edges {
              node {
                commitMessage
                branch
              }
            }
          }
          branches {
            name
            checkpointCount
          }
          users
          agents
          file(path: "src/caller.ts") {
            path
            language
            blobSha
            artefacts(filter: { kind: FUNCTION }, first: 10) {
              totalCount
              edges {
                node {
                  symbolFqn
                  path
                }
              }
            }
            deps(filter: { direction: OUT }, first: 10) {
              totalCount
              edges {
                node {
                  toSymbolRef
                }
              }
            }
          }
          files(path: "src/*.ts") {
            path
          }
          artefacts(filter: { kind: FUNCTION }, first: 10) {
            totalCount
            pageInfo {
              hasNextPage
              hasPreviousPage
            }
            edges {
              node {
                symbolFqn
                path
              }
            }
          }
          deps(filter: { direction: OUT }, first: 10) {
            totalCount
            edges {
              node {
                toSymbolRef
                toArtefact {
                  symbolFqn
                  path
                }
              }
            }
          }
        }
        "#,
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert!(
        payload.get("errors").is_none(),
        "graphql errors: {:?}",
        payload.get("errors")
    );
    assert_eq!(payload["data"]["defaultBranch"], "main");
    assert_eq!(payload["data"]["commits"]["totalCount"], 1);
    assert_eq!(payload["data"]["commits"]["pageInfo"]["hasNextPage"], false);
    assert_eq!(
        payload["data"]["commits"]["pageInfo"]["hasPreviousPage"],
        false
    );
    assert_eq!(
        payload["data"]["commits"]["edges"][0]["node"]["commitMessage"],
        "Seed GraphQL DevQL repo"
    );
    assert_eq!(
        payload["data"]["commits"]["edges"][0]["node"]["branch"],
        "main"
    );
    assert_eq!(payload["data"]["branches"], json!([]));
    let users = payload["data"]["users"]
        .as_array()
        .expect("users should be an array");
    assert!(
        users.is_empty(),
        "expected no users for repo-only fixture, got {users:?}"
    );
    assert_eq!(payload["data"]["agents"], json!([]));
    assert_eq!(payload["data"]["file"]["path"], "src/caller.ts");
    assert_eq!(payload["data"]["file"]["language"], "typescript");
    assert_eq!(payload["data"]["file"]["blobSha"], "blob-caller");
    assert_eq!(payload["data"]["file"]["artefacts"]["totalCount"], 2);
    assert_eq!(payload["data"]["file"]["deps"]["totalCount"], 2);
    assert_eq!(payload["data"]["files"].as_array().map(Vec::len), Some(3));
    assert_eq!(payload["data"]["artefacts"]["totalCount"], 4);
    assert_eq!(
        payload["data"]["artefacts"]["pageInfo"]["hasNextPage"],
        false
    );
    assert_eq!(
        payload["data"]["artefacts"]["pageInfo"]["hasPreviousPage"],
        false
    );
    assert_eq!(payload["data"]["deps"]["totalCount"], 0);
    assert_eq!(payload["data"]["deps"]["edges"], json!([]));
}

#[tokio::test]
async fn devql_interaction_queries_work_in_slim_and_global_scopes() {
    let repo = seed_dashboard_repo();
    let app = build_dashboard_router(test_state(
        repo.path().to_path_buf(),
        ServeMode::HelloWorld,
        repo.path().to_path_buf(),
    ));

    let (slim_status, slim_payload) = request_slim_query(
        app.clone(),
        repo.path(),
        r#"
        {
          interactionSessions(
            first: 10
            filter: {
              actorEmail: "alice@example.com"
              since: "2026-02-27T12:00:00Z"
              until: "2026-02-27T12:05:00Z"
            }
          ) {
            totalCount
            edges {
              node {
                id
                branch
                actor {
                  email
                }
                checkpointCount
                turnCount
                toolUses {
                  toolKind
                  taskDescription
                }
                latestCommitAuthor {
                  checkpointId
                  email
                }
              }
            }
          }
          searchInteractionTurns(
            input: {
              query: "dashboard"
              filter: { path: "app.rs" }
            }
          ) {
            score
            matchedFields
            turn {
              id
              sessionId
              summary
            }
            session {
              id
              actor {
                email
              }
            }
          }
        }
        "#,
    )
    .await;

    assert_eq!(slim_status, StatusCode::OK);
    assert_eq!(
        slim_payload["data"]["interactionSessions"]["totalCount"].as_u64(),
        Some(1)
    );
    assert_eq!(
        slim_payload["data"]["interactionSessions"]["edges"][0]["node"]["id"],
        "session-1"
    );
    assert_eq!(
        slim_payload["data"]["interactionSessions"]["edges"][0]["node"]["branch"],
        "main"
    );
    assert_eq!(
        slim_payload["data"]["interactionSessions"]["edges"][0]["node"]["actor"]["email"],
        "alice@example.com"
    );
    assert_eq!(
        slim_payload["data"]["interactionSessions"]["edges"][0]["node"]["checkpointCount"].as_u64(),
        Some(1)
    );
    assert_eq!(
        slim_payload["data"]["interactionSessions"]["edges"][0]["node"]["turnCount"].as_u64(),
        Some(1)
    );
    assert_eq!(
        slim_payload["data"]["interactionSessions"]["edges"][0]["node"]["toolUses"][0]["toolKind"],
        "edit"
    );
    assert_eq!(
        slim_payload["data"]["searchInteractionTurns"][0]["turn"]["id"],
        "turn-1"
    );
    assert_eq!(
        slim_payload["data"]["searchInteractionTurns"][0]["session"]["id"],
        "session-1"
    );
    assert!(
        slim_payload["data"]["searchInteractionTurns"][0]["matchedFields"]
            .as_array()
            .is_some_and(|fields| !fields.is_empty())
    );

    let (global_status, global_payload) = request_json_with_method_and_content_type(
        app,
        Method::POST,
        "/devql/global",
        "application/json",
        Body::from(
            json!({
                "query": r#"
                {
                  repo(name: "demo") {
                    interactionEvents(first: 10, filter: { toolKind: "edit" }) {
                      totalCount
                      edges {
                        node {
                          eventType
                          toolKind
                          toolUseId
                          taskDescription
                          subagentId
                          actor {
                            email
                          }
                        }
                      }
                    }
                    searchInteractionSessions(
                      input: {
                        query: "dashboard"
                        filter: { branch: "main" }
                      }
                    ) {
                      score
                      matchedFields
                      session {
                        id
                        turnCount
                        checkpointCount
                      }
                    }
                  }
                }
                "#
            })
            .to_string(),
        ),
    )
    .await;

    assert_eq!(global_status, StatusCode::OK);
    assert_eq!(
        global_payload["data"]["repo"]["interactionEvents"]["totalCount"].as_u64(),
        Some(2)
    );
    assert_eq!(
        global_payload["data"]["repo"]["interactionEvents"]["edges"][0]["node"]["toolKind"],
        "edit"
    );
    assert_eq!(
        global_payload["data"]["repo"]["interactionEvents"]["edges"][0]["node"]["actor"]["email"],
        "alice@example.com"
    );
    assert_eq!(
        global_payload["data"]["repo"]["searchInteractionSessions"][0]["session"]["id"],
        "session-1"
    );
    assert_eq!(
        global_payload["data"]["repo"]["searchInteractionSessions"][0]["session"]["checkpointCount"]
            .as_u64(),
        Some(1)
    );
}

#[tokio::test]
async fn devql_post_route_executes_slim_checkpoint_and_telemetry_queries() {
    let repo = seed_dashboard_repo_with_duckdb_events();
    let app = build_dashboard_router(test_state(
        repo.path().to_path_buf(),
        ServeMode::HelloWorld,
        repo.path().to_path_buf(),
    ));

    let (status, payload) = request_slim_query(
        app,
        repo.path(),
        r#"
        {
          checkpoints(first: 5) {
            totalCount
            edges {
              node {
                id
                sessionId
                commitSha
                branch
                agent
              }
            }
          }
          telemetry(first: 5) {
            totalCount
            edges {
              node {
                eventType
                agent
                branch
              }
            }
          }
          users
          agents
        }
        "#,
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert!(
        payload.get("errors").is_none(),
        "graphql errors: {:?}",
        payload.get("errors")
    );
    assert_eq!(payload["data"]["checkpoints"]["totalCount"], 2);
    let checkpoints = payload["data"]["checkpoints"]["edges"]
        .as_array()
        .expect("checkpoint edges should be an array");
    assert!(
        checkpoints.iter().any(|edge| {
            edge["node"]["branch"] == "main"
                && edge["node"]["id"]
                    .as_str()
                    .is_some_and(|value| !value.trim().is_empty())
        }),
        "expected a main-branch checkpoint, got {checkpoints:?}"
    );
    assert_eq!(payload["data"]["telemetry"]["totalCount"], 2);
    assert_eq!(
        payload["data"]["telemetry"]["edges"][0]["node"]["eventType"],
        "checkpoint_committed"
    );
    assert_eq!(payload["data"]["users"], json!(["alice@example.com"]));

    let agents = payload["data"]["agents"]
        .as_array()
        .expect("agents should be an array");
    assert!(
        agents.iter().any(|value| value == "claude-code"),
        "expected claude-code in agents: {agents:?}"
    );
}

#[tokio::test]
async fn devql_post_route_executes_slim_knowledge_queries() {
    let repo = seed_graphql_devql_repo();
    let seeded = seed_graphql_knowledge_data(repo.path());
    let app = build_dashboard_router(test_state(
        repo.path().to_path_buf(),
        ServeMode::HelloWorld,
        repo.path().to_path_buf(),
    ));

    let (status, payload) = request_slim_query(
        app,
        repo.path(),
        r#"
        {
          jiraOnly: knowledge(provider: JIRA, first: 10) {
            totalCount
          }
          knowledge(first: 10) {
            totalCount
            edges {
              node {
                id
                provider
                title
              }
            }
          }
        }
        "#,
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert!(
        payload.get("errors").is_none(),
        "graphql errors: {:?}",
        payload.get("errors")
    );
    assert_eq!(payload["data"]["jiraOnly"]["totalCount"], 1);
    assert_eq!(payload["data"]["knowledge"]["totalCount"], 2);
    assert_eq!(
        payload["data"]["knowledge"]["edges"][0]["node"]["id"],
        seeded.primary_item_id
    );
    assert_eq!(
        payload["data"]["knowledge"]["edges"][1]["node"]["id"],
        seeded.secondary_item_id
    );
    assert_eq!(
        payload["data"]["knowledge"]["edges"][0]["node"]["provider"],
        "JIRA"
    );
}

#[tokio::test]
async fn devql_post_route_executes_slim_clone_queries() {
    let repo = seed_graphql_monorepo_repo();
    seed_graphql_clone_data(repo.path());
    let app = build_dashboard_router(test_state(
        repo.path().to_path_buf(),
        ServeMode::HelloWorld,
        repo.path().to_path_buf(),
    ));

    let (status, payload) = request_slim_query(
        app,
        repo.path(),
        r#"
        {
          clones(filter: { minScore: 0.75 }, first: 10) {
            totalCount
            summary {
              totalCount
              groups {
                relationKind
                count
              }
            }
            edges {
              node {
                relationKind
                score
                sourceStartLine
                sourceEndLine
                targetStartLine
                targetEndLine
                sourceArtefact {
                  symbolFqn
                }
                targetArtefact {
                  symbolFqn
                }
              }
            }
          }
        }
        "#,
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert!(
        payload.get("errors").is_none(),
        "graphql errors: {:?}",
        payload.get("errors")
    );
    assert_eq!(payload["data"]["clones"]["totalCount"], 0);
    assert_eq!(payload["data"]["clones"]["summary"]["totalCount"], 0);
}

#[tokio::test]
async fn devql_post_route_executes_slim_clone_summary_queries() {
    let repo = seed_graphql_monorepo_repo();
    seed_graphql_clone_data(repo.path());
    let app = build_dashboard_router(test_state(
        repo.path().to_path_buf(),
        ServeMode::HelloWorld,
        repo.path().to_path_buf(),
    ));

    let (status, payload) = request_slim_query(
        app,
        repo.path(),
        r#"
        {
          cloneSummary(
            filter: { kind: FUNCTION }
            cloneFilter: { minScore: 0.68 }
          ) {
            totalCount
            groups {
              relationKind
              count
            }
          }
        }
        "#,
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert!(
        payload.get("errors").is_none(),
        "graphql errors: {:?}",
        payload.get("errors")
    );
    assert_eq!(payload["data"]["cloneSummary"]["totalCount"], 3);
    assert_eq!(
        payload["data"]["cloneSummary"]["groups"][0]["relationKind"],
        "similar_implementation"
    );
    assert_eq!(payload["data"]["cloneSummary"]["groups"][0]["count"], 2);
    assert_eq!(
        payload["data"]["cloneSummary"]["groups"][1]["relationKind"],
        "contextual_neighbor"
    );
    assert_eq!(payload["data"]["cloneSummary"]["groups"][1]["count"], 1);
}

#[tokio::test]
async fn devql_post_route_rejects_slim_clone_summary_invalid_inputs_and_temporal_scopes() {
    let repo = seed_graphql_monorepo_repo();
    seed_graphql_clone_data(repo.path());
    let commit_sha = git_ok(repo.path(), &["rev-parse", "HEAD"]);
    let app = build_dashboard_router(test_state(
        repo.path().to_path_buf(),
        ServeMode::HelloWorld,
        repo.path().to_path_buf(),
    ));

    let (_, payload) = request_slim_query(
        app,
        repo.path(),
        &format!(
            r#"
            {{
              badSummary: cloneSummary(cloneFilter: {{ minScore: 1.5 }}) {{
                totalCount
              }}
              asOf(input: {{ commit: "{commit_sha}" }}) {{
                project(path: "packages/api") {{
                  cloneSummary(filter: {{ kind: FUNCTION }}) {{
                    totalCount
                  }}
                }}
                file(path: "packages/api/src/caller.ts") {{
                  cloneSummary(filter: {{ kind: FUNCTION }}) {{
                    totalCount
                  }}
                }}
              }}
            }}
            "#,
        ),
    )
    .await;

    let errors = payload["errors"]
        .as_array()
        .expect("expected graphql errors");
    assert_eq!(errors.len(), 3, "unexpected errors: {errors:?}");
    let messages = errors
        .iter()
        .filter_map(|error| error["message"].as_str())
        .collect::<Vec<_>>();
    assert!(
        messages
            .iter()
            .any(|message| message.contains("`minScore` must be between 0 and 1")),
        "expected minScore validation error, got {messages:?}"
    );
    assert_eq!(
        messages
            .iter()
            .filter(|message| {
                message.contains(
                    "`clones` does not support historical or temporary `asOf(...)` scopes yet",
                )
            })
            .count(),
        2,
        "expected temporal cloneSummary errors, got {messages:?}"
    );
}

#[tokio::test]
async fn devql_post_route_executes_slim_test_harness_stage_queries() {
    let repo = seed_graphql_devql_repo();
    let commit_sha = git_ok(repo.path(), &["rev-parse", "HEAD"]);
    seed_graphql_test_harness_stage_data(
        repo.path(),
        &commit_sha,
        &[(
            "sym::caller",
            "artefact::caller",
            "src/caller.ts",
            "caller_tests",
        )],
    );
    let app = build_dashboard_router(test_state(
        repo.path().to_path_buf(),
        ServeMode::HelloWorld,
        repo.path().to_path_buf(),
    ));

    let (status, payload) = request_slim_query(
        app,
        repo.path(),
        r#"
        {
          tests(
            filter: { symbolFqn: "src/caller.ts::caller" },
            minConfidence: 0.8,
            linkageSource: "static_analysis",
            first: 5
          ) {
            artefact {
              artefactId
              filePath
            }
            coveringTests {
              testName
              linkageSource
            }
            summary {
              totalCoveringTests
            }
          }
          coverage(filter: { symbolFqn: "src/caller.ts::caller" }, first: 5) {
            artefact {
              artefactId
            }
            coverage {
              coverageSource
              lineCoveragePct
              branchDataAvailable
              uncoveredLines
            }
            summary {
              uncoveredLineCount
            }
          }
        }
        "#,
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert!(
        payload.get("errors").is_none(),
        "graphql errors: {:?}",
        payload.get("errors")
    );
    assert_eq!(
        payload["data"]["tests"][0]["artefact"]["artefactId"],
        "artefact::caller"
    );
    assert_eq!(
        payload["data"]["tests"][0]["coveringTests"][0]["testName"],
        "caller_tests"
    );
    assert_eq!(
        payload["data"]["tests"][0]["coveringTests"][0]["linkageSource"],
        "static_analysis"
    );
    assert_eq!(
        payload["data"]["tests"][0]["summary"]["totalCoveringTests"],
        1
    );
    assert_eq!(
        payload["data"]["coverage"][0]["coverage"]["coverageSource"],
        "lcov"
    );
    assert_eq!(
        payload["data"]["coverage"][0]["coverage"]["lineCoveragePct"],
        50.0
    );
    assert_eq!(
        payload["data"]["coverage"][0]["coverage"]["branchDataAvailable"],
        true
    );
    assert_eq!(
        payload["data"]["coverage"][0]["coverage"]["uncoveredLines"],
        json!([5])
    );
    assert_eq!(
        payload["data"]["coverage"][0]["summary"]["uncoveredLineCount"],
        1
    );
}

#[tokio::test]
async fn devql_post_route_executes_slim_as_of_queries_and_surfaces_validation_errors() {
    let seeded = seed_graphql_temporal_repo();
    let app = build_dashboard_router(test_state(
        seeded.repo.path().to_path_buf(),
        ServeMode::HelloWorld,
        seeded.repo.path().to_path_buf(),
    ));

    let (status, payload) = request_slim_query(
        app.clone(),
        seeded.repo.path(),
        &format!(
            r#"
            {{
              asOf(input: {{ commit: "{}" }}) {{
                resolvedCommit
                file(path: "packages/api/src/caller.ts") {{
                  path
                }}
              }}
            }}
            "#,
            seeded.first_commit
        ),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert!(
        payload.get("errors").is_none(),
        "graphql errors: {:?}",
        payload.get("errors")
    );
    assert_eq!(
        payload["data"]["asOf"]["resolvedCommit"],
        seeded.first_commit.as_str()
    );
    assert_eq!(
        payload["data"]["asOf"]["file"]["path"],
        "packages/api/src/caller.ts"
    );

    let (_, error_payload) = request_slim_query(
        app,
        seeded.repo.path(),
        r#"
        {
          badRange: commits(
            since: "2026-03-27T00:00:00Z",
            until: "2026-03-26T00:00:00Z",
            first: 1
          ) {
            totalCount
          }
          badCursor: commits(first: 1, after: "missing-cursor") {
            totalCount
          }
          badAsOf: asOf(input: { ref: "refs/heads/missing-temporal-branch" }) {
            resolvedCommit
          }
        }
        "#,
    )
    .await;

    let errors = error_payload["errors"]
        .as_array()
        .expect("expected graphql errors");
    assert_eq!(errors.len(), 3, "unexpected errors: {errors:?}");
    let messages = errors
        .iter()
        .filter_map(|error| error["message"].as_str())
        .collect::<Vec<_>>();
    assert!(
        messages
            .iter()
            .any(|message| message.contains("`since` must be earlier than or equal to `until`")),
        "expected invalid time range error, got {messages:?}"
    );
    assert!(
        messages
            .iter()
            .any(|message| message.contains("cursor `missing-cursor`")),
        "expected bad cursor error, got {messages:?}"
    );
    assert!(
        messages
            .iter()
            .any(|message| message.contains("unknown")
                || message.contains("missing-temporal-branch")),
        "expected bad asOf error, got {messages:?}"
    );
}

#[tokio::test]
async fn devql_post_route_surfaces_slim_stage_validation_errors() {
    let repo = seed_graphql_devql_repo();
    let commit_sha = git_ok(repo.path(), &["rev-parse", "HEAD"]);
    seed_graphql_test_harness_stage_data(
        repo.path(),
        &commit_sha,
        &[(
            "sym::caller",
            "artefact::caller",
            "src/caller.ts",
            "caller_tests",
        )],
    );
    let app = build_dashboard_router(test_state(
        repo.path().to_path_buf(),
        ServeMode::HelloWorld,
        repo.path().to_path_buf(),
    ));

    let (_, payload) = request_slim_query(
        app,
        repo.path(),
        r#"
        {
          badTests: tests(minConfidence: 1.1, first: 5) {
            artefact {
              artefactId
            }
          }
          badCoverage: coverage(first: 0) {
            artefact {
              artefactId
            }
          }
          badTestsSummary: testsSummary {
            commitSha
          }
        }
        "#,
    )
    .await;

    let errors = payload["errors"]
        .as_array()
        .expect("expected graphql errors");
    assert_eq!(errors.len(), 3, "unexpected errors: {errors:?}");
    let messages = errors
        .iter()
        .filter_map(|error| error["message"].as_str())
        .collect::<Vec<_>>();
    assert!(
        messages
            .iter()
            .any(|message| message.contains("`minConfidence` must be between 0 and 1")),
        "expected minConfidence validation error, got {messages:?}"
    );
    assert!(
        messages
            .iter()
            .any(|message| message.contains("`first` must be greater than 0")),
        "expected coverage limit validation error, got {messages:?}"
    );
    assert!(
        messages
            .iter()
            .any(|message| message.contains("requires a resolved commit")),
        "expected testsSummary commit error, got {messages:?}"
    );
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
async fn devql_runtime_ws_route_is_registered() {
    let temp = TempDir::new().expect("temp dir");
    let app = build_dashboard_router(test_state(
        temp.path().to_path_buf(),
        ServeMode::HelloWorld,
        temp.path().to_path_buf(),
    ));

    let (status, _) = request_text_with_method(app, Method::GET, "/devql/runtime/ws").await;

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
async fn devql_graphql_task_progress_subscription_receives_published_task_events() {
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
          taskProgress(taskId: "ingest-task-1") {
            task {
              taskId
              kind
              status
              ingestProgress {
                phase
                commitsTotal
                commitsProcessed
                checkpointCompanionsProcessed
                currentCheckpointId
                currentCommitSha
              }
            }
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

    let make_task = |task_id: &str,
                     status: crate::daemon::DevqlTaskStatus,
                     phase: crate::host::devql::IngestionProgressPhase,
                     commits_processed: usize| crate::daemon::DevqlTaskRecord {
        task_id: task_id.to_string(),
        repo_id: "repo-1".to_string(),
        repo_name: "demo".to_string(),
        repo_provider: "local".to_string(),
        repo_organisation: "local".to_string(),
        repo_identity: "local/demo".to_string(),
        daemon_config_root: repo.path().to_path_buf(),
        repo_root: repo.path().to_path_buf(),
        kind: crate::daemon::DevqlTaskKind::Ingest,
        source: crate::daemon::DevqlTaskSource::ManualCli,
        spec: crate::daemon::DevqlTaskSpec::Ingest(crate::daemon::IngestTaskSpec {
            backfill: Some(1),
        }),
        init_session_id: None,
        status,
        submitted_at_unix: 1,
        started_at_unix: Some(2),
        updated_at_unix: 3,
        completed_at_unix: None,
        queue_position: None,
        tasks_ahead: None,
        progress: crate::daemon::DevqlTaskProgress::Ingest(
            crate::host::devql::IngestionProgressUpdate {
                phase,
                commits_total: 1,
                commits_processed,
                current_checkpoint_id: Some("aabbccddeeff".to_string()),
                current_commit_sha: Some(git_ok(repo.path(), &["rev-parse", "HEAD"])),
                counters: crate::host::devql::IngestionCounters {
                    success: matches!(phase, crate::host::devql::IngestionProgressPhase::Complete),
                    checkpoint_companions_processed: commits_processed,
                    events_inserted: commits_processed,
                    artefacts_upserted: commits_processed + 1,
                    ..Default::default()
                },
            },
        ),
        error: None,
        result: None,
    };

    context.subscriptions().publish_task(make_task(
        "other-task",
        crate::daemon::DevqlTaskStatus::Running,
        crate::host::devql::IngestionProgressPhase::Failed,
        0,
    ));
    context.subscriptions().publish_task(make_task(
        "ingest-task-1",
        crate::daemon::DevqlTaskStatus::Running,
        crate::host::devql::IngestionProgressPhase::Initializing,
        0,
    ));
    context.subscriptions().publish_task(make_task(
        "ingest-task-1",
        crate::daemon::DevqlTaskStatus::Completed,
        crate::host::devql::IngestionProgressPhase::Complete,
        1,
    ));

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
        let payload = json["taskProgress"]["task"]["ingestProgress"].clone();
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
    assert_eq!(last_payload["commitsTotal"], Value::from(1));
    assert_eq!(last_payload["commitsProcessed"], Value::from(1));
    assert_eq!(
        last_payload["checkpointCompanionsProcessed"],
        Value::from(1)
    );
}

#[tokio::test]
async fn devql_task_and_checkpoint_events_publish_to_subscription_hub() {
    let repo = seed_dashboard_repo();
    let head_sha = git_ok(repo.path(), &["rev-parse", "HEAD"]);
    let context = crate::graphql::DevqlGraphqlContext::for_slim_request(
        repo.path().to_path_buf(),
        repo.path().to_path_buf(),
        Some("main".to_string()),
        None,
        None,
        true,
        super::super::db::DashboardDbPools::default(),
    );
    let mut progress_rx = context.subscriptions().subscribe_task_progress();
    let mut checkpoint_rx = context.subscriptions().subscribe_checkpoints();
    let checkpoint_info = crate::host::checkpoints::strategy::manual_commit::CommittedInfo {
        checkpoint_id: "checkpoint-1".to_string(),
        session_id: "session-1".to_string(),
        agent: "assistant".to_string(),
        created_at: "1970-01-01T00:00:00+00:00".to_string(),
        ..Default::default()
    };
    context.subscriptions().publish_checkpoint(
        "demo",
        crate::graphql::Checkpoint::from_ingested(&checkpoint_info, Some(head_sha.as_str())),
    );
    context
        .subscriptions()
        .publish_task(crate::daemon::DevqlTaskRecord {
            task_id: "ingest-task-1".to_string(),
            repo_id: "repo-1".to_string(),
            repo_name: "demo".to_string(),
            repo_provider: "local".to_string(),
            repo_organisation: "local".to_string(),
            repo_identity: "local/demo".to_string(),
            daemon_config_root: repo.path().to_path_buf(),
            repo_root: repo.path().to_path_buf(),
            kind: crate::daemon::DevqlTaskKind::Ingest,
            source: crate::daemon::DevqlTaskSource::ManualCli,
            spec: crate::daemon::DevqlTaskSpec::Ingest(crate::daemon::IngestTaskSpec::default()),
            init_session_id: None,
            status: crate::daemon::DevqlTaskStatus::Completed,
            submitted_at_unix: 1,
            started_at_unix: Some(2),
            updated_at_unix: 3,
            completed_at_unix: Some(4),
            queue_position: None,
            tasks_ahead: None,
            progress: crate::daemon::DevqlTaskProgress::Ingest(
                crate::host::devql::IngestionProgressUpdate {
                    phase: crate::host::devql::IngestionProgressPhase::Complete,
                    commits_total: 1,
                    commits_processed: 1,
                    current_checkpoint_id: Some("checkpoint-1".to_string()),
                    current_commit_sha: Some(head_sha.clone()),
                    counters: crate::host::devql::IngestionCounters {
                        success: true,
                        checkpoint_companions_processed: 1,
                        events_inserted: 1,
                        artefacts_upserted: 1,
                        ..Default::default()
                    },
                },
            ),
            error: None,
            result: None,
        });

    let checkpoint = tokio::time::timeout(std::time::Duration::from_secs(5), checkpoint_rx.recv())
        .await
        .expect("checkpoint event should arrive")
        .expect("checkpoint subscription payload");
    assert_eq!(checkpoint.checkpoint.commit_sha, Some(head_sha.clone()));

    let progress = tokio::time::timeout(std::time::Duration::from_secs(5), progress_rx.recv())
        .await
        .expect("task progress event should arrive")
        .expect("task progress subscription payload");
    assert_eq!(progress.task_id, "ingest-task-1");
    assert_eq!(
        progress.task.status,
        crate::daemon::DevqlTaskStatus::Completed
    );
}
