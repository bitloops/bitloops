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
use crate::test_support::process_state::{ProcessStateGuard, enter_env_vars, enter_process_state};
use async_graphql::futures_util::StreamExt;
use axum::{
    body::{Body, to_bytes},
    http::{Method, Request, StatusCode},
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::fmt::Write as _;
use std::fs;
use std::io::{Cursor, Read, Write};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::thread;
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

struct MockHttpResponse {
    status_code: u16,
    body: String,
}

impl MockHttpResponse {
    fn json(status_code: u16, body: serde_json::Value) -> Self {
        Self {
            status_code,
            body: serde_json::to_string(&body).expect("serialise mock body"),
        }
    }
}

struct MockSequentialHttpServer {
    url: String,
    handle: Option<thread::JoinHandle<()>>,
}

impl MockSequentialHttpServer {
    fn start(responses: Vec<MockHttpResponse>) -> Self {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind mock server");
        listener
            .set_nonblocking(true)
            .expect("set nonblocking mock server");
        let addr = listener.local_addr().expect("mock server addr");
        let url = format!("http://{}", addr);

        let handle = thread::spawn(move || {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
            let mut responses = std::collections::VecDeque::from(responses);

            while let Some(response) = responses.pop_front() {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        let mut buffer = [0_u8; 8192];
                        let _ = stream.read(&mut buffer);

                        let status_text = match response.status_code {
                            200 => "OK",
                            404 => "Not Found",
                            500 => "Internal Server Error",
                            _ => "Status",
                        };
                        let response_text = format!(
                            "HTTP/1.1 {} {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                            response.status_code,
                            status_text,
                            response.body.len(),
                            response.body
                        );
                        let _ = stream.write_all(response_text.as_bytes());
                    }
                    Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                        if std::time::Instant::now() >= deadline {
                            break;
                        }
                        thread::sleep(std::time::Duration::from_millis(10));
                        responses.push_front(response);
                    }
                    Err(_) => break,
                }
            }
        });

        Self {
            url,
            handle: Some(handle),
        }
    }
}

impl Drop for MockSequentialHttpServer {
    fn drop(&mut self) {
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

struct SeedGraphqlEvent<'a> {
    event_id: &'a str,
    event_time: &'a str,
    checkpoint_id: &'a str,
    session_id: &'a str,
    commit_sha: &'a str,
    branch: &'a str,
    event_type: &'a str,
    agent: &'a str,
    strategy: &'a str,
    files_touched: &'a [&'a str],
    payload: serde_json::Value,
}

fn duckdb_literal(value: &str) -> String {
    value.replace('\'', "''")
}

fn seed_duckdb_events(repo_root: &Path, events: &[SeedGraphqlEvent<'_>]) {
    let repo_id = crate::host::devql::resolve_repo_id(repo_root).expect("resolve repo id");
    let backend_config = crate::config::resolve_store_backend_config_for_repo(repo_root)
        .expect("resolve backend config");
    let duckdb_path = backend_config
        .events
        .resolve_duckdb_db_path_for_repo(repo_root);
    if let Some(parent) = duckdb_path.parent() {
        fs::create_dir_all(parent).expect("create duckdb parent");
    }

    let conn = duckdb::Connection::open(&duckdb_path).expect("open duckdb");
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS checkpoint_events (
            event_id VARCHAR PRIMARY KEY,
            event_time VARCHAR,
            repo_id VARCHAR,
            checkpoint_id VARCHAR,
            session_id VARCHAR,
            commit_sha VARCHAR,
            branch VARCHAR,
            event_type VARCHAR,
            agent VARCHAR,
            strategy VARCHAR,
            files_touched VARCHAR,
            payload VARCHAR
        );
        "#,
    )
    .expect("create checkpoint_events table");

    for event in events {
        let files_touched =
            serde_json::to_string(event.files_touched).expect("serialise files_touched");
        let payload = serde_json::to_string(&event.payload).expect("serialise payload");
        let sql = format!(
            "INSERT INTO checkpoint_events (
                event_id, event_time, repo_id, checkpoint_id, session_id, commit_sha,
                branch, event_type, agent, strategy, files_touched, payload
            ) VALUES (
                '{event_id}', '{event_time}', '{repo_id}', '{checkpoint_id}', '{session_id}',
                '{commit_sha}', '{branch}', '{event_type}', '{agent}', '{strategy}',
                '{files_touched}', '{payload}'
            )",
            event_id = duckdb_literal(event.event_id),
            event_time = duckdb_literal(event.event_time),
            repo_id = duckdb_literal(repo_id.as_str()),
            checkpoint_id = duckdb_literal(event.checkpoint_id),
            session_id = duckdb_literal(event.session_id),
            commit_sha = duckdb_literal(event.commit_sha),
            branch = duckdb_literal(event.branch),
            event_type = duckdb_literal(event.event_type),
            agent = duckdb_literal(event.agent),
            strategy = duckdb_literal(event.strategy),
            files_touched = duckdb_literal(&files_touched),
            payload = duckdb_literal(&payload),
        );
        conn.execute_batch(&sql).expect("insert checkpoint event");
    }
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

fn seed_dashboard_repo_with_duckdb_events() -> TempDir {
    let repo = seed_dashboard_repo();
    let head_commit = git_ok(repo.path(), &["rev-parse", "HEAD"]);
    let previous_commit = git_ok(repo.path(), &["rev-parse", "HEAD^"]);

    seed_duckdb_events(
        repo.path(),
        &[
            SeedGraphqlEvent {
                event_id: "evt-dashboard-head",
                event_time: "2026-03-26T09:30:00Z",
                checkpoint_id: "checkpoint-dashboard-head",
                session_id: "session-dashboard-head",
                commit_sha: &head_commit,
                branch: "main",
                event_type: "checkpoint_committed",
                agent: "codex",
                strategy: "manual-commit",
                files_touched: &["app.rs"],
                payload: json!({"source": "dashboard-head"}),
            },
            SeedGraphqlEvent {
                event_id: "evt-dashboard-previous",
                event_time: "2026-03-26T09:15:00Z",
                checkpoint_id: "checkpoint-dashboard-previous",
                session_id: "session-dashboard-previous",
                commit_sha: &previous_commit,
                branch: "main",
                event_type: "checkpoint_committed",
                agent: "codex",
                strategy: "manual-commit",
                files_touched: &["app.rs"],
                payload: json!({"source": "dashboard-previous"}),
            },
        ],
    );

    repo
}

fn seed_graphql_devql_repo() -> TempDir {
    let dir = TempDir::new().expect("temp dir");
    let repo_root = dir.path();

    init_test_repo(repo_root, "main", "Alice", "alice@example.com");
    fs::create_dir_all(repo_root.join("src")).expect("create src dir");
    fs::write(
        repo_root.join("src/caller.ts"),
        "export function caller() {\n  return target();\n}\nexport function helper() {\n  return missing();\n}\n",
    )
    .expect("write caller.ts");
    fs::write(
        repo_root.join("src/target.ts"),
        "export function target() {\n  return 42;\n}\n",
    )
    .expect("write target.ts");
    fs::write(
        repo_root.join("src/orphan.ts"),
        "export function orphan() {\n  return 'orphan';\n}\n",
    )
    .expect("write orphan.ts");
    git_ok(repo_root, &["add", "."]);
    git_ok(repo_root, &["commit", "-m", "Seed GraphQL DevQL repo"]);
    let commit_sha = git_ok(repo_root, &["rev-parse", "HEAD"]);

    let sqlite_path = repo_root
        .join(".bitloops")
        .join("stores")
        .join("graphql.sqlite");
    crate::storage::init::init_database(&sqlite_path, false, &commit_sha)
        .expect("initialise GraphQL sqlite store");
    write_envelope_config(
        repo_root,
        json!({
            "stores": {
                "relational": {
                    "sqlite_path": sqlite_path.to_string_lossy()
                }
            }
        }),
    );

    let repo_id = crate::host::devql::resolve_repo_id(repo_root).expect("resolve repo id");
    let conn = rusqlite::Connection::open(&sqlite_path).expect("open GraphQL sqlite store");
    conn.execute(
        "INSERT INTO repositories (repo_id, provider, organization, name, default_branch)
         VALUES (?1, 'local', 'local', 'graphql-devql', 'main')",
        rusqlite::params![repo_id.as_str()],
    )
    .expect("insert repository row");
    conn.execute(
        "INSERT INTO commits (commit_sha, repo_id, author_name, author_email, commit_message, committed_at)
         VALUES (?1, ?2, 'Alice', 'alice@example.com', 'Seed GraphQL DevQL repo', '2026-03-26T09:00:00Z')",
        rusqlite::params![commit_sha.as_str(), repo_id.as_str()],
    )
    .expect("insert commit row");

    for (path, blob_sha) in [
        ("src/caller.ts", "blob-caller"),
        ("src/target.ts", "blob-target"),
        ("src/orphan.ts", "blob-orphan"),
    ] {
        conn.execute(
            "INSERT INTO file_state (repo_id, commit_sha, path, blob_sha)
             VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![repo_id.as_str(), commit_sha.as_str(), path, blob_sha],
        )
        .expect("insert file_state row");
        conn.execute(
            "INSERT INTO current_file_state (repo_id, path, commit_sha, blob_sha, committed_at)
             VALUES (?1, ?2, ?3, ?4, '2026-03-26T09:00:00Z')",
            rusqlite::params![repo_id.as_str(), path, commit_sha.as_str(), blob_sha],
        )
        .expect("insert current_file_state row");
    }

    let artefacts = [
        (
            "file::caller",
            "artefact::file-caller",
            "blob-caller",
            "src/caller.ts",
            "file",
            "source_file",
            "src/caller.ts",
            Option::<&str>::None,
            1_i64,
            6_i64,
        ),
        (
            "sym::caller",
            "artefact::caller",
            "blob-caller",
            "src/caller.ts",
            "function",
            "function_declaration",
            "src/caller.ts::caller",
            Some("artefact::file-caller"),
            1_i64,
            3_i64,
        ),
        (
            "sym::helper",
            "artefact::helper",
            "blob-caller",
            "src/caller.ts",
            "function",
            "function_declaration",
            "src/caller.ts::helper",
            Some("artefact::file-caller"),
            4_i64,
            6_i64,
        ),
        (
            "file::target",
            "artefact::file-target",
            "blob-target",
            "src/target.ts",
            "file",
            "source_file",
            "src/target.ts",
            Option::<&str>::None,
            1_i64,
            3_i64,
        ),
        (
            "sym::target",
            "artefact::target",
            "blob-target",
            "src/target.ts",
            "function",
            "function_declaration",
            "src/target.ts::target",
            Some("artefact::file-target"),
            1_i64,
            3_i64,
        ),
        (
            "file::orphan",
            "artefact::file-orphan",
            "blob-orphan",
            "src/orphan.ts",
            "file",
            "source_file",
            "src/orphan.ts",
            Option::<&str>::None,
            1_i64,
            3_i64,
        ),
        (
            "sym::orphan",
            "artefact::orphan",
            "blob-orphan",
            "src/orphan.ts",
            "function",
            "function_declaration",
            "src/orphan.ts::orphan",
            Some("artefact::file-orphan"),
            1_i64,
            3_i64,
        ),
    ];

    for (
        symbol_id,
        artefact_id,
        blob_sha,
        path,
        canonical_kind,
        language_kind,
        symbol_fqn,
        parent_artefact_id,
        start_line,
        end_line,
    ) in artefacts
    {
        let parent_symbol_id = match parent_artefact_id {
            Some("artefact::file-caller") => Some("file::caller"),
            Some("artefact::file-target") => Some("file::target"),
            Some("artefact::file-orphan") => Some("file::orphan"),
            _ => None,
        };
        conn.execute(
            "INSERT INTO artefacts (
                artefact_id, symbol_id, repo_id, blob_sha, path, language, canonical_kind,
                language_kind, symbol_fqn, parent_artefact_id, start_line, end_line,
                start_byte, end_byte, signature, modifiers, docstring, content_hash, created_at
            ) VALUES (
                ?1, ?2, ?3, ?4, ?5, 'typescript', ?6,
                ?7, ?8, ?9, ?10, ?11, 0, ?12, NULL, ?13, ?14, ?15, '2026-03-26T09:00:00Z'
            )",
            rusqlite::params![
                artefact_id,
                symbol_id,
                repo_id.as_str(),
                blob_sha,
                path,
                canonical_kind,
                language_kind,
                symbol_fqn,
                parent_artefact_id,
                start_line,
                end_line,
                end_line * 10,
                if canonical_kind == "file" {
                    "[]"
                } else {
                    "[\"export\"]"
                },
                if canonical_kind == "file" {
                    Option::<&str>::None
                } else {
                    Some("Example docstring")
                },
                format!("hash-{artefact_id}"),
            ],
        )
        .expect("insert artefact row");
        conn.execute(
            "INSERT INTO artefacts_current (
                repo_id, branch, symbol_id, artefact_id, commit_sha, revision_kind, revision_id,
                temp_checkpoint_id, blob_sha, path, language, canonical_kind, language_kind,
                symbol_fqn, parent_symbol_id, parent_artefact_id, start_line, end_line,
                start_byte, end_byte, signature, modifiers, docstring, content_hash, updated_at
            ) VALUES (
                ?1, 'main', ?2, ?3, ?4, 'commit', ?4,
                NULL, ?5, ?6, 'typescript', ?7, ?8,
                ?9, ?10, ?11, ?12, ?13,
                0, ?14, NULL, ?15, ?16, ?17, '2026-03-26T09:00:00Z'
            )",
            rusqlite::params![
                repo_id.as_str(),
                symbol_id,
                artefact_id,
                commit_sha.as_str(),
                blob_sha,
                path,
                canonical_kind,
                language_kind,
                symbol_fqn,
                parent_symbol_id,
                parent_artefact_id,
                start_line,
                end_line,
                end_line * 10,
                if canonical_kind == "file" {
                    "[]"
                } else {
                    "[\"export\"]"
                },
                if canonical_kind == "file" {
                    Option::<&str>::None
                } else {
                    Some("Example docstring")
                },
                format!("hash-{artefact_id}"),
            ],
        )
        .expect("insert artefact current row");
    }

    for (
        edge_id,
        path,
        from_symbol_id,
        from_artefact_id,
        to_symbol_id,
        to_artefact_id,
        to_symbol_ref,
        line,
        metadata,
    ) in [
        (
            "edge-resolved",
            "src/caller.ts",
            "sym::caller",
            "artefact::caller",
            Some("sym::target"),
            Some("artefact::target"),
            Some("src/target.ts::target"),
            2_i64,
            "{\"resolution\":\"local\"}",
        ),
        (
            "edge-unresolved",
            "src/caller.ts",
            "sym::helper",
            "artefact::helper",
            Option::<&str>::None,
            Option::<&str>::None,
            Some("src/missing.ts::missing"),
            5_i64,
            "{\"resolution\":\"unresolved\"}",
        ),
    ] {
        conn.execute(
            "INSERT INTO artefact_edges_current (
                edge_id, repo_id, branch, commit_sha, revision_kind, revision_id,
                temp_checkpoint_id, blob_sha, path, from_symbol_id, from_artefact_id,
                to_symbol_id, to_artefact_id, to_symbol_ref, edge_kind, language,
                start_line, end_line, metadata, updated_at
            ) VALUES (
                ?1, ?2, 'main', ?3, 'commit', ?3,
                NULL, 'blob-caller', ?4, ?5, ?6,
                ?7, ?8, ?9, 'calls', 'typescript',
                ?10, ?10, ?11, '2026-03-26T09:00:00Z'
            )",
            rusqlite::params![
                edge_id,
                repo_id.as_str(),
                commit_sha.as_str(),
                path,
                from_symbol_id,
                from_artefact_id,
                to_symbol_id,
                to_artefact_id,
                to_symbol_ref,
                line,
                metadata,
            ],
        )
        .expect("insert edge current row");
    }

    dir
}

