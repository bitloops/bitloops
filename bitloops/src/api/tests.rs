#![allow(clippy::await_holding_lock)]

use super::router::build_dashboard_router;
use super::{
    ApiPage, DashboardServerConfig, DashboardStartupMode, DashboardState, DashboardTransport,
    GIT_FIELD_SEPARATOR, GIT_RECORD_SEPARATOR, ServeMode, branch_is_excluded, browser_host_for_url,
    build_branch_commit_log_args, canonical_agent_key, dashboard_user,
    default_bundle_dir_from_home, expand_tilde_with_home, format_dashboard_url, has_bundle_index,
    paginate, parse_branch_commit_log, parse_numstat_output, resolve_bundle_file,
    select_host_with_dashboard_preference, select_startup_mode, warning_block_lines,
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

mod dashboard_api_bundle;
mod dashboard_config_utils;
mod devql_knowledge_clone_events;
mod devql_mutations_and_health;
mod devql_repository_graph;
mod devql_routes_subscriptions;
mod numstat_output;

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

#[allow(clippy::too_many_arguments)]
fn insert_historical_function_artefact(
    conn: &rusqlite::Connection,
    repo_id: &str,
    artefact_id: &str,
    symbol_id: &str,
    blob_sha: &str,
    path: &str,
    symbol_fqn: &str,
    start_line: i64,
    end_line: i64,
    created_at: &str,
) {
    conn.execute(
        "INSERT INTO artefacts (
            artefact_id, symbol_id, repo_id, blob_sha, path, language, canonical_kind,
            language_kind, symbol_fqn, parent_artefact_id, start_line, end_line,
            start_byte, end_byte, signature, modifiers, docstring, content_hash, created_at
        ) VALUES (
            ?1, ?2, ?3, ?4, ?5, 'typescript', 'function',
            'function_declaration', ?6, NULL, ?7, ?8, 0, ?9, NULL, '[\"export\"]',
            'Event-backed docstring', ?10, ?11
        )",
        rusqlite::params![
            artefact_id,
            symbol_id,
            repo_id,
            blob_sha,
            path,
            symbol_fqn,
            start_line,
            end_line,
            end_line * 10,
            format!("hash-{artefact_id}"),
            created_at,
        ],
    )
    .expect("insert historical function artefact");
}

#[allow(clippy::too_many_arguments)]
fn insert_current_function_artefact(
    conn: &rusqlite::Connection,
    repo_id: &str,
    branch: &str,
    artefact_id: &str,
    symbol_id: &str,
    commit_sha: &str,
    revision_kind: &str,
    revision_id: &str,
    blob_sha: &str,
    path: &str,
    symbol_fqn: &str,
    start_line: i64,
    end_line: i64,
    updated_at: &str,
) {
    conn.execute(
        "INSERT INTO artefacts_current (
            repo_id, branch, symbol_id, artefact_id, commit_sha, revision_kind, revision_id,
            temp_checkpoint_id, blob_sha, path, language, canonical_kind, language_kind,
            symbol_fqn, parent_symbol_id, parent_artefact_id, start_line, end_line,
            start_byte, end_byte, signature, modifiers, docstring, content_hash, updated_at
        ) VALUES (
            ?1, ?2, ?3, ?4, ?5, ?6, ?7,
            NULL, ?8, ?9, 'typescript', 'function', 'function_declaration',
            ?10, NULL, NULL, ?11, ?12,
            0, ?13, NULL, '[\"export\"]', 'Event-backed docstring', ?14, ?15
        )",
        rusqlite::params![
            repo_id,
            branch,
            symbol_id,
            artefact_id,
            commit_sha,
            revision_kind,
            revision_id,
            blob_sha,
            path,
            symbol_fqn,
            start_line,
            end_line,
            end_line * 10,
            format!("hash-{artefact_id}"),
            updated_at,
        ],
    )
    .expect("insert current function artefact");
}

fn insert_file_state_row(
    conn: &rusqlite::Connection,
    repo_id: &str,
    commit_sha: &str,
    path: &str,
    blob_sha: &str,
) {
    conn.execute(
        "INSERT INTO file_state (repo_id, commit_sha, path, blob_sha)
         VALUES (?1, ?2, ?3, ?4)",
        rusqlite::params![repo_id, commit_sha, path, blob_sha],
    )
    .expect("insert file_state row");
}

