use super::*;
use crate::capability_packs::context_guidance::types::{
    GuidanceAppliesTo, GuidanceFactDraft, GuidanceSessionSummary,
};
use crate::host::capability_host::{CapabilityMigrationContext, MigrationRunner};
use crate::host::devql::RepoIdentity;
use std::path::{Path, PathBuf};

fn sqlite_pool_with_guidance_schema() -> SqliteConnectionPool {
    let temp = tempfile::NamedTempFile::new().expect("temp db");
    let path = temp.into_temp_path().keep().expect("keep temp db");
    let sqlite = SqliteConnectionPool::connect(path).expect("sqlite");
    sqlite
        .execute_batch(context_guidance_sqlite_schema_sql())
        .expect("schema");
    sqlite
}

fn repository() -> SqliteContextGuidanceRepository {
    SqliteContextGuidanceRepository::new(sqlite_pool_with_guidance_schema())
}

fn input(path: &str) -> GuidanceDistillationInput {
    GuidanceDistillationInput {
        checkpoint_id: Some("checkpoint-1".to_string()),
        session_id: "session-1".to_string(),
        turn_id: Some("turn-1".to_string()),
        event_time: Some("2026-04-29T10:00:00Z".to_string()),
        agent_type: Some("codex".to_string()),
        model: Some("gpt-5.4".to_string()),
        prompt: Some("Improve attr parsing".to_string()),
        transcript_fragment: Some("Rejected std::any::type_name approach".to_string()),
        files_modified: vec![path.to_string()],
        tool_events: vec![GuidanceToolEvidence {
            tool_kind: Some("shell".to_string()),
            input_summary: Some("cargo nextest".to_string()),
            output_summary: Some("tests passed".to_string()),
            command: Some("cargo nextest".to_string()),
        }],
    }
}

fn output(path: &str, category: GuidanceFactCategory, kind: &str) -> GuidanceDistillationOutput {
    GuidanceDistillationOutput {
        summary: GuidanceSessionSummary {
            intent: "Improve attribute parsing.".to_string(),
            outcome: "Replaced fragile parsing.".to_string(),
            decisions: vec!["Use token rendering.".to_string()],
            rejected_approaches: vec!["Do not use type_name.".to_string()],
            patterns: Vec::new(),
            verification: vec!["cargo nextest passed.".to_string()],
            open_items: Vec::new(),
        },
        guidance_facts: vec![GuidanceFactDraft {
            category,
            kind: kind.to_string(),
            guidance: "Do not derive keyword names from std::any::type_name.".to_string(),
            evidence_excerpt: "Rejected std::any::type_name approach.".to_string(),
            applies_to: GuidanceAppliesTo {
                paths: vec![path.to_string()],
                symbols: Vec::new(),
            },
            confidence: GuidanceFactConfidence::High,
        }],
    }
}

fn list_input_for_path(path: &str) -> ListSelectedContextGuidanceInput {
    ListSelectedContextGuidanceInput {
        repo_id: "repo-1".to_string(),
        selected_paths: vec![path.to_string()],
        selected_symbol_ids: Vec::new(),
        selected_symbol_fqns: Vec::new(),
        agent: None,
        since: None,
        evidence_kind: None,
        category: None,
        kind: None,
        limit: 10,
    }
}

#[test]
fn persist_and_query_guidance_by_path_and_filters() {
    let repo = repository();
    let input = input("src/target.rs");
    let output = output(
        "src/target.rs",
        GuidanceFactCategory::Decision,
        "rejected_approach",
    );

    let outcome = repo
        .persist_history_guidance_distillation(
            "repo-1",
            &input,
            &output,
            Some("guidance-model"),
            Some("guidance-profile"),
        )
        .expect("persist");

    assert!(outcome.inserted_run);
    assert_eq!(outcome.inserted_facts, 1);
    assert!(!outcome.unchanged);

    let rows = repo
        .list_selected_context_guidance(ListSelectedContextGuidanceInput {
            repo_id: "repo-1".to_string(),
            selected_paths: vec!["src/target.rs".to_string()],
            selected_symbol_ids: Vec::new(),
            selected_symbol_fqns: Vec::new(),
            agent: Some("codex".to_string()),
            since: Some("2026-04-29T09:00:00Z".to_string()),
            evidence_kind: Some("FILE_RELATION".to_string()),
            category: Some(GuidanceFactCategory::Decision),
            kind: Some("rejected_approach".to_string()),
            limit: 10,
        })
        .expect("query");

    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].source_model.as_deref(), Some("guidance-model"));
    assert_eq!(rows[0].targets[0].target_value, "src/target.rs");
    assert_eq!(rows[0].sources[0].source_type, "history.turn");
}

