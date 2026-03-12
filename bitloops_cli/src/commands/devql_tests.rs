use super::*;
use crate::engine::semantic_features::{
    PreStageArtefactRow, SemanticFeatureIndexState, SemanticFeatureInput,
    build_semantic_feature_rows, semantic_features_require_reindex,
};
use clap::Parser;

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
        semantic_provider: None,
        semantic_model: None,
        semantic_api_key: None,
        semantic_base_url: None,
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
fn semantic_summary_provider_resolves_default_openai_endpoint() {
    let endpoint =
        semantic::resolve_semantic_summary_endpoint("openai", None).expect("openai endpoint");
    assert_eq!(endpoint, "https://api.openai.com/v1/chat/completions");
}

#[test]
fn semantic_summary_provider_requires_model_when_enabled() {
    let mut cfg = test_cfg();
    cfg.semantic_provider = Some("openai".to_string());
    cfg.semantic_api_key = Some("test-key".to_string());

    let err = match semantic::build_semantic_summary_provider(&semantic_provider_config(&cfg)) {
        Ok(_) => panic!("provider should require model"),
        Err(err) => err,
    };
    assert!(
        err.to_string()
            .contains("BITLOOPS_DEVQL_SEMANTIC_MODEL is required")
    );
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

#[derive(Clone)]
struct MockSemanticSummaryProvider {
    candidate: Option<semantic::SemanticSummaryCandidate>,
}

impl semantic::SemanticSummaryProvider for MockSemanticSummaryProvider {
    fn generate(
        &self,
        _input: &SemanticFeatureInput,
    ) -> Option<semantic::SemanticSummaryCandidate> {
        self.candidate.clone()
    }

    fn prompt_version(&self) -> String {
        "semantic-summary-v4::provider=mock::model=test".to_string()
    }
}

fn mock_semantic_feature_blob_content() -> &'static str {
    r#"/* Service helpers for user operations. */
import { db } from './db';

export interface User {
  id: string;
  email: string;
}

export class UserService {
  // Fetch a user record by its id.
  async getById(id: string): Promise<User | null> {
    return db.users.findById(id);
  }
}

export function normalizeEmail(email: string): string {
  return email.trim().toLowerCase();
}

export const DEFAULT_PAGE_SIZE = 20;
export type UserId = string;
"#
}

