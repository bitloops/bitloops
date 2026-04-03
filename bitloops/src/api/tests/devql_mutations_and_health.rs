use super::*;

fn slim_schema_for_repo(repo_root: &Path) -> crate::graphql::SlimDevqlSchema {
    crate::graphql::build_slim_schema(crate::graphql::DevqlGraphqlContext::for_slim_request(
        repo_root.to_path_buf(),
        repo_root.to_path_buf(),
        Some("main".to_string()),
        None,
        None,
        true,
        super::super::db::DashboardDbPools::default(),
    ))
}

fn assert_bad_user_input_error(
    response: &async_graphql::Response,
    operation: &str,
    expected_message_fragment: &str,
) {
    assert_eq!(response.errors.len(), 1, "expected one graphql error");
    let extensions = response.errors[0]
        .extensions
        .as_ref()
        .expect("graphql error extensions");
    assert_eq!(
        extensions.get("code"),
        Some(&async_graphql::Value::from("BAD_USER_INPUT"))
    );
    assert_eq!(
        extensions.get("kind"),
        Some(&async_graphql::Value::from("validation"))
    );
    assert_eq!(
        extensions.get("operation"),
        Some(&async_graphql::Value::from(operation))
    );
    assert!(
        response.errors[0]
            .message
            .contains(expected_message_fragment),
        "expected error message to contain `{expected_message_fragment}`, got `{}`",
        response.errors[0].message
    );
}

#[tokio::test]
async fn devql_schema_builds_and_executes_in_process() {
    let temp = TempDir::new().expect("temp dir");
    let repo_name = crate::host::devql::resolve_repo_identity(temp.path())
        .expect("resolve repo identity")
        .name;
    let schema = crate::graphql::build_schema(crate::graphql::DevqlGraphqlContext::new(
        temp.path().to_path_buf(),
        super::super::db::DashboardDbPools::default(),
    ));

    let response = schema
        .execute(async_graphql::Request::new(format!(
            r#"{{ repo(name: "{repo_name}") {{ id name provider organization }} }}"#
        )))
        .await;

    assert!(
        response.errors.is_empty(),
        "graphql errors: {:?}",
        response.errors
    );

    let json = response.data.into_json().expect("graphql data to json");
    assert_eq!(json["repo"]["name"], repo_name);
    assert_eq!(json["repo"]["provider"], "local");
}

