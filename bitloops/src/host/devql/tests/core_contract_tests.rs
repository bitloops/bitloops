use super::*;
use crate::adapters::languages::rust::canonical::RUST_CANONICAL_MAPPINGS;
use crate::adapters::languages::ts_js::canonical::TS_JS_CANONICAL_MAPPINGS;
use crate::host::language_adapter::resolve_canonical_kind;

#[test]
fn canonical_projection_maps_to_core_contract_kinds() {
    assert_eq!(
        resolve_canonical_kind(TS_JS_CANONICAL_MAPPINGS, "function_declaration", false),
        Some(CanonicalKindProjection::Function)
    );
    assert_eq!(
        resolve_canonical_kind(RUST_CANONICAL_MAPPINGS, "function_item", true),
        Some(CanonicalKindProjection::Method)
    );
    assert_eq!(
        artefact_core_kind(Some("interface")),
        Some(CoreCanonicalArtefactKind::Type)
    );
    assert!(artefact_has_core_kind(
        Some("method"),
        CoreCanonicalArtefactKind::Callable
    ));
}

#[test]
fn canonical_kind_filter_sql_supports_core_aliases_without_breaking_legacy_values() {
    assert_eq!(
        canonical_kind_filter_sql("a.canonical_kind", "function"),
        "a.canonical_kind = 'function'"
    );
    assert_eq!(
        canonical_kind_filter_sql("a.canonical_kind", "callable"),
        "(a.canonical_kind = 'callable' OR a.canonical_kind = 'function' OR a.canonical_kind = 'method')"
    );
    assert_eq!(
        canonical_kind_filter_sql("a.canonical_kind", "custom_kind"),
        "a.canonical_kind = 'custom_kind'"
    );
}

#[test]
fn temporal_and_provenance_contracts_capture_revision_scope() {
    assert_eq!(
        TemporalRevisionKind::from_str("commit"),
        Some(TemporalRevisionKind::Commit)
    );
    assert_eq!(
        TemporalRevisionKind::from_str("temporary"),
        Some(TemporalRevisionKind::Temporary)
    );

    let provenance = CanonicalProvenanceRef::for_blob("repo-a", "blob-1")
        .with_source_anchor("commit-1", "src/lib.rs");
    assert_eq!(
        provenance.temporal_identity_scope().as_deref(),
        Some("commit-1|src/lib.rs|blob-1")
    );
}
