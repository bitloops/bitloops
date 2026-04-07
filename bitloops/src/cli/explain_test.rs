#![allow(non_snake_case)]

use crate::cli::explain::*;
use crate::host::checkpoints::checkpoint_id::CHECKPOINT_KEY;
use crate::test_support::process_state::git_command;
use anyhow::{Result, anyhow};
use clap::{Arg, ArgAction, Command};
use std::collections::HashMap;
use std::path::PathBuf;

fn setup_git_repo() -> (tempfile::TempDir, std::path::PathBuf) {
    setup_git_repo_with_initial_branch("main")
}

fn setup_git_repo_with_initial_branch(
    initial_branch: &str,
) -> (tempfile::TempDir, std::path::PathBuf) {
    let tmp = tempfile::TempDir::new().unwrap();
    let root = tmp.path().to_path_buf();
    run_git_cmd(&root, &["init"]);
    run_git_cmd(&root, &["checkout", "-B", initial_branch]);
    run_git_cmd(&root, &["config", "user.name", "Test"]);
    run_git_cmd(&root, &["config", "user.email", "test@example.com"]);
    run_git_cmd(&root, &["config", "commit.gpgsign", "false"]);
    crate::test_support::git_fixtures::write_test_daemon_config(&root);
    ensure_relational_store_file(&root);
    (tmp, root)
}

fn run_git_cmd(root: &std::path::Path, args: &[&str]) -> String {
    let output = git_command()
        .args(args)
        .current_dir(root)
        .output()
        .expect("failed to run git command");
    assert!(
        output.status.success(),
        "git {:?} failed:\nstdout:\n{}\nstderr:\n{}",
        args,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .expect("git stdout should be utf-8")
        .trim()
        .to_string()
}

fn make_commit(root: &std::path::Path, file: &str, content: &str, msg: &str) -> String {
    std::fs::write(root.join(file), content).unwrap();
    run_git_cmd(root, &["add", file]);
    run_git_cmd(root, &["commit", "-m", msg]);
    run_git_cmd(root, &["rev-parse", "HEAD"])
}

/// Creates a commit and returns its SHA. Does NOT insert DB mappings — callers must
/// call `insert_commit_checkpoint_mapping` + `insert_committed_checkpoint_row` after
/// any branch-switching operations (which may destroy untracked DB files).
fn make_checkpoint_commit(
    root: &std::path::Path,
    file: &str,
    content: &str,
    message: &str,
    _checkpoint_id: &str,
) -> String {
    make_commit(root, file, content, message)
}

fn checkpoint_sqlite_path(repo_root: &std::path::Path) -> PathBuf {
    let cfg = crate::config::resolve_store_backend_config_for_repo(repo_root)
        .expect("resolve backend config");
    let path = cfg
        .relational
        .sqlite_path
        .as_deref()
        .expect("test daemon config should set sqlite_path");
    crate::config::resolve_sqlite_db_path_for_repo(repo_root, Some(path))
        .expect("resolve configured sqlite path")
}

fn ensure_relational_store_file(repo_root: &std::path::Path) {
    let sqlite_path = checkpoint_sqlite_path(repo_root);
    let sqlite =
        crate::storage::SqliteConnectionPool::connect(sqlite_path).expect("connect sqlite");
    sqlite
        .initialise_checkpoint_schema()
        .expect("initialise checkpoint schema");
}

fn insert_commit_checkpoint_mapping(
    repo_root: &std::path::Path,
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
        .expect("insert commit checkpoint mapping");
}

fn insert_committed_checkpoint_row(repo_root: &std::path::Path, checkpoint_id: &str) {
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
                "INSERT INTO checkpoints (
                    checkpoint_id, repo_id, strategy, branch, cli_version,
                    checkpoints_count
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                rusqlite::params![
                    checkpoint_id,
                    repo_id.as_str(),
                    "manual-commit",
                    "",
                    "0.0.3",
                    1_i64,
                ],
            )?;
            Ok(())
        })
        .expect("insert committed checkpoint row");
}

fn write_committed_checkpoint_metadata_with_mappings(
    root: &std::path::Path,
    checkpoints: &[(&str, &str)], // (checkpoint_id, commit_sha)
) {
    let current_branch = run_git_cmd(root, &["rev-parse", "--abbrev-ref", "HEAD"]);
    run_git_cmd(root, &["checkout", "--orphan", "bitloops/checkpoints/v1"]);
    run_git_cmd(root, &["rm", "-rf", "--ignore-unmatch", "."]);

    let mut buckets_to_stage = std::collections::BTreeSet::new();
    for (checkpoint_id, _) in checkpoints {
        let bucket = &checkpoint_id[..2];
        let suffix = &checkpoint_id[2..];
        let metadata_dir = root.join(bucket).join(suffix);
        std::fs::create_dir_all(&metadata_dir).expect("failed to create metadata dir");
        buckets_to_stage.insert(bucket.to_string());
        let metadata_path = metadata_dir.join("metadata.json");
        let metadata_json = serde_json::json!({
            "checkpoint_id": checkpoint_id,
            "strategy": "manual-commit",
            "checkpoints_count": 1,
            "files_touched": [],
            "sessions": []
        });
        std::fs::write(
            metadata_path,
            serde_json::to_string_pretty(&metadata_json).expect("serialize metadata"),
        )
        .expect("write metadata");
    }

    for bucket in buckets_to_stage {
        run_git_cmd(root, &["add", bucket.as_str()]);
    }
    run_git_cmd(root, &["commit", "-m", "seed checkpoint metadata"]);
    run_git_cmd(root, &["checkout", &current_branch]);
    crate::test_support::git_fixtures::write_test_daemon_config(root);
    ensure_relational_store_file(root);

    for (checkpoint_id, commit_sha) in checkpoints {
        insert_committed_checkpoint_row(root, checkpoint_id);
        if !commit_sha.is_empty() {
            insert_commit_checkpoint_mapping(root, commit_sha, checkpoint_id);
        }
    }
}

fn sample_opts() -> ExplainExecutionOptions {
    ExplainExecutionOptions {
        no_pager: true,
        verbose: true,
        full: false,
        raw_transcript: false,
        generate: false,
        force: false,
        search_all: false,
    }
}

fn has_flag(cmd: &Command, id: &str) -> bool {
    cmd.get_arguments()
        .any(|arg: &Arg| arg.get_id().as_str() == id)
}

// Test-only command builder used by CLI-843 argument parsing tests.
fn new_explain_command() -> Command {
    Command::new("explain")
        .about("Explain a session, commit, or checkpoint")
        .arg(
            Arg::new("session")
                .long("session")
                .value_name("SESSION_ID")
                .help("Filter checkpoints by session ID (or prefix)"),
        )
        .arg(
            Arg::new("commit")
                .long("commit")
                .value_name("COMMIT_REF")
                .help("Explain a specific commit"),
        )
        .arg(
            Arg::new("checkpoint")
                .long("checkpoint")
                .short('c')
                .value_name("CHECKPOINT_ID")
                .help("Explain a specific checkpoint"),
        )
        .arg(
            Arg::new("no-pager")
                .long("no-pager")
                .action(ArgAction::SetTrue)
                .help("Disable pager output"),
        )
        .arg(
            Arg::new("short")
                .long("short")
                .short('s')
                .action(ArgAction::SetTrue)
                .conflicts_with_all(["full", "raw-transcript"])
                .help("Show summary only"),
        )
        .arg(
            Arg::new("full")
                .long("full")
                .action(ArgAction::SetTrue)
                .conflicts_with_all(["short", "raw-transcript"])
                .help("Show full parsed transcript"),
        )
        .arg(
            Arg::new("raw-transcript")
                .long("raw-transcript")
                .action(ArgAction::SetTrue)
                .conflicts_with_all(["short", "full", "generate"])
                .help("Show raw transcript"),
        )
        .arg(
            Arg::new("generate")
                .long("generate")
                .action(ArgAction::SetTrue)
                .conflicts_with("raw-transcript")
                .help("Generate checkpoint summary"),
        )
        .arg(
            Arg::new("force")
                .long("force")
                .action(ArgAction::SetTrue)
                .help("Force regenerate summary"),
        )
        .arg(
            Arg::new("search-all")
                .long("search-all")
                .action(ArgAction::SetTrue)
                .help("Search all commits in the DAG"),
        )
}

fn validate_no_positional_args(args: &[&str]) -> Result<()> {
    let expects_value = ["--session", "--commit", "--checkpoint", "-c"];
    let mut skip_next = false;

    for arg in args {
        if skip_next {
            skip_next = false;
            continue;
        }

        if expects_value.contains(arg) {
            skip_next = true;
            continue;
        }

        if arg.starts_with("--session=")
            || arg.starts_with("--commit=")
            || arg.starts_with("--checkpoint=")
            || arg.starts_with('-')
        {
            continue;
        }

        return Err(anyhow!(format!(
            "unexpected argument {arg:?}\nHint: use --checkpoint, --session, or --commit to specify what to explain"
        )));
    }

    Ok(())
}

