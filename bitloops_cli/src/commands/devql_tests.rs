use super::*;
use clap::Parser;
use std::env;

fn test_cfg() -> DevqlConfig {
    DevqlConfig {
        repo_root: PathBuf::from("/tmp/repo"),
        repo: RepoIdentity {
            provider: "github".to_string(),
            organization: "bitloops".to_string(),
            name: "temp2".to_string(),
            identity: "github/bitloops/temp2".to_string(),
            repo_id: deterministic_uuid("repo://github/bitloops/temp2"),
        },
        pg_dsn: None,
        clickhouse_url: "http://localhost:8123".to_string(),
        clickhouse_user: None,
        clickhouse_password: None,
        clickhouse_database: "default".to_string(),
    }
}

#[test]
fn parse_devql_pipeline_basic() {
    let parsed = parse_devql_query(
        r#"repo("bitloops-cli")->asOf(ref:"main")->file("src/main.rs")->artefacts(lines:1..50,kind:"file",agent:"claude-code",since:"2026-03-01")->select(path,canonical_kind)->limit(10)"#,
    )
    .unwrap();

    assert_eq!(parsed.repo.as_deref(), Some("bitloops-cli"));
    assert!(matches!(parsed.as_of, Some(AsOfSelector::Ref(ref v)) if v == "main"));
    assert_eq!(parsed.file.as_deref(), Some("src/main.rs"));
    assert_eq!(parsed.artefacts.kind.as_deref(), Some("file"));
    assert_eq!(parsed.artefacts.lines, Some((1, 50)));
    assert_eq!(parsed.artefacts.agent.as_deref(), Some("claude-code"));
    assert_eq!(parsed.artefacts.since.as_deref(), Some("2026-03-01"));
    assert_eq!(parsed.limit, 10);
    assert_eq!(parsed.select_fields, vec!["path", "canonical_kind"]);
}

#[test]
fn parse_devql_checkpoints_basic() {
    let parsed = parse_devql_query(
        r#"repo("bitloops-cli")->checkpoints(agent:"claude-code",since:"2026-03-01")->select(checkpoint_id,created_at)->limit(5)"#,
    )
    .unwrap();

    assert!(parsed.has_checkpoints_stage);
    assert_eq!(parsed.checkpoints.agent.as_deref(), Some("claude-code"));
    assert_eq!(parsed.checkpoints.since.as_deref(), Some("2026-03-01"));
    assert_eq!(parsed.limit, 5);
}

#[test]
fn parse_devql_chat_history_stage_basic() {
    let parsed = parse_devql_query(
        r#"repo("bitloops-cli")->file("index.ts")->artefacts(lines:1..10)->chatHistory()->limit(3)"#,
    )
    .unwrap();

    assert!(parsed.has_artefacts_stage);
    assert!(parsed.has_chat_history_stage);
    assert_eq!(parsed.limit, 3);
}

#[test]
fn parse_devql_deps_stage_basic() {
    let parsed = parse_devql_query(
        r#"repo("bitloops-cli")->file("src/main.ts")->artefacts(kind:"function")->deps(kind:"calls",direction:"both",include_unresolved:false)->limit(25)"#,
    )
    .unwrap();

    assert!(parsed.has_deps_stage);
    assert_eq!(parsed.deps.kind.as_deref(), Some("calls"));
    assert_eq!(parsed.deps.direction, "both");
    assert!(!parsed.deps.include_unresolved);
    assert_eq!(parsed.limit, 25);
}

