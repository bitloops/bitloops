use crate::host::devql::CanonicalKindProjection;
use crate::host::language_adapter::{
    CSharpKind, CanonicalMapping, LanguageKind, MappingCondition,
};

pub(crate) static CSHARP_CANONICAL_MAPPINGS: &[CanonicalMapping] = &[
    CanonicalMapping {
        language_kind: LanguageKind::csharp(CSharpKind::Class),
        projection: CanonicalKindProjection::Type,
        condition: MappingCondition::Always,
    },
    CanonicalMapping {
        language_kind: LanguageKind::csharp(CSharpKind::Struct),
        projection: CanonicalKindProjection::Type,
        condition: MappingCondition::Always,
    },
    CanonicalMapping {
        language_kind: LanguageKind::csharp(CSharpKind::Record),
        projection: CanonicalKindProjection::Type,
        condition: MappingCondition::Always,
    },
    CanonicalMapping {
        language_kind: LanguageKind::csharp(CSharpKind::Delegate),
        projection: CanonicalKindProjection::Type,
        condition: MappingCondition::Always,
    },
    CanonicalMapping {
        language_kind: LanguageKind::csharp(CSharpKind::Interface),
        projection: CanonicalKindProjection::Interface,
        condition: MappingCondition::Always,
    },
    CanonicalMapping {
        language_kind: LanguageKind::csharp(CSharpKind::Enum),
        projection: CanonicalKindProjection::Enum,
        condition: MappingCondition::Always,
    },
    CanonicalMapping {
        language_kind: LanguageKind::csharp(CSharpKind::Method),
        projection: CanonicalKindProjection::Method,
        condition: MappingCondition::WhenInsideParent,
    },
    CanonicalMapping {
        language_kind: LanguageKind::csharp(CSharpKind::Method),
        projection: CanonicalKindProjection::Function,
        condition: MappingCondition::Always,
    },
    CanonicalMapping {
        language_kind: LanguageKind::csharp(CSharpKind::Constructor),
        projection: CanonicalKindProjection::Method,
        condition: MappingCondition::Always,
    },
    CanonicalMapping {
        language_kind: LanguageKind::csharp(CSharpKind::Property),
        projection: CanonicalKindProjection::Variable,
        condition: MappingCondition::Always,
    },
    CanonicalMapping {
        language_kind: LanguageKind::csharp(CSharpKind::Field),
        projection: CanonicalKindProjection::Variable,
        condition: MappingCondition::Always,
    },
    CanonicalMapping {
        language_kind: LanguageKind::csharp(CSharpKind::Using),
        projection: CanonicalKindProjection::Import,
        condition: MappingCondition::Always,
    },
];

pub(crate) static CSHARP_SUPPORTED_LANGUAGE_KINDS: &[LanguageKind] = &[
    LanguageKind::csharp(CSharpKind::Class),
    LanguageKind::csharp(CSharpKind::Constructor),
    LanguageKind::csharp(CSharpKind::Method),
    LanguageKind::csharp(CSharpKind::Property),
    LanguageKind::csharp(CSharpKind::Field),
    LanguageKind::csharp(CSharpKind::Interface),
    LanguageKind::csharp(CSharpKind::Enum),
    LanguageKind::csharp(CSharpKind::Struct),
    LanguageKind::csharp(CSharpKind::Record),
    LanguageKind::csharp(CSharpKind::Delegate),
    LanguageKind::csharp(CSharpKind::Namespace),
    LanguageKind::csharp(CSharpKind::FileScopedNamespace),
    LanguageKind::csharp(CSharpKind::Using),
];
