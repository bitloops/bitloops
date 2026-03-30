use super::*;

#[tokio::test]
async fn devql_graphql_parent_loader_caches_within_a_request_and_resets_per_request() {
    let repo = seed_graphql_devql_repo();
    let context = crate::graphql::DevqlGraphqlContext::new(
        repo.path().to_path_buf(),
        super::super::super::db::DashboardDbPools::default(),
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
        super::super::super::db::DashboardDbPools::default(),
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
        super::super::super::db::DashboardDbPools::default(),
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
