use super::*;
use toml_edit::{Array, DocumentMut, Item, Table, Value as TomlValue};

pub(super) const SEEDED_REPO_NAME: &str = "demo";

pub(super) fn insert_commit_checkpoint_mapping(
    repo_root: &Path,
    commit_sha: &str,
    checkpoint_id: &str,
) {
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

pub(super) fn checkpoint_sqlite_path(repo_root: &Path) -> PathBuf {
    let cfg = crate::config::resolve_store_backend_config_for_repo(repo_root)
        .expect("resolve backend config");
    if let Some(path) = cfg.relational.sqlite_path.as_deref() {
        crate::config::resolve_sqlite_db_path_for_repo(repo_root, Some(path))
            .expect("resolve configured sqlite path")
    } else {
        crate::utils::paths::default_relational_db_path(repo_root)
    }
}

fn json_value_to_toml_item(value: &serde_json::Value) -> Item {
    match value {
        serde_json::Value::Null => Item::None,
        serde_json::Value::Bool(value) => Item::Value(TomlValue::from(*value)),
        serde_json::Value::Number(number) => {
            if let Some(value) = number.as_i64() {
                Item::Value(TomlValue::from(value))
            } else if let Some(value) = number.as_u64() {
                Item::Value(TomlValue::from(value as i64))
            } else if let Some(value) = number.as_f64() {
                Item::Value(TomlValue::from(value))
            } else {
                panic!("unsupported numeric config value: {number}");
            }
        }
        serde_json::Value::String(value) => Item::Value(TomlValue::from(value.as_str())),
        serde_json::Value::Array(values) => {
            let mut array = Array::new();
            for value in values {
                let Item::Value(value) = json_value_to_toml_item(value) else {
                    panic!("test config arrays must contain scalar values");
                };
                array.push(value);
            }
            Item::Value(TomlValue::Array(array))
        }
        serde_json::Value::Object(map) => {
            let mut table = Table::new();
            for (key, value) in map {
                table[key] = json_value_to_toml_item(value);
            }
            Item::Table(table)
        }
    }
}

fn write_daemon_test_config(repo_root: &Path, settings: serde_json::Value) {
    let config_path = repo_root.join(crate::config::BITLOOPS_CONFIG_RELATIVE_PATH);
    let parent = config_path.parent().expect("config parent");
    fs::create_dir_all(parent).expect("create config dir");

    let mut doc = DocumentMut::new();
    for (key, value) in settings.as_object().expect("top-level config object") {
        doc[key] = json_value_to_toml_item(value);
    }
    fs::write(config_path, doc.to_string()).expect("write config");
}

pub(super) fn write_envelope_config(repo_root: &Path, settings: serde_json::Value) {
    write_daemon_test_config(repo_root, settings);
}

pub(super) fn seed_repository_catalog_row(repo_root: &Path, repo_name: &str, default_branch: &str) {
    let head_commit = git_ok(repo_root, &["rev-parse", "HEAD"]);
    let sqlite_path = checkpoint_sqlite_path(repo_root);
    crate::storage::init::init_database(&sqlite_path, false, &head_commit)
        .expect("initialise relational sqlite store");

    let repo_id = crate::host::devql::resolve_repo_id(repo_root).expect("resolve repo id");
    let conn = rusqlite::Connection::open(&sqlite_path).expect("open relational sqlite store");
    conn.execute(
        "INSERT INTO repositories (repo_id, provider, organization, name, default_branch)
         VALUES (?1, 'local', 'local', ?2, ?3)
         ON CONFLICT(repo_id) DO UPDATE SET
            provider = excluded.provider,
            organization = excluded.organization,
            name = excluded.name,
            default_branch = excluded.default_branch",
        rusqlite::params![repo_id.as_str(), repo_name, default_branch],
    )
    .expect("upsert repository row");
}

pub(super) fn update_seeded_jira_site_url(repo_root: &Path, jira_site_url: &str) {
    let config_path = repo_root.join(crate::config::BITLOOPS_CONFIG_RELATIVE_PATH);
    let raw = fs::read_to_string(&config_path).expect("read config");
    let mut config: Value = toml_edit::de::from_str(&raw).expect("parse config");

    let knowledge = config
        .as_object_mut()
        .expect("config object")
        .entry("knowledge")
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .expect("knowledge object");
    let providers = knowledge
        .entry("providers")
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .expect("providers object");
    let jira = providers
        .entry("jira")
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .expect("jira object");
    jira.insert("site_url".to_string(), json!(jira_site_url));

    write_daemon_test_config(repo_root, config);
}

#[allow(clippy::too_many_arguments)]
pub(super) fn insert_historical_function_artefact(
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
pub(super) fn insert_current_function_artefact(
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

pub(super) fn insert_file_state_row(
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

pub(super) fn insert_current_file_state_row(
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
pub(super) fn insert_checkpoint_file_snapshot_row(
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

pub(super) struct MockHttpResponse {
    pub(super) status_code: u16,
    pub(super) body: String,
}

impl MockHttpResponse {
    pub(super) fn json(status_code: u16, body: serde_json::Value) -> Self {
        Self {
            status_code,
            body: serde_json::to_string(&body).expect("serialise mock body"),
        }
    }
}

pub(super) struct MockSequentialHttpServer {
    pub(super) url: String,
    pub(super) handle: Option<thread::JoinHandle<()>>,
    shutdown: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl MockSequentialHttpServer {
    pub(super) fn start(responses: Vec<MockHttpResponse>) -> Self {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind mock server");
        listener
            .set_nonblocking(true)
            .expect("set nonblocking mock server");
        let addr = listener.local_addr().expect("mock server addr");
        let url = format!("http://{}", addr);
        let shutdown = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let shutdown_for_thread = std::sync::Arc::clone(&shutdown);

        let handle = thread::spawn(move || {
            // CI can delay tests while process-global state locks are contended.
            // Give this mock server enough time before treating a missing request as a timeout.
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
            let mut responses = std::collections::VecDeque::from(responses);

            while let Some(response) = responses.pop_front() {
                if shutdown_for_thread.load(std::sync::atomic::Ordering::Relaxed) {
                    break;
                }
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
                        if shutdown_for_thread.load(std::sync::atomic::Ordering::Relaxed) {
                            break;
                        }
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
            shutdown,
        }
    }
}

impl Drop for MockSequentialHttpServer {
    fn drop(&mut self) {
        self.shutdown
            .store(true, std::sync::atomic::Ordering::Relaxed);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

pub(super) struct SeedGraphqlEvent<'a> {
    pub(super) event_id: &'a str,
    pub(super) event_time: &'a str,
    pub(super) checkpoint_id: &'a str,
    pub(super) session_id: &'a str,
    pub(super) commit_sha: &'a str,
    pub(super) branch: &'a str,
    pub(super) event_type: &'a str,
    pub(super) agent: &'a str,
    pub(super) strategy: &'a str,
    pub(super) files_touched: &'a [&'a str],
    pub(super) payload: serde_json::Value,
}

pub(super) fn duckdb_literal(value: &str) -> String {
    value.replace('\'', "''")
}

pub(super) fn seed_duckdb_events(repo_root: &Path, events: &[SeedGraphqlEvent<'_>]) {
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

pub(super) struct SeedCheckpointSession<'a> {
    pub(super) session_index: i64,
    pub(super) session_id: &'a str,
    pub(super) agent: &'a str,
    pub(super) created_at: &'a str,
    pub(super) checkpoints_count: i64,
    pub(super) transcript: &'a str,
    pub(super) prompts: &'a str,
    pub(super) context: &'a str,
}

pub(super) struct SeedCheckpointStorage<'a> {
    pub(super) commit_sha: &'a str,
    pub(super) checkpoint_id: &'a str,
    pub(super) branch: &'a str,
    pub(super) files_touched: &'a [&'a str],
    pub(super) checkpoints_count: i64,
    pub(super) token_usage: serde_json::Value,
    pub(super) sessions: &'a [SeedCheckpointSession<'a>],
    pub(super) insert_mapping: bool,
}

pub(super) fn seed_checkpoint_storage_for_dashboard(
    repo_root: &Path,
    seed: SeedCheckpointStorage<'_>,
) {
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
