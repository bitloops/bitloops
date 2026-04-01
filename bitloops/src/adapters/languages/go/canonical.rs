use crate::host::devql::CanonicalKindProjection;
use crate::host::language_adapter::{CanonicalMapping, GoKind, LanguageKind, MappingCondition};

pub(crate) static GO_CANONICAL_MAPPINGS: &[CanonicalMapping] = &[
    CanonicalMapping {
        language_kind: LanguageKind::go(GoKind::FunctionDeclaration),
        projection: CanonicalKindProjection::Function,
        condition: MappingCondition::Always,
    },
    CanonicalMapping {
        language_kind: LanguageKind::go(GoKind::MethodDeclaration),
        projection: CanonicalKindProjection::Method,
        condition: MappingCondition::Always,
    },
    CanonicalMapping {
        language_kind: LanguageKind::go(GoKind::TypeSpec),
        projection: CanonicalKindProjection::Type,
        condition: MappingCondition::Always,
    },
    CanonicalMapping {
        language_kind: LanguageKind::go(GoKind::TypeAlias),
        projection: CanonicalKindProjection::Type,
        condition: MappingCondition::Always,
    },
    CanonicalMapping {
        language_kind: LanguageKind::go(GoKind::StructType),
        projection: CanonicalKindProjection::Type,
        condition: MappingCondition::Always,
    },
    CanonicalMapping {
        language_kind: LanguageKind::go(GoKind::InterfaceType),
        projection: CanonicalKindProjection::Interface,
        condition: MappingCondition::Always,
    },
    CanonicalMapping {
        language_kind: LanguageKind::go(GoKind::ImportSpec),
        projection: CanonicalKindProjection::Import,
        condition: MappingCondition::Always,
    },
    CanonicalMapping {
        language_kind: LanguageKind::go(GoKind::VarSpec),
        projection: CanonicalKindProjection::Variable,
        condition: MappingCondition::Always,
    },
    CanonicalMapping {
        language_kind: LanguageKind::go(GoKind::ConstSpec),
        projection: CanonicalKindProjection::Variable,
        condition: MappingCondition::Always,
    },
];

pub(crate) static GO_SUPPORTED_LANGUAGE_KINDS: &[LanguageKind] = &[
    LanguageKind::go(GoKind::FunctionDeclaration),
    LanguageKind::go(GoKind::MethodDeclaration),
    LanguageKind::go(GoKind::TypeSpec),
    LanguageKind::go(GoKind::TypeAlias),
    LanguageKind::go(GoKind::StructType),
    LanguageKind::go(GoKind::InterfaceType),
    LanguageKind::go(GoKind::ImportSpec),
    LanguageKind::go(GoKind::VarSpec),
    LanguageKind::go(GoKind::ConstSpec),
];
