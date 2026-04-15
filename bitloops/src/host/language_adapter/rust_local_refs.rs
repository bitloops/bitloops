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

#[cfg(test)]
mod tests {
    use super::rust_local_symbol_fqn_candidates;

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
    fn rust_local_symbol_fqn_candidates_handle_crate_root_refs() {
        let candidates = rust_local_symbol_fqn_candidates("src/lib.rs", "crate::utils::slugify");

        assert_eq!(
            candidates,
            vec![
                "src/utils.rs::slugify".to_string(),
                "src/utils/mod.rs::slugify".to_string(),
            ]
        );
    }

    #[test]
    fn rust_local_symbol_fqn_candidates_handle_self_refs_from_non_mod_files() {
        let candidates = rust_local_symbol_fqn_candidates("src/nested/helpers.rs", "self::helper");

        assert_eq!(
            candidates,
            vec![
                "src/nested/helpers.rs::helper".to_string(),
                "src/nested/helpers/mod.rs::helper".to_string(),
            ]
        );
    }

    #[test]
    fn rust_local_symbol_fqn_candidates_handle_super_refs_from_mod_backed_modules() {
        let candidates =
            rust_local_symbol_fqn_candidates("src/nested/inner/mod.rs", "super::types::Config");

        assert_eq!(
            candidates,
            vec![
                "src/nested/types.rs::Config".to_string(),
                "src/nested/types/mod.rs::Config".to_string(),
            ]
        );
    }

    #[test]
    fn rust_local_symbol_fqn_candidates_ignore_external_refs() {
        assert!(rust_local_symbol_fqn_candidates("src/lib.rs", "log::info").is_empty());
    }

    #[test]
    fn rust_local_symbol_fqn_candidates_ignore_ambiguous_plain_refs() {
        assert!(rust_local_symbol_fqn_candidates("src/lib.rs", "foo::bar").is_empty());
    }
}
