use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LanguageEntryPointFile {
    pub path: String,
    pub language: String,
    pub content_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LanguageEntryPointArtefact {
    pub artefact_id: String,
    pub symbol_id: String,
    pub path: String,
    pub name: String,
    pub canonical_kind: Option<String>,
    pub language_kind: Option<String>,
    pub symbol_fqn: Option<String>,
    pub signature: Option<String>,
    pub modifiers: Vec<String>,
    pub start_line: i64,
    pub end_line: i64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LanguageEntryPointCandidate {
    pub path: String,
    pub artefact_id: Option<String>,
    pub symbol_id: Option<String>,
    pub name: String,
    pub entry_kind: String,
    pub confidence: f64,
    pub reason: String,
    pub evidence: Vec<String>,
}

pub trait LanguageEntryPointSupport: Send + Sync {
    fn detect_entry_points(
        &self,
        file: &LanguageEntryPointFile,
        artefacts: &[LanguageEntryPointArtefact],
    ) -> Vec<LanguageEntryPointCandidate>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuiltinEntryPointLanguage {
    CSharp,
    Go,
    Java,
    Php,
    Python,
    Rust,
    TsJs,
}

pub struct BuiltinLanguageEntryPointSupport {
    language: BuiltinEntryPointLanguage,
}

impl BuiltinLanguageEntryPointSupport {
    pub const fn new(language: BuiltinEntryPointLanguage) -> Self {
        Self { language }
    }
}

impl LanguageEntryPointSupport for BuiltinLanguageEntryPointSupport {
    fn detect_entry_points(
        &self,
        file: &LanguageEntryPointFile,
        artefacts: &[LanguageEntryPointArtefact],
    ) -> Vec<LanguageEntryPointCandidate> {
        detect_builtin_entry_points(self.language, file, artefacts)
    }
}

pub fn detect_builtin_entry_points(
    language: BuiltinEntryPointLanguage,
    file: &LanguageEntryPointFile,
    artefacts: &[LanguageEntryPointArtefact],
) -> Vec<LanguageEntryPointCandidate> {
    let mut candidates = Vec::new();
    let basename = basename(&file.path);
    let path = file.path.as_str();

    for artefact in artefacts {
        match language {
            BuiltinEntryPointLanguage::Rust => {
                if is_callable(artefact)
                    && artefact.name == "main"
                    && (path.ends_with("src/main.rs")
                        || path.contains("/bin/")
                        || path.starts_with("bin/")
                        || artefact
                            .symbol_fqn
                            .as_deref()
                            .is_some_and(|fqn| fqn.ends_with("::main")))
                {
                    candidates.push(candidate(
                        file,
                        artefact,
                        "rust_main",
                        0.98,
                        "Rust `main` function or binary root",
                    ));
                }
                if is_callable(artefact) && artefact.name == "run_from_args" {
                    candidates.push(candidate(
                        file,
                        artefact,
                        "rust_cli_dispatch",
                        0.88,
                        "Rust CLI argument dispatch function",
                    ));
                }
                if is_callable(artefact)
                    && matches!(artefact.name.as_str(), "parse_from" | "parse_args")
                    && path.ends_with("src/cli.rs")
                {
                    candidates.push(candidate(
                        file,
                        artefact,
                        "rust_cli_parser",
                        0.72,
                        "Rust CLI parser hook",
                    ));
                }
            }
            BuiltinEntryPointLanguage::Go => {
                if is_callable(artefact) && basename == "main.go" && artefact.name == "main" {
                    candidates.push(candidate(
                        file,
                        artefact,
                        "go_main",
                        0.98,
                        "Go `main.go` main function",
                    ));
                }
                if is_callable(artefact)
                    && is_handler_name(&artefact.name)
                    && path_contains_any(path, &["handler", "handlers", "http", "server"])
                {
                    candidates.push(candidate(
                        file,
                        artefact,
                        "go_http_handler",
                        0.70,
                        "Go HTTP/message handler-shaped function",
                    ));
                }
            }
            BuiltinEntryPointLanguage::Python => {
                if is_callable(artefact)
                    && matches!(
                        artefact.name.as_str(),
                        "main" | "handler" | "lambda_handler" | "application" | "app"
                    )
                    && matches!(
                        basename.as_str(),
                        "main.py" | "app.py" | "wsgi.py" | "asgi.py" | "__main__.py"
                    )
                {
                    candidates.push(candidate(
                        file,
                        artefact,
                        "python_runtime",
                        0.86,
                        "Python runtime-shaped function in a conventional entry file",
                    ));
                }
                if is_callable_or_value(artefact)
                    && matches!(artefact.name.as_str(), "app" | "application" | "create_app")
                    && (matches!(
                        basename.as_str(),
                        "app.py" | "main.py" | "asgi.py" | "wsgi.py"
                    ) || path_contains_any(path, &["api", "server", "web"]))
                {
                    candidates.push(candidate(
                        file,
                        artefact,
                        "python_web_app",
                        0.82,
                        "Python web application object or factory",
                    ));
                }
                if is_callable(artefact)
                    && is_handler_name(&artefact.name)
                    && path_contains_any(path, &["handler", "handlers", "lambda", "workers"])
                {
                    candidates.push(candidate(
                        file,
                        artefact,
                        "python_handler",
                        0.76,
                        "Python handler-shaped function",
                    ));
                }
            }
            BuiltinEntryPointLanguage::Php => {
                if is_callable(artefact)
                    && matches!(
                        artefact.name.as_str(),
                        "main" | "handle" | "__invoke" | "run" | "boot"
                    )
                    && matches!(
                        basename.as_str(),
                        "index.php" | "artisan" | "console.php" | "server.php"
                    )
                {
                    candidates.push(candidate(
                        file,
                        artefact,
                        "php_runtime",
                        0.84,
                        "PHP runtime-shaped function in a conventional entry file",
                    ));
                }
            }
            BuiltinEntryPointLanguage::TsJs => {
                if is_callable_or_value(artefact)
                    && (matches!(
                        artefact.name.as_str(),
                        "main" | "handler" | "GET" | "POST" | "PUT" | "PATCH" | "DELETE"
                    ) || artefact
                        .modifiers
                        .iter()
                        .any(|modifier| modifier == "export"))
                    && matches!(
                        basename.as_str(),
                        "index.ts"
                            | "index.tsx"
                            | "index.js"
                            | "index.jsx"
                            | "server.ts"
                            | "server.js"
                            | "app.ts"
                            | "app.js"
                    )
                {
                    candidates.push(candidate(
                        file,
                        artefact,
                        "js_ts_handler",
                        0.82,
                        "TypeScript/JavaScript exported or conventional handler",
                    ));
                }
                if is_callable_or_value(artefact)
                    && is_next_route_file(path, &basename)
                    && is_exported(artefact)
                    && is_http_method_name(&artefact.name)
                {
                    candidates.push(candidate(
                        file,
                        artefact,
                        "next_route_handler",
                        0.90,
                        "Next.js route handler export",
                    ));
                }
                if is_callable_or_value(artefact)
                    && is_exported(artefact)
                    && is_handler_name(&artefact.name)
                {
                    candidates.push(candidate(
                        file,
                        artefact,
                        "js_ts_exported_handler",
                        0.76,
                        "Exported TypeScript/JavaScript handler",
                    ));
                }
                if is_callable(artefact)
                    && matches!(
                        artefact.name.as_str(),
                        "bootstrap" | "createServer" | "startServer" | "listen"
                    )
                    && path_contains_any(path, &["server", "app", "api", "cmd"])
                {
                    candidates.push(candidate(
                        file,
                        artefact,
                        "js_ts_server_bootstrap",
                        0.74,
                        "TypeScript/JavaScript server bootstrap function",
                    ));
                }
            }
            BuiltinEntryPointLanguage::Java => {
                if is_callable(artefact) && artefact.name == "main" {
                    candidates.push(candidate(
                        file,
                        artefact,
                        "java_main",
                        0.92,
                        "Java `main` method",
                    ));
                } else if matches!(basename.as_str(), "App.java" | "Application.java")
                    && is_type(artefact)
                    && matches!(artefact.name.as_str(), "App" | "Application" | "Main")
                {
                    candidates.push(candidate(
                        file,
                        artefact,
                        "java_application",
                        0.70,
                        "Java conventional application type",
                    ));
                }
                if is_type(artefact)
                    && artefact.name.ends_with("Application")
                    && path_contains_any(path, &["src/main/java", "src/main/kotlin"])
                {
                    candidates.push(candidate(
                        file,
                        artefact,
                        "java_spring_application",
                        0.80,
                        "Java application bootstrap type",
                    ));
                }
            }
            BuiltinEntryPointLanguage::CSharp => {
                if is_callable(artefact)
                    && matches!(artefact.name.as_str(), "Main" | "Program")
                    && basename == "Program.cs"
                {
                    candidates.push(candidate(
                        file,
                        artefact,
                        "dotnet_program",
                        0.92,
                        ".NET Program entry point",
                    ));
                }
                if is_type(artefact)
                    && matches!(artefact.name.as_str(), "Program" | "Startup")
                    && basename == "Program.cs"
                {
                    candidates.push(candidate(
                        file,
                        artefact,
                        "dotnet_application",
                        0.78,
                        ".NET application bootstrap type",
                    ));
                }
            }
        }
    }

    if candidates.is_empty() && file_level_entry(language, path, &basename) {
        candidates.push(LanguageEntryPointCandidate {
            path: file.path.clone(),
            artefact_id: None,
            symbol_id: None,
            name: basename,
            entry_kind: "file_entry".to_string(),
            confidence: 0.55,
            reason: "Conventional entry-point file".to_string(),
            evidence: vec![file.path.clone()],
        });
    }

    candidates
}

fn candidate(
    file: &LanguageEntryPointFile,
    artefact: &LanguageEntryPointArtefact,
    entry_kind: &str,
    confidence: f64,
    reason: &str,
) -> LanguageEntryPointCandidate {
    LanguageEntryPointCandidate {
        path: file.path.clone(),
        artefact_id: Some(artefact.artefact_id.clone()),
        symbol_id: Some(artefact.symbol_id.clone()),
        name: artefact.name.clone(),
        entry_kind: entry_kind.to_string(),
        confidence,
        reason: reason.to_string(),
        evidence: vec![format!(
            "{}:{}-{}",
            file.path, artefact.start_line, artefact.end_line
        )],
    }
}

fn basename(path: &str) -> String {
    Path::new(path)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(path)
        .to_string()
}

fn is_callable(artefact: &LanguageEntryPointArtefact) -> bool {
    matches!(
        artefact.canonical_kind.as_deref(),
        Some("function" | "method" | "callable")
    )
}

fn is_callable_or_value(artefact: &LanguageEntryPointArtefact) -> bool {
    is_callable(artefact)
        || matches!(
            artefact.canonical_kind.as_deref(),
            Some("value" | "variable" | "member")
        )
}

fn is_type(artefact: &LanguageEntryPointArtefact) -> bool {
    matches!(
        artefact.canonical_kind.as_deref(),
        Some("type" | "class" | "interface" | "enum")
    )
}

fn is_exported(artefact: &LanguageEntryPointArtefact) -> bool {
    artefact
        .modifiers
        .iter()
        .any(|modifier| modifier == "export" || modifier == "pub" || modifier == "public")
}

fn is_handler_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower == "handler"
        || lower.ends_with("handler")
        || lower.starts_with("handle")
        || lower.ends_with("_handler")
}

fn is_http_method_name(name: &str) -> bool {
    matches!(
        name,
        "GET" | "POST" | "PUT" | "PATCH" | "DELETE" | "HEAD" | "OPTIONS"
    )
}

fn is_next_route_file(path: &str, basename: &str) -> bool {
    matches!(
        basename,
        "route.ts" | "route.tsx" | "route.js" | "route.jsx"
    ) && (path.contains("/app/") || path.starts_with("app/") || path.contains("/pages/api/"))
}

fn path_contains_any(path: &str, needles: &[&str]) -> bool {
    let lower = path.to_ascii_lowercase();
    needles.iter().any(|needle| lower.contains(needle))
}

fn file_level_entry(language: BuiltinEntryPointLanguage, path: &str, basename: &str) -> bool {
    match language {
        BuiltinEntryPointLanguage::Rust => {
            path.ends_with("src/main.rs") || path.contains("/bin/") || path.starts_with("bin/")
        }
        BuiltinEntryPointLanguage::Go => basename == "main.go",
        BuiltinEntryPointLanguage::Python => {
            matches!(
                basename,
                "main.py" | "app.py" | "wsgi.py" | "asgi.py" | "__main__.py"
            )
        }
        BuiltinEntryPointLanguage::Php => matches!(
            basename,
            "index.php" | "artisan" | "console.php" | "server.php"
        ),
        BuiltinEntryPointLanguage::TsJs => matches!(
            basename,
            "index.ts"
                | "index.tsx"
                | "index.js"
                | "index.jsx"
                | "server.ts"
                | "server.js"
                | "app.ts"
                | "app.js"
        ),
        BuiltinEntryPointLanguage::Java => matches!(basename, "App.java" | "Application.java"),
        BuiltinEntryPointLanguage::CSharp => basename == "Program.cs",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(path: &str, language: &str) -> LanguageEntryPointFile {
        LanguageEntryPointFile {
            path: path.to_string(),
            language: language.to_string(),
            content_id: "content".to_string(),
        }
    }

    fn artefact(name: &str, kind: &str, path: &str) -> LanguageEntryPointArtefact {
        LanguageEntryPointArtefact {
            artefact_id: format!("{name}-artefact"),
            symbol_id: format!("{name}-symbol"),
            path: path.to_string(),
            name: name.to_string(),
            canonical_kind: Some(kind.to_string()),
            language_kind: Some("function_item".to_string()),
            symbol_fqn: Some(format!("{path}::{name}")),
            signature: None,
            modifiers: Vec::new(),
            start_line: 1,
            end_line: 3,
        }
    }

    fn exported_artefact(name: &str, kind: &str, path: &str) -> LanguageEntryPointArtefact {
        let mut artefact = artefact(name, kind, path);
        artefact.modifiers = vec!["export".to_string()];
        artefact
    }

    #[test]
    fn detects_rust_main_entry_point() {
        let file = file("src/main.rs", "rust");
        let result = detect_builtin_entry_points(
            BuiltinEntryPointLanguage::Rust,
            &file,
            &[artefact("main", "function", "src/main.rs")],
        );

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].entry_kind, "rust_main");
        assert_eq!(result[0].confidence, 0.98);
    }

    #[test]
    fn detects_go_main_entry_point() {
        let file = file("cmd/server/main.go", "go");
        let result = detect_builtin_entry_points(
            BuiltinEntryPointLanguage::Go,
            &file,
            &[artefact("main", "function", "cmd/server/main.go")],
        );

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].entry_kind, "go_main");
    }

    #[test]
    fn detects_python_runtime_entry_point() {
        let file = file("service/app.py", "python");
        let result = detect_builtin_entry_points(
            BuiltinEntryPointLanguage::Python,
            &file,
            &[artefact("handler", "function", "service/app.py")],
        );

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].entry_kind, "python_runtime");
    }

    #[test]
    fn detects_ts_js_exported_handler_entry_point() {
        let file = file("packages/api/src/server.ts", "typescript");
        let result = detect_builtin_entry_points(
            BuiltinEntryPointLanguage::TsJs,
            &file,
            &[exported_artefact(
                "handle",
                "function",
                "packages/api/src/server.ts",
            )],
        );

        assert!(
            result
                .iter()
                .any(|candidate| candidate.entry_kind == "js_ts_handler")
        );
    }

    #[test]
    fn detects_java_main_entry_point() {
        let file = file("src/main/java/App.java", "java");
        let result = detect_builtin_entry_points(
            BuiltinEntryPointLanguage::Java,
            &file,
            &[artefact("main", "method", "src/main/java/App.java")],
        );

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].entry_kind, "java_main");
    }

    #[test]
    fn detects_csharp_program_entry_point() {
        let file = file("src/App/Program.cs", "csharp");
        let result = detect_builtin_entry_points(
            BuiltinEntryPointLanguage::CSharp,
            &file,
            &[artefact("Main", "method", "src/App/Program.cs")],
        );

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].entry_kind, "dotnet_program");
    }

    #[test]
    fn detects_file_level_entry_when_no_symbol_matches() {
        let file = file("cmd/demo/main.go", "go");
        let result = detect_builtin_entry_points(BuiltinEntryPointLanguage::Go, &file, &[]);

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].entry_kind, "file_entry");
        assert_eq!(result[0].artefact_id, None);
    }

    #[test]
    fn detects_rust_cli_dispatch_function() {
        let file = file("crates/bitloops-inference/src/lib.rs", "rust");
        let result = detect_builtin_entry_points(
            BuiltinEntryPointLanguage::Rust,
            &file,
            &[artefact(
                "run_from_args",
                "function",
                "crates/bitloops-inference/src/lib.rs",
            )],
        );

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].entry_kind, "rust_cli_dispatch");
        assert_eq!(result[0].confidence, 0.88);
    }

    #[test]
    fn detects_next_route_handler_export() {
        let file = file("app/api/users/route.ts", "typescript");
        let result = detect_builtin_entry_points(
            BuiltinEntryPointLanguage::TsJs,
            &file,
            &[exported_artefact(
                "POST",
                "function",
                "app/api/users/route.ts",
            )],
        );

        assert!(
            result
                .iter()
                .any(|candidate| candidate.entry_kind == "next_route_handler")
        );
    }

    #[test]
    fn detects_python_app_factory() {
        let file = file("service/api/app.py", "python");
        let result = detect_builtin_entry_points(
            BuiltinEntryPointLanguage::Python,
            &file,
            &[artefact("create_app", "function", "service/api/app.py")],
        );

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].entry_kind, "python_web_app");
    }
}
