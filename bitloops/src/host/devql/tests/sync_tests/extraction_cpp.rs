use super::fixtures::sync_test_cfg;

const TEST_EXTRACTION_FINGERPRINT: &str = "sync-test-fingerprint";

#[test]
fn sync_extraction_converts_cpp_content_to_cache_format() {
    let cfg = sync_test_cfg();
    let path = "src/main.cpp";
    let content = r#"#include <vector>

class Base {};
class Child : public Base {
public:
    int helper(int x) { return x + 1; }
    int run(int x) { return helper(x); }
};
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

    let file = extraction
        .artefacts
        .iter()
        .find(|artefact| artefact.canonical_kind.as_deref() == Some("file"))
        .expect("expected file artefact");
    let class = extraction
        .artefacts
        .iter()
        .find(|artefact| artefact.language_kind == "class_specifier" && artefact.name == "Child")
        .expect("expected class artefact");

    assert_eq!(
        class.parent_artifact_key.as_deref(),
        Some(file.artifact_key.as_str())
    );
    assert!(
        extraction
            .edges
            .iter()
            .any(|edge| edge.edge_kind == "imports" && edge.to_symbol_ref.is_some())
    );
}