fn execute_explain_args_for_test(args: &[&str]) -> Result<()> {
    validate_no_positional_args(args)?;

    let mut argv = vec!["explain"];
    argv.extend_from_slice(args);

    new_explain_command()
        .try_get_matches_from(argv)
        .map(|_| ())
        .map_err(|err| anyhow!(err.to_string()))
}

fn run_explain_branch_default_for_test(
    repo_root: &std::path::Path,
    no_pager: bool,
) -> Result<String> {
    run_explain_branch_with_filter_in(repo_root, "", no_pager)
}

fn sample_summary() -> CheckpointSummary {
    CheckpointSummary {
        checkpoint_id: "abc123def456".to_string(),
        checkpoints_count: 3,
        files_touched: vec!["main.rs".to_string(), "util.rs".to_string()],
        has_token_usage: true,
        token_input: 10_000,
        token_output: 5_000,
    }
}

fn sample_content() -> SessionContent {
    SessionContent {
        metadata: CheckpointMetadata {
            checkpoint_id: "abc123def456".to_string(),
            session_id: "2026-01-21-test-session".to_string(),
            created_at: "2026-01-21 10:30".to_string(),
            files_touched: vec!["main.rs".to_string(), "util.rs".to_string()],
            checkpoints_count: 3,
            checkpoint_transcript_start: 0,
            token_input: 10_000,
            token_output: 5_000,
            summary: None,
            ..CheckpointMetadata::default()
        },
        prompts: "Add a new feature".to_string(),
        transcript: Vec::new(),
    }
}

fn checkpoint_detail(index: usize, short_id: &str, message: &str) -> CheckpointDetail {
    CheckpointDetail {
        index,
        short_id: short_id.to_string(),
        timestamp: "2025-12-10 14:35".to_string(),
        is_task_checkpoint: false,
        message: message.to_string(),
        interactions: Vec::new(),
        files: Vec::new(),
    }
}

// CLI-842 --------------------------------------------------------------------

#[test]
fn TestFormatSessionInfo_CheckpointNumberingReversed() {
    let session = SessionInfo {
        id: "2025-12-09-test-session".to_string(),
        strategy: "auto-commit".to_string(),
        ..SessionInfo::default()
    };

    let mut newest = checkpoint_detail(3, "ccc3333", "Third checkpoint");
    newest.interactions = vec![Interaction {
        prompt: "Latest change".to_string(),
        ..Interaction::default()
    }];

    let mut middle = checkpoint_detail(2, "bbb2222", "Second checkpoint");
    middle.interactions = vec![Interaction {
        prompt: "Middle change".to_string(),
        ..Interaction::default()
    }];

    let mut oldest = checkpoint_detail(1, "aaa1111", "First checkpoint");
    oldest.interactions = vec![Interaction {
        prompt: "Initial change".to_string(),
        ..Interaction::default()
    }];

    let output = format_session_info(&session, "", &[newest, middle, oldest]);

    let idx3 = output.find("Checkpoint 3").unwrap_or(usize::MAX);
    let idx2 = output.find("Checkpoint 2").unwrap_or(usize::MAX);
    let idx1 = output.find("Checkpoint 1").unwrap_or(usize::MAX);

    assert!(
        idx3 < idx2 && idx2 < idx1,
        "expected 3,2,1 ordering in output: {output}"
    );
    assert!(
        output.contains("Latest change"),
        "expected newest prompt in output: {output}"
    );
    assert!(
        output.contains("Initial change"),
        "expected oldest prompt in output: {output}"
    );
}

#[test]
fn TestFormatSessionInfo_EmptyCheckpoints() {
    let session = SessionInfo {
        id: "2025-12-09-empty-session".to_string(),
        strategy: "manual-commit".to_string(),
        ..SessionInfo::default()
    };

    let output = format_session_info(&session, "", &[]);

    assert!(
        output.contains("Checkpoints: 0"),
        "expected checkpoints count in output: {output}"
    );
}

#[test]
fn TestFormatSessionInfo_CheckpointWithTaskMarker() {
    let session = SessionInfo {
        id: "2025-12-09-task-session".to_string(),
        strategy: "auto-commit".to_string(),
        ..SessionInfo::default()
    };

    let mut cp = checkpoint_detail(1, "abc1234", "Task checkpoint");
    cp.is_task_checkpoint = true;
    cp.interactions = vec![Interaction {
        prompt: "Run tests".to_string(),
        ..Interaction::default()
    }];

    let output = format_session_info(&session, "", &[cp]);

    assert!(
        output.contains("[Task]"),
        "expected task marker in output: {output}"
    );
}

#[test]
fn TestFormatSessionInfo_CheckpointWithDate() {
    let session = SessionInfo {
        id: "2025-12-10-dated-session".to_string(),
        strategy: "auto-commit".to_string(),
        ..SessionInfo::default()
    };

    let cp = checkpoint_detail(1, "abc1234", "Test checkpoint");
    let output = format_session_info(&session, "", &[cp]);

    assert!(
        output.contains("2025-12-10 14:35"),
        "expected formatted date in output: {output}"
    );
}

#[test]
fn TestFormatSessionInfo_ShowsMessageWhenNoInteractions() {
    let session = SessionInfo {
        id: "2025-12-12-incremental-session".to_string(),
        strategy: "auto-commit".to_string(),
        ..SessionInfo::default()
    };

    let mut cp = checkpoint_detail(
        1,
        "abc1234",
        "Starting 'dev' agent: Implement feature X (toolu_01ABC)",
    );
    cp.is_task_checkpoint = true;

    let output = format_session_info(&session, "", &[cp]);

    assert!(
        output.contains("Starting 'dev' agent: Implement feature X (toolu_01ABC)"),
        "expected checkpoint message in output: {output}"
    );
    assert!(
        !output.contains("## Prompt"),
        "did not expect prompt section without interactions: {output}"
    );
    assert!(
        !output.contains("## Responses"),
        "did not expect response section without interactions: {output}"
    );
}

#[test]
fn TestFormatSessionInfo_ShowsMessageAndFilesWhenNoInteractions() {
    let session = SessionInfo {
        id: "2025-12-12-incremental-with-files".to_string(),
        strategy: "auto-commit".to_string(),
        ..SessionInfo::default()
    };

    let mut cp = checkpoint_detail(1, "def5678", "Running tests for API endpoint (toolu_02DEF)");
    cp.is_task_checkpoint = true;
    cp.files = vec![
        "api/endpoint.rs".to_string(),
        "api/endpoint_test.rs".to_string(),
    ];

    let output = format_session_info(&session, "", &[cp]);

    assert!(
        output.contains("Running tests for API endpoint (toolu_02DEF)"),
        "expected message in output: {output}"
    );
    assert!(
        output.contains("Files Modified"),
        "expected files header in output: {output}"
    );
    assert!(
        output.contains("api/endpoint.rs"),
        "expected modified file in output: {output}"
    );
}

#[test]
fn TestFormatSessionInfo_DoesNotShowMessageWhenHasInteractions() {
    let session = SessionInfo {
        id: "2025-12-12-full-checkpoint".to_string(),
        strategy: "auto-commit".to_string(),
        ..SessionInfo::default()
    };

    let mut cp = checkpoint_detail(
        1,
        "ghi9012",
        "Completed 'dev' agent: Implement feature (toolu_03GHI)",
    );
    cp.is_task_checkpoint = true;
    cp.interactions = vec![Interaction {
        prompt: "Implement the feature".to_string(),
        responses: vec!["I've implemented the feature by...".to_string()],
        files: vec!["feature.rs".to_string()],
    }];

    let output = format_session_info(&session, "", &[cp]);

    assert!(
        output.contains("Implement the feature"),
        "expected prompt text in output: {output}"
    );
    assert!(
        output.contains("I've implemented the feature by"),
        "expected response text in output: {output}"
    );
    assert!(
        output.contains("## Prompt"),
        "expected prompt header for interactive checkpoints: {output}"
    );
}

