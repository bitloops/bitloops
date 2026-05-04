use super::*;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ClapCommandDescriptor {
    pub(super) name: String,
    pub(super) about: String,
}

pub(super) fn extract_clap_subcommand_enums(
    content: &str,
) -> Vec<(String, Vec<ClapCommandDescriptor>)> {
    let mut enum_names = Vec::new();
    let mut saw_subcommand_derive = false;
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("#[derive") && trimmed.contains("Subcommand") {
            saw_subcommand_derive = true;
            continue;
        }
        if saw_subcommand_derive {
            if let Some(enum_name) = enum_name_from_line(trimmed) {
                enum_names.push(enum_name.to_string());
            }
            saw_subcommand_derive = false;
        }
    }

    enum_names
        .into_iter()
        .filter_map(|enum_name| {
            let commands = extract_clap_enum_commands(content, &enum_name);
            (!commands.is_empty()).then_some((enum_name, commands))
        })
        .collect()
}

pub(super) fn extract_clap_enum_commands(
    content: &str,
    enum_name: &str,
) -> Vec<ClapCommandDescriptor> {
    let mut commands = Vec::new();
    let mut in_enum = false;
    let mut depth: i32 = 0;
    let mut docs = Vec::new();
    let mut attrs = Vec::new();

    for line in content.lines() {
        let trimmed = line.trim();
        if !in_enum {
            if trimmed.contains(&format!("enum {enum_name}")) {
                in_enum = true;
                depth += brace_delta(trimmed);
            }
            continue;
        }

        depth += brace_delta(trimmed);
        if depth <= 0 {
            break;
        }
        if let Some(doc) = trimmed.strip_prefix("///") {
            docs.push(doc.trim().to_string());
            continue;
        }
        if trimmed.starts_with("#[") {
            attrs.push(trimmed.to_string());
            continue;
        }
        let Some(variant) = clap_variant_name(trimmed) else {
            continue;
        };
        if attrs.iter().any(|attr| attr.contains("hide = true")) {
            docs.clear();
            attrs.clear();
            continue;
        }
        let command_name =
            command_name_from_attrs(&attrs).unwrap_or_else(|| variant_to_kebab(variant));
        let about = docs.join(" ");
        commands.push(ClapCommandDescriptor {
            name: command_name,
            about,
        });
        docs.clear();
        attrs.clear();
    }

    commands
}

pub(super) fn enum_name_from_line(line: &str) -> Option<&str> {
    let enum_start = line.find("enum ")?;
    let after_enum = &line[enum_start + "enum ".len()..];
    after_enum
        .split([' ', '{', '<'])
        .next()
        .map(str::trim)
        .filter(|name| !name.is_empty())
}

pub(super) fn brace_delta(line: &str) -> i32 {
    line.chars().fold(0, |delta, ch| match ch {
        '{' => delta + 1,
        '}' => delta - 1,
        _ => delta,
    })
}

pub(super) fn clap_variant_name(line: &str) -> Option<&str> {
    let candidate = line
        .split(['(', '{', ','])
        .next()
        .map(str::trim)
        .filter(|candidate| !candidate.is_empty())?;
    candidate
        .chars()
        .next()
        .is_some_and(|ch| ch.is_ascii_uppercase())
        .then_some(candidate)
}

pub(super) fn command_name_from_attrs(attrs: &[String]) -> Option<String> {
    attrs.iter().find_map(|attr| {
        let name_start = attr.find("name")?;
        let after_name = &attr[name_start..];
        let quote_start = after_name.find('"')?;
        let after_quote = &after_name[quote_start + 1..];
        let quote_end = after_quote.find('"')?;
        Some(after_quote[..quote_end].to_string())
    })
}

pub(super) fn variant_to_kebab(value: &str) -> String {
    let mut output = String::new();
    for (index, ch) in value.chars().enumerate() {
        if ch.is_ascii_uppercase() {
            if index > 0 {
                output.push('-');
            }
            output.push(ch.to_ascii_lowercase());
        } else {
            output.push(ch);
        }
    }
    output
}

pub(super) fn extract_axum_route_paths(content: &str) -> BTreeSet<String> {
    let lines = content.lines().collect::<Vec<_>>();
    let mut routes = BTreeSet::new();
    for (index, line) in lines.iter().enumerate() {
        if !line.contains(".route(") {
            continue;
        }
        for route_line in lines.iter().skip(index).take(4) {
            if let Some(route) = first_string_literal(route_line) {
                routes.insert(route);
                break;
            }
        }
    }
    routes
}

pub(super) fn first_string_literal(line: &str) -> Option<String> {
    let start = line.find('"')?;
    let rest = &line[start + 1..];
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
}

pub(super) fn is_primary_http_route(route: &str) -> bool {
    if !route.starts_with('/') || route == "/" {
        return false;
    }
    let normalised = route.trim_end_matches('/');
    let last_segment = normalised.rsplit('/').next().unwrap_or_default();
    if matches!(last_segment, "playground" | "sdl" | "ws") {
        return false;
    }
    let segment_count = normalised
        .split('/')
        .filter(|segment| !segment.is_empty())
        .count();
    segment_count <= 3 || route.contains('{') || route.contains(':')
}

pub(super) fn detect_pyproject_entry_points(
    repo_root: &Path,
    config_path: &str,
    file_paths: &BTreeSet<String>,
    artefacts_by_path: &BTreeMap<String, Vec<LanguageEntryPointArtefact>>,
    candidates: &mut Vec<LanguageEntryPointCandidate>,
) {
    let Some(content) = read_repo_file(repo_root, config_path) else {
        return;
    };
    let Ok(document) = content.parse::<toml_edit::DocumentMut>() else {
        return;
    };
    let Some(scripts) = document
        .get("project")
        .and_then(|project| project.get("scripts"))
        .and_then(|scripts| scripts.as_table())
    else {
        return;
    };
    for (name, value) in scripts {
        let Some(target) = value.as_str() else {
            continue;
        };
        let Some((module, _function)) = target.split_once(':') else {
            continue;
        };
        let module_path = module.replace('.', "/");
        let candidates_for_script = [
            repo_relative_join(config_path, &format!("{module_path}.py")),
            repo_relative_join(config_path, &format!("src/{module_path}.py")),
            repo_relative_join(config_path, &format!("{module_path}/__init__.py")),
            repo_relative_join(config_path, &format!("src/{module_path}/__init__.py")),
        ];
        if let Some(path) = candidates_for_script
            .into_iter()
            .find(|path| file_paths.contains(path))
        {
            candidates.push(config_candidate_for_path(
                &path,
                artefacts_by_path,
                "python_console_script",
                name,
                0.86,
                "pyproject.toml console script",
                vec![config_path.to_string(), target.to_string(), path.clone()],
            ));
        }
    }
}