fn seed_graphql_mutation_repo() -> TempDir {
    let dir = TempDir::new().expect("temp dir");
    let repo_root = dir.path();

    init_test_repo(repo_root, "main", "Alice", "alice@example.com");
    fs::create_dir_all(repo_root.join("src")).expect("create src dir");
    fs::write(
        repo_root.join("src/lib.rs"),
        "pub fn answer() -> i32 {\n    42\n}\n",
    )
    .expect("write lib.rs");
    git_ok(repo_root, &["add", "."]);
    git_ok(repo_root, &["commit", "-m", "Seed GraphQL mutation repo"]);

    write_envelope_config(
        repo_root,
        json!({
            "stores": {
                "relational": {
                    "sqlite_path": ".bitloops/stores/mutations.sqlite"
                },
                "events": {
                    "duckdb_path": ".bitloops/stores/mutations.duckdb"
                },
                "embedding_provider": "disabled"
            },
            "semantic": {
                "provider": "disabled"
            }
        }),
    );

    dir
}

fn seed_graphql_knowledge_mutation_repo(jira_site_url: &str) -> TempDir {
    let dir = TempDir::new().expect("temp dir");
    let repo_root = dir.path();

    init_test_repo(repo_root, "main", "Alice", "alice@example.com");
    fs::create_dir_all(repo_root.join("src")).expect("create src dir");
    fs::write(
        repo_root.join("src/lib.rs"),
        "pub fn answer() -> i32 {\n    42\n}\n",
    )
    .expect("write lib.rs");
    git_ok(repo_root, &["add", "."]);
    git_ok(
        repo_root,
        &["commit", "-m", "Seed GraphQL knowledge mutation repo"],
    );

    write_envelope_config(
        repo_root,
        json!({
            "stores": {
                "relational": {
                    "sqlite_path": ".bitloops/stores/knowledge-mutations.sqlite"
                },
                "events": {
                    "duckdb_path": ".bitloops/stores/knowledge-mutations.duckdb"
                },
                "embedding_provider": "disabled"
            },
            "semantic": {
                "provider": "disabled"
            },
            "knowledge": {
                "providers": {
                    "jira": {
                        "site_url": jira_site_url,
                        "email": "jira@example.com",
                        "token": "jira-token"
                    }
                }
            }
        }),
    );

    dir
}

fn knowledge_duckdb_path(repo_root: &Path) -> PathBuf {
    crate::config::resolve_store_backend_config_for_repo(repo_root)
        .expect("resolve backend config")
        .events
        .resolve_duckdb_db_path_for_repo(repo_root)
}

fn seed_graphql_chat_history_data(repo_root: &Path) {
    let commit_sha = git_ok(repo_root, &["rev-parse", "HEAD"]);

    let caller_sessions = [SeedCheckpointSession {
        session_index: 0,
        session_id: "session-chat-caller",
        agent: "codex",
        created_at: "2026-03-26T09:05:00Z",
        checkpoints_count: 1,
        transcript: r#"{"role":"user","content":"Explain caller()"}
{"role":"assistant","content":"caller() delegates directly to target()."}"#,
        prompts: "Explain caller()",
        context: "",
    }];
    seed_checkpoint_storage_for_dashboard(
        repo_root,
        SeedCheckpointStorage {
            commit_sha: &commit_sha,
            checkpoint_id: "checkpoint-chat-caller",
            branch: "main",
            files_touched: &["src/caller.ts"],
            checkpoints_count: 1,
            token_usage: json!({"input": 12, "output": 8}),
            sessions: &caller_sessions,
            insert_mapping: false,
        },
    );

    let target_sessions = [SeedCheckpointSession {
        session_index: 0,
        session_id: "session-chat-target",
        agent: "gemini",
        created_at: "2026-03-26T09:15:00Z",
        checkpoints_count: 1,
        transcript: r#"{"messages":[{"type":"user","content":"What does target() return?"},{"type":"gemini","content":"target() returns 42."}]}"#,
        prompts: "What does target() return?",
        context: "",
    }];
    seed_checkpoint_storage_for_dashboard(
        repo_root,
        SeedCheckpointStorage {
            commit_sha: &commit_sha,
            checkpoint_id: "checkpoint-chat-target",
            branch: "main",
            files_touched: &["src/target.ts"],
            checkpoints_count: 1,
            token_usage: json!({"input": 9, "output": 7}),
            sessions: &target_sessions,
            insert_mapping: false,
        },
    );

    seed_duckdb_events(
        repo_root,
        &[
            SeedGraphqlEvent {
                event_id: "evt-chat-caller",
                event_time: "2026-03-26T09:05:00Z",
                checkpoint_id: "checkpoint-chat-caller",
                session_id: "session-chat-caller",
                commit_sha: &commit_sha,
                branch: "main",
                event_type: "checkpoint_committed",
                agent: "codex",
                strategy: "manual-commit",
                files_touched: &["src/caller.ts"],
                payload: json!({"source": "chat-history"}),
            },
            SeedGraphqlEvent {
                event_id: "evt-chat-target",
                event_time: "2026-03-26T09:15:00Z",
                checkpoint_id: "checkpoint-chat-target",
                session_id: "session-chat-target",
                commit_sha: &commit_sha,
                branch: "main",
                event_type: "checkpoint_committed",
                agent: "gemini",
                strategy: "manual-commit",
                files_touched: &["src/target.ts"],
                payload: json!({"source": "chat-history"}),
            },
        ],
    );
}

#[derive(Debug, Clone)]
struct SeededKnowledgeFixture {
    primary_item_id: String,
    primary_latest_version_id: String,
    secondary_item_id: String,
    secondary_latest_version_id: String,
}

