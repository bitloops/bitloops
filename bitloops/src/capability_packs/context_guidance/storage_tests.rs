use super::*;
use crate::capability_packs::context_guidance::distillation::GuidanceToolEvidence;
use crate::capability_packs::context_guidance::types::{
    GuidanceAppliesTo, GuidanceFactDraft, GuidanceSessionSummary,
};

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
fn persist_knowledge_guidance_records_knowledge_sources() {
    let repo = repository();
    let input = KnowledgeGuidanceDistillationInput {
        knowledge_item_id: "item-1".to_string(),
        knowledge_item_version_id: "version-1".to_string(),
        relation_assertion_id: Some("relation-1".to_string()),
        provider: "github".to_string(),
        source_kind: "github_issue".to_string(),
        title: Some("Issue title".to_string()),
        url: Some("https://github.com/org/repo/issues/1".to_string()),
        updated_at: Some("2026-04-30T10:00:00Z".to_string()),
        body_preview: Some("Keep parser boundary centralized.".to_string()),
        normalized_fields_json: "{}".to_string(),
        target_paths: vec!["src/target.rs".to_string()],
        target_symbols: Vec::new(),
    };
    let mut output = output(
        "src/target.rs",
        GuidanceFactCategory::Decision,
        "preserve_parser_boundary",
    );
    output.guidance_facts[0].applies_to.paths.clear();

    repo.persist_knowledge_guidance_distillation("repo-1", &input, &output, None, None)
        .expect("persist");

    let rows = repo
        .list_selected_context_guidance(list_input_for_path("src/target.rs"))
        .expect("query");

    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].sources[0].source_type, "knowledge.item_version");
    assert_eq!(
        rows[0].sources[0].knowledge_item_id.as_deref(),
        Some("item-1")
    );
    assert_eq!(
        rows[0].sources[0].knowledge_item_version_id.as_deref(),
        Some("version-1")
    );
    assert_eq!(
        rows[0].sources[0].relation_assertion_id.as_deref(),
        Some("relation-1")
    );
    assert_eq!(
        rows[0].sources[0].url.as_deref(),
        Some("https://github.com/org/repo/issues/1")
    );
    assert_eq!(rows[0].targets[0].target_value, "src/target.rs");
}

#[test]
fn target_compaction_marks_duplicate_facts_inactive() {
    let repo = repository();
    let first_input = input("src/target.rs");
    let second_input = GuidanceDistillationInput {
        turn_id: Some("turn-2".to_string()),
        transcript_fragment: Some("Repeated decision with newer wording.".to_string()),
        ..input("src/target.rs")
    };
    let first_output = output(
        "src/target.rs",
        GuidanceFactCategory::Decision,
        "preserve_parser_boundary",
    );
    let second_output = output(
        "src/target.rs",
        GuidanceFactCategory::Decision,
        "preserve_parser_boundary",
    );

    repo.persist_history_guidance_distillation("repo-1", &first_input, &first_output, None, None)
        .expect("first persist");
    repo.persist_history_guidance_distillation("repo-1", &second_input, &second_output, None, None)
        .expect("second persist");

    let rows = repo
        .list_active_guidance_for_target("repo-1", "path", "src/target.rs", 10)
        .expect("list target");
    assert_eq!(rows.len(), 2);

    let retained = rows[0].guidance_id.clone();
    let duplicate = rows[1].guidance_id.clone();
    let outcome = repo
        .apply_target_compaction(
            "repo-1",
            ApplyTargetCompactionInput {
                compaction_run_id: "compaction-1".to_string(),
                target_type: "path".to_string(),
                target_value: "src/target.rs".to_string(),
                retained_guidance_ids: vec![retained.clone()],
                duplicate_guidance_ids: vec![duplicate],
                superseded_guidance_ids: Vec::new(),
                summary_json: r#"{"summary":"Keep parser boundary guidance."}"#.to_string(),
            },
        )
        .expect("compact");

    assert_eq!(outcome.retained_count, 1);
    assert_eq!(outcome.compacted_count, 1);
    let rows = repo
        .list_selected_context_guidance(list_input_for_path("src/target.rs"))
        .expect("query");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].guidance_id, retained);
}

