use super::*;

#[tokio::test]
async fn devql_repository_file_and_artefact_queries_resolve_current_devql_graph() {
    let repo = seed_graphql_devql_repo();
    let schema = crate::graphql::build_schema(crate::graphql::DevqlGraphqlContext::new(
        repo.path().to_path_buf(),
        super::super::super::db::DashboardDbPools::default(),
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
        super::super::super::db::DashboardDbPools::default(),
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
async fn devql_artefact_connection_supports_reverse_pagination_for_graphql_artefacts() {
    let repo = seed_graphql_devql_repo();
    let schema = crate::graphql::build_schema(crate::graphql::DevqlGraphqlContext::new(
        repo.path().to_path_buf(),
        super::super::super::db::DashboardDbPools::default(),
    ));

    let tail_page = schema
        .execute(async_graphql::Request::new(
            r#"
            {
              repo(name: "demo") {
                artefacts(filter: { kind: FUNCTION }, last: 1) {
                  totalCount
                  pageInfo {
                    hasNextPage
                    hasPreviousPage
                    startCursor
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
        tail_page.errors.is_empty(),
        "graphql errors: {:?}",
        tail_page.errors
    );

    let tail_json = tail_page.data.into_json().expect("graphql data to json");
    assert_eq!(tail_json["repo"]["artefacts"]["totalCount"], 4);
    assert_eq!(
        tail_json["repo"]["artefacts"]["pageInfo"]["hasNextPage"],
        false
    );
    assert_eq!(
        tail_json["repo"]["artefacts"]["pageInfo"]["hasPreviousPage"],
        true
    );
    assert_eq!(
        tail_json["repo"]["artefacts"]["edges"][0]["node"]["symbolId"],
        "sym::target"
    );

    let before_cursor = tail_json["repo"]["artefacts"]["pageInfo"]["startCursor"]
        .as_str()
        .expect("tail artefact cursor")
        .to_string();

    let before_page = schema
        .execute(async_graphql::Request::new(format!(
            r#"
            {{
              repo(name: "demo") {{
                artefacts(filter: {{ kind: FUNCTION }}, last: 1, before: "{before_cursor}") {{
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
        before_page.errors.is_empty(),
        "graphql errors: {:?}",
        before_page.errors
    );

    let before_json = before_page.data.into_json().expect("graphql data to json");
    assert_eq!(
        before_json["repo"]["artefacts"]["pageInfo"]["hasNextPage"],
        true
    );
    assert_eq!(
        before_json["repo"]["artefacts"]["pageInfo"]["hasPreviousPage"],
        true
    );
    assert_eq!(
        before_json["repo"]["artefacts"]["edges"][0]["node"]["symbolId"],
        "sym::orphan"
    );
}

#[tokio::test]
async fn devql_graphql_event_backed_artefact_connections_paginate_repository_scope() {
    let seeded = seed_graphql_event_backed_repo();
    let schema = crate::graphql::build_schema(crate::graphql::DevqlGraphqlContext::new(
        seeded.repo.path().to_path_buf(),
        super::super::super::db::DashboardDbPools::default(),
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
        super::super::super::db::DashboardDbPools::default(),
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
        super::super::super::db::DashboardDbPools::default(),
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
        super::super::super::db::DashboardDbPools::default(),
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
async fn devql_graphql_artefact_resolvers_validate_paths_and_line_ranges() {
    let repo = seed_graphql_devql_repo();
    let schema = crate::graphql::build_schema(crate::graphql::DevqlGraphqlContext::new(
        repo.path().to_path_buf(),
        super::super::super::db::DashboardDbPools::default(),
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
