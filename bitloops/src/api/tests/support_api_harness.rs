use super::*;

pub(super) fn seed_dashboard_repo_without_commit_mapping() -> TempDir {
    let dir = TempDir::new().expect("temp dir");
    let repo_root = dir.path();

    init_test_repo(repo_root, "main", "Alice", "alice@example.com");

    fs::write(repo_root.join("app.rs"), "fn main() {}\n").expect("write app.rs");
    git_ok(repo_root, &["add", "app.rs"]);
    git_ok(repo_root, &["commit", "-m", "Initial commit"]);

    fs::write(
        repo_root.join("app.rs"),
        "fn main() { println!(\"ok\"); }\n",
    )
    .expect("update app.rs");
    git_ok(repo_root, &["add", "app.rs"]);
    git_ok(repo_root, &["commit", "-m", "Checkpoint commit"]);
    let checkpoint_commit_sha = git_ok(repo_root, &["rev-parse", "HEAD"]);
    seed_repository_catalog_row(repo_root, SEEDED_REPO_NAME, "main");

    git_ok(
        repo_root,
        &["checkout", "--orphan", "bitloops/checkpoints/v1"],
    );
    let checkpoint_bucket = repo_root.join("aa").join("bbccddeeff");
    fs::create_dir_all(checkpoint_bucket.join("0")).expect("create checkpoint directories");

    let top_metadata = json!({
        "checkpoint_id": "aabbccddeeff",
        "strategy": "manual-commit",
        "branch": "main",
        "checkpoints_count": 2,
        "files_touched": ["app.rs"],
        "sessions": [{
            "metadata": "/aa/bbccddeeff/0/metadata.json",
            "transcript": "/aa/bbccddeeff/0/full.jsonl",
            "context": "/aa/bbccddeeff/0/context.md",
            "content_hash": "/aa/bbccddeeff/0/content_hash.txt",
            "prompt": "/aa/bbccddeeff/0/prompt.txt"
        }],
        "token_usage": {
            "input_tokens": 100,
            "output_tokens": 40,
            "cache_creation_tokens": 10,
            "cache_read_tokens": 5,
            "api_call_count": 3
        }
    });
    let session_metadata = json!({
        "checkpoint_id": "aabbccddeeff",
        "session_id": "session-1",
        "checkpoints_count": 2,
        "strategy": "manual-commit",
        "agent": "claude-code",
        "created_at": "2026-02-27T12:00:00Z",
        "cli_version": "0.0.3",
        "files_touched": ["app.rs"],
        "is_task": false,
        "tool_use_id": ""
    });
    fs::write(
        checkpoint_bucket.join("metadata.json"),
        serde_json::to_string_pretty(&top_metadata).expect("serialize top metadata"),
    )
    .expect("write top metadata");
    fs::write(
        checkpoint_bucket.join("0").join("metadata.json"),
        serde_json::to_string_pretty(&session_metadata).expect("serialize session metadata"),
    )
    .expect("write session metadata");
    let transcript_payload = "{\"type\":\"user\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"Build dashboard API\"}]}}\n\
{\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"Implemented\"},{\"type\":\"tool_use\",\"name\":\"Edit\",\"input\":{\"file_path\":\"dashboard.rs\"}}]}}\n";
    let prompt_payload = "Build dashboard API";
    let context_payload = "Repository context";
    fs::write(
        checkpoint_bucket.join("0").join("full.jsonl"),
        transcript_payload,
    )
    .expect("write transcript");
    fs::write(
        checkpoint_bucket.join("0").join("prompt.txt"),
        prompt_payload,
    )
    .expect("write prompt");
    fs::write(
        checkpoint_bucket.join("0").join("context.md"),
        context_payload,
    )
    .expect("write context");

    git_ok(repo_root, &["add", "aa"]);
    git_ok(repo_root, &["commit", "-m", "checkpoint metadata"]);
    git_ok(repo_root, &["checkout", "main"]);

    seed_checkpoint_storage_for_dashboard(
        repo_root,
        SeedCheckpointStorage {
            commit_sha: &checkpoint_commit_sha,
            checkpoint_id: "aabbccddeeff",
            branch: "main",
            files_touched: &["app.rs"],
            checkpoints_count: 2,
            token_usage: json!({
                "input_tokens": 100,
                "output_tokens": 40,
                "cache_creation_tokens": 10,
                "cache_read_tokens": 5,
                "api_call_count": 3
            }),
            sessions: &[SeedCheckpointSession {
                session_index: 0,
                session_id: "session-1",
                agent: "claude-code",
                created_at: "2026-02-27T12:00:00Z",
                checkpoints_count: 2,
                transcript: transcript_payload,
                prompts: prompt_payload,
                context: context_payload,
            }],
            insert_mapping: false,
        },
    );

    dir
}