#[test]
fn TestFormatSessionInfo() {
    let session = SessionInfo {
        id: "2025-12-09-test-session-abc".to_string(),
        description: "Test description".to_string(),
        strategy: "manual-commit".to_string(),
        start_time: "2025-12-09 10:00:00".to_string(),
        checkpoints: vec![
            SessionCheckpoint {
                checkpoint_id: "abc1234567890".to_string(),
                message: "First checkpoint".to_string(),
                timestamp: "2025-12-09 09:00:00".to_string(),
            },
            SessionCheckpoint {
                checkpoint_id: "def0987654321".to_string(),
                message: "Second checkpoint".to_string(),
                timestamp: "2025-12-09 10:00:00".to_string(),
            },
        ],
    };

    let checkpoint_details = vec![
        CheckpointDetail {
            index: 1,
            short_id: "abc1234".to_string(),
            timestamp: "2025-12-09 09:00".to_string(),
            message: "First checkpoint".to_string(),
            interactions: vec![Interaction {
                prompt: "Fix the bug".to_string(),
                responses: vec!["Fixed the bug in auth module".to_string()],
                files: vec!["auth.rs".to_string()],
            }],
            files: vec!["auth.rs".to_string()],
            ..CheckpointDetail::default()
        },
        CheckpointDetail {
            index: 2,
            short_id: "def0987".to_string(),
            timestamp: "2025-12-09 10:00".to_string(),
            message: "Second checkpoint".to_string(),
            interactions: vec![Interaction {
                prompt: "Add tests".to_string(),
                responses: vec!["Added unit tests".to_string()],
                files: vec!["auth_test.rs".to_string()],
            }],
            files: vec!["auth_test.rs".to_string()],
            ..CheckpointDetail::default()
        },
    ];

    let output = format_session_info(&session, "", &checkpoint_details);

    assert!(
        output.contains("Session:"),
        "expected output to contain 'Session:'"
    );
    assert!(
        output.contains(&session.id),
        "expected output to contain session ID"
    );
    assert!(
        output.contains("Strategy:"),
        "expected output to contain 'Strategy:'"
    );
    assert!(
        output.contains("manual-commit"),
        "expected output to contain strategy name"
    );
    assert!(
        output.contains("Checkpoints: 2"),
        "expected output to contain 'Checkpoints: 2'"
    );
    assert!(
        output.contains("Checkpoint 1"),
        "expected output to contain 'Checkpoint 1'"
    );
    assert!(
        output.contains("## Prompt"),
        "expected output to contain '## Prompt'"
    );
    assert!(
        output.contains("## Responses"),
        "expected output to contain '## Responses'"
    );
    assert!(
        output.contains("Files Modified"),
        "expected output to contain 'Files Modified'"
    );
}

#[test]
fn TestFormatSessionInfo_WithSourceRef() {
    let session = SessionInfo {
        id: "2025-12-09-test-session-abc".to_string(),
        description: "Test description".to_string(),
        strategy: "auto-commit".to_string(),
        start_time: "2025-12-09 10:00:00".to_string(),
        checkpoints: vec![SessionCheckpoint {
            checkpoint_id: "abc1234567890".to_string(),
            message: "First checkpoint".to_string(),
            timestamp: "2025-12-09 10:00:00".to_string(),
        }],
    };

    let checkpoint_details = vec![CheckpointDetail {
        index: 1,
        short_id: "abc1234".to_string(),
        timestamp: "2025-12-09 10:00".to_string(),
        message: "First checkpoint".to_string(),
        ..CheckpointDetail::default()
    }];

    let source_ref = "bitloops/metadata@abc123def456";
    let output = format_session_info(&session, source_ref, &checkpoint_details);

    assert!(
        output.contains("Source Ref:"),
        "expected output to contain 'Source Ref:'"
    );
    assert!(
        output.contains(source_ref),
        "expected output to contain source ref {source_ref:?}, got:\n{output}"
    );
}

#[test]
fn TestStrategySessionSourceInterface() {
    let strategy = new_manual_commit_strategy();
    let source: &dyn SessionSource = &strategy;
    if let Err(err) = source.get_additional_sessions() {
        eprintln!("get_additional_sessions returned error: {err}");
    }
}

// CLI-843 --------------------------------------------------------------------

#[test]
fn TestNewExplainCmd() {
    let cmd = new_explain_command();

    assert_eq!(cmd.get_name(), "explain");

    let has_session = has_flag(&cmd, "session");
    let has_commit = has_flag(&cmd, "commit");
    let has_generate = has_flag(&cmd, "generate");
    let has_force = has_flag(&cmd, "force");

    assert!(has_session, "expected --session flag");
    assert!(has_commit, "expected --commit flag");
    assert!(has_generate, "expected --generate flag");
    assert!(has_force, "expected --force flag");
}

#[test]
fn TestExplainCmd_SearchAllFlag() {
    let matches = new_explain_command()
        .try_get_matches_from(["explain"])
        .expect("expected explain command to parse without args");

    assert!(
        !matches.get_flag("search-all"),
        "expected --search-all default false"
    );
}

#[test]
fn TestExplainCmd_RejectsPositionalArgs() {
    let cases = vec![
        vec!["abc123"],
        vec!["abc123", "--checkpoint", "def456"],
        vec!["--checkpoint", "def456", "abc123"],
    ];

    for args in cases {
        let err =
            execute_explain_args_for_test(&args).expect_err("expected positional args to fail");
        let msg = err.to_string();
        assert!(
            msg.contains("unexpected argument"),
            "expected unexpected argument error, got: {msg}"
        );
        assert!(msg.contains("Hint:"), "expected hint in error, got: {msg}");
    }
}

#[test]
fn TestExplainBothFlagsError() {
    let err = run_explain("session-id", "commit-sha", "", &sample_opts())
        .expect_err("expected mutual exclusivity error");
    assert!(
        err.to_string()
            .to_lowercase()
            .contains("cannot specify multiple"),
        "expected mutual exclusivity message, got: {err}"
    );
}

#[test]
fn TestExplainCmd_HasCheckpointFlag() {
    let cmd = new_explain_command();
    let has_flag = has_flag(&cmd, "checkpoint");
    assert!(has_flag, "expected --checkpoint flag to exist");
}

#[test]
fn TestExplainCmd_HasShortFlag() {
    let cmd = new_explain_command();
    let short_arg = cmd
        .get_arguments()
        .find(|arg: &&Arg| arg.get_id().as_str() == "short")
        .expect("expected short flag to exist");

    assert_eq!(short_arg.get_short(), Some('s'));
}

#[test]
fn TestExplainCmd_HasFullFlag() {
    let cmd = new_explain_command();
    let has_flag = has_flag(&cmd, "full");
    assert!(has_flag, "expected --full flag to exist");
}

#[test]
fn TestExplainCmd_HasRawTranscriptFlag() {
    let cmd = new_explain_command();
    let has_flag = has_flag(&cmd, "raw-transcript");
    assert!(has_flag, "expected --raw-transcript flag to exist");
}

#[test]
fn TestRunExplain_MutualExclusivityError() {
    let err = run_explain("session-id", "", "checkpoint-id", &sample_opts())
        .expect_err("expected mutual exclusivity error");
    assert!(
        err.to_string().contains("cannot specify multiple"),
        "expected mutual exclusivity message, got: {err}"
    );
}

#[test]
fn TestRunExplainCheckpoint_NotFound() {
    let (tmp, root) = setup_git_repo();
    make_commit(&root, "file.txt", "content", "init");
    let err = run_explain_checkpoint_in(&root, "nonexistent123", &sample_opts())
        .expect_err("expected error for nonexistent checkpoint");
    drop(tmp);
    assert!(
        err.to_string().contains("checkpoint not found"),
        "expected 'checkpoint not found' error, got: {err}"
    );
}

#[test]
fn TestRunExplain_SessionFlagFiltersListView() {
    let route = run_explain("some-session", "", "", &sample_opts())
        .expect("--session alone should route to list view");

    assert_eq!(
        route,
        ExplainRoute::BranchList {
            session_filter: Some("some-session".to_string())
        }
    );
}

#[test]
fn TestRunExplain_SessionWithCheckpointStillMutuallyExclusive() {
    let err = run_explain("some-session", "", "some-checkpoint", &sample_opts())
        .expect_err("expected error when --session and --checkpoint are both set");
    assert!(
        err.to_string().contains("cannot specify multiple"),
        "expected mutual exclusivity message, got: {err}"
    );
}

#[test]
fn TestRunExplain_SessionWithCommitStillMutuallyExclusive() {
    let err = run_explain("some-session", "some-commit", "", &sample_opts())
        .expect_err("expected error when --session and --commit are both set");
    assert!(
        err.to_string().contains("cannot specify multiple"),
        "expected mutual exclusivity message, got: {err}"
    );
}

// CLI-844 --------------------------------------------------------------------

#[test]
fn TestExplainCommit_NotFound() {
    let (tmp, root) = setup_git_repo();
    make_commit(&root, "file.txt", "content", "init");
    // "refs/heads/nosuchwhatsoever" is a branch ref that won't exist in the fresh repo.
    let err = run_explain_commit_in(
        &root,
        "refs/heads/nosuchwhatsoever",
        true,
        false,
        false,
        false,
    )
    .expect_err("expected not found error");
    drop(tmp);
    let msg = err.to_string();
    assert!(
        msg.contains("not found") || msg.contains("resolve") || msg.contains("unknown"),
        "expected not-found style error, got: {msg}"
    );
}

