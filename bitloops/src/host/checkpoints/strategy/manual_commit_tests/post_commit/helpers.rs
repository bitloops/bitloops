use super::*;
use crate::host::interactions::db_store::{SqliteInteractionSpool, interaction_spool_db_path};
use crate::host::interactions::store::InteractionSpool;
use crate::host::interactions::types::{InteractionSession, InteractionTurn};

pub(crate) fn open_test_spool(repo_root: &Path) -> SqliteInteractionSpool {
    let repo_id = crate::host::devql::resolve_repo_identity(repo_root)
        .expect("resolve repo identity")
        .repo_id;
    let sqlite = SqliteConnectionPool::connect(interaction_spool_db_path(repo_root))
        .expect("open interaction spool sqlite");
    SqliteInteractionSpool::new(sqlite, repo_id).expect("initialise interaction spool")
}

pub(crate) fn seed_interaction_turn(
    repo_root: &Path,
    session_id: &str,
    turn_id: &str,
    files: &[&str],
) {
    let spool = open_test_spool(repo_root);
    let transcript_path = repo_root.join("transcript.jsonl");
    if !transcript_path.exists() {
        std::fs::write(&transcript_path, "{}\n").expect("seed transcript");
    }
    let session = InteractionSession {
        session_id: session_id.to_string(),
        repo_id: spool.repo_id().to_string(),
        agent_type: "codex".to_string(),
        model: "gpt-5.4".to_string(),
        first_prompt: "ship it".to_string(),
        transcript_path: transcript_path.to_string_lossy().to_string(),
        worktree_path: repo_root.to_string_lossy().to_string(),
        worktree_id: "main".to_string(),
        started_at: "2026-04-05T10:00:00Z".to_string(),
        last_event_at: "2026-04-05T10:00:01Z".to_string(),
        updated_at: "2026-04-05T10:00:01Z".to_string(),
        ..Default::default()
    };
    let turn = InteractionTurn {
        turn_id: turn_id.to_string(),
        session_id: session_id.to_string(),
        repo_id: spool.repo_id().to_string(),
        turn_number: 1,
        prompt: "make the change".to_string(),
        agent_type: "codex".to_string(),
        model: "gpt-5.4".to_string(),
        started_at: "2026-04-05T10:00:01Z".to_string(),
        ended_at: Some("2026-04-05T10:00:02Z".to_string()),
        token_usage: Some(TokenUsageMetadata {
            input_tokens: 10,
            output_tokens: 5,
            ..Default::default()
        }),
        summary: "implemented requested change".to_string(),
        prompt_count: 1,
        transcript_offset_start: Some(0),
        transcript_offset_end: Some(1),
        files_modified: files.iter().map(|file| file.to_string()).collect(),
        updated_at: "2026-04-05T10:00:02Z".to_string(),
        ..Default::default()
    };
    spool.record_session(&session).expect("record session");
    spool.record_turn(&turn).expect("record turn");
}

pub(crate) fn commit_file(repo_root: &Path, filename: &str, content: &str) {
    fs::write(repo_root.join(filename), content).unwrap();
    git_ok(repo_root, &["add", filename]);
    git_ok(repo_root, &["commit", "-m", "test commit"]);
}

pub(crate) fn init_devql_schema(repo_root: &Path) -> PathBuf {
    init_devql_schema_with_store_backend(repo_root, None, None, None, None, None)
}

pub(crate) fn init_devql_schema_with_postgres_dsn(
    repo_root: &Path,
    postgres_dsn: Option<&str>,
) -> PathBuf {
    init_devql_schema_with_store_backend(repo_root, postgres_dsn, None, None, None, None)
}

pub(crate) fn init_devql_schema_with_clickhouse(
    repo_root: &Path,
    clickhouse_url: &str,
    clickhouse_user: Option<&str>,
    clickhouse_password: Option<&str>,
    clickhouse_database: Option<&str>,
) -> PathBuf {
    init_devql_schema_with_store_backend(
        repo_root,
        None,
        Some(clickhouse_url),
        clickhouse_user,
        clickhouse_password,
        clickhouse_database.or(Some("default")),
    )
}

