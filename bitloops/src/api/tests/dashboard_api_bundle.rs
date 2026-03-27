use super::*;

#[tokio::test]
async fn api_kpis_includes_expected_aggregates() {
    let repo = seed_dashboard_repo();
    let app = build_dashboard_router(test_state(
        repo.path().to_path_buf(),
        ServeMode::HelloWorld,
        repo.path().to_path_buf(),
    ));

    let (status, payload) = request_json(app, "/api/kpis?branch=main").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(payload["total_commits"].as_u64(), Some(1));
    assert_eq!(payload["total_checkpoints"].as_u64(), Some(1));
    assert_eq!(payload["total_agents"].as_u64(), Some(1));
    assert_eq!(payload["total_sessions"].as_u64(), Some(1));
    assert_eq!(payload["files_touched_count"].as_u64(), Some(1));
    assert_eq!(payload["input_tokens"].as_u64(), Some(100));
    assert_eq!(payload["output_tokens"].as_u64(), Some(40));
    assert_eq!(payload["cache_creation_tokens"].as_u64(), Some(10));
    assert_eq!(payload["cache_read_tokens"].as_u64(), Some(5));
    assert_eq!(payload["api_call_count"].as_u64(), Some(3));
    assert_eq!(
        payload["average_tokens_per_checkpoint"].as_f64(),
        Some(155.0)
    );
    assert_eq!(
        payload["average_sessions_per_checkpoint"].as_f64(),
        Some(1.0)
    );
}

#[tokio::test]
async fn api_commits_filters_by_user_agent_and_time() {
    let repo = seed_dashboard_repo();
    let app = build_dashboard_router(test_state(
        repo.path().to_path_buf(),
        ServeMode::HelloWorld,
        repo.path().to_path_buf(),
    ));

    let (status, commits_payload) = request_json(app.clone(), "/api/commits?branch=main").await;
    assert_eq!(status, StatusCode::OK);
    let commits = commits_payload.as_array().expect("commits array");
    assert_eq!(commits.len(), 1);
    assert_eq!(commits[0]["checkpoint"]["checkpoint_id"], "aabbccddeeff");
    assert!(commits[0]["checkpoint"].get("agent").is_none());
    assert_eq!(
        commits[0]["checkpoint"]["agents"].as_array().map(Vec::len),
        Some(1)
    );
    assert_eq!(
        commits[0]["checkpoint"]["agents"][0].as_str(),
        Some("claude-code")
    );
    assert_eq!(
        commits[0]["checkpoint"]["first_prompt_preview"].as_str(),
        Some("Build dashboard API")
    );
    let commit_files_touched = commits[0]["commit"]["files_touched"]
        .as_array()
        .expect("commit files_touched array");
    assert_eq!(commit_files_touched.len(), 1);
    assert_eq!(commit_files_touched[0]["filepath"], "app.rs");
    assert_eq!(commit_files_touched[0]["additionsCount"].as_u64(), Some(1));
    assert_eq!(commit_files_touched[0]["deletionsCount"].as_u64(), Some(1));

    let checkpoint_files_touched = commits[0]["checkpoint"]["files_touched"]
        .as_array()
        .expect("checkpoint files_touched array");
    assert_eq!(checkpoint_files_touched.len(), 1);
    assert_eq!(checkpoint_files_touched[0]["filepath"], "app.rs");
    assert_eq!(
        checkpoint_files_touched[0]["additionsCount"].as_u64(),
        Some(1)
    );
    assert_eq!(
        checkpoint_files_touched[0]["deletionsCount"].as_u64(),
        Some(1)
    );

    let timestamp = commits[0]["commit"]["timestamp"]
        .as_i64()
        .expect("commit timestamp");

    let (status, user_filtered) =
        request_json(app.clone(), "/api/commits?branch=main&user=bob@example.com").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(user_filtered.as_array().map(Vec::len), Some(0));

    let (status, agent_filtered) =
        request_json(app.clone(), "/api/commits?branch=main&agent=gemini").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(agent_filtered.as_array().map(Vec::len), Some(0));

    let (status, time_filtered) = request_json(
        app,
        &format!("/api/commits?branch=main&from={}", timestamp + 1),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(time_filtered.as_array().map(Vec::len), Some(0));
}

#[tokio::test]
async fn api_commits_uses_db_mapping_when_commit_mapping_is_missing() {
    let repo = seed_dashboard_repo_without_commit_mapping();
    let checkpoint_commit_sha = git_ok(repo.path(), &["rev-parse", "HEAD"]);
    insert_commit_checkpoint_mapping(repo.path(), &checkpoint_commit_sha, "aabbccddeeff");

    let app = build_dashboard_router(test_state(
        repo.path().to_path_buf(),
        ServeMode::HelloWorld,
        repo.path().to_path_buf(),
    ));

    let (status, commits_payload) = request_json(app, "/api/commits?branch=main").await;
    assert_eq!(status, StatusCode::OK);

    let commits = commits_payload.as_array().expect("commits array");
    assert_eq!(commits.len(), 1);
    assert_eq!(
        commits[0]["checkpoint"]["checkpoint_id"].as_str(),
        Some("aabbccddeeff")
    );
    assert_eq!(
        commits[0]["commit"]["sha"].as_str(),
        Some(checkpoint_commit_sha.as_str())
    );
}

#[tokio::test]
async fn api_commits_includes_all_checkpoint_agents_and_first_prompt_preview() {
    let repo = seed_dashboard_repo_multi_session();
    let app = build_dashboard_router(test_state(
        repo.path().to_path_buf(),
        ServeMode::HelloWorld,
        repo.path().to_path_buf(),
    ));

    let (status, commits_payload) = request_json(app.clone(), "/api/commits?branch=main").await;
    assert_eq!(status, StatusCode::OK);
    let commits = commits_payload.as_array().expect("commits array");
    assert_eq!(commits.len(), 1);

    let checkpoint = &commits[0]["checkpoint"];
    assert_eq!(checkpoint["checkpoint_id"], "112233445566");
    assert_eq!(
        checkpoint["agents"].as_array().cloned().unwrap_or_default(),
        vec![json!("claude-code"), json!("gemini")]
    );
    let expected_preview = "A".repeat(160);
    assert_eq!(
        checkpoint["first_prompt_preview"].as_str(),
        Some(expected_preview.as_str())
    );
    assert!(checkpoint.get("agent").is_none());

    let (status, claude_filtered) =
        request_json(app.clone(), "/api/commits?branch=main&agent=claude-code").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(claude_filtered.as_array().map(Vec::len), Some(1));

    let (status, gemini_filtered) =
        request_json(app.clone(), "/api/commits?branch=main&agent=gemini").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(gemini_filtered.as_array().map(Vec::len), Some(1));

    let (status, agents_payload) = request_json(app, "/api/agents?branch=main").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        agents_payload.as_array().cloned().unwrap_or_default(),
        vec![json!({"key": "claude-code"}), json!({"key": "gemini"})]
    );
}

