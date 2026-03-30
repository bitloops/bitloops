use super::*;
use tempfile::TempDir;

fn git_ok(repo_root: &std::path::Path, args: &[&str]) -> String {
    run_git(repo_root, args).unwrap_or_else(|err| panic!("git {:?} failed: {err}", args))
}

fn seed_repo_with_single_commit(message: &str) -> (TempDir, String) {
    let dir = tempfile::tempdir().expect("temp dir");
    git_ok(dir.path(), &["init"]);
    git_ok(dir.path(), &["checkout", "-B", "main"]);
    git_ok(dir.path(), &["config", "user.name", "Explain Test"]);
    git_ok(
        dir.path(),
        &["config", "user.email", "explain-test@example.com"],
    );
    std::fs::write(dir.path().join("README.md"), "seed").expect("write readme");
    git_ok(dir.path(), &["add", "README.md"]);
    git_ok(dir.path(), &["commit", "-m", message]);
    let head_sha = git_ok(dir.path(), &["rev-parse", "HEAD"]);
    (dir, head_sha)
}

fn insert_commit_checkpoint_mapping(
    repo_root: &std::path::Path,
    commit_sha: &str,
    checkpoint_id: &str,
) {
    let sqlite_path = checkpoint_sqlite_path(repo_root);
    let sqlite =
        crate::storage::SqliteConnectionPool::connect(sqlite_path).expect("open checkpoint sqlite");
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
        .expect("insert commit checkpoint mapping");
}

fn checkpoint_sqlite_path(repo_root: &std::path::Path) -> std::path::PathBuf {
    let cfg = crate::config::resolve_store_backend_config_for_repo(repo_root)
        .expect("resolve backend config");
    if let Some(path) = cfg.relational.sqlite_path.as_deref() {
        crate::config::resolve_sqlite_db_path_for_repo(repo_root, Some(path))
            .expect("resolve configured sqlite path")
    } else {
        crate::utils::paths::default_relational_db_path(repo_root)
    }
}

fn ensure_checkpoint_schema(repo_root: &std::path::Path) {
    let sqlite_path = checkpoint_sqlite_path(repo_root);
    let sqlite =
        crate::storage::SqliteConnectionPool::connect(sqlite_path).expect("open checkpoint sqlite");
    sqlite
        .initialise_checkpoint_schema()
        .expect("initialise checkpoint schema");
}

fn insert_committed_checkpoint_row(repo_root: &std::path::Path, checkpoint_id: &str) {
    ensure_checkpoint_schema(repo_root);
    let sqlite_path = checkpoint_sqlite_path(repo_root);
    let sqlite =
        crate::storage::SqliteConnectionPool::connect(sqlite_path).expect("open checkpoint sqlite");
    let repo_id = crate::host::devql::resolve_repo_id(repo_root).expect("resolve repo id");

    sqlite
        .with_connection(|conn| {
            conn.execute(
                "INSERT INTO checkpoints (
                    checkpoint_id, repo_id, strategy, branch, cli_version,
                    files_touched, checkpoints_count
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                rusqlite::params![
                    checkpoint_id,
                    repo_id.as_str(),
                    "manual-commit",
                    "",
                    "0.0.3",
                    "[]",
                    1_i64,
                ],
            )?;
            Ok(())
        })
        .expect("insert committed checkpoint row");
}

#[test]
fn agent_type_from_str_maps_codex_to_codex() {
    assert_eq!(
        agent_type_from_str(crate::adapters::agents::AGENT_TYPE_CODEX),
        AgentType::Codex
    );
}

#[test]
fn metadata_from_json_sets_codex_agent_type() {
    let meta = serde_json::json!({
        "session_id": "session-1",
        "created_at": "2026-03-12T00:00:00Z",
        "files_touched": ["src/main.rs"],
        "checkpoints_count": 1,
        "checkpoint_transcript_start": 0,
        "agent": crate::adapters::agents::AGENT_TYPE_CODEX,
    });

    let parsed = metadata_from_json(&meta, "cp-1");

    assert_eq!(parsed.agent_type, AgentType::Codex);
}

#[test]
fn metadata_from_json_unknown_agent_defaults_to_claude() {
    let meta = serde_json::json!({
        "agent": "unknown-agent"
    });

    let parsed = metadata_from_json(&meta, "cp-2");

    assert_eq!(parsed.agent_type, AgentType::ClaudeCode);
}

