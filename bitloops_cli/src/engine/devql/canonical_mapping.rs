const LANGUAGE_ONLY_CANONICAL_KIND: &str = "language_only";

fn js_ts_canonical_kind(language_kind: &str) -> Option<&'static str> {
    match language_kind {
        "function_declaration" => Some("function"),
        "method_definition" => Some("method"),
        "interface_declaration" => Some("interface"),
        "type_alias_declaration" => Some("type"),
        "enum_declaration" => Some("enum"),
        "variable_declarator" => Some("variable"),
        "import_statement" => Some("import"),
        "module_declaration" | "internal_module" => Some("module"),
        "class_declaration" | "constructor" | "property_declaration" => {
            Some(LANGUAGE_ONLY_CANONICAL_KIND)
        }
        _ => None,
    }
}

fn rust_canonical_kind(language_kind: &str, inside_impl: bool) -> Option<&'static str> {
    match language_kind {
        "function_item" => Some(if inside_impl { "method" } else { "function" }),
        "trait_item" => Some("interface"),
        "type_item" => Some("type"),
        "enum_item" => Some("enum"),
        "use_declaration" => Some("import"),
        "mod_item" => Some("module"),
        "let_declaration" => Some("variable"),
        "impl_item" | "struct_item" | "const_item" | "static_item" | "macro_definition" => {
            Some(LANGUAGE_ONLY_CANONICAL_KIND)
        }
        _ => None,
    }
}
