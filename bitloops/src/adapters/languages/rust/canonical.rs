use crate::host::devql::CanonicalKindProjection;
use crate::host::language_adapter::{CanonicalMapping, MappingCondition, RustKinds};

pub(crate) static RUST_CANONICAL_MAPPINGS: &[CanonicalMapping] = &[
    CanonicalMapping {
        language_kind: RustKinds::FunctionItem,
        projection: CanonicalKindProjection::Function,
        condition: MappingCondition::Always,
    },
    CanonicalMapping {
        language_kind: RustKinds::FunctionItem,
        projection: CanonicalKindProjection::Method,
        condition: MappingCondition::WhenInsideParent,
    },
    CanonicalMapping {
        language_kind: RustKinds::TraitItem,
        projection: CanonicalKindProjection::Interface,
        condition: MappingCondition::Always,
    },
    CanonicalMapping {
        language_kind: RustKinds::TypeItem,
        projection: CanonicalKindProjection::Type,
        condition: MappingCondition::Always,
    },
    CanonicalMapping {
        language_kind: RustKinds::EnumItem,
        projection: CanonicalKindProjection::Enum,
        condition: MappingCondition::Always,
    },
    CanonicalMapping {
        language_kind: RustKinds::UseDeclaration,
        projection: CanonicalKindProjection::Import,
        condition: MappingCondition::Always,
    },
    CanonicalMapping {
        language_kind: RustKinds::ModItem,
        projection: CanonicalKindProjection::Module,
        condition: MappingCondition::Always,
    },
    CanonicalMapping {
        language_kind: RustKinds::LetDeclaration,
        projection: CanonicalKindProjection::Variable,
        condition: MappingCondition::Always,
    },
];

pub(crate) static RUST_SUPPORTED_LANGUAGE_KINDS: &[&str] = &[
    RustKinds::FunctionItem,
    RustKinds::TraitItem,
    RustKinds::TypeItem,
    RustKinds::EnumItem,
    RustKinds::UseDeclaration,
    RustKinds::ModItem,
    RustKinds::LetDeclaration,
    RustKinds::ImplItem,
    RustKinds::StructItem,
    RustKinds::ConstItem,
    RustKinds::StaticItem,
    RustKinds::MacroDefinition,
];
