use crate::host::devql::CanonicalKindProjection;
use crate::host::language_adapter::{CanonicalMapping, MappingCondition};

pub(crate) static PYTHON_CANONICAL_MAPPINGS: &[CanonicalMapping] = &[
    CanonicalMapping {
        language_kind: "function_definition",
        projection: CanonicalKindProjection::Function,
        condition: MappingCondition::Always,
    },
    CanonicalMapping {
        language_kind: "function_definition",
        projection: CanonicalKindProjection::Method,
        condition: MappingCondition::WhenInsideParent,
    },
    CanonicalMapping {
        language_kind: "class_definition",
        projection: CanonicalKindProjection::Type,
        condition: MappingCondition::Always,
    },
    CanonicalMapping {
        language_kind: "import_statement",
        projection: CanonicalKindProjection::Import,
        condition: MappingCondition::Always,
    },
    CanonicalMapping {
        language_kind: "import_from_statement",
        projection: CanonicalKindProjection::Import,
        condition: MappingCondition::Always,
    },
    CanonicalMapping {
        language_kind: "future_import_statement",
        projection: CanonicalKindProjection::Import,
        condition: MappingCondition::Always,
    },
    CanonicalMapping {
        language_kind: "assignment",
        projection: CanonicalKindProjection::Variable,
        condition: MappingCondition::Always,
    },
];

pub(crate) static PYTHON_SUPPORTED_LANGUAGE_KINDS: &[&str] = &[
    "function_definition",
    "class_definition",
    "import_statement",
    "import_from_statement",
    "future_import_statement",
    "assignment",
];
