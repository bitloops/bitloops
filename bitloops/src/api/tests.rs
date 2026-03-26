#![allow(clippy::await_holding_lock)]

use super::router::build_dashboard_router;
use super::{
    ApiPage, DashboardState, GIT_FIELD_SEPARATOR, GIT_RECORD_SEPARATOR, ServeMode,
    branch_is_excluded, browser_host_for_url, build_branch_commit_log_args, canonical_agent_key,
    dashboard_user, default_bundle_dir_from_home, expand_tilde_with_home, format_dashboard_url,
    has_bundle_index, paginate, parse_branch_commit_log, parse_numstat_output, resolve_bundle_file,
    select_host_with_dashboard_preference,
};
use crate::test_support::git_fixtures::{git_ok, init_test_repo, repo_local_blob_root};
use crate::test_support::process_state::{ProcessStateGuard, enter_env_vars};
use axum::{
    body::{Body, to_bytes},
    http::{Method, Request, StatusCode},
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::fs;
use std::io::Cursor;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::path::{Path, PathBuf};
use tempfile::TempDir;
use tower::util::ServiceExt;

fn insert_commit_checkpoint_mapping(repo_root: &Path, commit_sha: &str, checkpoint_id: &str) {
    let sqlite_path = checkpoint_sqlite_path(repo_root);
    let sqlite =
        crate::storage::SqliteConnectionPool::connect(sqlite_path).expect("connect sqlite");
    sqlite
        .initialise_checkpoint_schema()
        .expect("initialise checkpoint schema");
    let repo_id = crate::host::devql::resolve_repo_id(repo_root).expect("resolve repo id");
    sqlite
        .with_connection(|conn| {
            conn.execute(
                "INSERT INTO commit_checkpoints (commit_sha, checkpoint_id, repo_id)
                 VALUES (?1, ?2, ?3)",
                rusqlite::params![commit_sha, checkpoint_id, repo_id.as_str()],
            )?;
            Ok(())
        })
        .expect("insert commit-checkpoint mapping");
}

fn checkpoint_sqlite_path(repo_root: &Path) -> PathBuf {
    let cfg = crate::config::resolve_store_backend_config_for_repo(repo_root)
        .expect("resolve backend config");
    if let Some(path) = cfg.relational.sqlite_path.as_deref() {
        crate::config::resolve_sqlite_db_path_for_repo(repo_root, Some(path))
            .expect("resolve configured sqlite path")
    } else {
        crate::utils::paths::default_relational_db_path(repo_root)
    }
}

fn write_envelope_config(repo_root: &Path, settings: serde_json::Value) {
    let config_dir = repo_root.join(".bitloops");
    fs::create_dir_all(&config_dir).expect("create config dir");
    fs::write(
        config_dir.join("config.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "version": "1.0",
            "scope": "project",
            "settings": settings
        }))
        .expect("serialise config"),
    )
    .expect("write config");
}

struct SeedCheckpointSession<'a> {
    session_index: i64,
    session_id: &'a str,
    agent: &'a str,
    created_at: &'a str,
    checkpoints_count: i64,
    transcript: &'a str,
    prompts: &'a str,
    context: &'a str,
}

struct SeedCheckpointStorage<'a> {
    commit_sha: &'a str,
    checkpoint_id: &'a str,
    branch: &'a str,
    files_touched: &'a [&'a str],
    checkpoints_count: i64,
    token_usage: serde_json::Value,
    sessions: &'a [SeedCheckpointSession<'a>],
    insert_mapping: bool,
}

