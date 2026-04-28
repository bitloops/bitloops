use super::*;

#[tokio::test]
async fn devql_project_queries_scope_paths_and_isolate_cross_project_resolution() {
    let repo = seed_graphql_monorepo_repo();
    let schema = crate::graphql::build_schema(crate::graphql::DevqlGraphqlContext::new(
        repo.path().to_path_buf(),
        super::super::super::db::DashboardDbPools::default(),
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
                        outgoingDependencies {
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
                  dependencies(filter: { direction: OUT }, first: 10) {
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
        json["repo"]["api"]["artefacts"]["edges"][0]["node"]["outgoingDependencies"]["totalCount"],
        2
    );
    assert_eq!(
        json["repo"]["api"]["artefacts"]["edges"][0]["node"]["outgoingDependencies"]["edges"][0]["node"]
            ["toArtefact"]["symbolFqn"],
        "packages/api/src/target.ts::target"
    );
    assert_eq!(
        json["repo"]["api"]["artefacts"]["edges"][0]["node"]["outgoingDependencies"]["edges"][1]["node"]
            ["toSymbolRef"],
        "packages/web/src/page.ts::render"
    );
    assert_eq!(
        json["repo"]["api"]["artefacts"]["edges"][0]["node"]["outgoingDependencies"]["edges"][1]["node"]
            ["toArtefact"],
        serde_json::Value::Null
    );
    assert_eq!(json["repo"]["api"]["dependencies"]["totalCount"], 2);
    assert_eq!(
        json["repo"]["api"]["dependencies"]["edges"][1]["node"]["toArtefact"],
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
async fn devql_project_codecity_world_scopes_current_data_and_rejects_temporal_scopes() {
    fn assert_close(actual: &serde_json::Value, expected: f64) {
        let actual = actual.as_f64().expect("numeric JSON value");
        assert!(
            (actual - expected).abs() < 1e-6,
            "expected {expected}, got {actual}"
        );
    }

    let repo = seed_graphql_monorepo_repo();
    let repo_id = crate::host::devql::resolve_repo_id(repo.path()).expect("resolve repo id");
    let schema = crate::graphql::build_schema(crate::graphql::DevqlGraphqlContext::new(
        repo.path().to_path_buf(),
        super::super::super::db::DashboardDbPools::default(),
    ));

    let response = schema
        .execute(async_graphql::Request::new(
            r#"
            {
              repo(name: "demo") {
                project(path: "packages/api") {
                  codeCityWorld(includeDependencyArcs: true, first: 10) {
                    capability
                    stage
                    status
                    repoId
                    commitSha
                    summary {
                      fileCount
                      artefactCount
                      dependencyCount
                      includedFileCount
                      excludedFileCount
                      maxImportance
                      maxHeight
                    }
                    layout {
                      layoutKind
                      width
                      depth
                      gap
                    }
                    buildings {
                      path
                      importance {
                        score
                        blastRadius
                        weightedFanIn
                        articulationScore
                        normalizedBlastRadius
                        normalizedWeightedFanIn
                        normalizedArticulationScore
                      }
                      size {
                        loc
                        artefactCount
                        totalHeight
                      }
                      geometry {
                        x
                        z
                        width
                        depth
                        sideLength
                        footprintArea
                        height
                      }
                      floors {
                        name
                        canonicalKind
                        startLine
                        endLine
                        loc
                        floorIndex
                        floorHeight
                        colour
                        healthStatus
                      }
                    }
                    dependencyArcs {
                      fromPath
                      toPath
                      edgeCount
                      arcKind
                    }
                    diagnostics {
                      code
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
    let world = &json["repo"]["project"]["codeCityWorld"];
    assert_eq!(world["capability"], "codecity");
    assert_eq!(world["stage"], "codecity_world");
    assert_eq!(world["status"], "ok");
    assert_eq!(world["repoId"], repo_id);
    assert_eq!(world["commitSha"], serde_json::Value::Null);
    assert_eq!(world["summary"]["fileCount"], 3);
    assert_eq!(world["summary"]["artefactCount"], 2);
    assert_eq!(world["summary"]["dependencyCount"], 1);
    assert_eq!(world["summary"]["includedFileCount"], 2);
    assert_eq!(world["summary"]["excludedFileCount"], 1);
    assert_close(&world["summary"]["maxImportance"], 0.85);
    assert_close(&world["summary"]["maxHeight"], 0.36);
    assert_eq!(world["layout"]["layoutKind"], "phase1_grid_treemap");
    assert_close(&world["layout"]["gap"], 0.5);
    assert_close(&world["layout"]["width"], 13.141498903022176);
    assert_close(&world["layout"]["depth"], 11.641498903022176);

    let buildings = world["buildings"].as_array().expect("buildings array");
    assert_eq!(buildings.len(), 2);
    assert_eq!(buildings[0]["path"], "packages/api/src/target.ts");
    assert_eq!(buildings[1]["path"], "packages/api/src/caller.ts");

    assert_close(&buildings[0]["importance"]["score"], 0.85);
    assert_eq!(buildings[0]["importance"]["blastRadius"], 1);
    assert_close(
        &buildings[0]["importance"]["weightedFanIn"],
        0.6491228070166745,
    );
    assert_close(&buildings[0]["size"]["totalHeight"], 0.36);
    assert_eq!(buildings[0]["size"]["loc"], 3);
    assert_close(&buildings[0]["geometry"]["x"], 0.25);
    assert_close(&buildings[0]["geometry"]["z"], 0.25);
    assert_close(&buildings[0]["geometry"]["width"], 11.141498903022176);
    assert_close(&buildings[0]["geometry"]["height"], 0.36);
    assert_eq!(
        buildings[0]["floors"],
        json!([{
            "name": "target",
            "canonicalKind": "function",
            "startLine": 1,
            "endLine": 3,
            "loc": 3,
            "floorIndex": 0,
            "floorHeight": 0.36,
            "colour": "#888888",
            "healthStatus": "insufficient_data"
        }])
    );

    assert_close(&buildings[1]["importance"]["score"], 0.0);
    assert_eq!(buildings[1]["importance"]["blastRadius"], 0);
    assert_close(
        &buildings[1]["importance"]["weightedFanIn"],
        0.3508771929833254,
    );
    assert_eq!(buildings[1]["size"]["loc"], 3);
    assert_close(&buildings[1]["geometry"]["x"], 11.891498903022176);
    assert_close(&buildings[1]["geometry"]["z"], 0.25);
    assert_close(&buildings[1]["geometry"]["width"], 1.0);
    assert_close(&buildings[1]["geometry"]["height"], 0.36);
    assert_eq!(
        buildings[1]["floors"],
        json!([{
            "name": "caller",
            "canonicalKind": "function",
            "startLine": 4,
            "endLine": 6,
            "loc": 3,
            "floorIndex": 0,
            "floorHeight": 0.36,
            "colour": "#888888",
            "healthStatus": "insufficient_data"
        }])
    );

    assert_eq!(
        world["dependencyArcs"],
        json!([{
            "fromPath": "packages/api/src/caller.ts",
            "toPath": "packages/api/src/target.ts",
            "edgeCount": 1,
            "arcKind": "dependency"
        }])
    );

    let diagnostic_codes = world["diagnostics"]
        .as_array()
        .expect("diagnostics array")
        .iter()
        .map(|diagnostic| {
            diagnostic["code"]
                .as_str()
                .expect("diagnostic code")
                .to_string()
        })
        .collect::<Vec<_>>();
    assert!(diagnostic_codes.contains(&"codecity.source.cross_scope_edges_ignored".to_string()));
    assert!(diagnostic_codes.contains(&"codecity.health.deferred".to_string()));
    assert!(diagnostic_codes.contains(&"codecity.loc.line_span_phase1".to_string()));

    let commit_sha = git_ok(repo.path(), &["rev-parse", "HEAD"]);
    let temporal = schema
        .execute(async_graphql::Request::new(format!(
            r#"
            {{
              repo(name: "demo") {{
                asOf(input: {{ commit: "{commit_sha}" }}) {{
                  project(path: "packages/api") {{
                    codeCityWorld {{
                      status
                    }}
                  }}
                }}
              }}
            }}
            "#,
        )))
        .await;

    assert_eq!(
        temporal.errors.len(),
        1,
        "expected one temporal-scope validation error"
    );
    assert_eq!(
        temporal.errors[0]
            .extensions
            .as_ref()
            .and_then(|extensions| extensions.get("code")),
        Some(&async_graphql::Value::from("BAD_USER_INPUT"))
    );
    assert!(
        temporal.errors[0].message.contains(
            "`codeCityWorld` does not support historical or temporary `asOf(...)` scopes in phase 1"
        ),
        "unexpected error message: {}",
        temporal.errors[0].message
    );
}

#[tokio::test]
async fn devql_temporal_queries_resolve_historical_scope_once_and_propagate_to_children() {
    let seeded = seed_graphql_temporal_repo();
    let schema = crate::graphql::build_schema(crate::graphql::DevqlGraphqlContext::new(
        seeded.repo.path().to_path_buf(),
        super::super::super::db::DashboardDbPools::default(),
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
                            outgoingDependencies {{
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
                    dependencies(filter: {{ direction: OUT }}, first: 10) {{
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
        json["repo"]["repoScoped"]["project"]["file"]["artefacts"]["edges"][0]["node"]["outgoingDependencies"]
            ["totalCount"],
        1
    );
    assert_eq!(
        json["repo"]["repoScoped"]["project"]["file"]["artefacts"]["edges"][0]["node"]["outgoingDependencies"]
            ["edges"][0]["node"]["toArtefact"]["symbolFqn"],
        "packages/api/src/target.ts::target"
    );
    assert_eq!(
        json["repo"]["repoScoped"]["project"]["dependencies"]["totalCount"],
        1
    );
    assert_eq!(
        json["repo"]["repoScoped"]["project"]["dependencies"]["edges"][0]["node"]["toArtefact"]["symbolFqn"],
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
        super::super::super::db::DashboardDbPools::default(),
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
        super::super::super::db::DashboardDbPools::default(),
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
