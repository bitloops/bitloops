use super::*;

fn slim_schema_for_repo(repo_root: &Path) -> crate::graphql::SlimDevqlSchema {
    slim_schema_for_scope(repo_root, None)
}

fn slim_schema_for_scope(
    repo_root: &Path,
    project_path: Option<&str>,
) -> crate::graphql::SlimDevqlSchema {
    crate::graphql::build_slim_schema(crate::graphql::DevqlGraphqlContext::for_slim_request(
        repo_root.to_path_buf(),
        repo_root.to_path_buf(),
        Some("main".to_string()),
        project_path.map(str::to_string),
        None,
        true,
        super::super::db::DashboardDbPools::default(),
    ))
}

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

fn localhost_bind_available(test_name: &str) -> bool {
    match std::net::TcpListener::bind("127.0.0.1:0") {
        Ok(listener) => {
            drop(listener);
            true
        }
        Err(err) if err.kind() == std::io::ErrorKind::PermissionDenied => {
            eprintln!(
                "skipping {test_name}: loopback sockets are unavailable in this environment ({err})"
            );
            false
        }
        Err(err) => panic!("bind localhost for {test_name}: {err}"),
    }
}

fn enter_isolated_app_process_state(
    repo_root: &Path,
) -> (
    TempDir,
    crate::test_support::process_state::ProcessStateGuard,
) {
    let app_root = TempDir::new().expect("isolated app temp dir");
    let config_root = app_root.path().join("xdg-config");
    let data_root = app_root.path().join("xdg-data");
    let cache_root = app_root.path().join("xdg-cache");
    let state_root = app_root.path().join("xdg-state");

    let config_root_str = config_root.to_string_lossy().into_owned();
    let data_root_str = data_root.to_string_lossy().into_owned();
    let cache_root_str = cache_root.to_string_lossy().into_owned();
    let state_root_str = state_root.to_string_lossy().into_owned();

    let guard = enter_process_state(
        Some(repo_root),
        &[
            (
                "BITLOOPS_TEST_CONFIG_DIR_OVERRIDE",
                Some(config_root_str.as_str()),
            ),
            (
                "BITLOOPS_TEST_DATA_DIR_OVERRIDE",
                Some(data_root_str.as_str()),
            ),
            (
                "BITLOOPS_TEST_CACHE_DIR_OVERRIDE",
                Some(cache_root_str.as_str()),
            ),
            (
                "BITLOOPS_TEST_STATE_DIR_OVERRIDE",
                Some(state_root_str.as_str()),
            ),
        ],
    );

    (app_root, guard)
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
    let runtime_path = crate::daemon::repo_local_runtime_state_path_for_tests(temp.path())
        .unwrap_or_else(|| crate::daemon::runtime_state_path(temp.path()));
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
async fn slim_graphql_health_and_default_branch_after_init() {
    let repo = seed_graphql_mutation_repo();
    let schema = slim_schema_for_repo(repo.path());

    let init_response = schema
        .execute(async_graphql::Request::new(
            r#"mutation { initSchema { success } }"#,
        ))
        .await;
    assert!(
        init_response.errors.is_empty(),
        "graphql errors: {:?}",
        init_response.errors
    );

    let response = schema
        .execute(async_graphql::Request::new(
            r#"{
              health {
                relational { backend status }
                events { backend status }
                blob { backend status }
              }
              defaultBranch
            }"#,
        ))
        .await;

    assert!(
        response.errors.is_empty(),
        "graphql errors: {:?}",
        response.errors
    );
    let json = response.data.into_json().expect("graphql data to json");
    assert_eq!(json["health"]["relational"]["backend"], "sqlite");
    assert_eq!(json["health"]["events"]["backend"], "duckdb");
    assert_eq!(json["defaultBranch"], "main");
}

