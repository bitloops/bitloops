use crate::host::devql::CanonicalKindProjection;
use crate::host::language_adapter::{CanonicalMapping, MappingCondition};

pub(crate) static RUST_CANONICAL_MAPPINGS: &[CanonicalMapping] = &[
    CanonicalMapping {
        language_kind: "function_item",
        projection: CanonicalKindProjection::Function,
        condition: MappingCondition::Always,
    },
    CanonicalMapping {
        language_kind: "function_item",
        projection: CanonicalKindProjection::Method,
        condition: MappingCondition::WhenInsideParent,
    },
    CanonicalMapping {
        language_kind: "trait_item",
        projection: CanonicalKindProjection::Interface,
        condition: MappingCondition::Always,
    },
    CanonicalMapping {
        language_kind: "type_item",
        projection: CanonicalKindProjection::Type,
        condition: MappingCondition::Always,
    },
    CanonicalMapping {
        language_kind: "enum_item",
        projection: CanonicalKindProjection::Enum,
        condition: MappingCondition::Always,
    },
    CanonicalMapping {
        language_kind: "use_declaration",
        projection: CanonicalKindProjection::Import,
        condition: MappingCondition::Always,
    },
    CanonicalMapping {
        language_kind: "mod_item",
        projection: CanonicalKindProjection::Module,
        condition: MappingCondition::Always,
    },
    CanonicalMapping {
        language_kind: "let_declaration",
        projection: CanonicalKindProjection::Variable,
        condition: MappingCondition::Always,
    },
];

pub(crate) static RUST_SUPPORTED_LANGUAGE_KINDS: &[&str] = &[
    "function_item",
    "trait_item",
    "type_item",
    "enum_item",
    "use_declaration",
    "mod_item",
    "let_declaration",
    "impl_item",
    "struct_item",
    "const_item",
    "static_item",
    "macro_definition",
];
