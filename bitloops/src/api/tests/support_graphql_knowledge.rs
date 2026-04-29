use super::*;

pub(super) fn seed_graphql_devql_repo() -> TempDir {
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
         VALUES (?1, 'local', 'local', ?2, 'main')",
        rusqlite::params![repo_id.as_str(), SEEDED_REPO_NAME],
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
            "INSERT INTO current_file_state (
                repo_id, path, language,
                head_content_id, index_content_id, worktree_content_id,
                effective_content_id, effective_source,
                parser_version, extractor_version,
                exists_in_head, exists_in_index, exists_in_worktree,
                last_synced_at
            ) VALUES (?1, ?2, 'typescript', ?3, ?3, ?3, ?3, 'head', 'test', 'test', 1, 1, 1, '2026-03-26T09:00:00Z')",
            rusqlite::params![repo_id.as_str(), path, blob_sha],
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
                artefact_id, symbol_id, repo_id, language, canonical_kind,
                language_kind, symbol_fqn, signature, modifiers, docstring, content_hash, created_at
            ) VALUES (
                ?1, ?2, ?3, 'typescript', ?4,
                ?5, ?6, NULL, ?7, ?8, ?9, '2026-03-26T09:00:00Z'
            )",
            rusqlite::params![
                artefact_id,
                symbol_id,
                repo_id.as_str(),
                canonical_kind,
                language_kind,
                symbol_fqn,
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
        .expect("insert artefact metadata row");
        conn.execute(
            "INSERT INTO artefact_snapshots (
                repo_id, blob_sha, path, artefact_id, parent_artefact_id,
                start_line, end_line, start_byte, end_byte, created_at
            ) VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6, ?7, 0, ?8, '2026-03-26T09:00:00Z'
            )",
            rusqlite::params![
                repo_id.as_str(),
                blob_sha,
                path,
                artefact_id,
                parent_artefact_id,
                start_line,
                end_line,
                end_line * 10,
            ],
        )
        .expect("insert artefact snapshot row");
        conn.execute(
            "INSERT INTO artefacts_current (
                repo_id, path, content_id, symbol_id, artefact_id,
                language, canonical_kind, language_kind,
                symbol_fqn, parent_symbol_id, parent_artefact_id, start_line, end_line,
                start_byte, end_byte, signature, modifiers, docstring, updated_at
            ) VALUES (
                ?1, ?2, ?3, ?4, ?5,
                'typescript', ?6, ?7,
                ?8, ?9, ?10, ?11, ?12,
                0, ?13, NULL, ?14, ?15, '2026-03-26T09:00:00Z'
            )",
            rusqlite::params![
                repo_id.as_str(),
                path,
                blob_sha,
                symbol_id,
                artefact_id,
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
                edge_id, repo_id, path, content_id,
                from_symbol_id, from_artefact_id,
                to_symbol_id, to_artefact_id, to_symbol_ref, edge_kind, language,
                start_line, end_line, metadata, updated_at
            ) VALUES (
                ?1, ?2, ?3, 'blob-caller',
                ?4, ?5,
                ?6, ?7, ?8, 'calls', 'typescript',
                ?9, ?9, ?10, '2026-03-26T09:00:00Z'
            )",
            rusqlite::params![
                edge_id,
                repo_id.as_str(),
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

pub(super) fn seed_graphql_historical_context_data(repo_root: &Path) {
    use crate::host::checkpoints::strategy::manual_commit::TokenUsageMetadata;
    use crate::host::interactions::db_store::{SqliteInteractionSpool, interaction_spool_db_path};
    use crate::host::interactions::store::InteractionSpool;
    use crate::host::interactions::types::{
        InteractionEvent, InteractionEventType, InteractionSession, InteractionTurn,
    };
    use crate::storage::sqlite::SqliteConnectionPool;

    let commit_sha = git_ok(repo_root, &["rev-parse", "HEAD"]);
    let primary_sessions = [SeedCheckpointSession {
        session_index: 0,
        session_id: "session-historical-primary",
        agent: "codex",
        created_at: "2026-03-26T09:10:00Z",
        checkpoints_count: 1,
        transcript: r#"{"role":"user","content":"Update caller and target"}"#,
        prompts: "Update caller and target",
        context: "",
    }];
    seed_checkpoint_storage_for_dashboard(
        repo_root,
        SeedCheckpointStorage {
            commit_sha: &commit_sha,
            checkpoint_id: "checkpoint-historical-primary",
            branch: "main",
            files_touched: &["src/caller.ts", "src/target.ts"],
            checkpoints_count: 1,
            token_usage: json!({"input_tokens": 10, "output_tokens": 5}),
            sessions: &primary_sessions,
            insert_mapping: false,
        },
    );

    let partial_sessions = [SeedCheckpointSession {
        session_index: 0,
        session_id: "session-historical-partial",
        agent: "codex",
        created_at: "2026-03-26T09:20:00Z",
        checkpoints_count: 1,
        transcript: r#"{"role":"user","content":"Touch caller only"}"#,
        prompts: "Touch caller only",
        context: "",
    }];
    seed_checkpoint_storage_for_dashboard(
        repo_root,
        SeedCheckpointStorage {
            commit_sha: &commit_sha,
            checkpoint_id: "checkpoint-historical-partial",
            branch: "main",
            files_touched: &["src/caller.ts"],
            checkpoints_count: 1,
            token_usage: json!({"input_tokens": 7, "output_tokens": 3}),
            sessions: &partial_sessions,
            insert_mapping: false,
        },
    );

    let other_agent_sessions = [SeedCheckpointSession {
        session_index: 0,
        session_id: "session-historical-gemini",
        agent: "gemini",
        created_at: "2026-03-26T09:30:00Z",
        checkpoints_count: 1,
        transcript: r#"{"role":"user","content":"Update target as Gemini"}"#,
        prompts: "Update target as Gemini",
        context: "",
    }];
    seed_checkpoint_storage_for_dashboard(
        repo_root,
        SeedCheckpointStorage {
            commit_sha: &commit_sha,
            checkpoint_id: "checkpoint-historical-gemini",
            branch: "main",
            files_touched: &["src/target.ts"],
            checkpoints_count: 1,
            token_usage: json!({"input_tokens": 6, "output_tokens": 2}),
            sessions: &other_agent_sessions,
            insert_mapping: false,
        },
    );

    let repo_id = crate::host::devql::resolve_repo_id(repo_root).expect("resolve repo id");
    let conn = rusqlite::Connection::open(checkpoint_sqlite_path(repo_root))
        .expect("open historical context sqlite");
    conn.execute(
        "INSERT INTO checkpoint_artefacts (
            relation_id, repo_id, checkpoint_id, session_id, event_time, agent, branch, strategy,
            commit_sha, change_kind, before_symbol_id, after_symbol_id, before_artefact_id,
            after_artefact_id
        ) VALUES (
            'checkpoint-historical-primary-target-symbol', ?1, 'checkpoint-historical-primary',
            'session-historical-primary', '2026-03-26T09:10:00Z', 'codex', 'main',
            'manual-commit', ?2, 'modify', NULL, 'sym::target', NULL, 'artefact::target'
        )",
        rusqlite::params![repo_id.as_str(), commit_sha.as_str()],
    )
    .expect("insert symbol provenance row");

    insert_checkpoint_file_snapshot_row(
        &conn,
        repo_id.as_str(),
        "checkpoint-historical-gemini",
        "session-historical-gemini",
        "2026-03-26T09:30:00Z",
        "gemini",
        "main",
        "manual-commit",
        &commit_sha,
        "src/target.ts",
        "blob-target",
    );

    let spool_path = interaction_spool_db_path(repo_root).expect("resolve interaction spool path");
    let sqlite = SqliteConnectionPool::connect(spool_path).expect("connect interaction spool");
    let spool = SqliteInteractionSpool::new(sqlite, repo_id).expect("initialise interaction spool");
    let session = InteractionSession {
        session_id: "session-historical-primary".to_string(),
        repo_id: spool.repo_id().to_string(),
        branch: "main".to_string(),
        actor_id: "actor-historical".to_string(),
        actor_name: "Alice".to_string(),
        actor_email: "alice@example.com".to_string(),
        actor_source: "seed".to_string(),
        agent_type: "codex".to_string(),
        model: "gpt-5.4".to_string(),
        first_prompt: "Update caller and target".to_string(),
        transcript_path: repo_root
            .join("historical-transcript.jsonl")
            .to_string_lossy()
            .to_string(),
        worktree_path: repo_root.to_string_lossy().to_string(),
        worktree_id: "main".to_string(),
        started_at: "2026-03-26T09:09:00Z".to_string(),
        ended_at: Some("2026-03-26T09:11:00Z".to_string()),
        last_event_at: "2026-03-26T09:11:00Z".to_string(),
        updated_at: "2026-03-26T09:11:00Z".to_string(),
    };
    spool
        .record_session(&session)
        .expect("record historical interaction session");
    spool
        .record_turn(&InteractionTurn {
            turn_id: "turn-historical-primary".to_string(),
            session_id: session.session_id.clone(),
            repo_id: spool.repo_id().to_string(),
            branch: "main".to_string(),
            actor_id: session.actor_id.clone(),
            actor_name: session.actor_name.clone(),
            actor_email: session.actor_email.clone(),
            actor_source: session.actor_source.clone(),
            turn_number: 1,
            prompt: "Captured prompt for target change".to_string(),
            agent_type: "codex".to_string(),
            model: "gpt-5.4".to_string(),
            started_at: "2026-03-26T09:09:30Z".to_string(),
            ended_at: Some("2026-03-26T09:10:30Z".to_string()),
            token_usage: Some(TokenUsageMetadata {
                input_tokens: 10,
                output_tokens: 5,
                cache_creation_tokens: 0,
                cache_read_tokens: 0,
                api_call_count: 1,
                subagent_tokens: None,
            }),
            summary: "Captured summary for target change".to_string(),
            prompt_count: 1,
            transcript_offset_start: Some(0),
            transcript_offset_end: Some(64),
            transcript_fragment: "Captured transcript fragment for target change".to_string(),
            files_modified: vec!["src/target.ts".to_string()],
            checkpoint_id: Some("checkpoint-historical-primary".to_string()),
            updated_at: "2026-03-26T09:10:30Z".to_string(),
        })
        .expect("record historical interaction turn");
    spool
        .record_event(&InteractionEvent {
            event_id: "event-historical-tool".to_string(),
            session_id: session.session_id,
            turn_id: Some("turn-historical-primary".to_string()),
            repo_id: spool.repo_id().to_string(),
            branch: "main".to_string(),
            actor_id: "actor-historical".to_string(),
            actor_name: "Alice".to_string(),
            actor_email: "alice@example.com".to_string(),
            actor_source: "seed".to_string(),
            event_type: InteractionEventType::ToolInvocationObserved,
            event_time: "2026-03-26T09:10:00Z".to_string(),
            source: "test".to_string(),
            sequence_number: 1,
            agent_type: "codex".to_string(),
            model: "gpt-5.4".to_string(),
            tool_use_id: "toolu-historical-edit".to_string(),
            tool_kind: "edit".to_string(),
            task_description: "Edit src/target.ts".to_string(),
            subagent_id: String::new(),
            payload: json!({
                "tool_name": "edit",
                "input_summary": "Edit src/target.ts",
                "output_summary": "Updated target implementation",
                "command": "apply_patch src/target.ts",
                "command_binary": "apply_patch",
                "command_argv": ["apply_patch", "src/target.ts"]
            }),
        })
        .expect("record historical interaction tool event");
}

