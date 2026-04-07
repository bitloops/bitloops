use super::super::*;
use serde_json::json;

#[test]
fn parse_devql_clone_summary_stage_basic() {
    let parsed =
        parse_devql_query(r#"repo("r")->artefacts(kind:"function")->clones()->summary()"#)
            .unwrap();

    assert!(parsed.has_artefacts_stage);
    assert!(parsed.has_clones_stage);
    assert_eq!(parsed.registered_stages.len(), 1);
    assert_eq!(parsed.registered_stages[0].stage_name, "summary");
}

#[tokio::test]
async fn execute_devql_query_rejects_summary_without_clones() {
    let cfg = test_cfg();
    let events_cfg = default_events_cfg();
    let parsed = parse_devql_query(r#"repo("temp2")->artefacts(kind:"function")->summary()"#)
        .unwrap();

    let err = execute_devql_query(&cfg, &parsed, &events_cfg, None)
        .await
        .unwrap_err();

    assert!(
        err.to_string()
            .contains("summary() requires a clones() stage")
    );
}

#[tokio::test]
async fn execute_devql_query_rejects_summary_with_additional_registered_stages() {
    let cfg = test_cfg();
    let events_cfg = default_events_cfg();
    let parsed = parse_devql_query(
        r#"repo("temp2")->artefacts(kind:"function")->clones()->summary()->knowledge()->limit(1)"#,
    )
    .unwrap();

    let err = execute_devql_query(&cfg, &parsed, &events_cfg, None)
        .await
        .unwrap_err();

    assert!(err.to_string().contains(
        "summary() cannot currently be combined with additional registered capability-pack stages"
    ));
}

#[tokio::test]
async fn execute_registered_summary_stage_returns_grouped_counts() {
    let cfg = test_cfg();
    let parsed = parse_devql_query(r#"repo("temp2")->artefacts()->clones()->summary()"#)
        .expect("parse clone summary query");

    let rows = execute_registered_stages(
        &cfg,
        &parsed,
        vec![
            json!({ "relation_kind": "similar_implementation" }),
            json!({ "relation_kind": "contextual_neighbor" }),
            json!({ "relation_kind": "similar_implementation" }),
        ],
    )
    .await
    .expect("execute registered summary stage");

    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["total_count"], 3);
    assert_eq!(rows[0]["groups"][0]["relation_kind"], "similar_implementation");
    assert_eq!(rows[0]["groups"][0]["count"], 2);
    assert_eq!(rows[0]["groups"][1]["relation_kind"], "contextual_neighbor");
    assert_eq!(rows[0]["groups"][1]["count"], 1);
}

#[tokio::test]
async fn execute_registered_summary_stage_returns_zero_summary_for_empty_clone_sets() {
    let cfg = test_cfg();
    let parsed = parse_devql_query(r#"repo("temp2")->artefacts()->clones()->summary()"#)
        .expect("parse clone summary query");

    let rows = execute_registered_stages(&cfg, &parsed, Vec::new())
        .await
        .expect("execute registered summary stage");

    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["total_count"], 0);
    assert_eq!(
        rows[0]["groups"].as_array().expect("groups array").len(),
        0
    );
}

#[tokio::test]
async fn clone_summary_ignores_limit_and_aggregates_full_filtered_clone_set() {
    let cfg = test_cfg();
    let events_cfg = default_events_cfg();
    let temp = tempdir().expect("tempdir");
    let sqlite_path = temp.path().join("relational.sqlite");
    let relational = sqlite_relational_store_with_schema(&sqlite_path).await;
    let conn = rusqlite::Connection::open(&sqlite_path).expect("open sqlite");
    let repo_id = cfg.repo.repo_id.as_str();

    for (symbol_id, artefact_id, path, symbol_fqn) in [
        (
            "sym::source",
            "artefact::source",
            "src/source.ts",
            "src/source.ts::source",
        ),
        (
            "sym::target_a",
            "artefact::target_a",
            "src/target-a.ts",
            "src/target-a.ts::targetA",
        ),
        (
            "sym::target_b",
            "artefact::target_b",
            "src/target-b.ts",
            "src/target-b.ts::targetB",
        ),
    ] {
        conn.execute(
            "INSERT INTO artefacts_current (
                repo_id, path, content_id, symbol_id, artefact_id, language,
                canonical_kind, language_kind, symbol_fqn, start_line, end_line, start_byte,
                end_byte, signature, modifiers, updated_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, 'typescript', 'function', 'function_declaration', ?6, 1, 8, 0, 64, ?7, '[]', '2026-03-26T09:00:00Z')",
            rusqlite::params![
                repo_id,
                path,
                format!("blob-{symbol_id}"),
                symbol_id,
                artefact_id,
                symbol_fqn,
                format!("function {}", symbol_fqn.rsplit("::").next().unwrap_or("run")),
            ],
        )
        .expect("insert current artefact");
    }

    for (target_symbol_id, target_artefact_id, relation_kind, score) in [
        (
            "sym::target_a",
            "artefact::target_a",
            "similar_implementation",
            0.91_f64,
        ),
        (
            "sym::target_b",
            "artefact::target_b",
            "exact_duplicate",
            0.89_f64,
        ),
    ] {
        conn.execute(
            "INSERT INTO symbol_clone_edges (
                repo_id, source_symbol_id, source_artefact_id, target_symbol_id,
                target_artefact_id, relation_kind, score, semantic_score, lexical_score,
                structural_score, clone_input_hash, explanation_json
            ) VALUES (?1, 'sym::source', 'artefact::source', ?2, ?3, ?4, ?5, ?5, 0.6, 0.5, ?6, '{}')",
            rusqlite::params![
                repo_id,
                target_symbol_id,
                target_artefact_id,
                relation_kind,
                score,
                format!("clone-hash-{target_symbol_id}"),
            ],
        )
        .expect("insert clone edge");
    }

    let parsed = parse_devql_query(
        r#"repo("temp2")->artefacts(kind:"function",symbol_fqn:"src/source.ts::source")->clones(min_score:0.5)->summary()->limit(1)"#,
    )
    .expect("parse clone summary query");

    let clone_rows = execute_devql_query(&cfg, &parsed, &events_cfg, Some(&relational))
        .await
        .expect("execute clone query before summary aggregation");
    assert_eq!(clone_rows.len(), 2, "summary should ignore limit()");

    let summary_rows = execute_registered_stages(&cfg, &parsed, clone_rows)
        .await
        .expect("execute summary stage");
    assert_eq!(summary_rows.len(), 1);
    assert_eq!(summary_rows[0]["total_count"], 2);
    assert_eq!(summary_rows[0]["groups"][0]["count"], 1);
    assert_eq!(summary_rows[0]["groups"][1]["count"], 1);
}