#[test]
fn build_commit_graph_from_git_reads_checkpoint_from_db_mapping() {
    let checkpoint_id = "aabbccddeeff";
    let (repo, commit_sha) = seed_repo_with_single_commit("checkpoint via DB");
    insert_commit_checkpoint_mapping(repo.path(), &commit_sha, checkpoint_id);

    let commits = build_commit_graph_from_git(repo.path(), 50).expect("build commit graph");
    let associated =
        get_associated_commits(&commits, checkpoint_id, true).expect("resolve associated commits");

    assert_eq!(associated.len(), 1, "expected commit mapped from SQLite");
    assert_eq!(associated[0].sha, commit_sha);
    assert_eq!(associated[0].message, "checkpoint via DB");
}

#[test]
fn token_usage_json_has_values_detects_nested_subagent_usage() {
    let nested = serde_json::json!({
        "input_tokens": 0,
        "output_tokens": 0,
        "subagent_tokens": {
            "input_tokens": 3
        }
    });
    assert!(token_usage_json_has_values(&nested));

    let empty = serde_json::json!({
        "input_tokens": 0,
        "output_tokens": 0,
        "cache_creation_tokens": 0,
        "cache_read_tokens": 0,
        "api_call_count": 0
    });
    assert!(!token_usage_json_has_values(&empty));
}

#[test]
fn token_usage_metadata_has_values_detects_nested_subagent_usage() {
    let nested = crate::host::checkpoints::strategy::manual_commit::TokenUsageMetadata {
        subagent_tokens: Some(Box::new(
            crate::host::checkpoints::strategy::manual_commit::TokenUsageMetadata {
                output_tokens: 4,
                ..Default::default()
            },
        )),
        ..Default::default()
    };
    assert!(token_usage_metadata_has_values(&nested));

    let empty = crate::host::checkpoints::strategy::manual_commit::TokenUsageMetadata::default();
    assert!(!token_usage_metadata_has_values(&empty));
}

#[test]
fn parse_summary_details_handles_non_objects_and_full_payloads() {
    assert!(parse_summary_details(&serde_json::Value::Null).is_none());

    let parsed = parse_summary_details(&serde_json::json!({
        "intent": "Investigate a failing test",
        "outcome": "Fixed the flaky branch",
        "learnings": {
            "repo": ["tests live under cli/"],
            "workflow": ["rerun the filtered suite first"],
            "code": [{
                "path": "src/main.rs",
                "line": 41,
                "end_line": 43,
                "finding": "missing guard"
            }]
        },
        "friction": ["slow git operations"],
        "open_items": ["add more branch coverage"]
    }))
    .expect("object payload should parse");

    assert_eq!(parsed.intent, "Investigate a failing test");
    assert_eq!(parsed.outcome, "Fixed the flaky branch");
    assert_eq!(parsed.repo_learnings, vec!["tests live under cli/"]);
    assert_eq!(
        parsed.workflow_learnings,
        vec!["rerun the filtered suite first"]
    );
    assert_eq!(parsed.code_learnings.len(), 1);
    assert_eq!(parsed.code_learnings[0].path, "src/main.rs");
    assert_eq!(parsed.friction, vec!["slow git operations"]);
    assert_eq!(parsed.open_items, vec!["add more branch coverage"]);
}

#[test]
fn metadata_from_json_parses_summary_and_nested_token_usage() {
    let meta = serde_json::json!({
        "session_id": "session-1",
        "created_at": "2026-03-12T00:00:00Z",
        "files_touched": ["src/main.rs"],
        "checkpoints_count": 2,
        "checkpoint_transcript_start": 7,
        "agent": crate::adapters::agents::AGENT_TYPE_OPEN_CODE,
        "summary": {
            "intent": "Ship fix",
            "outcome": "Done",
            "learnings": {
                "repo": ["cli tests are module-local"]
            }
        },
        "token_usage": {
            "subagent_tokens": {
                "output_tokens": 5
            }
        }
    });

    let parsed = metadata_from_json(&meta, "cp-1");

    assert_eq!(parsed.checkpoint_id, "cp-1");
    assert_eq!(parsed.session_id, "session-1");
    assert_eq!(parsed.files_touched, vec!["src/main.rs"]);
    assert_eq!(parsed.checkpoints_count, 2);
    assert_eq!(parsed.checkpoint_transcript_start, 7);
    assert!(parsed.has_token_usage);
    assert_eq!(parsed.token_input, 0);
    assert_eq!(parsed.token_output, 0);
    assert_eq!(parsed.agent_type, AgentType::OpenCode);
    assert_eq!(
        parsed.summary.expect("summary expected").repo_learnings,
        vec!["cli tests are module-local"]
    );
}

