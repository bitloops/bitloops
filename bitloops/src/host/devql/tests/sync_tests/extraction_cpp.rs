use super::fixtures::sync_test_cfg;

const TEST_EXTRACTION_FINGERPRINT: &str = "sync-test-fingerprint";

#[test]
fn sync_extraction_converts_cpp_content_to_cache_format() {
    let cfg = sync_test_cfg();
    let path = "src/main.cpp";
    let content = r#"#include <vector>

typedef int Count;

enum class Status {
    Ready,
};

struct Helper {
    Count value;
};

template <typename T>
class Box {};

class Base {};

class Child : public Base, public Box<Helper> {
public:
    Helper helper;

    int local_helper(int x) { return x + 1; }

    Status run(Helper input) {
        return external::lookup() ? Status::Ready : Status::Ready;
    }
};

int make_count(Count value) { return value; }
"#;
    let content_id =
        crate::host::devql::sync::content_identity::compute_blob_oid(content.as_bytes());

    let extraction = crate::host::devql::sync::extraction::extract_to_cache_format(
        &cfg,
        crate::host::devql::sync::extraction::CacheExtractionRequest {
            path,
            language: "cpp",
            content_id: &content_id,
            extraction_fingerprint: TEST_EXTRACTION_FINGERPRINT,
            parser_version: "tree-sitter-cpp@1",
            extractor_version: "cpp-language-pack@1",
            content,
        },
    )
    .expect("extract cpp content into cache format")
    .expect("cpp cache extraction should be supported");

    assert_eq!(extraction.language, "cpp");
    assert_eq!(extraction.parse_status, "ok");
    assert_eq!(extraction.content_id, content_id);

    let repeated = crate::host::devql::sync::extraction::extract_to_cache_format(
        &cfg,
        crate::host::devql::sync::extraction::CacheExtractionRequest {
            path,
            language: "cpp",
            content_id: &content_id,
            extraction_fingerprint: TEST_EXTRACTION_FINGERPRINT,
            parser_version: "tree-sitter-cpp@1",
            extractor_version: "cpp-language-pack@1",
            content,
        },
    )
    .expect("repeat extract cpp content into cache format")
    .expect("repeated cpp cache extraction should be supported");

    assert_eq!(
        extraction, repeated,
        "cpp cache extraction should be deterministic"
    );

    let file = extraction
        .artefacts
        .iter()
        .find(|artefact| artefact.canonical_kind.as_deref() == Some("file"))
        .expect("expected file artefact");
    let typedef_artefact = extraction
        .artefacts
        .iter()
        .find(|artefact| artefact.language_kind == "type_definition" && artefact.name == "Count")
        .expect("expected typedef artefact");
    let enum_artefact = extraction
        .artefacts
        .iter()
        .find(|artefact| artefact.language_kind == "enum_specifier" && artefact.name == "Status")
        .expect("expected enum artefact");
    let struct_artefact = extraction
        .artefacts
        .iter()
        .find(|artefact| artefact.language_kind == "struct_specifier" && artefact.name == "Helper")
        .expect("expected struct artefact");
    let template_artefact = extraction
        .artefacts
        .iter()
        .find(|artefact| artefact.language_kind == "template_declaration")
        .expect("expected template artefact");
    let base_class = extraction
        .artefacts
        .iter()
        .find(|artefact| artefact.language_kind == "class_specifier" && artefact.name == "Base")
        .expect("expected base class artefact");
    let class = extraction
        .artefacts
        .iter()
        .find(|artefact| artefact.language_kind == "class_specifier" && artefact.name == "Child")
        .expect("expected class artefact");
    let field = extraction
        .artefacts
        .iter()
        .find(|artefact| artefact.language_kind == "field_declaration" && artefact.name == "helper")
        .expect("expected field artefact");
    let method = extraction
        .artefacts
        .iter()
        .find(|artefact| {
            artefact.canonical_kind.as_deref() == Some("method") && artefact.name == "run"
        })
        .expect("expected method artefact");
    let function = extraction
        .artefacts
        .iter()
        .find(|artefact| {
            artefact.canonical_kind.as_deref() == Some("function") && artefact.name == "make_count"
        })
        .expect("expected free function artefact");

    for artefact in [
        typedef_artefact,
        enum_artefact,
        struct_artefact,
        template_artefact,
        base_class,
        class,
        function,
    ] {
        assert_eq!(
            artefact.parent_artifact_key.as_deref(),
            Some(file.artifact_key.as_str())
        );
        assert!(
            !artefact.artifact_key.is_empty(),
            "top-level artefact keys should be deterministic and non-empty"
        );
    }
    assert_eq!(
        field.parent_artifact_key.as_deref(),
        Some(class.artifact_key.as_str())
    );
    assert_eq!(
        method.parent_artifact_key.as_deref(),
        Some(class.artifact_key.as_str())
    );

    assert!(extraction.edges.iter().any(|edge| {
        edge.edge_kind == "imports"
            && edge.from_artifact_key == file.artifact_key
            && edge.to_symbol_ref.as_deref() == Some("#include <vector>")
    }));
    assert!(extraction.edges.iter().any(|edge| {
        edge.edge_kind == "extends"
            && edge.from_artifact_key == class.artifact_key
            && edge.to_artifact_key.as_deref() == Some(base_class.artifact_key.as_str())
    }));
    assert!(extraction.edges.iter().any(|edge| {
        edge.edge_kind == "references"
            && edge.from_artifact_key == field.artifact_key
            && edge.to_artifact_key.as_deref() == Some(struct_artefact.artifact_key.as_str())
    }));
    assert!(extraction.edges.iter().any(|edge| {
        edge.edge_kind == "calls"
            && edge.from_artifact_key == method.artifact_key
            && edge.to_symbol_ref.as_deref() == Some("src/main.cpp::external::lookup")
    }));
}
