use crate::host::devql::CanonicalKindProjection;
use crate::host::language_adapter::{CanonicalMapping, CppKind, LanguageKind, MappingCondition};

pub(crate) static CPP_CANONICAL_MAPPINGS: &[CanonicalMapping] = &[
    CanonicalMapping {
        language_kind: LanguageKind::cpp(CppKind::NamespaceDefinition),
        projection: CanonicalKindProjection::Namespace,
        condition: MappingCondition::Always,
    },
    CanonicalMapping {
        language_kind: LanguageKind::cpp(CppKind::UsingDeclaration),
        projection: CanonicalKindProjection::Import,
        condition: MappingCondition::Always,
    },
    CanonicalMapping {
        language_kind: LanguageKind::cpp(CppKind::TypeDefinition),
        projection: CanonicalKindProjection::Type,
        condition: MappingCondition::Always,
    },
    CanonicalMapping {
        language_kind: LanguageKind::cpp(CppKind::ClassSpecifier),
        projection: CanonicalKindProjection::Type,
        condition: MappingCondition::Always,
    },
    CanonicalMapping {
        language_kind: LanguageKind::cpp(CppKind::StructSpecifier),
        projection: CanonicalKindProjection::Type,
        condition: MappingCondition::Always,
    },
    CanonicalMapping {
        language_kind: LanguageKind::cpp(CppKind::EnumSpecifier),
        projection: CanonicalKindProjection::Enum,
        condition: MappingCondition::Always,
    },
    CanonicalMapping {
        language_kind: LanguageKind::cpp(CppKind::FunctionDefinition),
        projection: CanonicalKindProjection::Function,
        condition: MappingCondition::Always,
    },
    CanonicalMapping {
        language_kind: LanguageKind::cpp(CppKind::FunctionDefinition),
        projection: CanonicalKindProjection::Method,
        condition: MappingCondition::WhenInsideParent,
    },
    CanonicalMapping {
        language_kind: LanguageKind::cpp(CppKind::FieldDeclaration),
        projection: CanonicalKindProjection::Variable,
        condition: MappingCondition::Always,
    },
    CanonicalMapping {
        language_kind: LanguageKind::cpp(CppKind::PreprocInclude),
        projection: CanonicalKindProjection::Import,
        condition: MappingCondition::Always,
    },
];

pub(crate) static CPP_SUPPORTED_LANGUAGE_KINDS: &[LanguageKind] = &[
    LanguageKind::cpp(CppKind::NamespaceDefinition),
    LanguageKind::cpp(CppKind::UsingDeclaration),
    LanguageKind::cpp(CppKind::TypeDefinition),
    LanguageKind::cpp(CppKind::ClassSpecifier),
    LanguageKind::cpp(CppKind::StructSpecifier),
    LanguageKind::cpp(CppKind::EnumSpecifier),
    LanguageKind::cpp(CppKind::FunctionDefinition),
    LanguageKind::cpp(CppKind::FieldDeclaration),
    LanguageKind::cpp(CppKind::TemplateDeclaration),
    LanguageKind::cpp(CppKind::PreprocInclude),
];

#[cfg(test)]
mod tests {
    use super::{CPP_CANONICAL_MAPPINGS, CPP_SUPPORTED_LANGUAGE_KINDS};

    #[test]
    fn cpp_canonical_mapping_uses_supported_language_kinds() {
        for mapping in CPP_CANONICAL_MAPPINGS {
            assert!(CPP_SUPPORTED_LANGUAGE_KINDS.contains(&mapping.language_kind));
        }
    }
}