#[tokio::test]
async fn execute_devql_query_rejects_chat_history_without_artefacts_stage() {
    let cfg = test_cfg();
    let parsed = parse_devql_query(r#"repo("temp2")->chatHistory()->limit(1)"#).unwrap();
    let err = execute_devql_query(&cfg, &parsed, None).await.unwrap_err();
    assert!(
        err.to_string()
            .contains("chatHistory() requires an artefacts() stage")
    );
}

#[tokio::test]
async fn execute_devql_query_rejects_combining_checkpoints_and_artefacts_stage() {
    let cfg = test_cfg();
    let parsed =
        parse_devql_query(r#"repo("temp2")->checkpoints()->artefacts(agent:"claude-code")"#)
            .unwrap();
    let err = execute_devql_query(&cfg, &parsed, None).await.unwrap_err();
    assert!(
        err.to_string()
            .contains("MVP limitation: telemetry/checkpoints stages cannot be combined")
    );
}

#[tokio::test]
async fn execute_devql_query_rejects_combining_deps_and_chat_history_stage() {
    let cfg = test_cfg();
    let parsed = parse_devql_query(
        r#"repo("temp2")->artefacts(kind:"function")->deps(kind:"calls")->chatHistory()->limit(1)"#,
    )
    .unwrap();
    let err = execute_devql_query(&cfg, &parsed, None).await.unwrap_err();
    assert!(
        err.to_string()
            .contains("deps() cannot be combined with chatHistory()")
    );
}

#[test]
fn extract_chat_messages_from_transcript_parses_jsonl() {
    let transcript = r#"{"type":"user","message":{"content":"Fix index.ts"}}
{"type":"assistant","message":{"content":[{"type":"text","text":"Done"},{"type":"tool_use","name":"Edit"}]}}
{"type":"assistant","content":"Added tests"}"#;

    let messages = extract_chat_messages_from_transcript(transcript);
    assert_eq!(messages.len(), 3);
    assert_eq!(messages[0]["role"], "user");
    assert_eq!(messages[0]["text"], "Fix index.ts");
    assert_eq!(messages[1]["role"], "assistant");
    assert_eq!(messages[1]["text"], "Done");
    assert_eq!(messages[2]["text"], "Added tests");
}

#[test]
fn deterministic_uuid_is_stable() {
    let a = deterministic_uuid("same-input");
    let b = deterministic_uuid("same-input");
    let c = deterministic_uuid("different-input");

    assert_eq!(a, b);
    assert_ne!(a, c);
    assert_eq!(a.len(), 36);
}

#[test]
fn devql_file_config_parses_nested_block() {
    let value = serde_json::json!({
        "devql": {
            "postgres_dsn": "postgres://user:pass@localhost:5432/bitloops",
            "clickhouse_url": "http://localhost:8123",
            "clickhouse_database": "default"
        }
    });

    let cfg = DevqlFileConfig::from_json_value(&value);
    assert_eq!(
        cfg.pg_dsn.as_deref(),
        Some("postgres://user:pass@localhost:5432/bitloops")
    );
    assert_eq!(cfg.clickhouse_url.as_deref(), Some("http://localhost:8123"));
    assert_eq!(cfg.clickhouse_database.as_deref(), Some("default"));
}

#[test]
fn devql_file_config_parses_top_level_env_keys() {
    let value = serde_json::json!({
        "BITLOOPS_DEVQL_PG_DSN": "postgres://x/y",
        "BITLOOPS_DEVQL_CH_URL": "http://ch:8123",
        "BITLOOPS_DEVQL_CH_DATABASE": "analytics"
    });

    let cfg = DevqlFileConfig::from_json_value(&value);
    assert_eq!(cfg.pg_dsn.as_deref(), Some("postgres://x/y"));
    assert_eq!(cfg.clickhouse_url.as_deref(), Some("http://ch:8123"));
    assert_eq!(cfg.clickhouse_database.as_deref(), Some("analytics"));
}

#[test]
fn classify_connection_error_authentication() {
    let status = classify_connection_error("psql failed: FATAL: password authentication failed");
    assert_eq!(status, DatabaseConnectionStatus::CouldNotAuthenticate);
}

#[test]
fn classify_connection_error_reachability() {
    let status =
        classify_connection_error("ClickHouse request failed: curl: (7) Failed to connect");
    assert_eq!(status, DatabaseConnectionStatus::CouldNotReachDb);
}

#[test]
fn classify_connection_error_unknown() {
    let status = classify_connection_error("unexpected database failure");
    assert_eq!(status, DatabaseConnectionStatus::Error);
}

#[test]
fn normalize_repo_path_removes_dot_prefix() {
    assert_eq!(normalize_repo_path("./index.ts"), "index.ts");
    assert_eq!(normalize_repo_path("index.ts"), "index.ts");
    assert_eq!(normalize_repo_path(".\\src\\index.ts"), "src/index.ts");
}

#[test]
fn build_path_candidates_includes_variants() {
    let candidates = build_path_candidates("./index.ts");
    assert!(candidates.contains(&"./index.ts".to_string()));
    assert!(candidates.contains(&"index.ts".to_string()));
}

#[test]
fn extract_js_ts_functions_detects_basic_function() {
    let content = r#"export function hello() {
  return "Hello World";
}
"#;
    let functions = extract_js_ts_functions(content).unwrap();
    assert_eq!(functions.len(), 1);
    assert_eq!(functions[0].name, "hello");
    assert_eq!(functions[0].start_line, 1);
    assert_eq!(functions[0].end_line, 3);
    assert_eq!(functions[0].start_byte, 0);
    assert_eq!(functions[0].end_byte as usize, content.len());
    assert_eq!(functions[0].signature, "export function hello() {");
}

#[test]
fn extract_js_ts_functions_detects_arrow_function_assignment() {
    let content = r#"export const hello = () => {
  return "Hello World";
}
"#;
    let functions = extract_js_ts_functions(content).unwrap();
    assert_eq!(functions.len(), 1);
    assert_eq!(functions[0].name, "hello");
    assert_eq!(functions[0].start_line, 1);
    assert_eq!(functions[0].end_line, 3);
    assert_eq!(functions[0].start_byte, 0);
    assert_eq!(functions[0].end_byte as usize, content.len());
    assert_eq!(functions[0].signature, "export const hello = () => {");
}

