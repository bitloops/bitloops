use super::*;
use crate::test_support::log_capture::capture_logs_async;

fn dashboard_app(repo_root: &Path, mode: ServeMode, bundle_dir: PathBuf) -> axum::Router {
    build_dashboard_router(test_state(repo_root.to_path_buf(), mode, bundle_dir))
}

fn dashboard_health_query() -> &'static str {
    r#"
    {
      health {
        relational { connected backend status detail }
        events { connected backend status detail }
        blob { connected backend status detail }
      }
    }
    "#
}

fn dashboard_kpis_query() -> &'static str {
    r#"
    {
      kpis(branch: "main") {
        totalCommits
        totalCheckpoints
        totalAgents
        totalSessions
        filesTouchedCount
        inputTokens
        outputTokens
        cacheCreationTokens
        cacheReadTokens
        apiCallCount
        averageTokensPerCheckpoint
        averageSessionsPerCheckpoint
      }
    }
    "#
}

fn record_dashboard_interaction(
    repo_root: &Path,
    session_id: &str,
    turn_id: &str,
    turn_number: u32,
) {
    use crate::host::checkpoints::lifecycle::interaction::resolve_interaction_spool;
    use crate::host::interactions::store::InteractionSpool;
    use crate::host::interactions::types::{InteractionSession, InteractionTurn};

    let spool = resolve_interaction_spool(repo_root).expect("resolve interaction spool");
    let transcript_path = repo_root.join(format!("{session_id}-transcript.jsonl"));
    fs::write(
        &transcript_path,
        format!(
            "{{\"type\":\"user\",\"content\":\"Session {session_id}\"}}\n{{\"type\":\"assistant\",\"content\":\"Turn {turn_id}\"}}\n"
        ),
    )
    .expect("write interaction transcript");

    let timestamp = format!("2026-04-01T10:{:02}:00Z", turn_number.min(59));
    spool
        .record_session(&InteractionSession {
            session_id: session_id.to_string(),
            repo_id: spool.repo_id().to_string(),
            branch: "main".to_string(),
            actor_id: "actor-2".to_string(),
            actor_name: "Bob".to_string(),
            actor_email: "bob@example.com".to_string(),
            actor_source: "bitloops-session".to_string(),
            agent_type: "codex".to_string(),
            model: "gpt-5.4".to_string(),
            first_prompt: format!("Start {session_id}"),
            transcript_path: transcript_path.to_string_lossy().to_string(),
            worktree_path: repo_root.to_string_lossy().to_string(),
            worktree_id: "worktree-2".to_string(),
            started_at: timestamp.clone(),
            ended_at: None,
            last_event_at: timestamp.clone(),
            updated_at: timestamp.clone(),
        })
        .expect("record interaction session");
    spool
        .record_turn(&InteractionTurn {
            turn_id: turn_id.to_string(),
            session_id: session_id.to_string(),
            repo_id: spool.repo_id().to_string(),
            branch: "main".to_string(),
            actor_id: "actor-2".to_string(),
            actor_name: "Bob".to_string(),
            actor_email: "bob@example.com".to_string(),
            actor_source: "bitloops-session".to_string(),
            turn_number,
            prompt: format!("Prompt {turn_id}"),
            agent_type: "codex".to_string(),
            model: "gpt-5.4".to_string(),
            started_at: timestamp.clone(),
            ended_at: None,
            token_usage: None,
            summary: format!("Summary {turn_id}"),
            prompt_count: 1,
            transcript_offset_start: Some(0),
            transcript_offset_end: Some(64),
            transcript_fragment: format!("Fragment {turn_id}"),
            files_modified: vec!["app.rs".to_string()],
            checkpoint_id: None,
            updated_at: timestamp,
        })
        .expect("record interaction turn");
}

#[tokio::test]
async fn dashboard_post_route_executes_graphql_requests() {
    let repo = seed_dashboard_repo();
    let app = dashboard_app(
        repo.path(),
        ServeMode::HelloWorld,
        repo.path().to_path_buf(),
    );

    let (status, payload) = request_dashboard_graphql(
        app,
        r#"
        {
          repositories {
            name
            provider
          }
          health {
            blob {
              backend
              connected
            }
          }
        }
        "#,
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(payload["data"]["repositories"][0]["name"], SEEDED_REPO_NAME);
    assert_eq!(payload["data"]["repositories"][0]["provider"], "local");
    assert_eq!(payload["data"]["health"]["blob"]["backend"], "local");
    assert_eq!(payload["data"]["health"]["blob"]["connected"], true);
}

#[tokio::test]
async fn dashboard_playground_route_serves_explorer() {
    let repo = seed_dashboard_repo();
    let app = dashboard_app(
        repo.path(),
        ServeMode::HelloWorld,
        repo.path().to_path_buf(),
    );

    let (status, body) = request_text(app.clone(), "/devql/dashboard").await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("DevQL Dashboard Explorer"));
    assert!(body.contains("/devql/dashboard"));
    assert!(body.contains("/devql/dashboard/ws"));

    let (status, body) = request_text(app, "/devql/dashboard/playground").await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("DevQL Dashboard Explorer"));
    assert!(body.contains("/devql/dashboard/ws"));
}

#[tokio::test]
async fn dashboard_sdl_route_returns_schema_text() {
    let repo = seed_dashboard_repo();
    let app = dashboard_app(
        repo.path(),
        ServeMode::HelloWorld,
        repo.path().to_path_buf(),
    );

    let (status, body) = request_text(app, "/devql/dashboard/sdl").await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, crate::api::dashboard_schema::dashboard_schema_sdl());
    assert!(body.contains("type DashboardQueryRoot"));
    assert!(body.contains("type DashboardMutationRoot"));
    assert!(body.contains("type DashboardSubscriptionRoot"));
    assert!(body.contains("health: HealthStatus!"));
    assert!(body.contains("repositories: [DashboardRepository!]!"));
    assert!(body.contains("kpis("));
    assert!(body.contains("interactionKpis("));
    assert!(body.contains("interactionSessions("));
    assert!(body.contains("interactionSession("));
    assert!(body.contains("interactionActors("));
    assert!(body.contains("interactionCommitAuthors("));
    assert!(body.contains("interactionAgents("));
    assert!(body.contains("searchInteractionSessions("));
    assert!(body.contains("searchInteractionTurns("));
    assert!(body.contains("interactionUpdates("));
    assert!(body.contains("fetchBundle: DashboardFetchBundleResult!"));
    assert!(!body.contains("postgres:"));
    assert!(!body.contains("clickhouse:"));
}

