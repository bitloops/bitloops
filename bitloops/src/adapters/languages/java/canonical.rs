use crate::host::devql::CanonicalKindProjection;
use crate::host::language_adapter::{CanonicalMapping, JavaKind, LanguageKind, MappingCondition};

pub(crate) static JAVA_CANONICAL_MAPPINGS: &[CanonicalMapping] = &[
    CanonicalMapping {
        language_kind: LanguageKind::java(JavaKind::Package),
        projection: CanonicalKindProjection::Module,
        condition: MappingCondition::Always,
    },
    CanonicalMapping {
        language_kind: LanguageKind::java(JavaKind::Import),
        projection: CanonicalKindProjection::Import,
        condition: MappingCondition::Always,
    },
    CanonicalMapping {
        language_kind: LanguageKind::java(JavaKind::Class),
        projection: CanonicalKindProjection::Type,
        condition: MappingCondition::Always,
    },
    CanonicalMapping {
        language_kind: LanguageKind::java(JavaKind::Interface),
        projection: CanonicalKindProjection::Interface,
        condition: MappingCondition::Always,
    },
    CanonicalMapping {
        language_kind: LanguageKind::java(JavaKind::Enum),
        projection: CanonicalKindProjection::Enum,
        condition: MappingCondition::Always,
    },
    CanonicalMapping {
        language_kind: LanguageKind::java(JavaKind::Constructor),
        projection: CanonicalKindProjection::Method,
        condition: MappingCondition::Always,
    },
    CanonicalMapping {
        language_kind: LanguageKind::java(JavaKind::Method),
        projection: CanonicalKindProjection::Method,
        condition: MappingCondition::Always,
    },
    CanonicalMapping {
        language_kind: LanguageKind::java(JavaKind::Field),
        projection: CanonicalKindProjection::Variable,
        condition: MappingCondition::Always,
    },
];

pub(crate) static JAVA_SUPPORTED_LANGUAGE_KINDS: &[LanguageKind] = &[
    LanguageKind::java(JavaKind::Package),
    LanguageKind::java(JavaKind::Import),
    LanguageKind::java(JavaKind::Class),
    LanguageKind::java(JavaKind::Interface),
    LanguageKind::java(JavaKind::Enum),
    LanguageKind::java(JavaKind::Constructor),
    LanguageKind::java(JavaKind::Method),
    LanguageKind::java(JavaKind::Field),
];
