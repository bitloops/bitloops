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

pub(crate) fn normalize_local_edge_symbol_refs(
    language: &str,
    _source_path: &str,
    edge_kind: &str,
    symbol_ref: &str,
) -> Vec<String> {
    let trimmed = symbol_ref.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }
    if edge_kind != "imports" {
        return vec![trimmed.to_string()];
    }

    match language.trim().to_ascii_lowercase().as_str() {
        "rust" => normalize_rust_import_symbol_refs(trimmed),
        _ => vec![trimmed.to_string()],
    }
}

fn normalize_rust_import_symbol_refs(symbol_ref: &str) -> Vec<String> {
    let expanded = expand_rust_import_expression(symbol_ref);
    if expanded.is_empty() {
        return vec![symbol_ref.trim().to_string()];
    }
    expanded
}

fn expand_rust_import_expression(expression: &str) -> Vec<String> {
    let expression = expression.trim();
    if expression.is_empty() {
        return Vec::new();
    }

    if let Some(open_idx) = find_top_level_char(expression, '{')
        && let Some(close_idx) = find_matching_brace(expression, open_idx)
    {
        let prefix = expression[..open_idx].trim().trim_end_matches("::");
        let inside = &expression[open_idx + 1..close_idx];
        let suffix = expression[close_idx + 1..].trim().trim_start_matches("::");
        let mut expanded = Vec::new();

        for part in split_top_level_commas(inside) {
            for nested in expand_rust_import_expression(part) {
                let nested = strip_rust_import_alias(&nested);
                if nested.is_empty() {
                    continue;
                }
                let combined = if prefix.is_empty() {
                    nested
                } else {
                    format!("{prefix}::{nested}")
                };
                if suffix.is_empty() {
                    expanded.push(combined);
                } else {
                    expanded.push(format!("{combined}::{suffix}"));
                }
            }
        }

        return expanded;
    }

    vec![strip_rust_import_alias(expression)]
}

fn strip_rust_import_alias(expression: &str) -> String {
    expression
        .split(" as ")
        .next()
        .unwrap_or(expression)
        .trim()
        .to_string()
}

fn find_top_level_char(value: &str, target: char) -> Option<usize> {
    let mut brace_depth = 0i32;
    for (idx, ch) in value.char_indices() {
        match ch {
            '{' => {
                if ch == target && brace_depth == 0 {
                    return Some(idx);
                }
                brace_depth += 1;
            }
            '}' => brace_depth -= 1,
            _ if ch == target && brace_depth == 0 => return Some(idx),
            _ => {}
        }
    }
    None
}

fn find_matching_brace(value: &str, open_idx: usize) -> Option<usize> {
    let mut depth = 0i32;
    for (idx, ch) in value.char_indices().skip_while(|(idx, _)| *idx < open_idx) {
        match ch {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(idx);
                }
            }
            _ => {}
        }
    }
    None
}