#[tokio::test]
async fn dashboard_ws_route_is_registered() {
    let repo = seed_dashboard_repo();
    let app = dashboard_app(
        repo.path(),
        ServeMode::HelloWorld,
        repo.path().to_path_buf(),
    );

    let (status, _) = request_text_with_method(app, Method::GET, "/devql/dashboard/ws").await;

    assert_ne!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn dashboard_ws_protocol_defaults_when_subprotocol_header_is_missing() {
    let request = Request::builder()
        .uri("/devql/dashboard/ws")
        .body(())
        .expect("request");
    let (mut parts, _) = request.into_parts();

    let extracted = <crate::api::dashboard_schema::DashboardWsProtocol as axum::extract::FromRequestParts<
        (),
    >>::from_request_parts(&mut parts, &())
    .await;

    assert!(extracted.is_ok(), "expected fallback protocol extraction");
    assert!(!parts.headers.contains_key("sec-websocket-protocol"));
}

#[tokio::test]
async fn dashboard_ws_protocol_preserves_explicit_subprotocol_header() {
    let request = Request::builder()
        .uri("/devql/dashboard/ws")
        .header("sec-websocket-protocol", "graphql-ws")
        .body(())
        .expect("request");
    let (mut parts, _) = request.into_parts();

    let extracted = <crate::api::dashboard_schema::DashboardWsProtocol as axum::extract::FromRequestParts<
        (),
    >>::from_request_parts(&mut parts, &())
    .await;

    assert!(extracted.is_ok(), "expected explicit protocol extraction");
    assert!(parts.headers.contains_key("sec-websocket-protocol"));
}

#[tokio::test]
async fn dashboard_interaction_updates_subscription_emits_when_interactions_change() {
    let repo = seed_dashboard_repo();
    let state = test_state(
        repo.path().to_path_buf(),
        ServeMode::HelloWorld,
        repo.path().to_path_buf(),
    );
    let schema = crate::api::dashboard_schema::build_dashboard_schema(state);
    let (event_tx, mut event_rx) = tokio::sync::mpsc::unbounded_channel();
    let stream = schema.execute_stream(async_graphql::Request::new(
        r#"
        subscription {
          interactionUpdates {
            repoId
            sessionCount
            turnCount
            latestSessionId
            latestTurnId
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

    let initial = tokio::time::timeout(std::time::Duration::from_secs(5), event_rx.recv())
        .await
        .expect("initial interaction update should arrive")
        .expect("initial interaction update payload");
    assert!(
        initial.errors.is_empty(),
        "subscription errors: {:?}",
        initial.errors
    );
    let initial_json = initial
        .data
        .into_json()
        .expect("initial interaction update json");
    assert_eq!(
        initial_json["interactionUpdates"]["sessionCount"],
        Value::from(1)
    );
    assert_eq!(
        initial_json["interactionUpdates"]["turnCount"],
        Value::from(1)
    );
    assert_eq!(
        initial_json["interactionUpdates"]["latestSessionId"],
        Value::from("session-1")
    );
    assert_eq!(
        initial_json["interactionUpdates"]["latestTurnId"],
        Value::from("turn-1")
    );

    record_dashboard_interaction(repo.path(), "session-2", "turn-2", 2);

    let updated = tokio::time::timeout(std::time::Duration::from_secs(5), event_rx.recv())
        .await
        .expect("updated interaction snapshot should arrive")
        .expect("updated interaction payload");
    assert!(
        updated.errors.is_empty(),
        "subscription errors: {:?}",
        updated.errors
    );
    let updated_json = updated
        .data
        .into_json()
        .expect("updated interaction update json");
    assert_eq!(
        updated_json["interactionUpdates"]["sessionCount"],
        Value::from(2)
    );
    assert_eq!(
        updated_json["interactionUpdates"]["turnCount"],
        Value::from(2)
    );
    assert_eq!(
        updated_json["interactionUpdates"]["latestSessionId"],
        Value::from("session-2")
    );
    assert_eq!(
        updated_json["interactionUpdates"]["latestTurnId"],
        Value::from("turn-2")
    );
}

#[test]
fn checked_in_dashboard_schema_file_matches_runtime_sdl() {
    let expected = crate::api::dashboard_schema::dashboard_schema_sdl();
    let schema_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("schema.dashboard.graphql");
    let actual =
        fs::read_to_string(&schema_path).expect("read checked-in schema.dashboard.graphql");
    assert_eq!(actual, expected);
}

#[tokio::test]
async fn dashboard_interaction_queries_return_session_detail_buckets_and_search_hits() {
    let repo = seed_dashboard_repo();
    let app = dashboard_app(
        repo.path(),
        ServeMode::HelloWorld,
        repo.path().to_path_buf(),
    );

    let (status, payload) = request_dashboard_graphql(
        app,
        r#"
        {
          interactionKpis(filter: { actorEmail: "alice@example.com" }) {
            totalSessions
            totalTurns
            totalCheckpoints
            totalToolUses
            totalActors
            totalAgents
            inputTokens
            outputTokens
          }
          interactionSessions(filter: { branch: "main", hasCheckpoint: true }) {
            sessionId
            branch
            actor {
              email
            }
            agentType
            model
            turnCount
            checkpointCount
            tokenUsage {
              inputTokens
              outputTokens
            }
            toolUses {
              toolKind
              taskDescription
            }
            latestCommitAuthor {
              checkpointId
              email
            }
          }
          interactionActors {
            actorEmail
            sessionCount
            turnCount
          }
          interactionCommitAuthors {
            authorEmail
            checkpointCount
            sessionCount
            turnCount
          }
          interactionAgents {
            key
            sessionCount
            turnCount
          }
          interactionSession(sessionId: "session-1") {
            summary {
              sessionId
              checkpointCount
              toolUses {
                toolKind
              }
            }
            turns {
              turnId
              checkpointId
              summary
              toolUses {
                toolKind
              }
            }
            rawEvents {
              eventType
              toolKind
              toolUseId
              actor {
                email
              }
            }
          }
          searchInteractionSessions(input: { query: "dashboard" }) {
            score
            matchedFields
            session {
              sessionId
              checkpointCount
            }
          }
          searchInteractionTurns(input: { query: "dashboard" }) {
            score
            matchedFields
            turn {
              turnId
              sessionId
            }
            session {
              sessionId
            }
          }
        }
        "#,
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        payload["data"]["interactionKpis"]["totalSessions"].as_u64(),
        Some(1)
    );
    assert_eq!(
        payload["data"]["interactionKpis"]["totalTurns"].as_u64(),
        Some(1)
    );
    assert_eq!(
        payload["data"]["interactionKpis"]["totalCheckpoints"].as_u64(),
        Some(1)
    );
    assert_eq!(
        payload["data"]["interactionKpis"]["totalToolUses"].as_u64(),
        Some(1)
    );
    assert_eq!(
        payload["data"]["interactionSessions"][0]["sessionId"],
        "session-1"
    );
    assert_eq!(
        payload["data"]["interactionSessions"][0]["actor"]["email"],
        "alice@example.com"
    );
    assert_eq!(
        payload["data"]["interactionSessions"][0]["toolUses"][0]["toolKind"],
        "edit"
    );
    assert_eq!(
        payload["data"]["interactionActors"][0]["actorEmail"],
        "alice@example.com"
    );
    assert_eq!(
        payload["data"]["interactionCommitAuthors"][0]["authorEmail"],
        "alice@example.com"
    );
    assert_eq!(
        payload["data"]["interactionAgents"][0]["key"],
        "claude-code"
    );
    assert_eq!(
        payload["data"]["interactionSession"]["summary"]["sessionId"],
        "session-1"
    );
    assert_eq!(
        payload["data"]["interactionSession"]["turns"][0]["turnId"],
        "turn-1"
    );
    assert_eq!(
        payload["data"]["interactionSession"]["turns"][0]["checkpointId"],
        "aabbccddeeff"
    );
    assert_eq!(
        payload["data"]["interactionSession"]["rawEvents"][0]["actor"]["email"],
        "alice@example.com"
    );
    assert_eq!(
        payload["data"]["searchInteractionSessions"][0]["session"]["sessionId"],
        "session-1"
    );
    assert_eq!(
        payload["data"]["searchInteractionTurns"][0]["turn"]["turnId"],
        "turn-1"
    );
    assert_eq!(
        payload["data"]["searchInteractionTurns"][0]["session"]["sessionId"],
        "session-1"
    );
}

#[test]
fn interaction_commit_author_buckets_do_not_block_on_relational_write_locks() {
    let repo = seed_dashboard_repo();
    let sqlite_path = checkpoint_sqlite_path(repo.path());
    let lock_conn = rusqlite::Connection::open(&sqlite_path).expect("open relational sqlite");
    lock_conn
        .execute_batch("BEGIN IMMEDIATE;")
        .expect("hold relational writer lock");

    let (tx, rx) = std::sync::mpsc::channel();
    let repo_root = repo.path().to_path_buf();
    let worker = std::thread::spawn(move || {
        let result = crate::host::interactions::query::list_commit_author_buckets(
            &repo_root,
            &crate::host::interactions::query::InteractionBrowseFilter::default(),
        );
        tx.send(result)
            .expect("send interaction commit author buckets");
    });

    let received = rx.recv_timeout(std::time::Duration::from_millis(250));

    lock_conn
        .execute_batch("ROLLBACK;")
        .expect("release relational writer lock");

    let buckets = match received {
        Ok(result) => result.expect("load interaction commit author buckets"),
        Err(err) => {
            let delayed = rx
                .recv_timeout(std::time::Duration::from_secs(1))
                .expect("query should finish after releasing relational writer lock");
            let delayed_error = delayed.err().map(|err| err.to_string());
            panic!(
                "interaction commit author query blocked on a transient relational write lock: {err}; eventual error after releasing lock: {delayed_error:?}"
            );
        }
    };

    worker.join().expect("join interaction query worker");
    assert_eq!(buckets.len(), 1);
    assert_eq!(buckets[0].author_email, "alice@example.com");
}

#[tokio::test]
async fn dashboard_kpis_includes_expected_aggregates() {
    let repo = seed_dashboard_repo();
    let app = dashboard_app(
        repo.path(),
        ServeMode::HelloWorld,
        repo.path().to_path_buf(),
    );

    let (status, payload) = request_dashboard_graphql(app, dashboard_kpis_query()).await;
    assert_eq!(status, StatusCode::OK);
    let kpis = &payload["data"]["kpis"];
    assert_eq!(kpis["totalCommits"].as_u64(), Some(1));
    assert_eq!(kpis["totalCheckpoints"].as_u64(), Some(1));
    assert_eq!(kpis["totalAgents"].as_u64(), Some(1));
    assert_eq!(kpis["totalSessions"].as_u64(), Some(1));
    assert_eq!(kpis["filesTouchedCount"].as_u64(), Some(1));
    assert_eq!(kpis["inputTokens"].as_u64(), Some(100));
    assert_eq!(kpis["outputTokens"].as_u64(), Some(40));
    assert_eq!(kpis["cacheCreationTokens"].as_u64(), Some(10));
    assert_eq!(kpis["cacheReadTokens"].as_u64(), Some(5));
    assert_eq!(kpis["apiCallCount"].as_u64(), Some(3));
    assert_eq!(kpis["averageTokensPerCheckpoint"].as_f64(), Some(155.0));
    assert_eq!(kpis["averageSessionsPerCheckpoint"].as_f64(), Some(1.0));
}

#[tokio::test]
async fn dashboard_commits_filters_by_user_agent_and_time() {
    let repo = seed_dashboard_repo();
    let app = dashboard_app(
        repo.path(),
        ServeMode::HelloWorld,
        repo.path().to_path_buf(),
    );

    let (status, commits_payload) = request_dashboard_graphql(
        app.clone(),
        r#"
        {
          commits(branch: "main") {
            commit {
              sha
              timestamp
              filesTouched {
                filepath
                additionsCount
                deletionsCount
                changeKind
              }
            }
            checkpoint {
              checkpointId
              agents
              firstPromptPreview
              filesTouched {
                filepath
                additionsCount
                deletionsCount
                changeKind
                copiedFromPath
                copiedFromBlobSha
              }
            }
          }
        }
        "#,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let commits = commits_payload["data"]["commits"]
        .as_array()
        .expect("commits array");
    assert_eq!(commits.len(), 1);
    assert_eq!(commits[0]["checkpoint"]["checkpointId"], "aabbccddeeff");
    assert_eq!(
        commits[0]["checkpoint"]["agents"][0].as_str(),
        Some("claude-code")
    );
    assert_eq!(
        commits[0]["checkpoint"]["firstPromptPreview"].as_str(),
        Some("Build dashboard API")
    );

    let commit_files_touched = commits[0]["commit"]["filesTouched"]
        .as_array()
        .expect("commit files_touched array");
    assert_eq!(commit_files_touched[0]["filepath"], "app.rs");
    assert_eq!(commit_files_touched[0]["additionsCount"].as_u64(), Some(1));
    assert_eq!(commit_files_touched[0]["deletionsCount"].as_u64(), Some(1));
    assert!(commit_files_touched[0]["changeKind"].is_null());

    let checkpoint_files_touched = commits[0]["checkpoint"]["filesTouched"]
        .as_array()
        .expect("checkpoint files_touched array");
    assert_eq!(checkpoint_files_touched[0]["filepath"], "app.rs");
    assert_eq!(checkpoint_files_touched[0]["changeKind"], "modify");
    assert!(checkpoint_files_touched[0]["copiedFromPath"].is_null());
    assert!(checkpoint_files_touched[0]["copiedFromBlobSha"].is_null());

    let timestamp = commits[0]["commit"]["timestamp"]
        .as_i64()
        .expect("commit timestamp");

    let (_status, user_filtered) = request_dashboard_graphql(
        app.clone(),
        r#"{ commits(branch: "main", user: "bob@example.com") { checkpoint { checkpointId } } }"#,
    )
    .await;
    assert_eq!(
        user_filtered["data"]["commits"].as_array().map(Vec::len),
        Some(0)
    );

    let (_status, agent_filtered) = request_dashboard_graphql(
        app.clone(),
        r#"{ commits(branch: "main", agent: "gemini") { checkpoint { checkpointId } } }"#,
    )
    .await;
    assert_eq!(
        agent_filtered["data"]["commits"].as_array().map(Vec::len),
        Some(0)
    );

    let (_status, time_filtered) = request_dashboard_graphql(
        app,
        &format!(
            r#"{{ commits(branch: "main", from: "{}") {{ checkpoint {{ checkpointId }} }} }}"#,
            timestamp + 1
        ),
    )
    .await;
    assert_eq!(
        time_filtered["data"]["commits"].as_array().map(Vec::len),
        Some(0)
    );
}

#[tokio::test]
async fn dashboard_commits_uses_db_mapping_when_commit_mapping_is_missing() {
    let repo = seed_dashboard_repo_without_commit_mapping();
    let checkpoint_commit_sha = git_ok(repo.path(), &["rev-parse", "HEAD"]);
    insert_commit_checkpoint_mapping(repo.path(), &checkpoint_commit_sha, "aabbccddeeff");

    let app = dashboard_app(
        repo.path(),
        ServeMode::HelloWorld,
        repo.path().to_path_buf(),
    );

    let (_status, payload) = request_dashboard_graphql(
        app,
        r#"{ commits(branch: "main") { commit { sha } checkpoint { checkpointId } } }"#,
    )
    .await;

    let commits = payload["data"]["commits"]
        .as_array()
        .expect("commits array");
    assert_eq!(commits.len(), 1);
    assert_eq!(commits[0]["checkpoint"]["checkpointId"], "aabbccddeeff");
    assert_eq!(commits[0]["commit"]["sha"], checkpoint_commit_sha);
}

#[tokio::test]
async fn dashboard_commits_include_all_checkpoint_agents_and_first_prompt_preview() {
    let repo = seed_dashboard_repo_multi_session();
    let app = dashboard_app(
        repo.path(),
        ServeMode::HelloWorld,
        repo.path().to_path_buf(),
    );

    let (_status, commits_payload) = request_dashboard_graphql(
        app.clone(),
        r#"{ commits(branch: "main") { checkpoint { checkpointId agents firstPromptPreview } } }"#,
    )
    .await;
    let commits = commits_payload["data"]["commits"]
        .as_array()
        .expect("commits array");
    assert_eq!(commits.len(), 1);
    assert_eq!(commits[0]["checkpoint"]["checkpointId"], "112233445566");
    assert_eq!(
        commits[0]["checkpoint"]["agents"]
            .as_array()
            .cloned()
            .unwrap_or_default(),
        vec![json!("claude-code"), json!("gemini")]
    );
    let expected_preview = "A".repeat(160);
    assert_eq!(
        commits[0]["checkpoint"]["firstPromptPreview"].as_str(),
        Some(expected_preview.as_str())
    );

    let (_status, claude_filtered) = request_dashboard_graphql(
        app.clone(),
        r#"{ commits(branch: "main", agent: "claude-code") { checkpoint { checkpointId } } }"#,
    )
    .await;
    assert_eq!(
        claude_filtered["data"]["commits"].as_array().map(Vec::len),
        Some(1)
    );

    let (_status, gemini_filtered) = request_dashboard_graphql(
        app.clone(),
        r#"{ commits(branch: "main", agent: "gemini") { checkpoint { checkpointId } } }"#,
    )
    .await;
    assert_eq!(
        gemini_filtered["data"]["commits"].as_array().map(Vec::len),
        Some(1)
    );

    let (_status, agents_payload) =
        request_dashboard_graphql(app, r#"{ agents(branch: "main") { key } }"#).await;
    assert_eq!(
        agents_payload["data"]["agents"]
            .as_array()
            .cloned()
            .unwrap_or_default(),
        vec![json!({"key": "claude-code"}), json!({"key": "gemini"})]
    );
}

#[tokio::test]
async fn dashboard_repositories_return_empty_list_when_catalog_is_empty() {
    let repo = seed_dashboard_repo();
    let sqlite_path = checkpoint_sqlite_path(repo.path());
    let conn = rusqlite::Connection::open(&sqlite_path).expect("open relational sqlite store");
    conn.execute("DELETE FROM repositories", [])
        .expect("clear repository catalog");

    let app = dashboard_app(
        repo.path(),
        ServeMode::HelloWorld,
        repo.path().to_path_buf(),
    );

    let (_status, payload) = request_dashboard_graphql(
        app,
        r#"{ repositories { repoId name provider organization } }"#,
    )
    .await;
    assert!(
        payload["data"]["repositories"]
            .as_array()
            .expect("repositories array")
            .is_empty()
    );
}

#[tokio::test]
async fn dashboard_repositories_list_all_known_repositories() {
    let repo = seed_dashboard_repo();
    let sqlite_path = checkpoint_sqlite_path(repo.path());
    let conn = rusqlite::Connection::open(&sqlite_path).expect("open relational sqlite store");
    conn.execute(
        "INSERT INTO repositories (repo_id, provider, organization, name, default_branch)
         VALUES
            (?1, 'github', 'acme', 'alpha', 'main'),
            (?2, 'github', 'acme', 'beta', 'develop')
         ON CONFLICT(repo_id) DO UPDATE SET
            provider = excluded.provider,
            organization = excluded.organization,
            name = excluded.name,
            default_branch = excluded.default_branch",
        rusqlite::params![
            "11111111-1111-1111-1111-111111111111",
            "22222222-2222-2222-2222-222222222222",
        ],
    )
    .expect("seed repository catalogue rows");

    let app = dashboard_app(
        repo.path(),
        ServeMode::HelloWorld,
        repo.path().to_path_buf(),
    );

    let (_status, payload) =
        request_dashboard_graphql(app, r#"{ repositories { name defaultBranch } }"#).await;
    let repositories = payload["data"]["repositories"]
        .as_array()
        .expect("repositories array");
    assert_eq!(repositories.len(), 3);
    assert_eq!(
        repositories
            .iter()
            .map(|repo| repo["name"].as_str().expect("repository name"))
            .collect::<Vec<_>>(),
        vec!["alpha", "beta", SEEDED_REPO_NAME]
    );
}

#[tokio::test]
async fn dashboard_checkpoint_returns_detailed_session_payload() {
    let repo = seed_dashboard_repo();
    let app = dashboard_app(
        repo.path(),
        ServeMode::HelloWorld,
        repo.path().to_path_buf(),
    );

    let (_status, payload) = request_dashboard_graphql(
        app,
        r#"
        {
          checkpoint(checkpointId: "aabbccddeeff") {
            checkpointId
            sessionCount
            tokenUsage { inputTokens }
            filesTouched {
              filepath
              additionsCount
              deletionsCount
              changeKind
              copiedFromPath
              copiedFromBlobSha
            }
            sessions {
              sessionIndex
              sessionId
              agent
              transcriptJsonl
              promptsText
              contextText
            }
          }
        }
        "#,
    )
    .await;

    let checkpoint = &payload["data"]["checkpoint"];
    assert_eq!(checkpoint["checkpointId"], "aabbccddeeff");
    assert_eq!(checkpoint["sessionCount"].as_u64(), Some(1));
    assert_eq!(checkpoint["tokenUsage"]["inputTokens"].as_u64(), Some(100));
    let files_touched = checkpoint["filesTouched"]
        .as_array()
        .expect("files_touched array");
    assert_eq!(files_touched[0]["filepath"], "app.rs");
    assert_eq!(files_touched[0]["additionsCount"].as_u64(), Some(1));
    assert_eq!(files_touched[0]["deletionsCount"].as_u64(), Some(1));
    assert_eq!(files_touched[0]["changeKind"], "modify");
    assert!(files_touched[0]["copiedFromPath"].is_null());
    assert!(files_touched[0]["copiedFromBlobSha"].is_null());

    let sessions = checkpoint["sessions"].as_array().expect("sessions array");
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0]["sessionIndex"].as_u64(), Some(0));
    assert_eq!(sessions[0]["sessionId"], "session-1");
    assert_eq!(sessions[0]["agent"], "claude-code");
    assert!(
        sessions[0]["transcriptJsonl"]
            .as_str()
            .unwrap_or_default()
            .contains("\"tool_use\"")
    );
    assert_eq!(sessions[0]["promptsText"], "Build dashboard API");
    assert_eq!(sessions[0]["contextText"], "Repository context");
}

#[tokio::test]
async fn dashboard_users_and_agents_queries_return_expected_values() {
    let repo = seed_dashboard_repo();
    let app = dashboard_app(
        repo.path(),
        ServeMode::HelloWorld,
        repo.path().to_path_buf(),
    );

    let (_status, users_payload) = request_dashboard_graphql(
        app.clone(),
        r#"{ users(branch: "main") { key name email } }"#,
    )
    .await;
    let users = users_payload["data"]["users"]
        .as_array()
        .expect("users array");
    assert_eq!(users.len(), 1);
    assert_eq!(users[0]["key"], "alice@example.com");
    assert_eq!(users[0]["name"], "Alice");
    assert_eq!(users[0]["email"], "alice@example.com");

    let (_status, agents_payload) =
        request_dashboard_graphql(app, r#"{ agents(branch: "main") { key } }"#).await;
    let agents = agents_payload["data"]["agents"]
        .as_array()
        .expect("agents array");
    assert_eq!(agents.len(), 1);
    assert_eq!(agents[0]["key"], "claude-code");
}

#[tokio::test]
async fn dashboard_health_matches_global_health_and_hides_rest_only_fields() {
    let repo = seed_dashboard_repo();
    let app = dashboard_app(
        repo.path(),
        ServeMode::HelloWorld,
        repo.path().to_path_buf(),
    );

    let (dashboard_status, dashboard_payload) =
        request_dashboard_graphql(app.clone(), dashboard_health_query()).await;
    assert_eq!(dashboard_status, StatusCode::OK);

    let (global_status, global_payload) = request_json_with_method_and_content_type(
        app,
        Method::POST,
        "/devql/global",
        "application/json",
        Body::from(json!({ "query": dashboard_health_query() }).to_string()),
    )
    .await;
    assert_eq!(global_status, StatusCode::OK);

    let dashboard_health = &dashboard_payload["data"]["health"];
    assert!(dashboard_health.get("postgres").is_none());
    assert!(dashboard_health.get("clickhouse").is_none());
    assert_eq!(dashboard_health, &global_payload["data"]["health"]);
}

#[tokio::test]
async fn dashboard_query_reports_bad_user_input_for_invalid_time_filter() {
    let repo = seed_dashboard_repo();
    let app = dashboard_app(
        repo.path(),
        ServeMode::HelloWorld,
        repo.path().to_path_buf(),
    );

    let (status, payload) = request_dashboard_graphql(
        app,
        r#"{ commits(branch: "main", from: "not-a-timestamp") { checkpoint { checkpointId } } }"#,
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(payload["errors"][0]["extensions"]["code"], "BAD_USER_INPUT");
    assert_eq!(
        payload["errors"][0]["message"],
        "invalid from; expected unix seconds"
    );
}

#[tokio::test]
async fn dashboard_query_reports_bad_user_input_for_out_of_range_time_filter() {
    let repo = seed_dashboard_repo();
    let app = dashboard_app(
        repo.path(),
        ServeMode::HelloWorld,
        repo.path().to_path_buf(),
    );

    let (status, payload) = request_dashboard_graphql(
        app,
        r#"{ commits(branch: "main", from: "9223372036854775807") { checkpoint { checkpointId } } }"#,
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(payload["errors"][0]["extensions"]["code"], "BAD_USER_INPUT");
    assert_eq!(
        payload["errors"][0]["message"],
        "invalid from; unix seconds out of range"
    );
}

#[tokio::test]
async fn dashboard_query_reports_not_found_for_unknown_repo_id() {
    let repo = seed_dashboard_repo();
    let app = dashboard_app(
        repo.path(),
        ServeMode::HelloWorld,
        repo.path().to_path_buf(),
    );

    let (status, payload) = request_dashboard_graphql(
        app,
        r#"{ commits(repoId: "missing-repo", branch: "main") { checkpoint { checkpointId } } }"#,
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(payload["errors"][0]["extensions"]["code"], "not_found");
    assert_eq!(
        payload["errors"][0]["message"],
        "repository not found: missing-repo"
    );
}

#[tokio::test]
async fn dashboard_query_reports_bad_user_input_for_ambiguous_repo_selector() {
    let repo = seed_dashboard_repo();
    let sqlite_path = checkpoint_sqlite_path(repo.path());
    let conn = rusqlite::Connection::open(&sqlite_path).expect("open relational sqlite store");
    conn.execute(
        "INSERT INTO repositories (repo_id, provider, organization, name, default_branch)
         VALUES (?1, 'github', 'acme', ?2, 'main')
         ON CONFLICT(repo_id) DO UPDATE SET
            provider = excluded.provider,
            organization = excluded.organization,
            name = excluded.name,
            default_branch = excluded.default_branch",
        rusqlite::params!["33333333-3333-3333-3333-333333333333", SEEDED_REPO_NAME,],
    )
    .expect("seed ambiguous repository row");

    let app = dashboard_app(
        repo.path(),
        ServeMode::HelloWorld,
        repo.path().to_path_buf(),
    );

    let (status, payload) = request_dashboard_graphql(
        app,
        &format!(
            r#"{{ branches(repoId: "{repo_name}") {{ branch checkpointCommits }} }}"#,
            repo_name = SEEDED_REPO_NAME,
        ),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(payload["errors"][0]["extensions"]["code"], "BAD_USER_INPUT");
    assert!(
        payload["errors"][0]["message"]
            .as_str()
            .expect("error message")
            .contains("ambiguous")
    );
}

#[tokio::test]
async fn dashboard_query_logs_repo_checkout_unknown_failure() {
    let repo = seed_dashboard_repo();
    let unrelated_root = TempDir::new().expect("temp dir");
    let mut state = test_state(
        repo.path().to_path_buf(),
        ServeMode::HelloWorld,
        repo.path().to_path_buf(),
    );
    state.repo_root = unrelated_root.path().to_path_buf();
    let app = build_dashboard_router(state);
    let repo_id = crate::host::devql::resolve_repo_id(repo.path()).expect("resolve repo id");

    let (response, logs) = capture_logs_async(request_dashboard_graphql(
        app,
        &format!(r#"{{ branches(repoId: "{repo_id}") {{ branch checkpointCommits }} }}"#),
    ))
    .await;
    let (status, payload) = response;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(payload["errors"][0]["extensions"]["code"], "internal");
    assert!(
        payload["errors"][0]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("dashboard GraphQL wrapper failed")
    );
    assert!(
        logs.iter().any(|entry| entry.level == log::Level::Error
            && entry.message.contains("repo checkout unknown")),
        "expected dashboard repo checkout failure to be logged, got logs: {logs:?}"
    );
}

#[tokio::test]
async fn dashboard_query_resolves_repo_checkout_from_repo_sync_state() {
    let repo = seed_dashboard_repo();
    let unrelated_root = TempDir::new().expect("temp dir");
    let repo_id = crate::host::devql::resolve_repo_id(repo.path()).expect("resolve repo id");
    let sqlite_path = checkpoint_sqlite_path(repo.path());
    let conn = rusqlite::Connection::open(&sqlite_path).expect("open relational sqlite store");
    let repo_root = repo.path().to_string_lossy().to_string();
    conn.execute(
        "INSERT OR REPLACE INTO repo_sync_state (
            repo_id, repo_root, parser_version, extractor_version, last_sync_status
         ) VALUES (?1, ?2, 'parser-test', 'extractor-test', 'completed')",
        rusqlite::params![repo_id.as_str(), repo_root],
    )
    .expect("seed repo sync state row");

    let mut state = test_state(
        repo.path().to_path_buf(),
        ServeMode::HelloWorld,
        repo.path().to_path_buf(),
    );
    state.repo_root = unrelated_root.path().to_path_buf();
    let app = build_dashboard_router(state);

    let (status, payload) = request_dashboard_graphql(
        app,
        &format!(r#"{{ branches(repoId: "{repo_id}") {{ branch checkpointCommits }} }}"#),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert!(
        payload.get("errors").is_none(),
        "unexpected errors: {payload}"
    );
    assert_eq!(payload["data"]["branches"][0]["branch"], "main");
    assert_eq!(payload["data"]["branches"][0]["checkpointCommits"], 1);
}

#[tokio::test]
async fn dashboard_git_blob_returns_file_bytes() {
    let repo = seed_dashboard_repo();
    let repo_id = crate::host::devql::resolve_repo_id(repo.path()).expect("resolve repo id");
    let blob_sha = git_ok(repo.path(), &["rev-parse", "HEAD:app.rs"]);
    let app = dashboard_app(
        repo.path(),
        ServeMode::HelloWorld,
        repo.path().to_path_buf(),
    );

    let uri = format!("/devql/dashboard/blobs/{repo_id}/{blob_sha}");
    let (status, body) = request_bytes(app, &uri).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, b"fn main() { println!(\"ok\"); }\n".as_slice());
}

#[tokio::test]
async fn dashboard_git_blob_rejects_invalid_oid_length() {
    let repo = seed_dashboard_repo();
    let repo_id = crate::host::devql::resolve_repo_id(repo.path()).expect("resolve repo id");
    let app = dashboard_app(
        repo.path(),
        ServeMode::HelloWorld,
        repo.path().to_path_buf(),
    );

    let short_sha = "a".repeat(39);
    let uri = format!("/devql/dashboard/blobs/{repo_id}/{short_sha}");
    let (status, payload) = request_json(app, &uri).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(payload["error"]["code"], "bad_request");
}

#[tokio::test]
async fn dashboard_git_blob_returns_not_found_for_unknown_object() {
    let repo = seed_dashboard_repo();
    let repo_id = crate::host::devql::resolve_repo_id(repo.path()).expect("resolve repo id");
    let app = dashboard_app(
        repo.path(),
        ServeMode::HelloWorld,
        repo.path().to_path_buf(),
    );

    let uri = format!("/devql/dashboard/blobs/{repo_id}/{}", "0".repeat(40));
    let (status, payload) = request_json(app, &uri).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(payload["error"]["code"], "not_found");
}

#[tokio::test]
async fn dashboard_git_blob_rejects_non_blob_object() {
    let repo = seed_dashboard_repo();
    let repo_id = crate::host::devql::resolve_repo_id(repo.path()).expect("resolve repo id");
    let commit_sha = git_ok(repo.path(), &["rev-parse", "HEAD"]);
    let app = dashboard_app(
        repo.path(),
        ServeMode::HelloWorld,
        repo.path().to_path_buf(),
    );

    let uri = format!("/devql/dashboard/blobs/{repo_id}/{commit_sha}");
    let (status, payload) = request_json(app, &uri).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(payload["error"]["code"], "bad_request");
    assert!(
        payload["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("not a blob")
    );
}

#[tokio::test]
async fn dashboard_git_blob_returns_payload_too_large_for_oversized_blob() {
    use crate::api::handlers::git_blob::MAX_GIT_BLOB_BYTES;

    let dir = TempDir::new().expect("temp dir");
    let repo_root = dir.path();
    init_test_repo(repo_root, "main", "Alice", "alice@example.com");
    let big_len = (MAX_GIT_BLOB_BYTES as usize).saturating_add(1);
    let big = vec![b'x'; big_len];
    fs::write(repo_root.join("huge.bin"), big).expect("write huge.bin");
    git_ok(repo_root, &["add", "huge.bin"]);
    git_ok(repo_root, &["commit", "-m", "huge"]);
    seed_repository_catalog_row(repo_root, SEEDED_REPO_NAME, "main");
    let repo_id = crate::host::devql::resolve_repo_id(repo_root).expect("resolve repo id");
    let blob_sha = git_ok(repo_root, &["rev-parse", "HEAD:huge.bin"]);
    let app = dashboard_app(repo_root, ServeMode::HelloWorld, repo_root.to_path_buf());

    let uri = format!("/devql/dashboard/blobs/{repo_id}/{blob_sha}");
    let (status, payload) = request_json(app, &uri).await;
    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(payload["error"]["code"], "payload_too_large");
}

#[tokio::test]
async fn removed_api_routes_are_not_routed() {
    let repo = seed_dashboard_repo();
    let app = dashboard_app(
        repo.path(),
        ServeMode::HelloWorld,
        repo.path().to_path_buf(),
    );

    for path in ["/api", "/api/", "/api/kpis", "/api/openapi.json"] {
        let (status, payload) = request_json(app.clone(), path).await;
        assert_eq!(
            status,
            StatusCode::NOT_FOUND,
            "unexpected status for {path}"
        );
        assert_eq!(payload["error"]["code"], "not_found");
    }

    let (status, payload) =
        request_json_with_method(app, Method::POST, "/api/fetch_bundle", Body::from("{}")).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(payload["error"]["code"], "not_found");
}

#[tokio::test]
async fn fallback_page_includes_install_bootstrap_script() {
    let repo = seed_dashboard_repo();
    let app = dashboard_app(
        repo.path(),
        ServeMode::HelloWorld,
        repo.path().join("missing-bundle"),
    );

    let (status, body) = request_text(app, "/").await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("/devql/dashboard"));
    assert!(body.contains("checkBundleVersion"));
    assert!(body.contains("fetchBundle"));
    assert!(body.contains("Install dashboard bundle"));
}

#[tokio::test]
async fn installed_bundle_page_injects_update_prompt_script() {
    let repo = seed_dashboard_repo();
    let bundle = TempDir::new().expect("bundle dir");
    fs::write(
        bundle.path().join("index.html"),
        "<!doctype html><html><body>installed bundle v0.0.0</body></html>",
    )
    .expect("write index");

    let app = dashboard_app(
        repo.path(),
        ServeMode::Bundle(bundle.path().to_path_buf()),
        bundle.path().to_path_buf(),
    );

    let (status, body) = request_text(app, "/").await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("installed bundle v0.0.0"));
    assert!(body.contains("bitloops-bundle-update-prompt-script"));
    assert!(body.contains("/devql/dashboard"));
    assert!(body.contains("checkBundleVersion"));
    assert!(body.contains("Update dashboard bundle"));
}

#[tokio::test]
async fn installed_bundle_non_html_assets_are_not_modified() {
    let repo = seed_dashboard_repo();
    let bundle = TempDir::new().expect("bundle dir");
    fs::write(
        bundle.path().join("index.html"),
        "<!doctype html><html><body>installed bundle</body></html>",
    )
    .expect("write index");
    fs::write(bundle.path().join("app.js"), "console.log('bundle-app');").expect("write app js");

    let app = dashboard_app(
        repo.path(),
        ServeMode::Bundle(bundle.path().to_path_buf()),
        bundle.path().to_path_buf(),
    );

    let (status, body) = request_text(app, "/app.js").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, "console.log('bundle-app');");
    assert!(!body.contains("bitloops-bundle-update-prompt-script"));
}

#[tokio::test]
async fn missing_bundle_asset_returns_not_found_instead_of_html() {
    let repo = seed_dashboard_repo();
    let bundle = TempDir::new().expect("bundle dir");
    fs::write(
        bundle.path().join("index.html"),
        "<!doctype html><html><body>installed bundle</body></html>",
    )
    .expect("write index");

    let app = dashboard_app(
        repo.path(),
        ServeMode::Bundle(bundle.path().to_path_buf()),
        bundle.path().to_path_buf(),
    );

    let (status, body) = request_text(app, "/assets/missing-chunk.js").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body, "Bundle asset not found.\n");
}

#[tokio::test]
async fn dashboard_check_bundle_version_returns_expected_fields() {
    let repo = seed_dashboard_repo();
    let bundle_dir = TempDir::new().expect("bundle dir");
    let archive = build_bundle_archive("1.2.3");
    let checksum = checksum_hex(&archive);
    let cdn = setup_local_bundle_cdn(&archive, &checksum, "1.2.3");
    let base_url = format!("file://{}/", cdn.path().display());
    let app = build_dashboard_router(
        test_state(
            repo.path().to_path_buf(),
            ServeMode::HelloWorld,
            bundle_dir.path().to_path_buf(),
        )
        .with_bundle_source_overrides(dashboard_bundle_source_overrides_for_cdn(&base_url)),
    );

    let (status, payload) = request_dashboard_graphql(
        app,
        r#"{ checkBundleVersion { currentVersion latestApplicableVersion installAvailable reason } }"#,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let version = &payload["data"]["checkBundleVersion"];
    assert_eq!(version["latestApplicableVersion"], "1.2.3");
    assert_eq!(version["installAvailable"], true);
    assert_eq!(version["reason"], "not_installed");
}

#[tokio::test]
async fn dashboard_fetch_bundle_installs_bundle_and_root_serves_it() {
    let repo = seed_dashboard_repo();
    let bundle_parent = TempDir::new().expect("bundle parent");
    let bundle_dir = bundle_parent.path().join("bundle");
    let archive = build_bundle_archive("2.0.0");
    let checksum = checksum_hex(&archive);
    let cdn = setup_local_bundle_cdn(&archive, &checksum, "2.0.0");
    let base_url = format!("file://{}/", cdn.path().display());
    let app = build_dashboard_router(
        test_state(
            repo.path().to_path_buf(),
            ServeMode::HelloWorld,
            bundle_dir.clone(),
        )
        .with_bundle_source_overrides(dashboard_bundle_source_overrides_for_cdn(&base_url)),
    );

    let (status, before_body) = request_text(app.clone(), "/").await;
    assert_eq!(status, StatusCode::OK);
    assert!(before_body.contains("Install dashboard bundle"));

    let (status, payload) = request_dashboard_graphql(
        app.clone(),
        r#"{ checkBundleVersion { installAvailable } }"#,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        payload["data"]["checkBundleVersion"]["installAvailable"],
        true
    );

    let (status, payload) = request_dashboard_graphql(
        app.clone(),
        r#"mutation { fetchBundle { status installedVersion checksumVerified } }"#,
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(payload["data"]["fetchBundle"]["status"], "installed");
    assert_eq!(payload["data"]["fetchBundle"]["installedVersion"], "2.0.0");
    assert_eq!(payload["data"]["fetchBundle"]["checksumVerified"], true);

    let (status, after_body) = request_text(app, "/").await;
    assert_eq!(status, StatusCode::OK);
    assert!(after_body.contains("installed bundle"));
    assert!(bundle_dir.join("index.html").is_file());
    assert!(bundle_dir.join("version.json").is_file());
}

#[tokio::test]
async fn dashboard_check_bundle_version_returns_manifest_fetch_failed() {
    let repo = seed_dashboard_repo();
    let bundle_dir = TempDir::new().expect("bundle dir");
    let app = build_dashboard_router(
        test_state(
            repo.path().to_path_buf(),
            ServeMode::HelloWorld,
            bundle_dir.path().to_path_buf(),
        )
        .with_bundle_source_overrides(dashboard_bundle_source_overrides_for_manifest(
            "http://127.0.0.1:9/bundle_versions.json",
        )),
    );

    let (status, payload) =
        request_dashboard_graphql(app, r#"{ checkBundleVersion { reason } }"#).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        payload["errors"][0]["extensions"]["code"],
        "manifest_fetch_failed"
    );
}

#[tokio::test]
async fn dashboard_check_bundle_version_returns_internal_on_manifest_parse_failure() {
    let repo = seed_dashboard_repo();
    let bundle_dir = TempDir::new().expect("bundle dir");
    let cdn = TempDir::new().expect("cdn temp");
    fs::write(cdn.path().join("bundle_versions.json"), "{not-valid-json")
        .expect("write invalid manifest");
    let base_url = format!("file://{}/", cdn.path().display());
    let app = build_dashboard_router(
        test_state(
            repo.path().to_path_buf(),
            ServeMode::HelloWorld,
            bundle_dir.path().to_path_buf(),
        )
        .with_bundle_source_overrides(dashboard_bundle_source_overrides_for_cdn(&base_url)),
    );

    let (status, payload) =
        request_dashboard_graphql(app, r#"{ checkBundleVersion { reason } }"#).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(payload["errors"][0]["extensions"]["code"], "internal");
}

#[tokio::test]
async fn dashboard_check_bundle_version_returns_up_to_date() {
    let repo = seed_dashboard_repo();
    let bundle_parent = TempDir::new().expect("bundle parent");
    let bundle_dir = bundle_parent.path().join("bundle");
    fs::create_dir_all(&bundle_dir).expect("create bundle dir");
    fs::write(
        bundle_dir.join("version.json"),
        r#"{"version":"1.2.3","source_url":"file:///tmp/bundle.tar.zst"}"#,
    )
    .expect("write version");

    let archive = build_bundle_archive("1.2.3");
    let checksum = checksum_hex(&archive);
    let cdn = setup_local_bundle_cdn(&archive, &checksum, "1.2.3");
    let base_url = format!("file://{}/", cdn.path().display());
    let app = build_dashboard_router(
        test_state(repo.path().to_path_buf(), ServeMode::HelloWorld, bundle_dir)
            .with_bundle_source_overrides(dashboard_bundle_source_overrides_for_cdn(&base_url)),
    );

    let (_status, payload) =
        request_dashboard_graphql(app, r#"{ checkBundleVersion { installAvailable reason } }"#)
            .await;
    assert_eq!(
        payload["data"]["checkBundleVersion"]["installAvailable"],
        false
    );
    assert_eq!(
        payload["data"]["checkBundleVersion"]["reason"],
        "up_to_date"
    );
}

#[tokio::test]
async fn dashboard_check_bundle_version_returns_update_available() {
    let repo = seed_dashboard_repo();
    let bundle_parent = TempDir::new().expect("bundle parent");
    let bundle_dir = bundle_parent.path().join("bundle");
    fs::create_dir_all(&bundle_dir).expect("create bundle dir");
    fs::write(
        bundle_dir.join("version.json"),
        r#"{"version":"1.0.0","source_url":"file:///tmp/bundle.tar.zst"}"#,
    )
    .expect("write version");

    let archive = build_bundle_archive("1.2.3");
    let checksum = checksum_hex(&archive);
    let cdn = setup_local_bundle_cdn(&archive, &checksum, "1.2.3");
    let base_url = format!("file://{}/", cdn.path().display());
    let app = build_dashboard_router(
        test_state(repo.path().to_path_buf(), ServeMode::HelloWorld, bundle_dir)
            .with_bundle_source_overrides(dashboard_bundle_source_overrides_for_cdn(&base_url)),
    );

    let (_status, payload) = request_dashboard_graphql(
        app,
        r#"{ checkBundleVersion { currentVersion latestApplicableVersion installAvailable reason } }"#,
    )
    .await;
    let version = &payload["data"]["checkBundleVersion"];
    assert_eq!(version["installAvailable"], true);
    assert_eq!(version["reason"], "update_available");
    assert_eq!(version["currentVersion"], "1.0.0");
    assert_eq!(version["latestApplicableVersion"], "1.2.3");
}

#[tokio::test]
async fn dashboard_check_bundle_version_fetches_manifest_on_every_call() {
    let repo = seed_dashboard_repo();
    let bundle_dir = TempDir::new().expect("bundle dir");
    let archive = build_bundle_archive("1.0.0");
    let checksum = checksum_hex(&archive);
    let manifest_v1 = r#"{"versions":[{"version":"1.0.0","min_required_cli_version":"0.0.1","max_required_cli_version":"latest","download_url":"bundle.tar.zst","checksum_url":"bundle.tar.zst.sha256"}]}"#;
    let cdn = setup_local_bundle_cdn_with_manifest(manifest_v1, Some(&archive), Some(&checksum));
    let base_url = format!("file://{}/", cdn.path().display());
    let app = build_dashboard_router(
        test_state(
            repo.path().to_path_buf(),
            ServeMode::HelloWorld,
            bundle_dir.path().to_path_buf(),
        )
        .with_bundle_source_overrides(dashboard_bundle_source_overrides_for_cdn(&base_url)),
    );

    let (_status, first) = request_dashboard_graphql(
        app.clone(),
        r#"{ checkBundleVersion { latestApplicableVersion } }"#,
    )
    .await;
    assert_eq!(
        first["data"]["checkBundleVersion"]["latestApplicableVersion"],
        "1.0.0"
    );

    let manifest_v2 = r#"{"versions":[{"version":"1.1.0","min_required_cli_version":"0.0.1","max_required_cli_version":"latest","download_url":"bundle.tar.zst","checksum_url":"bundle.tar.zst.sha256"}]}"#;
    fs::write(cdn.path().join("bundle_versions.json"), manifest_v2).expect("overwrite manifest");

    let (_status, second) =
        request_dashboard_graphql(app, r#"{ checkBundleVersion { latestApplicableVersion } }"#)
            .await;
    assert_eq!(
        second["data"]["checkBundleVersion"]["latestApplicableVersion"],
        "1.1.0"
    );
}

#[tokio::test]
async fn dashboard_check_bundle_version_returns_no_compatible_version_reason() {
    let repo = seed_dashboard_repo();
    let bundle_dir = TempDir::new().expect("bundle dir");
    let manifest = r#"{"versions":[{"version":"9.9.9","min_required_cli_version":"99.0.0","max_required_cli_version":"latest","download_url":"bundle.tar.zst","checksum_url":"bundle.tar.zst.sha256"}]}"#;
    let cdn = setup_local_bundle_cdn_with_manifest(manifest, None, None);
    let base_url = format!("file://{}/", cdn.path().display());
    let app = build_dashboard_router(
        test_state(
            repo.path().to_path_buf(),
            ServeMode::HelloWorld,
            bundle_dir.path().to_path_buf(),
        )
        .with_bundle_source_overrides(dashboard_bundle_source_overrides_for_cdn(&base_url)),
    );

    let (_status, payload) = request_dashboard_graphql(
        app,
        r#"{ checkBundleVersion { installAvailable latestApplicableVersion reason } }"#,
    )
    .await;
    let version = &payload["data"]["checkBundleVersion"];
    assert_eq!(version["installAvailable"], false);
    assert_eq!(version["reason"], "no_compatible_version");
    assert!(version["latestApplicableVersion"].is_null());
}

#[tokio::test]
async fn dashboard_fetch_bundle_returns_checksum_mismatch() {
    let repo = seed_dashboard_repo();
    let bundle_parent = TempDir::new().expect("bundle parent");
    let bundle_dir = bundle_parent.path().join("bundle");
    let archive = build_bundle_archive("2.1.0");
    let wrong_checksum =
        "0000000000000000000000000000000000000000000000000000000000000000".to_string();
    let cdn = setup_local_bundle_cdn(&archive, &wrong_checksum, "2.1.0");
    let base_url = format!("file://{}/", cdn.path().display());
    let app = build_dashboard_router(
        test_state(repo.path().to_path_buf(), ServeMode::HelloWorld, bundle_dir)
            .with_bundle_source_overrides(dashboard_bundle_source_overrides_for_cdn(&base_url)),
    );

    let (status, payload) =
        request_dashboard_graphql(app, r#"mutation { fetchBundle { installedVersion } }"#).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        payload["errors"][0]["extensions"]["code"],
        "checksum_mismatch"
    );
}

#[tokio::test]
async fn dashboard_fetch_bundle_returns_no_compatible_version() {
    let repo = seed_dashboard_repo();
    let bundle_parent = TempDir::new().expect("bundle parent");
    let bundle_dir = bundle_parent.path().join("bundle");
    let archive = build_bundle_archive("9.9.9");
    let checksum = checksum_hex(&archive);
    let manifest = r#"{"versions":[{"version":"9.9.9","min_required_cli_version":"99.0.0","max_required_cli_version":"latest","download_url":"bundle.tar.zst","checksum_url":"bundle.tar.zst.sha256"}]}"#;
    let cdn = setup_local_bundle_cdn_with_manifest(manifest, Some(&archive), Some(&checksum));
    let base_url = format!("file://{}/", cdn.path().display());
    let app = build_dashboard_router(
        test_state(repo.path().to_path_buf(), ServeMode::HelloWorld, bundle_dir)
            .with_bundle_source_overrides(dashboard_bundle_source_overrides_for_cdn(&base_url)),
    );

    let (_status, payload) =
        request_dashboard_graphql(app, r#"mutation { fetchBundle { installedVersion } }"#).await;
    assert_eq!(
        payload["errors"][0]["extensions"]["code"],
        "no_compatible_version"
    );
}

#[tokio::test]
async fn dashboard_fetch_bundle_returns_bundle_download_failed() {
    let repo = seed_dashboard_repo();
    let bundle_parent = TempDir::new().expect("bundle parent");
    let bundle_dir = bundle_parent.path().join("bundle");
    let manifest = r#"{"versions":[{"version":"3.0.0","min_required_cli_version":"0.0.1","max_required_cli_version":"latest","download_url":"missing.tar.zst","checksum_url":"missing.tar.zst.sha256"}]}"#;
    let cdn = setup_local_bundle_cdn_with_manifest(manifest, None, None);
    let base_url = format!("file://{}/", cdn.path().display());
    let app = build_dashboard_router(
        test_state(repo.path().to_path_buf(), ServeMode::HelloWorld, bundle_dir)
            .with_bundle_source_overrides(dashboard_bundle_source_overrides_for_cdn(&base_url)),
    );

    let (_status, payload) =
        request_dashboard_graphql(app, r#"mutation { fetchBundle { installedVersion } }"#).await;
    assert_eq!(
        payload["errors"][0]["extensions"]["code"],
        "bundle_download_failed"
    );
}

#[tokio::test]
async fn dashboard_fetch_bundle_returns_bundle_install_failed() {
    let repo = seed_dashboard_repo();
    let bundle_parent = TempDir::new().expect("bundle parent");
    let bundle_dir = bundle_parent.path().join("bundle");

    let mut tar_builder = tar::Builder::new(Vec::new());
    let content = b"bad bundle".to_vec();
    let mut header = tar::Header::new_gnu();
    header.set_size(content.len() as u64);
    header.set_mode(0o644);
    header.set_cksum();
    tar_builder
        .append_data(&mut header, "README.txt", Cursor::new(content))
        .expect("append readme");
    let tar_bytes = tar_builder.into_inner().expect("finalise tar");
    let archive = zstd::stream::encode_all(Cursor::new(tar_bytes), 0).expect("compress archive");
    let checksum = checksum_hex(&archive);

    let manifest = r#"{"versions":[{"version":"3.1.0","min_required_cli_version":"0.0.1","max_required_cli_version":"latest","download_url":"bundle.tar.zst","checksum_url":"bundle.tar.zst.sha256"}]}"#;
    let cdn = setup_local_bundle_cdn_with_manifest(manifest, Some(&archive), Some(&checksum));
    let base_url = format!("file://{}/", cdn.path().display());
    let app = build_dashboard_router(
        test_state(repo.path().to_path_buf(), ServeMode::HelloWorld, bundle_dir)
            .with_bundle_source_overrides(dashboard_bundle_source_overrides_for_cdn(&base_url)),
    );

    let (_status, payload) =
        request_dashboard_graphql(app, r#"mutation { fetchBundle { installedVersion } }"#).await;
    assert_eq!(
        payload["errors"][0]["extensions"]["code"],
        "bundle_install_failed"
    );
}

#[tokio::test]
async fn dashboard_fetch_bundle_install_failure_does_not_replace_existing_bundle() {
    let repo = seed_dashboard_repo();
    let bundle_parent = TempDir::new().expect("bundle parent");
    let bundle_dir = bundle_parent.path().join("bundle");
    fs::create_dir_all(&bundle_dir).expect("create bundle");
    fs::write(bundle_dir.join("index.html"), "existing dashboard").expect("seed existing index");

    let mut tar_builder = tar::Builder::new(Vec::new());
    let content = b"bad bundle".to_vec();
    let mut header = tar::Header::new_gnu();
    header.set_size(content.len() as u64);
    header.set_mode(0o644);
    header.set_cksum();
    tar_builder
        .append_data(&mut header, "README.txt", Cursor::new(content))
        .expect("append readme");
    let tar_bytes = tar_builder.into_inner().expect("finalise tar");
    let archive = zstd::stream::encode_all(Cursor::new(tar_bytes), 0).expect("compress archive");
    let checksum = checksum_hex(&archive);

    let manifest = r#"{"versions":[{"version":"3.2.0","min_required_cli_version":"0.0.1","max_required_cli_version":"latest","download_url":"bundle.tar.zst","checksum_url":"bundle.tar.zst.sha256"}]}"#;
    let cdn = setup_local_bundle_cdn_with_manifest(manifest, Some(&archive), Some(&checksum));
    let base_url = format!("file://{}/", cdn.path().display());
    let app = build_dashboard_router(
        test_state(
            repo.path().to_path_buf(),
            ServeMode::HelloWorld,
            bundle_dir.clone(),
        )
        .with_bundle_source_overrides(dashboard_bundle_source_overrides_for_cdn(&base_url)),
    );

    let (_status, payload) =
        request_dashboard_graphql(app, r#"mutation { fetchBundle { installedVersion } }"#).await;
    assert_eq!(
        payload["errors"][0]["extensions"]["code"],
        "bundle_install_failed"
    );
    assert_eq!(
        fs::read_to_string(bundle_dir.join("index.html")).expect("read existing index"),
        "existing dashboard"
    );
}

#[tokio::test]
async fn dashboard_fetch_bundle_returns_internal_on_manifest_parse_failure() {
    let repo = seed_dashboard_repo();
    let bundle_parent = TempDir::new().expect("bundle parent");
    let bundle_dir = bundle_parent.path().join("bundle");
    let cdn = TempDir::new().expect("cdn temp");
    fs::write(cdn.path().join("bundle_versions.json"), "{not-valid-json")
        .expect("write invalid manifest");
    let base_url = format!("file://{}/", cdn.path().display());
    let app = build_dashboard_router(
        test_state(repo.path().to_path_buf(), ServeMode::HelloWorld, bundle_dir)
            .with_bundle_source_overrides(dashboard_bundle_source_overrides_for_cdn(&base_url)),
    );

    let (_status, payload) =
        request_dashboard_graphql(app, r#"mutation { fetchBundle { installedVersion } }"#).await;
    assert_eq!(payload["errors"][0]["extensions"]["code"], "internal");
}
