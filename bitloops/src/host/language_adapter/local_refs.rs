use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LocalTargetInfo {
    pub(crate) symbol_fqn: String,
    pub(crate) symbol_id: String,
    pub(crate) artefact_id: String,
    pub(crate) language_kind: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct LocalSourceFacts {
    pub(crate) import_refs: Vec<String>,
    pub(crate) package_refs: Vec<String>,
    pub(crate) namespace_refs: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ResolvedLocalTarget {
    pub(crate) symbol_fqn: String,
    pub(crate) symbol_id: String,
    pub(crate) artefact_id: String,
    pub(crate) edge_kind: String,
}

fn disallowed_rust_ref_token(token: &str) -> bool {
    token.is_empty()
        || token.contains('{')
        || token.contains('}')
        || token.contains('*')
        || token.contains(',')
        || token.contains('(')
        || token.contains(')')
        || token.contains(' ')
}

fn split_rust_ref_tokens(symbol_ref: &str) -> Option<Vec<&str>> {
    let tokens = symbol_ref
        .trim()
        .split("::")
        .map(str::trim)
        .collect::<Vec<_>>();
    if tokens.is_empty() || tokens.iter().any(|token| disallowed_rust_ref_token(token)) {
        return None;
    }
    Some(tokens)
}

fn source_root_and_module_segments(path: &str) -> Option<(String, Vec<String>)> {
    let segments = path
        .split('/')
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>();
    let src_idx = segments.iter().rposition(|segment| *segment == "src")?;
    let crate_root = segments[..=src_idx].join("/");
    let relative = segments.get(src_idx + 1..)?;
    let file_name = *relative.last()?;
    let mut module_segments = relative[..relative.len().saturating_sub(1)]
        .iter()
        .map(|segment| (*segment).to_string())
        .collect::<Vec<_>>();

    match file_name {
        "lib.rs" | "main.rs" => {}
        "mod.rs" => {}
        _ => {
            let stem = file_name.strip_suffix(".rs")?;
            module_segments.push(stem.to_string());
        }
    }

    Some((crate_root, module_segments))
}

pub(crate) fn rust_local_symbol_fqn_candidates(path: &str, symbol_ref: &str) -> Vec<String> {
    let Some(tokens) = split_rust_ref_tokens(symbol_ref) else {
        return Vec::new();
    };
    let Some((crate_root, current_module)) = source_root_and_module_segments(path) else {
        return Vec::new();
    };

    let mut idx = 0usize;
    let mut module_prefix = match tokens.first().copied() {
        Some("crate") => {
            idx = 1;
            Vec::new()
        }
        Some("self") => {
            idx = 1;
            current_module
        }
        Some("super") => {
            let mut prefix = current_module;
            while idx < tokens.len() && tokens[idx] == "super" {
                if prefix.pop().is_none() {
                    return Vec::new();
                }
                idx += 1;
            }
            prefix
        }
        _ => return Vec::new(),
    };

    let tail = &tokens[idx..];
    if tail.is_empty() {
        return Vec::new();
    }
    let symbol_name = tail.last().copied().unwrap_or_default();
    if symbol_name.is_empty() {
        return Vec::new();
    }

    module_prefix.extend(
        tail[..tail.len().saturating_sub(1)]
            .iter()
            .map(|segment| segment.to_string()),
    );
    let module_path = module_prefix.join("/");

    let candidate_paths = if module_path.is_empty() {
        vec![
            format!("{crate_root}/lib.rs"),
            format!("{crate_root}/main.rs"),
        ]
    } else {
        vec![
            format!("{crate_root}/{module_path}.rs"),
            format!("{crate_root}/{module_path}/mod.rs"),
        ]
    };

    candidate_paths
        .into_iter()
        .map(|candidate_path| format!("{candidate_path}::{symbol_name}"))
        .collect()
}

pub(crate) fn resolve_local_symbol_ref(
    language: &str,
    source_path: &str,
    edge_kind: &str,
    symbol_ref: &str,
    source_facts: &LocalSourceFacts,
    targets: &[LocalTargetInfo],
) -> Option<ResolvedLocalTarget> {
    let normalized = language.trim().to_ascii_lowercase();
    let target = match normalized.as_str() {
        "rust" => resolve_exact_candidates(
            targets,
            rust_local_symbol_fqn_candidates(source_path, symbol_ref),
        ),
        "typescript" | "javascript" => resolve_exact_candidates(
            targets,
            ts_js_local_symbol_fqn_candidates(source_path, symbol_ref),
        ),
        "python" => {
            resolve_exact_candidates(targets, python_local_symbol_fqn_candidates(symbol_ref))
        }
        "go" => resolve_go_same_package_symbol_ref(source_path, symbol_ref, targets),
        "java" => {
            resolve_java_local_symbol_ref(source_path, edge_kind, symbol_ref, source_facts, targets)
        }
        "csharp" => resolve_csharp_local_symbol_ref(edge_kind, symbol_ref, source_facts, targets),
        _ => None,
    }?;

    Some(ResolvedLocalTarget {
        symbol_fqn: target.symbol_fqn,
        symbol_id: target.symbol_id,
        artefact_id: target.artefact_id,
        edge_kind: normalize_resolved_edge_kind(
            normalized.as_str(),
            edge_kind,
            &target.language_kind,
        ),
    })
}

fn resolve_exact_candidates(
    targets: &[LocalTargetInfo],
    candidates: Vec<String>,
) -> Option<LocalTargetInfo> {
    if candidates.is_empty() {
        return None;
    }
    let candidate_set = candidates.into_iter().collect::<HashSet<_>>();
    unique_match(
        targets
            .iter()
            .filter(|target| candidate_set.contains(&target.symbol_fqn))
            .cloned(),
    )
}

fn unique_match<I>(matches: I) -> Option<LocalTargetInfo>
where
    I: IntoIterator<Item = LocalTargetInfo>,
{
    let deduped = matches
        .into_iter()
        .map(|target| {
            (
                target.symbol_fqn.clone(),
                (
                    target.symbol_id.clone(),
                    target.artefact_id.clone(),
                    target.language_kind.clone(),
                ),
            )
        })
        .collect::<HashMap<_, _>>();
    if deduped.len() != 1 {
        return None;
    }
    let (symbol_fqn, (symbol_id, artefact_id, language_kind)) = deduped.into_iter().next()?;
    Some(LocalTargetInfo {
        symbol_fqn,
        symbol_id,
        artefact_id,
        language_kind,
    })
}

fn ts_js_local_symbol_fqn_candidates(source_path: &str, symbol_ref: &str) -> Vec<String> {
    let Some((module_ref, symbol_tail)) = symbol_ref.split_once("::") else {
        return Vec::new();
    };
    if !module_ref.starts_with("./") && !module_ref.starts_with("../") {
        return Vec::new();
    }
    let base_dir = path_dir(source_path);
    let Some(module_path) = normalize_relative_path(base_dir, module_ref) else {
        return Vec::new();
    };
    let exts = ["ts", "tsx", "js", "jsx"];
    exts.into_iter()
        .flat_map(|ext| {
            [
                format!("{module_path}.{ext}::{symbol_tail}"),
                format!("{module_path}/index.{ext}::{symbol_tail}"),
            ]
        })
        .collect()
}

fn python_local_symbol_fqn_candidates(symbol_ref: &str) -> Vec<String> {
    let (module_ref, symbol_tail) = match symbol_ref.split_once("::") {
        Some(parts) => parts,
        None => return Vec::new(),
    };
    if module_ref.is_empty()
        || module_ref.starts_with('.')
        || module_ref.contains('/')
        || module_ref.contains(' ')
    {
        return Vec::new();
    }
    let module_path = module_ref.replace('.', "/");
    vec![
        format!("{module_path}.py::{symbol_tail}"),
        format!("{module_path}/__init__.py::{symbol_tail}"),
    ]
}

fn resolve_go_same_package_symbol_ref(
    source_path: &str,
    symbol_ref: &str,
    targets: &[LocalTargetInfo],
) -> Option<LocalTargetInfo> {
    let tokens = symbol_ref.split("::").collect::<Vec<_>>();
    if tokens.len() < 3 || tokens.first().copied() != Some("package") {
        return None;
    }
    let source_dir = path_dir(source_path);
    let tail = &tokens[2..];
    unique_match(
        targets
            .iter()
            .filter(|target| path_of_symbol_fqn(&target.symbol_fqn).ends_with(".go"))
            .filter(|target| path_dir(path_of_symbol_fqn(&target.symbol_fqn)) == source_dir)
            .filter(|target| symbol_suffix_matches(&target.symbol_fqn, tail))
            .cloned(),
    )
}

fn resolve_java_local_symbol_ref(
    _source_path: &str,
    edge_kind: &str,
    symbol_ref: &str,
    source_facts: &LocalSourceFacts,
    targets: &[LocalTargetInfo],
) -> Option<LocalTargetInfo> {
    if let Some((owner_ref, member_ref)) = symbol_ref.split_once("::") {
        return resolve_java_owner_and_tail(owner_ref, &[member_ref], source_facts, targets);
    }

    if edge_kind == "calls" {
        let tokens = symbol_ref.split('.').collect::<Vec<_>>();
        if tokens.len() >= 2 {
            let owner_ref = tokens[..tokens.len() - 1].join(".");
            let member_ref = tokens[tokens.len() - 1];
            if let Some(resolved) =
                resolve_java_owner_and_tail(&owner_ref, &[member_ref], source_facts, targets)
            {
                return Some(resolved);
            }
        }
    }

    if edge_kind == "calls" {
        return None;
    }

    let owner_ref = if symbol_ref.contains('.') {
        symbol_ref.to_string()
    } else {
        let source_package = unique_source_value(&source_facts.package_refs)?;
        format!("{source_package}.{symbol_ref}")
    };
    resolve_java_owner_and_tail(&owner_ref, &[], source_facts, targets)
}

fn resolve_java_owner_and_tail(
    owner_ref: &str,
    tail: &[&str],
    source_facts: &LocalSourceFacts,
    targets: &[LocalTargetInfo],
) -> Option<LocalTargetInfo> {
    let owner_tokens = owner_ref
        .split('.')
        .filter(|token| !token.is_empty())
        .map(str::to_string)
        .collect::<Vec<_>>();
    let (package_tokens, type_name) = if owner_tokens.len() >= 2 {
        (
            owner_tokens[..owner_tokens.len() - 1].to_vec(),
            owner_tokens[owner_tokens.len() - 1].clone(),
        )
    } else if owner_tokens.len() == 1 {
        let source_package = unique_source_value(&source_facts.package_refs)?;
        let package_tokens = source_package
            .split('.')
            .map(str::to_string)
            .collect::<Vec<_>>();
        (package_tokens, owner_tokens[0].clone())
    } else {
        return None;
    };
    let path_suffix = format!("{}/{}.java", package_tokens.join("/"), type_name);
    let mut symbol_tail = vec![type_name];
    symbol_tail.extend(tail.iter().map(|segment| (*segment).to_string()));
    unique_match(
        targets
            .iter()
            .filter(|target| path_of_symbol_fqn(&target.symbol_fqn).ends_with(".java"))
            .filter(|target| path_of_symbol_fqn(&target.symbol_fqn).ends_with(&path_suffix))
            .filter(|target| symbol_suffix_matches_owned(&target.symbol_fqn, &symbol_tail))
            .cloned(),
    )
}

fn resolve_csharp_local_symbol_ref(
    edge_kind: &str,
    symbol_ref: &str,
    source_facts: &LocalSourceFacts,
    targets: &[LocalTargetInfo],
) -> Option<LocalTargetInfo> {
    if edge_kind == "calls" || symbol_ref.contains("::") || symbol_ref.contains('.') {
        return None;
    }
    let namespaces = source_facts
        .namespace_refs
        .iter()
        .chain(source_facts.import_refs.iter())
        .cloned()
        .collect::<HashSet<_>>();
    if namespaces.is_empty() {
        return None;
    }
    let target_namespaces = targets
        .iter()
        .filter_map(|target| {
            csharp_namespace_for_target(target, targets).map(|ns| (target.symbol_fqn.clone(), ns))
        })
        .collect::<HashMap<_, _>>();
    unique_match(
        targets
            .iter()
            .filter(|target| path_of_symbol_fqn(&target.symbol_fqn).ends_with(".cs"))
            .filter(|target| symbol_suffix_matches(&target.symbol_fqn, &[symbol_ref]))
            .filter(|target| {
                target_namespaces
                    .get(&target.symbol_fqn)
                    .is_some_and(|namespace| namespaces.contains(namespace))
            })
            .cloned(),
    )
}

fn csharp_namespace_for_target<'a>(
    target: &'a LocalTargetInfo,
    targets: &'a [LocalTargetInfo],
) -> Option<String> {
    let path = path_of_symbol_fqn(&target.symbol_fqn);
    targets
        .iter()
        .find(|candidate| {
            path_of_symbol_fqn(&candidate.symbol_fqn) == path
                && matches!(
                    candidate.language_kind.as_str(),
                    "namespace_declaration" | "file_scoped_namespace_declaration"
                )
        })
        .and_then(|namespace| namespace.symbol_fqn.split_once("::ns::"))
        .map(|(_, namespace)| namespace.to_string())
}

fn normalize_resolved_edge_kind(
    language: &str,
    edge_kind: &str,
    target_language_kind: &str,
) -> String {
    if language == "csharp"
        && edge_kind == "implements"
        && !matches!(target_language_kind, "interface_declaration" | "interface")
    {
        "extends".to_string()
    } else {
        edge_kind.to_string()
    }
}

fn unique_source_value(values: &[String]) -> Option<String> {
    let unique = values.iter().cloned().collect::<HashSet<_>>();
    if unique.len() != 1 {
        return None;
    }
    unique.into_iter().next()
}

fn normalize_relative_path(base_dir: &str, relative: &str) -> Option<String> {
    let mut parts = base_dir
        .split('/')
        .filter(|segment| !segment.is_empty())
        .map(str::to_string)
        .collect::<Vec<_>>();
    for token in relative.split('/') {
        match token {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            value => parts.push(value.to_string()),
        }
    }
    if parts.is_empty() {
        return None;
    }
    Some(parts.join("/"))
}

fn path_dir(path: &str) -> &str {
    path.rsplit_once('/').map(|(dir, _)| dir).unwrap_or("")
}

fn path_of_symbol_fqn(symbol_fqn: &str) -> &str {
    symbol_fqn
        .split_once("::")
        .map(|(path, _)| path)
        .unwrap_or(symbol_fqn)
}

fn symbol_suffix_matches(symbol_fqn: &str, expected_tail: &[&str]) -> bool {
    let Some((_, tail)) = symbol_fqn.split_once("::") else {
        return false;
    };
    let actual = tail.split("::").collect::<Vec<_>>();
    actual == expected_tail
}

fn symbol_suffix_matches_owned(symbol_fqn: &str, expected_tail: &[String]) -> bool {
    let Some((_, tail)) = symbol_fqn.split_once("::") else {
        return false;
    };
    let actual = tail.split("::").map(str::to_string).collect::<Vec<_>>();
    actual == expected_tail
}

#[cfg(test)]
mod tests {
    use super::{
        LocalSourceFacts, LocalTargetInfo, resolve_local_symbol_ref,
        rust_local_symbol_fqn_candidates,
    };

    fn target(
        symbol_fqn: &str,
        symbol_id: &str,
        artefact_id: &str,
        language_kind: &str,
    ) -> LocalTargetInfo {
        LocalTargetInfo {
            symbol_fqn: symbol_fqn.to_string(),
            symbol_id: symbol_id.to_string(),
            artefact_id: artefact_id.to_string(),
            language_kind: language_kind.to_string(),
        }
    }

    #[test]
    fn rust_local_symbol_fqn_candidates_handle_ruff_style_super_paths() {
        let candidates = rust_local_symbol_fqn_candidates(
            "crates/ruff_linter/src/rules/pyflakes/rules/strings.rs",
            "super::super::fixes::remove_unused_positional_arguments_from_format_call",
        );

        assert_eq!(
            candidates,
            vec![
                "crates/ruff_linter/src/rules/pyflakes/fixes.rs::remove_unused_positional_arguments_from_format_call".to_string(),
                "crates/ruff_linter/src/rules/pyflakes/fixes/mod.rs::remove_unused_positional_arguments_from_format_call".to_string(),
            ]
        );
    }

    #[test]
    fn resolve_local_symbol_ref_handles_typescript_relative_imports() {
        let resolved = resolve_local_symbol_ref(
            "typescript",
            "src/caller.ts",
            "calls",
            "./utils::helper",
            &LocalSourceFacts::default(),
            &[target(
                "src/utils.ts::helper",
                "helper",
                "artefact",
                "function_declaration",
            )],
        )
        .expect("expected relative import to resolve");

        assert_eq!(resolved.symbol_fqn, "src/utils.ts::helper");
    }

    #[test]
    fn resolve_local_symbol_ref_handles_python_module_imports() {
        let resolved = resolve_local_symbol_ref(
            "python",
            "pkg/main.py",
            "calls",
            "pkg.helpers::helper",
            &LocalSourceFacts::default(),
            &[target(
                "pkg/helpers.py::helper",
                "helper",
                "artefact",
                "function_definition",
            )],
        )
        .expect("expected python module import to resolve");

        assert_eq!(resolved.symbol_fqn, "pkg/helpers.py::helper");
    }

    #[test]
    fn resolve_local_symbol_ref_handles_go_same_package_refs() {
        let resolved = resolve_local_symbol_ref(
            "go",
            "service/run.go",
            "calls",
            "package::service::helper",
            &LocalSourceFacts::default(),
            &[target(
                "service/helper.go::helper",
                "helper",
                "artefact",
                "function_declaration",
            )],
        )
        .expect("expected go package ref to resolve");

        assert_eq!(resolved.symbol_fqn, "service/helper.go::helper");
    }

    #[test]
    fn resolve_local_symbol_ref_handles_java_package_qualified_calls() {
        let resolved = resolve_local_symbol_ref(
            "java",
            "src/com/acme/Greeter.java",
            "calls",
            "com.acme.Util::helper",
            &LocalSourceFacts {
                package_refs: vec!["com.acme".to_string()],
                ..LocalSourceFacts::default()
            },
            &[target(
                "src/com/acme/Util.java::Util::helper",
                "helper",
                "artefact",
                "method_declaration",
            )],
        )
        .expect("expected java imported type call to resolve");

        assert_eq!(resolved.symbol_fqn, "src/com/acme/Util.java::Util::helper");
    }

    #[test]
    fn resolve_local_symbol_ref_handles_java_same_package_type_refs() {
        let resolved = resolve_local_symbol_ref(
            "java",
            "src/com/acme/Greeter.java",
            "extends",
            "Base",
            &LocalSourceFacts {
                package_refs: vec!["com.acme".to_string()],
                ..LocalSourceFacts::default()
            },
            &[target(
                "src/com/acme/Base.java::Base",
                "base",
                "artefact",
                "class_declaration",
            )],
        )
        .expect("expected java same-package type ref to resolve");

        assert_eq!(resolved.symbol_fqn, "src/com/acme/Base.java::Base");
    }

    #[test]
    fn resolve_local_symbol_ref_handles_csharp_namespace_type_refs() {
        let targets = [
            target(
                "src/BaseService.cs::ns::MyApp.Services",
                "ns",
                "ns-artefact",
                "file_scoped_namespace_declaration",
            ),
            target(
                "src/BaseService.cs::BaseService",
                "base",
                "base-artefact",
                "class_declaration",
            ),
        ];
        let resolved = resolve_local_symbol_ref(
            "csharp",
            "src/UserService.cs",
            "extends",
            "BaseService",
            &LocalSourceFacts {
                namespace_refs: vec!["MyApp.Services".to_string()],
                ..LocalSourceFacts::default()
            },
            &targets,
        )
        .expect("expected csharp namespace type ref to resolve");

        assert_eq!(resolved.symbol_fqn, "src/BaseService.cs::BaseService");
        assert_eq!(resolved.edge_kind, "extends");
    }

    #[test]
    fn resolve_local_symbol_ref_handles_csharp_imported_namespace_type_refs() {
        let targets = [
            target(
                "src/BaseService.cs::ns::MyApp.Services",
                "ns",
                "ns-artefact",
                "file_scoped_namespace_declaration",
            ),
            target(
                "src/BaseService.cs::BaseService",
                "base",
                "base-artefact",
                "class_declaration",
            ),
        ];
        let resolved = resolve_local_symbol_ref(
            "csharp",
            "src/UserService.cs",
            "implements",
            "BaseService",
            &LocalSourceFacts {
                import_refs: vec!["MyApp.Services".to_string()],
                ..LocalSourceFacts::default()
            },
            &targets,
        )
        .expect("expected csharp imported namespace type ref to resolve");

        assert_eq!(resolved.symbol_fqn, "src/BaseService.cs::BaseService");
        assert_eq!(resolved.edge_kind, "extends");
    }

    #[test]
    fn resolve_local_symbol_ref_rejects_ambiguous_python_matches() {
        let resolved = resolve_local_symbol_ref(
            "python",
            "pkg/main.py",
            "calls",
            "pkg.helpers::helper",
            &LocalSourceFacts::default(),
            &[
                target(
                    "pkg/helpers.py::helper",
                    "a",
                    "artefact-a",
                    "function_definition",
                ),
                target(
                    "pkg/helpers/__init__.py::helper",
                    "b",
                    "artefact-b",
                    "function_definition",
                ),
            ],
        );

        assert!(resolved.is_none());
    }
}