fn split_top_level_commas(value: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut start = 0usize;
    let mut depth = 0i32;

    for (idx, ch) in value.char_indices() {
        match ch {
            '{' => depth += 1,
            '}' => depth -= 1,
            ',' if depth == 0 => {
                let part = value[start..idx].trim();
                if !part.is_empty() {
                    parts.push(part);
                }
                start = idx + 1;
            }
            _ => {}
        }
    }

    let tail = value[start..].trim();
    if !tail.is_empty() {
        parts.push(tail);
    }
    parts
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

fn rust_local_module_fqn_candidates(path: &str, symbol_ref: &str) -> Vec<String> {
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
        return vec![
            format!("{crate_root}/lib.rs"),
            format!("{crate_root}/main.rs"),
        ];
    }

    let module_tail = if tail.last().copied() == Some("self") {
        &tail[..tail.len().saturating_sub(1)]
    } else {
        tail
    };
    module_prefix.extend(module_tail.iter().map(|segment| (*segment).to_string()));
    let module_path = module_prefix.join("/");

    if module_path.is_empty() {
        return vec![
            format!("{crate_root}/lib.rs"),
            format!("{crate_root}/main.rs"),
        ];
    }

    vec![
        format!("{crate_root}/{module_path}.rs"),
        format!("{crate_root}/{module_path}/mod.rs"),
    ]
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
    let target = if edge_kind == "imports" {
        match normalized.as_str() {
            "rust" => {
                if symbol_ref
                    .rsplit("::")
                    .next()
                    .is_some_and(|token| token.trim() == "self")
                {
                    resolve_exact_candidates(
                        targets,
                        rust_local_module_fqn_candidates(source_path, symbol_ref),
                    )
                } else {
                    resolve_exact_candidates(
                        targets,
                        rust_local_symbol_fqn_candidates(source_path, symbol_ref),
                    )
                }
            }
            "typescript" | "javascript" => resolve_exact_candidates(
                targets,
                ts_js_local_import_fqn_candidates(source_path, symbol_ref),
            ),
            "python" => resolve_exact_candidates(
                targets,
                python_local_import_fqn_candidates(source_path, symbol_ref),
            ),
            "java" => resolve_java_import_ref(symbol_ref, source_facts, targets),
            "csharp" => resolve_csharp_import_ref(symbol_ref, targets),
            _ => None,
        }
    } else {
        match normalized.as_str() {
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
            "java" => resolve_java_local_symbol_ref(
                source_path,
                edge_kind,
                symbol_ref,
                source_facts,
                targets,
            ),
            "csharp" => {
                resolve_csharp_local_symbol_ref(edge_kind, symbol_ref, source_facts, targets)
            }
            _ => None,
        }
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

fn ts_js_local_import_fqn_candidates(source_path: &str, symbol_ref: &str) -> Vec<String> {
    if !symbol_ref.starts_with("./") && !symbol_ref.starts_with("../") {
        return Vec::new();
    }
    let base_dir = path_dir(source_path);
    let Some(module_path) = normalize_relative_path(base_dir, symbol_ref) else {
        return Vec::new();
    };
    let exts = ["ts", "tsx", "js", "jsx"];
    exts.into_iter()
        .flat_map(|ext| {
            [
                format!("{module_path}.{ext}"),
                format!("{module_path}/index.{ext}"),
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

fn python_local_import_fqn_candidates(source_path: &str, symbol_ref: &str) -> Vec<String> {
    python_module_path_candidates(source_path, symbol_ref)
}

fn python_module_path_candidates(source_path: &str, module_ref: &str) -> Vec<String> {
    let module_ref = module_ref.trim();
    if module_ref.is_empty() || module_ref.contains('/') || module_ref.contains(' ') {
        return Vec::new();
    }

    if module_ref.starts_with('.') {
        let leading_dots = module_ref.chars().take_while(|ch| *ch == '.').count();
        if leading_dots == 0 {
            return Vec::new();
        }
        let mut base_parts = path_dir(source_path)
            .split('/')
            .filter(|segment| !segment.is_empty())
            .map(str::to_string)
            .collect::<Vec<_>>();
        for _ in 1..leading_dots {
            if base_parts.pop().is_none() {
                return Vec::new();
            }
        }
        let remainder = module_ref[leading_dots..].trim();
        if remainder.is_empty() {
            let base = base_parts.join("/");
            return if base.is_empty() {
                vec!["__init__.py".to_string()]
            } else {
                vec![format!("{base}/__init__.py")]
            };
        }
        let mut module_parts = base_parts;
        module_parts.extend(
            remainder
                .split('.')
                .filter(|token| !token.is_empty())
                .map(str::to_string),
        );
        if module_parts.is_empty() {
            return Vec::new();
        }
        let module_path = module_parts.join("/");
        return vec![
            format!("{module_path}.py"),
            format!("{module_path}/__init__.py"),
        ];
    }

    let module_path = module_ref.replace('.', "/");
    vec![
        format!("{module_path}.py"),
        format!("{module_path}/__init__.py"),
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

fn resolve_java_import_ref(
    symbol_ref: &str,
    source_facts: &LocalSourceFacts,
    targets: &[LocalTargetInfo],
) -> Option<LocalTargetInfo> {
    if symbol_ref.ends_with(".*") {
        return None;
    }
    if let Some((owner_ref, member_ref)) = symbol_ref.rsplit_once('.')
        && owner_ref.split('.').count() >= 2
        && let Some(resolved) =
            resolve_java_owner_and_tail(owner_ref, &[member_ref], source_facts, targets)
    {
        return Some(resolved);
    }
    resolve_java_owner_and_tail(symbol_ref, &[], source_facts, targets)
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

fn resolve_csharp_import_ref(
    symbol_ref: &str,
    targets: &[LocalTargetInfo],
) -> Option<LocalTargetInfo> {
    let symbol_ref = symbol_ref.trim();
    if symbol_ref.is_empty() || symbol_ref.contains('=') || symbol_ref.contains("::") {
        return None;
    }

    let namespace_match = unique_match(
        targets
            .iter()
            .filter(|target| {
                matches!(
                    target.language_kind.as_str(),
                    "namespace_declaration" | "file_scoped_namespace_declaration"
                )
            })
            .filter(|target| {
                target
                    .symbol_fqn
                    .split_once("::ns::")
                    .map(|(_, namespace)| namespace == symbol_ref)
                    .unwrap_or(false)
            })
            .cloned(),
    );
    if namespace_match.is_some() {
        return namespace_match;
    }

    let (namespace_ref, type_name) = symbol_ref.rsplit_once('.')?;
    unique_match(
        targets
            .iter()
            .filter(|target| path_of_symbol_fqn(&target.symbol_fqn).ends_with(".cs"))
            .filter(|target| symbol_suffix_matches(&target.symbol_fqn, &[type_name]))
            .filter(|target| {
                csharp_namespace_for_target(target, targets)
                    .as_deref()
                    .is_some_and(|namespace| namespace == namespace_ref)
            })
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
    let namespaces = csharp_source_namespaces(source_facts, targets);
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

fn csharp_source_namespaces(
    source_facts: &LocalSourceFacts,
    targets: &[LocalTargetInfo],
) -> HashSet<String> {
    let mut namespaces = source_facts
        .namespace_refs
        .iter()
        .cloned()
        .collect::<HashSet<_>>();
    for import_ref in &source_facts.import_refs {
        let import_ref = import_ref.trim();
        if import_ref.is_empty() {
            continue;
        }
        if let Some((_, namespace)) = import_ref.split_once("::ns::") {
            namespaces.insert(namespace.to_string());
            continue;
        }
        if import_ref.contains("::") {
            if let Some(target) = targets
                .iter()
                .find(|target| target.symbol_fqn == import_ref)
                && let Some(namespace) = csharp_namespace_for_target(target, targets)
            {
                namespaces.insert(namespace);
            }
            continue;
        }
        if import_ref.contains('.') && !import_ref.contains('=') {
            namespaces.insert(import_ref.to_string());
        }
    }
    namespaces
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
mod tests;