fn insert_current_file_state_row(
    conn: &rusqlite::Connection,
    repo_id: &str,
    path: &str,
    commit_sha: &str,
    blob_sha: &str,
    committed_at: &str,
) {
    conn.execute(
        "INSERT INTO current_file_state (repo_id, path, commit_sha, blob_sha, committed_at)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        rusqlite::params![repo_id, path, commit_sha, blob_sha, committed_at],
    )
    .expect("insert current_file_state row");
}

#[allow(clippy::too_many_arguments)]
fn insert_checkpoint_file_snapshot_row(
    conn: &rusqlite::Connection,
    repo_id: &str,
    checkpoint_id: &str,
    session_id: &str,
    event_time: &str,
    agent: &str,
    branch: &str,
    strategy: &str,
    commit_sha: &str,
    path: &str,
    blob_sha: &str,
) {
    conn.execute(
        "INSERT INTO checkpoint_file_snapshots (
            repo_id, checkpoint_id, session_id, event_time, agent, branch, strategy,
            commit_sha, path, blob_sha
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        rusqlite::params![
            repo_id,
            checkpoint_id,
            session_id,
            event_time,
            agent,
            branch,
            strategy,
            commit_sha,
            path,
            blob_sha,
        ],
    )
    .expect("insert checkpoint_file_snapshots row");
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

struct SeededGraphqlEventBackedRepo {
    repo: TempDir,
    first_commit: String,
}

fn seed_graphql_event_backed_repo() -> SeededGraphqlEventBackedRepo {
    let dir = TempDir::new().expect("temp dir");
    let repo_root = dir.path();

    init_test_repo(repo_root, "main", "Alice", "alice@example.com");
    fs::create_dir_all(repo_root.join("packages/api/src")).expect("create api src dir");
    fs::write(
        repo_root.join("packages/api/src/caller.ts"),
        "export function callerV1() {\n  return 1;\n}\n",
    )
    .expect("write caller v1");
    fs::write(
        repo_root.join("packages/api/src/target.ts"),
        "export function targetV1() {\n  return 2;\n}\n",
    )
    .expect("write target v1");
    git_ok(repo_root, &["add", "."]);
    git_ok(
        repo_root,
        &["commit", "-m", "Seed event-backed GraphQL commit 1"],
    );
    let first_commit = git_ok(repo_root, &["rev-parse", "HEAD"]);

    fs::write(
        repo_root.join("packages/api/src/caller.ts"),
        "export function callerCurrent() {\n  return 10;\n}\n",
    )
    .expect("write caller current");
    fs::write(
        repo_root.join("packages/api/src/target.ts"),
        "export function targetCurrent() {\n  return 20;\n}\n",
    )
    .expect("write target current");
    fs::write(
        repo_root.join("packages/api/src/copy.ts"),
        "export function copyCurrent() {\n  return 20;\n}\n",
    )
    .expect("write copy current");
    git_ok(repo_root, &["add", "."]);
    git_ok(
        repo_root,
        &["commit", "-m", "Seed event-backed GraphQL commit 2"],
    );
    let second_commit = git_ok(repo_root, &["rev-parse", "HEAD"]);

    let sqlite_path = repo_root
        .join(".bitloops")
        .join("stores")
        .join("graphql-event-backed.sqlite");
    crate::storage::init::init_database(&sqlite_path, false, &second_commit)
        .expect("initialise GraphQL event-backed sqlite store");
    crate::storage::SqliteConnectionPool::connect(sqlite_path.clone())
        .expect("connect GraphQL event-backed sqlite store")
        .initialise_devql_schema()
        .expect("initialise DevQL schema for event-backed GraphQL sqlite store");
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
    let conn = rusqlite::Connection::open(&sqlite_path).expect("open GraphQL event-backed sqlite");
    conn.execute(
        "INSERT INTO repositories (repo_id, provider, organization, name, default_branch)
         VALUES (?1, 'local', 'local', 'graphql-event-backed', 'main')",
        rusqlite::params![repo_id.as_str()],
    )
    .expect("insert repository row");
    conn.execute(
        "INSERT INTO commits (commit_sha, repo_id, author_name, author_email, commit_message, committed_at)
         VALUES (?1, ?2, 'Alice', 'alice@example.com', 'Seed event-backed GraphQL commit 1', '2026-03-25T09:00:00Z')",
        rusqlite::params![first_commit.as_str(), repo_id.as_str()],
    )
    .expect("insert first commit row");
    conn.execute(
        "INSERT INTO commits (commit_sha, repo_id, author_name, author_email, commit_message, committed_at)
         VALUES (?1, ?2, 'Alice', 'alice@example.com', 'Seed event-backed GraphQL commit 2', '2026-03-26T09:00:00Z')",
        rusqlite::params![second_commit.as_str(), repo_id.as_str()],
    )
    .expect("insert second commit row");

    for (commit_sha, path, blob_sha) in [
        (
            first_commit.as_str(),
            "packages/api/src/caller.ts",
            "blob-caller-v1",
        ),
        (
            first_commit.as_str(),
            "packages/api/src/target.ts",
            "blob-target-v1",
        ),
        (
            second_commit.as_str(),
            "packages/api/src/caller.ts",
            "blob-caller-v2",
        ),
        (
            second_commit.as_str(),
            "packages/api/src/target.ts",
            "blob-target-v2",
        ),
        (
            second_commit.as_str(),
            "packages/api/src/copy.ts",
            "blob-target-v2",
        ),
    ] {
        insert_file_state_row(&conn, repo_id.as_str(), commit_sha, path, blob_sha);
    }

    for (path, blob_sha) in [
        ("packages/api/src/caller.ts", "blob-caller-v2"),
        ("packages/api/src/target.ts", "blob-target-v2"),
        ("packages/api/src/copy.ts", "blob-target-v2"),
    ] {
        insert_current_file_state_row(
            &conn,
            repo_id.as_str(),
            path,
            second_commit.as_str(),
            blob_sha,
            "2026-03-26T09:00:00Z",
        );
    }

    insert_historical_function_artefact(
        &conn,
        repo_id.as_str(),
        "artefact::caller-v1",
        "sym::caller-v1",
        "blob-caller-v1",
        "packages/api/src/caller.ts",
        "packages/api/src/caller.ts::callerV1",
        1,
        3,
        "2026-03-25T09:00:00Z",
    );
    insert_historical_function_artefact(
        &conn,
        repo_id.as_str(),
        "artefact::target-v1",
        "sym::target-v1",
        "blob-target-v1",
        "packages/api/src/target.ts",
        "packages/api/src/target.ts::targetV1",
        1,
        3,
        "2026-03-25T09:00:00Z",
    );

    insert_current_function_artefact(
        &conn,
        repo_id.as_str(),
        "main",
        "artefact::caller-current",
        "sym::caller-current",
        second_commit.as_str(),
        "commit",
        second_commit.as_str(),
        "blob-caller-v2",
        "packages/api/src/caller.ts",
        "packages/api/src/caller.ts::callerCurrent",
        1,
        3,
        "2026-03-26T09:00:00Z",
    );
    insert_current_function_artefact(
        &conn,
        repo_id.as_str(),
        "main",
        "artefact::target-current",
        "sym::target-current",
        second_commit.as_str(),
        "commit",
        second_commit.as_str(),
        "blob-target-v2",
        "packages/api/src/target.ts",
        "packages/api/src/target.ts::targetCurrent",
        1,
        3,
        "2026-03-26T09:00:00Z",
    );
    insert_current_function_artefact(
        &conn,
        repo_id.as_str(),
        "main",
        "artefact::copy-current",
        "sym::copy-current",
        second_commit.as_str(),
        "commit",
        second_commit.as_str(),
        "blob-target-v2",
        "packages/api/src/copy.ts",
        "packages/api/src/copy.ts::copyCurrent",
        1,
        3,
        "2026-03-26T09:00:00Z",
    );

    insert_checkpoint_file_snapshot_row(
        &conn,
        repo_id.as_str(),
        "checkpoint-v1-caller",
        "session-v1",
        "2026-03-25T09:30:00Z",
        "codex",
        "main",
        "manual",
        first_commit.as_str(),
        "packages/api/src/caller.ts",
        "blob-caller-v1",
    );
    insert_checkpoint_file_snapshot_row(
        &conn,
        repo_id.as_str(),
        "checkpoint-v2-caller",
        "session-v2",
        "2026-03-26T09:30:00Z",
        "codex",
        "main",
        "manual",
        second_commit.as_str(),
        "packages/api/src/caller.ts",
        "blob-caller-v2",
    );
    insert_checkpoint_file_snapshot_row(
        &conn,
        repo_id.as_str(),
        "checkpoint-v2-target",
        "session-v2",
        "2026-03-26T09:45:00Z",
        "codex",
        "main",
        "manual",
        second_commit.as_str(),
        "packages/api/src/target.ts",
        "blob-target-v2",
    );

    SeededGraphqlEventBackedRepo {
        repo: dir,
        first_commit,
    }
}

struct SeededGraphqlSaveRevisionEventBackedRepo {
    repo: TempDir,
    save_revision: String,
}

fn seed_graphql_save_revision_event_backed_repo() -> SeededGraphqlSaveRevisionEventBackedRepo {
    let dir = TempDir::new().expect("temp dir");
    let repo_root = dir.path();
    let save_revision = "temp:42".to_string();

    init_test_repo(repo_root, "main", "Alice", "alice@example.com");
    fs::create_dir_all(repo_root.join("packages/api/src")).expect("create api src dir");
    fs::write(
        repo_root.join("packages/api/src/caller.ts"),
        "export function callerBase() {\n  return 1;\n}\n",
    )
    .expect("write caller base");
    fs::write(
        repo_root.join("packages/api/src/target.ts"),
        "export function targetBase() {\n  return 2;\n}\n",
    )
    .expect("write target base");
    git_ok(repo_root, &["add", "."]);
    git_ok(
        repo_root,
        &[
            "commit",
            "-m",
            "Seed saveRevision event-backed GraphQL commit",
        ],
    );
    let commit_sha = git_ok(repo_root, &["rev-parse", "HEAD"]);

    let sqlite_path = repo_root
        .join(".bitloops")
        .join("stores")
        .join("graphql-save-revision.sqlite");
    crate::storage::init::init_database(&sqlite_path, false, &commit_sha)
        .expect("initialise GraphQL saveRevision sqlite store");
    crate::storage::SqliteConnectionPool::connect(sqlite_path.clone())
        .expect("connect GraphQL saveRevision sqlite store")
        .initialise_devql_schema()
        .expect("initialise DevQL schema for saveRevision GraphQL sqlite store");
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
    let conn = rusqlite::Connection::open(&sqlite_path).expect("open GraphQL saveRevision sqlite");
    conn.execute(
        "INSERT INTO repositories (repo_id, provider, organization, name, default_branch)
         VALUES (?1, 'local', 'local', 'graphql-save-revision', 'main')",
        rusqlite::params![repo_id.as_str()],
    )
    .expect("insert repository row");
    conn.execute(
        "INSERT INTO commits (commit_sha, repo_id, author_name, author_email, commit_message, committed_at)
         VALUES (?1, ?2, 'Alice', 'alice@example.com', 'Seed saveRevision event-backed GraphQL commit', '2026-03-26T11:00:00Z')",
        rusqlite::params![commit_sha.as_str(), repo_id.as_str()],
    )
    .expect("insert commit row");

    insert_current_function_artefact(
        &conn,
        repo_id.as_str(),
        "main",
        "artefact::caller-temp",
        "sym::caller-temp",
        commit_sha.as_str(),
        "temporary",
        save_revision.as_str(),
        "blob-caller-temp",
        "packages/api/src/caller.ts",
        "packages/api/src/caller.ts::callerTemp",
        1,
        3,
        "2026-03-26T11:15:00Z",
    );
    insert_current_function_artefact(
        &conn,
        repo_id.as_str(),
        "main",
        "artefact::target-temp",
        "sym::target-temp",
        commit_sha.as_str(),
        "temporary",
        save_revision.as_str(),
        "blob-target-temp",
        "packages/api/src/target.ts",
        "packages/api/src/target.ts::targetTemp",
        1,
        3,
        "2026-03-26T11:15:00Z",
    );

    insert_checkpoint_file_snapshot_row(
        &conn,
        repo_id.as_str(),
        "checkpoint-temp-caller",
        "session-temp",
        "2026-03-26T11:20:00Z",
        "codex",
        "main",
        "manual",
        commit_sha.as_str(),
        "packages/api/src/caller.ts",
        "blob-caller-temp",
    );
    insert_checkpoint_file_snapshot_row(
        &conn,
        repo_id.as_str(),
        "checkpoint-temp-target",
        "session-temp",
        "2026-03-26T11:25:00Z",
        "codex",
        "main",
        "manual",
        commit_sha.as_str(),
        "packages/api/src/target.ts",
        "blob-target-temp",
    );

    SeededGraphqlSaveRevisionEventBackedRepo {
        repo: dir,
        save_revision,
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
