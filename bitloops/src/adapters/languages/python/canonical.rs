use crate::host::devql::CanonicalKindProjection;
use crate::host::language_adapter::{CanonicalMapping, LanguageKind, MappingCondition};

pub(crate) static PYTHON_CANONICAL_MAPPINGS: &[CanonicalMapping] = &[
    CanonicalMapping {
        language_kind: LanguageKind::FunctionDefinition.as_str(),
        projection: CanonicalKindProjection::Function,
        condition: MappingCondition::Always,
    },
    CanonicalMapping {
        language_kind: LanguageKind::FunctionDefinition.as_str(),
        projection: CanonicalKindProjection::Method,
        condition: MappingCondition::WhenInsideParent,
    },
    CanonicalMapping {
        language_kind: LanguageKind::ClassDefinition.as_str(),
        projection: CanonicalKindProjection::Type,
        condition: MappingCondition::Always,
    },
    CanonicalMapping {
        language_kind: LanguageKind::ImportStatement.as_str(),
        projection: CanonicalKindProjection::Import,
        condition: MappingCondition::Always,
    },
    CanonicalMapping {
        language_kind: LanguageKind::ImportFromStatement.as_str(),
        projection: CanonicalKindProjection::Import,
        condition: MappingCondition::Always,
    },
    CanonicalMapping {
        language_kind: LanguageKind::FutureImportStatement.as_str(),
        projection: CanonicalKindProjection::Import,
        condition: MappingCondition::Always,
    },
    CanonicalMapping {
        language_kind: LanguageKind::Assignment.as_str(),
        projection: CanonicalKindProjection::Variable,
        condition: MappingCondition::Always,
    },
];

pub(crate) static PYTHON_SUPPORTED_LANGUAGE_KINDS: &[&str] = &[
    LanguageKind::FunctionDefinition.as_str(),
    LanguageKind::ClassDefinition.as_str(),
    LanguageKind::ImportStatement.as_str(),
    LanguageKind::ImportFromStatement.as_str(),
    LanguageKind::FutureImportStatement.as_str(),
    LanguageKind::Assignment.as_str(),
];