#[test]
fn extract_js_ts_artefacts_covers_phase1_kinds() {
    let content = r#"import { helper } from "./helper";
export interface User {
  id: string;
}
export type UserId = string;
export class Service {
  run(input: string) {
    return input;
  }
}
export const answer = 42;
export function greet(name: string) {
  return helper(name);
}
"#;

    let artefacts = extract_js_ts_artefacts(content, "src/sample.ts").unwrap();
    let kinds = artefacts
        .iter()
        .map(|a| a.canonical_kind.as_str())
        .collect::<Vec<_>>();

    assert!(kinds.contains(&"import"));
    assert!(kinds.contains(&"interface"));
    assert!(kinds.contains(&"type"));
    assert!(kinds.contains(&"class"));
    assert!(kinds.contains(&"method"));
    assert!(kinds.contains(&"variable"));
    assert!(kinds.contains(&"function"));

    let method = artefacts
        .iter()
        .find(|a| a.canonical_kind == "method" && a.name == "run")
        .expect("expected class method artefact");
    assert_eq!(
        method.parent_symbol_fqn.as_deref(),
        Some("src/sample.ts::Service")
    );
    assert_eq!(method.symbol_fqn, "src/sample.ts::Service::run");
}

#[test]
fn extract_js_ts_dependency_edges_resolves_imports_and_calls() {
    let content = r#"import { helper as extHelper } from "./utils";
function local() {
  return 1;
}
function caller() {
  local();
  extHelper();
}
"#;
    let artefacts = extract_js_ts_artefacts(content, "src/sample.ts").unwrap();
    let edges = extract_js_ts_dependency_edges(content, "src/sample.ts", &artefacts).unwrap();

    assert!(edges.iter().any(|e| {
        e.edge_kind == "imports"
            && e.from_symbol_fqn == "src/sample.ts"
            && e.to_symbol_ref.as_deref() == Some("./utils")
    }));

    assert!(edges.iter().any(|e| {
        e.edge_kind == "calls"
            && e.from_symbol_fqn == "src/sample.ts::caller"
            && e.to_target_symbol_fqn.as_deref() == Some("src/sample.ts::local")
    }));

    assert!(edges.iter().any(|e| {
        e.edge_kind == "calls"
            && e.from_symbol_fqn == "src/sample.ts::caller"
            && e.to_symbol_ref.as_deref() == Some("./utils::helper")
    }));
}

#[test]
fn extract_js_ts_dependency_edges_emits_unresolved_call_fallback() {
    let content = r#"function caller() {
  mystery();
}
"#;
    let artefacts = extract_js_ts_artefacts(content, "src/sample.ts").unwrap();
    let edges = extract_js_ts_dependency_edges(content, "src/sample.ts", &artefacts).unwrap();

    assert!(edges.iter().any(|e| {
        e.edge_kind == "calls"
            && e.from_symbol_fqn == "src/sample.ts::caller"
            && e.to_symbol_ref.as_deref() == Some("src/sample.ts::mystery")
    }));
}