#[test]
fn TestExplainCommit_NoBitloopsData() {
    let (tmp, root) = setup_git_repo();
    make_commit(&root, "file.txt", "content", "feat: no checkpoint commit");

    let sha = run_git_cmd(&root, &["rev-parse", "HEAD"]);
    let output = run_explain_commit_in(&root, &sha, true, false, false, false)
        .expect("expected non-bitloops commits to be explainable");
    drop(tmp);

    assert!(
        output.contains("No associated Bitloops checkpoint"),
        "expected no-checkpoint message, got: {output}"
    );
}

#[test]
fn TestExplainCommit_WithoutCheckpointMapping() {
    let (tmp, root) = setup_git_repo();
    // Commit without a DB checkpoint mapping.
    make_commit(&root, "file.txt", "content", "feat: metadata only");

    let sha = run_git_cmd(&root, &["rev-parse", "HEAD"]);
    let output = run_explain_commit_in(&root, &sha, true, false, false, false)
        .expect("expected commit with no checkpoint mapping to succeed");
    drop(tmp);

    assert!(
        output.contains("No associated Bitloops checkpoint"),
        "expected no-checkpoint message, got: {output}"
    );
}

#[test]
fn TestExplainDefault_ShowsBranchView() {
    let (_tmp, root) = setup_git_repo();
    let output = run_explain_branch_default_for_test(&root, true)
        .expect("expected explain default to show branch view");

    assert!(
        output.contains("Branch:"),
        "expected 'Branch:' in output, got: {output}"
    );
    assert!(
        output.contains("Checkpoints:"),
        "expected 'Checkpoints:' in output, got: {output}"
    );
}

#[test]
fn TestExplainDefault_NoCheckpoints_ShowsHelpfulMessage() {
    let (_tmp, root) = setup_git_repo();
    let output = run_explain_branch_default_for_test(&root, true)
        .expect("expected explain default without checkpoints to succeed");

    assert!(
        output.contains("Checkpoints: 0"),
        "expected 'Checkpoints: 0' in output, got: {output}"
    );
    assert!(
        output.contains("Checkpoints will appear") && output.contains("agent session"),
        "expected helpful checkpoints message, got: {output}"
    );
}

#[test]
fn TestRunExplainCommit_NoCheckpointTrailer() {
    let (tmp, root) = setup_git_repo();
    make_commit(
        &root,
        "file.txt",
        "content",
        "feat: plain commit no checkpoint",
    );

    let sha = run_git_cmd(&root, &["rev-parse", "HEAD"]);
    let output = run_explain_commit_in(&root, &sha, false, false, false, false)
        .expect("expected explain commit to succeed without checkpoint mapping");
    drop(tmp);

    assert!(
        output.contains("No associated Bitloops checkpoint"),
        "expected no-checkpoint message, got: {output}"
    );
}

#[test]
fn TestRunExplainCommit_WithoutDbMapping() {
    let (tmp, root) = setup_git_repo();
    // Commit without a DB checkpoint mapping — explain should report no checkpoint.
    make_commit(&root, "file.txt", "content", "feat: add feature");

    let sha = run_git_cmd(&root, &["rev-parse", "HEAD"]);
    let output = run_explain_commit_in(&root, &sha, false, false, false, false)
        .expect("expected no-checkpoint output when commit mapping is missing");
    drop(tmp);

    assert!(
        output.contains("No associated Bitloops checkpoint"),
        "expected no-checkpoint message, got: {output}"
    );
}

// CLI-845 --------------------------------------------------------------------

#[test]
fn TestFormatCheckpointOutput_Short() {
    let summary = sample_summary();
    let content = sample_content();

    let output = format_checkpoint_output(
        &summary,
        &content,
        "abc123def456",
        None,
        &Author::default(),
        false,
        false,
    );

    assert!(
        output.contains("abc123def456"),
        "expected checkpoint id in output: {output}"
    );
    assert!(
        output.contains("2026-01-21-test-session"),
        "expected session id in output: {output}"
    );
    assert!(
        output.contains("2026-01-21"),
        "expected date in output: {output}"
    );
    assert!(
        output.contains("15000"),
        "expected total tokens in output: {output}"
    );
    assert!(
        output.contains("Intent:"),
        "expected intent label in output: {output}"
    );
    assert!(
        !output.contains("main.rs"),
        "default output should not include file list: {output}"
    );
}

#[test]
fn TestFormatCheckpointOutput_Verbose() {
    let mut content = sample_content();
    content.metadata.files_touched = vec![
        "main.rs".to_string(),
        "util.rs".to_string(),
        "config.yaml".to_string(),
    ];
    content.prompts = "Add a new feature\nFix the bug\nRefactor the code".to_string();
    content.transcript = br#"{"type":"user","message":{"content":"Add a new feature"}}
{"type":"assistant","message":{"content":[{"type":"text","text":"I'll add the feature"}]}}
{"type":"user","message":{"content":"Fix the bug"}}
{"type":"assistant","message":{"content":[{"type":"text","text":"Fixed it"}]}}
"#
    .to_vec();

    let output = format_checkpoint_output(
        &sample_summary(),
        &content,
        "abc123def456",
        None,
        &Author::default(),
        true,
        false,
    );

    assert!(
        output.contains("abc123def456"),
        "expected checkpoint id in output: {output}"
    );
    assert!(
        output.contains("2026-01-21-test-session"),
        "expected session id in output: {output}"
    );
    assert!(
        output.contains("Files:"),
        "expected files section in verbose output: {output}"
    );
    assert!(
        output.contains("main.rs"),
        "expected file list in verbose output: {output}"
    );
    assert!(
        output.contains("Transcript (checkpoint scope):"),
        "expected scoped transcript heading: {output}"
    );
    assert!(
        output.contains("Add a new feature"),
        "expected prompt from scoped transcript: {output}"
    );
}

#[test]
fn TestFormatCheckpointOutput_Verbose_NoCommitMessage() {
    let output = format_checkpoint_output(
        &sample_summary(),
        &sample_content(),
        "abc123def456",
        None,
        &Author::default(),
        true,
        false,
    );

    assert!(
        !output.contains("Commits:"),
        "expected no commits section when associated commits are nil: {output}"
    );
}

#[test]
fn TestFormatCheckpointOutput_Full() {
    let mut content = sample_content();
    content.transcript = br#"{"type":"user","message":{"content":"Add a new feature"}}
{"type":"assistant","message":{"content":[{"type":"text","text":"I'll add that feature for you."}]}}
"#
    .to_vec();

    let output = format_checkpoint_output(
        &sample_summary(),
        &content,
        "abc123def456",
        None,
        &Author::default(),
        false,
        true,
    );

    assert!(
        output.contains("abc123def456"),
        "expected checkpoint id in output: {output}"
    );
    assert!(
        output.contains("Files:"),
        "expected full mode to include files section: {output}"
    );
    assert!(
        output.contains("Transcript (full session):"),
        "expected full transcript heading: {output}"
    );
    assert!(
        output.contains("Add a new feature"),
        "expected transcript content: {output}"
    );
    assert!(
        output.contains("[Assistant]"),
        "expected assistant message marker: {output}"
    );
}

#[test]
fn TestFormatCheckpointOutput_WithSummary() {
    let summary = CheckpointSummary {
        checkpoint_id: "abc123456789".to_string(),
        files_touched: vec!["file1.rs".to_string(), "file2.rs".to_string()],
        ..CheckpointSummary::default()
    };

    let content = SessionContent {
        metadata: CheckpointMetadata {
            checkpoint_id: "abc123456789".to_string(),
            session_id: "2026-01-22-test-session".to_string(),
            created_at: "2026-01-22 10:30".to_string(),
            files_touched: vec!["file1.rs".to_string(), "file2.rs".to_string()],
            summary: Some(SummaryDetails {
                intent: "Implement user authentication".to_string(),
                outcome: "Added login and logout functionality".to_string(),
                repo_learnings: vec!["Uses JWT for auth tokens".to_string()],
                code_learnings: vec![CodeLearning {
                    path: "auth.rs".to_string(),
                    line: 42,
                    end_line: 42,
                    finding: "Token validation happens here".to_string(),
                }],
                workflow_learnings: vec!["Always run tests after auth changes".to_string()],
                friction: vec!["Had to refactor session handling".to_string()],
                open_items: vec!["Add password reset flow".to_string()],
            }),
            ..CheckpointMetadata::default()
        },
        prompts: "Add user authentication".to_string(),
        ..SessionContent::default()
    };

    let default_output = format_checkpoint_output(
        &summary,
        &content,
        "abc123456789",
        None,
        &Author::default(),
        false,
        false,
    );

    assert!(
        default_output.contains("Intent: Implement user authentication"),
        "expected AI intent in output: {default_output}"
    );
    assert!(
        default_output.contains("Outcome: Added login and logout functionality"),
        "expected AI outcome in output: {default_output}"
    );
    assert!(
        !default_output.contains("Learnings:"),
        "default mode should not include detailed learnings: {default_output}"
    );

    let verbose_output = format_checkpoint_output(
        &summary,
        &content,
        "abc123456789",
        None,
        &Author::default(),
        true,
        false,
    );

    assert!(
        verbose_output.contains("Learnings:"),
        "expected learnings section in verbose output: {verbose_output}"
    );
    assert!(
        verbose_output.contains("Repository:"),
        "expected repository learnings in verbose output: {verbose_output}"
    );
    assert!(
        verbose_output.contains("auth.rs:42"),
        "expected code learning location in verbose output: {verbose_output}"
    );
    assert!(
        verbose_output.contains("Workflow:"),
        "expected workflow learnings in verbose output: {verbose_output}"
    );
    assert!(
        verbose_output.contains("Friction:"),
        "expected friction section in verbose output: {verbose_output}"
    );
    assert!(
        verbose_output.contains("Open Items:"),
        "expected open-items section in verbose output: {verbose_output}"
    );
}

