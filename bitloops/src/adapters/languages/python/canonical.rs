use crate::host::devql::CanonicalKindProjection;
use crate::host::language_adapter::{CanonicalMapping, LanguageKind, MappingCondition, PythonKind};

pub(crate) static PYTHON_CANONICAL_MAPPINGS: &[CanonicalMapping] = &[
    CanonicalMapping {
        language_kind: LanguageKind::python(PythonKind::FunctionDefinition),
        projection: CanonicalKindProjection::Function,
        condition: MappingCondition::Always,
    },
    CanonicalMapping {
        language_kind: LanguageKind::python(PythonKind::FunctionDefinition),
        projection: CanonicalKindProjection::Method,
        condition: MappingCondition::WhenInsideParent,
    },
    CanonicalMapping {
        language_kind: LanguageKind::python(PythonKind::ClassDefinition),
        projection: CanonicalKindProjection::Type,
        condition: MappingCondition::Always,
    },
    CanonicalMapping {
        language_kind: LanguageKind::python(PythonKind::ImportStatement),
        projection: CanonicalKindProjection::Import,
        condition: MappingCondition::Always,
    },
    CanonicalMapping {
        language_kind: LanguageKind::python(PythonKind::ImportFromStatement),
        projection: CanonicalKindProjection::Import,
        condition: MappingCondition::Always,
    },
    CanonicalMapping {
        language_kind: LanguageKind::python(PythonKind::FutureImportStatement),
        projection: CanonicalKindProjection::Import,
        condition: MappingCondition::Always,
    },
    CanonicalMapping {
        language_kind: LanguageKind::python(PythonKind::Assignment),
        projection: CanonicalKindProjection::Variable,
        condition: MappingCondition::Always,
    },
];

pub(crate) static PYTHON_SUPPORTED_LANGUAGE_KINDS: &[LanguageKind] = &[
    LanguageKind::python(PythonKind::FunctionDefinition),
    LanguageKind::python(PythonKind::ClassDefinition),
    LanguageKind::python(PythonKind::ImportStatement),
    LanguageKind::python(PythonKind::ImportFromStatement),
    LanguageKind::python(PythonKind::FutureImportStatement),
    LanguageKind::python(PythonKind::Assignment),
];