pub(super) fn seed_dashboard_repo_multi_session() -> TempDir {
    let dir = TempDir::new().expect("temp dir");
    let repo_root = dir.path();

    init_test_repo(repo_root, "main", "Alice", "alice@example.com");

    fs::write(repo_root.join("app.rs"), "fn main() {}\n").expect("write app.rs");
    git_ok(repo_root, &["add", "app.rs"]);
    git_ok(repo_root, &["commit", "-m", "Initial commit"]);

    fs::write(
        repo_root.join("app.rs"),
        "fn main() { println!(\"ok\"); }\n",
    )
    .expect("update app.rs");
    git_ok(repo_root, &["add", "app.rs"]);
    git_ok(repo_root, &["commit", "-m", "Checkpoint commit"]);
    let checkpoint_commit_sha = git_ok(repo_root, &["rev-parse", "HEAD"]);
    seed_repository_catalog_row(repo_root, SEEDED_REPO_NAME, "main");

    git_ok(
        repo_root,
        &["checkout", "--orphan", "bitloops/checkpoints/v1"],
    );
    let checkpoint_bucket = repo_root.join("11").join("2233445566");
    fs::create_dir_all(checkpoint_bucket.join("0")).expect("create checkpoint directories");
    fs::create_dir_all(checkpoint_bucket.join("1")).expect("create checkpoint directories");

    let top_metadata = json!({
        "checkpoint_id": "112233445566",
        "strategy": "manual-commit",
        "branch": "main",
        "checkpoints_count": 3,
        "files_touched": ["app.rs"],
        "sessions": [{
            "metadata": "/11/2233445566/0/metadata.json",
            "transcript": "/11/2233445566/0/full.jsonl",
            "context": "/11/2233445566/0/context.md",
            "content_hash": "/11/2233445566/0/content_hash.txt",
            "prompt": "/11/2233445566/0/prompt.txt"
        }, {
            "metadata": "/11/2233445566/1/metadata.json",
            "transcript": "/11/2233445566/1/full.jsonl",
            "context": "/11/2233445566/1/context.md",
            "content_hash": "/11/2233445566/1/content_hash.txt",
            "prompt": "/11/2233445566/1/prompt.txt"
        }],
        "token_usage": {
            "input_tokens": 200,
            "output_tokens": 80,
            "cache_creation_tokens": 20,
            "cache_read_tokens": 10,
            "api_call_count": 6
        }
    });
    let session_zero_metadata = json!({
        "checkpoint_id": "112233445566",
        "session_id": "session-1",
        "checkpoints_count": 1,
        "strategy": "manual-commit",
        "agent": "claude-code",
        "created_at": "2026-02-27T12:00:00Z",
        "cli_version": "0.0.3",
        "files_touched": ["app.rs"],
        "is_task": false,
        "tool_use_id": ""
    });
    let session_one_metadata = json!({
        "checkpoint_id": "112233445566",
        "session_id": "session-2",
        "checkpoints_count": 2,
        "strategy": "manual-commit",
        "agent": "gemini",
        "created_at": "2026-02-27T12:10:00Z",
        "cli_version": "0.0.3",
        "files_touched": ["app.rs"],
        "is_task": false,
        "tool_use_id": ""
    });

    fs::write(
        checkpoint_bucket.join("metadata.json"),
        serde_json::to_string_pretty(&top_metadata).expect("serialize top metadata"),
    )
    .expect("write top metadata");
    fs::write(
        checkpoint_bucket.join("0").join("metadata.json"),
        serde_json::to_string_pretty(&session_zero_metadata).expect("serialize session metadata"),
    )
    .expect("write session metadata");
    fs::write(
        checkpoint_bucket.join("1").join("metadata.json"),
        serde_json::to_string_pretty(&session_one_metadata).expect("serialize session metadata"),
    )
    .expect("write session metadata");
    let session_zero_transcript =
        "{\"type\":\"user\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"A\"}]}}\n";
    let session_one_transcript =
        "{\"type\":\"user\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"B\"}]}}\n";
    fs::write(
        checkpoint_bucket.join("0").join("full.jsonl"),
        session_zero_transcript,
    )
    .expect("write transcript");
    fs::write(
        checkpoint_bucket.join("1").join("full.jsonl"),
        session_one_transcript,
    )
    .expect("write transcript");

    let first_prompt_core = "A".repeat(200);
    let first_prompt = format!(
        "<file_bundle>\nfoo.txt\nbar.md\n</file_bundle>\n<context_block>\nrepo-index\n</context_block>\n   \n\t{first_prompt_core}"
    );
    let session_zero_prompt = format!("{first_prompt}\n\n---\n\nSecond prompt in first session");
    let session_one_prompt = "Second session prompt";
    let session_zero_context = "Context one";
    let session_one_context = "Context two";
    fs::write(
        checkpoint_bucket.join("0").join("prompt.txt"),
        &session_zero_prompt,
    )
    .expect("write prompt");
    fs::write(
        checkpoint_bucket.join("1").join("prompt.txt"),
        session_one_prompt,
    )
    .expect("write prompt");
    fs::write(
        checkpoint_bucket.join("0").join("context.md"),
        session_zero_context,
    )
    .expect("write context");
    fs::write(
        checkpoint_bucket.join("1").join("context.md"),
        session_one_context,
    )
    .expect("write context");

    git_ok(repo_root, &["add", "11"]);
    git_ok(repo_root, &["commit", "-m", "checkpoint metadata"]);
    git_ok(repo_root, &["checkout", "main"]);

    seed_checkpoint_storage_for_dashboard(
        repo_root,
        SeedCheckpointStorage {
            commit_sha: &checkpoint_commit_sha,
            checkpoint_id: "112233445566",
            branch: "main",
            files_touched: &["app.rs"],
            checkpoints_count: 3,
            token_usage: json!({
                "input_tokens": 200,
                "output_tokens": 80,
                "cache_creation_tokens": 20,
                "cache_read_tokens": 10,
                "api_call_count": 6
            }),
            sessions: &[
                SeedCheckpointSession {
                    session_index: 0,
                    session_id: "session-1",
                    agent: "claude-code",
                    created_at: "2026-02-27T12:00:00Z",
                    checkpoints_count: 1,
                    transcript: session_zero_transcript,
                    prompts: &session_zero_prompt,
                    context: session_zero_context,
                },
                SeedCheckpointSession {
                    session_index: 1,
                    session_id: "session-2",
                    agent: "gemini",
                    created_at: "2026-02-27T12:10:00Z",
                    checkpoints_count: 2,
                    transcript: session_one_transcript,
                    prompts: session_one_prompt,
                    context: session_one_context,
                },
            ],
            insert_mapping: true,
        },
    );

    dir
}