fn mock_prestage_artefacts() -> Vec<PreStageArtefactRow> {
    serde_json::from_value(serde_json::json!([
        {
            "artefact_id": "a1b2c3d4-e5f6-7890-abcd-ef1234567890",
            "symbol_id": "1a2b3c4d-5e6f-7890-abcd-ef1234567890",
            "repo_id": "f47ac10b-58cc-4372-a567-0e02b2c3d479",
            "blob_sha": "a3f1e2b7c4d9e0f1a2b3c4d5e6f7a8b9c0d1e2f3",
            "path": "src/services/user.ts",
            "language": "typescript",
            "canonical_kind": "file",
            "language_kind": "module",
            "symbol_fqn": "src/services/user.ts",
            "parent_artefact_id": null,
            "start_line": 1,
            "end_line": 21,
            "start_byte": null,
            "end_byte": null,
            "signature": "src/services/user.ts",
            "content_hash": "c9d2e3f4-1a2b-3c4d-5e6f-7a8b9c0d1e2f"
        },
        {
            "artefact_id": "b2c3d4e5-f6a7-8901-bcde-f12345678901",
            "symbol_id": "2b3c4d5e-6f7a-8901-bcde-f12345678901",
            "repo_id": "f47ac10b-58cc-4372-a567-0e02b2c3d479",
            "blob_sha": "a3f1e2b7c4d9e0f1a2b3c4d5e6f7a8b9c0d1e2f3",
            "path": "src/services/user.ts",
            "language": "typescript",
            "canonical_kind": "import",
            "language_kind": "import",
            "symbol_fqn": "src/services/user.ts::import::./db",
            "parent_artefact_id": "a1b2c3d4-e5f6-7890-abcd-ef1234567890",
            "start_line": 2,
            "end_line": 2,
            "start_byte": null,
            "end_byte": null,
            "signature": "import { db } from './db';",
            "content_hash": null
        },
        {
            "artefact_id": "c3d4e5f6-a7b8-9012-cdef-123456789012",
            "symbol_id": "3c4d5e6f-7a8b-9012-cdef-123456789012",
            "repo_id": "f47ac10b-58cc-4372-a567-0e02b2c3d479",
            "blob_sha": "a3f1e2b7c4d9e0f1a2b3c4d5e6f7a8b9c0d1e2f3",
            "path": "src/services/user.ts",
            "language": "typescript",
            "canonical_kind": "interface",
            "language_kind": "interface",
            "symbol_fqn": "src/services/user.ts::User",
            "parent_artefact_id": "a1b2c3d4-e5f6-7890-abcd-ef1234567890",
            "start_line": 4,
            "end_line": 7,
            "start_byte": null,
            "end_byte": null,
            "signature": "export interface User {",
            "content_hash": "d4e5f6a7-2b3c-4d5e-6f7a-8b9c0d1e2f3a"
        },
        {
            "artefact_id": "d4e5f6a7-b8c9-0123-defa-234567890123",
            "symbol_id": "4d5e6f7a-8b9c-0123-defa-234567890123",
            "repo_id": "f47ac10b-58cc-4372-a567-0e02b2c3d479",
            "blob_sha": "a3f1e2b7c4d9e0f1a2b3c4d5e6f7a8b9c0d1e2f3",
            "path": "src/services/user.ts",
            "language": "typescript",
            "canonical_kind": "class",
            "language_kind": "class",
            "symbol_fqn": "src/services/user.ts::UserService",
            "parent_artefact_id": "a1b2c3d4-e5f6-7890-abcd-ef1234567890",
            "start_line": 9,
            "end_line": 14,
            "start_byte": null,
            "end_byte": null,
            "signature": "export class UserService {",
            "content_hash": "e5f6a7b8-3c4d-5e6f-7a8b-9c0d1e2f3a4b"
        },
        {
            "artefact_id": "e5f6a7b8-c9d0-1234-efab-345678901234",
            "symbol_id": "5e6f7a8b-9c0d-1234-efab-345678901234",
            "repo_id": "f47ac10b-58cc-4372-a567-0e02b2c3d479",
            "blob_sha": "a3f1e2b7c4d9e0f1a2b3c4d5e6f7a8b9c0d1e2f3",
            "path": "src/services/user.ts",
            "language": "typescript",
            "canonical_kind": "method",
            "language_kind": "method",
            "symbol_fqn": "src/services/user.ts::UserService::getById",
            "parent_artefact_id": "d4e5f6a7-b8c9-0123-defa-234567890123",
            "start_line": 11,
            "end_line": 13,
            "start_byte": null,
            "end_byte": null,
            "signature": "async getById(id: string): Promise<User | null> {",
            "doc_comment": "Fetch a user record by its id.",
            "content_hash": "f6a7b8c9-4d5e-6f7a-8b9c-0d1e2f3a4b5c"
        },
        {
            "artefact_id": "f6a7b8c9-d0e1-2345-fabc-456789012345",
            "symbol_id": "6f7a8b9c-0d1e-2345-fabc-456789012345",
            "repo_id": "f47ac10b-58cc-4372-a567-0e02b2c3d479",
            "blob_sha": "a3f1e2b7c4d9e0f1a2b3c4d5e6f7a8b9c0d1e2f3",
            "path": "src/services/user.ts",
            "language": "typescript",
            "canonical_kind": "function",
            "language_kind": "function",
            "symbol_fqn": "src/services/user.ts::normalizeEmail",
            "parent_artefact_id": "a1b2c3d4-e5f6-7890-abcd-ef1234567890",
            "start_line": 16,
            "end_line": 18,
            "start_byte": null,
            "end_byte": null,
            "signature": "export function normalizeEmail(email: string): string {",
            "content_hash": "a7b8c9d0-5e6f-7a8b-9c0d-1e2f3a4b5c6d"
        },
        {
            "artefact_id": "a8b9c0d1-e2f3-4567-abcd-567890123456",
            "symbol_id": "7a8b9c0d-1e2f-4567-abcd-567890123456",
            "repo_id": "f47ac10b-58cc-4372-a567-0e02b2c3d479",
            "blob_sha": "a3f1e2b7c4d9e0f1a2b3c4d5e6f7a8b9c0d1e2f3",
            "path": "src/services/user.ts",
            "language": "typescript",
            "canonical_kind": "variable",
            "language_kind": "const",
            "symbol_fqn": "src/services/user.ts::DEFAULT_PAGE_SIZE",
            "parent_artefact_id": "a1b2c3d4-e5f6-7890-abcd-ef1234567890",
            "start_line": 20,
            "end_line": 20,
            "start_byte": null,
            "end_byte": null,
            "signature": "export const DEFAULT_PAGE_SIZE = 20;",
            "content_hash": null
        },
        {
            "artefact_id": "b9c0d1e2-f3a4-5678-bcde-678901234567",
            "symbol_id": "8b9c0d1e-2f3a-5678-bcde-678901234567",
            "repo_id": "f47ac10b-58cc-4372-a567-0e02b2c3d479",
            "blob_sha": "a3f1e2b7c4d9e0f1a2b3c4d5e6f7a8b9c0d1e2f3",
            "path": "src/services/user.ts",
            "language": "typescript",
            "canonical_kind": "type",
            "language_kind": "type_alias",
            "symbol_fqn": "src/services/user.ts::UserId",
            "parent_artefact_id": "a1b2c3d4-e5f6-7890-abcd-ef1234567890",
            "start_line": 21,
            "end_line": 21,
            "start_byte": null,
            "end_byte": null,
            "signature": "export type UserId = string;",
            "content_hash": null
        }
    ]))
    .expect("mock artefacts should parse")
}

