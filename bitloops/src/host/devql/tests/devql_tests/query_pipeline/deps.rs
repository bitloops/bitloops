use super::super::*;

#[test]
fn parse_devql_deps_stage_basic() {
    let parsed = parse_devql_query(
        r#"repo("bitloops-cli")->file("src/main.ts")->artefacts(kind:"function")->deps(kind:"calls",direction:"both",include_unresolved:false)->limit(25)"#,
    )
    .unwrap();

    assert!(parsed.has_deps_stage);
    assert_eq!(parsed.deps.kind, Some(DepsKind::Calls));
    assert_eq!(parsed.deps.direction, DepsDirection::Both);
    assert!(!parsed.deps.include_unresolved);
    assert_eq!(parsed.limit, 25);
}

#[tokio::test]
async fn execute_devql_query_rejects_combining_deps_and_checkpoints_stage() {
    let cfg = test_cfg();
    let events_cfg = default_events_cfg();
    let parsed = parse_devql_query(r#"repo("temp2")->checkpoints()->artefacts()->deps()"#).unwrap();
    let err = execute_devql_query(&cfg, &parsed, &events_cfg, None)
        .await
        .unwrap_err();
    assert!(
        err.to_string()
            .contains("MVP limitation: telemetry/checkpoints stages cannot be combined")
    );
}

#[test]
fn parse_devql_deps_stage_accepts_all_v1_edge_kinds() {
    for kind in [
        "imports",
        "calls",
        "references",
        "extends",
        "implements",
        "exports",
    ] {
        let parsed = parse_devql_query(&format!(
            r#"repo("bitloops-cli")->artefacts(kind:"function")->deps(kind:"{kind}")->limit(5)"#
        ))
        .unwrap();

        assert_eq!(parsed.deps.kind, DepsKind::from_str(kind));
    }

    let legacy = parse_devql_query(
        r#"repo("bitloops-cli")->artefacts(kind:"function")->deps(kind:"inherits")->limit(5)"#,
    )
    .unwrap();
    assert_eq!(legacy.deps.kind, Some(DepsKind::Extends));
}

#[test]
fn build_postgres_deps_query_respects_direction_and_unresolved_filters() {
    let cfg = test_cfg();
    let out = parse_devql_query(
        r#"repo("bitloops-cli")->file("src/main.ts")->artefacts(kind:"function")->deps(kind:"calls",direction:"out",include_unresolved:false)->limit(5)"#,
    )
    .unwrap();
    let in_query = parse_devql_query(
        r#"repo("bitloops-cli")->artefacts(kind:"interface")->deps(kind:"references",direction:"in")->limit(5)"#,
    )
    .unwrap();
    let both = parse_devql_query(
        r#"repo("bitloops-cli")->artefacts()->deps(kind:"exports",direction:"both")->limit(5)"#,
    )
    .unwrap();

    let out_sql = build_postgres_deps_query(&cfg, &out, &cfg.repo.repo_id).unwrap();
    let in_sql = build_postgres_deps_query(&cfg, &in_query, &cfg.repo.repo_id).unwrap();
    let both_sql = build_postgres_deps_query(&cfg, &both, &cfg.repo.repo_id).unwrap();

    assert!(out_sql.contains("e.edge_kind = 'calls'"));
    assert!(out_sql.contains("e.to_artefact_id IS NOT NULL"));
    assert!(
        out_sql.contains("LEFT JOIN artefacts_current at ON at.artefact_id = e.to_artefact_id")
    );
    assert!(!out_sql.contains(" a."));

    assert!(in_sql.contains("e.edge_kind = 'references'"));
    assert!(in_sql.contains("JOIN artefacts_current at ON at.artefact_id = e.to_artefact_id"));
    assert!(!in_sql.contains("WITH out_edges AS"));

    assert!(both_sql.contains("e.edge_kind = 'exports'"));
    assert!(both_sql.contains("FROM artefact_edges_current e JOIN artefacts_current a"));
    assert!(both_sql.contains("WITH out_edges AS"));
    assert!(both_sql.contains("UNION ALL"));
    assert!(both_sql.contains("SELECT DISTINCT"));
}

#[test]
fn build_postgres_deps_query_uses_historical_tables_for_asof_queries() {
    let cfg = test_cfg();
    let parsed = parse_devql_query(
        r#"repo("bitloops-cli")->asOf(commit:"abc123")->file("src/main.ts")->artefacts(kind:"function")->deps(kind:"calls")->limit(5)"#,
    )
    .unwrap();

    let sql = build_postgres_deps_query(&cfg, &parsed, &cfg.repo.repo_id).unwrap();

    assert!(sql.contains("FROM artefact_edges e"));
    assert!(sql.contains(
        "JOIN artefacts_historical af ON af.artefact_id = e.from_artefact_id"
    ));
    assert!(sql.contains(
        "LEFT JOIN artefacts_historical at ON at.artefact_id = e.to_artefact_id"
    ));
    assert!(!sql.contains("artefact_edges_current"));
    assert!(!sql.contains("artefacts_current"));
    assert!(!sql.contains("a.revision_kind"));
    assert!(!sql.contains("a.revision_id"));
    assert!(!sql.contains("e.revision_kind"));
    assert!(!sql.contains("e.revision_id"));
    assert!(!sql.contains("current_scope"));
}

#[test]
fn build_postgres_deps_query_uses_sync_shaped_current_tables_for_save_revision() {
    let cfg = test_cfg();
    let parsed = parse_devql_query(
        r#"repo("bitloops-cli")->asOf(saveRevision:"temp:42")->artefacts(kind:"function")->deps(kind:"calls",direction:"both")->limit(10)"#,
    )
    .unwrap();

    let sql = build_postgres_deps_query(&cfg, &parsed, &cfg.repo.repo_id).unwrap();

    assert!(sql.contains("FROM artefact_edges_current e"));
    assert!(sql.contains("JOIN artefacts_current a ON a.artefact_id = e.from_artefact_id"));
    assert!(sql.contains("AND a.repo_id = e.repo_id"));
    assert!(!sql.contains("e.revision_kind"));
    assert!(!sql.contains("e.revision_id"));
    assert!(!sql.contains("a.revision_kind"));
    assert!(!sql.contains("a.revision_id"));
    assert!(!sql.contains("e.branch"));
    assert!(!sql.contains("FROM artefact_edges e"));
    assert!(!sql.contains("JOIN artefacts a ON a.artefact_id = e.from_artefact_id"));
}

#[test]
fn build_postgres_deps_query_supports_symbol_fqn_filter() {
    let cfg = test_cfg();
    let parsed = parse_devql_query(
        r#"repo("rust-example")->artefacts(kind:"method",symbol_fqn:"hello_rust/src/main.rs::impl@1::handle_factorial")->deps(kind:"calls",direction:"out")->limit(20)"#,
    )
    .unwrap();

    let sql = build_postgres_deps_query(&cfg, &parsed, &cfg.repo.repo_id).unwrap();

    assert!(sql.contains("af.symbol_fqn = 'hello_rust/src/main.rs::impl@1::handle_factorial'"));
}

#[test]
fn build_postgres_deps_query_rejects_invalid_direction() {
    let err = parse_devql_query(
        r#"repo("bitloops-cli")->artefacts()->deps(kind:"calls",direction:"sideways")->limit(5)"#,
    )
    .unwrap_err();
    assert!(
        err.to_string()
            .contains("deps(direction:...) must be one of: out, in, both")
    );
}

#[test]
fn build_postgres_deps_query_rejects_invalid_kind() {
    let err =
        parse_devql_query(r#"repo("bitloops-cli")->artefacts()->deps(kind:"surprise")->limit(5)"#)
            .unwrap_err();
    assert!(err.to_string().contains(
        "deps(kind:...) must be one of: imports, calls, references, extends, implements, exports"
    ));
}

#[tokio::test]
async fn execute_devql_query_rejects_combining_deps_and_chat_history_stage() {
    let cfg = test_cfg();
    let events_cfg = default_events_cfg();
    let parsed = parse_devql_query(
        r#"repo("temp2")->artefacts(kind:"function")->deps(kind:"calls")->chatHistory()->limit(1)"#,
    )
    .unwrap();
    let err = execute_devql_query(&cfg, &parsed, &events_cfg, None)
        .await
        .unwrap_err();
    assert!(
        err.to_string()
            .contains("deps() cannot be combined with chatHistory()")
    );
}