#[test]
fn persist_and_query_guidance_by_historical_evidence_kind() {
    let repo = repository();
    let input = input("src/target.rs");
    let output = output(
        "src/target.rs",
        GuidanceFactCategory::Decision,
        "rejected_approach",
    );

    repo.persist_history_guidance_distillation("repo-1", &input, &output, None, None)
        .expect("persist");
    let mut list_input = list_input_for_path("src/target.rs");
    list_input.evidence_kind = Some("FILE_RELATION".to_string());
    let rows = repo
        .list_selected_context_guidance(list_input)
        .expect("query evidence kind");

    assert_eq!(rows.len(), 1);
    assert_eq!(
        rows[0].sources[0].evidence_kind.as_deref(),
        Some("FILE_RELATION")
    );
}

#[test]
fn query_guidance_by_symbol_fqn() {
    let repo = repository();
    let input = input("src/target.rs");
    let output = GuidanceDistillationOutput {
        summary: output(
            "src/target.rs",
            GuidanceFactCategory::Decision,
            "rejected_approach",
        )
        .summary,
        guidance_facts: vec![GuidanceFactDraft {
            category: GuidanceFactCategory::Pattern,
            kind: "implementation_pattern".to_string(),
            guidance: "Keep parser state transitions centralized.".to_string(),
            evidence_excerpt: "Centralized the parser state transition.".to_string(),
            applies_to: GuidanceAppliesTo {
                paths: Vec::new(),
                symbols: vec!["crate::parser::parse_attr".to_string()],
            },
            confidence: GuidanceFactConfidence::Medium,
        }],
    };

    repo.persist_history_guidance_distillation("repo-1", &input, &output, None, None)
        .expect("persist");
    let rows = repo
        .list_selected_context_guidance(ListSelectedContextGuidanceInput {
            repo_id: "repo-1".to_string(),
            selected_paths: Vec::new(),
            selected_symbol_ids: Vec::new(),
            selected_symbol_fqns: vec!["crate::parser::parse_attr".to_string()],
            agent: None,
            since: None,
            evidence_kind: None,
            category: Some(GuidanceFactCategory::Pattern),
            kind: Some("implementation_pattern".to_string()),
            limit: 10,
        })
        .expect("query");

    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].targets[0].target_type, "symbol_fqn");
}

#[test]
fn query_returns_no_rows_for_unrelated_path_or_late_since_filter() {
    let repo = repository();
    let input = input("src/target.rs");
    let output = output(
        "src/target.rs",
        GuidanceFactCategory::Decision,
        "rejected_approach",
    );

    repo.persist_history_guidance_distillation("repo-1", &input, &output, None, None)
        .expect("persist");

    let unrelated_rows = repo
        .list_selected_context_guidance(list_input_for_path("src/other.rs"))
        .expect("query unrelated");
    assert!(unrelated_rows.is_empty());

    let late_rows = repo
        .list_selected_context_guidance(ListSelectedContextGuidanceInput {
            since: Some("2026-04-30T00:00:00Z".to_string()),
            ..list_input_for_path("src/target.rs")
        })
        .expect("query late since");
    assert!(late_rows.is_empty());
}

#[test]
fn persisting_same_scope_and_hash_is_noop() {
    let repo = repository();
    let input = input("src/target.rs");
    let output = output(
        "src/target.rs",
        GuidanceFactCategory::Decision,
        "rejected_approach",
    );

    repo.persist_history_guidance_distillation("repo-1", &input, &output, None, None)
        .expect("first persist");
    let second = repo
        .persist_history_guidance_distillation("repo-1", &input, &output, None, None)
        .expect("second persist");

    assert!(second.unchanged);
    assert!(!second.inserted_run);
    assert_eq!(second.inserted_facts, 0);
}

#[test]
fn changed_input_inactivates_old_facts() {
    let repo = repository();
    let first_input = input("src/target.rs");
    let second_input = GuidanceDistillationInput {
        transcript_fragment: Some("New transcript".to_string()),
        ..input("src/target.rs")
    };
    let output = output(
        "src/target.rs",
        GuidanceFactCategory::Decision,
        "rejected_approach",
    );

    repo.persist_history_guidance_distillation("repo-1", &first_input, &output, None, None)
        .expect("first persist");
    repo.persist_history_guidance_distillation("repo-1", &second_input, &output, None, None)
        .expect("second persist");

    let rows = repo
        .list_selected_context_guidance(ListSelectedContextGuidanceInput {
            repo_id: "repo-1".to_string(),
            selected_paths: vec!["src/target.rs".to_string()],
            selected_symbol_ids: Vec::new(),
            selected_symbol_fqns: Vec::new(),
            agent: None,
            since: None,
            evidence_kind: None,
            category: None,
            kind: None,
            limit: 10,
        })
        .expect("query");

    assert_eq!(rows.len(), 1);
}