#[tokio::test]
async fn api_validates_missing_required_branch() {
    let repo = seed_dashboard_repo();
    let app = build_dashboard_router(test_state(
        repo.path().to_path_buf(),
        ServeMode::HelloWorld,
        repo.path().to_path_buf(),
    ));

    let (status, payload) = request_json(app, "/api/kpis").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(payload["error"]["code"], "bad_request");
    assert_eq!(payload["error"]["message"], "branch is required");
}

#[tokio::test]
async fn api_checkpoint_returns_detailed_session_payload() {
    let repo = seed_dashboard_repo();
    let app = build_dashboard_router(test_state(
        repo.path().to_path_buf(),
        ServeMode::HelloWorld,
        repo.path().to_path_buf(),
    ));

    let (status, payload) = request_json(app, "/api/checkpoints/aabbccddeeff").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(payload["checkpoint_id"], "aabbccddeeff");
    assert_eq!(payload["session_count"].as_u64(), Some(1));
    assert_eq!(payload["token_usage"]["input_tokens"].as_u64(), Some(100));
    let files_touched = payload["files_touched"]
        .as_array()
        .expect("files_touched array");
    assert_eq!(files_touched.len(), 1);
    assert_eq!(files_touched[0]["filepath"], "app.rs");
    assert_eq!(files_touched[0]["additionsCount"].as_u64(), Some(1));
    assert_eq!(files_touched[0]["deletionsCount"].as_u64(), Some(1));

    let sessions = payload["sessions"].as_array().expect("sessions array");
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0]["session_index"].as_u64(), Some(0));
    assert_eq!(sessions[0]["session_id"], "session-1");
    assert_eq!(sessions[0]["agent"], "claude-code");
    assert!(
        sessions[0]["transcript_jsonl"]
            .as_str()
            .unwrap_or_default()
            .contains("\"tool_use\"")
    );
    assert_eq!(
        sessions[0]["prompts_text"].as_str().unwrap_or_default(),
        "Build dashboard API"
    );
    assert_eq!(
        sessions[0]["context_text"].as_str().unwrap_or_default(),
        "Repository context"
    );
}

