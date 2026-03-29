use super::*;
use crate::adapters::languages::rust::canonical::{
    RUST_CANONICAL_MAPPINGS, RUST_SUPPORTED_LANGUAGE_KINDS,
};
use crate::adapters::languages::ts_js::canonical::{
    TS_JS_CANONICAL_MAPPINGS, TS_JS_SUPPORTED_LANGUAGE_KINDS,
};
use crate::host::language_adapter::{is_supported_language_kind, resolve_canonical_kind};

fn extension_runtime_cfg() -> DevqlConfig {
    DevqlConfig {
        config_root: PathBuf::from("/tmp/repo"),
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
        embedding_provider: None,
        embedding_model: None,
        embedding_api_key: None,
    }
}

fn canonical_kind(artefact: &LanguageArtefact) -> Option<&str> {
    artefact.canonical_kind.as_deref()
}

fn artefact_by_language_kind<'a>(
    artefacts: &'a [LanguageArtefact],
    language_kind: &str,
) -> &'a LanguageArtefact {
    artefacts
        .iter()
        .find(|artefact| artefact.language_kind == language_kind)
        .unwrap_or_else(|| panic!("missing artefact with language_kind {language_kind}"))
}

fn artefact_by_name_and_language_kind<'a>(
    artefacts: &'a [LanguageArtefact],
    language_kind: &str,
    name: &str,
) -> &'a LanguageArtefact {
    artefacts
        .iter()
        .find(|artefact| artefact.language_kind == language_kind && artefact.name == name)
        .unwrap_or_else(|| panic!("missing artefact {name} with language_kind {language_kind}"))
}

#[test]
fn js_ts_canonical_mapping_covers_supported_kind_table() {
    let expected = [
        ("function_declaration", true, Some("function")),
        ("method_definition", true, Some("method")),
        ("interface_declaration", true, Some("interface")),
        ("type_alias_declaration", true, Some("type")),
        ("enum_declaration", true, Some("enum")),
        ("variable_declarator", true, Some("variable")),
        ("import_statement", true, Some("import")),
        ("module_declaration", true, Some("module")),
        ("internal_module", true, Some("module")),
        ("class_declaration", true, None),
        ("constructor", true, None),
        ("property_declaration", true, None),
        ("public_field_definition", true, None),
        ("call_expression", false, None),
    ];

    for (language_kind, supported, canonical_kind) in expected {
        assert_eq!(
            is_supported_language_kind(TS_JS_SUPPORTED_LANGUAGE_KINDS, language_kind),
            supported
        );
        assert_eq!(
            resolve_canonical_kind(TS_JS_CANONICAL_MAPPINGS, language_kind, false)
                .map(CanonicalKindProjection::as_str),
            canonical_kind
        );
    }
}

#[test]
fn js_ts_canonical_mapping_is_abstraction_only_and_preserves_parser_kinds() {
    let content = r#"import { helper } from "./helper";
export interface Contract {
  id: string;
}
export type Identifier = string;
export enum Status {
  Ready,
}
const API_URL = "/v1";
export class Service {
  constructor(private readonly prefix: string) {}

  run() {
    return helper();
  }
}
export function helper() {
  return "ok";
}
"#;

    let artefacts = extract_js_ts_artefacts(content, "src/sample.ts").unwrap();

    let import = artefact_by_language_kind(&artefacts, "import_statement");
    assert_eq!(canonical_kind(import), Some("import"));

    let interface =
        artefact_by_name_and_language_kind(&artefacts, "interface_declaration", "Contract");
    assert_eq!(canonical_kind(interface), Some("interface"));

    let type_alias =
        artefact_by_name_and_language_kind(&artefacts, "type_alias_declaration", "Identifier");
    assert_eq!(canonical_kind(type_alias), Some("type"));

    let variable = artefact_by_name_and_language_kind(&artefacts, "variable_declarator", "API_URL");
    assert_eq!(canonical_kind(variable), Some("variable"));

    let function = artefact_by_name_and_language_kind(&artefacts, "function_declaration", "helper");
    assert_eq!(canonical_kind(function), Some("function"));

    let constructor = artefact_by_name_and_language_kind(&artefacts, "constructor", "constructor");
    assert_eq!(canonical_kind(constructor), None);

    let method = artefact_by_name_and_language_kind(&artefacts, "method_definition", "run");
    assert_eq!(canonical_kind(method), Some("method"));

    let class = artefact_by_name_and_language_kind(&artefacts, "class_declaration", "Service");
    assert_eq!(canonical_kind(class), None);
}