#[tokio::test]
async fn devql_mutations_initialise_schema_and_ingest_with_typed_results() {
    let repo = seed_graphql_mutation_repo();
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
    write_current_repo_runtime_state(repo.path());

    let ingest_response = schema
        .execute(async_graphql::Request::new(
            r#"
            mutation {
              ingest {
                success
                commitsProcessed
                checkpointCompanionsProcessed
                eventsInserted
                artefactsUpserted
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
    assert_eq!(ingest_json["ingest"]["commitsProcessed"], 1);
    assert_eq!(ingest_json["ingest"]["checkpointCompanionsProcessed"], 0);
    assert_eq!(ingest_json["ingest"]["eventsInserted"], 0);
    assert_eq!(ingest_json["ingest"]["artefactsUpserted"], 1);
    assert_eq!(ingest_json["ingest"]["semanticFeatureRowsUpserted"], 2);
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
async fn devql_ingest_mutation_with_backfill_limits_history_window() {
    use rusqlite::OptionalExtension;

    let repo = seed_graphql_mutation_repo();
    fs::write(
        repo.path().join("src/lib.rs"),
        "pub fn answer() -> i32 {\n    42\n}\n\npub fn second() -> i32 {\n    2\n}\n",
    )
    .expect("write second revision");
    git_ok(repo.path(), &["add", "."]);
    git_ok(repo.path(), &["commit", "-m", "add second"]);
    let head_sha = git_ok(repo.path(), &["rev-parse", "HEAD"]);
    let previous_sha = git_ok(repo.path(), &["rev-parse", "HEAD^"]);

    let sqlite_path = checkpoint_sqlite_path(repo.path());
    let schema = slim_schema_for_repo(repo.path());

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
    write_current_repo_runtime_state(repo.path());

    let ingest_response = schema
        .execute(async_graphql::Request::new(
            r#"
            mutation {
              ingest(input: { backfill: 1 }) {
                success
                commitsProcessed
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
    assert_eq!(ingest_json["ingest"]["commitsProcessed"], 1);

    let sqlite = rusqlite::Connection::open(sqlite_path).expect("open sqlite");
    let previous_ledger: Option<String> = sqlite
        .query_row(
            "SELECT history_status FROM commit_ingest_ledger WHERE commit_sha = ?1",
            rusqlite::params![previous_sha.as_str()],
            |row| row.get(0),
        )
        .optional()
        .expect("read previous ledger");
    let head_ledger: Option<String> = sqlite
        .query_row(
            "SELECT history_status FROM commit_ingest_ledger WHERE commit_sha = ?1",
            rusqlite::params![head_sha.as_str()],
            |row| row.get(0),
        )
        .optional()
        .expect("read head ledger");

    assert!(
        previous_ledger.is_none(),
        "backfill=1 should skip older history"
    );
    assert_eq!(head_ledger.as_deref(), Some("completed"));
}

#[tokio::test]
async fn devql_ingest_mutation_without_backfill_keeps_full_missing_segment() {
    use rusqlite::OptionalExtension;

    let repo = seed_graphql_mutation_repo();
    fs::write(
        repo.path().join("src/lib.rs"),
        "pub fn answer() -> i32 {\n    42\n}\n\npub fn second() -> i32 {\n    2\n}\n",
    )
    .expect("write second revision");
    git_ok(repo.path(), &["add", "."]);
    git_ok(repo.path(), &["commit", "-m", "add second"]);
    let head_sha = git_ok(repo.path(), &["rev-parse", "HEAD"]);
    let previous_sha = git_ok(repo.path(), &["rev-parse", "HEAD^"]);

    let sqlite_path = checkpoint_sqlite_path(repo.path());
    let schema = slim_schema_for_repo(repo.path());

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
    write_current_repo_runtime_state(repo.path());

    let ingest_response = schema
        .execute(async_graphql::Request::new(
            r#"
            mutation {
              ingest {
                success
                commitsProcessed
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
    assert_eq!(ingest_json["ingest"]["commitsProcessed"], 2);

    let sqlite = rusqlite::Connection::open(sqlite_path).expect("open sqlite");
    let previous_ledger: Option<String> = sqlite
        .query_row(
            "SELECT history_status FROM commit_ingest_ledger WHERE commit_sha = ?1",
            rusqlite::params![previous_sha.as_str()],
            |row| row.get(0),
        )
        .optional()
        .expect("read previous ledger");
    let head_ledger: Option<String> = sqlite
        .query_row(
            "SELECT history_status FROM commit_ingest_ledger WHERE commit_sha = ?1",
            rusqlite::params![head_sha.as_str()],
            |row| row.get(0),
        )
        .optional()
        .expect("read head ledger");

    assert_eq!(previous_ledger.as_deref(), Some("completed"));
    assert_eq!(head_ledger.as_deref(), Some("completed"));
}

#[tokio::test]
async fn enqueue_sync_rejects_conflicting_mode_selectors() {
    let repo = seed_graphql_mutation_repo();
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
async fn graphql_sync_mutation_is_not_exposed() {
    let repo = seed_graphql_mutation_repo();
    let schema = slim_schema_for_repo(repo.path());

    let response = schema
        .execute(async_graphql::Request::new(
            r#"
            mutation {
              sync(input: {}) {
                success
              }
            }
            "#,
        ))
        .await;

    assert_eq!(response.errors.len(), 1, "expected one graphql error");
    assert!(
        response.errors[0].message.contains("Unknown field")
            && response.errors[0].message.contains("sync"),
        "expected GraphQL schema validation error for removed sync mutation, got {:?}",
        response.errors
    );
}

#[tokio::test]
async fn enqueue_sync_without_selector_defaults_to_auto_mode() {
    let repo = seed_graphql_mutation_repo();
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
    if !localhost_bind_available("daemon_bootstrap_creates_devql_schema_tables") {
        return;
    }
    let repo = seed_graphql_mutation_repo();
    let (_app_root, _guard) = enter_isolated_app_process_state(repo.path());
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
    let schema = slim_schema_for_repo(repo.path());

    let missing_schema = schema
        .execute(async_graphql::Request::new(
            r#"
            mutation {
              ingest {
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
        Some(&async_graphql::Value::from("configuration"))
    );
    assert_eq!(
        backend_extensions.get("operation"),
        Some(&async_graphql::Value::from("ingest"))
    );
}

#[tokio::test]
async fn devql_mutations_manage_knowledge_and_apply_migrations() {
    if !localhost_bind_available("devql_mutations_manage_knowledge_and_apply_migrations") {
        return;
    }
    let repo = seed_graphql_knowledge_mutation_repo("https://seed.invalid");
    let server = match MockSequentialHttpServer::try_start(vec![
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
    ]) {
        Ok(server) => server,
        Err(err) if err.kind() == std::io::ErrorKind::PermissionDenied => {
            eprintln!(
                "skipping devql_mutations_manage_knowledge_and_apply_migrations: loopback sockets are unavailable in this environment ({err})"
            );
            return;
        }
        Err(err) => panic!("bind mock server: {err}"),
    };
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
    if !localhost_bind_available(
        "devql_mutations_surface_provider_and_reference_errors_for_knowledge_flows",
    ) {
        return;
    }
    let repo = seed_graphql_knowledge_mutation_repo("https://seed.invalid");
    let server = match MockSequentialHttpServer::try_start(vec![MockHttpResponse::json(
        500,
        json!({ "errorMessages": ["provider boom"] }),
    )]) {
        Ok(server) => server,
        Err(err) if err.kind() == std::io::ErrorKind::PermissionDenied => {
            eprintln!(
                "skipping devql_mutations_surface_provider_and_reference_errors_for_knowledge_flows: loopback sockets are unavailable in this environment ({err})"
            );
            return;
        }
        Err(err) => panic!("bind mock server: {err}"),
    };
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
    let schema = crate::graphql::build_schema(crate::graphql::DevqlGraphqlContext::new(
        repo.path().to_path_buf(),
        super::super::db::DashboardDbPools::default(),
    ));

    let response = schema
        .execute(async_graphql::Request::new(
            r#"
            mutation {
              ingest {
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

#[tokio::test]
async fn slim_select_artefacts_resolves_symbol_selection_and_empty_checkpoint_schema() {
    let repo = seed_graphql_devql_repo();
    let schema = slim_schema_for_repo(repo.path());

    let response = schema
        .execute(async_graphql::Request::new(
            r#"
            {
              selectArtefacts(by: { symbolFqn: "src/target.ts::target" }) {
                count
                artefacts {
                  path
                  symbolFqn
                }
                checkpoints {
                  summary
                  schema
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
    assert_eq!(json["selectArtefacts"]["count"], 1);
    assert_eq!(
        json["selectArtefacts"]["artefacts"][0]["path"],
        "src/target.ts"
    );
    assert_eq!(
        json["selectArtefacts"]["artefacts"][0]["symbolFqn"],
        "src/target.ts::target"
    );
    assert_eq!(
        json["selectArtefacts"]["checkpoints"]["summary"]["totalCount"],
        0
    );
    assert!(json["selectArtefacts"]["checkpoints"]["schema"].is_null());
}

#[tokio::test]
async fn slim_select_artefacts_resolves_project_scoped_relative_paths() {
    let repo = seed_graphql_monorepo_repo();
    let schema = slim_schema_for_scope(repo.path(), Some("packages/api"));

    let response = schema
        .execute(async_graphql::Request::new(
            r#"
            {
              selectArtefacts(by: { path: "src/caller.ts" }) {
                count
                artefacts {
                  path
                  symbolFqn
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
    assert_eq!(json["selectArtefacts"]["count"], 2);
    let artefacts = json["selectArtefacts"]["artefacts"]
        .as_array()
        .expect("artefacts array");
    assert!(
        artefacts
            .iter()
            .all(|artefact| artefact["path"] == "packages/api/src/caller.ts"),
        "unexpected artefact paths: {artefacts:?}"
    );
    assert!(
        artefacts
            .iter()
            .any(|artefact| artefact["symbolFqn"] == "packages/api/src/caller.ts::caller"),
        "expected project-scoped caller artefact, got {artefacts:?}"
    );
}

#[tokio::test]
async fn slim_select_artefacts_summary_aggregates_categories_and_deps_expose_items() {
    let repo = seed_graphql_monorepo_repo_with_duckdb_events();
    seed_graphql_clone_data(repo.path());
    let commit_sha = git_ok(repo.path(), &["rev-parse", "HEAD"]);
    seed_graphql_test_harness_stage_data(
        repo.path(),
        &commit_sha,
        &[(
            "sym::api-caller",
            "artefact::api-caller",
            "packages/api/src/caller.ts",
            "caller delegates to target",
        )],
    );
    let schema = slim_schema_for_scope(repo.path(), Some("packages/api"));

    let response = schema
        .execute(async_graphql::Request::new(
            r#"
            {
              selectArtefacts(by: { path: "src/caller.ts", lines: { start: 4, end: 6 } }) {
                count
                summary
                deps {
                  schema
                  items(first: 5) {
                    id
                    edgeKind
                    toSymbolRef
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
    assert_eq!(json["selectArtefacts"]["count"], 2);
    assert_eq!(
        json["selectArtefacts"]["summary"]["selectedArtefactCount"],
        2
    );
    assert_eq!(
        json["selectArtefacts"]["summary"]["checkpoints"]["summary"]["totalCount"],
        0
    );
    assert!(
        json["selectArtefacts"]["summary"]["checkpoints"]["schema"].is_null(),
        "expected empty checkpoint schema in aggregate summary: {json:#}"
    );
    assert_eq!(
        json["selectArtefacts"]["summary"]["clones"]["summary"]["totalCount"],
        2
    );
    assert_eq!(
        json["selectArtefacts"]["summary"]["deps"]["summary"]["totalCount"],
        2
    );
    assert_eq!(
        json["selectArtefacts"]["summary"]["tests"]["summary"]["matchedArtefactCount"],
        2
    );
    let aggregate_deps_schema = json["selectArtefacts"]["summary"]["deps"]["schema"]
        .as_str()
        .expect("aggregate dependency schema string");
    assert!(
        aggregate_deps_schema.contains("items(first: Int! = 20): [DependencyEdge!]!"),
        "expected aggregate summary to surface dependency items(...), got {aggregate_deps_schema}"
    );

    let deps_schema = json["selectArtefacts"]["deps"]["schema"]
        .as_str()
        .expect("dependency schema string");
    assert!(
        deps_schema.contains("items(first: Int! = 20): [DependencyEdge!]!"),
        "expected dependency schema to expose items(...), got {deps_schema}"
    );
    let deps_items = json["selectArtefacts"]["deps"]["items"]
        .as_array()
        .expect("dependency items array");
    assert_eq!(deps_items.len(), 2);
    assert_eq!(deps_items[0]["edgeKind"], "CALLS");
    assert_eq!(
        deps_items[0]["toSymbolRef"],
        "packages/api/src/target.ts::target"
    );
    assert_eq!(deps_items[1]["edgeKind"], "CALLS");
    assert_eq!(
        deps_items[1]["toSymbolRef"],
        "packages/web/src/page.ts::render"
    );
}