fn seed_graphql_knowledge_data(repo_root: &Path) -> SeededKnowledgeFixture {
    let repo_id = crate::host::devql::resolve_repo_id(repo_root).expect("resolve repo id");
    let backend_config = crate::config::resolve_store_backend_config_for_repo(repo_root)
        .expect("resolve backend config");
    let sqlite_path = backend_config
        .relational
        .resolve_sqlite_db_path_for_repo(repo_root)
        .expect("resolve sqlite path");
    let duckdb_path = backend_config
        .events
        .resolve_duckdb_db_path_for_repo(repo_root);
    if let Some(parent) = duckdb_path.parent() {
        fs::create_dir_all(parent).expect("create duckdb parent");
    }

    let sqlite = rusqlite::Connection::open(&sqlite_path).expect("open sqlite");
    sqlite
        .execute_batch(crate::host::devql::knowledge_schema_sql_sqlite())
        .expect("initialise knowledge sqlite schema");

    let duckdb = duckdb::Connection::open(&duckdb_path).expect("open duckdb");
    duckdb
        .execute_batch(crate::host::devql::knowledge_schema_sql_duckdb())
        .expect("initialise knowledge duckdb schema");

    let primary_source_id = crate::capability_packs::knowledge::storage::knowledge_source_id(
        "https://bitloops.atlassian.net/browse/CLI-1521",
    );
    let primary_item_id = crate::capability_packs::knowledge::storage::knowledge_item_id(
        repo_id.as_str(),
        &primary_source_id,
    );
    let primary_v1_payload = json!({
        "raw_payload": {
            "key": "CLI-1521",
            "summary": "Implement knowledge queries"
        },
        "body_text": "Initial GraphQL knowledge design.",
        "body_html": "<p>Initial GraphQL knowledge design.</p>",
        "body_adf": null,
        "discussion": null
    });
    let primary_v2_payload = json!({
        "raw_payload": {
            "key": "CLI-1521",
            "summary": "Implement knowledge queries and payload loading"
        },
        "body_text": "Deliver the typed GraphQL knowledge model and lazy payload reads.",
        "body_html": "<p>Deliver the typed GraphQL knowledge model and lazy payload reads.</p>",
        "body_adf": null,
        "discussion": null
    });
    let primary_v1_bytes =
        crate::capability_packs::knowledge::storage::serialize_payload(&primary_v1_payload)
            .expect("serialise primary v1 payload");
    let primary_v2_bytes =
        crate::capability_packs::knowledge::storage::serialize_payload(&primary_v2_payload)
            .expect("serialise primary v2 payload");
    let primary_v1_hash =
        crate::capability_packs::knowledge::storage::content_hash(&primary_v1_bytes);
    let primary_v2_hash =
        crate::capability_packs::knowledge::storage::content_hash(&primary_v2_bytes);
    let primary_v1_id = crate::capability_packs::knowledge::storage::knowledge_item_version_id(
        &primary_item_id,
        &primary_v1_hash,
    );
    let primary_v2_id = crate::capability_packs::knowledge::storage::knowledge_item_version_id(
        &primary_item_id,
        &primary_v2_hash,
    );

    let secondary_source_id = crate::capability_packs::knowledge::storage::knowledge_source_id(
        "https://github.com/bitloops/bitloops/issues/42",
    );
    let secondary_item_id = crate::capability_packs::knowledge::storage::knowledge_item_id(
        repo_id.as_str(),
        &secondary_source_id,
    );
    let secondary_payload = json!({
        "raw_payload": {
            "number": 42,
            "title": "Secondary GraphQL knowledge item"
        },
        "body_text": "Secondary knowledge item used for relation traversal tests.",
        "body_html": null,
        "body_adf": null,
        "discussion": null
    });
    let secondary_bytes =
        crate::capability_packs::knowledge::storage::serialize_payload(&secondary_payload)
            .expect("serialise secondary payload");
    let secondary_hash =
        crate::capability_packs::knowledge::storage::content_hash(&secondary_bytes);
    let secondary_v1_id = crate::capability_packs::knowledge::storage::knowledge_item_version_id(
        &secondary_item_id,
        &secondary_hash,
    );

    sqlite
        .execute(
            "INSERT INTO knowledge_sources (
                knowledge_source_id, provider, source_kind, canonical_external_id, canonical_url,
                provenance_json, created_at, updated_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            rusqlite::params![
                primary_source_id.as_str(),
                "jira",
                "jira_issue",
                "https://bitloops.atlassian.net/browse/CLI-1521",
                "https://bitloops.atlassian.net/browse/CLI-1521",
                "{\"seed\":\"graphql-tests\"}",
                "2026-03-25T09:00:00Z",
                "2026-03-26T09:30:00Z",
            ],
        )
        .expect("insert primary source");
    sqlite
        .execute(
            "INSERT INTO knowledge_sources (
                knowledge_source_id, provider, source_kind, canonical_external_id, canonical_url,
                provenance_json, created_at, updated_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            rusqlite::params![
                secondary_source_id.as_str(),
                "github",
                "github_issue",
                "https://github.com/bitloops/bitloops/issues/42",
                "https://github.com/bitloops/bitloops/issues/42",
                "{\"seed\":\"graphql-tests\"}",
                "2026-03-25T08:00:00Z",
                "2026-03-26T08:30:00Z",
            ],
        )
        .expect("insert secondary source");

    sqlite
        .execute(
            "INSERT INTO knowledge_items (
                knowledge_item_id, repo_id, knowledge_source_id, item_kind,
                latest_knowledge_item_version_id, provenance_json, created_at, updated_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            rusqlite::params![
                primary_item_id.as_str(),
                repo_id.as_str(),
                primary_source_id.as_str(),
                "jira_issue",
                primary_v2_id.as_str(),
                "{\"seed\":\"graphql-tests\"}",
                "2026-03-25T09:00:00Z",
                "2026-03-26T09:30:00Z",
            ],
        )
        .expect("insert primary item");
    sqlite
        .execute(
            "INSERT INTO knowledge_items (
                knowledge_item_id, repo_id, knowledge_source_id, item_kind,
                latest_knowledge_item_version_id, provenance_json, created_at, updated_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            rusqlite::params![
                secondary_item_id.as_str(),
                repo_id.as_str(),
                secondary_source_id.as_str(),
                "github_issue",
                secondary_v1_id.as_str(),
                "{\"seed\":\"graphql-tests\"}",
                "2026-03-25T08:00:00Z",
                "2026-03-26T08:30:00Z",
            ],
        )
        .expect("insert secondary item");

    let primary_v1_path = crate::capability_packs::knowledge::storage::knowledge_payload_key(
        repo_id.as_str(),
        &primary_item_id,
        &primary_v1_id,
    );
    let primary_v2_path = crate::capability_packs::knowledge::storage::knowledge_payload_key(
        repo_id.as_str(),
        &primary_item_id,
        &primary_v2_id,
    );
    let secondary_v1_path = crate::capability_packs::knowledge::storage::knowledge_payload_key(
        repo_id.as_str(),
        &secondary_item_id,
        &secondary_v1_id,
    );

    duckdb
        .execute(
            "INSERT INTO knowledge_document_versions (
                knowledge_item_version_id, knowledge_item_id, provider, source_kind, content_hash,
                title, state, author, updated_at, body_preview, normalized_fields_json,
                storage_backend, storage_path, payload_mime_type, payload_size_bytes,
                provenance_json, created_at
             ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            duckdb::params![
                primary_v1_id.as_str(),
                primary_item_id.as_str(),
                "jira",
                "jira_issue",
                primary_v1_hash.as_str(),
                "CLI-1521 draft design",
                "open",
                "Vasilis Danias",
                "2026-03-25T09:00:00Z",
                "Initial GraphQL knowledge design.",
                "{\"summary\":\"draft\"}",
                "local",
                primary_v1_path.as_str(),
                "application/json",
                primary_v1_bytes.len() as i64,
                "{\"seed\":\"graphql-tests\"}",
                "2026-03-25 09:00:00",
            ],
        )
        .expect("insert primary v1");
    duckdb
        .execute(
            "INSERT INTO knowledge_document_versions (
                knowledge_item_version_id, knowledge_item_id, provider, source_kind, content_hash,
                title, state, author, updated_at, body_preview, normalized_fields_json,
                storage_backend, storage_path, payload_mime_type, payload_size_bytes,
                provenance_json, created_at
             ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            duckdb::params![
                primary_v2_id.as_str(),
                primary_item_id.as_str(),
                "jira",
                "jira_issue",
                primary_v2_hash.as_str(),
                "Implement knowledge queries and payload loading",
                "in_progress",
                "Vasilis Danias",
                "2026-03-26T09:30:00Z",
                "Deliver the typed GraphQL knowledge model and lazy payload reads.",
                "{\"summary\":\"latest\"}",
                "local",
                primary_v2_path.as_str(),
                "application/json",
                primary_v2_bytes.len() as i64,
                "{\"seed\":\"graphql-tests\"}",
                "2026-03-26 09:30:00",
            ],
        )
        .expect("insert primary v2");
    duckdb
        .execute(
            "INSERT INTO knowledge_document_versions (
                knowledge_item_version_id, knowledge_item_id, provider, source_kind, content_hash,
                title, state, author, updated_at, body_preview, normalized_fields_json,
                storage_backend, storage_path, payload_mime_type, payload_size_bytes,
                provenance_json, created_at
             ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            duckdb::params![
                secondary_v1_id.as_str(),
                secondary_item_id.as_str(),
                "github",
                "github_issue",
                secondary_hash.as_str(),
                "Secondary GraphQL knowledge item",
                "open",
                "Alice",
                "2026-03-26T08:30:00Z",
                "Secondary knowledge item used for relation traversal tests.",
                "{\"summary\":\"secondary\"}",
                "local",
                secondary_v1_path.as_str(),
                "application/json",
                secondary_bytes.len() as i64,
                "{\"seed\":\"graphql-tests\"}",
                "2026-03-26 08:30:00",
            ],
        )
        .expect("insert secondary v1");

    sqlite
        .execute(
            "INSERT INTO knowledge_relation_assertions (
                relation_assertion_id, repo_id, knowledge_item_id, source_knowledge_item_version_id,
                target_type, target_id, target_knowledge_item_version_id, relation_type,
                association_method, confidence, provenance_json, created_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            rusqlite::params![
                crate::capability_packs::knowledge::storage::relation_assertion_id(
                    &primary_item_id,
                    &primary_v2_id,
                    "knowledge_item",
                    &secondary_item_id,
                    Some(&secondary_v1_id),
                    "manual_attachment",
                ),
                repo_id.as_str(),
                primary_item_id.as_str(),
                primary_v2_id.as_str(),
                "knowledge_item",
                secondary_item_id.as_str(),
                secondary_v1_id.as_str(),
                "associated_with",
                "manual_attachment",
                0.9_f64,
                "{\"source\":\"graphql-tests\"}",
                "2026-03-26T09:31:00Z",
            ],
        )
        .expect("insert knowledge relation");

    let blob_root = repo_local_blob_root(repo_root);
    for (storage_path, bytes) in [
        (primary_v1_path.as_str(), primary_v1_bytes.as_slice()),
        (primary_v2_path.as_str(), primary_v2_bytes.as_slice()),
    ] {
        let path = blob_root.join(storage_path);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create knowledge blob parent");
        }
        fs::write(path, bytes).expect("write knowledge blob");
    }

    SeededKnowledgeFixture {
        primary_item_id,
        primary_latest_version_id: primary_v2_id,
        secondary_item_id,
        secondary_latest_version_id: secondary_v1_id,
    }
}