pub(super) async fn request_json(app: axum::Router, uri: &str) -> (StatusCode, Value) {
    request_json_with_method(app, Method::GET, uri, Body::empty()).await
}

pub(super) async fn request_json_with_method(
    app: axum::Router,
    method: Method,
    uri: &str,
    body: Body,
) -> (StatusCode, Value) {
    let response = app
        .oneshot(
            Request::builder()
                .method(method)
                .uri(uri)
                .body(body)
                .expect("request"),
        )
        .await
        .expect("router response");
    let status = response.status();
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read body");
    let parsed = serde_json::from_slice::<Value>(&body).unwrap_or_else(|_| json!({}));
    (status, parsed)
}

pub(super) async fn request_json_with_method_and_content_type(
    app: axum::Router,
    method: Method,
    uri: &str,
    content_type: &str,
    body: Body,
) -> (StatusCode, Value) {
    let response = app
        .oneshot(
            Request::builder()
                .method(method)
                .uri(uri)
                .header("content-type", content_type)
                .body(body)
                .expect("request"),
        )
        .await
        .expect("router response");
    let status = response.status();
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read body");
    let parsed = serde_json::from_slice::<Value>(&body).unwrap_or_else(|_| json!({}));
    (status, parsed)
}

pub(super) async fn request_json_with_method_content_type_and_headers(
    app: axum::Router,
    method: Method,
    uri: &str,
    content_type: &str,
    headers: &[(&str, &str)],
    body: Body,
) -> (StatusCode, Value) {
    let mut builder = Request::builder()
        .method(method)
        .uri(uri)
        .header("content-type", content_type);
    for (name, value) in headers {
        builder = builder.header(*name, *value);
    }
    let response = app
        .oneshot(builder.body(body).expect("request"))
        .await
        .expect("router response");
    let status = response.status();
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read body");
    let parsed = serde_json::from_slice::<Value>(&body).unwrap_or_else(|_| json!({}));
    (status, parsed)
}