#[test]
fn extract_rust_artefacts_covers_phase1_kinds() {
    let content = r#"use std::fmt::Debug;

struct User {
    id: u64,
}

trait DoThing {
    fn do_it(&self);
}

impl DoThing for User {
    fn do_it(&self) {}
}

fn run() {
    println!("ok");
}
"#;
    let artefacts = extract_rust_artefacts(content, "src/lib.rs").unwrap();
    let kinds = artefacts
        .iter()
        .map(|a| a.canonical_kind.as_str())
        .collect::<Vec<_>>();
    assert!(kinds.contains(&"import"));
    assert!(kinds.contains(&"struct"));
    assert!(kinds.contains(&"trait"));
    assert!(kinds.contains(&"impl"));
    assert!(kinds.contains(&"method"));
    assert!(kinds.contains(&"function"));
}

#[test]
fn extract_rust_dependency_edges_emits_import_calls_and_implements() {
    let content = r#"use crate::math::sum;

trait DoThing { fn do_it(&self); }
struct User;

impl DoThing for User {
    fn do_it(&self) {
        sum(1, 2);
    }
}
"#;
    let artefacts = extract_rust_artefacts(content, "src/lib.rs").unwrap();
    let edges = extract_rust_dependency_edges(content, "src/lib.rs", &artefacts).unwrap();

    assert!(edges.iter().any(|e| {
        e.edge_kind == "imports" && e.to_symbol_ref.as_deref() == Some("crate::math::sum")
    }));
    assert!(edges.iter().any(|e| e.edge_kind == "implements" && e.to_symbol_ref.as_deref() == Some("DoThing")));
    assert!(edges.iter().any(|e| e.edge_kind == "calls"));
}

#[test]
fn postgres_schema_sql_includes_artefact_edges_hardening() {
    let sql = postgres_schema_sql();
    assert!(sql.contains("CREATE TABLE IF NOT EXISTS artefact_edges"));
    assert!(sql.contains("CONSTRAINT artefact_edges_target_chk"));
    assert!(sql.contains("CONSTRAINT artefact_edges_line_range_chk"));
    assert!(sql.contains("metadata JSONB DEFAULT '{}'::jsonb"));
    assert!(sql.contains("CREATE UNIQUE INDEX IF NOT EXISTS artefact_edges_natural_uq"));
    assert!(sql.contains("CREATE INDEX IF NOT EXISTS artefact_edges_symbol_ref_idx"));
}

#[test]
fn artefact_edges_hardening_sql_includes_constraints_and_indexes() {
    let sql = artefact_edges_hardening_sql();
    assert!(sql.contains("ADD CONSTRAINT artefact_edges_target_chk"));
    assert!(sql.contains("ADD CONSTRAINT artefact_edges_line_range_chk"));
    assert!(sql.contains("CREATE UNIQUE INDEX IF NOT EXISTS artefact_edges_natural_uq"));
    assert!(sql.contains("CREATE INDEX IF NOT EXISTS artefact_edges_symbol_ref_idx"));
}

#[test]
fn devql_ingest_accepts_explicit_false_for_init() {
    let parsed =
        crate::commands::Cli::try_parse_from(["bitloops", "devql", "ingest", "--init=false"])
            .expect("devql ingest should parse with explicit boolean value");

    let Some(crate::commands::Commands::Devql(args)) = parsed.command else {
        panic!("expected devql command");
    };
    let Some(DevqlCommand::Ingest(ingest)) = args.command else {
        panic!("expected devql ingest command");
    };

    assert!(!ingest.init);
}

#[test]
fn resolve_repo_id_for_query_is_strict_for_unknown_repo_names() {
    let cfg = test_cfg();

    let local = resolve_repo_id_for_query(&cfg, Some("temp2"));
    let unknown = resolve_repo_id_for_query(&cfg, Some("test2"));

    assert_eq!(local, cfg.repo.repo_id);
    assert_ne!(unknown, cfg.repo.repo_id);
}

#[test]
fn postgres_sslmode_validation_allows_default_dsn_without_sslmode() {
    let dsn = "postgres://user:pass@localhost:5432/bitloops";
    let pg_cfg: tokio_postgres::Config = dsn.parse().expect("valid dsn");
    assert!(matches!(pg_cfg.get_ssl_mode(), SslMode::Prefer));
    validate_postgres_sslmode_for_notls(dsn, pg_cfg.get_ssl_mode()).expect("prefer is allowed");
}