#[test]
fn verification_sources_are_relevant_deduplicated_and_capped() {
    let repo = repository();
    let mut input = input("src/target.rs");
    input.tool_events = vec![
        GuidanceToolEvidence {
            tool_kind: Some("Bash".to_string()),
            input_summary: Some("cargo nextest run -p axum-macros debug_handler".to_string()),
            output_summary: Some("debug_handler Self receiver regression check passed".to_string()),
            command: Some("cargo nextest run -p axum-macros debug_handler".to_string()),
        },
        GuidanceToolEvidence {
            tool_kind: Some("Bash".to_string()),
            input_summary: Some("cargo nextest run -p axum-macros debug_handler".to_string()),
            output_summary: Some("debug_handler Self receiver regression check passed".to_string()),
            command: Some("cargo nextest run -p axum-macros debug_handler".to_string()),
        },
        GuidanceToolEvidence {
            tool_kind: Some("Read".to_string()),
            input_summary: Some("src/unrelated.rs".to_string()),
            output_summary: Some("receiver behavior regression notes".to_string()),
            command: None,
        },
        GuidanceToolEvidence {
            tool_kind: Some("Bash".to_string()),
            input_summary: Some("cargo clippy -p axum-macros".to_string()),
            output_summary: None,
            command: Some("cargo clippy -p axum-macros".to_string()),
        },
        GuidanceToolEvidence {
            tool_kind: Some("Bash".to_string()),
            input_summary: Some(
                "cargo nextest run -p axum-macros debug_handler secondary".to_string(),
            ),
            output_summary: Some("debug_handler secondary regression check passed".to_string()),
            command: Some("cargo nextest run -p axum-macros debug_handler secondary".to_string()),
        },
    ];
    let output = GuidanceDistillationOutput {
        summary: output(
            "src/target.rs",
            GuidanceFactCategory::Decision,
            "fixture_summary",
        )
        .summary,
        guidance_facts: vec![GuidanceFactDraft {
            category: GuidanceFactCategory::Verification,
            kind: "debug_handler_self_receiver_regression_check".to_string(),
            guidance: "Run cargo nextest and cargo clippy for debug_handler Self receiver cases because macro receiver behavior can regress.".to_string(),
            evidence_excerpt: "debug_handler Self receiver regression check passed, and cargo clippy was run.".to_string(),
            applies_to: GuidanceAppliesTo {
                paths: vec!["src/target.rs".to_string()],
                symbols: Vec::new(),
            },
            confidence: GuidanceFactConfidence::High,
        }],
    };

    repo.persist_history_guidance_distillation("repo-1", &input, &output, None, None)
        .expect("persist");

    let rows = repo
        .list_selected_context_guidance(list_input_for_path("src/target.rs"))
        .expect("query");

    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].sources.len(), 3);
    assert_eq!(rows[0].sources[0].source_type, "history.turn");
    assert_eq!(rows[0].sources[1].source_type, "history.tool_event");
    assert_eq!(
        rows[0].sources[1].excerpt.as_deref(),
        Some("debug_handler Self receiver regression check passed")
    );
    assert_eq!(rows[0].sources[2].source_type, "history.tool_event");
    assert_eq!(rows[0].sources[2].tool_kind.as_deref(), Some("Bash"));
    assert_eq!(rows[0].sources[2].excerpt, None);
    assert!(
        rows[0]
            .sources
            .iter()
            .all(|source| source.excerpt.as_deref() != Some("receiver behavior regression notes"))
    );
    assert!(
        rows[0]
            .sources
            .iter()
            .all(|source| source.excerpt.as_deref()
                != Some("debug_handler secondary regression check passed"))
    );
}

#[test]
fn query_orders_higher_value_guidance_before_weaker_categories() {
    let repo = repository();
    let input = input("src/target.rs");
    let output = GuidanceDistillationOutput {
        summary: output(
            "src/target.rs",
            GuidanceFactCategory::Decision,
            "fixture_summary",
        )
        .summary,
        guidance_facts: vec![
            GuidanceFactDraft {
                category: GuidanceFactCategory::Verification,
                kind: "specific_regression_check".to_string(),
                guidance: "Run cargo nextest parser regression cases because macro behavior can regress.".to_string(),
                evidence_excerpt: "Ran cargo nextest parser regression cases and confirmed behavior.".to_string(),
                applies_to: GuidanceAppliesTo {
                    paths: vec!["src/target.rs".to_string()],
                    symbols: Vec::new(),
                },
                confidence: GuidanceFactConfidence::High,
            },
            GuidanceFactDraft {
                category: GuidanceFactCategory::Decision,
                kind: "preserve_parser_boundary".to_string(),
                guidance: "Keep parser boundary logic centralized so future changes do not split validation across call sites.".to_string(),
                evidence_excerpt: "Decision: parser boundary logic was centralized to avoid split validation.".to_string(),
                applies_to: GuidanceAppliesTo {
                    paths: vec!["src/target.rs".to_string()],
                    symbols: Vec::new(),
                },
                confidence: GuidanceFactConfidence::High,
            },
        ],
    };

    repo.persist_history_guidance_distillation("repo-1", &input, &output, None, None)
        .expect("persist");

    let rows = repo
        .list_selected_context_guidance(ListSelectedContextGuidanceInput {
            limit: 1,
            ..list_input_for_path("src/target.rs")
        })
        .expect("query");

    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].kind, "preserve_parser_boundary");
}

#[test]
fn persisted_guidance_facts_include_lifecycle_metadata() {
    let repo = repository();
    let input = input("src/target.rs");
    let output = output(
        "src/target.rs",
        GuidanceFactCategory::Decision,
        "preserve_parser_boundary",
    );

    repo.persist_history_guidance_distillation("repo-1", &input, &output, None, None)
        .expect("persist");

    let rows = repo
        .list_selected_context_guidance(list_input_for_path("src/target.rs"))
        .expect("query");

    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].lifecycle_status, "active");
    assert!(!rows[0].fact_fingerprint.is_empty());
    assert!(rows[0].value_score > 0.0);
}

#[test]
fn list_selected_context_guidance_excludes_superseded_lifecycle_facts() {
    let sqlite = sqlite_pool_with_guidance_schema();
    let repo = SqliteContextGuidanceRepository::new(sqlite.clone());
    let input = input("src/target.rs");
    let output = output(
        "src/target.rs",
        GuidanceFactCategory::Decision,
        "preserve_parser_boundary",
    );

    repo.persist_history_guidance_distillation("repo-1", &input, &output, None, None)
        .expect("persist");
    sqlite
        .execute_batch(
            "UPDATE context_guidance_facts
             SET lifecycle_status = 'superseded', superseded_by_guidance_id = 'new-guidance'
             WHERE repo_id = 'repo-1';",
        )
        .expect("mark superseded");

    let rows = repo
        .list_selected_context_guidance(list_input_for_path("src/target.rs"))
        .expect("query");

    assert!(rows.is_empty());
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

#[path = "storage_migration_tests.rs"]
mod migration_tests;
