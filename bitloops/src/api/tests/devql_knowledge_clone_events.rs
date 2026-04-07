use super::*;

#[tokio::test]
async fn devql_graphql_knowledge_queries_resolve_metadata_versions_relations_and_project_access() {
    let repo = seed_graphql_devql_repo();
    let seeded = seed_graphql_knowledge_data(repo.path());
    let schema = crate::graphql::build_schema(crate::graphql::DevqlGraphqlContext::new(
        repo.path().to_path_buf(),
        super::super::db::DashboardDbPools::default(),
    ));

    let response = schema
        .execute(async_graphql::Request::new(
            r#"
            {
              repo(name: "demo") {
                jiraOnly: knowledge(provider: JIRA, first: 10) {
                  totalCount
                }
                knowledge(first: 10) {
                  totalCount
                  edges {
                    node {
                      id
                      provider
                      sourceKind
                      canonicalExternalId
                      externalUrl
                      title
                      latestVersion {
                        id
                        contentHash
                        title
                        state
                        author
                        updatedAt
                        bodyPreview
                        createdAt
                        payload {
                          bodyText
                          bodyHtml
                          rawPayload
                        }
                      }
                      versions(first: 10) {
                        totalCount
                        edges {
                          node {
                            id
                            title
                            updatedAt
                            createdAt
                          }
                        }
                      }
                      relations(first: 10) {
                        totalCount
                        edges {
                          node {
                            targetType
                            targetId
                            targetVersionId
                            relationType
                            associationMethod
                            confidence
                            provenance
                          }
                        }
                      }
                    }
                  }
                }
                project(path: "src") {
                  knowledge(first: 10) {
                    totalCount
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
    assert_eq!(json["repo"]["jiraOnly"]["totalCount"], 1);
    assert_eq!(json["repo"]["knowledge"]["totalCount"], 2);
    assert_eq!(
        json["repo"]["knowledge"]["edges"][0]["node"]["id"],
        seeded.primary_item_id
    );
    assert_eq!(
        json["repo"]["knowledge"]["edges"][0]["node"]["provider"],
        "JIRA"
    );
    assert_eq!(
        json["repo"]["knowledge"]["edges"][0]["node"]["sourceKind"],
        "JIRA_ISSUE"
    );
    assert_eq!(
        json["repo"]["knowledge"]["edges"][0]["node"]["canonicalExternalId"],
        "https://bitloops.atlassian.net/browse/CLI-1521"
    );
    assert_eq!(
        json["repo"]["knowledge"]["edges"][0]["node"]["externalUrl"],
        "https://bitloops.atlassian.net/browse/CLI-1521"
    );
    assert_eq!(
        json["repo"]["knowledge"]["edges"][0]["node"]["title"],
        "Implement knowledge queries and payload loading"
    );
    assert_eq!(
        json["repo"]["knowledge"]["edges"][0]["node"]["latestVersion"]["id"],
        seeded.primary_latest_version_id
    );
    assert_eq!(
        json["repo"]["knowledge"]["edges"][0]["node"]["latestVersion"]["title"],
        "Implement knowledge queries and payload loading"
    );
    assert_eq!(
        json["repo"]["knowledge"]["edges"][0]["node"]["latestVersion"]["updatedAt"],
        "2026-03-26T09:30:00+00:00"
    );
    assert_eq!(
        json["repo"]["knowledge"]["edges"][0]["node"]["latestVersion"]["bodyPreview"],
        "Deliver the typed GraphQL knowledge model and lazy payload reads."
    );
    assert_eq!(
        json["repo"]["knowledge"]["edges"][0]["node"]["latestVersion"]["payload"]["bodyText"],
        "Deliver the typed GraphQL knowledge model and lazy payload reads."
    );
    assert_eq!(
        json["repo"]["knowledge"]["edges"][0]["node"]["latestVersion"]["payload"]["rawPayload"],
        json!({
            "key": "CLI-1521",
            "summary": "Implement knowledge queries and payload loading"
        })
    );
    assert_eq!(
        json["repo"]["knowledge"]["edges"][0]["node"]["versions"]["totalCount"],
        2
    );
    assert_eq!(
        json["repo"]["knowledge"]["edges"][0]["node"]["versions"]["edges"][0]["node"]["title"],
        "Implement knowledge queries and payload loading"
    );
    assert_eq!(
        json["repo"]["knowledge"]["edges"][0]["node"]["versions"]["edges"][1]["node"]["title"],
        "CLI-1521 draft design"
    );
    assert_eq!(
        json["repo"]["knowledge"]["edges"][0]["node"]["relations"]["totalCount"],
        1
    );
    assert_eq!(
        json["repo"]["knowledge"]["edges"][0]["node"]["relations"]["edges"][0]["node"]["targetType"],
        "KNOWLEDGE"
    );
    assert_eq!(
        json["repo"]["knowledge"]["edges"][0]["node"]["relations"]["edges"][0]["node"]["targetId"],
        seeded.secondary_item_id
    );
    assert_eq!(
        json["repo"]["knowledge"]["edges"][0]["node"]["relations"]["edges"][0]["node"]["targetVersionId"],
        seeded.secondary_latest_version_id
    );
    assert_eq!(
        json["repo"]["knowledge"]["edges"][0]["node"]["relations"]["edges"][0]["node"]["relationType"],
        "associated_with"
    );
    assert_eq!(
        json["repo"]["knowledge"]["edges"][0]["node"]["relations"]["edges"][0]["node"]["associationMethod"],
        "manual_attachment"
    );
    assert_eq!(
        json["repo"]["knowledge"]["edges"][0]["node"]["relations"]["edges"][0]["node"]["confidence"],
        0.9
    );
    assert_eq!(json["repo"]["project"]["knowledge"]["totalCount"], 2);
}

#[tokio::test]
async fn devql_graphql_knowledge_payloads_are_lazy_and_missing_blobs_return_null() {
    let repo = seed_graphql_devql_repo();
    let seeded = seed_graphql_knowledge_data(repo.path());
    let schema = crate::graphql::build_schema(crate::graphql::DevqlGraphqlContext::new(
        repo.path().to_path_buf(),
        super::super::db::DashboardDbPools::default(),
    ));

    let metadata_only = schema
        .execute(async_graphql::Request::new(
            r#"
            {
              repo(name: "demo") {
                knowledge(first: 10) {
                  edges {
                    node {
                      id
                      title
                      latestVersion {
                        id
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
        metadata_only.errors.is_empty(),
        "graphql errors: {:?}",
        metadata_only.errors
    );

    let metadata_json = metadata_only
        .data
        .into_json()
        .expect("graphql data to json");
    assert_eq!(
        metadata_json["repo"]["knowledge"]["edges"][0]["node"]["id"],
        seeded.primary_item_id
    );
    assert_eq!(
        metadata_json["repo"]["knowledge"]["edges"][1]["node"]["id"],
        seeded.secondary_item_id
    );

    let with_payloads = schema
        .execute(async_graphql::Request::new(
            r#"
            {
              repo(name: "demo") {
                knowledge(first: 10) {
                  edges {
                    node {
                      id
                      latestVersion {
                        payload {
                          bodyText
                          rawPayload
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
        with_payloads.errors.is_empty(),
        "graphql errors: {:?}",
        with_payloads.errors
    );

    let payload_json = with_payloads
        .data
        .into_json()
        .expect("graphql data to json");
    assert_eq!(
        payload_json["repo"]["knowledge"]["edges"][0]["node"]["latestVersion"]["payload"]["bodyText"],
        "Deliver the typed GraphQL knowledge model and lazy payload reads."
    );
    assert_eq!(
        payload_json["repo"]["knowledge"]["edges"][1]["node"]["latestVersion"]["payload"],
        Value::Null
    );
}

#[tokio::test]
async fn devql_graphql_knowledge_version_loader_caches_within_a_request_and_resets_per_request() {
    let repo = seed_graphql_devql_repo();
    seed_graphql_knowledge_data(repo.path());
    let context = crate::graphql::DevqlGraphqlContext::new(
        repo.path().to_path_buf(),
        super::super::db::DashboardDbPools::default(),
    );
    let schema = crate::graphql::build_schema(context.clone());
    let query = r#"
        {
          repo(name: "demo") {
            knowledge(first: 10) {
              edges {
                node {
                  versions(first: 10) {
                    totalCount
                  }
                  versionsAgain: versions(first: 10) {
                    totalCount
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
    assert_eq!(first_snapshot.knowledge_version_batches, 1);

    let second_response = schema.execute(async_graphql::Request::new(query)).await;
    assert!(
        second_response.errors.is_empty(),
        "graphql errors: {:?}",
        second_response.errors
    );
    let second_snapshot = context.loader_metrics_snapshot();
    assert_eq!(second_snapshot.knowledge_version_batches, 2);
}

#[tokio::test]
async fn devql_graphql_chat_history_loader_batches_within_a_request_and_resets_per_request() {
    let repo = seed_graphql_devql_repo();
    seed_graphql_chat_history_data(repo.path());
    let context = crate::graphql::DevqlGraphqlContext::new(
        repo.path().to_path_buf(),
        super::super::db::DashboardDbPools::default(),
    );
    let schema = crate::graphql::build_schema(context.clone());
    let query = r#"
        {
          repo(name: "demo") {
            caller: file(path: "src/caller.ts") {
              artefacts(filter: { symbolFqn: "src/caller.ts::caller" }, first: 10) {
                edges {
                  node {
                    symbolFqn
                    chatHistory(first: 10) {
                      totalCount
                      edges {
                        node {
                          sessionId
                          agent
                          role
                          content
                          metadata
                        }
                      }
                    }
                    chatHistoryAgain: chatHistory(first: 1) {
                      totalCount
                    }
                  }
                }
              }
            }
            target: file(path: "src/target.ts") {
              artefacts(filter: { symbolFqn: "src/target.ts::target" }, first: 10) {
                edges {
                  node {
                    symbolFqn
                    chatHistory(first: 10) {
                      totalCount
                      edges {
                        node {
                          sessionId
                          agent
                          role
                          content
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

    let json = first_response
        .data
        .into_json()
        .expect("graphql data to json");
    assert_eq!(
        json["repo"]["caller"]["artefacts"]["edges"][0]["node"]["chatHistory"]["totalCount"],
        2
    );
    assert_eq!(
        json["repo"]["caller"]["artefacts"]["edges"][0]["node"]["chatHistory"]["edges"][0]["node"]
            ["role"],
        "USER"
    );
    assert_eq!(
        json["repo"]["caller"]["artefacts"]["edges"][0]["node"]["chatHistory"]["edges"][0]["node"]
            ["content"],
        "Explain caller()"
    );
    assert_eq!(
        json["repo"]["caller"]["artefacts"]["edges"][0]["node"]["chatHistoryAgain"]["totalCount"],
        2
    );
    assert_eq!(
        json["repo"]["target"]["artefacts"]["edges"][0]["node"]["chatHistory"]["totalCount"],
        2
    );
    assert_eq!(
        json["repo"]["target"]["artefacts"]["edges"][0]["node"]["chatHistory"]["edges"][1]["node"]
            ["content"],
        "target() returns 42."
    );

    let first_snapshot = context.loader_metrics_snapshot();
    assert!(
        (1..=2).contains(&first_snapshot.chat_history_batches),
        "expected one or two chat-history batches for the first request, got {}",
        first_snapshot.chat_history_batches
    );

    let second_response = schema.execute(async_graphql::Request::new(query)).await;
    assert!(
        second_response.errors.is_empty(),
        "graphql errors: {:?}",
        second_response.errors
    );

    let second_snapshot = context.loader_metrics_snapshot();
    assert!(
        second_snapshot.chat_history_batches > first_snapshot.chat_history_batches,
        "expected the second request to schedule an additional chat-history batch"
    );
}

#[tokio::test]
async fn devql_graphql_chat_history_surfaces_backend_error_when_events_store_is_missing() {
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
                  artefacts(filter: { symbolFqn: "src/caller.ts::caller" }, first: 10) {
                    edges {
                      node {
                        chatHistory(first: 10) {
                          totalCount
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
        "chat-history should no longer depend on the events store: {:?}",
        response.errors
    );
    let json = response.data.into_json().expect("graphql data to json");
    assert_eq!(
        json["repo"]["file"]["artefacts"]["edges"][0]["node"]["chatHistory"]["totalCount"],
        0
    );
}

#[tokio::test]
async fn devql_graphql_clone_queries_resolve_project_and_artefact_results() {
    let repo = seed_graphql_monorepo_repo();
    seed_graphql_clone_data(repo.path());
    let schema = crate::graphql::build_schema(crate::graphql::DevqlGraphqlContext::new(
        repo.path().to_path_buf(),
        super::super::db::DashboardDbPools::default(),
    ));

    let response = schema
        .execute(async_graphql::Request::new(
            r#"
            {
              repo(name: "demo") {
                project(path: "packages/api") {
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
                        metadata
                        sourceArtefact {
                          symbolFqn
                        }
                        targetArtefact {
                          symbolFqn
                        }
                      }
                    }
                  }
                  file(path: "src/caller.ts") {
                    artefacts(filter: { symbolFqn: "packages/api/src/caller.ts::caller" }, first: 10) {
                      edges {
                        node {
                          clones(filter: { minScore: 0.70 }, first: 10) {
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
                                targetArtefact {
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
    assert_eq!(json["repo"]["project"]["clones"]["totalCount"], 1);
    assert_eq!(
        json["repo"]["project"]["clones"]["summary"]["totalCount"],
        1
    );
    assert_eq!(
        json["repo"]["project"]["clones"]["edges"][0]["node"]["relationKind"],
        "similar_implementation"
    );
    assert_eq!(
        json["repo"]["project"]["clones"]["summary"]["groups"][0]["relationKind"],
        "similar_implementation"
    );
    assert_eq!(
        json["repo"]["project"]["clones"]["summary"]["groups"][0]["count"],
        1
    );
    assert_eq!(
        json["repo"]["project"]["clones"]["edges"][0]["node"]["score"],
        0.93
    );
    assert_eq!(
        json["repo"]["project"]["clones"]["edges"][0]["node"]["sourceArtefact"]["symbolFqn"],
        "packages/api/src/caller.ts::caller"
    );
    assert_eq!(
        json["repo"]["project"]["clones"]["edges"][0]["node"]["targetArtefact"]["symbolFqn"],
        "packages/api/src/target.ts::target"
    );
    assert_eq!(
        json["repo"]["project"]["file"]["artefacts"]["edges"][0]["node"]["clones"]["totalCount"],
        2
    );
    assert_eq!(
        json["repo"]["project"]["file"]["artefacts"]["edges"][0]["node"]["clones"]["summary"]["totalCount"],
        2
    );
    assert_eq!(
        json["repo"]["project"]["file"]["artefacts"]["edges"][0]["node"]["clones"]["summary"]["groups"]
            [0]["relationKind"],
        "similar_implementation"
    );
    assert_eq!(
        json["repo"]["project"]["file"]["artefacts"]["edges"][0]["node"]["clones"]["summary"]["groups"]
            [0]["count"],
        2
    );
    assert_eq!(
        json["repo"]["project"]["file"]["artefacts"]["edges"][0]["node"]["clones"]["edges"][0]["node"]
            ["targetArtefact"]["symbolFqn"],
        "packages/api/src/target.ts::target"
    );
    assert_eq!(
        json["repo"]["project"]["file"]["artefacts"]["edges"][0]["node"]["clones"]["edges"][1]["node"]
            ["targetArtefact"]["symbolFqn"],
        "packages/web/src/page.ts::render"
    );
}

#[tokio::test]
async fn devql_graphql_clone_summary_queries_resolve_grouped_counts() {
    let repo = seed_graphql_monorepo_repo();
    seed_graphql_clone_data(repo.path());
    let schema = crate::graphql::build_schema(crate::graphql::DevqlGraphqlContext::new(
        repo.path().to_path_buf(),
        super::super::db::DashboardDbPools::default(),
    ));

    let response = schema
        .execute(async_graphql::Request::new(
            r#"
            {
              repo(name: "demo") {
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
                project(path: "packages/api") {
                  filtered: cloneSummary(
                    filter: { symbolFqn: "packages/api/src/caller.ts::caller" }
                    cloneFilter: { relationKind: "similar_implementation", minScore: 0.75 }
                  ) {
                    totalCount
                    groups {
                      relationKind
                      count
                    }
                  }
                  empty: cloneSummary(
                    filter: { symbolFqn: "packages/api/src/caller.ts::caller" }
                    cloneFilter: { relationKind: "exact_duplicate" }
                  ) {
                    totalCount
                    groups {
                      relationKind
                      count
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
    assert_eq!(json["repo"]["cloneSummary"]["totalCount"], 3);
    assert_eq!(
        json["repo"]["cloneSummary"]["groups"][0]["relationKind"],
        "similar_implementation"
    );
    assert_eq!(json["repo"]["cloneSummary"]["groups"][0]["count"], 2);
    assert_eq!(
        json["repo"]["cloneSummary"]["groups"][1]["relationKind"],
        "contextual_neighbor"
    );
    assert_eq!(json["repo"]["cloneSummary"]["groups"][1]["count"], 1);
    assert_eq!(json["repo"]["project"]["filtered"]["totalCount"], 1);
    assert_eq!(
        json["repo"]["project"]["filtered"]["groups"][0]["relationKind"],
        "similar_implementation"
    );
    assert_eq!(json["repo"]["project"]["filtered"]["groups"][0]["count"], 1);
    assert_eq!(json["repo"]["project"]["empty"]["totalCount"], 0);
    assert_eq!(
        json["repo"]["project"]["empty"]["groups"]
            .as_array()
            .expect("groups array")
            .len(),
        0
    );
}

#[tokio::test]
async fn devql_graphql_file_clone_summary_queries_resolve_grouped_counts() {
    let repo = seed_graphql_monorepo_repo();
    seed_graphql_clone_data(repo.path());
    let schema = crate::graphql::build_schema(crate::graphql::DevqlGraphqlContext::new(
        repo.path().to_path_buf(),
        super::super::db::DashboardDbPools::default(),
    ));

    let response = schema
        .execute(async_graphql::Request::new(
            r#"
            {
              repo(name: "demo") {
                file(path: "packages/api/src/caller.ts") {
                  cloneSummary(
                    filter: { symbolFqn: "packages/api/src/caller.ts::caller" }
                    cloneFilter: { minScore: 0.68 }
                  ) {
                    totalCount
                    groups {
                      relationKind
                      count
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
    assert_eq!(json["repo"]["file"]["cloneSummary"]["totalCount"], 2);
    assert_eq!(
        json["repo"]["file"]["cloneSummary"]["groups"][0]["relationKind"],
        "similar_implementation"
    );
    assert_eq!(
        json["repo"]["file"]["cloneSummary"]["groups"][0]["count"],
        2
    );
    assert_eq!(
        json["repo"]["file"]["cloneSummary"]["groups"]
            .as_array()
            .expect("groups array")
            .len(),
        1
    );
}

#[tokio::test]
async fn devql_graphql_same_file_method_clone_summaries_match_across_repo_file_and_artefact_views()
{
    let repo = seed_graphql_monorepo_repo();
    seed_graphql_same_file_method_clone_data(repo.path());
    let schema = crate::graphql::build_schema(crate::graphql::DevqlGraphqlContext::new(
        repo.path().to_path_buf(),
        super::super::db::DashboardDbPools::default(),
    ));

    let response = schema
        .execute(async_graphql::Request::new(
            r#"
            {
              repo(name: "demo") {
                cloneSummary(
                  filter: {
                    kind: METHOD
                    symbolFqn: "packages/api/src/change-path.ts::ChangePathOfCodeFileCommandHandler::execute"
                  }
                ) {
                  totalCount
                  groups {
                    relationKind
                    count
                  }
                }
                file(path: "packages/api/src/change-path.ts") {
                  cloneSummary(
                    filter: {
                      kind: METHOD
                      symbolFqn: "packages/api/src/change-path.ts::ChangePathOfCodeFileCommandHandler::execute"
                    }
                  ) {
                    totalCount
                    groups {
                      relationKind
                      count
                    }
                  }
                  artefacts(
                    filter: {
                      kind: METHOD
                      symbolFqn: "packages/api/src/change-path.ts::ChangePathOfCodeFileCommandHandler::execute"
                    }
                    first: 10
                  ) {
                    totalCount
                    edges {
                      node {
                        symbolFqn
                        clones(first: 10) {
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
                              targetArtefact {
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
    assert_eq!(json["repo"]["cloneSummary"]["totalCount"], 1);
    assert_eq!(
        json["repo"]["cloneSummary"]["groups"][0]["relationKind"],
        "weak_clone_candidate"
    );
    assert_eq!(json["repo"]["file"]["cloneSummary"]["totalCount"], 1);
    assert_eq!(
        json["repo"]["file"]["cloneSummary"]["groups"][0]["relationKind"],
        "weak_clone_candidate"
    );
    assert_eq!(json["repo"]["file"]["artefacts"]["totalCount"], 1);
    assert_eq!(
        json["repo"]["file"]["artefacts"]["edges"][0]["node"]["clones"]["totalCount"],
        1
    );
    assert_eq!(
        json["repo"]["file"]["artefacts"]["edges"][0]["node"]["clones"]["summary"]["totalCount"],
        1
    );
    assert_eq!(
        json["repo"]["file"]["artefacts"]["edges"][0]["node"]["clones"]["summary"]["groups"][0]["relationKind"],
        "weak_clone_candidate"
    );
    assert_eq!(
        json["repo"]["file"]["artefacts"]["edges"][0]["node"]["clones"]["edges"][0]["node"]["targetArtefact"]
            ["symbolFqn"],
        "packages/api/src/change-path.ts::ChangePathOfCodeFileCommandHandler::command"
    );
}

#[tokio::test]
async fn devql_graphql_clone_summary_validates_inputs_and_rejects_temporal_scopes() {
    let repo = seed_graphql_monorepo_repo();
    seed_graphql_clone_data(repo.path());
    let commit_sha = git_ok(repo.path(), &["rev-parse", "HEAD"]);
    let schema = crate::graphql::build_schema(crate::graphql::DevqlGraphqlContext::new(
        repo.path().to_path_buf(),
        super::super::db::DashboardDbPools::default(),
    ));

    let response = schema
        .execute(async_graphql::Request::new(format!(
            r#"
            {{
              repo(name: "demo") {{
                badScore: cloneSummary(cloneFilter: {{ minScore: 1.5 }}) {{
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
            }}
            "#,
        )))
        .await;

    let messages = response
        .errors
        .iter()
        .map(|error| error.message.as_str())
        .collect::<Vec<_>>();
    assert_eq!(messages.len(), 3, "unexpected errors: {messages:?}");
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
async fn devql_graphql_clone_source_target_loader_caches_within_a_request_and_resets_per_request() {
    let repo = seed_graphql_monorepo_repo();
    seed_graphql_clone_data(repo.path());
    let context = crate::graphql::DevqlGraphqlContext::new(
        repo.path().to_path_buf(),
        super::super::db::DashboardDbPools::default(),
    );
    let schema = crate::graphql::build_schema(context.clone());
    let query = r#"
        {
          repo(name: "demo") {
            project(path: "packages/api") {
              clones(filter: { minScore: 0.75 }, first: 10) {
                edges {
                  node {
                    sourceArtefact {
                      id
                    }
                    sourceAgain: sourceArtefact {
                      id
                    }
                    targetArtefact {
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
async fn devql_graphql_test_harness_pack_fields_resolve_typed_results() {
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
    let schema = crate::graphql::build_schema(crate::graphql::DevqlGraphqlContext::new(
        repo.path().to_path_buf(),
        super::super::db::DashboardDbPools::default(),
    ));

    let response = schema
        .execute(async_graphql::Request::new(format!(
            r#"
            {{
              repo(name: "demo") {{
                file(path: "src/caller.ts") {{
                  artefacts(filter: {{ symbolFqn: "src/caller.ts::caller" }}, first: 10) {{
                    edges {{
                      node {{
                        tests(minConfidence: 0.8, linkageSource: "static_analysis", first: 5) {{
                          artefact {{
                            artefactId
                            filePath
                          }}
                          coveringTests {{
                            testName
                            confidence
                            linkageSource
                          }}
                          summary {{
                            totalCoveringTests
                          }}
                        }}
                        coverage(first: 5) {{
                          artefact {{
                            artefactId
                          }}
                          coverage {{
                            coverageSource
                            lineCoveragePct
                            branchDataAvailable
                            uncoveredLines
                          }}
                          summary {{
                            uncoveredLineCount
                          }}
                        }}
                      }}
                    }}
                  }}
                }}
                asOf(input: {{ commit: "{commit_sha}" }}) {{
                  project(path: "src") {{
                    testsSummary {{
                      capability
                      stage
                      status
                      commitSha
                      coveragePresent
                      counts {{
                        testArtefacts
                        testArtefactEdges
                        testClassifications
                        coverageCaptures
                        coverageHits
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
        response.errors.is_empty(),
        "graphql errors: {:?}",
        response.errors
    );

    let json = response.data.into_json().expect("graphql data to json");
    let node = &json["repo"]["file"]["artefacts"]["edges"][0]["node"];
    assert_eq!(
        node["tests"][0]["artefact"]["artefactId"],
        json!("artefact::caller")
    );
    assert_eq!(
        node["tests"][0]["artefact"]["filePath"],
        json!("src/caller.ts")
    );
    assert_eq!(
        node["tests"][0]["coveringTests"][0]["testName"],
        json!("caller_tests")
    );
    assert_eq!(
        node["tests"][0]["coveringTests"][0]["linkageSource"],
        json!("static_analysis")
    );
    assert_eq!(node["tests"][0]["summary"]["totalCoveringTests"], json!(1));
    assert_eq!(
        node["coverage"][0]["coverage"]["coverageSource"],
        json!("lcov")
    );
    assert_eq!(
        node["coverage"][0]["coverage"]["lineCoveragePct"],
        json!(50.0)
    );
    assert_eq!(
        node["coverage"][0]["coverage"]["branchDataAvailable"],
        json!(true)
    );
    assert_eq!(
        node["coverage"][0]["coverage"]["uncoveredLines"],
        json!([5])
    );
    assert_eq!(
        node["coverage"][0]["summary"]["uncoveredLineCount"],
        json!(1)
    );
    assert_eq!(
        json["repo"]["asOf"]["project"]["testsSummary"]["commitSha"],
        json!(commit_sha)
    );
    assert_eq!(
        json["repo"]["asOf"]["project"]["testsSummary"]["coveragePresent"],
        json!(true)
    );
    assert_eq!(
        json["repo"]["asOf"]["project"]["testsSummary"]["stage"],
        json!("test_harness_tests_summary")
    );
    assert_eq!(
        json["repo"]["asOf"]["project"]["testsSummary"]["counts"]["testArtefacts"],
        json!(2)
    );
    assert_eq!(
        json["repo"]["asOf"]["project"]["testsSummary"]["counts"]["coverageCaptures"],
        json!(1)
    );
}

#[tokio::test]
async fn devql_graphql_project_typed_fields_respect_scope_and_extension_is_removed() {
    let repo = seed_graphql_monorepo_repo();
    let commit_sha = git_ok(repo.path(), &["rev-parse", "HEAD"]);
    seed_graphql_test_harness_stage_data(
        repo.path(),
        &commit_sha,
        &[
            (
                "sym::api-caller",
                "artefact::api-caller",
                "packages/api/src/caller.ts",
                "api_tests",
            ),
            (
                "sym::web-render",
                "artefact::web-render",
                "packages/web/src/page.ts",
                "web_tests",
            ),
        ],
    );
    let schema = crate::graphql::build_schema(crate::graphql::DevqlGraphqlContext::new(
        repo.path().to_path_buf(),
        super::super::db::DashboardDbPools::default(),
    ));

    let response = schema
        .execute(async_graphql::Request::new(format!(
            r#"
            {{
              repo(name: "demo") {{
                asOf(input: {{ commit: "{commit_sha}" }}) {{
                  project(path: "packages/api") {{
                    coverage(first: 10) {{
                      artefact {{
                        filePath
                      }}
                      coverage {{
                        coverageSource
                      }}
                    }}
                    testsSummary {{
                      commitSha
                      coveragePresent
                    }}
                  }}
                }}
              }}
            }}
            "#
        )))
        .await;

    assert!(
        response.errors.is_empty(),
        "graphql errors: {:?}",
        response.errors
    );

    let json = response.data.into_json().expect("graphql data to json");
    let rows = json["repo"]["asOf"]["project"]["coverage"]
        .as_array()
        .expect("coverage rows");
    assert_eq!(rows.len(), 4);
    assert!(
        rows.iter().all(|row| {
            row["artefact"]["filePath"]
                .as_str()
                .unwrap_or_default()
                .starts_with("packages/api/")
        }),
        "expected only project-scoped rows, got {rows:?}"
    );
    assert_eq!(
        json["repo"]["asOf"]["project"]["testsSummary"]["commitSha"],
        json!(commit_sha)
    );
    assert_eq!(
        json["repo"]["asOf"]["project"]["testsSummary"]["coveragePresent"],
        json!(true)
    );

    let bad_stage = schema
        .execute(async_graphql::Request::new(
            r#"
            {
              repo(name: "demo") {
                project(path: "packages/api") {
                  extension(stage: "unknown_stage", first: 10)
                }
              }
            }
            "#,
        ))
        .await;

    assert!(
        bad_stage
            .errors
            .iter()
            .any(|error| error.message.contains("Unknown field \"extension\"")),
        "unexpected error: {:?}",
        bad_stage.errors
    );

    let bad_args = schema
        .execute(async_graphql::Request::new(
            r#"
            {
              repo(name: "demo") {
                project(path: "packages/api") {
                  testsSummary {
                    commitSha
                  }
                }
              }
            }
            "#,
        ))
        .await;

    assert_eq!(bad_args.errors.len(), 1, "expected one graphql error");
    assert!(
        bad_args.errors[0]
            .message
            .contains("requires a resolved commit"),
        "unexpected error: {:?}",
        bad_args.errors
    );
}

#[tokio::test]
async fn devql_event_resolvers_query_duckdb_checkpoints_and_telemetry() {
    let repo = seed_graphql_monorepo_repo_with_duckdb_events();
    let schema = crate::graphql::build_schema(crate::graphql::DevqlGraphqlContext::new(
        repo.path().to_path_buf(),
        super::super::db::DashboardDbPools::default(),
    ));

    let response = schema
        .execute(async_graphql::Request::new(
            r#"
            {
              repo(name: "demo") {
                checkpoints(first: 10) {
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
                    }
                  }
                }
                telemetry(eventType: "tool_invocation", first: 10) {
                  totalCount
                  edges {
                    node {
                      id
                      sessionId
                      eventType
                      agent
                      eventTime
                      commitSha
                      branch
                      payload
                    }
                  }
                }
                project(path: "packages/api") {
                  checkpoints(first: 10) {
                    totalCount
                    edges {
                      node {
                        id
                        filesTouched
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
    assert_eq!(json["repo"]["checkpoints"]["totalCount"], 2);
    assert_eq!(
        json["repo"]["checkpoints"]["edges"][0]["node"]["id"],
        "checkpoint-web"
    );
    assert_eq!(
        json["repo"]["checkpoints"]["edges"][1]["node"]["id"],
        "checkpoint-api"
    );
    assert_eq!(
        json["repo"]["checkpoints"]["edges"][1]["node"]["filesTouched"],
        json!(["packages/api/src/caller.ts", "packages/api/src/target.ts"])
    );
    assert_eq!(json["repo"]["telemetry"]["totalCount"], 1);
    assert_eq!(
        json["repo"]["telemetry"]["edges"][0]["node"]["eventType"],
        "tool_invocation"
    );
    assert_eq!(
        json["repo"]["telemetry"]["edges"][0]["node"]["payload"],
        json!({"tool": "Edit", "path": "packages/api/src/caller.ts"})
    );
    assert_eq!(json["repo"]["project"]["checkpoints"]["totalCount"], 1);
    assert_eq!(
        json["repo"]["project"]["checkpoints"]["edges"][0]["node"]["id"],
        "checkpoint-api"
    );
}

#[tokio::test]
async fn devql_event_resolvers_surface_backend_errors_when_duckdb_store_is_missing() {
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
                checkpoints(first: 1) {
                  totalCount
                }
              }
            }
            "#,
        ))
        .await;

    assert!(
        response.errors.is_empty(),
        "checkpoint queries should no longer depend on the events store: {:?}",
        response.errors
    );
    let json = response.data.into_json().expect("graphql data to json");
    assert_eq!(json["repo"]["checkpoints"]["totalCount"], 0);
}

#[tokio::test]
async fn devql_event_checkpoint_commit_loader_batches_repository_checkpoint_reads() {
    let repo = seed_dashboard_repo_with_duckdb_events();
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
                checkpoints(first: 2) {
                  totalCount
                  edges {
                    node {
                      id
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
            "#,
        ))
        .await;

    assert!(
        response.errors.is_empty(),
        "graphql errors: {:?}",
        response.errors
    );

    let json = response.data.into_json().expect("graphql data to json");
    assert_eq!(json["repo"]["checkpoints"]["totalCount"], 2);
    assert_eq!(
        json["repo"]["checkpoints"]["edges"][0]["node"]["commit"]["sha"],
        json["repo"]["checkpoints"]["edges"][0]["node"]["commitAgain"]["sha"]
    );
    assert_eq!(
        json["repo"]["checkpoints"]["edges"][1]["node"]["commit"]["sha"],
        json["repo"]["checkpoints"]["edges"][1]["node"]["commitAgain"]["sha"]
    );

    let snapshot = context.loader_metrics_snapshot();
    assert_eq!(snapshot.commit_by_sha_batches, 1);
}
