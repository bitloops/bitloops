use crate::host::devql::CanonicalKindProjection;
use crate::host::language_adapter::{CanonicalMapping, GolangKinds, MappingCondition};

pub(crate) static GO_CANONICAL_MAPPINGS: &[CanonicalMapping] = &[
    CanonicalMapping {
        language_kind: GolangKinds::FunctionDeclaration,
        projection: CanonicalKindProjection::Function,
        condition: MappingCondition::Always,
    },
    CanonicalMapping {
        language_kind: GolangKinds::MethodDeclaration,
        projection: CanonicalKindProjection::Method,
        condition: MappingCondition::Always,
    },
    CanonicalMapping {
        language_kind: GolangKinds::TypeSpec,
        projection: CanonicalKindProjection::Type,
        condition: MappingCondition::Always,
    },
    CanonicalMapping {
        language_kind: GolangKinds::TypeAlias,
        projection: CanonicalKindProjection::Type,
        condition: MappingCondition::Always,
    },
    CanonicalMapping {
        language_kind: GolangKinds::StructType,
        projection: CanonicalKindProjection::Type,
        condition: MappingCondition::Always,
    },
    CanonicalMapping {
        language_kind: GolangKinds::InterfaceType,
        projection: CanonicalKindProjection::Interface,
        condition: MappingCondition::Always,
    },
    CanonicalMapping {
        language_kind: GolangKinds::ImportSpec,
        projection: CanonicalKindProjection::Import,
        condition: MappingCondition::Always,
    },
    CanonicalMapping {
        language_kind: GolangKinds::VarSpec,
        projection: CanonicalKindProjection::Variable,
        condition: MappingCondition::Always,
    },
    CanonicalMapping {
        language_kind: GolangKinds::ConstSpec,
        projection: CanonicalKindProjection::Variable,
        condition: MappingCondition::Always,
    },
];

pub(crate) static GO_SUPPORTED_LANGUAGE_KINDS: &[&str] = &[
    GolangKinds::FunctionDeclaration,
    GolangKinds::MethodDeclaration,
    GolangKinds::TypeSpec,
    GolangKinds::TypeAlias,
    GolangKinds::StructType,
    GolangKinds::InterfaceType,
    GolangKinds::ImportSpec,
    GolangKinds::VarSpec,
    GolangKinds::ConstSpec,
];
