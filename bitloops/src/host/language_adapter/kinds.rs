#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum LanguageKind {
    CSharp(CSharpKind),
    Go(GoKind),
    Java(JavaKind),
    Php(PhpKind),
    Python(PythonKind),
    Rust(RustKind),
    TsJs(TsJsKind),
}

impl LanguageKind {
    pub(crate) const fn csharp(kind: CSharpKind) -> Self {
        Self::CSharp(kind)
    }

    pub(crate) const fn go(kind: GoKind) -> Self {
        Self::Go(kind)
    }

    pub(crate) const fn python(kind: PythonKind) -> Self {
        Self::Python(kind)
    }

    pub(crate) const fn php(kind: PhpKind) -> Self {
        Self::Php(kind)
    }

    pub(crate) const fn java(kind: JavaKind) -> Self {
        Self::Java(kind)
    }

    pub(crate) const fn rust(kind: RustKind) -> Self {
        Self::Rust(kind)
    }

    pub(crate) const fn ts_js(kind: TsJsKind) -> Self {
        Self::TsJs(kind)
    }

    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::CSharp(kind) => kind.as_str(),
            Self::Go(kind) => kind.as_str(),
            Self::Java(kind) => kind.as_str(),
            Self::Php(kind) => kind.as_str(),
            Self::Python(kind) => kind.as_str(),
            Self::Rust(kind) => kind.as_str(),
            Self::TsJs(kind) => kind.as_str(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum PhpKind {
    NamespaceDefinition,
    NamespaceUseDeclaration,
    ClassDeclaration,
    InterfaceDeclaration,
    TraitDeclaration,
    EnumDeclaration,
    FunctionDefinition,
    MethodDeclaration,
    PropertyDeclaration,
    ConstDeclaration,
}

impl PhpKind {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::NamespaceDefinition => "namespace_definition",
            Self::NamespaceUseDeclaration => "namespace_use_declaration",
            Self::ClassDeclaration => "class_declaration",
            Self::InterfaceDeclaration => "interface_declaration",
            Self::TraitDeclaration => "trait_declaration",
            Self::EnumDeclaration => "enum_declaration",
            Self::FunctionDefinition => "function_definition",
            Self::MethodDeclaration => "method_declaration",
            Self::PropertyDeclaration => "property_declaration",
            Self::ConstDeclaration => "const_declaration",
        }
    }

    pub(crate) fn from_tree_sitter_kind(kind: &str) -> Option<Self> {
        match kind {
            "namespace_definition" => Some(Self::NamespaceDefinition),
            "namespace_use_declaration" => Some(Self::NamespaceUseDeclaration),
            "class_declaration" => Some(Self::ClassDeclaration),
            "interface_declaration" => Some(Self::InterfaceDeclaration),
            "trait_declaration" => Some(Self::TraitDeclaration),
            "enum_declaration" => Some(Self::EnumDeclaration),
            "function_definition" => Some(Self::FunctionDefinition),
            "method_declaration" => Some(Self::MethodDeclaration),
            "property_declaration" => Some(Self::PropertyDeclaration),
            "const_declaration" => Some(Self::ConstDeclaration),
            _ => None,
        }
    }
}

impl std::fmt::Display for LanguageKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum CSharpKind {
    Class,
    Constructor,
    Method,
    Property,
    Field,
    Interface,
    Enum,
    Struct,
    Record,
    Delegate,
    Namespace,
    FileScopedNamespace,
    Using,
}

