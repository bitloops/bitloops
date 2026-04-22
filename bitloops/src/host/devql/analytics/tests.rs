use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use tempfile::TempDir;

use super::super::RepoIdentity;
use super::*;

fn sample_repo_identity() -> RepoIdentity {
    RepoIdentity {
        provider: "local".to_string(),
        organization: "bitloops".to_string(),
        name: "analytics".to_string(),
        identity: "local://bitloops/analytics".to_string(),
        repo_id: "repo-analytics".to_string(),
    }
}

fn sample_cfg(root: &Path) -> DevqlConfig {
    DevqlConfig {
        daemon_config_root: root.to_path_buf(),
        repo_root: root.to_path_buf(),
        repo: sample_repo_identity(),
        pg_dsn: None,
        clickhouse_url: "http://localhost:8123".to_string(),
        clickhouse_user: None,
        clickhouse_password: None,
        clickhouse_database: "default".to_string(),
    }
}

fn write_test_config(root: &Path, sqlite_path: &Path, duckdb_path: &Path) {
    std::fs::create_dir_all(root).expect("create daemon root");
    std::fs::write(
        root.join(crate::config::BITLOOPS_CONFIG_RELATIVE_PATH),
        format!(
            "[stores]\n[stores.relational]\nsqlite_path = {:?}\n\n[stores.events]\nduckdb_path = {:?}\n",
            sqlite_path.to_string_lossy(),
            duckdb_path.to_string_lossy()
        ),
    )
    .expect("write config");
    crate::config::settings::write_repo_daemon_binding(
        &root.join(crate::config::REPO_POLICY_LOCAL_FILE_NAME),
        &root.join(crate::config::BITLOOPS_CONFIG_RELATIVE_PATH),
    )
    .expect("write repo daemon binding");
}

