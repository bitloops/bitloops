use super::*;

#[tokio::test]
async fn devql_repository_queries_resolve_repo_commit_branch_user_agent_and_checkpoint_data() {
    let repo = seed_dashboard_repo();
    let schema = crate::graphql::build_schema(crate::graphql::DevqlGraphqlContext::new(
        repo.path().to_path_buf(),
        super::super::db::DashboardDbPools::default(),
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
async fn devql_commit_connection_supports_cursor_pagination() {
    let repo = seed_dashboard_repo();
    let schema = crate::graphql::build_schema(crate::graphql::DevqlGraphqlContext::new(
        repo.path().to_path_buf(),
        super::super::db::DashboardDbPools::default(),
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
        super::super::db::DashboardDbPools::default(),
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

    let schema = crate::graphql::build_schema(crate::graphql::DevqlGraphqlContext::new(
        repo.path().to_path_buf(),
        super::super::db::DashboardDbPools::default(),
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
async fn devql_repository_file_and_artefact_queries_resolve_current_devql_graph() {
    let repo = seed_graphql_devql_repo();
    let schema = crate::graphql::build_schema(crate::graphql::DevqlGraphqlContext::new(
        repo.path().to_path_buf(),
        super::super::db::DashboardDbPools::default(),
    ));

    let response = schema
        .execute(async_graphql::Request::new(
            r#"
            {
              repo(name: "demo") {
                files(path: "src/*.ts") {
                  path
                  language
                  blobSha
                }
                artefacts(filter: { kind: FUNCTION }, first: 10) {
                  totalCount
                  edges {
                    node {
                      id
                      symbolId
                      path
                      canonicalKind
                      symbolFqn
                      docstring
                    }
                  }
                }
                file(path: "src/caller.ts") {
                  path
                  language
                  blobSha
                  artefacts(first: 10) {
                    totalCount
                    edges {
                      node {
                        id
                        canonicalKind
                        symbolFqn
                        parentArtefactId
                        parent {
                          id
                          canonicalKind
                        }
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
    assert_eq!(
        json["repo"]["files"],
        json!([
            {
                "path": "src/caller.ts",
                "language": "typescript",
                "blobSha": "blob-caller"
            },
            {
                "path": "src/orphan.ts",
                "language": "typescript",
                "blobSha": "blob-orphan"
            },
            {
                "path": "src/target.ts",
                "language": "typescript",
                "blobSha": "blob-target"
            }
        ])
    );
    assert_eq!(json["repo"]["artefacts"]["totalCount"], 4);
    assert_eq!(
        json["repo"]["artefacts"]["edges"][0]["node"]["symbolId"],
        "sym::caller"
    );
    assert_eq!(
        json["repo"]["artefacts"]["edges"][0]["node"]["canonicalKind"],
        "FUNCTION"
    );
    assert_eq!(
        json["repo"]["artefacts"]["edges"][0]["node"]["docstring"],
        "Example docstring"
    );
    assert_eq!(json["repo"]["file"]["path"], "src/caller.ts");
    assert_eq!(json["repo"]["file"]["language"], "typescript");
    assert_eq!(json["repo"]["file"]["blobSha"], "blob-caller");
    assert_eq!(json["repo"]["file"]["artefacts"]["totalCount"], 3);
    assert_eq!(
        json["repo"]["file"]["artefacts"]["edges"][0]["node"]["canonicalKind"],
        "FILE"
    );
    assert_eq!(
        json["repo"]["file"]["artefacts"]["edges"][1]["node"]["symbolFqn"],
        "src/caller.ts::caller"
    );
    assert_eq!(
        json["repo"]["file"]["artefacts"]["edges"][1]["node"]["parentArtefactId"],
        "artefact::file-caller"
    );
    assert_eq!(
        json["repo"]["file"]["artefacts"]["edges"][1]["node"]["parent"]["id"],
        "artefact::file-caller"
    );
    assert_eq!(
        json["repo"]["file"]["artefacts"]["edges"][1]["node"]["parent"]["canonicalKind"],
        "FILE"
    );
}

#[tokio::test]
async fn devql_artefact_connection_supports_cursor_pagination_for_graphql_artefacts() {
    let repo = seed_graphql_devql_repo();
    let schema = crate::graphql::build_schema(crate::graphql::DevqlGraphqlContext::new(
        repo.path().to_path_buf(),
        super::super::db::DashboardDbPools::default(),
    ));

    let first_page = schema
        .execute(async_graphql::Request::new(
            r#"
            {
              repo(name: "demo") {
                artefacts(filter: { kind: FUNCTION }, first: 1) {
                  totalCount
                  pageInfo {
                    hasNextPage
                    hasPreviousPage
                    endCursor
                  }
                  edges {
                    node {
                      symbolId
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
    let cursor = first_json["repo"]["artefacts"]["pageInfo"]["endCursor"]
        .as_str()
        .expect("first artefact page cursor")
        .to_string();
    assert_eq!(first_json["repo"]["artefacts"]["totalCount"], 4);
    assert_eq!(
        first_json["repo"]["artefacts"]["pageInfo"]["hasNextPage"],
        true
    );
    assert_eq!(
        first_json["repo"]["artefacts"]["pageInfo"]["hasPreviousPage"],
        false
    );
    assert_eq!(
        first_json["repo"]["artefacts"]["edges"][0]["node"]["symbolId"],
        "sym::caller"
    );

    let second_page = schema
        .execute(async_graphql::Request::new(format!(
            r#"
            {{
              repo(name: "demo") {{
                artefacts(filter: {{ kind: FUNCTION }}, first: 1, after: "{cursor}") {{
                  totalCount
                  pageInfo {{
                    hasNextPage
                    hasPreviousPage
                  }}
                  edges {{
                    node {{
                      symbolId
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
    assert_eq!(second_json["repo"]["artefacts"]["totalCount"], 4);
    assert_eq!(
        second_json["repo"]["artefacts"]["pageInfo"]["hasPreviousPage"],
        true
    );
    assert_eq!(
        second_json["repo"]["artefacts"]["edges"][0]["node"]["symbolId"],
        "sym::helper"
    );
}

#[tokio::test]
async fn devql_graphql_event_backed_artefact_connections_paginate_repository_scope() {
    let seeded = seed_graphql_event_backed_repo();
    let schema = crate::graphql::build_schema(crate::graphql::DevqlGraphqlContext::new(
        seeded.repo.path().to_path_buf(),
        super::super::db::DashboardDbPools::default(),
    ));

    let first_page = schema
        .execute(async_graphql::Request::new(
            r#"
            {
              repo(name: "demo") {
                artefacts(filter: { kind: FUNCTION, agent: "codex" }, first: 1) {
                  totalCount
                  pageInfo {
                    hasNextPage
                    hasPreviousPage
                    endCursor
                  }
                  edges {
                    node {
                      symbolFqn
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
    let cursor = first_json["repo"]["artefacts"]["pageInfo"]["endCursor"]
        .as_str()
        .expect("first event-backed artefact page cursor")
        .to_string();
    assert_eq!(first_json["repo"]["artefacts"]["totalCount"], 2);
    assert_eq!(
        first_json["repo"]["artefacts"]["pageInfo"]["hasNextPage"],
        true
    );
    assert_eq!(
        first_json["repo"]["artefacts"]["pageInfo"]["hasPreviousPage"],
        false
    );
    assert_eq!(
        first_json["repo"]["artefacts"]["edges"][0]["node"]["symbolFqn"],
        "packages/api/src/caller.ts::callerCurrent"
    );

    let second_page = schema
        .execute(async_graphql::Request::new(format!(
            r#"
            {{
              repo(name: "demo") {{
                artefacts(filter: {{ kind: FUNCTION, agent: "codex" }}, first: 1, after: "{cursor}") {{
                  totalCount
                  pageInfo {{
                    hasNextPage
                    hasPreviousPage
                  }}
                  edges {{
                    node {{
                      symbolFqn
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
    assert_eq!(second_json["repo"]["artefacts"]["totalCount"], 2);
    assert_eq!(
        second_json["repo"]["artefacts"]["pageInfo"]["hasNextPage"],
        false
    );
    assert_eq!(
        second_json["repo"]["artefacts"]["pageInfo"]["hasPreviousPage"],
        true
    );
    assert_eq!(
        second_json["repo"]["artefacts"]["edges"][0]["node"]["symbolFqn"],
        "packages/api/src/target.ts::targetCurrent"
    );
}

#[tokio::test]
async fn devql_graphql_event_backed_artefact_connections_cover_project_file_and_historical_scopes()
{
    let seeded = seed_graphql_event_backed_repo();
    let schema = crate::graphql::build_schema(crate::graphql::DevqlGraphqlContext::new(
        seeded.repo.path().to_path_buf(),
        super::super::db::DashboardDbPools::default(),
    ));

    let response = schema
        .execute(async_graphql::Request::new(format!(
            r#"
            {{
              repo(name: "demo") {{
                project(path: "packages/api") {{
                  artefacts(filter: {{ kind: FUNCTION, since: "2026-03-26T00:00:00Z" }}, first: 10) {{
                    totalCount
                    edges {{
                      node {{
                        symbolFqn
                      }}
                    }}
                  }}
                  file(path: "src/copy.ts") {{
                    artefacts(filter: {{ kind: FUNCTION, agent: "codex" }}, first: 10) {{
                      totalCount
                    }}
                  }}
                }}
                history: asOf(input: {{ commit: "{}" }}) {{
                  project(path: "packages/api") {{
                    artefacts(filter: {{ kind: FUNCTION, agent: "codex" }}, first: 10) {{
                      totalCount
                      edges {{
                        node {{
                          symbolFqn
                        }}
                      }}
                    }}
                  }}
                }}
              }}
            }}
            "#,
            seeded.first_commit,
        )))
        .await;

    assert!(
        response.errors.is_empty(),
        "graphql errors: {:?}",
        response.errors
    );

    let json = response.data.into_json().expect("graphql data to json");
    assert_eq!(json["repo"]["project"]["artefacts"]["totalCount"], 2);
    assert_eq!(
        json["repo"]["project"]["artefacts"]["edges"][0]["node"]["symbolFqn"],
        "packages/api/src/caller.ts::callerCurrent"
    );
    assert_eq!(
        json["repo"]["project"]["artefacts"]["edges"][1]["node"]["symbolFqn"],
        "packages/api/src/target.ts::targetCurrent"
    );
    assert_eq!(
        json["repo"]["project"]["file"]["artefacts"]["totalCount"],
        0
    );
    assert_eq!(
        json["repo"]["history"]["project"]["artefacts"]["totalCount"],
        1
    );
    assert_eq!(
        json["repo"]["history"]["project"]["artefacts"]["edges"][0]["node"]["symbolFqn"],
        "packages/api/src/caller.ts::callerV1"
    );
}

#[tokio::test]
async fn devql_graphql_event_backed_artefact_connections_support_save_revision_scope() {
    let seeded = seed_graphql_save_revision_event_backed_repo();
    let schema = crate::graphql::build_schema(crate::graphql::DevqlGraphqlContext::new(
        seeded.repo.path().to_path_buf(),
        super::super::db::DashboardDbPools::default(),
    ));

    let response = schema
        .execute(async_graphql::Request::new(format!(
            r#"
            {{
              repo(name: "demo") {{
                asOf(input: {{ saveRevision: "{}" }}) {{
                  project(path: "packages/api") {{
                    artefacts(filter: {{ kind: FUNCTION, agent: "codex" }}, first: 10) {{
                      totalCount
                      edges {{
                        node {{
                          symbolFqn
                        }}
                      }}
                    }}
                  }}
                }}
              }}
            }}
            "#,
            seeded.save_revision,
        )))
        .await;

    assert!(
        response.errors.is_empty(),
        "graphql errors: {:?}",
        response.errors
    );

    let json = response.data.into_json().expect("graphql data to json");
    assert_eq!(
        json["repo"]["asOf"]["project"]["artefacts"]["totalCount"],
        2
    );
    assert_eq!(
        json["repo"]["asOf"]["project"]["artefacts"]["edges"][0]["node"]["symbolFqn"],
        "packages/api/src/caller.ts::callerTemp"
    );
    assert_eq!(
        json["repo"]["asOf"]["project"]["artefacts"]["edges"][1]["node"]["symbolFqn"],
        "packages/api/src/target.ts::targetTemp"
    );
}

#[tokio::test]
async fn devql_dependency_queries_resolve_direction_and_unresolved_targets() {
    let repo = seed_graphql_devql_repo();
    let schema = crate::graphql::build_schema(crate::graphql::DevqlGraphqlContext::new(
        repo.path().to_path_buf(),
        super::super::db::DashboardDbPools::default(),
    ));

    let response = schema
        .execute(async_graphql::Request::new(
            r#"
            {
              repo(name: "demo") {
                file(path: "src/caller.ts") {
                  deps(filter: { direction: BOTH, includeUnresolved: true }) {
                    totalCount
                    edges {
                      node {
                        id
                        edgeKind
                        toArtefactId
                        toSymbolRef
                        fromArtefact {
                          symbolFqn
                        }
                        toArtefact {
                          symbolFqn
                        }
                      }
                    }
                  }
                  artefacts(filter: { kind: FUNCTION }) {
                    edges {
                      node {
                        symbolFqn
                        outgoingDeps(filter: { includeUnresolved: true }) {
                          totalCount
                          edges {
                            node {
                              id
                              toArtefactId
                              toSymbolRef
                            }
                          }
                        }
                      }
                    }
                  }
                }
                artefacts(filter: { symbolFqn: "src/target.ts::target" }) {
                  edges {
                    node {
                      incomingDeps {
                        totalCount
                        edges {
                          node {
                            id
                            fromArtefact {
                              symbolFqn
                            }
                          }
                        }
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
    assert_eq!(json["repo"]["file"]["deps"]["totalCount"], 2);
    assert_eq!(
        json["repo"]["file"]["deps"]["edges"][0]["node"]["edgeKind"],
        "CALLS"
    );
    assert_eq!(
        json["repo"]["file"]["deps"]["edges"][0]["node"]["fromArtefact"]["symbolFqn"],
        "src/caller.ts::caller"
    );
    assert_eq!(
        json["repo"]["file"]["deps"]["edges"][0]["node"]["toArtefact"]["symbolFqn"],
        "src/target.ts::target"
    );
    assert_eq!(
        json["repo"]["file"]["deps"]["edges"][1]["node"]["toArtefactId"],
        serde_json::Value::Null
    );
    assert_eq!(
        json["repo"]["file"]["deps"]["edges"][1]["node"]["toSymbolRef"],
        "src/missing.ts::missing"
    );
    assert_eq!(
        json["repo"]["file"]["artefacts"]["edges"][0]["node"]["outgoingDeps"]["totalCount"],
        1
    );
    assert_eq!(
        json["repo"]["file"]["artefacts"]["edges"][1]["node"]["outgoingDeps"]["totalCount"],
        1
    );
    assert_eq!(
        json["repo"]["artefacts"]["edges"][0]["node"]["incomingDeps"]["totalCount"],
        1
    );
    assert_eq!(
        json["repo"]["artefacts"]["edges"][0]["node"]["incomingDeps"]["edges"][0]["node"]["fromArtefact"]
            ["symbolFqn"],
        "src/caller.ts::caller"
    );
}

#[tokio::test]
async fn devql_project_queries_scope_paths_and_isolate_cross_project_resolution() {
    let repo = seed_graphql_monorepo_repo();
    let schema = crate::graphql::build_schema(crate::graphql::DevqlGraphqlContext::new(
        repo.path().to_path_buf(),
        super::super::db::DashboardDbPools::default(),
    ));

    let response = schema
        .execute(async_graphql::Request::new(
            r#"
            {
              repo(name: "demo") {
                api: project(path: "packages/api") {
                  path
                  file(path: "src/caller.ts") {
                    path
                  }
                  files(path: "src/*.ts") {
                    path
                  }
                  artefacts(filter: { kind: FUNCTION }, first: 10) {
                    totalCount
                    edges {
                      node {
                        symbolFqn
                        path
                        outgoingDeps {
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
                web: project(path: "packages/web") {
                  path
                  artefacts(filter: { kind: FUNCTION }, first: 10) {
                    totalCount
                    edges {
                      node {
                        symbolFqn
                        path
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
    assert_eq!(json["repo"]["api"]["path"], "packages/api");
    assert_eq!(
        json["repo"]["api"]["file"]["path"],
        "packages/api/src/caller.ts"
    );
    assert_eq!(
        json["repo"]["api"]["files"],
        json!([
            { "path": "packages/api/src/caller.ts" },
            { "path": "packages/api/src/target.ts" }
        ])
    );
    assert_eq!(json["repo"]["api"]["artefacts"]["totalCount"], 2);
    assert_eq!(
        json["repo"]["api"]["artefacts"]["edges"][0]["node"]["symbolFqn"],
        "packages/api/src/caller.ts::caller"
    );
    assert_eq!(
        json["repo"]["api"]["artefacts"]["edges"][1]["node"]["symbolFqn"],
        "packages/api/src/target.ts::target"
    );
    assert_eq!(
        json["repo"]["api"]["artefacts"]["edges"][0]["node"]["outgoingDeps"]["totalCount"],
        2
    );
    assert_eq!(
        json["repo"]["api"]["artefacts"]["edges"][0]["node"]["outgoingDeps"]["edges"][0]["node"]["toArtefact"]
            ["symbolFqn"],
        "packages/api/src/target.ts::target"
    );
    assert_eq!(
        json["repo"]["api"]["artefacts"]["edges"][0]["node"]["outgoingDeps"]["edges"][1]["node"]["toSymbolRef"],
        "packages/web/src/page.ts::render"
    );
    assert_eq!(
        json["repo"]["api"]["artefacts"]["edges"][0]["node"]["outgoingDeps"]["edges"][1]["node"]["toArtefact"],
        serde_json::Value::Null
    );
    assert_eq!(json["repo"]["api"]["deps"]["totalCount"], 2);
    assert_eq!(
        json["repo"]["api"]["deps"]["edges"][1]["node"]["toArtefact"],
        serde_json::Value::Null
    );
    assert_eq!(json["repo"]["web"]["path"], "packages/web");
    assert_eq!(json["repo"]["web"]["artefacts"]["totalCount"], 1);
    assert_eq!(
        json["repo"]["web"]["artefacts"]["edges"][0]["node"]["symbolFqn"],
        "packages/web/src/page.ts::render"
    );
}

#[tokio::test]
async fn devql_temporal_queries_resolve_historical_scope_once_and_propagate_to_children() {
    let seeded = seed_graphql_temporal_repo();
    let schema = crate::graphql::build_schema(crate::graphql::DevqlGraphqlContext::new(
        seeded.repo.path().to_path_buf(),
        super::super::db::DashboardDbPools::default(),
    ));

    let response = schema
        .execute(async_graphql::Request::new(format!(
            r#"
            {{
              repo(name: "demo") {{
                repoScoped: asOf(input: {{ commit: "{}" }}) {{
                  resolvedCommit
                  project(path: "packages/api") {{
                    path
                    files(path: "src/*.ts") {{
                      path
                      blobSha
                    }}
                    file(path: "src/caller.ts") {{
                      path
                      artefacts(filter: {{ kind: FUNCTION }}, first: 10) {{
                        totalCount
                        edges {{
                          node {{
                            symbolFqn
                            outgoingDeps {{
                              totalCount
                              edges {{
                                node {{
                                  toArtefact {{
                                    symbolFqn
                                    path
                                  }}
                                }}
                              }}
                            }}
                          }}
                        }}
                      }}
                    }}
                    artefacts(filter: {{ kind: FUNCTION }}, first: 10) {{
                      totalCount
                      edges {{
                        node {{
                          symbolFqn
                          path
                        }}
                      }}
                    }}
                    deps(filter: {{ direction: OUT }}, first: 10) {{
                      totalCount
                      edges {{
                        node {{
                          toArtefact {{
                            symbolFqn
                            path
                          }}
                        }}
                      }}
                    }}
                  }}
                }}
                project(path: "packages/api") {{
                  projectScoped: asOf(input: {{ commit: "{}" }}) {{
                    resolvedCommit
                    artefacts(filter: {{ kind: FUNCTION }}, first: 10) {{
                      totalCount
                      edges {{
                        node {{
                          symbolFqn
                          path
                        }}
                      }}
                    }}
                  }}
                }}
              }}
            }}
            "#,
            seeded.first_commit, seeded.first_commit,
        )))
        .await;

    assert!(
        response.errors.is_empty(),
        "graphql errors: {:?}",
        response.errors
    );

    let json = response.data.into_json().expect("graphql data to json");
    assert_eq!(
        json["repo"]["repoScoped"]["resolvedCommit"],
        seeded.first_commit
    );
    assert_eq!(
        json["repo"]["repoScoped"]["project"]["path"],
        "packages/api"
    );
    assert_eq!(
        json["repo"]["repoScoped"]["project"]["files"],
        json!([
            {
                "path": "packages/api/src/caller.ts",
                "blobSha": "blob-api-caller-v1"
            },
            {
                "path": "packages/api/src/target.ts",
                "blobSha": "blob-api-target-v1"
            }
        ])
    );
    assert_eq!(
        json["repo"]["repoScoped"]["project"]["file"]["path"],
        "packages/api/src/caller.ts"
    );
    assert_eq!(
        json["repo"]["repoScoped"]["project"]["artefacts"]["totalCount"],
        2
    );
    assert_eq!(
        json["repo"]["repoScoped"]["project"]["artefacts"]["edges"][0]["node"]["symbolFqn"],
        "packages/api/src/caller.ts::caller"
    );
    assert_eq!(
        json["repo"]["repoScoped"]["project"]["artefacts"]["edges"][1]["node"]["symbolFqn"],
        "packages/api/src/target.ts::target"
    );
    assert_eq!(
        json["repo"]["repoScoped"]["project"]["file"]["artefacts"]["totalCount"],
        1
    );
    assert_eq!(
        json["repo"]["repoScoped"]["project"]["file"]["artefacts"]["edges"][0]["node"]["symbolFqn"],
        "packages/api/src/caller.ts::caller"
    );
    assert_eq!(
        json["repo"]["repoScoped"]["project"]["file"]["artefacts"]["edges"][0]["node"]["outgoingDeps"]
            ["totalCount"],
        1
    );
    assert_eq!(
        json["repo"]["repoScoped"]["project"]["file"]["artefacts"]["edges"][0]["node"]["outgoingDeps"]
            ["edges"][0]["node"]["toArtefact"]["symbolFqn"],
        "packages/api/src/target.ts::target"
    );
    assert_eq!(
        json["repo"]["repoScoped"]["project"]["deps"]["totalCount"],
        1
    );
    assert_eq!(
        json["repo"]["repoScoped"]["project"]["deps"]["edges"][0]["node"]["toArtefact"]["symbolFqn"],
        "packages/api/src/target.ts::target"
    );
    assert_eq!(
        json["repo"]["project"]["projectScoped"]["resolvedCommit"],
        seeded.first_commit
    );
    assert_eq!(
        json["repo"]["project"]["projectScoped"]["artefacts"]["totalCount"],
        2
    );
}

#[tokio::test]
async fn devql_temporal_queries_validate_inputs_and_unknown_refs() {
    let seeded = seed_graphql_temporal_repo();
    let schema = crate::graphql::build_schema(crate::graphql::DevqlGraphqlContext::new(
        seeded.repo.path().to_path_buf(),
        super::super::db::DashboardDbPools::default(),
    ));

    let invalid_selector = schema
        .execute(async_graphql::Request::new(
            r#"
            {
              repo(name: "demo") {
                asOf(input: { commit: "abc123", ref: "main" }) {
                  resolvedCommit
                }
              }
            }
            "#,
        ))
        .await;
    assert_eq!(
        invalid_selector.errors.len(),
        1,
        "expected invalid asOf selector error"
    );
    assert_eq!(
        invalid_selector.errors[0]
            .extensions
            .as_ref()
            .and_then(|extensions| extensions.get("code")),
        Some(&async_graphql::Value::from("BAD_USER_INPUT"))
    );

    let unknown_ref = schema
        .execute(async_graphql::Request::new(
            r#"
            {
              repo(name: "demo") {
                asOf(input: { ref: "refs/heads/missing-temporal-branch" }) {
                  resolvedCommit
                }
              }
            }
            "#,
        ))
        .await;
    assert_eq!(
        unknown_ref.errors.len(),
        1,
        "expected one unknown-ref error"
    );
    assert_eq!(
        unknown_ref.errors[0]
            .extensions
            .as_ref()
            .and_then(|extensions| extensions.get("code")),
        Some(&async_graphql::Value::from("BAD_USER_INPUT"))
    );
}

#[tokio::test]
async fn devql_project_queries_validate_project_paths() {
    let repo = seed_graphql_monorepo_repo();
    let schema = crate::graphql::build_schema(crate::graphql::DevqlGraphqlContext::new(
        repo.path().to_path_buf(),
        super::super::db::DashboardDbPools::default(),
    ));

    let invalid = schema
        .execute(async_graphql::Request::new(
            r#"
            {
              repo(name: "demo") {
                project(path: "../packages/api") {
                  path
                }
              }
            }
            "#,
        ))
        .await;
    assert_eq!(
        invalid.errors.len(),
        1,
        "expected invalid project path error"
    );
    assert_eq!(
        invalid.errors[0]
            .extensions
            .as_ref()
            .and_then(|extensions| extensions.get("code")),
        Some(&async_graphql::Value::from("BAD_USER_INPUT"))
    );

    let missing = schema
        .execute(async_graphql::Request::new(
            r#"
            {
              repo(name: "demo") {
                project(path: "packages/missing") {
                  path
                }
              }
            }
            "#,
        ))
        .await;
    assert_eq!(
        missing.errors.len(),
        1,
        "expected missing project path error"
    );
    assert_eq!(
        missing.errors[0]
            .extensions
            .as_ref()
            .and_then(|extensions| extensions.get("code")),
        Some(&async_graphql::Value::from("BAD_USER_INPUT"))
    );

    let not_directory = schema
        .execute(async_graphql::Request::new(
            r#"
            {
              repo(name: "demo") {
                project(path: "packages/api/src/caller.ts") {
                  path
                }
              }
            }
            "#,
        ))
        .await;
    assert_eq!(
        not_directory.errors.len(),
        1,
        "expected non-directory project path error"
    );
    assert_eq!(
        not_directory.errors[0]
            .extensions
            .as_ref()
            .and_then(|extensions| extensions.get("code")),
        Some(&async_graphql::Value::from("BAD_USER_INPUT"))
    );
}

#[tokio::test]
async fn devql_graphql_artefact_resolvers_validate_paths_and_line_ranges() {
    let repo = seed_graphql_devql_repo();
    let schema = crate::graphql::build_schema(crate::graphql::DevqlGraphqlContext::new(
        repo.path().to_path_buf(),
        super::super::db::DashboardDbPools::default(),
    ));

    let invalid_path = schema
        .execute(async_graphql::Request::new(
            r#"
            {
              repo(name: "demo") {
                file(path: "../src/caller.ts") {
                  path
                }
              }
            }
            "#,
        ))
        .await;
    assert_eq!(invalid_path.errors.len(), 1, "expected invalid path error");
    assert_eq!(
        invalid_path.errors[0]
            .extensions
            .as_ref()
            .and_then(|extensions| extensions.get("code")),
        Some(&async_graphql::Value::from("BAD_USER_INPUT"))
    );

    let missing_path = schema
        .execute(async_graphql::Request::new(
            r#"
            {
              repo(name: "demo") {
                file(path: "src/missing.ts") {
                  path
                }
              }
            }
            "#,
        ))
        .await;
    assert_eq!(missing_path.errors.len(), 1, "expected missing path error");
    assert_eq!(
        missing_path.errors[0]
            .extensions
            .as_ref()
            .and_then(|extensions| extensions.get("code")),
        Some(&async_graphql::Value::from("BAD_USER_INPUT"))
    );

    let invalid_lines = schema
        .execute(async_graphql::Request::new(
            r#"
            {
              repo(name: "demo") {
                artefacts(filter: { lines: { start: 10, end: 2 } }) {
                  totalCount
                }
              }
            }
            "#,
        ))
        .await;
    assert_eq!(
        invalid_lines.errors.len(),
        1,
        "expected invalid lines error"
    );
    assert_eq!(
        invalid_lines.errors[0]
            .extensions
            .as_ref()
            .and_then(|extensions| extensions.get("code")),
        Some(&async_graphql::Value::from("BAD_USER_INPUT"))
    );
}

#[tokio::test]
async fn devql_graphql_parent_loader_caches_within_a_request_and_resets_per_request() {
    let repo = seed_graphql_devql_repo();
    let context = crate::graphql::DevqlGraphqlContext::new(
        repo.path().to_path_buf(),
        super::super::db::DashboardDbPools::default(),
    );
    let schema = crate::graphql::build_schema(context.clone());
    let query = r#"
        {
          repo(name: "demo") {
            file(path: "src/caller.ts") {
              artefacts(filter: { kind: FUNCTION }, first: 10) {
                edges {
                  node {
                    symbolFqn
                    parent {
                      id
                    }
                    parentAgain: parent {
                      id
                    }
                  }
                }
              }
            }
          }
        }
    "#;

    let first_response = schema.execute(async_graphql::Request::new(query)).await;
    assert!(
        first_response.errors.is_empty(),
        "graphql errors: {:?}",
        first_response.errors
    );
    let first_snapshot = context.loader_metrics_snapshot();
    assert_eq!(first_snapshot.artefact_by_id_batches, 1);

    let second_response = schema.execute(async_graphql::Request::new(query)).await;
    assert!(
        second_response.errors.is_empty(),
        "graphql errors: {:?}",
        second_response.errors
    );
    let second_snapshot = context.loader_metrics_snapshot();
    assert_eq!(second_snapshot.artefact_by_id_batches, 2);
}

#[tokio::test]
async fn devql_graphql_dependency_loaders_batch_nested_edge_and_artefact_reads() {
    let repo = seed_graphql_devql_repo();
    let context = crate::graphql::DevqlGraphqlContext::new(
        repo.path().to_path_buf(),
        super::super::db::DashboardDbPools::default(),
    );
    let schema = crate::graphql::build_schema(context.clone());

    let response = schema
        .execute(async_graphql::Request::new(
            r#"
            {
              repo(name: "demo") {
                artefacts(filter: { kind: FUNCTION }, first: 10) {
                  edges {
                    node {
                      symbolFqn
                      outgoingDeps(filter: { includeUnresolved: true }) {
                        totalCount
                        edges {
                          node {
                            fromArtefact {
                              id
                            }
                            fromAgain: fromArtefact {
                              id
                            }
                            toArtefact {
                              id
                            }
                          }
                        }
                      }
                      incomingDeps {
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
    assert_eq!(
        json["repo"]["artefacts"]["edges"][0]["node"]["symbolFqn"],
        "src/caller.ts::caller"
    );
    assert_eq!(
        json["repo"]["artefacts"]["edges"][1]["node"]["symbolFqn"],
        "src/caller.ts::helper"
    );
    assert_eq!(
        json["repo"]["artefacts"]["edges"][2]["node"]["symbolFqn"],
        "src/orphan.ts::orphan"
    );
    assert_eq!(
        json["repo"]["artefacts"]["edges"][3]["node"]["symbolFqn"],
        "src/target.ts::target"
    );
    assert_eq!(
        json["repo"]["artefacts"]["edges"][0]["node"]["outgoingDeps"]["totalCount"],
        1
    );
    assert_eq!(
        json["repo"]["artefacts"]["edges"][1]["node"]["outgoingDeps"]["totalCount"],
        1
    );
    assert_eq!(
        json["repo"]["artefacts"]["edges"][2]["node"]["incomingDeps"]["totalCount"],
        0
    );
    assert_eq!(
        json["repo"]["artefacts"]["edges"][3]["node"]["incomingDeps"]["totalCount"],
        1
    );

    let snapshot = context.loader_metrics_snapshot();
    assert_eq!(snapshot.outgoing_edge_batches, 1);
    assert_eq!(snapshot.incoming_edge_batches, 1);
    assert_eq!(snapshot.artefact_by_id_batches, 1);
}

#[tokio::test]
async fn devql_graphql_commit_loader_caches_within_a_request_and_resets_per_request() {
    let repo = seed_dashboard_repo();
    let context = crate::graphql::DevqlGraphqlContext::new(
        repo.path().to_path_buf(),
        super::super::db::DashboardDbPools::default(),
    );
    let schema = crate::graphql::build_schema(context.clone());
    let query = r#"
        {
          repo(name: "demo") {
            commits(first: 1) {
              edges {
                node {
                  checkpoints(first: 1) {
                    edges {
                      node {
                        commit {
                          sha
                          branch
                        }
                        commitAgain: commit {
                          sha
                          branch
                        }
                      }
                    }
                  }
                }
              }
            }
          }
        }
    "#;

    let first_response = schema.execute(async_graphql::Request::new(query)).await;
    assert!(
        first_response.errors.is_empty(),
        "graphql errors: {:?}",
        first_response.errors
    );
    let first_snapshot = context.loader_metrics_snapshot();
    assert_eq!(first_snapshot.commit_by_sha_batches, 1);

    let second_response = schema.execute(async_graphql::Request::new(query)).await;
    assert!(
        second_response.errors.is_empty(),
        "graphql errors: {:?}",
        second_response.errors
    );
    let second_snapshot = context.loader_metrics_snapshot();
    assert_eq!(second_snapshot.commit_by_sha_batches, 2);
}

