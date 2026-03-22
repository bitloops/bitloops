fn js_ts_canonical_projection(language_kind: &str) -> Option<CanonicalKindProjection> {
    match language_kind {
        "function_declaration" => Some(CanonicalKindProjection::Function),
        "method_definition" => Some(CanonicalKindProjection::Method),
        "interface_declaration" => Some(CanonicalKindProjection::Interface),
        "type_alias_declaration" => Some(CanonicalKindProjection::Type),
        "enum_declaration" => Some(CanonicalKindProjection::Enum),
        "variable_declarator" => Some(CanonicalKindProjection::Variable),
        "import_statement" => Some(CanonicalKindProjection::Import),
        "module_declaration" | "internal_module" => Some(CanonicalKindProjection::Module),
        _ => None,
    }
}

fn js_ts_canonical_kind(language_kind: &str) -> Option<&'static str> {
    js_ts_canonical_projection(language_kind).map(CanonicalKindProjection::as_str)
}

fn js_ts_supports_language_kind(language_kind: &str) -> bool {
    matches!(
        language_kind,
        "function_declaration"
            | "method_definition"
            | "interface_declaration"
            | "type_alias_declaration"
            | "enum_declaration"
            | "variable_declarator"
            | "import_statement"
            | "module_declaration"
            | "internal_module"
            | "class_declaration"
            | "constructor"
            | "property_declaration"
            | "public_field_definition"
    )
}

fn rust_canonical_projection(
    language_kind: &str,
    inside_impl: bool,
) -> Option<CanonicalKindProjection> {
    match language_kind {
        "function_item" => Some(if inside_impl {
            CanonicalKindProjection::Method
        } else {
            CanonicalKindProjection::Function
        }),
        "trait_item" => Some(CanonicalKindProjection::Interface),
        "type_item" => Some(CanonicalKindProjection::Type),
        "enum_item" => Some(CanonicalKindProjection::Enum),
        "use_declaration" => Some(CanonicalKindProjection::Import),
        "mod_item" => Some(CanonicalKindProjection::Module),
        "let_declaration" => Some(CanonicalKindProjection::Variable),
        _ => None,
    }
}

fn rust_canonical_kind(language_kind: &str, inside_impl: bool) -> Option<&'static str> {
    rust_canonical_projection(language_kind, inside_impl).map(CanonicalKindProjection::as_str)
}

fn rust_supports_language_kind(language_kind: &str) -> bool {
    matches!(
        language_kind,
        "function_item"
            | "trait_item"
            | "type_item"
            | "enum_item"
            | "use_declaration"
            | "mod_item"
            | "let_declaration"
            | "impl_item"
            | "struct_item"
            | "const_item"
            | "static_item"
            | "macro_definition"
    )
}
