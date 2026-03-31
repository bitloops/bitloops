use crate::host::devql::CanonicalKindProjection;
use crate::host::language_adapter::{CanonicalMapping, LanguageKind, MappingCondition};

pub(crate) static RUST_CANONICAL_MAPPINGS: &[CanonicalMapping] = &[
    CanonicalMapping {
        language_kind: LanguageKind::FunctionItem.as_str(),
        projection: CanonicalKindProjection::Function,
        condition: MappingCondition::Always,
    },
    CanonicalMapping {
        language_kind: LanguageKind::FunctionItem.as_str(),
        projection: CanonicalKindProjection::Method,
        condition: MappingCondition::WhenInsideParent,
    },
    CanonicalMapping {
        language_kind: LanguageKind::TraitItem.as_str(),
        projection: CanonicalKindProjection::Interface,
        condition: MappingCondition::Always,
    },
    CanonicalMapping {
        language_kind: LanguageKind::TypeItem.as_str(),
        projection: CanonicalKindProjection::Type,
        condition: MappingCondition::Always,
    },
    CanonicalMapping {
        language_kind: LanguageKind::EnumItem.as_str(),
        projection: CanonicalKindProjection::Enum,
        condition: MappingCondition::Always,
    },
    CanonicalMapping {
        language_kind: LanguageKind::UseDeclaration.as_str(),
        projection: CanonicalKindProjection::Import,
        condition: MappingCondition::Always,
    },
    CanonicalMapping {
        language_kind: LanguageKind::ModItem.as_str(),
        projection: CanonicalKindProjection::Module,
        condition: MappingCondition::Always,
    },
    CanonicalMapping {
        language_kind: LanguageKind::LetDeclaration.as_str(),
        projection: CanonicalKindProjection::Variable,
        condition: MappingCondition::Always,
    },
];

pub(crate) static RUST_SUPPORTED_LANGUAGE_KINDS: &[&str] = &[
    LanguageKind::FunctionItem.as_str(),
    LanguageKind::TraitItem.as_str(),
    LanguageKind::TypeItem.as_str(),
    LanguageKind::EnumItem.as_str(),
    LanguageKind::UseDeclaration.as_str(),
    LanguageKind::ModItem.as_str(),
    LanguageKind::LetDeclaration.as_str(),
    LanguageKind::ImplItem.as_str(),
    LanguageKind::StructItem.as_str(),
    LanguageKind::ConstItem.as_str(),
    LanguageKind::StaticItem.as_str(),
    LanguageKind::MacroDefinition.as_str(),
];