#[tokio::test]
async fn api_agents_returns_kebab_case_keys() {
    let repo = seed_dashboard_repo();
    let app = build_dashboard_router(test_state(
        repo.path().to_path_buf(),
        ServeMode::HelloWorld,
        repo.path().to_path_buf(),
    ));

    let (status, payload) = request_json(app, "/api/agents?branch=main").await;
    assert_eq!(status, StatusCode::OK);

    let agents = payload.as_array().expect("agents array");
    assert_eq!(agents.len(), 1);
    assert_eq!(agents[0]["key"], "claude-code");
}

#[tokio::test]
async fn api_users_returns_name_and_email_from_graphql_wrapper() {
    let repo = seed_dashboard_repo();
    let app = build_dashboard_router(test_state(
        repo.path().to_path_buf(),
        ServeMode::HelloWorld,
        repo.path().to_path_buf(),
    ));

    let (status, payload) = request_json(app, "/api/users?branch=main").await;
    assert_eq!(status, StatusCode::OK);

    let users = payload.as_array().expect("users array");
    assert_eq!(users.len(), 1);
    assert_eq!(users[0]["key"], "alice@example.com");
    assert_eq!(users[0]["name"], "Alice");
    assert_eq!(users[0]["email"], "alice@example.com");
}

#[tokio::test]
async fn api_checkpoint_validates_checkpoint_id() {
    let repo = seed_dashboard_repo();
    let app = build_dashboard_router(test_state(
        repo.path().to_path_buf(),
        ServeMode::HelloWorld,
        repo.path().to_path_buf(),
    ));

    let (status, payload) = request_json(app, "/api/checkpoints/not-an-id").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(payload["error"]["code"], "bad_request");
    assert_eq!(
        payload["error"]["message"],
        "invalid checkpoint_id; expected 12 lowercase hex characters"
    );
}

#[tokio::test]
async fn api_openapi_json_lists_dashboard_paths() {
    let repo = seed_dashboard_repo();
    let app = build_dashboard_router(test_state(
        repo.path().to_path_buf(),
        ServeMode::HelloWorld,
        repo.path().to_path_buf(),
    ));

    let (status, payload) = request_json(app, "/api/openapi.json").await;
    assert_eq!(status, StatusCode::OK);
    assert!(payload["paths"].get("/api/kpis").is_some());
    assert!(payload["paths"].get("/api/commits").is_some());
    assert!(payload["paths"].get("/api/branches").is_some());
    assert!(payload["paths"].get("/api/users").is_some());
    assert!(payload["paths"].get("/api/agents").is_some());
    assert!(payload["paths"].get("/api/db/health").is_some());
    assert!(
        payload["paths"]
            .get("/api/checkpoints/{checkpoint_id}")
            .is_some()
    );
    assert!(payload["paths"].get("/api/check_bundle_version").is_some());
    assert!(payload["paths"].get("/api/fetch_bundle").is_some());
    assert!(
        payload["paths"]["/api/check_bundle_version"]["get"]["responses"]
            .get("200")
            .is_some()
    );
    assert!(
        payload["paths"]["/api/check_bundle_version"]["get"]["responses"]
            .get("502")
            .is_some()
    );
    assert!(
        payload["paths"]["/api/check_bundle_version"]["get"]["responses"]
            .get("500")
            .is_some()
    );
    assert!(
        payload["paths"]["/api/fetch_bundle"]["post"]["responses"]
            .get("200")
            .is_some()
    );
    assert!(
        payload["paths"]["/api/fetch_bundle"]["post"]["responses"]
            .get("409")
            .is_some()
    );
    assert!(
        payload["paths"]["/api/fetch_bundle"]["post"]["responses"]
            .get("422")
            .is_some()
    );
    assert!(
        payload["paths"]["/api/fetch_bundle"]["post"]["responses"]
            .get("502")
            .is_some()
    );
    assert!(
        payload["paths"]["/api/fetch_bundle"]["post"]["responses"]
            .get("500")
            .is_some()
    );
}