#[test]
fn TestFormatCheckpointOutput_HidesTokensWhenTokenUsageMissing() {
    let summary = CheckpointSummary {
        checkpoint_id: "abc123def456".to_string(),
        has_token_usage: false,
        ..CheckpointSummary::default()
    };
    let content = SessionContent {
        metadata: CheckpointMetadata {
            checkpoint_id: "abc123def456".to_string(),
            session_id: "2026-01-22-test-session".to_string(),
            created_at: "2026-01-22 10:30".to_string(),
            has_token_usage: false,
            ..CheckpointMetadata::default()
        },
        ..SessionContent::default()
    };

    let output = format_checkpoint_output(
        &summary,
        &content,
        "abc123def456",
        None,
        &Author::default(),
        false,
        false,
    );

    assert!(
        !output.contains("Tokens:"),
        "expected tokens line to be hidden when token usage is missing: {output}"
    );
}

#[test]
fn TestFormatCheckpointOutput_ShowsTokensWhenTokenUsageExists() {
    let summary = CheckpointSummary {
        checkpoint_id: "abc123def456".to_string(),
        has_token_usage: true,
        token_input: 10,
        token_output: 5,
        ..CheckpointSummary::default()
    };
    let content = SessionContent {
        metadata: CheckpointMetadata {
            checkpoint_id: "abc123def456".to_string(),
            session_id: "2026-01-22-test-session".to_string(),
            created_at: "2026-01-22 10:30".to_string(),
            has_token_usage: true,
            token_input: 10,
            token_output: 5,
            ..CheckpointMetadata::default()
        },
        ..SessionContent::default()
    };

    let output = format_checkpoint_output(
        &summary,
        &content,
        "abc123def456",
        None,
        &Author::default(),
        false,
        false,
    );

    assert!(
        output.contains("Tokens: 15"),
        "expected tokens line when token usage exists: {output}"
    );
}

#[test]
fn TestFormatSummaryDetails() {
    let summary = SummaryDetails {
        intent: "Test intent".to_string(),
        outcome: "Test outcome".to_string(),
        repo_learnings: vec!["Repo learning 1".to_string(), "Repo learning 2".to_string()],
        code_learnings: vec![CodeLearning {
            path: "test.rs".to_string(),
            line: 10,
            end_line: 20,
            finding: "Code finding".to_string(),
        }],
        workflow_learnings: vec!["Workflow learning".to_string()],
        friction: vec!["Friction item".to_string()],
        open_items: vec!["Open item 1".to_string(), "Open item 2".to_string()],
    };

    let output = format_summary_details(&summary);

    assert!(
        output.contains("Learnings:"),
        "expected learnings section in output: {output}"
    );
    assert!(
        output.contains("Repo learning 1"),
        "expected repo learning content in output: {output}"
    );
    assert!(
        output.contains("test.rs:10-20:"),
        "expected code-learning line range in output: {output}"
    );
    assert!(
        output.contains("Friction:"),
        "expected friction section in output: {output}"
    );
    assert!(
        output.contains("Open Items:"),
        "expected open-items section in output: {output}"
    );
}

#[test]
fn TestFormatSummaryDetails_EmptyCategories() {
    let summary = SummaryDetails {
        intent: "Test intent".to_string(),
        outcome: "Test outcome".to_string(),
        ..SummaryDetails::default()
    };

    let output = format_summary_details(&summary);

    assert!(
        !output.contains("Learnings:"),
        "did not expect empty learnings section: {output}"
    );
    assert!(
        !output.contains("Friction:"),
        "did not expect empty friction section: {output}"
    );
    assert!(
        !output.contains("Open Items:"),
        "did not expect empty open-items section: {output}"
    );
}

#[test]
fn TestFormatCheckpointOutput_WithAuthor() {
    let output = format_checkpoint_output(
        &sample_summary(),
        &sample_content(),
        "abc123def456",
        None,
        &Author {
            name: "Alice Developer".to_string(),
            email: "alice@example.com".to_string(),
        },
        true,
        false,
    );

    assert!(
        output.contains("Author: Alice Developer <alice@example.com>"),
        "expected author line in output: {output}"
    );
}

#[test]
fn TestFormatCheckpointOutput_EmptyAuthor() {
    let output = format_checkpoint_output(
        &sample_summary(),
        &sample_content(),
        "abc123def456",
        None,
        &Author::default(),
        true,
        false,
    );

    assert!(
        !output.contains("Author:"),
        "did not expect author line for empty author: {output}"
    );
}

#[test]
fn TestFormatCheckpointOutput_WithAssociatedCommits() {
    let commits = vec![
        AssociatedCommit {
            sha: "abc123def4567890abc123def4567890abc12345".to_string(),
            short_sha: "abc123d".to_string(),
            message: "feat: add feature".to_string(),
            author: "Alice Developer".to_string(),
            date: "2026-02-04".to_string(),
        },
        AssociatedCommit {
            sha: "def456abc7890123def456abc7890123def45678".to_string(),
            short_sha: "def456a".to_string(),
            message: "fix: update feature".to_string(),
            author: "Bob Developer".to_string(),
            date: "2026-02-04".to_string(),
        },
    ];

    let output = format_checkpoint_output(
        &sample_summary(),
        &sample_content(),
        "abc123def456",
        Some(&commits),
        &Author::default(),
        true,
        false,
    );

    assert!(
        output.contains("Commits: (2)"),
        "expected commits count in output: {output}"
    );
    assert!(
        output.contains("abc123d"),
        "expected first short SHA in output: {output}"
    );
    assert!(
        output.contains("def456a"),
        "expected second short SHA in output: {output}"
    );
    assert!(
        output.contains("feat: add feature"),
        "expected first commit message in output: {output}"
    );
    assert!(
        output.contains("fix: update feature"),
        "expected second commit message in output: {output}"
    );
    assert!(
        output.contains("2026-02-04"),
        "expected commit date in output: {output}"
    );
}

#[test]
fn TestFormatCheckpointOutput_NoCommitsOnBranch() {
    let empty_commits: Vec<AssociatedCommit> = Vec::new();

    let output = format_checkpoint_output(
        &sample_summary(),
        &sample_content(),
        "abc123def456",
        Some(&empty_commits),
        &Author::default(),
        true,
        false,
    );

    assert!(
        output.contains("Commits: No commits found on this branch"),
        "expected no-commits message in output: {output}"
    );
}

// CLI-846 --------------------------------------------------------------------

#[test]
fn TestScopeTranscriptForCheckpoint_SlicesTranscript() {
    let full_transcript = br#"{"type":"user","uuid":"u1","message":{"content":"prompt 1"}}
{"type":"assistant","uuid":"a1","message":{"content":[{"type":"text","text":"response 1"}]}}
{"type":"user","uuid":"u2","message":{"content":"prompt 2"}}
{"type":"assistant","uuid":"a2","message":{"content":[{"type":"text","text":"response 2"}]}}
{"type":"user","uuid":"u3","message":{"content":"prompt 3"}}
"#;

    let scoped = scope_transcript_for_checkpoint(full_transcript, 2, AgentType::ClaudeCode);
    let text = String::from_utf8(scoped).expect("expected UTF-8 transcript");

    assert!(
        text.starts_with("{\"type\":\"user\",\"uuid\":\"u2\""),
        "expected scoped transcript to start at line 2: {text}"
    );
    assert!(
        text.contains("\"uuid\":\"u3\""),
        "expected scoped transcript to include final prompt: {text}"
    );
    assert!(
        !text.contains("\"uuid\":\"u1\""),
        "expected scoped transcript to omit earlier prompt: {text}"
    );
}

#[test]
fn TestScopeTranscriptForCheckpoint_ZeroLinesReturnsAll() {
    let transcript_data = br#"{"type":"user","uuid":"u1","message":{"content":"prompt 1"}}
{"type":"user","uuid":"u2","message":{"content":"prompt 2"}}
"#;

    let scoped = scope_transcript_for_checkpoint(transcript_data, 0, AgentType::ClaudeCode);
    let text = String::from_utf8(scoped).expect("expected UTF-8 transcript");

    assert!(
        text.contains("\"uuid\":\"u1\""),
        "expected full transcript when offset=0: {text}"
    );
    assert!(
        text.contains("\"uuid\":\"u2\""),
        "expected full transcript when offset=0: {text}"
    );
}

