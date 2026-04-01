use crate::host::devql::CanonicalKindProjection;
use crate::host::language_adapter::{CanonicalMapping, MappingCondition, TsJsKinds};

pub(crate) static TS_JS_CANONICAL_MAPPINGS: &[CanonicalMapping] = &[
    CanonicalMapping {
        language_kind: TsJsKinds::FunctionDeclaration,
        projection: CanonicalKindProjection::Function,
        condition: MappingCondition::Always,
    },
    CanonicalMapping {
        language_kind: TsJsKinds::MethodDefinition,
        projection: CanonicalKindProjection::Method,
        condition: MappingCondition::Always,
    },
    CanonicalMapping {
        language_kind: TsJsKinds::InterfaceDeclaration,
        projection: CanonicalKindProjection::Interface,
        condition: MappingCondition::Always,
    },
    CanonicalMapping {
        language_kind: TsJsKinds::TypeAliasDeclaration,
        projection: CanonicalKindProjection::Type,
        condition: MappingCondition::Always,
    },
    CanonicalMapping {
        language_kind: TsJsKinds::EnumDeclaration,
        projection: CanonicalKindProjection::Enum,
        condition: MappingCondition::Always,
    },
    CanonicalMapping {
        language_kind: TsJsKinds::VariableDeclarator,
        projection: CanonicalKindProjection::Variable,
        condition: MappingCondition::Always,
    },
    CanonicalMapping {
        language_kind: TsJsKinds::ImportStatement,
        projection: CanonicalKindProjection::Import,
        condition: MappingCondition::Always,
    },
    CanonicalMapping {
        language_kind: TsJsKinds::ModuleDeclaration,
        projection: CanonicalKindProjection::Module,
        condition: MappingCondition::Always,
    },
    CanonicalMapping {
        language_kind: TsJsKinds::InternalModule,
        projection: CanonicalKindProjection::Module,
        condition: MappingCondition::Always,
    },
];

pub(crate) static TS_JS_SUPPORTED_LANGUAGE_KINDS: &[&str] = &[
    TsJsKinds::FunctionDeclaration,
    TsJsKinds::MethodDefinition,
    TsJsKinds::InterfaceDeclaration,
    TsJsKinds::TypeAliasDeclaration,
    TsJsKinds::EnumDeclaration,
    TsJsKinds::VariableDeclarator,
    TsJsKinds::ImportStatement,
    TsJsKinds::ModuleDeclaration,
    TsJsKinds::InternalModule,
    TsJsKinds::ClassDeclaration,
    TsJsKinds::Constructor,
    TsJsKinds::PropertyDeclaration,
    TsJsKinds::PublicFieldDefinition,
];
