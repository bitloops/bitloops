use crate::host::devql::CanonicalKindProjection;
use crate::host::language_adapter::{CanonicalMapping, LanguageKind, MappingCondition};

pub(crate) static GO_CANONICAL_MAPPINGS: &[CanonicalMapping] = &[
    CanonicalMapping {
        language_kind: LanguageKind::FunctionDeclaration.as_str(),
        projection: CanonicalKindProjection::Function,
        condition: MappingCondition::Always,
    },
    CanonicalMapping {
        language_kind: LanguageKind::MethodDeclaration.as_str(),
        projection: CanonicalKindProjection::Method,
        condition: MappingCondition::Always,
    },
    CanonicalMapping {
        language_kind: LanguageKind::TypeSpec.as_str(),
        projection: CanonicalKindProjection::Type,
        condition: MappingCondition::Always,
    },
    CanonicalMapping {
        language_kind: LanguageKind::TypeAlias.as_str(),
        projection: CanonicalKindProjection::Type,
        condition: MappingCondition::Always,
    },
    CanonicalMapping {
        language_kind: LanguageKind::StructType.as_str(),
        projection: CanonicalKindProjection::Type,
        condition: MappingCondition::Always,
    },
    CanonicalMapping {
        language_kind: LanguageKind::InterfaceType.as_str(),
        projection: CanonicalKindProjection::Interface,
        condition: MappingCondition::Always,
    },
    CanonicalMapping {
        language_kind: LanguageKind::ImportSpec.as_str(),
        projection: CanonicalKindProjection::Import,
        condition: MappingCondition::Always,
    },
    CanonicalMapping {
        language_kind: LanguageKind::VarSpec.as_str(),
        projection: CanonicalKindProjection::Variable,
        condition: MappingCondition::Always,
    },
    CanonicalMapping {
        language_kind: LanguageKind::ConstSpec.as_str(),
        projection: CanonicalKindProjection::Variable,
        condition: MappingCondition::Always,
    },
];

pub(crate) static GO_SUPPORTED_LANGUAGE_KINDS: &[&str] = &[
    LanguageKind::FunctionDeclaration.as_str(),
    LanguageKind::MethodDeclaration.as_str(),
    LanguageKind::TypeSpec.as_str(),
    LanguageKind::TypeAlias.as_str(),
    LanguageKind::StructType.as_str(),
    LanguageKind::InterfaceType.as_str(),
    LanguageKind::ImportSpec.as_str(),
    LanguageKind::VarSpec.as_str(),
    LanguageKind::ConstSpec.as_str(),
];