#[test]
fn TestExtractPromptsFromScopedTranscript() {
    let transcript_data = br#"{"type":"user","uuid":"u1","message":{"content":"First prompt"}}
{"type":"assistant","uuid":"a1","message":{"content":[{"type":"text","text":"First response"}]}}
{"type":"user","uuid":"u2","message":{"content":"Second prompt"}}
{"type":"assistant","uuid":"a2","message":{"content":[{"type":"text","text":"Second response"}]}}
"#;

    let prompts = extract_prompts_from_transcript(transcript_data, AgentType::ClaudeCode);

    assert_eq!(prompts.len(), 2, "expected two extracted prompts");
    assert_eq!(prompts.first().map(String::as_str), Some("First prompt"));
    assert_eq!(prompts.get(1).map(String::as_str), Some("Second prompt"));
}

#[test]
fn TestFormatCheckpointOutput_UsesScopedPrompts() {
    let full_transcript =
        br#"{"type":"user","uuid":"u1","message":{"content":"First prompt - should NOT appear"}}
{"type":"assistant","uuid":"a1","message":{"content":[{"type":"text","text":"First response"}]}}
{"type":"user","uuid":"u2","message":{"content":"Second prompt - SHOULD appear"}}
{"type":"assistant","uuid":"a2","message":{"content":[{"type":"text","text":"Second response"}]}}
"#
        .to_vec();

    let summary = sample_summary();
    let mut content = sample_content();
    content.metadata.checkpoint_transcript_start = 2;
    content.prompts = "First prompt - should NOT appear\nSecond prompt - SHOULD appear".to_string();
    content.transcript = full_transcript;

    let output = format_checkpoint_output(
        &summary,
        &content,
        "abc123def456",
        None,
        &Author::default(),
        true,
        false,
    );

    assert!(
        output.contains("Second prompt - SHOULD appear"),
        "expected scoped prompt in output: {output}"
    );
    assert!(
        !output.contains("First prompt - should NOT appear"),
        "expected old prompt excluded from scoped output: {output}"
    );
}

#[test]
fn TestFormatCheckpointOutput_FallsBackToStoredPrompts() {
    let summary = sample_summary();
    let mut content = sample_content();
    content.prompts = "Stored prompt from older checkpoint".to_string();
    content.transcript = Vec::new();

    let output = format_checkpoint_output(
        &summary,
        &content,
        "abc123def456",
        None,
        &Author::default(),
        true,
        false,
    );

    assert!(
        output.contains("Stored prompt from older checkpoint"),
        "expected stored prompts fallback in output: {output}"
    );
}

#[test]
fn TestFormatCheckpointOutput_FullShowsBitloopsTranscript() {
    let summary = sample_summary();
    let mut content = sample_content();
    content.metadata.checkpoint_transcript_start = 2;
    content.transcript = br#"{"type":"user","uuid":"u1","message":{"content":"First prompt"}}
{"type":"assistant","uuid":"a1","message":{"content":[{"type":"text","text":"First response"}]}}
{"type":"user","uuid":"u2","message":{"content":"Second prompt"}}
{"type":"assistant","uuid":"a2","message":{"content":[{"type":"text","text":"Second response"}]}}
"#
    .to_vec();

    let output = format_checkpoint_output(
        &summary,
        &content,
        "abc123def456",
        None,
        &Author::default(),
        false,
        true,
    );

    assert!(
        output.contains("First prompt"),
        "expected full transcript to include first prompt: {output}"
    );
    assert!(
        output.contains("Second prompt"),
        "expected full transcript to include second prompt: {output}"
    );
}

// CLI-847 --------------------------------------------------------------------

#[test]
fn TestGetAssociatedCommits() {
    let checkpoint_id = "abc123def456";

    let commits = vec![
        CommitNode {
            sha: "1111111111111111111111111111111111111111".to_string(),
            message: "initial commit".to_string(),
            author: "Test".to_string(),
            timestamp: 1,
            ..CommitNode::default()
        },
        CommitNode {
            sha: "2222222222222222222222222222222222222222".to_string(),
            message: "feat: add feature".to_string(),
            author: "Alice Developer".to_string(),
            timestamp: 2,
            checkpoints: HashMap::from([(CHECKPOINT_KEY.to_string(), checkpoint_id.to_string())]),
            ..CommitNode::default()
        },
        CommitNode {
            sha: "3333333333333333333333333333333333333333".to_string(),
            message: "unrelated commit".to_string(),
            author: "Test".to_string(),
            timestamp: 3,
            ..CommitNode::default()
        },
    ];

    let associated = get_associated_commits(&commits, checkpoint_id, false)
        .expect("expected associated commit scan to succeed");

    assert_eq!(
        associated.len(),
        1,
        "expected exactly one associated commit"
    );
    assert_eq!(associated[0].author, "Alice Developer");
    assert!(associated[0].message.contains("feat: add feature"));
    assert_eq!(
        associated[0].short_sha.len(),
        7,
        "expected short SHA length"
    );
    assert_eq!(associated[0].sha.len(), 40, "expected full SHA length");
}

#[test]
fn TestGetAssociatedCommits_NoMatches() {
    let commits = vec![CommitNode {
        sha: "1111111111111111111111111111111111111111".to_string(),
        message: "regular commit".to_string(),
        author: "Test".to_string(),
        timestamp: 1,
        ..CommitNode::default()
    }];

    let associated = get_associated_commits(&commits, "aaaa11112222", false)
        .expect("expected commit scan to succeed");

    assert_eq!(associated.len(), 0, "expected no associated commits");
}

#[test]
fn TestGetAssociatedCommits_MultipleMatches() {
    let checkpoint_id = "abc123def456";
    let commits = vec![
        CommitNode {
            sha: "1111111111111111111111111111111111111111".to_string(),
            message: "first checkpoint commit".to_string(),
            author: "Test".to_string(),
            timestamp: 1,
            checkpoints: HashMap::from([(CHECKPOINT_KEY.to_string(), checkpoint_id.to_string())]),
            ..CommitNode::default()
        },
        CommitNode {
            sha: "2222222222222222222222222222222222222222".to_string(),
            message: "second checkpoint commit".to_string(),
            author: "Test".to_string(),
            timestamp: 2,
            checkpoints: HashMap::from([(CHECKPOINT_KEY.to_string(), checkpoint_id.to_string())]),
            ..CommitNode::default()
        },
    ];

    let associated = get_associated_commits(&commits, checkpoint_id, false)
        .expect("expected associated commit scan to succeed");

    assert_eq!(associated.len(), 2, "expected two matching commits");
    assert!(
        associated[0].message.contains("second"),
        "expected reverse chronological ordering"
    );
    assert!(
        associated[1].message.contains("first"),
        "expected reverse chronological ordering"
    );
}

#[test]
fn TestGetAssociatedCommits_SearchAllFindsMergedBranchCommits() {
    let checkpoint_id = "aabb11223344";

    let commits = vec![
        CommitNode {
            sha: "aaaa000000000000000000000000000000000000".to_string(),
            message: "Merge feature into main".to_string(),
            parents: vec![
                "bbbb000000000000000000000000000000000000".to_string(),
                "cccc000000000000000000000000000000000000".to_string(),
            ],
            timestamp: 4,
            author: "Test".to_string(),
            ..CommitNode::default()
        },
        CommitNode {
            sha: "bbbb000000000000000000000000000000000000".to_string(),
            message: "main: parallel work".to_string(),
            parents: vec!["dddd000000000000000000000000000000000000".to_string()],
            timestamp: 3,
            author: "Test".to_string(),
            ..CommitNode::default()
        },
        CommitNode {
            sha: "cccc000000000000000000000000000000000000".to_string(),
            message: "feat: add feature".to_string(),
            parents: vec!["dddd000000000000000000000000000000000000".to_string()],
            timestamp: 2,
            author: "Feature Dev".to_string(),
            checkpoints: HashMap::from([(CHECKPOINT_KEY.to_string(), checkpoint_id.to_string())]),
            ..CommitNode::default()
        },
    ];

    let first_parent_only = get_associated_commits(&commits, checkpoint_id, false)
        .expect("expected commit scan to succeed");
    assert_eq!(
        first_parent_only.len(),
        0,
        "expected no matches without --search-all"
    );

    let search_all = get_associated_commits(&commits, checkpoint_id, true)
        .expect("expected commit scan to succeed");
    assert_eq!(
        search_all.len(),
        1,
        "expected merged-branch match with --search-all"
    );
    assert_eq!(search_all[0].author, "Feature Dev");
}

// CLI-848 --------------------------------------------------------------------