fn seed_checkpoint_storage_for_dashboard(repo_root: &Path, seed: SeedCheckpointStorage<'_>) {
    let sqlite_path = checkpoint_sqlite_path(repo_root);
    let sqlite =
        crate::storage::SqliteConnectionPool::connect(sqlite_path).expect("connect sqlite");
    sqlite
        .initialise_checkpoint_schema()
        .expect("initialise checkpoint schema");
    let repo_id = crate::host::devql::resolve_repo_id(repo_root).expect("resolve repo id");
    let files_touched_raw =
        serde_json::to_string(seed.files_touched).expect("serialise files_touched");
    let token_usage_raw = serde_json::to_string(&seed.token_usage).expect("serialise token_usage");

    sqlite
        .with_connection(|conn| {
            conn.execute(
                "INSERT INTO checkpoints (
                    checkpoint_id, repo_id, strategy, branch, cli_version,
                    files_touched, checkpoints_count, token_usage
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                rusqlite::params![
                    seed.checkpoint_id,
                    repo_id.as_str(),
                    "manual-commit",
                    seed.branch,
                    "0.0.3",
                    files_touched_raw.as_str(),
                    seed.checkpoints_count,
                    token_usage_raw.as_str(),
                ],
            )?;

            for session in seed.sessions {
                conn.execute(
                    "INSERT INTO checkpoint_sessions (
                        checkpoint_id, session_id, session_index, agent, turn_id, checkpoints_count,
                        files_touched, is_task, tool_use_id, transcript_identifier_at_start,
                        checkpoint_transcript_start, initial_attribution, token_usage, summary,
                        author_name, author_email, transcript_path, created_at
                    ) VALUES (
                        ?1, ?2, ?3, ?4, '', ?5,
                        ?6, 0, '', '', 0, NULL, NULL, NULL,
                        'Alice', 'alice@example.com', '', ?7
                    )",
                    rusqlite::params![
                        seed.checkpoint_id,
                        session.session_id,
                        session.session_index,
                        session.agent,
                        session.checkpoints_count,
                        files_touched_raw.as_str(),
                        session.created_at,
                    ],
                )?;
            }

            Ok(())
        })
        .expect("insert checkpoint rows");

    let blob_root = repo_local_blob_root(repo_root);

    for session in seed.sessions {
        let blob_payloads = [
            (
                crate::storage::blob::BlobType::Transcript,
                session.transcript,
            ),
            (crate::storage::blob::BlobType::Prompts, session.prompts),
            (crate::storage::blob::BlobType::Context, session.context),
        ];

        for (blob_type, payload) in blob_payloads {
            let key = crate::storage::blob::build_blob_key(
                repo_id.as_str(),
                seed.checkpoint_id,
                session.session_index,
                blob_type,
            );
            let path = blob_root.join(&key);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).expect("create seeded blob parent");
            }
            fs::write(&path, payload.as_bytes()).expect("write seeded blob");
            let reference = crate::storage::blob::CheckpointBlobReference::new(
                seed.checkpoint_id,
                session.session_index,
                blob_type,
                "local",
                key,
                "",
                payload.len() as i64,
            );
            crate::storage::blob::upsert_checkpoint_blob_reference(&sqlite, &reference)
                .expect("upsert checkpoint blob reference");
        }
    }

    if seed.insert_mapping {
        insert_commit_checkpoint_mapping(repo_root, seed.commit_sha, seed.checkpoint_id);
    }
}

fn test_state(repo_root: PathBuf, mode: ServeMode, bundle_dir: PathBuf) -> DashboardState {
    let db = super::db::DashboardDbPools::default();
    DashboardState {
        devql_schema: crate::graphql::build_schema(crate::graphql::DevqlGraphqlContext::new(
            repo_root.clone(),
            db.clone(),
        )),
        repo_root,
        mode,
        db,
        bundle_dir,
    }
}

fn seed_dashboard_repo() -> TempDir {
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
            insert_mapping: true,
        },
    );

    dir
}