pub(super) fn seed_graphql_context_guidance_data(repo_root: &Path) {
    use crate::capability_packs::context_guidance::distillation::{
        GuidanceDistillationInput, GuidanceToolEvidence,
    };
    use crate::capability_packs::context_guidance::storage::{
        ContextGuidanceRepository, SqliteContextGuidanceRepository,
    };
    use crate::capability_packs::context_guidance::types::{
        GuidanceAppliesTo, GuidanceDistillationOutput, GuidanceFactCategory,
        GuidanceFactConfidence, GuidanceFactDraft, GuidanceSessionSummary,
    };
    use crate::host::relational_store::DefaultRelationalStore;

    let repo_id = crate::host::devql::resolve_repo_id(repo_root).expect("resolve repo id");
    let relational_store =
        DefaultRelationalStore::open_local_for_repo_root(repo_root).expect("open relational store");
    let sqlite = relational_store
        .local_sqlite_pool_allow_create()
        .expect("sqlite pool");
    let repository = SqliteContextGuidanceRepository::new(sqlite);
    repository
        .initialise_schema()
        .expect("context guidance schema");
    let input = GuidanceDistillationInput {
        checkpoint_id: Some("checkpoint-historical-primary".to_string()),
        session_id: "session-historical-primary".to_string(),
        turn_id: Some("turn-historical-primary".to_string()),
        event_time: Some("2026-03-26T09:10:00Z".to_string()),
        agent_type: Some("codex".to_string()),
        model: Some("gpt-5.4".to_string()),
        prompt: Some("Improve attr parsing".to_string()),
        transcript_fragment: Some("Rejected std::any::type_name parsing.".to_string()),
        files_modified: vec!["src/target.ts".to_string()],
        tool_events: vec![GuidanceToolEvidence {
            tool_kind: Some("shell".to_string()),
            input_summary: Some("cargo nextest run --lib context_guidance".to_string()),
            output_summary: Some("nextest passed".to_string()),
            command: Some("cargo nextest run --lib context_guidance".to_string()),
        }],
    };
    let output = GuidanceDistillationOutput {
        summary: GuidanceSessionSummary {
            intent: "Improve target implementation.".to_string(),
            outcome: "Stored guidance for target.".to_string(),
            decisions: vec!["Use token rendering.".to_string()],
            rejected_approaches: vec!["Avoid std::any::type_name parsing.".to_string()],
            patterns: Vec::new(),
            verification: vec!["nextest passed.".to_string()],
            open_items: Vec::new(),
        },
        guidance_facts: vec![
            GuidanceFactDraft {
                category: GuidanceFactCategory::Decision,
                kind: "rejected_approach".to_string(),
                guidance: "Do not derive attribute keyword names from std::any::type_name."
                    .to_string(),
                evidence_excerpt: "Rejected std::any::type_name parsing.".to_string(),
                applies_to: GuidanceAppliesTo {
                    paths: vec!["src/target.ts".to_string()],
                    symbols: Vec::new(),
                },
                confidence: GuidanceFactConfidence::High,
            },
            GuidanceFactDraft {
                category: GuidanceFactCategory::Verification,
                kind: "test_success".to_string(),
                guidance: "The context guidance path was verified with nextest.".to_string(),
                evidence_excerpt: "cargo nextest run --lib context_guidance passed.".to_string(),
                applies_to: GuidanceAppliesTo {
                    paths: vec!["src/target.ts".to_string()],
                    symbols: Vec::new(),
                },
                confidence: GuidanceFactConfidence::High,
            },
        ],
    };
    repository
        .persist_history_guidance_distillation(
            repo_id.as_str(),
            &input,
            &output,
            Some("gpt-guidance"),
            Some("guidance-profile"),
        )
        .expect("persist context guidance");
}

pub(super) fn seed_graphql_mutation_repo() -> TempDir {
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
                }
            }
        }),
    );

    dir
}

#[cfg(feature = "slow-tests")]
pub(super) fn seed_graphql_knowledge_mutation_repo(jira_site_url: &str) -> TempDir {
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
                }
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

#[cfg(feature = "slow-tests")]
pub(super) fn knowledge_duckdb_path(repo_root: &Path) -> PathBuf {
    crate::config::resolve_store_backend_config_for_repo(repo_root)
        .expect("resolve backend config")
        .events
        .resolve_duckdb_db_path_for_repo(repo_root)
}

pub(super) fn seed_graphql_chat_history_data(repo_root: &Path) {
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
pub(super) struct SeededKnowledgeFixture {
    pub(super) primary_item_id: String,
    pub(super) primary_latest_version_id: String,
    pub(super) secondary_item_id: String,
    pub(super) secondary_latest_version_id: String,
}

pub(super) fn seed_graphql_knowledge_data(repo_root: &Path) -> SeededKnowledgeFixture {
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