#[tokio::test]
async fn api_db_health_reports_skip_when_backends_not_configured() {
    let repo = seed_dashboard_repo();
    let app = build_dashboard_router(test_state(
        repo.path().to_path_buf(),
        ServeMode::HelloWorld,
        repo.path().to_path_buf(),
    ));

    let (status, payload) = request_json(app, "/api/db/health").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(payload["relational"]["status"], "SKIP");
    assert_eq!(payload["events"]["status"], "SKIP");
    assert_eq!(payload["postgres"]["status"], "SKIP");
    assert_eq!(payload["clickhouse"]["status"], "SKIP");
}

#[tokio::test]
async fn api_root_stays_in_json_namespace() {
    let repo = seed_dashboard_repo();
    let app = build_dashboard_router(test_state(
        repo.path().to_path_buf(),
        ServeMode::HelloWorld,
        repo.path().to_path_buf(),
    ));

    let (status, payload) = request_json(app.clone(), "/api").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(payload["name"], "bitloops-dashboard-api");
    assert_eq!(payload["openapi"], "/api/openapi.json");

    let (status, payload) = request_json(app, "/api/").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(payload["name"], "bitloops-dashboard-api");
}

#[tokio::test]
async fn fallback_page_includes_install_bootstrap_script() {
    let repo = seed_dashboard_repo();
    let app = build_dashboard_router(test_state(
        repo.path().to_path_buf(),
        ServeMode::HelloWorld,
        repo.path().join("missing-bundle"),
    ));

    let (status, body) = request_text(app, "/").await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("check_bundle_version"));
    assert!(body.contains("fetch_bundle"));
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

    let app = build_dashboard_router(test_state(
        repo.path().to_path_buf(),
        ServeMode::Bundle(bundle.path().to_path_buf()),
        bundle.path().to_path_buf(),
    ));

    let (status, body) = request_text(app, "/").await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("installed bundle v0.0.0"));
    assert!(body.contains("bitloops-bundle-update-prompt-script"));
    assert!(body.contains("/api/check_bundle_version"));
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

    let app = build_dashboard_router(test_state(
        repo.path().to_path_buf(),
        ServeMode::Bundle(bundle.path().to_path_buf()),
        bundle.path().to_path_buf(),
    ));

    let (status, body) = request_text(app, "/app.js").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, "console.log('bundle-app');");
    assert!(!body.contains("bitloops-bundle-update-prompt-script"));
}

#[tokio::test]
async fn api_check_bundle_version_returns_expected_fields() {
    let repo = seed_dashboard_repo();
    let bundle_dir = TempDir::new().expect("bundle dir");
    let archive = build_bundle_archive("1.2.3");
    let checksum = checksum_hex(&archive);
    let cdn = setup_local_bundle_cdn(&archive, &checksum, "1.2.3");
    let base_url = format!("file://{}/", cdn.path().display());
    let _state = with_dashboard_cdn_base_url(&base_url);

    let app = build_dashboard_router(test_state(
        repo.path().to_path_buf(),
        ServeMode::HelloWorld,
        bundle_dir.path().to_path_buf(),
    ));

    let (status, payload) = request_json(app, "/api/check_bundle_version").await;
    assert_eq!(status, StatusCode::OK);
    assert!(payload.get("currentVersion").is_some());
    assert!(payload.get("latestApplicableVersion").is_some());
    assert!(payload.get("installAvailable").is_some());
    assert!(payload.get("reason").is_some());
    assert_eq!(payload["latestApplicableVersion"], "1.2.3");
    assert_eq!(payload["installAvailable"], true);
    assert_eq!(payload["reason"], "not_installed");
}