fn seed_dashboard_repo_without_commit_mapping() -> TempDir {
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

fn seed_dashboard_repo_multi_session() -> TempDir {
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

async fn request_json(app: axum::Router, uri: &str) -> (StatusCode, Value) {
    request_json_with_method(app, Method::GET, uri, Body::empty()).await
}

async fn request_json_with_method(
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

async fn request_json_with_method_and_content_type(
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

const DASHBOARD_CDN_BASE_URL_ENV: &str = "BITLOOPS_DASHBOARD_CDN_BASE_URL";
const DASHBOARD_MANIFEST_URL_ENV: &str = "BITLOOPS_DASHBOARD_MANIFEST_URL";

fn with_dashboard_cdn_base_url(base_url: &str) -> ProcessStateGuard {
    enter_env_vars(&[
        (DASHBOARD_MANIFEST_URL_ENV, None),
        (DASHBOARD_CDN_BASE_URL_ENV, Some(base_url)),
    ])
}

fn with_dashboard_manifest_url(manifest_url: &str) -> ProcessStateGuard {
    enter_env_vars(&[
        (DASHBOARD_CDN_BASE_URL_ENV, None),
        (DASHBOARD_MANIFEST_URL_ENV, Some(manifest_url)),
    ])
}

fn build_bundle_archive(version: &str) -> Vec<u8> {
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

fn checksum_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

fn setup_local_bundle_cdn(archive_bytes: &[u8], checksum: &str, manifest_version: &str) -> TempDir {
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

fn setup_local_bundle_cdn_with_manifest(
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

async fn request_text(app: axum::Router, uri: &str) -> (StatusCode, String) {
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

async fn request_text_with_method(
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

#[tokio::test]
async fn devql_schema_builds_and_executes_in_process() {
    let temp = TempDir::new().expect("temp dir");
    let schema = crate::graphql::build_schema(crate::graphql::DevqlGraphqlContext::new(
        temp.path().to_path_buf(),
        super::db::DashboardDbPools::default(),
    ));

    let response = schema
        .execute(async_graphql::Request::new(
            r#"{ repo(name: "demo") { id name provider organization } }"#,
        ))
        .await;

    assert!(
        response.errors.is_empty(),
        "graphql errors: {:?}",
        response.errors
    );

    let json = response.data.into_json().expect("graphql data to json");
    assert_eq!(json["repo"]["name"], "demo");
    assert_eq!(json["repo"]["provider"], "local");
}

#[tokio::test]
async fn devql_health_query_reports_backend_and_blob_status_in_process() {
    let repo = seed_dashboard_repo();
    let schema = crate::graphql::build_schema(crate::graphql::DevqlGraphqlContext::new(
        repo.path().to_path_buf(),
        super::db::DashboardDbPools::default(),
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
        super::db::DashboardDbPools::default(),
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
async fn devql_repository_queries_resolve_repo_commit_branch_user_agent_and_checkpoint_data() {
    let repo = seed_dashboard_repo();
    let schema = crate::graphql::build_schema(crate::graphql::DevqlGraphqlContext::new(
        repo.path().to_path_buf(),
        super::db::DashboardDbPools::default(),
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
        super::db::DashboardDbPools::default(),
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
        super::db::DashboardDbPools::default(),
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
        super::db::DashboardDbPools::default(),
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
async fn devql_playground_route_serves_explorer() {
    let temp = TempDir::new().expect("temp dir");
    let app = build_dashboard_router(test_state(
        temp.path().to_path_buf(),
        ServeMode::HelloWorld,
        temp.path().to_path_buf(),
    ));

    let (status, body) = request_text(app, "/devql/playground").await;

    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("DevQL Explorer"));
    assert!(body.contains("/devql"));
}

#[tokio::test]
async fn devql_sdl_route_returns_schema_text() {
    let temp = TempDir::new().expect("temp dir");
    let app = build_dashboard_router(test_state(
        temp.path().to_path_buf(),
        ServeMode::HelloWorld,
        temp.path().to_path_buf(),
    ));

    let (status, body) = request_text(app, "/devql/sdl").await;

    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("health: HealthStatus!"));
    assert!(body.contains("type QueryRoot"));
    assert!(body.contains("repo(name: String!): Repository!"));
}

#[tokio::test]
async fn devql_post_route_executes_graphql_requests() {
    let temp = TempDir::new().expect("temp dir");
    let app = build_dashboard_router(test_state(
        temp.path().to_path_buf(),
        ServeMode::HelloWorld,
        temp.path().to_path_buf(),
    ));

    let (status, payload) = request_json_with_method_and_content_type(
        app,
        Method::POST,
        "/devql",
        "application/json",
        Body::from(
            r#"{"query":"{ repo(name: \"demo\") { name provider } health { blob { backend connected } } }"}"#,
        ),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(payload["data"]["repo"]["name"], "demo");
    assert_eq!(payload["data"]["repo"]["provider"], "local");
    assert_eq!(payload["data"]["health"]["blob"]["backend"], "local");
    assert_eq!(payload["data"]["health"]["blob"]["connected"], true);
}

#[tokio::test]
async fn devql_ws_route_is_registered() {
    let temp = TempDir::new().expect("temp dir");
    let app = build_dashboard_router(test_state(
        temp.path().to_path_buf(),
        ServeMode::HelloWorld,
        temp.path().to_path_buf(),
    ));

    let (status, _) = request_text_with_method(app, Method::GET, "/devql/ws").await;

    assert_ne!(status, StatusCode::NOT_FOUND);
}

#[test]
fn select_host_prefers_bitloops_local_when_config_enabled() {
    let selected = select_host_with_dashboard_preference(None, true);
    assert_eq!(selected, "bitloops.local");
}

#[test]
fn select_host_falls_back_to_localhost_when_config_disabled() {
    let selected = select_host_with_dashboard_preference(None, false);
    assert_eq!(selected, "127.0.0.1");
}

#[test]
fn select_host_respects_explicit_host() {
    let selected = select_host_with_dashboard_preference(Some("localhost"), true);
    assert_eq!(selected, "localhost");
}

#[test]
fn default_bundle_dir_uses_home_directory() {
    let path = default_bundle_dir_from_home(Some(Path::new("/tmp/home")));
    assert_eq!(path, PathBuf::from("/tmp/home/.bitloops/dashboard/bundle"));
}

#[test]
fn expand_tilde_replaces_user_home_prefix() {
    let expanded = expand_tilde_with_home(Path::new("~/bundle"), Some(Path::new("/tmp/home")));
    assert_eq!(expanded, PathBuf::from("/tmp/home/bundle"));
}

#[test]
fn resolve_bundle_file_rejects_parent_traversal() {
    let root = Path::new("/tmp/root");
    let resolved = resolve_bundle_file(root, "/../../etc/passwd");
    assert!(resolved.is_none());
}

#[test]
fn resolve_bundle_file_maps_root_to_index() {
    let root = Path::new("/tmp/root");
    let resolved = resolve_bundle_file(root, "/").expect("path should resolve");
    assert_eq!(resolved, PathBuf::from("/tmp/root/index.html"));
}

#[cfg(unix)]
#[tokio::test]
async fn bundle_request_does_not_follow_symlink_outside_bundle() {
    let bundle_dir = TempDir::new().expect("bundle temp dir");
    let outside_dir = TempDir::new().expect("outside temp dir");

    let secret = outside_dir.path().join("secret.txt");
    fs::write(&secret, "secret").expect("write secret");
    fs::write(bundle_dir.path().join("index.html"), "safe index").expect("write index");
    std::os::unix::fs::symlink(&secret, bundle_dir.path().join("leak.txt")).expect("symlink");

    let app = build_dashboard_router(test_state(
        bundle_dir.path().to_path_buf(),
        ServeMode::Bundle(bundle_dir.path().to_path_buf()),
        bundle_dir.path().to_path_buf(),
    ));

    let (status, body) = request_text(app, "/leak.txt").await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("safe index"));
    assert!(!body.contains("secret"));
}

#[cfg(unix)]
#[tokio::test]
async fn bundle_request_rejects_symlinked_index_outside_bundle() {
    let bundle_dir = TempDir::new().expect("bundle temp dir");
    let outside_dir = TempDir::new().expect("outside temp dir");

    let secret = outside_dir.path().join("secret.html");
    fs::write(&secret, "secret").expect("write secret");
    std::os::unix::fs::symlink(&secret, bundle_dir.path().join("index.html")).expect("symlink");

    let app = build_dashboard_router(test_state(
        bundle_dir.path().to_path_buf(),
        ServeMode::Bundle(bundle_dir.path().to_path_buf()),
        bundle_dir.path().to_path_buf(),
    ));

    let (status, body) = request_text(app, "/").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body, "Bundle not found.\n");
}

#[test]
fn has_bundle_index_true_when_index_exists() {
    let temp = TempDir::new().expect("temp dir");
    std::fs::write(temp.path().join("index.html"), "ok").expect("write file");
    assert!(has_bundle_index(temp.path()));
}

#[test]
fn browser_host_uses_loopback_for_unspecified_ipv4_bind() {
    let host = browser_host_for_url(
        "0.0.0.0",
        SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 5667),
    );
    assert_eq!(host, "127.0.0.1");
}

#[test]
fn browser_host_uses_localhost_for_unspecified_ipv6_bind() {
    let host = browser_host_for_url(
        "::",
        SocketAddr::new(IpAddr::V6(Ipv6Addr::UNSPECIFIED), 5667),
    );
    assert_eq!(host, "localhost");
}

#[test]
fn format_dashboard_url_wraps_ipv6_hosts() {
    assert_eq!(format_dashboard_url("::1", 5667), "http://[::1]:5667");
}

#[test]
fn dashboard_user_uses_email_as_canonical_key() {
    let user = dashboard_user("Alice", "ALICE@Example.com");
    assert_eq!(user.key, "alice@example.com");
    assert_eq!(user.name, "Alice");
    assert_eq!(user.email, "alice@example.com");
}

#[test]
fn dashboard_user_falls_back_to_name_key_when_email_missing() {
    let user = dashboard_user("Alice Example", "");
    assert_eq!(user.key, "name:alice example");
    assert_eq!(user.name, "Alice Example");
    assert_eq!(user.email, "");
}

#[test]
fn canonical_agent_key_normalizes_to_kebab_case() {
    assert_eq!(canonical_agent_key("Claude Code"), "claude-code");
    assert_eq!(canonical_agent_key("Codex"), "codex");
    assert_eq!(canonical_agent_key("Gemini"), "gemini");
    assert_eq!(canonical_agent_key("cursor"), "cursor");
    assert_eq!(canonical_agent_key(""), "");
}

#[test]
fn branch_filter_excludes_internal_branches() {
    assert!(branch_is_excluded("bitloops/checkpoints/v1"));
    assert!(branch_is_excluded("bitloops/feature-shadow"));
    assert!(branch_is_excluded("origin/bitloops/feature-shadow"));
    assert!(branch_is_excluded(
        "refs/remotes/origin/bitloops/feature-shadow"
    ));
    assert!(branch_is_excluded("bitloops/legacy-shadow"));
    assert!(!branch_is_excluded("main"));
    assert!(!branch_is_excluded("origin/release/1.0"));
}

#[test]
fn build_branch_commit_log_args_uses_commit_time_range() {
    let args = build_branch_commit_log_args("main", Some(1700000000), Some(1700001000), 0);
    assert!(args.iter().any(|arg| arg == "--since=@1700000000"));
    assert!(args.iter().any(|arg| arg == "--until=@1700001000"));
    assert!(args.iter().any(|arg| arg == "main"));
    assert!(
        args.windows(2)
            .any(|pair| pair[0] == "--max-count" && pair[1] == "1")
    );
}

#[test]
fn parse_branch_commit_log_skips_malformed_records_without_crashing() {
    let raw = format!(
        "abcd{f}parent{f}Alice{f}alice@example.com{f}1700000000{f}msg{f}aabbccddeeff{r}broken{r}",
        f = GIT_FIELD_SEPARATOR,
        r = GIT_RECORD_SEPARATOR
    );
    let parsed = parse_branch_commit_log(&raw);
    assert_eq!(parsed.len(), 1);
    assert_eq!(parsed[0].sha, "abcd");
    assert_eq!(parsed[0].checkpoint_id, "");
}

#[test]
fn parse_branch_commit_log_never_extracts_checkpoint_ids_from_git_log_records() {
    let raw = format!(
        "abcd{f}parent{f}Alice{f}alice@example.com{f}1700000000{f}msg{f}invalid-checkpoint{r}",
        f = GIT_FIELD_SEPARATOR,
        r = GIT_RECORD_SEPARATOR
    );
    let parsed = parse_branch_commit_log(&raw);
    assert_eq!(parsed.len(), 1);
    assert_eq!(parsed[0].checkpoint_id, "");
}

#[test]
fn paginate_clamps_limit_and_offset() {
    let page = ApiPage {
        limit: usize::MAX,
        offset: 3,
    };
    let items = vec![1, 2, 3, 4, 5, 6];
    let paged = paginate(&items, page);
    assert_eq!(paged, vec![4, 5, 6]);
}

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

#[test]
fn parse_numstat_output_parses_normal_line() {
    let raw = "5\t2\tsrc/a.rs\n";
    let stats = parse_numstat_output(raw);
    assert_eq!(stats.get("src/a.rs"), Some(&(5, 2)));
}

#[test]
fn parse_numstat_output_treats_binary_as_zero() {
    let raw = "-\t-\tassets/logo.png\n";
    let stats = parse_numstat_output(raw);
    assert_eq!(stats.get("assets/logo.png"), Some(&(0, 0)));
}

#[test]
fn parse_numstat_output_ignores_malformed_lines() {
    let raw = "not-a-valid-line\n5\t2\tsrc/a.rs\n";
    let stats = parse_numstat_output(raw);
    assert_eq!(stats.len(), 1);
    assert_eq!(stats.get("src/a.rs"), Some(&(5, 2)));
}

#[test]
fn parse_numstat_output_accumulates_duplicate_paths() {
    let raw = "3\t1\tsrc/a.rs\n2\t0\tsrc/a.rs\n";
    let stats = parse_numstat_output(raw);
    assert_eq!(stats.get("src/a.rs"), Some(&(5, 1)));
}
