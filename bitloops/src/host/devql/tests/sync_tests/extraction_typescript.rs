use super::fixtures::sync_test_cfg;

#[test]
fn sync_extraction_converts_typescript_content_to_cache_format() {
    let cfg = sync_test_cfg();
    let path = "src/sample.ts";
    let content = r#"import { remoteFoo } from "./remote";

class Service {
  run(): number {
    return localHelper() + remoteFoo();
  }
}

function localHelper(): number {
  return 1;
}
"#;
    let content_id =
        crate::host::devql::sync::content_identity::compute_blob_oid(content.as_bytes());

    let extraction = crate::host::devql::sync::extraction::extract_to_cache_format(
        &cfg,
        path,
        &content_id,
        "tree-sitter-ts@1",
        "ts-language-pack@1",
        content,
    )
    .expect("extract TypeScript content into cache format")
    .expect("TypeScript cache extraction should be supported");

    assert_eq!(extraction.content_id, content_id);
    assert_eq!(extraction.language, "typescript");
    assert_eq!(extraction.parser_version, "tree-sitter-ts@1");
    assert_eq!(extraction.extractor_version, "ts-language-pack@1");
    assert_eq!(extraction.parse_status, "ok");

    let repeated = crate::host::devql::sync::extraction::extract_to_cache_format(
        &cfg,
        path,
        &content_id,
        "tree-sitter-ts@1",
        "ts-language-pack@1",
        content,
    )
    .expect("repeat extract TypeScript content into cache format")
    .expect("repeated TypeScript cache extraction should be supported");
    assert_eq!(
        extraction, repeated,
        "cache extraction should be deterministic"
    );

    let file = extraction
        .artefacts
        .iter()
        .find(|artefact| {
            artefact.canonical_kind.as_deref() == Some("file") && artefact.name == path
        })
        .expect("expected file artefact");
    let class = extraction
        .artefacts
        .iter()
        .find(|artefact| {
            artefact.language_kind == "class_declaration" && artefact.name == "Service"
        })
        .expect("expected class artefact");
    let method = extraction
        .artefacts
        .iter()
        .find(|artefact| {
            artefact.canonical_kind.as_deref() == Some("method") && artefact.name == "run"
        })
        .expect("expected method artefact");
    let helper = extraction
        .artefacts
        .iter()
        .find(|artefact| {
            artefact.canonical_kind.as_deref() == Some("function") && artefact.name == "localHelper"
        })
        .expect("expected local helper artefact");

    assert!(
        !file.artifact_key.is_empty(),
        "file artefact key should be deterministic and non-empty"
    );
    assert!(
        !class.artifact_key.is_empty(),
        "class artefact key should be deterministic and non-empty"
    );
    assert!(
        !method.artifact_key.is_empty(),
        "method artefact key should be deterministic and non-empty"
    );
    assert!(
        !helper.artifact_key.is_empty(),
        "helper artefact key should be deterministic and non-empty"
    );
    assert_eq!(
        class.parent_artifact_key.as_deref(),
        Some(file.artifact_key.as_str())
    );
    assert_eq!(
        method.parent_artifact_key.as_deref(),
        Some(class.artifact_key.as_str())
    );
    assert_eq!(
        helper.parent_artifact_key.as_deref(),
        Some(file.artifact_key.as_str())
    );

    let same_file_call = extraction
        .edges
        .iter()
        .find(|edge| {
            edge.edge_kind == "calls"
                && edge.from_artifact_key == method.artifact_key
                && edge.to_artifact_key.as_deref() == Some(helper.artifact_key.as_str())
        })
        .expect("expected same-file call edge");
    assert!(
        !same_file_call.edge_key.is_empty(),
        "same-file edge key should be deterministic and non-empty"
    );
    assert_eq!(same_file_call.to_symbol_ref, None);

    let cross_file_call = extraction
        .edges
        .iter()
        .find(|edge| {
            edge.edge_kind == "calls"
                && edge.from_artifact_key == method.artifact_key
                && edge.to_symbol_ref.as_deref() == Some("./remote::remoteFoo")
        })
        .expect("expected cross-file call edge");
    assert!(
        !cross_file_call.edge_key.is_empty(),
        "cross-file edge key should be deterministic and non-empty"
    );
    assert_eq!(cross_file_call.to_artifact_key, None);

    let import_edge = extraction
        .edges
        .iter()
        .find(|edge| {
            edge.edge_kind == "imports"
                && edge.from_artifact_key == file.artifact_key
                && edge.to_symbol_ref.as_deref() == Some("./remote")
        })
        .expect("expected file-level import edge");
    assert!(
        !import_edge.edge_key.is_empty(),
        "import edge key should be deterministic and non-empty"
    );
    assert_eq!(import_edge.to_artifact_key, None);
}

#[test]
fn sync_extraction_uses_path_agnostic_artifact_keys_for_same_content() {
    let cfg = sync_test_cfg();
    let content = r#"class Service {
  run(): number {
    return localHelper();
  }
}

function localHelper(): number {
  return 1;
}
"#;
    let content_id =
        crate::host::devql::sync::content_identity::compute_blob_oid(content.as_bytes());

    let first = crate::host::devql::sync::extraction::extract_to_cache_format(
        &cfg,
        "src/sample.ts",
        &content_id,
        "tree-sitter-ts@1",
        "ts-language-pack@1",
        content,
    )
    .expect("extract first TypeScript path")
    .expect("first TypeScript cache extraction should be supported");
    let second = crate::host::devql::sync::extraction::extract_to_cache_format(
        &cfg,
        "nested/other.ts",
        &content_id,
        "tree-sitter-ts@1",
        "ts-language-pack@1",
        content,
    )
    .expect("extract second TypeScript path")
    .expect("second TypeScript cache extraction should be supported");

    let key_for = |extraction: &crate::host::devql::sync::content_cache::CachedExtraction,
                   name: &str,
                   language_kind: &str| {
        extraction
            .artefacts
            .iter()
            .find(|artefact| artefact.name == name && artefact.language_kind == language_kind)
            .map(|artefact| artefact.artifact_key.clone())
            .expect("expected artefact key")
    };

    assert_eq!(
        first
            .artefacts
            .iter()
            .find(|artefact| artefact.canonical_kind.as_deref() == Some("file"))
            .map(|artefact| artefact.artifact_key.clone()),
        second
            .artefacts
            .iter()
            .find(|artefact| artefact.canonical_kind.as_deref() == Some("file"))
            .map(|artefact| artefact.artifact_key.clone())
    );
    assert_eq!(
        key_for(&first, "Service", "class_declaration"),
        key_for(&second, "Service", "class_declaration")
    );
    assert_eq!(
        key_for(&first, "run", "method_definition"),
        key_for(&second, "run", "method_definition")
    );
    assert_eq!(
        key_for(&first, "localHelper", "function_declaration"),
        key_for(&second, "localHelper", "function_declaration")
    );

    let same_file_edge_key =
        |extraction: &crate::host::devql::sync::content_cache::CachedExtraction| {
            extraction
                .edges
                .iter()
                .find(|edge| edge.edge_kind == "calls" && edge.to_artifact_key.is_some())
                .map(|edge| edge.edge_key.clone())
                .expect("expected same-file edge key")
        };

    assert_eq!(same_file_edge_key(&first), same_file_edge_key(&second));
}