fn init_devql_schema_with_store_backend(
    repo_root: &Path,
    postgres_dsn: Option<&str>,
    clickhouse_url: Option<&str>,
    clickhouse_user: Option<&str>,
    clickhouse_password: Option<&str>,
    clickhouse_database: Option<&str>,
) -> PathBuf {
    let bitloops_dir = repo_root.join(".bitloops");
    fs::create_dir_all(&bitloops_dir).expect("create .bitloops directory");
    write_post_commit_test_config(
        repo_root,
        None,
        clickhouse_url,
        clickhouse_user,
        clickhouse_password,
        clickhouse_database,
    );

    let repo = crate::host::devql::resolve_repo_identity(repo_root).expect("resolve repo identity");
    let cfg = crate::host::devql::DevqlConfig::from_env(repo_root.to_path_buf(), repo)
        .expect("build devql cfg for post-commit test");
    let runtime = tokio::runtime::Runtime::new().expect("create tokio runtime for devql init");
    runtime
        .block_on(crate::host::devql::run_init(&cfg))
        .expect("initialise DevQL schema for post-commit test");

    let sqlite_path = repo_root.join(".bitloops/stores/relational/post-commit-devql.db");
    let sqlite = rusqlite::Connection::open(&sqlite_path)
        .expect("open relational sqlite after DevQL init for post-commit test");
    sqlite
        .execute_batch(
            r#"
CREATE TABLE IF NOT EXISTS repositories (
    repo_id TEXT PRIMARY KEY,
    provider TEXT NOT NULL,
    organization TEXT NOT NULL,
    name TEXT NOT NULL,
    default_branch TEXT,
    created_at TEXT DEFAULT (datetime('now'))
);
"#,
        )
        .expect("ensure DevQL repository catalog exists for post-commit test");
    sqlite
        .execute(
            "INSERT INTO repositories (repo_id, provider, organization, name, default_branch) \
             VALUES (?1, ?2, ?3, ?4, 'main') \
             ON CONFLICT(repo_id) DO UPDATE SET \
               provider = excluded.provider, \
               organization = excluded.organization, \
               name = excluded.name, \
               default_branch = excluded.default_branch",
            rusqlite::params![
                cfg.repo.repo_id.as_str(),
                cfg.repo.provider.as_str(),
                cfg.repo.organization.as_str(),
                cfg.repo.name.as_str(),
            ],
        )
        .expect("seed DevQL repository catalog row for post-commit test");
    sqlite
        .execute_batch(crate::host::devql::checkpoint_schema_sql_sqlite())
        .expect("ensure checkpoint projection tables exist for post-commit test");
    sqlite
        .execute_batch(crate::host::devql::sync::schema::sync_schema_sql())
        .expect("ensure DevQL sync tables exist for post-commit test");
    let has_artefacts_current: i64 = sqlite
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'artefacts_current'",
            [],
            |row| row.get(0),
        )
        .expect("query sqlite_master for artefacts_current table");
    assert_eq!(
        has_artefacts_current, 1,
        "post-commit test must initialise DevQL relational schema in the configured sqlite path"
    );
    let has_repo_sync_state: i64 = sqlite
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'repo_sync_state'",
            [],
            |row| row.get(0),
        )
        .expect("query sqlite_master for repo_sync_state table");
    assert_eq!(
        has_repo_sync_state, 1,
        "post-commit test must initialise DevQL sync schema in the configured sqlite path"
    );

    write_post_commit_test_config(
        repo_root,
        postgres_dsn,
        clickhouse_url,
        clickhouse_user,
        clickhouse_password,
        clickhouse_database,
    );

    sqlite_path
}

fn write_post_commit_test_config(
    repo_root: &Path,
    postgres_dsn: Option<&str>,
    clickhouse_url: Option<&str>,
    clickhouse_user: Option<&str>,
    clickhouse_password: Option<&str>,
    clickhouse_database: Option<&str>,
) {
    let sqlite_path = repo_root.join(".bitloops/stores/relational/post-commit-devql.db");
    let duckdb_path = repo_root.join(".bitloops/stores/events/post-commit-events.duckdb");
    let blob_local_path = repo_root.join(".bitloops/stores/blobs/post-commit");
    let postgres_line = postgres_dsn
        .map(|dsn| format!("postgres_dsn = {dsn:?}\n"))
        .unwrap_or_default();
    let clickhouse_lines = match clickhouse_url {
        Some(url) => {
            let mut lines = format!(
                "clickhouse_url = {url:?}\nclickhouse_database = {database:?}\n",
                database = clickhouse_database.unwrap_or("default"),
            );
            if let Some(user) = clickhouse_user {
                lines.push_str(&format!("clickhouse_user = {user:?}\n"));
            }
            if let Some(password) = clickhouse_password {
                lines.push_str(&format!("clickhouse_password = {password:?}\n"));
            }
            lines
        }
        None => format!(
            "duckdb_path = {duckdb_path:?}\n",
            duckdb_path = duckdb_path.to_string_lossy()
        ),
    };
    fs::write(
        repo_root.join(crate::config::BITLOOPS_CONFIG_RELATIVE_PATH),
        format!(
            "[stores.relational]\nsqlite_path = {sqlite_path:?}\n{postgres_line}\n[stores.event]\n{clickhouse_lines}\n[stores.blob]\nlocal_path = {blob_local_path:?}\n",
            sqlite_path = sqlite_path.to_string_lossy(),
            clickhouse_lines = clickhouse_lines,
            blob_local_path = blob_local_path.to_string_lossy(),
        ),
    )
    .expect("write repo-local store config for post-commit tests");
}
