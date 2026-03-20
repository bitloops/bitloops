use super::*;

fn extension_runtime_cfg() -> DevqlConfig {
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
        embedding_provider: None,
        embedding_model: None,
        embedding_api_key: None,
    }
}

fn canonical_kind(artefact: &JsTsArtefact) -> Option<&str> {
    artefact.canonical_kind.as_deref()
}

fn artefact_by_language_kind<'a>(
    artefacts: &'a [JsTsArtefact],
    language_kind: &str,
) -> &'a JsTsArtefact {
    artefacts
        .iter()
        .find(|artefact| artefact.language_kind == language_kind)
        .unwrap_or_else(|| panic!("missing artefact with language_kind {language_kind}"))
}

fn artefact_by_name_and_language_kind<'a>(
    artefacts: &'a [JsTsArtefact],
    language_kind: &str,
    name: &str,
) -> &'a JsTsArtefact {
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
        assert_eq!(js_ts_supports_language_kind(language_kind), supported);
        assert_eq!(js_ts_canonical_kind(language_kind), canonical_kind);
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
        assert_eq!(rust_supports_language_kind(language_kind), supported);
        assert_eq!(
            rust_canonical_kind(language_kind, inside_impl),
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
    assert!(is_supported_symbol_language("rust"));
    assert!(is_supported_symbol_language("typescript"));
    assert!(is_supported_symbol_language("javascript"));
    assert!(!is_supported_symbol_language("python"));
}

#[test]
fn devql_extension_host_builds_capability_contexts_from_registered_owners() {
    let cfg = extension_runtime_cfg();

    let stage_context = capability_execution_context_for_stage(
        &cfg,
        Some("abc123"),
        SEMANTIC_CLONES_CAPABILITY_STAGE_ID,
    )
    .expect("resolve semantic clones stage owner");
    assert_eq!(
        stage_context.capability_pack_id,
        "semantic-clones-capability-pack"
    );
    assert_eq!(stage_context.stage_id, SEMANTIC_CLONES_CAPABILITY_STAGE_ID);
    assert_eq!(stage_context.commit_sha.as_deref(), Some("abc123"));

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