fn seed_graphql_monorepo_repo() -> TempDir {
    let dir = TempDir::new().expect("temp dir");
    let repo_root = dir.path();

    init_test_repo(repo_root, "main", "Alice", "alice@example.com");
    fs::create_dir_all(repo_root.join("packages/api/src")).expect("create api src dir");
    fs::create_dir_all(repo_root.join("packages/web/src")).expect("create web src dir");
    fs::write(
        repo_root.join("packages/api/src/caller.ts"),
        "import { target } from \"./target\";\nimport { render } from \"../../web/src/page\";\n\nexport function caller() {\n  return target() + render();\n}\n",
    )
    .expect("write api caller.ts");
    fs::write(
        repo_root.join("packages/api/src/target.ts"),
        "export function target() {\n  return 41;\n}\n",
    )
    .expect("write api target.ts");
    fs::write(
        repo_root.join("packages/web/src/page.ts"),
        "export function render() {\n  return 1;\n}\n",
    )
    .expect("write web page.ts");
    git_ok(repo_root, &["add", "."]);
    git_ok(repo_root, &["commit", "-m", "Seed GraphQL monorepo repo"]);
    let commit_sha = git_ok(repo_root, &["rev-parse", "HEAD"]);

    let sqlite_path = repo_root
        .join(".bitloops")
        .join("stores")
        .join("graphql-monorepo.sqlite");
    crate::storage::init::init_database(&sqlite_path, false, &commit_sha)
        .expect("initialise GraphQL monorepo sqlite store");
    write_envelope_config(
        repo_root,
        json!({
            "stores": {
                "relational": {
                    "sqlite_path": sqlite_path.to_string_lossy()
                }
            }
        }),
    );

    let repo_id = crate::host::devql::resolve_repo_id(repo_root).expect("resolve repo id");
    let conn = rusqlite::Connection::open(&sqlite_path).expect("open GraphQL monorepo sqlite");
    conn.execute(
        "INSERT INTO repositories (repo_id, provider, organization, name, default_branch)
         VALUES (?1, 'local', 'local', 'graphql-monorepo', 'main')",
        rusqlite::params![repo_id.as_str()],
    )
    .expect("insert repository row");
    conn.execute(
        "INSERT INTO commits (commit_sha, repo_id, author_name, author_email, commit_message, committed_at)
         VALUES (?1, ?2, 'Alice', 'alice@example.com', 'Seed GraphQL monorepo repo', '2026-03-26T10:00:00Z')",
        rusqlite::params![commit_sha.as_str(), repo_id.as_str()],
    )
    .expect("insert commit row");

    for (path, blob_sha) in [
        ("packages/api/src/caller.ts", "blob-api-caller"),
        ("packages/api/src/target.ts", "blob-api-target"),
        ("packages/web/src/page.ts", "blob-web-page"),
    ] {
        conn.execute(
            "INSERT INTO file_state (repo_id, commit_sha, path, blob_sha)
             VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![repo_id.as_str(), commit_sha.as_str(), path, blob_sha],
        )
        .expect("insert file_state row");
        conn.execute(
            "INSERT INTO current_file_state (repo_id, path, commit_sha, blob_sha, committed_at)
             VALUES (?1, ?2, ?3, ?4, '2026-03-26T10:00:00Z')",
            rusqlite::params![repo_id.as_str(), path, commit_sha.as_str(), blob_sha],
        )
        .expect("insert current_file_state row");
    }

    let artefacts = [
        (
            "file::api-caller",
            "artefact::file-api-caller",
            "blob-api-caller",
            "packages/api/src/caller.ts",
            "file",
            "source_file",
            "packages/api/src/caller.ts",
            Option::<&str>::None,
            1_i64,
            6_i64,
        ),
        (
            "sym::api-caller",
            "artefact::api-caller",
            "blob-api-caller",
            "packages/api/src/caller.ts",
            "function",
            "function_declaration",
            "packages/api/src/caller.ts::caller",
            Some("artefact::file-api-caller"),
            4_i64,
            6_i64,
        ),
        (
            "file::api-target",
            "artefact::file-api-target",
            "blob-api-target",
            "packages/api/src/target.ts",
            "file",
            "source_file",
            "packages/api/src/target.ts",
            Option::<&str>::None,
            1_i64,
            3_i64,
        ),
        (
            "sym::api-target",
            "artefact::api-target",
            "blob-api-target",
            "packages/api/src/target.ts",
            "function",
            "function_declaration",
            "packages/api/src/target.ts::target",
            Some("artefact::file-api-target"),
            1_i64,
            3_i64,
        ),
        (
            "file::web-page",
            "artefact::file-web-page",
            "blob-web-page",
            "packages/web/src/page.ts",
            "file",
            "source_file",
            "packages/web/src/page.ts",
            Option::<&str>::None,
            1_i64,
            3_i64,
        ),
        (
            "sym::web-render",
            "artefact::web-render",
            "blob-web-page",
            "packages/web/src/page.ts",
            "function",
            "function_declaration",
            "packages/web/src/page.ts::render",
            Some("artefact::file-web-page"),
            1_i64,
            3_i64,
        ),
    ];

    for (
        symbol_id,
        artefact_id,
        blob_sha,
        path,
        canonical_kind,
        language_kind,
        symbol_fqn,
        parent_artefact_id,
        start_line,
        end_line,
    ) in artefacts
    {
        let parent_symbol_id = match parent_artefact_id {
            Some("artefact::file-api-caller") => Some("file::api-caller"),
            Some("artefact::file-api-target") => Some("file::api-target"),
            Some("artefact::file-web-page") => Some("file::web-page"),
            _ => None,
        };
        conn.execute(
            "INSERT INTO artefacts (
                artefact_id, symbol_id, repo_id, blob_sha, path, language, canonical_kind,
                language_kind, symbol_fqn, parent_artefact_id, start_line, end_line,
                start_byte, end_byte, signature, modifiers, docstring, content_hash, created_at
            ) VALUES (
                ?1, ?2, ?3, ?4, ?5, 'typescript', ?6,
                ?7, ?8, ?9, ?10, ?11, 0, ?12, NULL, ?13, ?14, ?15, '2026-03-26T10:00:00Z'
            )",
            rusqlite::params![
                artefact_id,
                symbol_id,
                repo_id.as_str(),
                blob_sha,
                path,
                canonical_kind,
                language_kind,
                symbol_fqn,
                parent_artefact_id,
                start_line,
                end_line,
                end_line * 10,
                if canonical_kind == "file" {
                    "[]"
                } else {
                    "[\"export\"]"
                },
                if canonical_kind == "file" {
                    Option::<&str>::None
                } else {
                    Some("Monorepo docstring")
                },
                format!("hash-{artefact_id}"),
            ],
        )
        .expect("insert artefact row");
        conn.execute(
            "INSERT INTO artefacts_current (
                repo_id, branch, symbol_id, artefact_id, commit_sha, revision_kind, revision_id,
                temp_checkpoint_id, blob_sha, path, language, canonical_kind, language_kind,
                symbol_fqn, parent_symbol_id, parent_artefact_id, start_line, end_line,
                start_byte, end_byte, signature, modifiers, docstring, content_hash, updated_at
            ) VALUES (
                ?1, 'main', ?2, ?3, ?4, 'commit', ?4,
                NULL, ?5, ?6, 'typescript', ?7, ?8,
                ?9, ?10, ?11, ?12, ?13,
                0, ?14, NULL, ?15, ?16, ?17, '2026-03-26T10:00:00Z'
            )",
            rusqlite::params![
                repo_id.as_str(),
                symbol_id,
                artefact_id,
                commit_sha.as_str(),
                blob_sha,
                path,
                canonical_kind,
                language_kind,
                symbol_fqn,
                parent_symbol_id,
                parent_artefact_id,
                start_line,
                end_line,
                end_line * 10,
                if canonical_kind == "file" {
                    "[]"
                } else {
                    "[\"export\"]"
                },
                if canonical_kind == "file" {
                    Option::<&str>::None
                } else {
                    Some("Monorepo docstring")
                },
                format!("hash-{artefact_id}"),
            ],
        )
        .expect("insert artefact current row");
    }

    for (
        edge_id,
        from_symbol_id,
        from_artefact_id,
        to_symbol_id,
        to_artefact_id,
        to_symbol_ref,
        line,
    ) in [
        (
            "edge-api-local",
            "sym::api-caller",
            "artefact::api-caller",
            Some("sym::api-target"),
            Some("artefact::api-target"),
            Some("packages/api/src/target.ts::target"),
            5_i64,
        ),
        (
            "edge-api-cross",
            "sym::api-caller",
            "artefact::api-caller",
            Some("sym::web-render"),
            Some("artefact::web-render"),
            Some("packages/web/src/page.ts::render"),
            5_i64,
        ),
    ] {
        conn.execute(
            "INSERT INTO artefact_edges_current (
                edge_id, repo_id, branch, commit_sha, revision_kind, revision_id,
                temp_checkpoint_id, blob_sha, path, from_symbol_id, from_artefact_id,
                to_symbol_id, to_artefact_id, to_symbol_ref, edge_kind, language,
                start_line, end_line, metadata, updated_at
            ) VALUES (
                ?1, ?2, 'main', ?3, 'commit', ?3,
                NULL, 'blob-api-caller', 'packages/api/src/caller.ts', ?4, ?5,
                ?6, ?7, ?8, 'calls', 'typescript',
                ?9, ?9, '{\"resolution\":\"local\"}', '2026-03-26T10:00:00Z'
            )",
            rusqlite::params![
                edge_id,
                repo_id.as_str(),
                commit_sha.as_str(),
                from_symbol_id,
                from_artefact_id,
                to_symbol_id,
                to_artefact_id,
                to_symbol_ref,
                line,
            ],
        )
        .expect("insert edge current row");
    }

    dir
}

fn seed_graphql_clone_data(repo_root: &Path) {
    let sqlite_path = checkpoint_sqlite_path(repo_root);
    let repo_id = crate::host::devql::resolve_repo_id(repo_root).expect("resolve repo id");
    let conn = rusqlite::Connection::open(&sqlite_path).expect("open clone sqlite");

    conn.execute_batch(
        crate::capability_packs::semantic_clones::schema::semantic_clones_sqlite_schema_sql(),
    )
    .expect("initialise clone sqlite schema");

    for (
        source_symbol_id,
        source_artefact_id,
        target_symbol_id,
        target_artefact_id,
        relation_kind,
        score,
        semantic_score,
        lexical_score,
        structural_score,
        clone_input_hash,
        explanation_json,
    ) in [
        (
            "sym::api-caller",
            "artefact::api-caller",
            "sym::api-target",
            "artefact::api-target",
            "similar_implementation",
            0.93_f64,
            0.91_f64,
            0.84_f64,
            0.72_f64,
            "clone-hash-1",
            r#"{"reason":"shared invoice assembly"}"#,
        ),
        (
            "sym::api-caller",
            "artefact::api-caller",
            "sym::web-render",
            "artefact::web-render",
            "similar_implementation",
            0.71_f64,
            0.68_f64,
            0.64_f64,
            0.58_f64,
            "clone-hash-2",
            r#"{"reason":"shared rendering pattern"}"#,
        ),
        (
            "sym::web-render",
            "artefact::web-render",
            "sym::api-target",
            "artefact::api-target",
            "contextual_neighbor",
            0.68_f64,
            0.66_f64,
            0.52_f64,
            0.61_f64,
            "clone-hash-3",
            r#"{"reason":"cross-package helper overlap"}"#,
        ),
    ] {
        conn.execute(
            "INSERT INTO symbol_clone_edges (
                repo_id, source_symbol_id, source_artefact_id, target_symbol_id, target_artefact_id,
                relation_kind, score, semantic_score, lexical_score, structural_score,
                clone_input_hash, explanation_json
            ) VALUES (
                ?1, ?2, ?3, ?4, ?5,
                ?6, ?7, ?8, ?9, ?10,
                ?11, ?12
            )",
            rusqlite::params![
                repo_id.as_str(),
                source_symbol_id,
                source_artefact_id,
                target_symbol_id,
                target_artefact_id,
                relation_kind,
                score,
                semantic_score,
                lexical_score,
                structural_score,
                clone_input_hash,
                explanation_json,
            ],
        )
        .expect("insert clone edge");
    }
}

