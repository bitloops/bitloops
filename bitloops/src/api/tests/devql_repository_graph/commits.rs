use super::*;

#[tokio::test]
async fn devql_repository_queries_resolve_repo_commit_branch_user_agent_and_checkpoint_data() {
    let repo = seed_dashboard_repo();
    let schema = crate::graphql::build_schema(crate::graphql::DevqlGraphqlContext::new(
        repo.path().to_path_buf(),
        super::super::super::db::DashboardDbPools::default(),
    ));

    let response = schema
        .execute(async_graphql::Request::new(
            r#"
            {
              repo(name: "demo") {
                defaultBranch
                commits(first: 2) {
                  totalCount
                  pageInfo {
                    hasNextPage
                    hasPreviousPage
                    startCursor
                    endCursor
                  }
                  edges {
                    cursor
                    node {
                      sha
                      authorName
                      authorEmail
                      commitMessage
                      branch
                      filesChanged
                      checkpoints(first: 5) {
                        totalCount
                        pageInfo {
                          hasNextPage
                          hasPreviousPage
                          startCursor
                          endCursor
                        }
                        edges {
                          cursor
                          node {
                            id
                            sessionId
                            commitSha
                            branch
                            agent
                            strategy
                            filesTouched
                            eventTime
                          }
                        }
                      }
                    }
                  }
                }
                branches {
                  name
                  checkpointCount
                  latestCheckpointAt
                }
                users
                agents
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

    let json = response.data.into_json().expect("graphql data to json");
    assert_eq!(json["repo"]["defaultBranch"], "main");
    assert_eq!(json["repo"]["commits"]["totalCount"], 2);
    assert_eq!(json["repo"]["commits"]["pageInfo"]["hasNextPage"], false);
    assert_eq!(
        json["repo"]["commits"]["pageInfo"]["hasPreviousPage"],
        false
    );
    assert_eq!(
        json["repo"]["commits"]["edges"][0]["node"]["commitMessage"],
        "Checkpoint commit"
    );
    assert_eq!(
        json["repo"]["commits"]["edges"][0]["node"]["branch"],
        "main"
    );
    assert_eq!(
        json["repo"]["commits"]["edges"][0]["node"]["filesChanged"],
        json!(["app.rs"])
    );
    assert_eq!(
        json["repo"]["commits"]["edges"][0]["node"]["checkpoints"]["totalCount"],
        1
    );
    assert_eq!(
        json["repo"]["commits"]["edges"][0]["node"]["checkpoints"]["edges"][0]["node"]["id"],
        "aabbccddeeff"
    );
    assert_eq!(
        json["repo"]["commits"]["edges"][0]["node"]["checkpoints"]["edges"][0]["node"]["sessionId"],
        "session-1"
    );
    assert_eq!(
        json["repo"]["commits"]["edges"][0]["node"]["checkpoints"]["edges"][0]["node"]["commitSha"],
        json["repo"]["commits"]["edges"][0]["node"]["sha"]
    );
    assert_eq!(
        json["repo"]["commits"]["edges"][0]["node"]["checkpoints"]["edges"][0]["node"]["branch"],
        "main"
    );
    assert_eq!(
        json["repo"]["commits"]["edges"][0]["node"]["checkpoints"]["edges"][0]["node"]["agent"],
        "claude-code"
    );
    assert_eq!(
        json["repo"]["commits"]["edges"][0]["node"]["checkpoints"]["edges"][0]["node"]["strategy"],
        "manual-commit"
    );
    assert_eq!(
        json["repo"]["commits"]["edges"][0]["node"]["checkpoints"]["edges"][0]["node"]["filesTouched"],
        json!(["app.rs"])
    );
    assert_eq!(
        json["repo"]["commits"]["edges"][0]["node"]["checkpoints"]["edges"][0]["node"]["eventTime"],
        "2026-02-27T12:00:00+00:00"
    );
    assert_eq!(
        json["repo"]["commits"]["edges"][1]["node"]["commitMessage"],
        "Initial commit"
    );
    assert_eq!(
        json["repo"]["commits"]["edges"][1]["node"]["checkpoints"]["totalCount"],
        0
    );
    assert_eq!(
        json["repo"]["branches"],
        json!([{
            "name": "main",
            "checkpointCount": 1,
            "latestCheckpointAt": "2026-02-27T12:00:00+00:00"
        }])
    );
    assert_eq!(json["repo"]["users"], json!(["alice@example.com"]));
    assert_eq!(json["repo"]["agents"], json!(["claude-code"]));
}

#[tokio::test]
async fn devql_repository_checkpoint_queries_fall_back_to_committed_storage_when_event_store_is_empty()
 {
    let repo = seed_dashboard_repo();
    seed_duckdb_events(repo.path(), &[]);
    let schema = crate::graphql::build_schema(crate::graphql::DevqlGraphqlContext::new(
        repo.path().to_path_buf(),
        super::super::super::db::DashboardDbPools::default(),
    ));

    let response = schema
        .execute(async_graphql::Request::new(
            r#"
            {
              repo(name: "demo") {
                checkpoints(first: 5) {
                  totalCount
                  edges {
                    node {
                      id
                      sessionId
                      commitSha
                      branch
                      agent
                      strategy
                      filesTouched
                      eventTime
                      checkpointsCount
                      sessionCount
                      createdAt
                      firstPromptPreview
                      agents
                      tokenUsage {
                        inputTokens
                        outputTokens
                        cacheCreationTokens
                        cacheReadTokens
                        apiCallCount
                      }
                    }
                  }
                }
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

    let json = response.data.into_json().expect("graphql data to json");
    assert_eq!(json["repo"]["checkpoints"]["totalCount"], 1);
    assert_eq!(
        json["repo"]["checkpoints"]["edges"][0]["node"]["id"],
        "aabbccddeeff"
    );
    assert_eq!(
        json["repo"]["checkpoints"]["edges"][0]["node"]["sessionId"],
        "session-1"
    );
    assert_eq!(
        json["repo"]["checkpoints"]["edges"][0]["node"]["branch"],
        "main"
    );
    assert_eq!(
        json["repo"]["checkpoints"]["edges"][0]["node"]["agent"],
        "claude-code"
    );
    assert_eq!(
        json["repo"]["checkpoints"]["edges"][0]["node"]["strategy"],
        "manual-commit"
    );
    assert_eq!(
        json["repo"]["checkpoints"]["edges"][0]["node"]["filesTouched"],
        json!(["app.rs"])
    );
    assert_eq!(
        json["repo"]["checkpoints"]["edges"][0]["node"]["eventTime"],
        "2026-02-27T12:00:00+00:00"
    );
    assert_eq!(
        json["repo"]["checkpoints"]["edges"][0]["node"]["checkpointsCount"],
        2
    );
    assert_eq!(
        json["repo"]["checkpoints"]["edges"][0]["node"]["sessionCount"],
        1
    );
    assert_eq!(
        json["repo"]["checkpoints"]["edges"][0]["node"]["createdAt"],
        "2026-02-27T12:00:00Z"
    );
    assert_eq!(
        json["repo"]["checkpoints"]["edges"][0]["node"]["firstPromptPreview"],
        "Build dashboard API"
    );
    assert_eq!(
        json["repo"]["checkpoints"]["edges"][0]["node"]["agents"],
        json!(["claude-code"])
    );
    assert_eq!(
        json["repo"]["checkpoints"]["edges"][0]["node"]["tokenUsage"],
        json!({
            "inputTokens": 100,
            "outputTokens": 40,
            "cacheCreationTokens": 10,
            "cacheReadTokens": 5,
            "apiCallCount": 3
        })
    );
}

#[tokio::test]
async fn devql_commit_connection_supports_cursor_pagination() {
    let repo = seed_dashboard_repo();
    let schema = crate::graphql::build_schema(crate::graphql::DevqlGraphqlContext::new(
        repo.path().to_path_buf(),
        super::super::super::db::DashboardDbPools::default(),
    ));

    let first_page = schema
        .execute(async_graphql::Request::new(
            r#"
            {
              repo(name: "demo") {
                commits(first: 1) {
                  totalCount
                  pageInfo {
                    hasNextPage
                    hasPreviousPage
                    startCursor
                    endCursor
                  }
                  edges {
                    cursor
                    node {
                      commitMessage
                      checkpoints(first: 1) {
                        totalCount
                      }
                    }
                  }
                }
              }
            }
            "#,
        ))
        .await;

    assert!(
        first_page.errors.is_empty(),
        "graphql errors: {:?}",
        first_page.errors
    );

    let first_json = first_page.data.into_json().expect("graphql data to json");
    let cursor = first_json["repo"]["commits"]["pageInfo"]["endCursor"]
        .as_str()
        .expect("first page end cursor")
        .to_string();
    assert_eq!(first_json["repo"]["commits"]["totalCount"], 2);
    assert_eq!(
        first_json["repo"]["commits"]["pageInfo"]["hasNextPage"],
        true
    );
    assert_eq!(
        first_json["repo"]["commits"]["pageInfo"]["hasPreviousPage"],
        false
    );
    assert_eq!(
        first_json["repo"]["commits"]["edges"][0]["node"]["commitMessage"],
        "Checkpoint commit"
    );
    assert_eq!(
        first_json["repo"]["commits"]["edges"][0]["node"]["checkpoints"]["totalCount"],
        1
    );

    let second_page = schema
        .execute(async_graphql::Request::new(format!(
            r#"
            {{
              repo(name: "demo") {{
                commits(first: 1, after: "{cursor}") {{
                  totalCount
                  pageInfo {{
                    hasNextPage
                    hasPreviousPage
                    startCursor
                    endCursor
                  }}
                  edges {{
                    cursor
                    node {{
                      commitMessage
                      checkpoints(first: 1) {{
                        totalCount
                      }}
                    }}
                  }}
                }}
              }}
            }}
            "#
        )))
        .await;

    assert!(
        second_page.errors.is_empty(),
        "graphql errors: {:?}",
        second_page.errors
    );

    let second_json = second_page.data.into_json().expect("graphql data to json");
    assert_eq!(second_json["repo"]["commits"]["totalCount"], 2);
    assert_eq!(
        second_json["repo"]["commits"]["pageInfo"]["hasNextPage"],
        false
    );
    assert_eq!(
        second_json["repo"]["commits"]["pageInfo"]["hasPreviousPage"],
        true
    );
    assert_eq!(
        second_json["repo"]["commits"]["edges"][0]["node"]["commitMessage"],
        "Initial commit"
    );
    assert_eq!(
        second_json["repo"]["commits"]["edges"][0]["node"]["checkpoints"]["totalCount"],
        0
    );
}

#[tokio::test]
async fn devql_commit_connection_surfaces_structured_cursor_errors() {
    let repo = seed_dashboard_repo();
    let schema = crate::graphql::build_schema(crate::graphql::DevqlGraphqlContext::new(
        repo.path().to_path_buf(),
        super::super::super::db::DashboardDbPools::default(),
    ));

    let response = schema
        .execute(async_graphql::Request::new(
            r#"
            {
              repo(name: "demo") {
                commits(first: 1, after: "missing-cursor") {
                  edges {
                    cursor
                  }
                }
              }
            }
            "#,
        ))
        .await;

    assert_eq!(response.errors.len(), 1, "expected one graphql error");
    let extensions = response.errors[0]
        .extensions
        .as_ref()
        .expect("graphql error extensions");
    assert_eq!(
        extensions.get("code"),
        Some(&async_graphql::Value::from("BAD_CURSOR"))
    );
}

#[tokio::test]
async fn devql_commit_connection_supports_reverse_pagination() {
    let repo = seed_dashboard_repo();
    let schema = crate::graphql::build_schema(crate::graphql::DevqlGraphqlContext::new(
        repo.path().to_path_buf(),
        super::super::super::db::DashboardDbPools::default(),
    ));

    let tail_page = schema
        .execute(async_graphql::Request::new(
            r#"
            {
              repo(name: "demo") {
                commits(last: 1) {
                  totalCount
                  pageInfo {
                    hasNextPage
                    hasPreviousPage
                    startCursor
                    endCursor
                  }
                  edges {
                    node {
                      commitMessage
                    }
                  }
                }
              }
            }
            "#,
        ))
        .await;

    assert!(
        tail_page.errors.is_empty(),
        "graphql errors: {:?}",
        tail_page.errors
    );

    let tail_json = tail_page.data.into_json().expect("graphql data to json");
    assert_eq!(tail_json["repo"]["commits"]["totalCount"], 2);
    assert_eq!(
        tail_json["repo"]["commits"]["pageInfo"]["hasNextPage"],
        false
    );
    assert_eq!(
        tail_json["repo"]["commits"]["pageInfo"]["hasPreviousPage"],
        true
    );
    assert_eq!(
        tail_json["repo"]["commits"]["edges"][0]["node"]["commitMessage"],
        "Initial commit"
    );

    let before_cursor = tail_json["repo"]["commits"]["pageInfo"]["startCursor"]
        .as_str()
        .expect("tail start cursor")
        .to_string();

    let before_page = schema
        .execute(async_graphql::Request::new(format!(
            r#"
            {{
              repo(name: "demo") {{
                commits(last: 1, before: "{before_cursor}") {{
                  pageInfo {{
                    hasNextPage
                    hasPreviousPage
                  }}
                  edges {{
                    node {{
                      commitMessage
                    }}
                  }}
                }}
              }}
            }}
            "#
        )))
        .await;

    assert!(
        before_page.errors.is_empty(),
        "graphql errors: {:?}",
        before_page.errors
    );

    let before_json = before_page.data.into_json().expect("graphql data to json");
    assert_eq!(
        before_json["repo"]["commits"]["pageInfo"]["hasNextPage"],
        true
    );
    assert_eq!(
        before_json["repo"]["commits"]["pageInfo"]["hasPreviousPage"],
        false
    );
    assert_eq!(
        before_json["repo"]["commits"]["edges"][0]["node"]["commitMessage"],
        "Checkpoint commit"
    );
}

#[tokio::test]
async fn devql_commit_connection_rejects_mixed_forward_and_reverse_arguments() {
    let repo = seed_dashboard_repo();
    let schema = crate::graphql::build_schema(crate::graphql::DevqlGraphqlContext::new(
        repo.path().to_path_buf(),
        super::super::super::db::DashboardDbPools::default(),
    ));

    let response = schema
        .execute(async_graphql::Request::new(
            r#"
            {
              repo(name: "demo") {
                commits(first: 1, last: 1) {
                  edges {
                    cursor
                  }
                }
              }
            }
            "#,
        ))
        .await;

    assert_eq!(response.errors.len(), 1, "expected one graphql error");
    let extensions = response.errors[0]
        .extensions
        .as_ref()
        .expect("graphql error extensions");
    assert_eq!(
        extensions.get("code"),
        Some(&async_graphql::Value::from("BAD_USER_INPUT"))
    );
}

#[tokio::test]
async fn devql_repository_queries_handle_repos_without_checkpoint_storage() {
    let repo = TempDir::new().expect("temp dir");
    init_test_repo(repo.path(), "main", "Alice", "alice@example.com");
    fs::write(repo.path().join("app.rs"), "fn main() {}\n").expect("write app.rs");
    git_ok(repo.path(), &["add", "app.rs"]);
    git_ok(repo.path(), &["commit", "-m", "Initial commit"]);
    fs::write(
        repo.path().join("app.rs"),
        "fn main() { println!(\"ok\"); }\n",
    )
    .expect("update app.rs");
    git_ok(repo.path(), &["add", "app.rs"]);
    git_ok(repo.path(), &["commit", "-m", "Second commit"]);
    seed_repository_catalog_row(repo.path(), SEEDED_REPO_NAME, "main");

    let schema = crate::graphql::build_schema(crate::graphql::DevqlGraphqlContext::new(
        repo.path().to_path_buf(),
        super::super::super::db::DashboardDbPools::default(),
    ));

    let response = schema
        .execute(async_graphql::Request::new(
            r#"
            {
              repo(name: "demo") {
                branches {
                  name
                }
                users
                agents
                commits(first: 2) {
                  totalCount
                  edges {
                    node {
                      commitMessage
                      checkpoints(first: 1) {
                        totalCount
                      }
                    }
                  }
                }
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

    let json = response.data.into_json().expect("graphql data to json");
    assert_eq!(json["repo"]["branches"], json!([]));
    assert_eq!(json["repo"]["users"], json!([]));
    assert_eq!(json["repo"]["agents"], json!([]));
    assert_eq!(json["repo"]["commits"]["totalCount"], 2);
    assert_eq!(
        json["repo"]["commits"]["edges"][0]["node"]["commitMessage"],
        "Second commit"
    );
    assert_eq!(
        json["repo"]["commits"]["edges"][0]["node"]["checkpoints"]["totalCount"],
        0
    );
}

#[tokio::test]
async fn devql_repository_branch_selector_applies_live_branch_scope() {
    let repo = seed_dashboard_repo();
    let schema = crate::graphql::build_schema(crate::graphql::DevqlGraphqlContext::new(
        repo.path().to_path_buf(),
        super::super::super::db::DashboardDbPools::default(),
    ));

    let response = schema
        .execute(async_graphql::Request::new(
            r#"
            {
              repo(name: "demo") {
                branch(name: "main") {
                  defaultBranch
                  commits(first: 1) {
                    totalCount
                    edges {
                      node {
                        branch
                      }
                    }
                  }
                }
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
    let json = response.data.into_json().expect("graphql data to json");
    assert_eq!(json["repo"]["branch"]["defaultBranch"], "main");
    assert_eq!(json["repo"]["branch"]["commits"]["totalCount"], 2);
    assert_eq!(
        json["repo"]["branch"]["commits"]["edges"][0]["node"]["branch"],
        "main"
    );
}

#[tokio::test]
async fn devql_global_queries_fail_cleanly_when_repo_checkout_is_unknown() {
    let repo = seed_dashboard_repo();
    let registry_dir = TempDir::new().expect("temp dir");
    let unrelated_root = TempDir::new().expect("temp dir");
    let context = crate::graphql::DevqlGraphqlContext::for_global_request(
        repo.path().to_path_buf(),
        unrelated_root.path().to_path_buf(),
        Some(registry_dir.path().join("repo-path-registry.json")),
        super::super::super::db::DashboardDbPools::default(),
    );
    let schema = crate::graphql::build_schema(context);

    let response = schema
        .execute(async_graphql::Request::new(
            r#"
            {
              repo(name: "demo") {
                commits(first: 1) {
                  totalCount
                }
              }
            }
            "#,
        ))
        .await;

    assert_eq!(response.errors.len(), 1, "expected one graphql error");
    assert!(
        response.errors[0].message.contains("repo checkout unknown"),
        "unexpected graphql errors: {:?}",
        response.errors
    );
}
