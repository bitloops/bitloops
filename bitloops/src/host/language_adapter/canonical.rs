use crate::host::devql::CanonicalKindProjection;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CanonicalMapping {
    pub(crate) language_kind: &'static str,
    pub(crate) projection: CanonicalKindProjection,
    pub(crate) condition: MappingCondition,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MappingCondition {
    Always,
    WhenInsideParent,
}

/// Resolve the canonical kind for a language_kind using a pack's mapping table.
/// When `inside_parent` is true and a `WhenInsideParent` entry exists for this
/// language_kind, it takes precedence over the `Always` entry.
pub(crate) fn resolve_canonical_kind(
    mappings: &[CanonicalMapping],
    language_kind: &str,
    inside_parent: bool,
) -> Option<CanonicalKindProjection> {
    let mut always_match = None;
    let mut parent_match = None;

    for mapping in mappings {
        if mapping.language_kind != language_kind {
            continue;
        }
        match mapping.condition {
            MappingCondition::Always => always_match = Some(mapping.projection),
            MappingCondition::WhenInsideParent => parent_match = Some(mapping.projection),
        }
    }

    if inside_parent {
        parent_match.or(always_match)
    } else {
        always_match
    }
}

/// Check whether a language_kind is in the supported set.
pub(crate) fn is_supported_language_kind(
    supported: &[&str],
    language_kind: &str,
) -> bool {
    supported.contains(&language_kind)
}