fn seed_graphql_test_harness_stage_data(
    repo_root: &Path,
    commit_sha: &str,
    rows: &[(&str, &str, &str, &str)],
) {
    use crate::capability_packs::test_harness::storage::{
        TestHarnessRepository, open_repository_for_repo,
    };
    use crate::models::{
        CoverageCaptureRecord, CoverageFormat, CoverageHitRecord, ScopeKind,
        TestArtefactCurrentRecord, TestArtefactEdgeCurrentRecord, TestDiscoveryRunRecord,
    };

    let repo_id = crate::host::devql::resolve_repo_id(repo_root).expect("resolve repo id");
    let mut repository = open_repository_for_repo(repo_root).expect("open test harness repository");
    let discovery_run = TestDiscoveryRunRecord {
        discovery_run_id: format!("discovery:{commit_sha}"),
        repo_id: repo_id.clone(),
        commit_sha: commit_sha.to_string(),
        language: Some("typescript".to_string()),
        started_at: "2026-03-26T11:00:00Z".to_string(),
        finished_at: Some("2026-03-26T11:00:01Z".to_string()),
        status: "complete".to_string(),
        enumeration_status: Some("complete".to_string()),
        notes_json: None,
        stats_json: None,
    };

    let mut test_artefacts = Vec::<TestArtefactCurrentRecord>::new();
    let mut test_edges = Vec::<TestArtefactEdgeCurrentRecord>::new();
    let mut coverage_captures = Vec::<CoverageCaptureRecord>::new();
    let mut coverage_hits = Vec::<CoverageHitRecord>::new();

    for (index, (production_symbol_id, production_artefact_id, production_path, test_name)) in
        rows.iter().enumerate()
    {
        let suite_symbol_id = format!("test-suite-symbol-{index}");
        let suite_artefact_id = format!("test-suite-artefact-{index}");
        let test_symbol_id = format!("test-scenario-symbol-{index}");
        let test_artefact_id = format!("test-scenario-artefact-{index}");
        let test_path = format!("tests/generated_{index}.ts");

        test_artefacts.push(TestArtefactCurrentRecord {
            artefact_id: suite_artefact_id.clone(),
            symbol_id: suite_symbol_id.clone(),
            repo_id: repo_id.clone(),
            commit_sha: commit_sha.to_string(),
            blob_sha: format!("test-blob-suite-{index}"),
            path: test_path.clone(),
            language: "typescript".to_string(),
            canonical_kind: "test_suite".to_string(),
            language_kind: Some("describe".to_string()),
            symbol_fqn: Some(format!("tests::suite::{index}")),
            name: format!("suite_{index}"),
            parent_artefact_id: None,
            parent_symbol_id: None,
            start_line: 1,
            end_line: 20,
            start_byte: Some(0),
            end_byte: Some(200),
            signature: None,
            modifiers: "[]".to_string(),
            docstring: None,
            content_hash: None,
            discovery_source: "static".to_string(),
            revision_kind: "commit".to_string(),
            revision_id: commit_sha.to_string(),
        });

        test_artefacts.push(TestArtefactCurrentRecord {
            artefact_id: test_artefact_id.clone(),
            symbol_id: test_symbol_id.clone(),
            repo_id: repo_id.clone(),
            commit_sha: commit_sha.to_string(),
            blob_sha: format!("test-blob-scenario-{index}"),
            path: test_path.clone(),
            language: "typescript".to_string(),
            canonical_kind: "test_scenario".to_string(),
            language_kind: Some("it".to_string()),
            symbol_fqn: Some(format!("tests::scenario::{index}")),
            name: (*test_name).to_string(),
            parent_artefact_id: Some(suite_artefact_id.clone()),
            parent_symbol_id: Some(suite_symbol_id.clone()),
            start_line: 2,
            end_line: 10,
            start_byte: Some(10),
            end_byte: Some(100),
            signature: None,
            modifiers: "[]".to_string(),
            docstring: None,
            content_hash: None,
            discovery_source: "static".to_string(),
            revision_kind: "commit".to_string(),
            revision_id: commit_sha.to_string(),
        });

        test_edges.push(TestArtefactEdgeCurrentRecord {
            edge_id: format!("test-edge-{index}"),
            repo_id: repo_id.clone(),
            commit_sha: commit_sha.to_string(),
            blob_sha: format!("test-blob-edge-{index}"),
            path: test_path,
            from_artefact_id: test_artefact_id.clone(),
            from_symbol_id: test_symbol_id.clone(),
            to_artefact_id: Some((*production_artefact_id).to_string()),
            to_symbol_id: Some((*production_symbol_id).to_string()),
            to_symbol_ref: None,
            edge_kind: "covers".to_string(),
            language: "typescript".to_string(),
            start_line: Some(3),
            end_line: Some(8),
            metadata: json!({
                "confidence": 0.91,
                "link_source": "static_analysis",
                "linkage_status": "linked"
            })
            .to_string(),
            revision_kind: "commit".to_string(),
            revision_id: commit_sha.to_string(),
        });

        coverage_captures.push(CoverageCaptureRecord {
            capture_id: format!("capture-{index}"),
            repo_id: repo_id.clone(),
            commit_sha: commit_sha.to_string(),
            tool: "lcov".to_string(),
            format: CoverageFormat::Lcov,
            scope_kind: ScopeKind::TestScenario,
            subject_test_symbol_id: Some(test_symbol_id),
            line_truth: true,
            branch_truth: true,
            captured_at: format!("2026-03-26T11:00:0{}Z", index + 2),
            status: "complete".to_string(),
            metadata_json: None,
        });

        coverage_hits.push(CoverageHitRecord {
            capture_id: format!("capture-{index}"),
            production_symbol_id: (*production_symbol_id).to_string(),
            file_path: (*production_path).to_string(),
            line: 4,
            branch_id: -1,
            covered: true,
            hit_count: 2,
        });
        coverage_hits.push(CoverageHitRecord {
            capture_id: format!("capture-{index}"),
            production_symbol_id: (*production_symbol_id).to_string(),
            file_path: (*production_path).to_string(),
            line: 5,
            branch_id: -1,
            covered: false,
            hit_count: 0,
        });
        coverage_hits.push(CoverageHitRecord {
            capture_id: format!("capture-{index}"),
            production_symbol_id: (*production_symbol_id).to_string(),
            file_path: (*production_path).to_string(),
            line: 4,
            branch_id: 0,
            covered: true,
            hit_count: 2,
        });
    }

    repository
        .replace_test_discovery(
            commit_sha,
            &test_artefacts,
            &test_edges,
            &discovery_run,
            &[],
        )
        .expect("replace test discovery");
    for capture in &coverage_captures {
        repository
            .insert_coverage_capture(capture)
            .expect("insert coverage capture");
    }
    repository
        .insert_coverage_hits(&coverage_hits)
        .expect("insert coverage hits");
    repository
        .rebuild_classifications_from_coverage(commit_sha)
        .expect("rebuild classifications from coverage");
}

fn seed_graphql_monorepo_repo_with_duckdb_events() -> TempDir {
    let repo = seed_graphql_monorepo_repo();
    let commit_sha = git_ok(repo.path(), &["rev-parse", "HEAD"]);

    seed_duckdb_events(
        repo.path(),
        &[
            SeedGraphqlEvent {
                event_id: "evt-checkpoint-api",
                event_time: "2026-03-26T10:20:00Z",
                checkpoint_id: "checkpoint-api",
                session_id: "session-api",
                commit_sha: &commit_sha,
                branch: "main",
                event_type: "checkpoint_committed",
                agent: "codex",
                strategy: "manual-commit",
                files_touched: &["packages/api/src/caller.ts", "packages/api/src/target.ts"],
                payload: json!({"scope": "api"}),
            },
            SeedGraphqlEvent {
                event_id: "evt-checkpoint-web",
                event_time: "2026-03-26T10:25:00Z",
                checkpoint_id: "checkpoint-web",
                session_id: "session-web",
                commit_sha: &commit_sha,
                branch: "main",
                event_type: "checkpoint_committed",
                agent: "codex",
                strategy: "manual-commit",
                files_touched: &["packages/web/src/page.ts"],
                payload: json!({"scope": "web"}),
            },
            SeedGraphqlEvent {
                event_id: "evt-telemetry-tool",
                event_time: "2026-03-26T10:30:00Z",
                checkpoint_id: "",
                session_id: "session-api",
                commit_sha: &commit_sha,
                branch: "main",
                event_type: "tool_invocation",
                agent: "codex",
                strategy: "",
                files_touched: &["packages/api/src/caller.ts"],
                payload: json!({"tool": "Edit", "path": "packages/api/src/caller.ts"}),
            },
        ],
    );

    repo
}

struct SeededGraphqlTemporalRepo {
    repo: TempDir,
    first_commit: String,
}

