use crate::host::devql::CanonicalKindProjection;
use crate::host::language_adapter::{CanonicalMapping, LanguageKind, MappingCondition};

pub(crate) static TS_JS_CANONICAL_MAPPINGS: &[CanonicalMapping] = &[
    CanonicalMapping {
        language_kind: LanguageKind::FunctionDeclaration.as_str(),
        projection: CanonicalKindProjection::Function,
        condition: MappingCondition::Always,
    },
    CanonicalMapping {
        language_kind: LanguageKind::MethodDefinition.as_str(),
        projection: CanonicalKindProjection::Method,
        condition: MappingCondition::Always,
    },
    CanonicalMapping {
        language_kind: LanguageKind::InterfaceDeclaration.as_str(),
        projection: CanonicalKindProjection::Interface,
        condition: MappingCondition::Always,
    },
    CanonicalMapping {
        language_kind: LanguageKind::TypeAliasDeclaration.as_str(),
        projection: CanonicalKindProjection::Type,
        condition: MappingCondition::Always,
    },
    CanonicalMapping {
        language_kind: LanguageKind::EnumDeclaration.as_str(),
        projection: CanonicalKindProjection::Enum,
        condition: MappingCondition::Always,
    },
    CanonicalMapping {
        language_kind: LanguageKind::VariableDeclarator.as_str(),
        projection: CanonicalKindProjection::Variable,
        condition: MappingCondition::Always,
    },
    CanonicalMapping {
        language_kind: LanguageKind::ImportStatement.as_str(),
        projection: CanonicalKindProjection::Import,
        condition: MappingCondition::Always,
    },
    CanonicalMapping {
        language_kind: LanguageKind::ModuleDeclaration.as_str(),
        projection: CanonicalKindProjection::Module,
        condition: MappingCondition::Always,
    },
    CanonicalMapping {
        language_kind: LanguageKind::InternalModule.as_str(),
        projection: CanonicalKindProjection::Module,
        condition: MappingCondition::Always,
    },
];

pub(crate) static TS_JS_SUPPORTED_LANGUAGE_KINDS: &[&str] = &[
    LanguageKind::FunctionDeclaration.as_str(),
    LanguageKind::MethodDefinition.as_str(),
    LanguageKind::InterfaceDeclaration.as_str(),
    LanguageKind::TypeAliasDeclaration.as_str(),
    LanguageKind::EnumDeclaration.as_str(),
    LanguageKind::VariableDeclarator.as_str(),
    LanguageKind::ImportStatement.as_str(),
    LanguageKind::ModuleDeclaration.as_str(),
    LanguageKind::InternalModule.as_str(),
    LanguageKind::ClassDeclaration.as_str(),
    LanguageKind::Constructor.as_str(),
    LanguageKind::PropertyDeclaration.as_str(),
    LanguageKind::PublicFieldDefinition.as_str(),
];