#[test]
fn TestFormatBranchCheckpoints_BasicOutput() {
    let points = vec![
        RewindPoint {
            id: "abc123def456".to_string(),
            message: "Add feature X".to_string(),
            date: "2026-01-22".to_string(),
            checkpoint_id: "chk123456789".to_string(),
            session_id: "2026-01-22-session-1".to_string(),
            session_prompt: "Implement feature X".to_string(),
            ..RewindPoint::default()
        },
        RewindPoint {
            id: "def456ghi789".to_string(),
            message: "Fix bug in Y".to_string(),
            date: "2026-01-22".to_string(),
            checkpoint_id: "chk987654321".to_string(),
            session_id: "2026-01-22-session-2".to_string(),
            session_prompt: "Fix the bug".to_string(),
            ..RewindPoint::default()
        },
    ];

    let output = format_branch_checkpoints("feature/my-branch", &points, "");

    assert!(
        output.contains("feature/my-branch"),
        "expected branch name in output: {output}"
    );
    assert!(
        output.contains("Checkpoints: 2"),
        "expected checkpoints count in output: {output}"
    );
    assert!(
        output.contains("Add feature X"),
        "expected first checkpoint message in output: {output}"
    );
    assert!(
        output.contains("Fix bug in Y"),
        "expected second checkpoint message in output: {output}"
    );
}

#[test]
fn TestFormatBranchCheckpoints_GroupedByCheckpointID() {
    let points = vec![
        RewindPoint {
            id: "abc123def456".to_string(),
            message: "Today checkpoint 1".to_string(),
            date: "01-22".to_string(),
            checkpoint_id: "chk111111111".to_string(),
            session_id: "2026-01-22-session-1".to_string(),
            session_prompt: "First task today".to_string(),
            ..RewindPoint::default()
        },
        RewindPoint {
            id: "def456ghi789".to_string(),
            message: "Today checkpoint 2".to_string(),
            date: "01-22".to_string(),
            checkpoint_id: "chk222222222".to_string(),
            session_id: "2026-01-22-session-1".to_string(),
            session_prompt: "First task today".to_string(),
            ..RewindPoint::default()
        },
        RewindPoint {
            id: "ghi789jkl012".to_string(),
            message: "Yesterday checkpoint".to_string(),
            date: "01-21".to_string(),
            checkpoint_id: "chk333333333".to_string(),
            session_id: "2026-01-21-session-2".to_string(),
            session_prompt: "Task from yesterday".to_string(),
            ..RewindPoint::default()
        },
    ];

    let output = format_branch_checkpoints("main", &points, "");

    assert!(
        output.contains("[chk111111111]"),
        "expected first checkpoint group header: {output}"
    );
    assert!(
        output.contains("[chk333333333]"),
        "expected second checkpoint group header: {output}"
    );
    assert!(
        output.contains("01-22"),
        "expected recent date in output: {output}"
    );
    assert!(
        output.contains("01-21"),
        "expected older date in output: {output}"
    );

    let today_idx = output.find("chk111111111").unwrap_or(usize::MAX);
    let yesterday_idx = output.find("chk333333333").unwrap_or(usize::MAX);
    assert!(
        today_idx < yesterday_idx,
        "expected recent checkpoint group before older one: {output}"
    );
}

#[test]
fn TestFormatBranchCheckpoints_NoCheckpoints() {
    let output = format_branch_checkpoints("feature/empty-branch", &[], "");

    assert!(
        output.contains("feature/empty-branch"),
        "expected branch name in output: {output}"
    );
    assert!(
        output.contains("Checkpoints: 0") || output.contains("No checkpoints"),
        "expected no-checkpoints indication in output: {output}"
    );
}

#[test]
fn TestFormatBranchCheckpoints_ShowsSessionInfo() {
    let points = vec![RewindPoint {
        id: "abc123def456".to_string(),
        message: "Test checkpoint".to_string(),
        date: "2026-01-22".to_string(),
        checkpoint_id: "chk123456789".to_string(),
        session_id: "2026-01-22-test-session".to_string(),
        session_prompt: "This is my test prompt".to_string(),
        ..RewindPoint::default()
    }];

    let output = format_branch_checkpoints("main", &points, "");

    assert!(
        output.contains("This is my test prompt"),
        "expected session prompt in output: {output}"
    );
}

#[test]
fn TestFormatBranchCheckpoints_ShowsTemporaryIndicator() {
    let points = vec![
        RewindPoint {
            id: "abc123def456".to_string(),
            message: "Committed checkpoint".to_string(),
            date: "2026-01-22".to_string(),
            checkpoint_id: "chk123456789".to_string(),
            is_logs_only: true,
            session_id: "2026-01-22-session-1".to_string(),
            ..RewindPoint::default()
        },
        RewindPoint {
            id: "def456ghi789".to_string(),
            message: "Active checkpoint".to_string(),
            date: "2026-01-22".to_string(),
            checkpoint_id: "chk987654321".to_string(),
            is_logs_only: false,
            session_id: "2026-01-22-session-1".to_string(),
            ..RewindPoint::default()
        },
    ];

    let output = format_branch_checkpoints("main", &points, "");

    assert!(
        output.contains("[temporary]"),
        "expected temporary marker for non-committed checkpoint: {output}"
    );

    for line in output.lines() {
        if line.contains("chk123456789") {
            assert!(
                !line.contains("[temporary]"),
                "committed checkpoint line should not show temporary marker: {line}"
            );
        }
    }
}

#[test]
fn TestFormatBranchCheckpoints_ShowsTaskCheckpoints() {
    let points = vec![RewindPoint {
        id: "abc123def456".to_string(),
        message: "Running tests (toolu_01ABC)".to_string(),
        date: "2026-01-22".to_string(),
        checkpoint_id: "chk123456789".to_string(),
        is_task_checkpoint: true,
        tool_use_id: "toolu_01ABC".to_string(),
        session_id: "2026-01-22-session-1".to_string(),
        ..RewindPoint::default()
    }];

    let output = format_branch_checkpoints("main", &points, "");

    assert!(
        output.contains("[Task]") || output.to_lowercase().contains("task"),
        "expected task checkpoint indicator: {output}"
    );
}

#[test]
fn TestFormatBranchCheckpoints_TruncatesLongMessages() {
    let long_message = "a".repeat(200);
    let points = vec![RewindPoint {
        id: "abc123def456".to_string(),
        message: long_message.clone(),
        date: "2026-01-22".to_string(),
        checkpoint_id: "chk123456789".to_string(),
        session_id: "2026-01-22-session-1".to_string(),
        ..RewindPoint::default()
    }];

    let output = format_branch_checkpoints("main", &points, "");

    assert!(
        !output.contains(&long_message),
        "expected long message truncation: {output}"
    );
    assert!(
        output.contains("..."),
        "expected truncation ellipsis in output: {output}"
    );
}

#[test]
fn TestFormatBranchCheckpoints_SessionFilter() {
    let points = vec![
        RewindPoint {
            id: "abc123def456".to_string(),
            message: "Checkpoint from session 1".to_string(),
            date: "2026-01-22".to_string(),
            checkpoint_id: "chk111111111".to_string(),
            session_id: "2026-01-22-session-alpha".to_string(),
            session_prompt: "Task for session alpha".to_string(),
            ..RewindPoint::default()
        },
        RewindPoint {
            id: "def456ghi789".to_string(),
            message: "Checkpoint from session 2".to_string(),
            date: "2026-01-22".to_string(),
            checkpoint_id: "chk222222222".to_string(),
            session_id: "2026-01-22-session-beta".to_string(),
            session_prompt: "Task for session beta".to_string(),
            ..RewindPoint::default()
        },
        RewindPoint {
            id: "ghi789jkl012".to_string(),
            message: "Another checkpoint from session 1".to_string(),
            date: "2026-01-22".to_string(),
            checkpoint_id: "chk333333333".to_string(),
            session_id: "2026-01-22-session-alpha".to_string(),
            session_prompt: "Another task for session alpha".to_string(),
            ..RewindPoint::default()
        },
    ];

    let no_filter = format_branch_checkpoints("main", &points, "");
    assert!(
        no_filter.contains("Checkpoints: 3"),
        "expected all checkpoints without filter: {no_filter}"
    );
    assert!(
        no_filter.contains("Task for session alpha"),
        "expected alpha prompt without filter: {no_filter}"
    );
    assert!(
        no_filter.contains("Task for session beta"),
        "expected beta prompt without filter: {no_filter}"
    );

    let exact = format_branch_checkpoints("main", &points, "2026-01-22-session-alpha");
    assert!(
        exact.contains("Checkpoints: 2"),
        "expected two checkpoints for exact filter: {exact}"
    );
    assert!(
        exact.contains("Task for session alpha"),
        "expected alpha prompt in filtered output: {exact}"
    );
    assert!(
        !exact.contains("Task for session beta"),
        "did not expect beta prompt with alpha filter: {exact}"
    );
    assert!(
        exact.contains("Filtered by session:"),
        "expected filter metadata in output: {exact}"
    );

    let prefix = format_branch_checkpoints("main", &points, "2026-01-22-session-b");
    assert!(
        prefix.contains("Checkpoints: 1"),
        "expected one checkpoint with beta prefix filter: {prefix}"
    );
    assert!(
        prefix.contains("Task for session beta"),
        "expected beta prompt with beta prefix filter: {prefix}"
    );

    let no_match = format_branch_checkpoints("main", &points, "nonexistent-session");
    assert!(
        no_match.contains("Checkpoints: 0"),
        "expected zero checkpoints for unmatched filter: {no_match}"
    );
    assert!(
        no_match.contains("Filtered by session:"),
        "expected filter metadata even with no matches: {no_match}"
    );
}

