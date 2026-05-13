use crate::host::devql::CanonicalKindProjection;
use crate::host::language_adapter::{CanonicalMapping, LanguageKind, MappingCondition, PhpKind};

pub(crate) static PHP_CANONICAL_MAPPINGS: &[CanonicalMapping] = &[
    CanonicalMapping {
        language_kind: LanguageKind::php(PhpKind::NamespaceDefinition),
        projection: CanonicalKindProjection::Module,
        condition: MappingCondition::Always,
    },
    CanonicalMapping {
        language_kind: LanguageKind::php(PhpKind::NamespaceUseDeclaration),
        projection: CanonicalKindProjection::Import,
        condition: MappingCondition::Always,
    },
    CanonicalMapping {
        language_kind: LanguageKind::php(PhpKind::ClassDeclaration),
        projection: CanonicalKindProjection::Type,
        condition: MappingCondition::Always,
    },
    CanonicalMapping {
        language_kind: LanguageKind::php(PhpKind::InterfaceDeclaration),
        projection: CanonicalKindProjection::Type,
        condition: MappingCondition::Always,
    },
    CanonicalMapping {
        language_kind: LanguageKind::php(PhpKind::TraitDeclaration),
        projection: CanonicalKindProjection::Type,
        condition: MappingCondition::Always,
    },
    CanonicalMapping {
        language_kind: LanguageKind::php(PhpKind::EnumDeclaration),
        projection: CanonicalKindProjection::Type,
        condition: MappingCondition::Always,
    },
    CanonicalMapping {
        language_kind: LanguageKind::php(PhpKind::FunctionDefinition),
        projection: CanonicalKindProjection::Function,
        condition: MappingCondition::Always,
    },
    CanonicalMapping {
        language_kind: LanguageKind::php(PhpKind::MethodDeclaration),
        projection: CanonicalKindProjection::Method,
        condition: MappingCondition::Always,
    },
    CanonicalMapping {
        language_kind: LanguageKind::php(PhpKind::PropertyDeclaration),
        projection: CanonicalKindProjection::Variable,
        condition: MappingCondition::Always,
    },
    CanonicalMapping {
        language_kind: LanguageKind::php(PhpKind::ConstDeclaration),
        projection: CanonicalKindProjection::Variable,
        condition: MappingCondition::Always,
    },
];

pub(crate) static PHP_SUPPORTED_LANGUAGE_KINDS: &[LanguageKind] = &[
    LanguageKind::php(PhpKind::NamespaceDefinition),
    LanguageKind::php(PhpKind::NamespaceUseDeclaration),
    LanguageKind::php(PhpKind::ClassDeclaration),
    LanguageKind::php(PhpKind::InterfaceDeclaration),
    LanguageKind::php(PhpKind::TraitDeclaration),
    LanguageKind::php(PhpKind::EnumDeclaration),
    LanguageKind::php(PhpKind::FunctionDefinition),
    LanguageKind::php(PhpKind::MethodDeclaration),
    LanguageKind::php(PhpKind::PropertyDeclaration),
    LanguageKind::php(PhpKind::ConstDeclaration),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn php_canonical_mappings_reference_only_supported_kinds() {
        for mapping in PHP_CANONICAL_MAPPINGS {
            assert!(PHP_SUPPORTED_LANGUAGE_KINDS.contains(&mapping.language_kind));
        }
    }
}