impl CSharpKind {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Class => "class_declaration",
            Self::Constructor => "constructor_declaration",
            Self::Method => "method_declaration",
            Self::Property => "property_declaration",
            Self::Field => "field_declaration",
            Self::Interface => "interface_declaration",
            Self::Enum => "enum_declaration",
            Self::Struct => "struct_declaration",
            Self::Record => "record_declaration",
            Self::Delegate => "delegate_declaration",
            Self::Namespace => "namespace_declaration",
            Self::FileScopedNamespace => "file_scoped_namespace_declaration",
            Self::Using => "using_directive",
        }
    }

    pub(crate) fn from_tree_sitter_kind(kind: &str) -> Option<Self> {
        match kind {
            "class_declaration" => Some(Self::Class),
            "constructor_declaration" => Some(Self::Constructor),
            "method_declaration" => Some(Self::Method),
            "property_declaration" => Some(Self::Property),
            "field_declaration" => Some(Self::Field),
            "interface_declaration" => Some(Self::Interface),
            "enum_declaration" => Some(Self::Enum),
            "struct_declaration" => Some(Self::Struct),
            "record_declaration" => Some(Self::Record),
            "delegate_declaration" => Some(Self::Delegate),
            "namespace_declaration" => Some(Self::Namespace),
            "file_scoped_namespace_declaration" => Some(Self::FileScopedNamespace),
            "using_directive" => Some(Self::Using),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum GoKind {
    FunctionDeclaration,
    MethodDeclaration,
    TypeSpec,
    TypeAlias,
    StructType,
    InterfaceType,
    ImportSpec,
    VarSpec,
    ConstSpec,
}

impl GoKind {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::FunctionDeclaration => "function_declaration",
            Self::MethodDeclaration => "method_declaration",
            Self::TypeSpec => "type_spec",
            Self::TypeAlias => "type_alias",
            Self::StructType => "struct_type",
            Self::InterfaceType => "interface_type",
            Self::ImportSpec => "import_spec",
            Self::VarSpec => "var_spec",
            Self::ConstSpec => "const_spec",
        }
    }

    pub(crate) fn from_tree_sitter_kind(kind: &str) -> Option<Self> {
        match kind {
            "function_declaration" => Some(Self::FunctionDeclaration),
            "method_declaration" => Some(Self::MethodDeclaration),
            "type_spec" => Some(Self::TypeSpec),
            "type_alias" => Some(Self::TypeAlias),
            "struct_type" => Some(Self::StructType),
            "interface_type" => Some(Self::InterfaceType),
            "import_spec" => Some(Self::ImportSpec),
            "var_spec" => Some(Self::VarSpec),
            "const_spec" => Some(Self::ConstSpec),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum JavaKind {
    Package,
    Import,
    Class,
    Interface,
    Enum,
    Constructor,
    Method,
    Field,
}

impl JavaKind {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Package => "package_declaration",
            Self::Import => "import_declaration",
            Self::Class => "class_declaration",
            Self::Interface => "interface_declaration",
            Self::Enum => "enum_declaration",
            Self::Constructor => "constructor_declaration",
            Self::Method => "method_declaration",
            Self::Field => "field_declaration",
        }
    }

    pub(crate) fn from_tree_sitter_kind(kind: &str) -> Option<Self> {
        match kind {
            "package_declaration" => Some(Self::Package),
            "import_declaration" => Some(Self::Import),
            "class_declaration" => Some(Self::Class),
            "interface_declaration" => Some(Self::Interface),
            "enum_declaration" => Some(Self::Enum),
            "constructor_declaration" => Some(Self::Constructor),
            "method_declaration" => Some(Self::Method),
            "field_declaration" => Some(Self::Field),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum PythonKind {
    Assignment,
    ClassDefinition,
    ImportFromStatement,
    FunctionDefinition,
    FutureImportStatement,
    ImportStatement,
}

impl PythonKind {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Assignment => "assignment",
            Self::ClassDefinition => "class_definition",
            Self::ImportFromStatement => "import_from_statement",
            Self::FunctionDefinition => "function_definition",
            Self::FutureImportStatement => "future_import_statement",
            Self::ImportStatement => "import_statement",
        }
    }

    pub(crate) fn from_tree_sitter_kind(kind: &str) -> Option<Self> {
        match kind {
            "assignment" => Some(Self::Assignment),
            "class_definition" => Some(Self::ClassDefinition),
            "import_from_statement" => Some(Self::ImportFromStatement),
            "function_definition" => Some(Self::FunctionDefinition),
            "future_import_statement" => Some(Self::FutureImportStatement),
            "import_statement" => Some(Self::ImportStatement),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum RustKind {
    ConstItem,
    EnumItem,
    FunctionItem,
    ImplItem,
    LetDeclaration,
    MacroDefinition,
    ModItem,
    StaticItem,
    StructItem,
    TraitItem,
    TypeItem,
    UseDeclaration,
}

impl RustKind {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::ConstItem => "const_item",
            Self::EnumItem => "enum_item",
            Self::FunctionItem => "function_item",
            Self::ImplItem => "impl_item",
            Self::LetDeclaration => "let_declaration",
            Self::MacroDefinition => "macro_definition",
            Self::ModItem => "mod_item",
            Self::StaticItem => "static_item",
            Self::StructItem => "struct_item",
            Self::TraitItem => "trait_item",
            Self::TypeItem => "type_item",
            Self::UseDeclaration => "use_declaration",
        }
    }

    pub(crate) fn from_tree_sitter_kind(kind: &str) -> Option<Self> {
        match kind {
            "const_item" => Some(Self::ConstItem),
            "enum_item" => Some(Self::EnumItem),
            "function_item" => Some(Self::FunctionItem),
            "impl_item" => Some(Self::ImplItem),
            "let_declaration" => Some(Self::LetDeclaration),
            "macro_definition" => Some(Self::MacroDefinition),
            "mod_item" => Some(Self::ModItem),
            "static_item" => Some(Self::StaticItem),
            "struct_item" => Some(Self::StructItem),
            "trait_item" => Some(Self::TraitItem),
            "type_item" => Some(Self::TypeItem),
            "use_declaration" => Some(Self::UseDeclaration),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum TsJsKind {
    ClassDeclaration,
    Constructor,
    EnumDeclaration,
    FunctionDeclaration,
    ImportStatement,
    InterfaceDeclaration,
    InternalModule,
    MethodDefinition,
    ModuleDeclaration,
    PropertyDeclaration,
    PublicFieldDefinition,
    TypeAliasDeclaration,
    VariableDeclarator,
}

impl TsJsKind {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::ClassDeclaration => "class_declaration",
            Self::Constructor => "constructor",
            Self::EnumDeclaration => "enum_declaration",
            Self::FunctionDeclaration => "function_declaration",
            Self::ImportStatement => "import_statement",
            Self::InterfaceDeclaration => "interface_declaration",
            Self::InternalModule => "internal_module",
            Self::MethodDefinition => "method_definition",
            Self::ModuleDeclaration => "module_declaration",
            Self::PropertyDeclaration => "property_declaration",
            Self::PublicFieldDefinition => "public_field_definition",
            Self::TypeAliasDeclaration => "type_alias_declaration",
            Self::VariableDeclarator => "variable_declarator",
        }
    }

    pub(crate) fn from_tree_sitter_kind(kind: &str) -> Option<Self> {
        match kind {
            "class_declaration" => Some(Self::ClassDeclaration),
            "constructor" => Some(Self::Constructor),
            "enum_declaration" => Some(Self::EnumDeclaration),
            "function_declaration" => Some(Self::FunctionDeclaration),
            "import_statement" => Some(Self::ImportStatement),
            "interface_declaration" => Some(Self::InterfaceDeclaration),
            "internal_module" => Some(Self::InternalModule),
            "method_definition" => Some(Self::MethodDefinition),
            "module_declaration" => Some(Self::ModuleDeclaration),
            "property_declaration" => Some(Self::PropertyDeclaration),
            "public_field_definition" => Some(Self::PublicFieldDefinition),
            "type_alias_declaration" => Some(Self::TypeAliasDeclaration),
            "variable_declarator" => Some(Self::VariableDeclarator),
            _ => None,
        }
    }
}

impl From<CSharpKind> for LanguageKind {
    fn from(value: CSharpKind) -> Self {
        Self::CSharp(value)
    }
}

impl From<GoKind> for LanguageKind {
    fn from(value: GoKind) -> Self {
        Self::Go(value)
    }
}

impl From<JavaKind> for LanguageKind {
    fn from(value: JavaKind) -> Self {
        Self::Java(value)
    }
}

impl From<PythonKind> for LanguageKind {
    fn from(value: PythonKind) -> Self {
        Self::Python(value)
    }
}

impl From<PhpKind> for LanguageKind {
    fn from(value: PhpKind) -> Self {
        Self::Php(value)
    }
}

impl From<RustKind> for LanguageKind {
    fn from(value: RustKind) -> Self {
        Self::Rust(value)
    }
}

impl From<TsJsKind> for LanguageKind {
    fn from(value: TsJsKind) -> Self {
        Self::TsJs(value)
    }
}

impl TryFrom<&str> for LanguageKind {
    type Error = ();

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        let csharp = CSharpKind::from_tree_sitter_kind(value).map(Self::CSharp);
        let rust = RustKind::from_tree_sitter_kind(value).map(Self::Rust);
        let ts_js = TsJsKind::from_tree_sitter_kind(value).map(Self::TsJs);
        let php = PhpKind::from_tree_sitter_kind(value).map(Self::Php);
        let python = PythonKind::from_tree_sitter_kind(value).map(Self::Python);
        let go = GoKind::from_tree_sitter_kind(value).map(Self::Go);
        let java = JavaKind::from_tree_sitter_kind(value).map(Self::Java);

        match [rust, ts_js, php, python, go, java, csharp]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .as_slice()
        {
            [kind] => Ok(*kind),
            _ => Err(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        CSharpKind, GoKind, JavaKind, LanguageKind, PhpKind, PythonKind, RustKind, TsJsKind,
    };

    #[test]
    fn per_language_kind_parsers_round_trip() {
        let csharp_cases = [
            CSharpKind::Class,
            CSharpKind::Constructor,
            CSharpKind::Method,
            CSharpKind::Property,
            CSharpKind::Field,
            CSharpKind::Interface,
            CSharpKind::Enum,
            CSharpKind::Struct,
            CSharpKind::Record,
            CSharpKind::Delegate,
            CSharpKind::Namespace,
            CSharpKind::FileScopedNamespace,
            CSharpKind::Using,
        ];
        for kind in csharp_cases {
            assert_eq!(CSharpKind::from_tree_sitter_kind(kind.as_str()), Some(kind));
        }

        let go_cases = [
            GoKind::FunctionDeclaration,
            GoKind::MethodDeclaration,
            GoKind::TypeSpec,
            GoKind::TypeAlias,
            GoKind::StructType,
            GoKind::InterfaceType,
            GoKind::ImportSpec,
            GoKind::VarSpec,
            GoKind::ConstSpec,
        ];
        for kind in go_cases {
            assert_eq!(GoKind::from_tree_sitter_kind(kind.as_str()), Some(kind));
        }

        let python_cases = [
            PythonKind::Assignment,
            PythonKind::ClassDefinition,
            PythonKind::ImportFromStatement,
            PythonKind::FunctionDefinition,
            PythonKind::FutureImportStatement,
            PythonKind::ImportStatement,
        ];
        for kind in python_cases {
            assert_eq!(PythonKind::from_tree_sitter_kind(kind.as_str()), Some(kind));
        }

        let php_cases = [
            PhpKind::NamespaceDefinition,
            PhpKind::NamespaceUseDeclaration,
            PhpKind::ClassDeclaration,
            PhpKind::InterfaceDeclaration,
            PhpKind::TraitDeclaration,
            PhpKind::EnumDeclaration,
            PhpKind::FunctionDefinition,
            PhpKind::MethodDeclaration,
            PhpKind::PropertyDeclaration,
            PhpKind::ConstDeclaration,
        ];
        for kind in php_cases {
            assert_eq!(PhpKind::from_tree_sitter_kind(kind.as_str()), Some(kind));
        }

        let java_cases = [
            JavaKind::Package,
            JavaKind::Import,
            JavaKind::Class,
            JavaKind::Interface,
            JavaKind::Enum,
            JavaKind::Constructor,
            JavaKind::Method,
            JavaKind::Field,
        ];
        for kind in java_cases {
            assert_eq!(JavaKind::from_tree_sitter_kind(kind.as_str()), Some(kind));
        }

        let rust_cases = [
            RustKind::ConstItem,
            RustKind::EnumItem,
            RustKind::FunctionItem,
            RustKind::ImplItem,
            RustKind::LetDeclaration,
            RustKind::MacroDefinition,
            RustKind::ModItem,
            RustKind::StaticItem,
            RustKind::StructItem,
            RustKind::TraitItem,
            RustKind::TypeItem,
            RustKind::UseDeclaration,
        ];
        for kind in rust_cases {
            assert_eq!(RustKind::from_tree_sitter_kind(kind.as_str()), Some(kind));
        }

        let ts_js_cases = [
            TsJsKind::ClassDeclaration,
            TsJsKind::Constructor,
            TsJsKind::EnumDeclaration,
            TsJsKind::FunctionDeclaration,
            TsJsKind::ImportStatement,
            TsJsKind::InterfaceDeclaration,
            TsJsKind::InternalModule,
            TsJsKind::MethodDefinition,
            TsJsKind::ModuleDeclaration,
            TsJsKind::PropertyDeclaration,
            TsJsKind::PublicFieldDefinition,
            TsJsKind::TypeAliasDeclaration,
            TsJsKind::VariableDeclarator,
        ];
        for kind in ts_js_cases {
            assert_eq!(TsJsKind::from_tree_sitter_kind(kind.as_str()), Some(kind));
        }
    }

    #[test]
    fn top_level_parser_rejects_ambiguous_or_unknown_kinds() {
        assert!(LanguageKind::try_from("function_declaration").is_err());
        assert!(LanguageKind::try_from("import_statement").is_err());
        assert!(LanguageKind::try_from("class_declaration").is_err());
        assert!(LanguageKind::try_from("interface_declaration").is_err());
        assert!(LanguageKind::try_from("enum_declaration").is_err());
        assert!(LanguageKind::try_from("method_declaration").is_err());
        assert!(LanguageKind::try_from("totally_unknown_kind").is_err());

        assert_eq!(
            LanguageKind::try_from("impl_item").ok(),
            Some(LanguageKind::rust(RustKind::ImplItem))
        );
        assert_eq!(
            LanguageKind::try_from("assignment").ok(),
            Some(LanguageKind::python(PythonKind::Assignment))
        );
        assert_eq!(
            LanguageKind::try_from("package_declaration").ok(),
            Some(LanguageKind::java(JavaKind::Package))
        );

        assert_eq!(
            TsJsKind::from_tree_sitter_kind("class_declaration"),
            Some(TsJsKind::ClassDeclaration)
        );
        assert_eq!(
            JavaKind::from_tree_sitter_kind("class_declaration"),
            Some(JavaKind::Class)
        );
        assert_eq!(
            TsJsKind::from_tree_sitter_kind("interface_declaration"),
            Some(TsJsKind::InterfaceDeclaration)
        );
        assert_eq!(
            JavaKind::from_tree_sitter_kind("interface_declaration"),
            Some(JavaKind::Interface)
        );
        assert_eq!(
            TsJsKind::from_tree_sitter_kind("enum_declaration"),
            Some(TsJsKind::EnumDeclaration)
        );
        assert_eq!(
            JavaKind::from_tree_sitter_kind("enum_declaration"),
            Some(JavaKind::Enum)
        );
        assert_eq!(
            GoKind::from_tree_sitter_kind("method_declaration"),
            Some(GoKind::MethodDeclaration)
        );
        assert_eq!(
            JavaKind::from_tree_sitter_kind("method_declaration"),
            Some(JavaKind::Method)
        );
    }
}