fn semantic_feature_input_named(name: &str) -> SemanticFeatureInput {
    build_semantic_feature_inputs_from_artefacts(
        &mock_prestage_artefacts(),
        mock_semantic_feature_blob_content(),
    )
    .into_iter()
    .find(|input| input.name == name)
    .expect("semantic feature input should exist")
}

#[test]
fn semantic_features_build_inputs_from_mock_prestage_rows() {
    let inputs = build_semantic_feature_inputs_from_artefacts(
        &mock_prestage_artefacts(),
        mock_semantic_feature_blob_content(),
    );

    assert_eq!(
        inputs.len(),
        7,
        "import rows should not be enriched in stage 1"
    );
    assert!(inputs.iter().all(|input| input.canonical_kind != "import"));

    let file = inputs
        .iter()
        .find(|input| input.canonical_kind == "file")
        .expect("file input");
    assert!(
        file.local_relationships
            .contains(&"contains:class".to_string()),
        "{:?}",
        file.local_relationships
    );

    let method = inputs
        .iter()
        .find(|input| input.name == "getById")
        .expect("method input");
    assert_eq!(method.parent_kind.as_deref(), Some("class"));
    assert_eq!(
        method.parent_symbol.as_deref(),
        Some("src/services/user.ts::UserService")
    );
    assert!(
        method.body.contains("findById"),
        "expected method body to be sliced from blob content"
    );
}

#[test]
fn semantic_features_prefer_doc_comment_summary_for_mock_method() {
    let input = semantic_feature_input_named("getById");
    let output = build_semantic_feature_rows(&input, &semantic::NoopSemanticSummaryProvider);

    assert_eq!(output.semantics.summary, "Fetch a user record by its id.");
    assert_eq!(
        output.semantics.doc_comment_summary.as_deref(),
        Some("Fetch a user record by its id.")
    );
    assert_eq!(output.semantics.llm_summary, None);
    assert_eq!(output.semantics.template_summary, "Method get by id.");
    assert_eq!(
        output.semantics.summary_source,
        semantic::SemanticSummarySource::DocComment
    );
    assert_eq!(output.features.normalized_name, "get_by_id");
    assert!(
        output
            .features
            .normalized_body_tokens
            .contains(&"find".to_string())
    );
}

