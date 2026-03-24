//! Test artefact identity functions.
//!
//! The test harness follows the same deterministic identity algorithm as the
//! core artefact pipeline while keeping the pack-level API local.

fn normalize_identity_fragment(input: &str) -> String {
    let normalized = input
        .chars()
        .filter(|ch| !ch.is_whitespace())
        .collect::<String>();
    if normalized.is_empty() {
        input.trim().to_string()
    } else {
        normalized
    }
}

/// Stable logical identity for a test artefact (suite or scenario).
pub fn test_structural_symbol_id(
    path: &str,
    canonical_kind: &str,
    language_kind: Option<&str>,
    parent_symbol_id: Option<&str>,
    name: &str,
    signature: Option<&str>,
) -> String {
    let normalized_signature = signature
        .map(normalize_identity_fragment)
        .unwrap_or_default();
    crate::host::devql::deterministic_uuid(&format!(
        "{}|{}|{}|{}|{}|{}",
        path,
        canonical_kind,
        language_kind.unwrap_or("<null>"),
        parent_symbol_id.unwrap_or(""),
        normalize_identity_fragment(name),
        normalized_signature,
    ))
}

/// Revision-specific identity for a test artefact.
pub fn test_revision_artefact_id(repo_id: &str, blob_sha: &str, symbol_id: &str) -> String {
    crate::host::devql::deterministic_uuid(&format!("{repo_id}|{blob_sha}|{symbol_id}"))
}

/// Deterministic edge identity for a test-to-production relationship.
pub fn test_edge_id(
    repo_id: &str,
    from_symbol_id: &str,
    edge_kind: &str,
    to_symbol_id_or_ref: &str,
) -> String {
    crate::host::devql::deterministic_uuid(&format!(
        "{repo_id}|{from_symbol_id}|{edge_kind}|{to_symbol_id_or_ref}"
    ))
}

#[cfg(test)]
mod tests {
    use super::{test_edge_id, test_revision_artefact_id, test_structural_symbol_id};

    #[test]
    fn structural_symbol_id_ignores_signature_whitespace() {
        assert_eq!(
            test_structural_symbol_id(
                "tests/example.rs",
                "test_scenario",
                Some("test_fn"),
                Some("parent"),
                "example_case",
                Some("fn  example_case ( value : i32 )"),
            ),
            test_structural_symbol_id(
                "tests/example.rs",
                "test_scenario",
                Some("test_fn"),
                Some("parent"),
                "example_case",
                Some("fn example_case(value:i32)"),
            )
        );
    }

    #[test]
    fn revision_artefact_id_changes_with_blob_sha() {
        let symbol_id = test_structural_symbol_id(
            "tests/example.rs",
            "test_scenario",
            Some("test_fn"),
            None,
            "example_case",
            Some("fn example_case()"),
        );
        assert_ne!(
            test_revision_artefact_id("repo", "blob-a", &symbol_id),
            test_revision_artefact_id("repo", "blob-b", &symbol_id)
        );
    }

    #[test]
    fn edge_id_is_stable_for_same_natural_key() {
        assert_eq!(
            test_edge_id("repo", "from", "tests", "to-symbol"),
            test_edge_id("repo", "from", "tests", "to-symbol")
        );
    }
}