fn seed_local_sources(root: &Path) -> (PathBuf, PathBuf) {
    use crate::host::interactions::interaction_repository::create_interaction_repository;
    use crate::host::interactions::store::InteractionEventRepository;

    let sqlite_path = root.join("stores").join("relational").join("devql.sqlite");
    let duckdb_path = root.join("stores").join("events").join("events.duckdb");
    std::fs::create_dir_all(sqlite_path.parent().expect("sqlite parent"))
        .expect("create sqlite dir");
    std::fs::create_dir_all(duckdb_path.parent().expect("duckdb parent"))
        .expect("create duckdb dir");

    let sqlite = rusqlite::Connection::open(&sqlite_path).expect("open sqlite");
    sqlite
        .execute_batch(
            "
            CREATE TABLE repositories (
                repo_id TEXT PRIMARY KEY,
                provider TEXT NOT NULL,
                organization TEXT NOT NULL,
                name TEXT NOT NULL,
                default_branch TEXT,
                metadata_json TEXT,
                created_at TEXT
            );
            CREATE TABLE repo_sync_state (
                repo_id TEXT PRIMARY KEY,
                repo_root TEXT NOT NULL,
                active_branch TEXT,
                head_commit_sha TEXT,
                head_tree_sha TEXT,
                parser_version TEXT NOT NULL,
                extractor_version TEXT NOT NULL,
                scope_exclusions_fingerprint TEXT,
                last_sync_started_at TEXT,
                last_sync_completed_at TEXT,
                last_sync_status TEXT,
                last_sync_reason TEXT
            );
            CREATE TABLE current_file_state (
                repo_id TEXT NOT NULL,
                path TEXT NOT NULL,
                analysis_mode TEXT NOT NULL,
                file_role TEXT NOT NULL,
                text_index_mode TEXT NOT NULL,
                language TEXT NOT NULL,
                resolved_language TEXT NOT NULL,
                dialect TEXT,
                primary_context_id TEXT,
                secondary_context_ids_json TEXT NOT NULL,
                frameworks_json TEXT NOT NULL,
                runtime_profile TEXT,
                classification_reason TEXT NOT NULL,
                context_fingerprint TEXT,
                extraction_fingerprint TEXT NOT NULL,
                head_content_id TEXT,
                index_content_id TEXT,
                worktree_content_id TEXT,
                effective_content_id TEXT NOT NULL,
                effective_source TEXT NOT NULL,
                parser_version TEXT NOT NULL,
                extractor_version TEXT NOT NULL,
                exists_in_head INTEGER NOT NULL,
                exists_in_index INTEGER NOT NULL,
                exists_in_worktree INTEGER NOT NULL,
                last_synced_at TEXT NOT NULL,
                PRIMARY KEY (repo_id, path)
            );",
        )
        .expect("create sqlite analytics tables");
    sqlite
        .execute(
            "INSERT INTO repositories (repo_id, provider, organization, name, default_branch, metadata_json, created_at)
             VALUES (?1, 'local', 'bitloops', 'analytics', 'main', '{}', '2026-04-22T09:00:00Z')",
            rusqlite::params!["repo-analytics"],
        )
        .expect("insert repositories row");
    sqlite
        .execute(
            "INSERT INTO repo_sync_state (repo_id, repo_root, active_branch, head_commit_sha, head_tree_sha, parser_version, extractor_version, scope_exclusions_fingerprint, last_sync_started_at, last_sync_completed_at, last_sync_status, last_sync_reason)
             VALUES (?1, ?2, 'main', 'abc', 'def', '1', '1', '', '2026-04-22T09:00:00Z', '2026-04-22T09:05:00Z', 'completed', '')",
            rusqlite::params!["repo-analytics", root.to_string_lossy().to_string()],
        )
        .expect("insert repo_sync_state row");
    sqlite
        .execute(
            "INSERT INTO current_file_state (
                repo_id, path, analysis_mode, file_role, text_index_mode, language, resolved_language, dialect,
                primary_context_id, secondary_context_ids_json, frameworks_json, runtime_profile, classification_reason,
                context_fingerprint, extraction_fingerprint, head_content_id, index_content_id, worktree_content_id,
                effective_content_id, effective_source, parser_version, extractor_version, exists_in_head, exists_in_index,
                exists_in_worktree, last_synced_at
             ) VALUES (
                'repo-analytics', 'src/lib.rs', 'code', 'source_code', 'none', 'rust', 'rust', '', '', '[]', '[]', '',
                'seeded', '', 'fingerprint-1', 'head-1', 'index-1', 'worktree-1', 'effective-1', 'worktree', '1', '1', 1, 1, 1,
                '2026-04-22T09:04:00Z'
             )",
            [],
        )
        .expect("insert current_file_state row");

    let events_cfg = crate::config::EventsBackendConfig {
        duckdb_path: Some(duckdb_path.to_string_lossy().to_string()),
        clickhouse_url: None,
        clickhouse_user: None,
        clickhouse_password: None,
        clickhouse_database: None,
    };
    let repository = create_interaction_repository(&events_cfg, root, "repo-analytics".to_string())
        .expect("create interaction repository");
    repository
        .upsert_session(&crate::host::interactions::types::InteractionSession {
            session_id: "session-1".to_string(),
            repo_id: "repo-analytics".to_string(),
            branch: "main".to_string(),
            actor_id: "actor-1".to_string(),
            actor_name: "Alice".to_string(),
            actor_email: "alice@example.com".to_string(),
            actor_source: "seed".to_string(),
            agent_type: "codex".to_string(),
            model: "gpt-5.4".to_string(),
            first_prompt: "Inspect analytics".to_string(),
            transcript_path: "/tmp/transcript.jsonl".to_string(),
            worktree_path: root.to_string_lossy().to_string(),
            worktree_id: "worktree-1".to_string(),
            started_at: "2026-04-22T09:00:00Z".to_string(),
            ended_at: None,
            last_event_at: "2026-04-22T09:03:00Z".to_string(),
            updated_at: "2026-04-22T09:03:00Z".to_string(),
        })
        .expect("upsert analytics session");
    repository
        .upsert_turn(&crate::host::interactions::types::InteractionTurn {
            turn_id: "turn-1".to_string(),
            session_id: "session-1".to_string(),
            repo_id: "repo-analytics".to_string(),
            branch: "main".to_string(),
            actor_id: "actor-1".to_string(),
            actor_name: "Alice".to_string(),
            actor_email: "alice@example.com".to_string(),
            actor_source: "seed".to_string(),
            turn_number: 1,
            prompt: "Inspect analytics".to_string(),
            agent_type: "codex".to_string(),
            model: "gpt-5.4".to_string(),
            started_at: "2026-04-22T09:00:00Z".to_string(),
            ended_at: Some("2026-04-22T09:03:00Z".to_string()),
            token_usage: Some(
                crate::host::checkpoints::strategy::manual_commit::TokenUsageMetadata {
                    input_tokens: 120,
                    output_tokens: 80,
                    cache_creation_tokens: 0,
                    cache_read_tokens: 0,
                    api_call_count: 1,
                    subagent_tokens: None,
                },
            ),
            summary: "Analysed tool usage".to_string(),
            prompt_count: 1,
            transcript_offset_start: Some(0),
            transcript_offset_end: Some(100),
            transcript_fragment: "fragment".to_string(),
            files_modified: vec!["src/lib.rs".to_string()],
            checkpoint_id: None,
            updated_at: "2026-04-22T09:03:00Z".to_string(),
        })
        .expect("upsert analytics turn");
    repository
        .append_event(&crate::host::interactions::types::InteractionEvent {
            event_id: "event-1".to_string(),
            session_id: "session-1".to_string(),
            turn_id: Some("turn-1".to_string()),
            repo_id: "repo-analytics".to_string(),
            branch: "main".to_string(),
            actor_id: "actor-1".to_string(),
            actor_name: "Alice".to_string(),
            actor_email: "alice@example.com".to_string(),
            actor_source: "seed".to_string(),
            event_type:
                crate::host::interactions::types::InteractionEventType::ToolInvocationObserved,
            event_time: "2026-04-22T09:01:00Z".to_string(),
            source: "transcript_derivation".to_string(),
            sequence_number: 1,
            agent_type: "codex".to_string(),
            model: "gpt-5.4".to_string(),
            tool_use_id: "toolu-1".to_string(),
            tool_kind: "bash".to_string(),
            task_description: "rg tool analytics".to_string(),
            subagent_id: String::new(),
            payload: json!({
                "tool_name": "bash",
                "input_summary": "rg tool analytics",
                "command": "rg tool analytics",
                "command_binary": "rg",
                "command_argv": ["rg", "tool", "analytics"],
                "transcript_path": "/tmp/transcript.jsonl"
            }),
        })
        .expect("append tool invocation event");
    repository
        .append_event(&crate::host::interactions::types::InteractionEvent {
            event_id: "event-2".to_string(),
            session_id: "session-1".to_string(),
            turn_id: Some("turn-1".to_string()),
            repo_id: "repo-analytics".to_string(),
            branch: "main".to_string(),
            actor_id: "actor-1".to_string(),
            actor_name: "Alice".to_string(),
            actor_email: "alice@example.com".to_string(),
            actor_source: "seed".to_string(),
            event_type: crate::host::interactions::types::InteractionEventType::ToolResultObserved,
            event_time: "2026-04-22T09:02:00Z".to_string(),
            source: "transcript_derivation".to_string(),
            sequence_number: 2,
            agent_type: "codex".to_string(),
            model: "gpt-5.4".to_string(),
            tool_use_id: "toolu-1".to_string(),
            tool_kind: "bash".to_string(),
            task_description: "rg tool analytics".to_string(),
            subagent_id: String::new(),
            payload: json!({
                "tool_name": "bash",
                "output_summary": "2 matches",
                "command": "rg tool analytics",
                "command_binary": "rg",
                "command_argv": ["rg", "tool", "analytics"],
                "transcript_path": "/tmp/transcript.jsonl"
            }),
        })
        .expect("append tool result event");

    (sqlite_path, duckdb_path)
}

