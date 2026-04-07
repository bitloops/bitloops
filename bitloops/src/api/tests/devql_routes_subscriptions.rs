use super::*;

fn write_current_repo_runtime_state(repo_root: &Path) {
    let runtime_path = crate::daemon::runtime_state_path(repo_root);
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
    assert_eq!(payload["data"]["file"]["deps"]["totalCount"], 1);
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
            commitsTotal
            commitsProcessed
            checkpointCompanionsProcessed
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
            commits_total: 99,
            commits_processed: 13,
            checkpoint_companions_processed: 8,
            current_checkpoint_id: Some("wrong-repo-checkpoint".to_string()),
            current_commit_sha: Some("wrong-repo-sha".to_string()),
            events_inserted: 8,
            artefacts_upserted: 5,
        },
    );
    context.subscriptions().publish_progress(
        "demo",
        crate::graphql::IngestionProgressEvent {
            phase: crate::graphql::IngestionPhase::Initializing,
            commits_total: 1,
            commits_processed: 0,
            checkpoint_companions_processed: 0,
            current_checkpoint_id: None,
            current_commit_sha: None,
            events_inserted: 0,
            artefacts_upserted: 0,
        },
    );
    context.subscriptions().publish_progress(
        "demo",
        crate::graphql::IngestionProgressEvent {
            phase: crate::graphql::IngestionPhase::Complete,
            commits_total: 1,
            commits_processed: 1,
            checkpoint_companions_processed: 1,
            current_checkpoint_id: Some("aabbccddeeff".to_string()),
            current_commit_sha: Some(git_ok(repo.path(), &["rev-parse", "HEAD"])),
            events_inserted: 1,
            artefacts_upserted: 2,
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
    assert_eq!(last_payload["commitsTotal"], Value::from(1));
    assert_eq!(last_payload["commitsProcessed"], Value::from(1));
    assert_eq!(
        last_payload["checkpointCompanionsProcessed"],
        Value::from(1)
    );
}

#[tokio::test]
async fn devql_ingest_mutation_publishes_progress_and_checkpoint_events_to_subscription_hub() {
    let repo = seed_dashboard_repo();
    let daemon_state = TempDir::new().expect("temp dir");
    let daemon_state_str = daemon_state.path().to_string_lossy().to_string();
    let _guard = enter_process_state(
        Some(repo.path()),
        &[
            ("BITLOOPS_DEVQL_SEMANTIC_PROVIDER", Some("disabled")),
            (
                "BITLOOPS_TEST_STATE_DIR_OVERRIDE",
                Some(daemon_state_str.as_str()),
            ),
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
                }
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

    let init_response = schema
        .execute(async_graphql::Request::new(
            r#"
            mutation {
              initSchema {
                success
              }
            }
            "#,
        ))
        .await;
    assert!(
        init_response.errors.is_empty(),
        "graphql errors: {:?}",
        init_response.errors
    );
    let init_json = init_response.data.into_json().expect("init data to json");
    assert_eq!(init_json["initSchema"]["success"], true);
    write_current_repo_runtime_state(repo.path());

    let response = schema
        .execute(async_graphql::Request::new(
            r#"
            mutation {
              ingest {
                success
                commitsProcessed
                checkpointCompanionsProcessed
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
    assert!(
        response_json["ingest"]["checkpointCompanionsProcessed"].is_number(),
        "expected checkpoint companion counter in ingest response"
    );
    if response_json["ingest"]["commitsProcessed"].as_i64() == Some(1) {
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
