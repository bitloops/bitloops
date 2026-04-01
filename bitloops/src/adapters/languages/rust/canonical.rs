use crate::host::devql::CanonicalKindProjection;
use crate::host::language_adapter::{CanonicalMapping, LanguageKind, MappingCondition, RustKind};

pub(crate) static RUST_CANONICAL_MAPPINGS: &[CanonicalMapping] = &[
    CanonicalMapping {
        language_kind: LanguageKind::rust(RustKind::FunctionItem),
        projection: CanonicalKindProjection::Function,
        condition: MappingCondition::Always,
    },
    CanonicalMapping {
        language_kind: LanguageKind::rust(RustKind::FunctionItem),
        projection: CanonicalKindProjection::Method,
        condition: MappingCondition::WhenInsideParent,
    },
    CanonicalMapping {
        language_kind: LanguageKind::rust(RustKind::TraitItem),
        projection: CanonicalKindProjection::Interface,
        condition: MappingCondition::Always,
    },
    CanonicalMapping {
        language_kind: LanguageKind::rust(RustKind::TypeItem),
        projection: CanonicalKindProjection::Type,
        condition: MappingCondition::Always,
    },
    CanonicalMapping {
        language_kind: LanguageKind::rust(RustKind::EnumItem),
        projection: CanonicalKindProjection::Enum,
        condition: MappingCondition::Always,
    },
    CanonicalMapping {
        language_kind: LanguageKind::rust(RustKind::UseDeclaration),
        projection: CanonicalKindProjection::Import,
        condition: MappingCondition::Always,
    },
    CanonicalMapping {
        language_kind: LanguageKind::rust(RustKind::ModItem),
        projection: CanonicalKindProjection::Module,
        condition: MappingCondition::Always,
    },
    CanonicalMapping {
        language_kind: LanguageKind::rust(RustKind::LetDeclaration),
        projection: CanonicalKindProjection::Variable,
        condition: MappingCondition::Always,
    },
];

pub(crate) static RUST_SUPPORTED_LANGUAGE_KINDS: &[LanguageKind] = &[
    LanguageKind::rust(RustKind::FunctionItem),
    LanguageKind::rust(RustKind::TraitItem),
    LanguageKind::rust(RustKind::TypeItem),
    LanguageKind::rust(RustKind::EnumItem),
    LanguageKind::rust(RustKind::UseDeclaration),
    LanguageKind::rust(RustKind::ModItem),
    LanguageKind::rust(RustKind::LetDeclaration),
    LanguageKind::rust(RustKind::ImplItem),
    LanguageKind::rust(RustKind::StructItem),
    LanguageKind::rust(RustKind::ConstItem),
    LanguageKind::rust(RustKind::StaticItem),
    LanguageKind::rust(RustKind::MacroDefinition),
];