#[tokio::test]
async fn api_fetch_bundle_installs_bundle_and_root_serves_it() {
    let repo = seed_dashboard_repo();
    let bundle_parent = TempDir::new().expect("bundle parent");
    let bundle_dir = bundle_parent.path().join("bundle");
    let archive = build_bundle_archive("2.0.0");
    let checksum = checksum_hex(&archive);
    let cdn = setup_local_bundle_cdn(&archive, &checksum, "2.0.0");
    let base_url = format!("file://{}/", cdn.path().display());
    let _state = with_dashboard_cdn_base_url(&base_url);

    let app = build_dashboard_router(test_state(
        repo.path().to_path_buf(),
        ServeMode::HelloWorld,
        bundle_dir.clone(),
    ));

    let (status, before_body) = request_text(app.clone(), "/").await;
    assert_eq!(status, StatusCode::OK);
    assert!(before_body.contains("Install dashboard bundle"));

    let (status, payload) = request_json_with_method(
        app.clone(),
        Method::POST,
        "/api/fetch_bundle",
        Body::from("{}"),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(payload["status"], "installed");
    assert_eq!(payload["installedVersion"], "2.0.0");
    assert_eq!(payload["checksumVerified"], true);

    let (status, after_body) = request_text(app, "/").await;
    assert_eq!(status, StatusCode::OK);
    assert!(after_body.contains("installed bundle"));
    assert!(bundle_dir.join("index.html").is_file());
    assert!(bundle_dir.join("version.json").is_file());
}

#[tokio::test]
async fn api_check_bundle_version_returns_manifest_fetch_failed() {
    let repo = seed_dashboard_repo();
    let bundle_dir = TempDir::new().expect("bundle dir");
    let _state = with_dashboard_manifest_url("http://127.0.0.1:9/bundle_versions.json");

    let app = build_dashboard_router(test_state(
        repo.path().to_path_buf(),
        ServeMode::HelloWorld,
        bundle_dir.path().to_path_buf(),
    ));

    let (status, payload) = request_json(app, "/api/check_bundle_version").await;
    assert_eq!(status, StatusCode::BAD_GATEWAY);
    assert_eq!(payload["error"]["code"], "manifest_fetch_failed");
    assert!(payload["error"].get("message").is_some());
}

#[tokio::test]
async fn api_check_bundle_version_returns_internal_on_manifest_parse_failure() {
    let repo = seed_dashboard_repo();
    let bundle_dir = TempDir::new().expect("bundle dir");
    let cdn = TempDir::new().expect("cdn temp");
    fs::write(cdn.path().join("bundle_versions.json"), "{not-valid-json")
        .expect("write invalid manifest");
    let base_url = format!("file://{}/", cdn.path().display());
    let _state = with_dashboard_cdn_base_url(&base_url);

    let app = build_dashboard_router(test_state(
        repo.path().to_path_buf(),
        ServeMode::HelloWorld,
        bundle_dir.path().to_path_buf(),
    ));

    let (status, payload) = request_json(app, "/api/check_bundle_version").await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(payload["error"]["code"], "internal");
}

#[tokio::test]
async fn api_check_bundle_version_returns_up_to_date() {
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
    let _state = with_dashboard_cdn_base_url(&base_url);

    let app = build_dashboard_router(test_state(
        repo.path().to_path_buf(),
        ServeMode::HelloWorld,
        bundle_dir,
    ));

    let (status, payload) = request_json(app, "/api/check_bundle_version").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(payload["installAvailable"], false);
    assert_eq!(payload["reason"], "up_to_date");
}

#[tokio::test]
async fn api_check_bundle_version_returns_update_available() {
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
    let _state = with_dashboard_cdn_base_url(&base_url);

    let app = build_dashboard_router(test_state(
        repo.path().to_path_buf(),
        ServeMode::HelloWorld,
        bundle_dir,
    ));

    let (status, payload) = request_json(app, "/api/check_bundle_version").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(payload["installAvailable"], true);
    assert_eq!(payload["reason"], "update_available");
    assert_eq!(payload["currentVersion"], "1.0.0");
    assert_eq!(payload["latestApplicableVersion"], "1.2.3");
}

#[tokio::test]
async fn api_check_bundle_version_fetches_manifest_on_every_call() {
    let repo = seed_dashboard_repo();
    let bundle_dir = TempDir::new().expect("bundle dir");
    let archive = build_bundle_archive("1.0.0");
    let checksum = checksum_hex(&archive);
    let manifest_v1 = r#"{"versions":[{"version":"1.0.0","min_required_cli_version":"0.0.1","max_required_cli_version":"latest","download_url":"bundle.tar.zst","checksum_url":"bundle.tar.zst.sha256"}]}"#;
    let cdn = setup_local_bundle_cdn_with_manifest(manifest_v1, Some(&archive), Some(&checksum));

    let base_url = format!("file://{}/", cdn.path().display());
    let _state = with_dashboard_cdn_base_url(&base_url);

    let app = build_dashboard_router(test_state(
        repo.path().to_path_buf(),
        ServeMode::HelloWorld,
        bundle_dir.path().to_path_buf(),
    ));

    let (status_first, payload_first) =
        request_json(app.clone(), "/api/check_bundle_version").await;
    assert_eq!(status_first, StatusCode::OK);
    assert_eq!(payload_first["latestApplicableVersion"], "1.0.0");

    let manifest_v2 = r#"{"versions":[{"version":"1.1.0","min_required_cli_version":"0.0.1","max_required_cli_version":"latest","download_url":"bundle.tar.zst","checksum_url":"bundle.tar.zst.sha256"}]}"#;
    fs::write(cdn.path().join("bundle_versions.json"), manifest_v2).expect("overwrite manifest");

    let (status_second, payload_second) = request_json(app, "/api/check_bundle_version").await;
    assert_eq!(status_second, StatusCode::OK);
    assert_eq!(payload_second["latestApplicableVersion"], "1.1.0");
}

#[tokio::test]
async fn api_check_bundle_version_returns_no_compatible_version_reason() {
    let repo = seed_dashboard_repo();
    let bundle_dir = TempDir::new().expect("bundle dir");
    let manifest = r#"{"versions":[{"version":"9.9.9","min_required_cli_version":"99.0.0","max_required_cli_version":"latest","download_url":"bundle.tar.zst","checksum_url":"bundle.tar.zst.sha256"}]}"#;
    let cdn = setup_local_bundle_cdn_with_manifest(manifest, None, None);
    let base_url = format!("file://{}/", cdn.path().display());
    let _state = with_dashboard_cdn_base_url(&base_url);

    let app = build_dashboard_router(test_state(
        repo.path().to_path_buf(),
        ServeMode::HelloWorld,
        bundle_dir.path().to_path_buf(),
    ));

    let (status, payload) = request_json(app, "/api/check_bundle_version").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(payload["installAvailable"], false);
    assert_eq!(payload["reason"], "no_compatible_version");
    assert!(payload["latestApplicableVersion"].is_null());
}

#[tokio::test]
async fn api_fetch_bundle_returns_checksum_mismatch() {
    let repo = seed_dashboard_repo();
    let bundle_parent = TempDir::new().expect("bundle parent");
    let bundle_dir = bundle_parent.path().join("bundle");
    let archive = build_bundle_archive("2.1.0");
    let wrong_checksum =
        "0000000000000000000000000000000000000000000000000000000000000000".to_string();
    let cdn = setup_local_bundle_cdn(&archive, &wrong_checksum, "2.1.0");
    let base_url = format!("file://{}/", cdn.path().display());
    let _state = with_dashboard_cdn_base_url(&base_url);

    let app = build_dashboard_router(test_state(
        repo.path().to_path_buf(),
        ServeMode::HelloWorld,
        bundle_dir,
    ));

    let (status, payload) =
        request_json_with_method(app, Method::POST, "/api/fetch_bundle", Body::from("{}")).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(payload["error"]["code"], "checksum_mismatch");
}

#[tokio::test]
async fn api_fetch_bundle_returns_no_compatible_version() {
    let repo = seed_dashboard_repo();
    let bundle_parent = TempDir::new().expect("bundle parent");
    let bundle_dir = bundle_parent.path().join("bundle");
    let archive = build_bundle_archive("9.9.9");
    let checksum = checksum_hex(&archive);
    let manifest = r#"{"versions":[{"version":"9.9.9","min_required_cli_version":"99.0.0","max_required_cli_version":"latest","download_url":"bundle.tar.zst","checksum_url":"bundle.tar.zst.sha256"}]}"#;
    let cdn = setup_local_bundle_cdn_with_manifest(manifest, Some(&archive), Some(&checksum));
    let base_url = format!("file://{}/", cdn.path().display());
    let _state = with_dashboard_cdn_base_url(&base_url);

    let app = build_dashboard_router(test_state(
        repo.path().to_path_buf(),
        ServeMode::HelloWorld,
        bundle_dir,
    ));

    let (status, payload) =
        request_json_with_method(app, Method::POST, "/api/fetch_bundle", Body::from("{}")).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(payload["error"]["code"], "no_compatible_version");
}

#[tokio::test]
async fn api_fetch_bundle_returns_bundle_download_failed() {
    let repo = seed_dashboard_repo();
    let bundle_parent = TempDir::new().expect("bundle parent");
    let bundle_dir = bundle_parent.path().join("bundle");
    let manifest = r#"{"versions":[{"version":"3.0.0","min_required_cli_version":"0.0.1","max_required_cli_version":"latest","download_url":"missing.tar.zst","checksum_url":"missing.tar.zst.sha256"}]}"#;
    let cdn = setup_local_bundle_cdn_with_manifest(manifest, None, None);
    let base_url = format!("file://{}/", cdn.path().display());
    let _state = with_dashboard_cdn_base_url(&base_url);

    let app = build_dashboard_router(test_state(
        repo.path().to_path_buf(),
        ServeMode::HelloWorld,
        bundle_dir,
    ));

    let (status, payload) =
        request_json_with_method(app, Method::POST, "/api/fetch_bundle", Body::from("{}")).await;
    assert_eq!(status, StatusCode::BAD_GATEWAY);
    assert_eq!(payload["error"]["code"], "bundle_download_failed");
}

#[tokio::test]
async fn api_fetch_bundle_returns_bundle_install_failed() {
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
    let tar_bytes = tar_builder.into_inner().expect("finalize tar");
    let archive = zstd::stream::encode_all(Cursor::new(tar_bytes), 0).expect("compress archive");
    let checksum = checksum_hex(&archive);

    let manifest = r#"{"versions":[{"version":"3.1.0","min_required_cli_version":"0.0.1","max_required_cli_version":"latest","download_url":"bundle.tar.zst","checksum_url":"bundle.tar.zst.sha256"}]}"#;
    let cdn = setup_local_bundle_cdn_with_manifest(manifest, Some(&archive), Some(&checksum));
    let base_url = format!("file://{}/", cdn.path().display());
    let _state = with_dashboard_cdn_base_url(&base_url);

    let app = build_dashboard_router(test_state(
        repo.path().to_path_buf(),
        ServeMode::HelloWorld,
        bundle_dir,
    ));

    let (status, payload) =
        request_json_with_method(app, Method::POST, "/api/fetch_bundle", Body::from("{}")).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(payload["error"]["code"], "bundle_install_failed");
}

#[tokio::test]
async fn api_fetch_bundle_install_failure_does_not_replace_existing_bundle() {
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
    let tar_bytes = tar_builder.into_inner().expect("finalize tar");
    let archive = zstd::stream::encode_all(Cursor::new(tar_bytes), 0).expect("compress archive");
    let checksum = checksum_hex(&archive);

    let manifest = r#"{"versions":[{"version":"3.2.0","min_required_cli_version":"0.0.1","max_required_cli_version":"latest","download_url":"bundle.tar.zst","checksum_url":"bundle.tar.zst.sha256"}]}"#;
    let cdn = setup_local_bundle_cdn_with_manifest(manifest, Some(&archive), Some(&checksum));
    let base_url = format!("file://{}/", cdn.path().display());
    let _state = with_dashboard_cdn_base_url(&base_url);

    let app = build_dashboard_router(test_state(
        repo.path().to_path_buf(),
        ServeMode::HelloWorld,
        bundle_dir.clone(),
    ));

    let (status, payload) =
        request_json_with_method(app, Method::POST, "/api/fetch_bundle", Body::from("{}")).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(payload["error"]["code"], "bundle_install_failed");
    assert_eq!(
        fs::read_to_string(bundle_dir.join("index.html")).expect("read existing index"),
        "existing dashboard"
    );
}

#[tokio::test]
async fn api_fetch_bundle_returns_internal_on_manifest_parse_failure() {
    let repo = seed_dashboard_repo();
    let bundle_parent = TempDir::new().expect("bundle parent");
    let bundle_dir = bundle_parent.path().join("bundle");
    let cdn = TempDir::new().expect("cdn temp");
    fs::write(cdn.path().join("bundle_versions.json"), "{not-valid-json")
        .expect("write invalid manifest");
    let base_url = format!("file://{}/", cdn.path().display());
    let _state = with_dashboard_cdn_base_url(&base_url);

    let app = build_dashboard_router(test_state(
        repo.path().to_path_buf(),
        ServeMode::HelloWorld,
        bundle_dir,
    ));

    let (status, payload) =
        request_json_with_method(app, Method::POST, "/api/fetch_bundle", Body::from("{}")).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(payload["error"]["code"], "internal");
}