#[test]
fn rust_canonical_mapping_covers_supported_kind_table() {
    let expected = [
        (("function_item", false), true, Some("function")),
        (("function_item", true), true, Some("method")),
        (("trait_item", false), true, Some("interface")),
        (("type_item", false), true, Some("type")),
        (("enum_item", false), true, Some("enum")),
        (("use_declaration", false), true, Some("import")),
        (("mod_item", false), true, Some("module")),
        (("let_declaration", false), true, Some("variable")),
        (("impl_item", false), true, None),
        (("struct_item", false), true, None),
        (("const_item", false), true, None),
        (("static_item", false), true, None),
        (("macro_definition", false), true, None),
        (("call_expression", false), false, None),
    ];

    for ((language_kind, inside_impl), supported, canonical_kind) in expected {
        assert_eq!(
            is_supported_language_kind(RUST_SUPPORTED_LANGUAGE_KINDS, language_kind),
            supported
        );
        assert_eq!(
            resolve_canonical_kind(RUST_CANONICAL_MAPPINGS, language_kind, inside_impl)
                .map(CanonicalKindProjection::as_str),
            canonical_kind
        );
    }
}

#[test]
fn rust_canonical_mapping_normalizes_traits_and_marks_language_only_symbols() {
    let content = r#"use crate::fmt::Display;

mod api {}

type UserId = u64;
enum Role {
    Admin,
}
struct User;

trait Repository {
    fn save(&self);
}

impl Repository for User {
    fn save(&self) {}
}

const LIMIT: usize = 4;
static NAME: &str = "demo";

fn run() {}
"#;

    let artefacts = extract_rust_artefacts(content, "src/lib.rs").unwrap();

    let import = artefact_by_language_kind(&artefacts, "use_declaration");
    assert_eq!(canonical_kind(import), Some("import"));

    let module = artefact_by_name_and_language_kind(&artefacts, "mod_item", "api");
    assert_eq!(canonical_kind(module), Some("module"));

    let type_item = artefact_by_name_and_language_kind(&artefacts, "type_item", "UserId");
    assert_eq!(canonical_kind(type_item), Some("type"));

    let enum_item = artefact_by_name_and_language_kind(&artefacts, "enum_item", "Role");
    assert_eq!(canonical_kind(enum_item), Some("enum"));

    let trait_item = artefact_by_name_and_language_kind(&artefacts, "trait_item", "Repository");
    assert_eq!(canonical_kind(trait_item), Some("interface"));

    let free_function = artefacts
        .iter()
        .find(|artefact| {
            artefact.language_kind == "function_item"
                && artefact.name == "run"
                && artefact.parent_symbol_fqn.is_none()
        })
        .expect("missing free function artefact");
    assert_eq!(canonical_kind(free_function), Some("function"));

    let struct_item = artefact_by_name_and_language_kind(&artefacts, "struct_item", "User");
    assert_eq!(canonical_kind(struct_item), None);

    let impl_item = artefact_by_language_kind(&artefacts, "impl_item");
    assert_eq!(canonical_kind(impl_item), None);

    let const_item = artefact_by_name_and_language_kind(&artefacts, "const_item", "LIMIT");
    assert_eq!(canonical_kind(const_item), None);

    let static_item = artefact_by_name_and_language_kind(&artefacts, "static_item", "NAME");
    assert_eq!(canonical_kind(static_item), None);
}

#[test]
fn rust_trait_method_signatures_are_not_emitted_as_free_functions() {
    let content = r#"trait Repository {
    fn save(&self);
}

struct User;

impl Repository for User {
    fn save(&self) {}
}
"#;

    let artefacts = extract_rust_artefacts(content, "src/lib.rs").unwrap();

    let trait_item = artefact_by_name_and_language_kind(&artefacts, "trait_item", "Repository");
    assert_eq!(canonical_kind(trait_item), Some("interface"));

    let save_callables = artefacts
        .iter()
        .filter(|artefact| {
            artefact.name == "save"
                && matches!(
                    artefact.canonical_kind.as_deref(),
                    Some("function") | Some("method")
                )
        })
        .collect::<Vec<_>>();

    assert_eq!(
        save_callables.len(),
        1,
        "trait signatures should not be emitted as standalone callable artefacts"
    );
    assert_eq!(canonical_kind(save_callables[0]), Some("method"));
    assert!(
        save_callables[0]
            .parent_symbol_fqn
            .as_deref()
            .is_some_and(|parent| parent.starts_with("src/lib.rs::impl@"))
    );
}

#[test]
fn devql_extension_host_resolves_built_in_language_pack_ownership() {
    assert_eq!(
        resolve_language_pack_owner("rust"),
        Some(RUST_LANGUAGE_PACK_ID)
    );
    assert_eq!(
        resolve_language_pack_owner("typescript"),
        Some(TS_JS_LANGUAGE_PACK_ID)
    );
    assert_eq!(
        resolve_language_pack_owner("javascript"),
        Some(TS_JS_LANGUAGE_PACK_ID)
    );
    assert!(resolve_language_pack_owner("python").is_none());
    assert_eq!(
        resolve_language_id_for_file_path("src/lib.rs"),
        Some("rust")
    );
    assert_eq!(
        resolve_language_id_for_file_path("src/main.ts"),
        Some("typescript")
    );
    assert_eq!(
        resolve_language_id_for_file_path("src/main.jsx"),
        Some("javascript")
    );
    assert!(resolve_language_id_for_file_path("README").is_none());
}