#[test]
fn postgres_sslmode_validation_rejects_require() {
    let dsn = "postgres://user:pass@localhost:5432/bitloops?sslmode=require";
    let pg_cfg: tokio_postgres::Config = dsn.parse().expect("valid dsn");
    let err = validate_postgres_sslmode_for_notls(dsn, pg_cfg.get_ssl_mode()).unwrap_err();
    assert!(
        err.to_string()
            .contains("Postgres DSN requires TLS (sslmode=Require)")
    );
}

#[test]
fn postgres_sslmode_validation_rejects_verify_full_dsn() {
    let dsn = "postgres://user:pass@localhost:5432/bitloops?sslmode=verify-full";
    let err = validate_postgres_sslmode_for_notls(dsn, SslMode::Prefer).unwrap_err();
    assert!(err.to_string().contains("sslmode=verify-ca/verify-full"));
}

#[tokio::test]
#[ignore = "requires BITLOOPS_TEST_PG_DSN"]
async fn artefact_edges_constraints_and_dedup_work_in_postgres() {
    let dsn = env::var("BITLOOPS_TEST_PG_DSN").expect("BITLOOPS_TEST_PG_DSN must be set");
    let (client, connection) = tokio_postgres::connect(&dsn, NoTls).await.unwrap();
    tokio::spawn(async move {
        let _ = connection.await;
    });

    let cfg = test_cfg();
    init_postgres_schema(&cfg, &client).await.unwrap();

    let artefact_id = deterministic_uuid("test-art-a");
    let upsert_artefact_sql = format!(
        "INSERT INTO artefacts (artefact_id, repo_id, blob_sha, path, language, canonical_kind, language_kind, symbol_fqn, parent_artefact_id, start_line, end_line, start_byte, end_byte, signature, content_hash) \
VALUES ('{}', '{}', 'blob1', 'src/a.ts', 'typescript', 'function', 'function_declaration', 'src/a.ts::a', NULL, 1, 3, 0, 10, 'function a() {{', 'h1') \
ON CONFLICT (artefact_id) DO NOTHING",
        esc_pg(&artefact_id),
        esc_pg(&cfg.repo.repo_id)
    );
    postgres_exec(&client, &upsert_artefact_sql).await.unwrap();

    let invalid_target_sql = format!(
        "INSERT INTO artefact_edges (edge_id, repo_id, blob_sha, from_artefact_id, edge_kind, language) \
VALUES ('{}', '{}', 'blob1', '{}', 'calls', 'typescript')",
        esc_pg(&deterministic_uuid("invalid-target")),
        esc_pg(&cfg.repo.repo_id),
        esc_pg(&artefact_id),
    );
    assert!(postgres_exec(&client, &invalid_target_sql).await.is_err());

    let invalid_range_sql = format!(
        "INSERT INTO artefact_edges (edge_id, repo_id, blob_sha, from_artefact_id, to_symbol_ref, edge_kind, language, start_line, end_line) \
VALUES ('{}', '{}', 'blob1', '{}', 'x', 'calls', 'typescript', 4, 3)",
        esc_pg(&deterministic_uuid("invalid-range")),
        esc_pg(&cfg.repo.repo_id),
        esc_pg(&artefact_id),
    );
    assert!(postgres_exec(&client, &invalid_range_sql).await.is_err());

    let edge_id_a = deterministic_uuid("dedup-a");
    let edge_insert_a = format!(
        "INSERT INTO artefact_edges (edge_id, repo_id, blob_sha, from_artefact_id, to_symbol_ref, edge_kind, language, start_line, end_line) \
VALUES ('{}', '{}', 'blob1', '{}', 'src/a.ts::x', 'calls', 'typescript', 2, 2)",
        esc_pg(&edge_id_a),
        esc_pg(&cfg.repo.repo_id),
        esc_pg(&artefact_id),
    );
    postgres_exec(&client, &edge_insert_a).await.unwrap();

    let edge_id_b = deterministic_uuid("dedup-b");
    let edge_insert_b = format!(
        "INSERT INTO artefact_edges (edge_id, repo_id, blob_sha, from_artefact_id, to_symbol_ref, edge_kind, language, start_line, end_line) \
VALUES ('{}', '{}', 'blob1', '{}', 'src/a.ts::x', 'calls', 'typescript', 2, 2)",
        esc_pg(&edge_id_b),
        esc_pg(&cfg.repo.repo_id),
        esc_pg(&artefact_id),
    );
    assert!(postgres_exec(&client, &edge_insert_b).await.is_err());
}