pub(super) const DASHBOARD_CDN_BASE_URL_ENV: &str = "BITLOOPS_DASHBOARD_CDN_BASE_URL";
pub(super) const DASHBOARD_MANIFEST_URL_ENV: &str = "BITLOOPS_DASHBOARD_MANIFEST_URL";

pub(super) fn with_dashboard_cdn_base_url(base_url: &str) -> ProcessStateGuard {
    enter_env_vars(&[
        (DASHBOARD_MANIFEST_URL_ENV, None),
        (DASHBOARD_CDN_BASE_URL_ENV, Some(base_url)),
    ])
}

pub(super) fn with_dashboard_manifest_url(manifest_url: &str) -> ProcessStateGuard {
    enter_env_vars(&[
        (DASHBOARD_CDN_BASE_URL_ENV, None),
        (DASHBOARD_MANIFEST_URL_ENV, Some(manifest_url)),
    ])
}

pub(super) fn build_bundle_archive(version: &str) -> Vec<u8> {
    let mut tar_builder = tar::Builder::new(Vec::new());

    let index = b"<html><body>installed bundle</body></html>".to_vec();
    let version_json =
        format!(r#"{{"version":"{version}","source_url":"https://cdn.test/bundle.tar.zst"}}"#)
            .into_bytes();

    let mut index_header = tar::Header::new_gnu();
    index_header.set_size(index.len() as u64);
    index_header.set_mode(0o644);
    index_header.set_cksum();
    tar_builder
        .append_data(&mut index_header, "index.html", Cursor::new(index))
        .expect("append index");

    let mut version_header = tar::Header::new_gnu();
    version_header.set_size(version_json.len() as u64);
    version_header.set_mode(0o644);
    version_header.set_cksum();
    tar_builder
        .append_data(
            &mut version_header,
            "version.json",
            Cursor::new(version_json),
        )
        .expect("append version.json");

    let tar_bytes = tar_builder.into_inner().expect("finalize tar");
    zstd::stream::encode_all(Cursor::new(tar_bytes), 0).expect("compress archive")
}

pub(super) fn checksum_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex::encode(hasher.finalize())
}

pub(super) fn setup_local_bundle_cdn(
    archive_bytes: &[u8],
    checksum: &str,
    manifest_version: &str,
) -> TempDir {
    let temp = TempDir::new().expect("local cdn temp dir");
    let root = temp.path();

    fs::write(root.join("bundle.tar.zst"), archive_bytes).expect("write bundle archive");
    fs::write(
        root.join("bundle.tar.zst.sha256"),
        format!("{checksum}  bundle.tar.zst\n"),
    )
    .expect("write checksum");

    let manifest = format!(
        r#"{{"versions":[{{"version":"{version}","min_required_cli_version":"0.0.1","max_required_cli_version":"latest","download_url":"bundle.tar.zst","checksum_url":"bundle.tar.zst.sha256"}}]}}"#,
        version = manifest_version
    );
    fs::write(root.join("bundle_versions.json"), manifest).expect("write manifest");
    temp
}

pub(super) fn setup_local_bundle_cdn_with_manifest(
    manifest: &str,
    archive_bytes: Option<&[u8]>,
    checksum: Option<&str>,
) -> TempDir {
    let temp = TempDir::new().expect("local cdn temp dir");
    let root = temp.path();
    if let Some(bytes) = archive_bytes {
        fs::write(root.join("bundle.tar.zst"), bytes).expect("write bundle archive");
    }
    if let Some(checksum) = checksum {
        fs::write(
            root.join("bundle.tar.zst.sha256"),
            format!("{checksum}  bundle.tar.zst\n"),
        )
        .expect("write checksum");
    }
    fs::write(root.join("bundle_versions.json"), manifest).expect("write manifest");
    temp
}

pub(super) async fn request_text(app: axum::Router, uri: &str) -> (StatusCode, String) {
    let response = app
        .oneshot(
            Request::builder()
                .uri(uri)
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("router response");
    let status = response.status();
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read body");
    (status, String::from_utf8_lossy(&body).into_owned())
}

pub(super) async fn request_text_with_method(
    app: axum::Router,
    method: Method,
    uri: &str,
) -> (StatusCode, String) {
    let response = app
        .oneshot(
            Request::builder()
                .method(method)
                .uri(uri)
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("router response");
    let status = response.status();
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read body");
    (status, String::from_utf8_lossy(&body).into_owned())
}