#[test]
fn validate_analytics_sql_accepts_single_select_and_with() {
    assert!(validate_analytics_sql("SELECT * FROM analytics.repositories").is_ok());
    assert!(validate_analytics_sql("WITH t AS (SELECT 1) SELECT * FROM t").is_ok());
}

#[test]
fn validate_analytics_sql_rejects_multi_statement_and_blocked_keywords() {
    let err = validate_analytics_sql("SELECT 1; SELECT 2").expect_err("should reject");
    assert!(err.to_string().contains("exactly one statement"));

    let err = validate_analytics_sql("CREATE TABLE nope AS SELECT 1")
        .expect_err("should reject blocked keyword");
    assert!(err.to_string().contains("SELECT or WITH") || err.to_string().contains("read-only"));
}

#[tokio::test]
async fn analytics_query_returns_curated_views_and_shell_commands() {
    let temp = TempDir::new().expect("temp dir");
    let (sqlite_path, duckdb_path) = seed_local_sources(temp.path());
    write_test_config(temp.path(), &sqlite_path, &duckdb_path);
    let cfg = sample_cfg(temp.path());

    let result = execute_analytics_sql(
        &cfg,
        AnalyticsRepoScope::CurrentRepo,
        "SELECT repo_id, path FROM analytics.current_file_state",
    )
    .await
    .expect("execute analytics sql");

    assert_eq!(result.row_count, 1);
    assert_eq!(
        result.rows.as_array().expect("rows")[0]["path"],
        Value::from("src/lib.rs")
    );

    let shell = execute_analytics_sql(
        &cfg,
        AnalyticsRepoScope::CurrentRepo,
        "SELECT command_binary, command FROM analytics.shell_commands",
    )
    .await
    .expect("execute shell query");
    assert_eq!(shell.row_count, 1);
    assert_eq!(
        shell.rows.as_array().expect("shell rows")[0]["command_binary"],
        Value::from("rg")
    );
}
