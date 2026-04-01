use crate::host::devql::CanonicalKindProjection;
use crate::host::language_adapter::{CanonicalMapping, MappingCondition, PythonKinds};

pub(crate) static PYTHON_CANONICAL_MAPPINGS: &[CanonicalMapping] = &[
    CanonicalMapping {
        language_kind: PythonKinds::FunctionDefinition,
        projection: CanonicalKindProjection::Function,
        condition: MappingCondition::Always,
    },
    CanonicalMapping {
        language_kind: PythonKinds::FunctionDefinition,
        projection: CanonicalKindProjection::Method,
        condition: MappingCondition::WhenInsideParent,
    },
    CanonicalMapping {
        language_kind: PythonKinds::ClassDefinition,
        projection: CanonicalKindProjection::Type,
        condition: MappingCondition::Always,
    },
    CanonicalMapping {
        language_kind: PythonKinds::ImportStatement,
        projection: CanonicalKindProjection::Import,
        condition: MappingCondition::Always,
    },
    CanonicalMapping {
        language_kind: PythonKinds::ImportFromStatement,
        projection: CanonicalKindProjection::Import,
        condition: MappingCondition::Always,
    },
    CanonicalMapping {
        language_kind: PythonKinds::FutureImportStatement,
        projection: CanonicalKindProjection::Import,
        condition: MappingCondition::Always,
    },
    CanonicalMapping {
        language_kind: PythonKinds::Assignment,
        projection: CanonicalKindProjection::Variable,
        condition: MappingCondition::Always,
    },
];

pub(crate) static PYTHON_SUPPORTED_LANGUAGE_KINDS: &[&str] = &[
    PythonKinds::FunctionDefinition,
    PythonKinds::ClassDefinition,
    PythonKinds::ImportStatement,
    PythonKinds::ImportFromStatement,
    PythonKinds::FutureImportStatement,
    PythonKinds::Assignment,
];
