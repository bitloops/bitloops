use crate::host::devql::CanonicalKindProjection;
use crate::host::language_adapter::{CanonicalMapping, LanguageKind, MappingCondition, TsJsKind};

pub(crate) static TS_JS_CANONICAL_MAPPINGS: &[CanonicalMapping] = &[
    CanonicalMapping {
        language_kind: LanguageKind::ts_js(TsJsKind::FunctionDeclaration),
        projection: CanonicalKindProjection::Function,
        condition: MappingCondition::Always,
    },
    CanonicalMapping {
        language_kind: LanguageKind::ts_js(TsJsKind::MethodDefinition),
        projection: CanonicalKindProjection::Method,
        condition: MappingCondition::Always,
    },
    CanonicalMapping {
        language_kind: LanguageKind::ts_js(TsJsKind::InterfaceDeclaration),
        projection: CanonicalKindProjection::Interface,
        condition: MappingCondition::Always,
    },
    CanonicalMapping {
        language_kind: LanguageKind::ts_js(TsJsKind::TypeAliasDeclaration),
        projection: CanonicalKindProjection::Type,
        condition: MappingCondition::Always,
    },
    CanonicalMapping {
        language_kind: LanguageKind::ts_js(TsJsKind::EnumDeclaration),
        projection: CanonicalKindProjection::Enum,
        condition: MappingCondition::Always,
    },
    CanonicalMapping {
        language_kind: LanguageKind::ts_js(TsJsKind::VariableDeclarator),
        projection: CanonicalKindProjection::Variable,
        condition: MappingCondition::Always,
    },
    CanonicalMapping {
        language_kind: LanguageKind::ts_js(TsJsKind::ImportStatement),
        projection: CanonicalKindProjection::Import,
        condition: MappingCondition::Always,
    },
    CanonicalMapping {
        language_kind: LanguageKind::ts_js(TsJsKind::ModuleDeclaration),
        projection: CanonicalKindProjection::Module,
        condition: MappingCondition::Always,
    },
    CanonicalMapping {
        language_kind: LanguageKind::ts_js(TsJsKind::InternalModule),
        projection: CanonicalKindProjection::Module,
        condition: MappingCondition::Always,
    },
];

pub(crate) static TS_JS_SUPPORTED_LANGUAGE_KINDS: &[LanguageKind] = &[
    LanguageKind::ts_js(TsJsKind::FunctionDeclaration),
    LanguageKind::ts_js(TsJsKind::MethodDefinition),
    LanguageKind::ts_js(TsJsKind::InterfaceDeclaration),
    LanguageKind::ts_js(TsJsKind::TypeAliasDeclaration),
    LanguageKind::ts_js(TsJsKind::EnumDeclaration),
    LanguageKind::ts_js(TsJsKind::VariableDeclarator),
    LanguageKind::ts_js(TsJsKind::ImportStatement),
    LanguageKind::ts_js(TsJsKind::ModuleDeclaration),
    LanguageKind::ts_js(TsJsKind::InternalModule),
    LanguageKind::ts_js(TsJsKind::ClassDeclaration),
    LanguageKind::ts_js(TsJsKind::Constructor),
    LanguageKind::ts_js(TsJsKind::PropertyDeclaration),
    LanguageKind::ts_js(TsJsKind::PublicFieldDefinition),
];