#[test]
fn semantic_features_use_mock_llm_summary_when_doc_comment_missing() {
    let input = semantic_feature_input_named("normalizeEmail");
    let output = build_semantic_feature_rows(
        &input,
        &MockSemanticSummaryProvider {
            candidate: Some(semantic::SemanticSummaryCandidate {
                summary: "Normalizes email addresses before storage".to_string(),
                confidence: 0.87,
                source_model: Some("mock-llm".to_string()),
            }),
        },
    );

    assert_eq!(
        output.semantics.summary,
        "Normalizes email addresses before storage."
    );
    assert_eq!(output.semantics.doc_comment_summary, None);
    assert_eq!(
        output.semantics.llm_summary.as_deref(),
        Some("Normalizes email addresses before storage")
    );
    assert_eq!(
        output.semantics.template_summary,
        "Function normalize email."
    );
    assert_eq!(
        output.semantics.summary_source,
        semantic::SemanticSummarySource::Llm
    );
    assert_eq!(output.semantics.source_model.as_deref(), Some("mock-llm"));
}

#[test]
fn semantic_features_fall_back_to_template_when_mock_llm_summary_is_invalid() {
    let input = semantic_feature_input_named("normalizeEmail");
    let output = build_semantic_feature_rows(
        &input,
        &MockSemanticSummaryProvider {
            candidate: Some(semantic::SemanticSummaryCandidate {
                summary: "bad".to_string(),
                confidence: 0.8,
                source_model: Some("mock-llm".to_string()),
            }),
        },
    );

    assert_eq!(output.semantics.summary, "Function normalize email.");
    assert_eq!(output.semantics.doc_comment_summary, None);
    assert_eq!(output.semantics.llm_summary.as_deref(), Some("bad"));
    assert_eq!(
        output.semantics.template_summary,
        "Function normalize email."
    );
    assert_eq!(
        output.semantics.summary_source,
        semantic::SemanticSummarySource::TemplateFallback
    );
}

#[test]
fn semantic_features_store_doc_comment_and_llm_candidates_together() {
    let input = semantic_feature_input_named("getById");
    let output = build_semantic_feature_rows(
        &input,
        &MockSemanticSummaryProvider {
            candidate: Some(semantic::SemanticSummaryCandidate {
                summary: "Loads a user entity by id from storage".to_string(),
                confidence: 0.82,
                source_model: Some("mock-llm".to_string()),
            }),
        },
    );

    assert_eq!(
        output.semantics.doc_comment_summary.as_deref(),
        Some("Fetch a user record by its id.")
    );
    assert_eq!(
        output.semantics.llm_summary.as_deref(),
        Some("Loads a user entity by id from storage")
    );
    assert_eq!(output.semantics.template_summary, "Method get by id.");
    assert_eq!(
        output.semantics.summary,
        "Loads a user entity by id from storage."
    );
    assert_eq!(
        output.semantics.summary_source,
        semantic::SemanticSummarySource::Llm
    );
    assert_eq!(output.semantics.source_model.as_deref(), Some("mock-llm"));
}

#[test]
fn semantic_features_reindex_when_hash_or_prompt_version_changes() {
    let input = semantic_feature_input_named("normalizeEmail");
    let output = build_semantic_feature_rows(&input, &semantic::NoopSemanticSummaryProvider);
    let hash = output.semantic_features_input_hash.clone();

    let unchanged = SemanticFeatureIndexState {
        semantics_hash: Some(hash.clone()),
        semantics_prompt_version: Some(output.semantics.prompt_version.clone()),
        features_hash: Some(hash.clone()),
        features_prompt_version: Some(output.features.prompt_version.clone()),
    };
    assert!(!semantic_features_require_reindex(
        &unchanged,
        &hash,
        &output.semantics.prompt_version,
        &output.features.prompt_version,
    ));

    let stale_prompt = SemanticFeatureIndexState {
        semantics_prompt_version: Some("semantic-summary-v0::provider=noop".to_string()),
        ..unchanged.clone()
    };
    assert!(semantic_features_require_reindex(
        &stale_prompt,
        &hash,
        &output.semantics.prompt_version,
        &output.features.prompt_version,
    ));

    let stale_hash = SemanticFeatureIndexState {
        features_hash: Some("different-hash".to_string()),
        ..unchanged
    };
    assert!(semantic_features_require_reindex(
        &stale_hash,
        &hash,
        &output.semantics.prompt_version,
        &output.features.prompt_version,
    ));
}