#[test]
fn persistence_does_not_modify_checkpoint_session_summary() {
    let sqlite = sqlite_pool_with_guidance_schema();
    sqlite
        .execute_batch(
            "CREATE TABLE checkpoint_sessions (
                    session_id TEXT PRIMARY KEY,
                    summary TEXT
                 );
                 INSERT INTO checkpoint_sessions (session_id, summary)
                 VALUES ('session-1', 'legacy summary');",
        )
        .expect("checkpoint table");
    let repo = SqliteContextGuidanceRepository::new(sqlite.clone());
    let input = input("src/target.rs");
    let output = output(
        "src/target.rs",
        GuidanceFactCategory::Decision,
        "rejected_approach",
    );

    repo.persist_history_guidance_distillation("repo-1", &input, &output, None, None)
        .expect("persist");

    let summary = sqlite
        .with_connection(|conn| {
            conn.query_row(
                "SELECT summary FROM checkpoint_sessions WHERE session_id = 'session-1'",
                [],
                |row| row.get::<_, String>(0),
            )
            .map_err(anyhow::Error::from)
        })
        .expect("summary");
    assert_eq!(summary, "legacy summary");
}

struct MigrationTestContext {
    repo: RepoIdentity,
    repo_root: PathBuf,
    sqlite: SqliteConnectionPool,
}

impl CapabilityMigrationContext for MigrationTestContext {
    fn repo(&self) -> &RepoIdentity {
        &self.repo
    }

    fn repo_root(&self) -> &Path {
        &self.repo_root
    }

    fn apply_devql_sqlite_ddl(&self, sql: &str) -> Result<()> {
        self.sqlite.execute_batch(sql)
    }
}

#[test]
fn migration_initializes_tables_indexes_and_attribution_columns() {
    let temp = tempfile::NamedTempFile::new().expect("temp db");
    let path = temp.into_temp_path().keep().expect("keep temp db");
    let sqlite = SqliteConnectionPool::connect(path).expect("sqlite");
    let mut ctx = MigrationTestContext {
        repo: RepoIdentity {
            provider: "local".to_string(),
            organization: "bitloops".to_string(),
            name: "repo".to_string(),
            identity: "local/repo".to_string(),
            repo_id: "repo-1".to_string(),
        },
        repo_root: PathBuf::from("."),
        sqlite: sqlite.clone(),
    };

    match super::super::migrations::CONTEXT_GUIDANCE_MIGRATIONS[0].run {
        MigrationRunner::Core(run) => run(&mut ctx).expect("migration"),
        MigrationRunner::Knowledge(_) => panic!("context guidance migration must be core"),
    }

    let table_names = [
        "context_guidance_distillation_runs",
        "context_guidance_facts",
        "context_guidance_sources",
        "context_guidance_targets",
    ];
    let index_names = [
        "context_guidance_runs_scope_input_idx",
        "context_guidance_runs_scope_idx",
        "context_guidance_facts_repo_category_idx",
        "context_guidance_facts_run_idx",
        "context_guidance_sources_guidance_idx",
        "context_guidance_sources_history_idx",
        "context_guidance_sources_filter_idx",
        "context_guidance_sources_knowledge_idx",
        "context_guidance_targets_lookup_idx",
        "context_guidance_targets_guidance_idx",
    ];

    sqlite
        .with_connection(|conn| {
            for table in table_names {
                let count: i64 = conn.query_row(
                    "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
                    params![table],
                    |row| row.get(0),
                )?;
                assert_eq!(count, 1, "missing table {table}");
            }
            for index in index_names {
                let count: i64 = conn.query_row(
                    "SELECT COUNT(*) FROM sqlite_master WHERE type = 'index' AND name = ?1",
                    params![index],
                    |row| row.get(0),
                )?;
                assert_eq!(count, 1, "missing index {index}");
            }
            let columns = conn
                .prepare("PRAGMA table_info(context_guidance_distillation_runs)")?
                .query_map([], |row| row.get::<_, String>(1))?
                .collect::<Result<Vec<_>, _>>()?;
            assert!(columns.iter().any(|column| column == "capability_id"));
            assert!(columns.iter().any(|column| column == "capability_version"));
            Ok(())
        })
        .expect("inspect schema");
}