#[tokio::test]
async fn global_mutation_updates_cli_telemetry_consent() {
    let temp = TempDir::new().expect("temp dir");
    let config_path = temp
        .path()
        .join(crate::config::BITLOOPS_CONFIG_RELATIVE_PATH);
    fs::write(
        &config_path,
        r#"[runtime]
local_dev = false
cli_version = "0.0.1"

[telemetry]
enabled = false
"#,
    )
    .expect("write daemon config");

    let schema = crate::graphql::build_schema(crate::graphql::DevqlGraphqlContext::new(
        temp.path().to_path_buf(),
        super::super::db::DashboardDbPools::default(),
    ));
    let runtime_path = crate::daemon::runtime_state_path(temp.path());
    let runtime_state = crate::daemon::DaemonRuntimeState {
        version: 1,
        config_path: config_path.clone(),
        config_root: temp.path().to_path_buf(),
        pid: std::process::id(),
        mode: crate::daemon::DaemonMode::Detached,
        service_name: None,
        url: "http://127.0.0.1:5667".to_string(),
        host: "127.0.0.1".to_string(),
        port: 5667,
        bundle_dir: temp.path().join("bundle"),
        relational_db_path: temp.path().join("relational.db"),
        events_db_path: temp.path().join("events.duckdb"),
        blob_store_path: temp.path().join("blob"),
        repo_registry_path: temp.path().join("repo-registry.json"),
        binary_fingerprint: "test".to_string(),
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

    let response = schema
        .execute(async_graphql::Request::new(format!(
            r#"
            mutation {{
              updateCliTelemetryConsent(cliVersion: "{version}") {{
                telemetry
                needsPrompt
              }}
            }}
            "#,
            version = crate::cli::telemetry_consent::CURRENT_CLI_VERSION,
        )))
        .await;

    assert!(
        response.errors.is_empty(),
        "graphql errors: {:?}",
        response.errors
    );

    let json = response.data.into_json().expect("graphql data to json");
    assert_eq!(
        json["updateCliTelemetryConsent"]["telemetry"],
        serde_json::Value::Null
    );
    assert_eq!(json["updateCliTelemetryConsent"]["needsPrompt"], true);

    let rendered = fs::read_to_string(&config_path).expect("read daemon config");
    assert!(rendered.contains(&format!(
        "cli_version = \"{}\"",
        crate::cli::telemetry_consent::CURRENT_CLI_VERSION
    )));
    assert!(!rendered.contains("enabled = false"));
}

#[tokio::test]
async fn devql_mutations_initialise_schema_and_ingest_with_typed_results() {
    let repo = seed_graphql_mutation_repo();
    let _guard = enter_process_state(Some(repo.path()), &[]);
    let sqlite_path = checkpoint_sqlite_path(repo.path());
    let schema = slim_schema_for_repo(repo.path());

    let init_response = schema
        .execute(async_graphql::Request::new(
            r#"
            mutation {
              initSchema {
                success
                repoIdentity
                repoId
                relationalBackend
                eventsBackend
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
    let init_json = init_response
        .data
        .into_json()
        .expect("graphql data to json");
    assert_eq!(init_json["initSchema"]["success"], true);
    assert_eq!(init_json["initSchema"]["relationalBackend"], "sqlite");
    assert_eq!(init_json["initSchema"]["eventsBackend"], "duckdb");

    let conn = rusqlite::Connection::open(&sqlite_path).expect("open sqlite");
    for table in ["repositories", "artefacts", "artefacts_current"] {
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
                [table],
                |row| row.get(0),
            )
            .expect("query sqlite schema");
        assert_eq!(count, 1, "expected sqlite table `{table}`");
    }

    let second_init = schema
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
        second_init.errors.is_empty(),
        "graphql errors: {:?}",
        second_init.errors
    );
    let second_init_json = second_init.data.into_json().expect("graphql data to json");
    assert_eq!(second_init_json["initSchema"]["success"], true);

    let ingest_response = schema
        .execute(async_graphql::Request::new(
            r#"
            mutation {
              ingest(input: { maxCheckpoints: 500 }) {
                success
                checkpointsProcessed
                eventsInserted
                artefactsUpserted
                checkpointsWithoutCommit
                temporaryRowsPromoted
                semanticFeatureRowsUpserted
                semanticFeatureRowsSkipped
                symbolEmbeddingRowsUpserted
                symbolEmbeddingRowsSkipped
                symbolCloneEdgesUpserted
                symbolCloneSourcesScored
              }
            }
            "#,
        ))
        .await;

    assert!(
        ingest_response.errors.is_empty(),
        "graphql errors: {:?}",
        ingest_response.errors
    );
    let ingest_json = ingest_response
        .data
        .into_json()
        .expect("graphql data to json");
    assert_eq!(ingest_json["ingest"]["success"], true);
    assert_eq!(ingest_json["ingest"]["checkpointsProcessed"], 0);
    assert_eq!(ingest_json["ingest"]["eventsInserted"], 0);
    assert_eq!(ingest_json["ingest"]["artefactsUpserted"], 0);
    assert_eq!(ingest_json["ingest"]["checkpointsWithoutCommit"], 0);
    assert_eq!(ingest_json["ingest"]["temporaryRowsPromoted"], 0);
    assert_eq!(ingest_json["ingest"]["semanticFeatureRowsUpserted"], 0);
    assert_eq!(ingest_json["ingest"]["semanticFeatureRowsSkipped"], 0);
    assert_eq!(ingest_json["ingest"]["symbolEmbeddingRowsUpserted"], 0);
    assert_eq!(ingest_json["ingest"]["symbolEmbeddingRowsSkipped"], 0);
    assert_eq!(ingest_json["ingest"]["symbolCloneEdgesUpserted"], 0);
    assert_eq!(ingest_json["ingest"]["symbolCloneSourcesScored"], 0);

    let repository_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM repositories", [], |row| row.get(0))
        .expect("count repositories");
    assert_eq!(repository_count, 1, "expected repository row after ingest");
}

#[tokio::test]
async fn enqueue_sync_rejects_conflicting_mode_selectors() {
    let repo = seed_graphql_mutation_repo();
    let _guard = enter_process_state(Some(repo.path()), &[]);
    let schema = slim_schema_for_repo(repo.path());

    let validate_and_full = schema
        .execute(async_graphql::Request::new(
            r#"
            mutation {
              enqueueSync(input: { validate: true, full: true }) {
                merged
              }
            }
            "#,
        ))
        .await;
    assert_bad_user_input_error(
        &validate_and_full,
        "enqueueSync",
        "at most one of `full`, `paths`, `repair`, or `validate` may be specified",
    );

    let repair_and_paths = schema
        .execute(async_graphql::Request::new(
            r#"
            mutation {
              enqueueSync(input: { repair: true, paths: ["src/lib.rs"] }) {
                merged
              }
            }
            "#,
        ))
        .await;
    assert_bad_user_input_error(
        &repair_and_paths,
        "enqueueSync",
        "at most one of `full`, `paths`, `repair`, or `validate` may be specified",
    );
}

#[tokio::test]
async fn sync_rejects_conflicting_mode_selectors() {
    let repo = seed_graphql_mutation_repo();
    let _guard = enter_process_state(Some(repo.path()), &[]);
    let schema = slim_schema_for_repo(repo.path());

    let validate_and_full = schema
        .execute(async_graphql::Request::new(
            r#"
            mutation {
              sync(input: { validate: true, full: true }) {
                success
              }
            }
            "#,
        ))
        .await;
    assert_bad_user_input_error(
        &validate_and_full,
        "sync",
        "at most one of `full`, `paths`, `repair`, or `validate` may be specified",
    );

    let repair_and_paths = schema
        .execute(async_graphql::Request::new(
            r#"
            mutation {
              sync(input: { repair: true, paths: ["src/lib.rs"] }) {
                success
              }
            }
            "#,
        ))
        .await;
    assert_bad_user_input_error(
        &repair_and_paths,
        "sync",
        "at most one of `full`, `paths`, `repair`, or `validate` may be specified",
    );
}

#[tokio::test]
async fn sync_without_selector_uses_the_default_auto_behaviour() {
    let repo = seed_graphql_mutation_repo();
    let _guard = enter_process_state(Some(repo.path()), &[]);
    let schema = slim_schema_for_repo(repo.path());

    let response = schema
        .execute(async_graphql::Request::new(
            r#"
            mutation {
              sync(input: {}) {
                success
                mode
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
    assert_eq!(json["sync"]["success"], true);
    assert_eq!(
        json["sync"]["mode"], "full",
        "auto sync requests currently execute with the full-workspace summary mode"
    );
}

#[tokio::test]
async fn enqueue_sync_without_selector_defaults_to_auto_mode() {
    let repo = seed_graphql_mutation_repo();
    let _guard = enter_process_state(Some(repo.path()), &[]);
    let schema = slim_schema_for_repo(repo.path());

    let response = schema
        .execute(async_graphql::Request::new(
            r#"
            mutation {
              enqueueSync(input: {}) {
                merged
                task {
                  mode
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
    assert_eq!(json["enqueueSync"]["task"]["mode"], "auto");
}

#[tokio::test]
async fn daemon_bootstrap_creates_devql_schema_tables() {
    let repo = seed_graphql_mutation_repo();
    let _guard = enter_process_state(Some(repo.path()), &[]);
    let sqlite_path = checkpoint_sqlite_path(repo.path());
    seed_repository_catalog_row(repo.path(), SEEDED_REPO_NAME, "main");
    seed_duckdb_events(repo.path(), &[]);

    let daemon = tokio::spawn(crate::api::run_with_options(
        crate::api::DashboardServerConfig {
            host: Some("127.0.0.1".to_string()),
            port: 0,
            no_open: true,
            force_http: true,
            recheck_local_dashboard_net: false,
            bundle_dir: None,
        },
        crate::api::DashboardRuntimeOptions {
            ready_subject: "Test daemon".to_string(),
            print_ready_banner: false,
            open_browser: false,
            shutdown_message: None,
            on_ready: None,
            on_shutdown: None,
            config_root: Some(repo.path().to_path_buf()),
            repo_registry_path: None,
        },
    ));

    let wait = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            if sqlite_path.exists() {
                let conn = rusqlite::Connection::open(&sqlite_path).expect("open sqlite");
                let required_tables = [
                    "repo_sync_state",
                    "current_file_state",
                    "artefacts_current",
                    "content_cache",
                ];
                let all_exist = required_tables.iter().all(|table| {
                    conn.query_row(
                        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
                        [*table],
                        |row| row.get::<_, i64>(0),
                    )
                    .map(|count| count == 1)
                    .unwrap_or(false)
                });
                if all_exist {
                    break;
                }
            }

            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
    })
    .await;

    if wait.is_err() && daemon.is_finished() {
        let result = daemon.await.expect("daemon join");
        panic!("daemon exited early: {result:#?}");
    }

    daemon.abort();
    let _ = daemon.await;

    assert!(wait.is_ok(), "schema tables were not bootstrapped in time");
}

#[tokio::test]
async fn devql_mutations_report_validation_and_backend_errors() {
    let repo = seed_graphql_mutation_repo();
    let _guard = enter_process_state(Some(repo.path()), &[]);
    let schema = slim_schema_for_repo(repo.path());

    let invalid_input = schema
        .execute(async_graphql::Request::new(
            r#"
            mutation {
              ingest(input: { maxCheckpoints: -1 }) {
                success
              }
            }
            "#,
        ))
        .await;
    assert_eq!(invalid_input.errors.len(), 1, "expected one graphql error");
    let invalid_extensions = invalid_input.errors[0]
        .extensions
        .as_ref()
        .expect("graphql error extensions");
    assert_eq!(
        invalid_extensions.get("code"),
        Some(&async_graphql::Value::from("BAD_USER_INPUT"))
    );
    assert_eq!(
        invalid_extensions.get("kind"),
        Some(&async_graphql::Value::from("validation"))
    );
    assert_eq!(
        invalid_extensions.get("operation"),
        Some(&async_graphql::Value::from("ingest"))
    );

    let missing_schema = schema
        .execute(async_graphql::Request::new(
            r#"
            mutation {
              ingest(input: { maxCheckpoints: 1 }) {
                success
              }
            }
            "#,
        ))
        .await;
    assert_eq!(missing_schema.errors.len(), 1, "expected one graphql error");
    let backend_extensions = missing_schema.errors[0]
        .extensions
        .as_ref()
        .expect("graphql error extensions");
    assert_eq!(
        backend_extensions.get("code"),
        Some(&async_graphql::Value::from("BACKEND_ERROR"))
    );
    assert_eq!(
        backend_extensions.get("kind"),
        Some(&async_graphql::Value::from("ingestion"))
    );
    assert_eq!(
        backend_extensions.get("operation"),
        Some(&async_graphql::Value::from("ingest"))
    );
}

#[tokio::test]
async fn devql_mutations_manage_knowledge_and_apply_migrations() {
    let repo = seed_graphql_knowledge_mutation_repo("https://seed.invalid");
    let _guard = enter_process_state(Some(repo.path()), &[]);
    let server = MockSequentialHttpServer::start(vec![
        MockHttpResponse::json(
            200,
            json!({
                "fields": {
                    "summary": "Knowledge item",
                    "status": { "name": "Open" },
                    "reporter": { "displayName": "Spiros" },
                    "updated": "2026-03-26T10:00:00Z",
                    "description": {
                        "type": "doc",
                        "content": [
                            {
                                "type": "paragraph",
                                "content": [{ "type": "text", "text": "First Jira body" }]
                            }
                        ]
                    }
                }
            }),
        ),
        MockHttpResponse::json(
            200,
            json!({
                "fields": {
                    "summary": "Knowledge item",
                    "status": { "name": "In Progress" },
                    "reporter": { "displayName": "Spiros" },
                    "updated": "2026-03-26T11:00:00Z",
                    "description": {
                        "type": "doc",
                        "content": [
                            {
                                "type": "paragraph",
                                "content": [{ "type": "text", "text": "Updated Jira body" }]
                            }
                        ]
                    }
                }
            }),
        ),
    ]);
    update_seeded_jira_site_url(repo.path(), server.url.as_str());
    let sqlite_path = checkpoint_sqlite_path(repo.path());
    let duckdb_path = knowledge_duckdb_path(repo.path());
    let schema = slim_schema_for_repo(repo.path());

    let apply_response = schema
        .execute(async_graphql::Request::new(
            r#"
            mutation {
              applyMigrations {
                success
                migrationsApplied {
                  packId
                  migrationName
                  description
                  appliedAt
                }
              }
            }
            "#,
        ))
        .await;
    assert!(
        apply_response.errors.is_empty(),
        "graphql errors: {:?}",
        apply_response.errors
    );
    let apply_json = apply_response
        .data
        .into_json()
        .expect("graphql data to json");
    assert_eq!(apply_json["applyMigrations"]["success"], true);
    let applied = apply_json["applyMigrations"]["migrationsApplied"]
        .as_array()
        .expect("migrationsApplied array");
    assert!(
        applied
            .iter()
            .any(|migration| migration["packId"] == "knowledge"),
        "expected knowledge pack migration in {applied:?}"
    );

    let add_response = schema
        .execute(async_graphql::Request::new(format!(
            r#"
            mutation {{
              addKnowledge(input: {{ url: "{}/browse/CLI-1525" }}) {{
                success
                knowledgeItemVersionId
                itemCreated
                newVersionCreated
                knowledgeItem {{
                  id
                  provider
                  sourceKind
                  externalUrl
                  latestVersion {{
                    id
                    title
                    bodyPreview
                  }}
                }}
              }}
            }}
            "#,
            server.url
        )))
        .await;
    assert!(
        add_response.errors.is_empty(),
        "graphql errors: {:?}",
        add_response.errors
    );
    let add_json = add_response.data.into_json().expect("graphql data to json");
    assert_eq!(add_json["addKnowledge"]["success"], true);
    assert_eq!(add_json["addKnowledge"]["itemCreated"], true);
    assert_eq!(add_json["addKnowledge"]["newVersionCreated"], true);
    assert_eq!(
        add_json["addKnowledge"]["knowledgeItem"]["provider"],
        "JIRA"
    );
    assert_eq!(
        add_json["addKnowledge"]["knowledgeItem"]["latestVersion"]["bodyPreview"],
        "First Jira body"
    );
    let knowledge_item_id = add_json["addKnowledge"]["knowledgeItem"]["id"]
        .as_str()
        .expect("knowledge item id")
        .to_string();
    let first_version_id = add_json["addKnowledge"]["knowledgeItemVersionId"]
        .as_str()
        .expect("knowledge item version id")
        .to_string();

    let associate_response = schema
        .execute(async_graphql::Request::new(format!(
            r#"
            mutation {{
              associateKnowledge(
                input: {{
                  sourceRef: "knowledge:{knowledge_item_id}"
                  targetRef: "commit:HEAD"
                }}
              ) {{
                success
                relation {{
                  id
                  targetType
                  targetId
                  relationType
                  associationMethod
                }}
              }}
            }}
            "#
        )))
        .await;
    assert!(
        associate_response.errors.is_empty(),
        "graphql errors: {:?}",
        associate_response.errors
    );
    let associate_json = associate_response
        .data
        .into_json()
        .expect("graphql data to json");
    assert_eq!(associate_json["associateKnowledge"]["success"], true);
    assert_eq!(
        associate_json["associateKnowledge"]["relation"]["targetType"],
        "COMMIT"
    );
    assert_eq!(
        associate_json["associateKnowledge"]["relation"]["relationType"],
        "associated_with"
    );

    let refresh_response = schema
        .execute(async_graphql::Request::new(format!(
            r#"
            mutation {{
              refreshKnowledge(input: {{ knowledgeRef: "knowledge:{knowledge_item_id}" }}) {{
                success
                latestDocumentVersionId
                contentChanged
                newVersionCreated
                knowledgeItem {{
                  id
                  latestVersion {{
                    id
                    title
                    bodyPreview
                  }}
                }}
              }}
            }}
            "#
        )))
        .await;
    assert!(
        refresh_response.errors.is_empty(),
        "graphql errors: {:?}",
        refresh_response.errors
    );
    let refresh_json = refresh_response
        .data
        .into_json()
        .expect("graphql data to json");
    assert_eq!(refresh_json["refreshKnowledge"]["success"], true);
    assert_eq!(refresh_json["refreshKnowledge"]["contentChanged"], true);
    assert_eq!(refresh_json["refreshKnowledge"]["newVersionCreated"], true);
    assert_ne!(
        refresh_json["refreshKnowledge"]["latestDocumentVersionId"],
        json!(first_version_id)
    );
    assert_eq!(
        refresh_json["refreshKnowledge"]["knowledgeItem"]["latestVersion"]["bodyPreview"],
        "Updated Jira body"
    );

    let sqlite = rusqlite::Connection::open(sqlite_path).expect("open sqlite");
    let knowledge_item_count: i64 = sqlite
        .query_row("SELECT COUNT(*) FROM knowledge_items", [], |row| row.get(0))
        .expect("count knowledge items");
    assert_eq!(knowledge_item_count, 1);
    let relation_count: i64 = sqlite
        .query_row(
            "SELECT COUNT(*) FROM knowledge_relation_assertions",
            [],
            |row| row.get(0),
        )
        .expect("count knowledge relations");
    assert_eq!(relation_count, 1);

    let duckdb = duckdb::Connection::open(duckdb_path).expect("open duckdb");
    let document_count: i64 = duckdb
        .query_row(
            "SELECT COUNT(*) FROM knowledge_document_versions",
            [],
            |row| row.get(0),
        )
        .expect("count knowledge versions");
    assert_eq!(document_count, 2);
}

#[tokio::test]
async fn devql_mutations_surface_provider_and_reference_errors_for_knowledge_flows() {
    let repo = seed_graphql_knowledge_mutation_repo("https://seed.invalid");
    let _guard = enter_process_state(Some(repo.path()), &[]);
    let server = MockSequentialHttpServer::start(vec![MockHttpResponse::json(
        500,
        json!({ "errorMessages": ["provider boom"] }),
    )]);
    update_seeded_jira_site_url(repo.path(), server.url.as_str());
    let schema = slim_schema_for_repo(repo.path());

    let provider_error = schema
        .execute(async_graphql::Request::new(format!(
            r#"
            mutation {{
              addKnowledge(input: {{ url: "{}/browse/CLI-1525" }}) {{
                success
              }}
            }}
            "#,
            server.url
        )))
        .await;
    assert_eq!(provider_error.errors.len(), 1, "expected one graphql error");
    let provider_extensions = provider_error.errors[0]
        .extensions
        .as_ref()
        .expect("graphql error extensions");
    assert_eq!(
        provider_extensions.get("code"),
        Some(&async_graphql::Value::from("BACKEND_ERROR"))
    );
    assert_eq!(
        provider_extensions.get("kind"),
        Some(&async_graphql::Value::from("provider"))
    );
    assert_eq!(
        provider_extensions.get("operation"),
        Some(&async_graphql::Value::from("addKnowledge"))
    );

    let invalid_reference = schema
        .execute(async_graphql::Request::new(
            r#"
            mutation {
              associateKnowledge(
                input: {
                  sourceRef: "knowledge:missing-item"
                  targetRef: "commit:HEAD"
                }
              ) {
                success
              }
            }
            "#,
        ))
        .await;
    assert_eq!(
        invalid_reference.errors.len(),
        1,
        "expected one graphql error"
    );
    let reference_extensions = invalid_reference.errors[0]
        .extensions
        .as_ref()
        .expect("graphql error extensions");
    assert_eq!(
        reference_extensions.get("code"),
        Some(&async_graphql::Value::from("BAD_USER_INPUT"))
    );
    assert_eq!(
        reference_extensions.get("kind"),
        Some(&async_graphql::Value::from("reference"))
    );
    assert_eq!(
        reference_extensions.get("operation"),
        Some(&async_graphql::Value::from("associateKnowledge"))
    );
}

#[tokio::test]
async fn devql_global_repo_mutations_require_slim_cli_scope() {
    let repo = seed_graphql_mutation_repo();
    let _guard = enter_process_state(Some(repo.path()), &[]);
    let schema = crate::graphql::build_schema(crate::graphql::DevqlGraphqlContext::new(
        repo.path().to_path_buf(),
        super::super::db::DashboardDbPools::default(),
    ));

    let response = schema
        .execute(async_graphql::Request::new(
            r#"
            mutation {
              ingest(input: { maxCheckpoints: 1 }) {
                success
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
    assert_eq!(
        extensions.get("kind"),
        Some(&async_graphql::Value::from("validation"))
    );
    assert_eq!(
        extensions.get("operation"),
        Some(&async_graphql::Value::from("ingest"))
    );
    assert!(
        response.errors[0]
            .message
            .contains("repo-scoped DevQL mutations require CLI repository scope")
    );
}

#[tokio::test]
async fn devql_health_query_reports_backend_and_blob_status_in_process() {
    let repo = seed_dashboard_repo();
    let schema = crate::graphql::build_schema(crate::graphql::DevqlGraphqlContext::new(
        repo.path().to_path_buf(),
        super::super::db::DashboardDbPools::default(),
    ));

    let response = schema
        .execute(async_graphql::Request::new(
            r#"{ health { relational { backend status connected } events { backend status connected } blob { backend status connected } } }"#,
        ))
        .await;

    assert!(
        response.errors.is_empty(),
        "graphql errors: {:?}",
        response.errors
    );

    let json = response.data.into_json().expect("graphql data to json");
    assert_eq!(json["health"]["relational"]["backend"], "sqlite");
    assert_eq!(json["health"]["relational"]["status"], "SKIP");
    assert_eq!(json["health"]["relational"]["connected"], false);
    assert_eq!(json["health"]["events"]["backend"], "duckdb");
    assert_eq!(json["health"]["events"]["status"], "SKIP");
    assert_eq!(json["health"]["events"]["connected"], false);
    assert_eq!(json["health"]["blob"]["backend"], "local");
    assert_eq!(json["health"]["blob"]["status"], "OK");
    assert_eq!(json["health"]["blob"]["connected"], true);
}

#[tokio::test]
async fn devql_health_query_surfaces_blob_bootstrap_errors() {
    let repo = seed_dashboard_repo();
    write_envelope_config(
        repo.path(),
        json!({
            "stores": {
                "blob": {
                    "s3_bucket": "bucket-a",
                    "gcs_bucket": "bucket-b"
                }
            }
        }),
    );
    let schema = crate::graphql::build_schema(crate::graphql::DevqlGraphqlContext::new(
        repo.path().to_path_buf(),
        super::super::db::DashboardDbPools::default(),
    ));

    let response = schema
        .execute(async_graphql::Request::new(
            r#"{ health { blob { backend status connected detail } } }"#,
        ))
        .await;

    assert!(
        response.errors.is_empty(),
        "graphql errors: {:?}",
        response.errors
    );

    let json = response.data.into_json().expect("graphql data to json");
    assert_eq!(json["health"]["blob"]["backend"], "invalid");
    assert_eq!(json["health"]["blob"]["status"], "FAIL");
    assert_eq!(json["health"]["blob"]["connected"], false);
    assert!(
        json["health"]["blob"]["detail"]
            .as_str()
            .expect("blob detail string")
            .contains("both s3_bucket and gcs_bucket are set")
    );
}