// CLI-849 --------------------------------------------------------------------

#[test]
fn TestGetCurrentWorktreeHash_MainWorktree() {
    let hash = get_current_worktree_hash("");
    // SHA256("") first 6 hex chars = "e3b0c4"
    let expected = "e3b0c4";
    assert_eq!(hash, expected, "expected SHA256 hash for empty worktree id");
}

#[test]
fn TestRunExplainBranchDefault_DetachedHead() {
    let (_tmp, root) = setup_git_repo();
    let _ = make_commit(&root, "file.txt", "initial", "initial commit");
    run_git_cmd(&root, &["checkout", "--detach"]);

    let output = run_explain_branch_default_for_test(&root, true)
        .expect("expected detached-head branch explain output");

    assert!(
        output.contains("HEAD") || output.to_lowercase().contains("detached"),
        "expected detached-head signal in output: {output}"
    );
}

#[test]
fn TestGetBranchCheckpointsReal_OnFeatureBranch() {
    let (_tmp, root) = setup_git_repo();
    let _ = make_commit(&root, "file.txt", "initial", "initial commit");
    run_git_cmd(&root, &["checkout", "-b", "feature/test"]);

    let points = get_branch_checkpoints_real(&root, 20)
        .expect("expected branch checkpoint lookup to succeed");
    assert_eq!(
        points.len(),
        0,
        "expected no checkpoints on fresh feature branch"
    );
}

#[test]
fn TestGetBranchCheckpointsReal_FiltersMainCommits() {
    let (_tmp, root) = setup_git_repo();
    let main_checkpoint = "aaa111bbb222";
    let feature_checkpoint = "ccc333ddd444";

    let main_sha = make_checkpoint_commit(
        &root,
        "file.txt",
        "main content",
        "main: checkpoint",
        main_checkpoint,
    );
    let default_branch = run_git_cmd(&root, &["rev-parse", "--abbrev-ref", "HEAD"]);

    run_git_cmd(&root, &["checkout", "-b", "feature/test"]);
    let feature_sha = make_checkpoint_commit(
        &root,
        "feature.txt",
        "feature content",
        "feat: feature checkpoint",
        feature_checkpoint,
    );
    write_committed_checkpoint_metadata_with_mappings(
        &root,
        &[
            (main_checkpoint, &main_sha),
            (feature_checkpoint, &feature_sha),
        ],
    );
    run_git_cmd(&root, &["checkout", "feature/test"]);

    let points = get_branch_checkpoints_real(&root, 20)
        .expect("expected branch checkpoint lookup to succeed");

    assert!(
        points.iter().all(|p| p.checkpoint_id != main_checkpoint),
        "expected main-branch checkpoints filtered from feature branch"
    );
    assert!(
        points.iter().any(|p| p.checkpoint_id == feature_checkpoint),
        "expected feature checkpoint to remain visible"
    );
    run_git_cmd(&root, &["checkout", &default_branch]);
}

#[test]
fn TestGetBranchCheckpointsReal_DefaultBranchFindsMergedCheckpoints() {
    let (_tmp, root) = setup_git_repo_with_initial_branch("trunk");
    let checkpoint_id = "fea112233344";

    let _ = make_commit(&root, "file.txt", "initial", "initial commit");
    let default_branch = run_git_cmd(&root, &["rev-parse", "--abbrev-ref", "HEAD"]);
    run_git_cmd(&root, &["checkout", "-b", "feature/test"]);
    let feature_sha = make_checkpoint_commit(
        &root,
        "feature.txt",
        "feature content",
        "feat: feature checkpoint",
        checkpoint_id,
    );
    run_git_cmd(&root, &["checkout", &default_branch]);
    run_git_cmd(
        &root,
        &["merge", "--no-ff", "feature/test", "-m", "merge feature"],
    );
    write_committed_checkpoint_metadata_with_mappings(&root, &[(checkpoint_id, &feature_sha)]);
    run_git_cmd(&root, &["checkout", &default_branch]);

    let points = get_branch_checkpoints_real(&root, 100)
        .expect("expected default-branch checkpoint lookup to succeed");

    assert!(
        points.iter().any(|p| p.checkpoint_id == checkpoint_id),
        "expected merged feature checkpoint to be reachable from default branch"
    );
}

// CLI-850 --------------------------------------------------------------------

#[test]
fn TestWalkFirstParentCommits_SkipsMergeParents() {
    let commit_map = HashMap::from([
        (
            "M".to_string(),
            CommitNode {
                sha: "M".to_string(),
                message: "M: merge main into feature".to_string(),
                parents: vec!["B".to_string(), "C".to_string()],
                ..CommitNode::default()
            },
        ),
        (
            "B".to_string(),
            CommitNode {
                sha: "B".to_string(),
                message: "B: feature work".to_string(),
                parents: vec!["A".to_string()],
                ..CommitNode::default()
            },
        ),
        (
            "C".to_string(),
            CommitNode {
                sha: "C".to_string(),
                message: "C: main work".to_string(),
                parents: vec!["A".to_string()],
                ..CommitNode::default()
            },
        ),
        (
            "A".to_string(),
            CommitNode {
                sha: "A".to_string(),
                message: "A: initial".to_string(),
                parents: vec![],
                ..CommitNode::default()
            },
        ),
    ]);

    let visited = walk_first_parent_commits("M", &commit_map, 0)
        .expect("expected first-parent walk to succeed");
    let messages: Vec<&str> = visited.iter().map(|c| c.message.as_str()).collect();

    assert_eq!(
        messages,
        vec![
            "M: merge main into feature",
            "B: feature work",
            "A: initial"
        ]
    );
    assert!(
        !messages.iter().any(|m| m.contains("C: main work")),
        "expected second parent commits to be skipped"
    );
}

#[test]
fn TestIsAncestorOf() {
    let commit_map = HashMap::from([
        (
            "1".to_string(),
            CommitNode {
                sha: "1".to_string(),
                message: "first".to_string(),
                parents: vec![],
                ..CommitNode::default()
            },
        ),
        (
            "2".to_string(),
            CommitNode {
                sha: "2".to_string(),
                message: "second".to_string(),
                parents: vec!["1".to_string()],
                ..CommitNode::default()
            },
        ),
    ]);

    assert!(
        is_ancestor_of(&commit_map, "1", "2"),
        "expected commit 1 to be ancestor of commit 2"
    );
    assert!(
        !is_ancestor_of(&commit_map, "2", "1"),
        "expected commit 2 not to be ancestor of commit 1"
    );
    assert!(
        is_ancestor_of(&commit_map, "1", "1"),
        "expected commit to be ancestor of itself"
    );
}

#[test]
fn TestHasCodeChanges_FirstCommitReturnsTrue() {
    let changed = vec!["test.txt".to_string()];
    assert!(
        has_code_changes(&changed, true),
        "expected first commit to be treated as code-changing"
    );
}

#[test]
fn TestHasCodeChanges_OnlyMetadataChanges() {
    let changed = vec![
        ".bitloops/internal/sessions/session-123/full.jsonl".to_string(),
        ".bitloops/internal/sessions/session-123/prompt.txt".to_string(),
    ];
    assert!(
        !has_code_changes(&changed, false),
        "expected metadata-only change set to be treated as non-code"
    );
}

#[test]
fn TestHasCodeChanges_OnlyCheckpointArtefactTaskChanges() {
    let changed = vec![
        ".bitloops/internal/sessions/session-123/tasks/toolu_abc/context.json".to_string(),
        ".bitloops/internal/sessions/session-123/tasks/toolu_abc/output.json".to_string(),
    ];
    assert!(
        !has_code_changes(&changed, false),
        "expected checkpoint artefact-only change set to be treated as non-code"
    );
}

#[test]
fn TestHasCodeChanges_WithCodeChanges() {
    let changed = vec!["src/main.rs".to_string()];
    assert!(
        has_code_changes(&changed, false),
        "expected source change to be treated as code-changing"
    );
}

#[test]
fn TestHasCodeChanges_MixedChanges() {
    let changed = vec![
        ".bitloops/internal/sessions/session-123/full.jsonl".to_string(),
        "src/main.rs".to_string(),
    ];
    assert!(
        has_code_changes(&changed, false),
        "expected mixed metadata+code change set to be treated as code-changing"
    );
}