#[test]
fn build_commit_graph_from_git_respects_limit() {
    let (repo, _) = seed_repo_with_single_commit("first commit");
    ensure_checkpoint_schema(repo.path());
    std::fs::write(repo.path().join("second.txt"), "second").expect("write second file");
    git_ok(repo.path(), &["add", "second.txt"]);
    git_ok(repo.path(), &["commit", "-m", "second commit"]);

    let commits = build_commit_graph_from_git(repo.path(), 1).expect("build commit graph");
    assert_eq!(commits.len(), 1);
    assert_eq!(commits[0].message, "second commit");
}

#[test]
fn run_explain_checkpoint_in_rejects_empty_prefix_and_generate_without_committed_checkpoint() {
    let repo = tempfile::tempdir().expect("temp dir");

    let err = run_explain_checkpoint_in(repo.path(), "", &ExplainExecutionOptions::default())
        .expect_err("empty prefix must fail");
    assert!(format!("{err:#}").contains("checkpoint not found"));

    let seeded = seed_repo_with_single_commit("no checkpoints here").0;
    ensure_checkpoint_schema(seeded.path());
    let err = run_explain_checkpoint_in(
        seeded.path(),
        "temp-123",
        &ExplainExecutionOptions {
            generate: true,
            ..Default::default()
        },
    )
    .expect_err("temporary checkpoint generate path must fail");
    assert!(format!("{err:#}").contains("only committed checkpoints supported"));
}

#[test]
fn run_explain_checkpoint_in_rejects_ambiguous_prefixes() {
    let (repo, _) = seed_repo_with_single_commit("checkpoint ambiguity");
    insert_committed_checkpoint_row(repo.path(), "abc123aaaaaa");
    insert_committed_checkpoint_row(repo.path(), "abc123bbbbbb");

    let err = run_explain_checkpoint_in(repo.path(), "abc123", &ExplainExecutionOptions::default())
        .expect_err("ambiguous prefix must fail");
    assert!(format!("{err:#}").contains("ambiguous checkpoint prefix"));
}

#[test]
fn generate_checkpoint_summary_validates_preconditions() {
    let repo = tempfile::tempdir().expect("temp dir");
    let content = SessionContent::default();

    let err = generate_checkpoint_summary(repo.path(), "", &content, false)
        .expect_err("missing checkpoint id must fail");
    assert!(format!("{err:#}").contains("checkpoint id is required"));

    let err = generate_checkpoint_summary(repo.path(), "cp-1", &content, false)
        .expect_err("missing transcript must fail");
    assert!(format!("{err:#}").contains("has no transcript to summarize"));

    let summary_content = SessionContent {
        metadata: CheckpointMetadata {
            summary: Some(SummaryDetails {
                intent: "Already summarised".to_string(),
                ..Default::default()
            }),
            ..Default::default()
        },
        transcript: b"{\"type\":\"user\"}\n".to_vec(),
        ..Default::default()
    };
    let err = generate_checkpoint_summary(repo.path(), "cp-1", &summary_content, false)
        .expect_err("existing summary without --force must fail");
    assert!(format!("{err:#}").contains("already has a summary"));
}

#[test]
fn generate_checkpoint_summary_rejects_empty_scoped_transcript() {
    let repo = tempfile::tempdir().expect("temp dir");
    let content = SessionContent {
        metadata: CheckpointMetadata {
            checkpoint_transcript_start: 5,
            agent_type: AgentType::ClaudeCode,
            ..Default::default()
        },
        transcript: b"{\"type\":\"user\",\"message\":{\"content\":\"hello\"}}\n".to_vec(),
        ..Default::default()
    };

    let err = generate_checkpoint_summary(repo.path(), "cp-1", &content, true)
        .expect_err("empty scoped transcript must fail");
    assert!(format!("{err:#}").contains("has no transcript content for this checkpoint"));
}