fn seed_graphql_temporal_repo() -> SeededGraphqlTemporalRepo {
    let dir = TempDir::new().expect("temp dir");
    let repo_root = dir.path();

    init_test_repo(repo_root, "main", "Alice", "alice@example.com");
    fs::create_dir_all(repo_root.join("packages/api/src")).expect("create api src dir");
    fs::create_dir_all(repo_root.join("packages/web/src")).expect("create web src dir");
    fs::write(
        repo_root.join("packages/api/src/caller.ts"),
        "import { target } from \"./target\";\n\nexport function caller() {\n  return target();\n}\n",
    )
    .expect("write api caller v1");
    fs::write(
        repo_root.join("packages/api/src/target.ts"),
        "export function target() {\n  return 41;\n}\n",
    )
    .expect("write api target v1");
    fs::write(
        repo_root.join("packages/web/src/page.ts"),
        "export function render() {\n  return 1;\n}\n",
    )
    .expect("write web page");
    git_ok(repo_root, &["add", "."]);
    git_ok(
        repo_root,
        &["commit", "-m", "Seed temporal GraphQL commit 1"],
    );
    let first_commit = git_ok(repo_root, &["rev-parse", "HEAD"]);

    fs::write(
        repo_root.join("packages/api/src/caller.ts"),
        "import { render } from \"../../web/src/page\";\n\nexport function callerCurrent() {\n  return render();\n}\n",
    )
    .expect("write api caller v2");
    fs::remove_file(repo_root.join("packages/api/src/target.ts")).expect("remove api target");
    git_ok(repo_root, &["add", "-A"]);
    git_ok(
        repo_root,
        &["commit", "-m", "Seed temporal GraphQL commit 2"],
    );
    let second_commit = git_ok(repo_root, &["rev-parse", "HEAD"]);

    let sqlite_path = repo_root
        .join(".bitloops")
        .join("stores")
        .join("graphql-temporal.sqlite");
    crate::storage::init::init_database(&sqlite_path, false, &second_commit)
        .expect("initialise GraphQL temporal sqlite store");
    write_envelope_config(
        repo_root,
        json!({
            "stores": {
                "relational": {
                    "sqlite_path": sqlite_path.to_string_lossy()
                }
            }
        }),
    );

    let repo_id = crate::host::devql::resolve_repo_id(repo_root).expect("resolve repo id");
    let conn = rusqlite::Connection::open(&sqlite_path).expect("open GraphQL temporal sqlite");
    conn.execute(
        "INSERT INTO repositories (repo_id, provider, organization, name, default_branch)
         VALUES (?1, 'local', 'local', 'graphql-temporal', 'main')",
        rusqlite::params![repo_id.as_str()],
    )
    .expect("insert repository row");
    conn.execute(
        "INSERT INTO commits (commit_sha, repo_id, author_name, author_email, commit_message, committed_at)
         VALUES (?1, ?2, 'Alice', 'alice@example.com', 'Seed temporal GraphQL commit 1', '2026-03-25T09:00:00Z')",
        rusqlite::params![first_commit.as_str(), repo_id.as_str()],
    )
    .expect("insert first commit row");
    conn.execute(
        "INSERT INTO commits (commit_sha, repo_id, author_name, author_email, commit_message, committed_at)
         VALUES (?1, ?2, 'Alice', 'alice@example.com', 'Seed temporal GraphQL commit 2', '2026-03-26T09:00:00Z')",
        rusqlite::params![second_commit.as_str(), repo_id.as_str()],
    )
    .expect("insert second commit row");

    for (commit_sha, path, blob_sha) in [
        (
            first_commit.as_str(),
            "packages/api/src/caller.ts",
            "blob-api-caller-v1",
        ),
        (
            first_commit.as_str(),
            "packages/api/src/target.ts",
            "blob-api-target-v1",
        ),
        (
            first_commit.as_str(),
            "packages/web/src/page.ts",
            "blob-web-page-v1",
        ),
        (
            second_commit.as_str(),
            "packages/api/src/caller.ts",
            "blob-api-caller-v2",
        ),
        (
            second_commit.as_str(),
            "packages/web/src/page.ts",
            "blob-web-page-v1",
        ),
    ] {
        conn.execute(
            "INSERT INTO file_state (repo_id, commit_sha, path, blob_sha)
             VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![repo_id.as_str(), commit_sha, path, blob_sha],
        )
        .expect("insert file_state row");
    }

    for (path, blob_sha) in [
        ("packages/api/src/caller.ts", "blob-api-caller-v2"),
        ("packages/web/src/page.ts", "blob-web-page-v1"),
    ] {
        conn.execute(
            "INSERT INTO current_file_state (repo_id, path, commit_sha, blob_sha, committed_at)
             VALUES (?1, ?2, ?3, ?4, '2026-03-26T09:00:00Z')",
            rusqlite::params![repo_id.as_str(), path, second_commit.as_str(), blob_sha],
        )
        .expect("insert current_file_state row");
    }

    for (
        symbol_id,
        artefact_id,
        blob_sha,
        path,
        canonical_kind,
        language_kind,
        symbol_fqn,
        parent_artefact_id,
        start_line,
        end_line,
        created_at,
    ) in [
        (
            "file::v1-api-caller",
            "artefact::v1-file-api-caller",
            "blob-api-caller-v1",
            "packages/api/src/caller.ts",
            "file",
            "source_file",
            "packages/api/src/caller.ts",
            Option::<&str>::None,
            1_i64,
            5_i64,
            "2026-03-25T09:00:00Z",
        ),
        (
            "sym::v1-api-caller",
            "artefact::v1-api-caller",
            "blob-api-caller-v1",
            "packages/api/src/caller.ts",
            "function",
            "function_declaration",
            "packages/api/src/caller.ts::caller",
            Some("artefact::v1-file-api-caller"),
            3_i64,
            5_i64,
            "2026-03-25T09:00:00Z",
        ),
        (
            "file::v1-api-target",
            "artefact::v1-file-api-target",
            "blob-api-target-v1",
            "packages/api/src/target.ts",
            "file",
            "source_file",
            "packages/api/src/target.ts",
            Option::<&str>::None,
            1_i64,
            3_i64,
            "2026-03-25T09:00:00Z",
        ),
        (
            "sym::v1-api-target",
            "artefact::v1-api-target",
            "blob-api-target-v1",
            "packages/api/src/target.ts",
            "function",
            "function_declaration",
            "packages/api/src/target.ts::target",
            Some("artefact::v1-file-api-target"),
            1_i64,
            3_i64,
            "2026-03-25T09:00:00Z",
        ),
        (
            "file::v1-web-page",
            "artefact::v1-file-web-page",
            "blob-web-page-v1",
            "packages/web/src/page.ts",
            "file",
            "source_file",
            "packages/web/src/page.ts",
            Option::<&str>::None,
            1_i64,
            3_i64,
            "2026-03-25T09:00:00Z",
        ),
        (
            "sym::v1-web-render",
            "artefact::v1-web-render",
            "blob-web-page-v1",
            "packages/web/src/page.ts",
            "function",
            "function_declaration",
            "packages/web/src/page.ts::render",
            Some("artefact::v1-file-web-page"),
            1_i64,
            3_i64,
            "2026-03-25T09:00:00Z",
        ),
        (
            "file::v2-api-caller",
            "artefact::v2-file-api-caller",
            "blob-api-caller-v2",
            "packages/api/src/caller.ts",
            "file",
            "source_file",
            "packages/api/src/caller.ts",
            Option::<&str>::None,
            1_i64,
            5_i64,
            "2026-03-26T09:00:00Z",
        ),
        (
            "sym::v2-api-caller",
            "artefact::v2-api-caller",
            "blob-api-caller-v2",
            "packages/api/src/caller.ts",
            "function",
            "function_declaration",
            "packages/api/src/caller.ts::callerCurrent",
            Some("artefact::v2-file-api-caller"),
            3_i64,
            5_i64,
            "2026-03-26T09:00:00Z",
        ),
        (
            "file::v2-web-page",
            "artefact::v2-file-web-page",
            "blob-web-page-v1",
            "packages/web/src/page.ts",
            "file",
            "source_file",
            "packages/web/src/page.ts",
            Option::<&str>::None,
            1_i64,
            3_i64,
            "2026-03-26T09:00:00Z",
        ),
        (
            "sym::v2-web-render",
            "artefact::v2-web-render",
            "blob-web-page-v1",
            "packages/web/src/page.ts",
            "function",
            "function_declaration",
            "packages/web/src/page.ts::render",
            Some("artefact::v2-file-web-page"),
            1_i64,
            3_i64,
            "2026-03-26T09:00:00Z",
        ),
    ] {
        conn.execute(
            "INSERT INTO artefacts (
                artefact_id, symbol_id, repo_id, blob_sha, path, language, canonical_kind,
                language_kind, symbol_fqn, parent_artefact_id, start_line, end_line,
                start_byte, end_byte, signature, modifiers, docstring, content_hash, created_at
            ) VALUES (
                ?1, ?2, ?3, ?4, ?5, 'typescript', ?6,
                ?7, ?8, ?9, ?10, ?11, 0, ?12, NULL, ?13, ?14, ?15, ?16
            )",
            rusqlite::params![
                artefact_id,
                symbol_id,
                repo_id.as_str(),
                blob_sha,
                path,
                canonical_kind,
                language_kind,
                symbol_fqn,
                parent_artefact_id,
                start_line,
                end_line,
                end_line * 10,
                if canonical_kind == "file" {
                    "[]"
                } else {
                    "[\"export\"]"
                },
                if canonical_kind == "file" {
                    Option::<&str>::None
                } else {
                    Some("Temporal docstring")
                },
                format!("hash-{artefact_id}"),
                created_at,
            ],
        )
        .expect("insert historical artefact row");
    }

    for (
        symbol_id,
        artefact_id,
        blob_sha,
        path,
        canonical_kind,
        language_kind,
        symbol_fqn,
        parent_symbol_id,
        parent_artefact_id,
        start_line,
        end_line,
    ) in [
        (
            "file::v2-api-caller",
            "artefact::v2-file-api-caller",
            "blob-api-caller-v2",
            "packages/api/src/caller.ts",
            "file",
            "source_file",
            "packages/api/src/caller.ts",
            Option::<&str>::None,
            Option::<&str>::None,
            1_i64,
            5_i64,
        ),
        (
            "sym::v2-api-caller",
            "artefact::v2-api-caller",
            "blob-api-caller-v2",
            "packages/api/src/caller.ts",
            "function",
            "function_declaration",
            "packages/api/src/caller.ts::callerCurrent",
            Some("file::v2-api-caller"),
            Some("artefact::v2-file-api-caller"),
            3_i64,
            5_i64,
        ),
        (
            "file::v2-web-page",
            "artefact::v2-file-web-page",
            "blob-web-page-v1",
            "packages/web/src/page.ts",
            "file",
            "source_file",
            "packages/web/src/page.ts",
            Option::<&str>::None,
            Option::<&str>::None,
            1_i64,
            3_i64,
        ),
        (
            "sym::v2-web-render",
            "artefact::v2-web-render",
            "blob-web-page-v1",
            "packages/web/src/page.ts",
            "function",
            "function_declaration",
            "packages/web/src/page.ts::render",
            Some("file::v2-web-page"),
            Some("artefact::v2-file-web-page"),
            1_i64,
            3_i64,
        ),
    ] {
        conn.execute(
            "INSERT INTO artefacts_current (
                repo_id, branch, symbol_id, artefact_id, commit_sha, revision_kind, revision_id,
                temp_checkpoint_id, blob_sha, path, language, canonical_kind, language_kind,
                symbol_fqn, parent_symbol_id, parent_artefact_id, start_line, end_line,
                start_byte, end_byte, signature, modifiers, docstring, content_hash, updated_at
            ) VALUES (
                ?1, 'main', ?2, ?3, ?4, 'commit', ?4,
                NULL, ?5, ?6, 'typescript', ?7, ?8,
                ?9, ?10, ?11, ?12, ?13,
                0, ?14, NULL, ?15, ?16, ?17, '2026-03-26T09:00:00Z'
            )",
            rusqlite::params![
                repo_id.as_str(),
                symbol_id,
                artefact_id,
                second_commit.as_str(),
                blob_sha,
                path,
                canonical_kind,
                language_kind,
                symbol_fqn,
                parent_symbol_id,
                parent_artefact_id,
                start_line,
                end_line,
                end_line * 10,
                if canonical_kind == "file" {
                    "[]"
                } else {
                    "[\"export\"]"
                },
                if canonical_kind == "file" {
                    Option::<&str>::None
                } else {
                    Some("Current temporal docstring")
                },
                format!("hash-{artefact_id}"),
            ],
        )
        .expect("insert current artefact row");
    }

    conn.execute(
        "INSERT INTO artefact_edges (
            edge_id, repo_id, blob_sha, from_artefact_id, to_artefact_id, to_symbol_ref,
            edge_kind, language, start_line, end_line, metadata, created_at
        ) VALUES (
            'edge::v1-api-caller-target', ?1, 'blob-api-caller-v1', 'artefact::v1-api-caller',
            'artefact::v1-api-target', 'packages/api/src/target.ts::target',
            'calls', 'typescript', 4, 4, '{\"resolution\":\"local\"}', '2026-03-25T09:00:00Z'
        )",
        rusqlite::params![repo_id.as_str()],
    )
    .expect("insert historical edge row");

    conn.execute(
        "INSERT INTO artefact_edges_current (
            edge_id, repo_id, branch, commit_sha, revision_kind, revision_id,
            temp_checkpoint_id, blob_sha, path, from_symbol_id, from_artefact_id,
            to_symbol_id, to_artefact_id, to_symbol_ref, edge_kind, language,
            start_line, end_line, metadata, updated_at
        ) VALUES (
            'edge::v2-api-caller-web', ?1, 'main', ?2, 'commit', ?2,
            NULL, 'blob-api-caller-v2', 'packages/api/src/caller.ts', 'sym::v2-api-caller', 'artefact::v2-api-caller',
            'sym::v2-web-render', 'artefact::v2-web-render', 'packages/web/src/page.ts::render', 'calls', 'typescript',
            4, 4, '{\"resolution\":\"local\"}', '2026-03-26T09:00:00Z'
        )",
        rusqlite::params![repo_id.as_str(), second_commit.as_str()],
    )
    .expect("insert current edge row");

    SeededGraphqlTemporalRepo {
        repo: dir,
        first_commit,
    }
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
    hex::encode(hasher.finalize())
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
async fn devql_mutations_initialise_schema_and_ingest_with_typed_results() {
    let repo = seed_graphql_mutation_repo();
    let _guard = enter_process_state(Some(repo.path()), &[]);
    let sqlite_path = checkpoint_sqlite_path(repo.path());
    let schema = crate::graphql::build_schema(crate::graphql::DevqlGraphqlContext::new(
        repo.path().to_path_buf(),
        super::db::DashboardDbPools::default(),
    ));

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
              ingest(input: { init: true, maxCheckpoints: 500 }) {
                success
                initRequested
                checkpointsProcessed
                eventsInserted
                artefactsUpserted
                checkpointsWithoutCommit
                temporaryRowsPromoted
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
    assert_eq!(ingest_json["ingest"]["initRequested"], true);
    assert_eq!(ingest_json["ingest"]["checkpointsProcessed"], 0);
    assert_eq!(ingest_json["ingest"]["eventsInserted"], 0);
    assert_eq!(ingest_json["ingest"]["temporaryRowsPromoted"], 0);

    let repository_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM repositories", [], |row| row.get(0))
        .expect("count repositories");
    assert_eq!(repository_count, 1, "expected repository row after ingest");
}

#[tokio::test]
async fn devql_mutations_report_validation_and_backend_errors() {
    let repo = seed_graphql_mutation_repo();
    let _guard = enter_process_state(Some(repo.path()), &[]);
    let schema = crate::graphql::build_schema(crate::graphql::DevqlGraphqlContext::new(
        repo.path().to_path_buf(),
        super::db::DashboardDbPools::default(),
    ));

    let invalid_input = schema
        .execute(async_graphql::Request::new(
            r#"
            mutation {
              ingest(input: { init: true, maxCheckpoints: -1 }) {
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
              ingest(input: { init: false, maxCheckpoints: 1 }) {
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
    let repo = seed_graphql_knowledge_mutation_repo(server.url.as_str());
    let _guard = enter_process_state(Some(repo.path()), &[]);
    let sqlite_path = checkpoint_sqlite_path(repo.path());
    let duckdb_path = knowledge_duckdb_path(repo.path());
    let schema = crate::graphql::build_schema(crate::graphql::DevqlGraphqlContext::new(
        repo.path().to_path_buf(),
        super::db::DashboardDbPools::default(),
    ));

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
    let server = MockSequentialHttpServer::start(vec![MockHttpResponse::json(
        500,
        json!({ "errorMessages": ["provider boom"] }),
    )]);
    let repo = seed_graphql_knowledge_mutation_repo(server.url.as_str());
    let _guard = enter_process_state(Some(repo.path()), &[]);
    let schema = crate::graphql::build_schema(crate::graphql::DevqlGraphqlContext::new(
        repo.path().to_path_buf(),
        super::db::DashboardDbPools::default(),
    ));

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
async fn devql_repository_file_and_artefact_queries_resolve_current_devql_graph() {
    let repo = seed_graphql_devql_repo();
    let schema = crate::graphql::build_schema(crate::graphql::DevqlGraphqlContext::new(
        repo.path().to_path_buf(),
        super::db::DashboardDbPools::default(),
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
        super::db::DashboardDbPools::default(),
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
async fn devql_dependency_queries_resolve_direction_and_unresolved_targets() {
    let repo = seed_graphql_devql_repo();
    let schema = crate::graphql::build_schema(crate::graphql::DevqlGraphqlContext::new(
        repo.path().to_path_buf(),
        super::db::DashboardDbPools::default(),
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
async fn devql_project_queries_scope_paths_and_isolate_cross_project_resolution() {
    let repo = seed_graphql_monorepo_repo();
    let schema = crate::graphql::build_schema(crate::graphql::DevqlGraphqlContext::new(
        repo.path().to_path_buf(),
        super::db::DashboardDbPools::default(),
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
                        outgoingDeps {
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
                  deps(filter: { direction: OUT }, first: 10) {
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
        json["repo"]["api"]["artefacts"]["edges"][0]["node"]["outgoingDeps"]["totalCount"],
        2
    );
    assert_eq!(
        json["repo"]["api"]["artefacts"]["edges"][0]["node"]["outgoingDeps"]["edges"][0]["node"]["toArtefact"]
            ["symbolFqn"],
        "packages/api/src/target.ts::target"
    );
    assert_eq!(
        json["repo"]["api"]["artefacts"]["edges"][0]["node"]["outgoingDeps"]["edges"][1]["node"]["toSymbolRef"],
        "packages/web/src/page.ts::render"
    );
    assert_eq!(
        json["repo"]["api"]["artefacts"]["edges"][0]["node"]["outgoingDeps"]["edges"][1]["node"]["toArtefact"],
        serde_json::Value::Null
    );
    assert_eq!(json["repo"]["api"]["deps"]["totalCount"], 2);
    assert_eq!(
        json["repo"]["api"]["deps"]["edges"][1]["node"]["toArtefact"],
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
async fn devql_temporal_queries_resolve_historical_scope_once_and_propagate_to_children() {
    let seeded = seed_graphql_temporal_repo();
    let schema = crate::graphql::build_schema(crate::graphql::DevqlGraphqlContext::new(
        seeded.repo.path().to_path_buf(),
        super::db::DashboardDbPools::default(),
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
                            outgoingDeps {{
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
                    deps(filter: {{ direction: OUT }}, first: 10) {{
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
        json["repo"]["repoScoped"]["project"]["file"]["artefacts"]["edges"][0]["node"]["outgoingDeps"]
            ["totalCount"],
        1
    );
    assert_eq!(
        json["repo"]["repoScoped"]["project"]["file"]["artefacts"]["edges"][0]["node"]["outgoingDeps"]
            ["edges"][0]["node"]["toArtefact"]["symbolFqn"],
        "packages/api/src/target.ts::target"
    );
    assert_eq!(
        json["repo"]["repoScoped"]["project"]["deps"]["totalCount"],
        1
    );
    assert_eq!(
        json["repo"]["repoScoped"]["project"]["deps"]["edges"][0]["node"]["toArtefact"]["symbolFqn"],
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
        super::db::DashboardDbPools::default(),
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
        super::db::DashboardDbPools::default(),
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

#[tokio::test]
async fn devql_graphql_artefact_resolvers_validate_paths_and_line_ranges() {
    let repo = seed_graphql_devql_repo();
    let schema = crate::graphql::build_schema(crate::graphql::DevqlGraphqlContext::new(
        repo.path().to_path_buf(),
        super::db::DashboardDbPools::default(),
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

#[tokio::test]
async fn devql_graphql_parent_loader_caches_within_a_request_and_resets_per_request() {
    let repo = seed_graphql_devql_repo();
    let context = crate::graphql::DevqlGraphqlContext::new(
        repo.path().to_path_buf(),
        super::db::DashboardDbPools::default(),
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
        super::db::DashboardDbPools::default(),
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
        super::db::DashboardDbPools::default(),
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

#[tokio::test]
async fn devql_graphql_knowledge_queries_resolve_metadata_versions_relations_and_project_access() {
    let repo = seed_graphql_devql_repo();
    let seeded = seed_graphql_knowledge_data(repo.path());
    let schema = crate::graphql::build_schema(crate::graphql::DevqlGraphqlContext::new(
        repo.path().to_path_buf(),
        super::db::DashboardDbPools::default(),
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
        super::db::DashboardDbPools::default(),
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
        super::db::DashboardDbPools::default(),
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
        super::db::DashboardDbPools::default(),
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
    assert_eq!(first_snapshot.chat_history_batches, 1);

    let second_response = schema.execute(async_graphql::Request::new(query)).await;
    assert!(
        second_response.errors.is_empty(),
        "graphql errors: {:?}",
        second_response.errors
    );

    let second_snapshot = context.loader_metrics_snapshot();
    assert_eq!(second_snapshot.chat_history_batches, 2);
}

#[tokio::test]
async fn devql_graphql_chat_history_surfaces_backend_error_when_events_store_is_missing() {
    let repo = seed_graphql_devql_repo();
    let schema = crate::graphql::build_schema(crate::graphql::DevqlGraphqlContext::new(
        repo.path().to_path_buf(),
        super::db::DashboardDbPools::default(),
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

    assert_eq!(response.errors.len(), 1, "expected one graphql error");
    let extensions = response.errors[0]
        .extensions
        .as_ref()
        .expect("graphql error extensions");
    assert_eq!(
        extensions.get("code"),
        Some(&async_graphql::Value::from("BACKEND_ERROR"))
    );
    assert!(
        response.errors[0]
            .message
            .contains("DuckDB database file not found"),
        "unexpected error: {:?}",
        response.errors
    );
}

#[tokio::test]
async fn devql_graphql_clone_queries_resolve_project_and_artefact_results() {
    let repo = seed_graphql_monorepo_repo();
    seed_graphql_clone_data(repo.path());
    let schema = crate::graphql::build_schema(crate::graphql::DevqlGraphqlContext::new(
        repo.path().to_path_buf(),
        super::db::DashboardDbPools::default(),
    ));

    let response = schema
        .execute(async_graphql::Request::new(
            r#"
            {
              repo(name: "demo") {
                project(path: "packages/api") {
                  clones(filter: { minScore: 0.75 }, first: 10) {
                    totalCount
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
        json["repo"]["project"]["clones"]["edges"][0]["node"]["relationKind"],
        "similar_implementation"
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
async fn devql_graphql_clone_source_target_loader_caches_within_a_request_and_resets_per_request() {
    let repo = seed_graphql_monorepo_repo();
    seed_graphql_clone_data(repo.path());
    let context = crate::graphql::DevqlGraphqlContext::new(
        repo.path().to_path_buf(),
        super::db::DashboardDbPools::default(),
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
async fn devql_graphql_test_harness_pack_fields_resolve_typed_and_generic_results() {
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
        super::db::DashboardDbPools::default(),
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
                        extension(stage: "coverage", first: 5)
                      }}
                    }}
                  }}
                }}
                asOf(input: {{ commit: "{commit_sha}" }}) {{
                  project(path: "src") {{
                    extension(stage: "test_harness_tests_summary", first: 5)
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
        node["extension"][0]["coverage"]["coverage_source"],
        json!("lcov")
    );
    assert_eq!(
        json["repo"]["asOf"]["project"]["extension"][0]["commit_sha"],
        json!(commit_sha)
    );
    assert_eq!(
        json["repo"]["asOf"]["project"]["extension"][0]["coverage_present"],
        json!(true)
    );
}

#[tokio::test]
async fn devql_graphql_project_extension_respects_scope_and_unknown_stage_errors() {
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
        super::db::DashboardDbPools::default(),
    ));

    let response = schema
        .execute(async_graphql::Request::new(format!(
            r#"
            {{
              repo(name: "demo") {{
                asOf(input: {{ commit: "{commit_sha}" }}) {{
                  project(path: "packages/api") {{
                    extension(stage: "coverage", first: 10)
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
    let rows = json["repo"]["asOf"]["project"]["extension"]
        .as_array()
        .expect("extension rows");
    assert_eq!(rows.len(), 4);
    assert!(
        rows.iter().all(|row| {
            row["artefact"]["file_path"]
                .as_str()
                .unwrap_or_default()
                .starts_with("packages/api/")
        }),
        "expected only project-scoped rows, got {rows:?}"
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

    assert_eq!(bad_stage.errors.len(), 1, "expected one graphql error");
    let extensions = bad_stage.errors[0]
        .extensions
        .as_ref()
        .expect("graphql error extensions");
    assert_eq!(
        extensions.get("code"),
        Some(&async_graphql::Value::from("BAD_USER_INPUT"))
    );
    assert!(
        bad_stage.errors[0]
            .message
            .contains("unsupported DevQL stage"),
        "unexpected error: {:?}",
        bad_stage.errors
    );

    let bad_args = schema
        .execute(async_graphql::Request::new(
            r#"
            {
              repo(name: "demo") {
                project(path: "packages/api") {
                  extension(stage: "coverage", args: { nested: { enabled: true } }, first: 10)
                }
              }
            }
            "#,
        ))
        .await;

    assert_eq!(bad_args.errors.len(), 1, "expected one graphql error");
    let extensions = bad_args.errors[0]
        .extensions
        .as_ref()
        .expect("graphql error extensions");
    assert_eq!(
        extensions.get("code"),
        Some(&async_graphql::Value::from("BAD_USER_INPUT"))
    );
    assert!(
        bad_args.errors[0]
            .message
            .contains("extension args must contain only string, number, boolean, or null values"),
        "unexpected error: {:?}",
        bad_args.errors
    );
}

#[tokio::test]
async fn devql_event_resolvers_query_duckdb_checkpoints_and_telemetry() {
    let repo = seed_graphql_monorepo_repo_with_duckdb_events();
    let schema = crate::graphql::build_schema(crate::graphql::DevqlGraphqlContext::new(
        repo.path().to_path_buf(),
        super::db::DashboardDbPools::default(),
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
        super::db::DashboardDbPools::default(),
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

    assert_eq!(response.errors.len(), 1, "expected one graphql error");
    let extensions = response.errors[0]
        .extensions
        .as_ref()
        .expect("graphql error extensions");
    assert_eq!(
        extensions.get("code"),
        Some(&async_graphql::Value::from("BACKEND_ERROR"))
    );
    assert!(
        response.errors[0]
            .message
            .contains("DuckDB database file not found"),
        "unexpected error: {:?}",
        response.errors
    );
}

#[tokio::test]
async fn devql_event_checkpoint_commit_loader_batches_repository_checkpoint_reads() {
    let repo = seed_dashboard_repo_with_duckdb_events();
    let context = crate::graphql::DevqlGraphqlContext::new(
        repo.path().to_path_buf(),
        super::db::DashboardDbPools::default(),
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
    assert!(body.contains("type MutationRoot"));
    assert!(body.contains("repo(name: String!): Repository!"));
    assert!(body.contains("initSchema: InitSchemaResult!"));
    assert!(body.contains("ingest(input: IngestInput!): IngestResult!"));
    assert!(body.contains("checkpoints(agent: String, since: DateTime"));
    assert!(body.contains("telemetry(eventType: String, agent: String"));
    assert!(body.contains("knowledge(provider: KnowledgeProvider"));
    assert!(body.contains("clones(filter:"));
    assert!(body.contains("chatHistory"));
    assert!(body.contains("input IngestInput"));
    assert!(body.contains("type Clone"));
    assert!(body.contains("type ChatEntry"));
    assert!(body.contains("type InitSchemaResult"));
    assert!(body.contains("type IngestResult"));
    assert!(body.contains("type KnowledgeItem"));
    assert!(body.contains("type KnowledgePayload"));
    assert!(body.contains("type TelemetryEvent"));
    assert!(body.contains("type TelemetryEventConnection"));
    assert!(body.contains("type SubscriptionRoot"));
    assert!(body.contains("checkpointIngested(repoName: String!): Checkpoint!"));
    assert!(body.contains("ingestionProgress(repoName: String!): IngestionProgressEvent!"));
    assert!(body.contains("type IngestionProgressEvent"));
    assert!(body.contains("enum IngestionPhase"));
    assert!(body.contains("project(path: String!): Project!"));
    assert!(body.contains("asOf(input: AsOfInput!): TemporalScope!"));
    assert!(body.contains("input AsOfInput"));
    assert!(body.contains("type TemporalScope"));
}

#[test]
fn checked_in_schema_file_matches_runtime_sdl() {
    let expected = crate::graphql::schema_sdl();
    let schema_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("schema.graphql");
    let actual = fs::read_to_string(&schema_path).expect("read checked-in schema.graphql");
    assert_eq!(actual, expected);
}

fn graphql_parent_depth_limit_query(parent_depth: usize) -> String {
    let mut query = String::from(
        r#"{ repo(name: "demo") { file(path: "src/caller.ts") { artefacts(first: 1) { edges { node {"#,
    );
    for _ in 0..parent_depth {
        query.push_str(" parent {");
    }
    query.push_str(" id");
    for _ in 0..parent_depth {
        query.push_str(" }");
    }
    query.push_str(" } } } } } }");
    query
}

fn graphql_complexity_limit_query(alias_count: usize) -> String {
    let mut query = String::from("{");
    for index in 0..alias_count {
        write!(
            &mut query,
            r#" q{index}: health {{ relational {{ backend connected }} }}"#
        )
        .expect("writing GraphQL query should succeed");
    }
    query.push('}');
    query
}

#[tokio::test]
async fn devql_graphql_rejects_queries_over_the_depth_limit() {
    let repo = seed_graphql_devql_repo();
    let schema = crate::graphql::build_schema(crate::graphql::DevqlGraphqlContext::new(
        repo.path().to_path_buf(),
        super::db::DashboardDbPools::default(),
    ));
    let query = graphql_parent_depth_limit_query(crate::graphql::MAX_DEVQL_QUERY_DEPTH);

    let response = schema.execute(async_graphql::Request::new(query)).await;

    assert_eq!(response.errors.len(), 1, "expected one graphql error");
    assert!(
        response.errors[0]
            .message
            .to_ascii_lowercase()
            .contains("nested too deep"),
        "expected depth-limit error, got {:?}",
        response.errors
    );
}

#[tokio::test]
async fn devql_graphql_rejects_queries_over_the_complexity_limit() {
    let temp = TempDir::new().expect("temp dir");
    let schema = crate::graphql::build_schema(crate::graphql::DevqlGraphqlContext::new(
        temp.path().to_path_buf(),
        super::db::DashboardDbPools::default(),
    ));
    let query = graphql_complexity_limit_query(
        crate::graphql::MAX_DEVQL_QUERY_COMPLEXITY.saturating_add(1),
    );

    let response = schema.execute(async_graphql::Request::new(query)).await;

    assert_eq!(response.errors.len(), 1, "expected one graphql error");
    assert!(
        response.errors[0]
            .message
            .to_ascii_lowercase()
            .contains("too complex"),
        "expected complexity-limit error, got {:?}",
        response.errors
    );
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

#[tokio::test]
async fn devql_graphql_checkpoint_ingested_subscription_receives_published_checkpoint_events() {
    let repo = seed_dashboard_repo();
    let context = crate::graphql::DevqlGraphqlContext::new(
        repo.path().to_path_buf(),
        super::db::DashboardDbPools::default(),
    );
    let schema = crate::graphql::build_schema(context.clone());
    let (event_tx, mut event_rx) = tokio::sync::mpsc::unbounded_channel();
    let stream = schema.execute_stream(async_graphql::Request::new(
        r#"
        subscription {
          checkpointIngested(repoName: "demo") {
            id
            commitSha
            sessionId
          }
        }
        "#,
    ));
    let _collector = tokio::spawn(async move {
        let mut stream = stream;
        if let Some(event) = stream.next().await {
            let _ = event_tx.send(event);
        }
    });
    tokio::task::yield_now().await;

    let checkpoint = crate::host::checkpoints::strategy::manual_commit::read_committed_info(
        repo.path(),
        "aabbccddeeff",
    )
    .expect("read committed checkpoint")
    .expect("seeded checkpoint info");
    let mut other_repo_checkpoint =
        crate::graphql::Checkpoint::from_ingested(&checkpoint, Some("wrong-repo-sha"));
    other_repo_checkpoint.session_id = "other-repo-session".to_string();
    context
        .subscriptions()
        .publish_checkpoint("other-repo", other_repo_checkpoint);
    context.subscriptions().publish_checkpoint(
        "demo",
        crate::graphql::Checkpoint::from_ingested(
            &checkpoint,
            Some(&git_ok(repo.path(), &["rev-parse", "HEAD"])),
        ),
    );

    let first_event = tokio::time::timeout(std::time::Duration::from_secs(5), event_rx.recv())
        .await
        .expect("checkpoint subscription event should arrive")
        .expect("checkpoint subscription response");
    assert!(
        first_event.errors.is_empty(),
        "subscription errors: {:?}",
        first_event.errors
    );

    let event_json = first_event
        .data
        .into_json()
        .expect("subscription data to json");
    assert_eq!(
        event_json["checkpointIngested"]["commitSha"],
        Value::String(git_ok(repo.path(), &["rev-parse", "HEAD"]))
    );
    assert!(
        event_json["checkpointIngested"]["id"]
            .as_str()
            .is_some_and(|value| !value.trim().is_empty())
    );
    assert!(event_json["checkpointIngested"]["sessionId"].is_string());
}

#[tokio::test]
async fn devql_graphql_ingestion_progress_subscription_receives_published_progress_events() {
    let repo = seed_dashboard_repo();
    let context = crate::graphql::DevqlGraphqlContext::new(
        repo.path().to_path_buf(),
        super::db::DashboardDbPools::default(),
    );
    let schema = crate::graphql::build_schema(context.clone());
    let (event_tx, mut event_rx) = tokio::sync::mpsc::unbounded_channel();
    let stream = schema.execute_stream(async_graphql::Request::new(
        r#"
        subscription {
          ingestionProgress(repoName: "demo") {
            phase
            checkpointsTotal
            checkpointsProcessed
            currentCheckpointId
            currentCommitSha
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
    tokio::task::yield_now().await;

    context.subscriptions().publish_progress(
        "other-repo",
        crate::graphql::IngestionProgressEvent {
            phase: crate::graphql::IngestionPhase::Failed,
            checkpoints_total: 99,
            checkpoints_processed: 13,
            current_checkpoint_id: Some("wrong-repo-checkpoint".to_string()),
            current_commit_sha: Some("wrong-repo-sha".to_string()),
            events_inserted: 8,
            artefacts_upserted: 5,
            checkpoints_without_commit: 3,
            temporary_rows_promoted: 2,
        },
    );
    context.subscriptions().publish_progress(
        "demo",
        crate::graphql::IngestionProgressEvent {
            phase: crate::graphql::IngestionPhase::Initializing,
            checkpoints_total: 1,
            checkpoints_processed: 0,
            current_checkpoint_id: None,
            current_commit_sha: None,
            events_inserted: 0,
            artefacts_upserted: 0,
            checkpoints_without_commit: 0,
            temporary_rows_promoted: 0,
        },
    );
    context.subscriptions().publish_progress(
        "demo",
        crate::graphql::IngestionProgressEvent {
            phase: crate::graphql::IngestionPhase::Complete,
            checkpoints_total: 1,
            checkpoints_processed: 1,
            current_checkpoint_id: Some("aabbccddeeff".to_string()),
            current_commit_sha: Some(git_ok(repo.path(), &["rev-parse", "HEAD"])),
            events_inserted: 1,
            artefacts_upserted: 2,
            checkpoints_without_commit: 0,
            temporary_rows_promoted: 0,
        },
    );

    let mut phases = Vec::new();
    let mut last_payload = Value::Null;
    for _ in 0..2 {
        let event = tokio::time::timeout(std::time::Duration::from_secs(5), event_rx.recv())
            .await
            .expect("progress subscription should emit")
            .expect("progress response");
        assert!(
            event.errors.is_empty(),
            "subscription errors: {:?}",
            event.errors
        );
        let json = event.data.into_json().expect("progress event json");
        let payload = json["ingestionProgress"].clone();
        let phase = payload["phase"]
            .as_str()
            .expect("progress phase string")
            .to_string();
        phases.push(phase.clone());
        last_payload = payload;
        if phase == "COMPLETE" {
            break;
        }
    }
    assert!(phases.iter().any(|phase| phase == "INITIALIZING"));
    assert_eq!(phases.last(), Some(&"COMPLETE".to_string()));
    assert_eq!(last_payload["checkpointsTotal"], Value::from(1));
    assert_eq!(last_payload["checkpointsProcessed"], Value::from(1));
}

#[tokio::test]
async fn devql_ingest_mutation_publishes_progress_and_checkpoint_events_to_subscription_hub() {
    let repo = seed_dashboard_repo();
    let _guard = enter_process_state(
        Some(repo.path()),
        &[
            ("BITLOOPS_DEVQL_EMBEDDING_PROVIDER", Some("disabled")),
            ("BITLOOPS_DEVQL_SEMANTIC_PROVIDER", Some("disabled")),
        ],
    );
    write_envelope_config(
        repo.path(),
        json!({
            "stores": {
                "relational": {
                    "sqlite_path": ".bitloops/stores/subscriptions.sqlite"
                },
                "events": {
                    "duckdb_path": ".bitloops/stores/subscriptions.duckdb"
                },
                "embedding_provider": "disabled"
            },
            "semantic": {
                "provider": "disabled"
            }
        }),
    );
    let context = crate::graphql::DevqlGraphqlContext::new(
        repo.path().to_path_buf(),
        super::db::DashboardDbPools::default(),
    );
    let mut progress_rx = context.subscriptions().subscribe_progress();
    let mut checkpoint_rx = context.subscriptions().subscribe_checkpoints();
    let schema = crate::graphql::build_schema(context);

    let response = schema
        .execute(async_graphql::Request::new(
            r#"
            mutation {
              ingest(input: { init: true, maxCheckpoints: 1 }) {
                success
                checkpointsProcessed
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
    let response_json = response.data.into_json().expect("mutation data to json");
    if response_json["ingest"]["checkpointsProcessed"].as_i64() == Some(1) {
        let checkpoint =
            tokio::time::timeout(std::time::Duration::from_secs(5), checkpoint_rx.recv())
                .await
                .expect("checkpoint event should arrive")
                .expect("checkpoint subscription payload");
        assert_eq!(
            checkpoint.checkpoint.commit_sha,
            Some(git_ok(repo.path(), &["rev-parse", "HEAD"]))
        );
    }

    let mut saw_complete = false;
    for _ in 0..8 {
        let progress = tokio::time::timeout(std::time::Duration::from_secs(5), progress_rx.recv())
            .await
            .expect("progress event should arrive")
            .expect("progress subscription payload");
        if progress.event.phase == crate::graphql::IngestionPhase::Complete {
            saw_complete = true;
            break;
        }
    }
    assert!(saw_complete, "expected a COMPLETE ingestion progress event");
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