#[test]
fn devql_language_adapter_registry_resolves_built_in_pack_implementations() {
    let registry = language_adapter_registry().expect("initialize language adapter registry");
    assert_eq!(
        registry.registered_pack_ids(),
        vec![RUST_LANGUAGE_PACK_ID, TS_JS_LANGUAGE_PACK_ID]
    );
    assert!(registry.get(RUST_LANGUAGE_PACK_ID).is_some());
    assert!(registry.get(TS_JS_LANGUAGE_PACK_ID).is_some());
    assert!(registry.get("unknown-pack").is_none());
}

#[test]
fn devql_language_adapter_registry_executes_rust_and_ts_js_built_ins() {
    let registry = language_adapter_registry().expect("initialize language adapter registry");
    let rust_pack = registry
        .get(RUST_LANGUAGE_PACK_ID)
        .expect("resolve rust built-in language adapter pack");
    let rust_content = r#"//! crate docs
fn greet() {
    helper();
}

fn helper() {}
"#;
    let rust_artefacts = rust_pack
        .extract_artefacts(rust_content, "src/lib.rs")
        .expect("extract rust artefacts via language adapter registry");
    assert!(
        rust_artefacts
            .iter()
            .any(|artefact| artefact.name == "greet"),
        "rust built-in registry pack should surface function artefacts"
    );
    assert!(
        rust_pack.extract_file_docstring(rust_content).is_some(),
        "rust built-in registry pack should expose crate-level docstrings"
    );

    let ts_pack = registry
        .get(TS_JS_LANGUAGE_PACK_ID)
        .expect("resolve ts/js built-in language adapter pack");
    let ts_content = r#"export function greet() {
    return helper();
}

function helper() {
    return 1;
}
"#;
    let ts_artefacts = ts_pack
        .extract_artefacts(ts_content, "src/main.ts")
        .expect("extract ts artefacts via language adapter registry");
    assert!(
        ts_artefacts.iter().any(|artefact| artefact.name == "greet"),
        "ts/js built-in registry pack should surface function artefacts"
    );
    let ts_edges = ts_pack
        .extract_dependency_edges(ts_content, "src/main.ts", &ts_artefacts)
        .expect("extract ts dependency edges via language adapter registry");
    assert!(
        ts_edges
            .iter()
            .any(|edge| edge.edge_kind == EdgeKind::Calls),
        "ts/js built-in registry pack should emit call edges"
    );
}

#[test]
fn devql_extension_host_builds_capability_contexts_from_registered_owners() {
    let cfg = extension_runtime_cfg();

    let ingest_context = capability_ingest_context_for_ingester(
        &cfg,
        Some("abc123"),
        TEST_HARNESS_CAPABILITY_INGESTER_ID,
    )
    .expect("resolve test-harness ingester owner");
    assert_eq!(
        ingest_context.capability_pack_id,
        "test-harness-capability-pack"
    );
    assert_eq!(
        ingest_context.ingester_id,
        TEST_HARNESS_CAPABILITY_INGESTER_ID
    );
    assert_eq!(ingest_context.commit_sha.as_deref(), Some("abc123"));
}

#[test]
fn devql_language_adapter_lifecycle_summary_reports_builtins_and_readiness() {
    let cfg = extension_runtime_cfg();
    let lifecycle = collect_language_adapter_lifecycle(&cfg, "local-cli", false, false)
        .expect("collect language adapter lifecycle summary");

    let pack_ids = lifecycle
        .summary
        .packs
        .iter()
        .map(|pack| pack.id.as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        pack_ids,
        vec![RUST_LANGUAGE_PACK_ID, TS_JS_LANGUAGE_PACK_ID]
    );
    assert!(
        lifecycle
            .readiness_reports
            .iter()
            .all(|report| report.ready),
        "built-in language adapters should report ready without pending migrations"
    );
}

#[test]
fn core_extension_host_registry_report_with_language_adapter_snapshot_includes_adapter_entries() {
    let cfg = extension_runtime_cfg();
    let lifecycle = collect_language_adapter_lifecycle(&cfg, "local-cli", false, false)
        .expect("collect language adapter lifecycle summary");
    let ext_host = crate::host::extension_host::CoreExtensionHost::with_builtins()
        .expect("bootstrap core extension host");
    let snapshot = ext_host
        .readiness_snapshot()
        .with_language_adapter_readiness(
            lifecycle
                .summary
                .packs
                .iter()
                .map(|pack| pack.id.clone())
                .collect(),
            lifecycle.readiness_reports,
        );
    let report = ext_host.registry_report_with_snapshot(snapshot);

    assert_eq!(
        report.language_adapter_pack_ids,
        vec![
            RUST_LANGUAGE_PACK_ID.to_string(),
            TS_JS_LANGUAGE_PACK_ID.to_string()
        ]
    );
    assert!(
        report
            .readiness
            .iter()
            .any(|entry| entry.family == "language-adapter-pack"),
        "language adapter readiness entries should be present in extension report"
    );
}
