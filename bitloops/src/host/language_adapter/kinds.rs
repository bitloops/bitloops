macro_rules! define_language_kinds {
    ($name:ident { $($kind:ident => $value:literal,)* }) => {
        pub(crate) struct $name;

        #[allow(non_upper_case_globals)]
        impl $name {
            $(pub(crate) const $kind: &'static str = $value;)*
        }
    };
}

define_language_kinds!(GolangKinds {
    FunctionDeclaration => "function_declaration",
    MethodDeclaration => "method_declaration",
    TypeSpec => "type_spec",
    TypeAlias => "type_alias",
    StructType => "struct_type",
    InterfaceType => "interface_type",
    ImportSpec => "import_spec",
    VarSpec => "var_spec",
    ConstSpec => "const_spec",
});

define_language_kinds!(TsJsKinds {
    ClassDeclaration => "class_declaration",
    Constructor => "constructor",
    EnumDeclaration => "enum_declaration",
    FunctionDeclaration => "function_declaration",
    ImportStatement => "import_statement",
    InterfaceDeclaration => "interface_declaration",
    InternalModule => "internal_module",
    MethodDefinition => "method_definition",
    ModuleDeclaration => "module_declaration",
    PropertyDeclaration => "property_declaration",
    PublicFieldDefinition => "public_field_definition",
    TypeAliasDeclaration => "type_alias_declaration",
    VariableDeclarator => "variable_declarator",
});

define_language_kinds!(RustKinds {
    ConstItem => "const_item",
    EnumItem => "enum_item",
    FunctionItem => "function_item",
    ImplItem => "impl_item",
    LetDeclaration => "let_declaration",
    MacroDefinition => "macro_definition",
    ModItem => "mod_item",
    StaticItem => "static_item",
    StructItem => "struct_item",
    TraitItem => "trait_item",
    TypeItem => "type_item",
    UseDeclaration => "use_declaration",
});

define_language_kinds!(PythonKinds {
    Assignment => "assignment",
    ClassDefinition => "class_definition",
    ImportFromStatement => "import_from_statement",
    FunctionDefinition => "function_definition",
    FutureImportStatement => "future_import_statement",
    ImportStatement => "import_statement",
});
